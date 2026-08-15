//! Tauri application wiring.
//!
//! The only file that knows Tauri exists. Everything below it is a plain Rust
//! library with plain tests, which is why the cascade tests can drive the real
//! runtime without a window.

use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::commands::{self, AppState};
use crate::config;
use crate::db::Store;
use crate::llm::openrouter::LlmClient;
use crate::proxy;
use crate::runtime::events::{EventSink, UiEvent, CHANNEL};
use crate::runtime::Runtime;
use crate::workspace::Workspace;

/// Bridges runtime events onto the webview's event bus.
struct TauriSink {
    app: tauri::AppHandle,
}

impl EventSink for TauriSink {
    fn emit(&self, event: UiEvent) {
        // A failed emit means the window is gone. That is not an error worth
        // propagating into an agent's turn; the transcript is already durable.
        if let Err(err) = self.app.emit(CHANNEL, &event) {
            tracing::debug!(%err, "dropping event, no window to receive it");
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

    // Owned here so it outlives the app. Agent actors are spawned onto this.
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("guac-agent")
        .build()
        .expect("failed to start the async runtime");
    let handle = tokio_runtime.handle().clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let data_dir = app.path().app_data_dir()?;
            let config_dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            std::fs::create_dir_all(&config_dir)?;

            let db_path = data_dir.join("guac.db");
            let config_path = config_dir.join("config.json");
            // Plain markdown on disk, one file per agent, so the operator can
            // read and edit an agent's memory without going through the app.
            let workspace_dir = data_dir.join("workspace");

            let store = Store::open(&db_path)?;
            let app_config = config::load(&config_path)?;
            let sink = Arc::new(TauriSink { app: app.handle().clone() });

            let runtime = Runtime::with_handle(
                handle.clone(),
                store,
                LlmClient::new()?,
                app_config,
                Workspace::new(workspace_dir.clone()),
                sink,
            );
            let started = runtime.start_all()?;
            // Agents keep their own appointments.
            runtime.start_scheduler();

            // The viewer for agents' computers. Loopback only: it holds the
            // tokens that reach a running machine.
            let viewer_port = tauri::async_runtime::block_on(proxy::start(runtime.store().clone()))
                .map_err(|e| format!("could not start the computer viewer: {e}"))?;
            runtime.set_viewer_port(viewer_port);

            // Anything this app left running that no agent still refers to is
            // released, since a forgotten sandbox bills exactly like a used one.
            {
                let runtime = runtime.clone();
                tauri::async_runtime::spawn(async move {
                    match runtime.sweep_computers().await {
                        Ok(0) => {}
                        Ok(n) => tracing::info!(released = n, "released orphaned sandboxes"),
                        Err(err) => tracing::warn!(%err, "could not sweep sandboxes"),
                    }
                });
            }

            tracing::info!(
                db = %db_path.display(),
                config = %config_path.display(),
                workspace = %workspace_dir.display(),
                agents = started,
                "guac ready"
            );

            app.manage(AppState { runtime, config_path });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::agent_computer,
            commands::start_agent_computer,
            commands::stop_agent_computer,
            commands::delete_agent_computer,
            commands::list_groups,
            commands::create_group,
            commands::update_group,
            commands::delete_group,
            commands::list_agents,
            commands::create_agent,
            commands::update_agent,
            commands::delete_agent,
            commands::set_agent_paused,
            commands::agent_activity,
            commands::agent_last_active,
            commands::agent_notes,
            commands::set_agent_notes,
            commands::channel_messages,
            commands::conversation_flow,
            commands::send_message,
            commands::clear_channel,
            commands::get_settings,
            commands::update_settings,
            commands::test_connection,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Guac");

    drop(tokio_runtime);
}
