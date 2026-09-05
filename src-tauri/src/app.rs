//! Native client wiring. Agent execution belongs exclusively to guacad.
use crate::host::{Connection, LocalHost, Status};
use crate::tray::{self, Tray};
use std::sync::Arc;
use tauri::Manager;

#[tauri::command]
async fn local_host_status(state: tauri::State<'_, LocalHost>) -> Result<Status, String> {
    Ok(state.status().await)
}
#[tauri::command]
async fn local_host_start(state: tauri::State<'_, LocalHost>) -> Result<Connection, String> {
    state.start().await
}
#[tauri::command]
async fn local_host_update(state: tauri::State<'_, LocalHost>) -> Result<Connection, String> {
    state.update().await
}
#[tauri::command]
async fn local_hosts(
    state: tauri::State<'_, LocalHost>,
) -> Result<Vec<crate::host::ExistingHost>, String> {
    state.existing().await
}
#[tauri::command]
async fn connect_local_host(
    state: tauri::State<'_, LocalHost>,
    name: String,
) -> Result<Connection, String> {
    state.connect_existing(&name).await
}
#[tauri::command]
async fn open_docker() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let status = tokio::process::Command::new("/usr/bin/open")
            .args(["-a", "Docker"])
            .status()
            .await
            .map_err(|e| e.to_string())?;
        if status.success() {
            return Ok(());
        }
    }
    Err("Open Docker Desktop from your applications, then check again.".into())
}
#[tauri::command]
async fn forward_files(
    origin: String,
    token: String,
    paths: Vec<String>,
) -> crate::commands::Reply<crate::commands::Staged> {
    crate::commands::upload_local_files(origin, token, paths).await
}
#[tauri::command]
async fn report_presence(app: tauri::AppHandle, presence: Option<crate::menubar::Presence>) {
    if let Some(tray) = app.try_state::<Arc<Tray>>() {
        tray.feed(presence);
    }
}

fn hide_rather_than_quit(window: &tauri::Window, event: &tauri::WindowEvent) {
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        if window.app_handle().tray_by_id(tray::TRAY_ID).is_some() {
            api.prevent_close();
            if let Err(err) = window.hide() {
                tracing::warn!(%err, "could not hide window");
            }
        }
    }
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("GUAC_LOG")
                .unwrap_or_else(|_| "guac=info,warn".into()),
        )
        .init();
    crate::programs::adopt_operator_path();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .on_window_event(hide_rather_than_quit)
        .setup(|app| {
            app.manage(LocalHost::new(&app.config().identifier, crate::host::IMAGE));
            match Tray::install(app.handle()) {
                Ok(tray) => {
                    app.manage(tray);
                }
                Err(err) => tracing::warn!(%err, "no menu bar presence this session"),
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            local_host_status,
            local_host_start,
            local_host_update,
            local_hosts,
            connect_local_host,
            open_docker,
            forward_files,
            report_presence,
            legacy_groups,
            export_legacy_group,
            save_group_export,
        ])
        .run(tauri::generate_context!())
        .expect("could not run Guaca");
}

#[derive(serde::Serialize)]
struct LegacyGroup {
    id: crate::domain::ids::GroupId,
    name: String,
}
fn legacy_root(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let own = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let parent = own.parent().ok_or("The application data folder is unavailable.")?;
    Ok(parent.join("com.madebywelch.guac"))
}
#[tauri::command]
async fn legacy_groups(app: tauri::AppHandle) -> Result<Vec<LegacyGroup>, String> {
    let root = legacy_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = root.join("guac.db");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let conn =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|e| e.to_string())?;
        let mut query = conn
            .prepare("SELECT id,name FROM groups ORDER BY created_at")
            .map_err(|e| e.to_string())?;
        let rows = query
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        rows.map(|r| {
            let (id, name) = r.map_err(|e| e.to_string())?;
            Ok(LegacyGroup { id: id.parse().map_err(|_| "Invalid group identifier.")?, name })
        })
        .collect()
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn export_legacy_group(
    app: tauri::AppHandle,
    id: crate::domain::ids::GroupId,
) -> Result<crate::transfer::Archive, String> {
    let root = legacy_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = rusqlite::Connection::open_with_flags(
            root.join("guac.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| e.to_string())?;
        let disk = crate::runtime::OnDisk::under(&root);
        crate::transfer::export(&mut conn, id, &disk.workspace, &disk.files)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn save_group_export(
    app: tauri::AppHandle,
    archive: crate::transfer::Archive,
) -> Result<String, String> {
    let root = app.path().download_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::Write;
        let bytes = serde_json::to_vec(&archive).map_err(|e| e.to_string())?;
        if bytes.len() > crate::transfer::MAX_ARCHIVE {
            return Err("The group file exceeds 64 MB.".into());
        }
        let path = root.join(format!("Guaca-group-{}.guaca.json", uuid::Uuid::new_v4()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(|e| e.to_string())?;
        file.write_all(&bytes).and_then(|_| file.sync_all()).map_err(|e| e.to_string())?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}
