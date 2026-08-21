//! Tauri application wiring.
//!
//! The only file that knows Tauri exists. Everything below it is a plain Rust
//! library with plain tests, which is why the cascade tests can drive the real
//! runtime without a window.

use std::sync::Arc;

use tauri::http::{Request, Response};
use tauri::{Emitter, Manager, UriSchemeContext};

use crate::commands::{self, AppState};
use crate::config;
use crate::db::Store;
use crate::files::FileStore;
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

/// The scheme a preview is addressed on: `guacfile://localhost/{digest}/{name}`.
///
/// Its own scheme rather than a command returning bytes. A transcript crosses
/// IPC in bulk, forty messages into a prompt and hundreds into the activity
/// view, which is the whole reason a document is a digest in an envelope rather
/// than the document; handing those same bytes back over the same channel to
/// draw a thumbnail would give it up. A URL is fetched once, by one element,
/// only while it is on screen, and the webview caches and ranges it.
///
/// It is also narrower than the asset protocol, which would open a scoped part
/// of the disk. Nothing is addressable here but a digest this app stored.
const FILE_SCHEME: &str = "guacfile";

/// Serves one stored file to the webview.
fn serve_file(
    context: UriSchemeContext<'_, tauri::Wry>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let refuse = |status: u16, why: &str| {
        Response::builder()
            .status(status)
            .header("content-type", "text/plain; charset=utf-8")
            .body(why.as_bytes().to_vec())
            .expect("a refusal is a valid response")
    };

    // A request can in principle arrive before `setup` has managed the state,
    // and a window that asks too early should be told to come back rather than
    // take the app down with it.
    let Some(state) = context.app_handle().try_state::<AppState>() else {
        return refuse(503, "The file store is not open yet.");
    };

    let range = request.headers().get("range").and_then(|value| value.to_str().ok());
    let target = request.uri().path().trim_start_matches('/');
    match state.runtime.files().serve(target, range) {
        Ok(served) => {
            // A preview that comes up blank is either a request that never
            // happened, which means the CSP, or one that was answered wrongly.
            // The two look identical on screen and nowhere else.
            tracing::debug!(
                target,
                range,
                served.status,
                bytes = served.body.len(),
                "served a file"
            );
            let mut response = Response::builder()
                .status(served.status)
                .header("content-type", served.mime)
                // Said even on a whole-file answer: a PDF viewer asks for the
                // tail first, and one that is told ranges are unavailable
                // re-reads the document from the start for every page.
                .header("accept-ranges", "bytes")
                // The bytes are on this machine already and addressed by their
                // own content, so nothing they could be revalidated against
                // will ever have changed.
                .header("cache-control", "private, max-age=31536000, immutable");
            if let Some(content_range) = served.content_range {
                response = response.header("content-range", content_range);
            }
            response.body(served.body).expect("a stored file is a valid response")
        }
        Err(err) => {
            tracing::debug!(%err, target, "refused a file the webview asked for");
            refuse(404, "No file here with that content.")
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
        .register_uri_scheme_protocol(FILE_SCHEME, serve_file)
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
            // Attachments, addressed by content and shared by every agent. Kept
            // beside the memories rather than in SQLite: a proposal document
            // does not belong in a table that is read forty rows at a time.
            let files_dir = data_dir.join("files");

            let store = Store::open(&db_path)?;
            // A permission request is answered by a turn that is holding the
            // line for it, and nothing holds a line across a restart. Anything
            // still pending here is waiting on an agent that no longer exists,
            // so it is closed rather than left drawing live buttons.
            match store.expire_pending_approvals() {
                Ok(0) => {}
                Ok(n) => {
                    tracing::info!(expired = n, "closed permission requests left by a restart")
                }
                Err(err) => tracing::warn!(%err, "could not close stale permission requests"),
            }
            let app_config = config::load(&config_path)?;
            let sink = Arc::new(TauriSink { app: app.handle().clone() });

            let runtime = Runtime::with_handle(
                handle.clone(),
                store,
                LlmClient::new()?,
                app_config,
                Workspace::new(workspace_dir.clone()),
                FileStore::new(files_dir),
                sink,
            );
            let started = runtime.start_all()?;
            // Agents keep their own appointments.
            runtime.start_scheduler();
            // And find out what their browsers are already signed in to, so the
            // roster is right before anybody asks rather than after.
            runtime.start_signin_sweep();

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
                        // Said even when it is nothing, because "no orphans" and
                        // "the sweep never ran" look identical from the outside
                        // and only one of them is fine.
                        Ok(0) => tracing::debug!("swept: no orphaned sandboxes"),
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

            // Where a saved attachment lands. The operating system's own
            // downloads folder is the one place a person already knows to look;
            // the home directory is the fallback for a machine that has no such
            // folder, and the app's own data directory is where a copy goes
            // rather than nowhere.
            let downloads = app
                .path()
                .download_dir()
                .or_else(|_| app.path().home_dir())
                .unwrap_or_else(|_| data_dir.clone());

            app.manage(AppState { runtime, config_path, downloads });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::agent_computer,
            commands::start_agent_computer,
            commands::stop_agent_computer,
            commands::delete_agent_computer,
            commands::group_connectors,
            commands::create_connector,
            commands::delete_connector,
            commands::scan_agent_signins,
            commands::agent_signins,
            commands::list_groups,
            commands::create_group,
            commands::update_group,
            commands::delete_group,
            commands::approval_states,
            commands::decide_approval,
            commands::agent_grants,
            commands::revoke_grant,
            commands::list_agents,
            commands::create_agent,
            commands::update_agent,
            commands::delete_agent,
            commands::duplicate_agent,
            commands::hire_agents,
            commands::set_agent_paused,
            commands::set_agent_pinned,
            commands::move_agent,
            commands::agent_activity,
            commands::agent_last_active,
            commands::agent_notes,
            commands::set_agent_notes,
            commands::channel_messages,
            commands::pair_messages,
            commands::conversation_flow,
            commands::search,
            commands::stage_files,
            commands::save_file,
            commands::send_message,
            commands::retry_turn,
            commands::clear_channel,
            commands::clear_group,
            commands::agent_routines,
            commands::create_routine,
            commands::update_routine,
            commands::set_routine_active,
            commands::test_routine,
            commands::routine_runs,
            commands::delete_routine,
            commands::usage_summary,
            commands::usage_for_runs,
            commands::get_settings,
            commands::update_settings,
            commands::test_connection,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Guac");

    drop(tokio_runtime);
}
