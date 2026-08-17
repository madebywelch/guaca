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
use crate::files::FileStore;
use crate::llm::openrouter::LlmClient;
use crate::proxy;
use crate::runtime::events::{EventSink, UiEvent, CHANNEL};
use crate::runtime::Runtime;
use crate::workspace::Workspace;

/// How long a quit waits for local machines to stop. Long enough for a
/// handful of them at a couple of seconds each, short of the point where an
/// operator decides the app has hung and force-quits it anyway.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(15);

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
            // A machine on this Mac has no server-side timeout to stop it, so
            // this is what makes its idle period idle time: a touch on the
            // guest's heartbeat while it is used, and a stop when it is not.
            runtime.computers().start_idle_ticker(handle.clone());

            // The viewer for agents' computers. Loopback only: it holds the
            // tokens that reach a running machine.
            let viewer_port =
                tauri::async_runtime::block_on(proxy::start(Arc::new(runtime.computers().clone())))
                    .map_err(|e| format!("could not start the computer viewer: {e}"))?;
            runtime.computers().set_viewer_port(viewer_port);

            // Anything this app left running that nothing still refers to is
            // released, since a forgotten computer bills exactly like a used one.
            {
                let runtime = runtime.clone();
                tauri::async_runtime::spawn(async move {
                    match runtime.sweep_computers().await {
                        // Said even when it is nothing, because "no orphans" and
                        // "the sweep never ran" look identical from the outside
                        // and only one of them is fine.
                        Ok(0) => tracing::debug!("swept: no orphaned computers"),
                        Ok(n) => tracing::info!(released = n, "released orphaned computers"),
                        Err(err) => tracing::warn!(%err, "could not sweep computers"),
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
            commands::set_agent_paused,
            commands::set_agent_pinned,
            commands::agent_activity,
            commands::agent_last_active,
            commands::agent_notes,
            commands::set_agent_notes,
            commands::channel_messages,
            commands::pair_messages,
            commands::conversation_flow,
            commands::search,
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
        .build(tauri::generate_context!())
        .expect("error while running Guac")
        .run(|app, event| {
            // A local machine has no server-side timeout: left running, it
            // holds its VM's memory for a whole idle period after the window
            // is gone. Hosted ones are left to the timeout they already have.
            //
            // Bounded, because this blocks the quit: a runtime that will not
            // answer must not turn closing the app into a beachball, and the
            // guest's own watchdog stops anything this gave up on.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let runtime = app.state::<AppState>().runtime.clone();
                tauri::async_runtime::block_on(async move {
                    let stopping = runtime.computers().stop_local_machines();
                    if tokio::time::timeout(SHUTDOWN_GRACE, stopping).await.is_err() {
                        tracing::warn!(
                            "gave up stopping local computers; their watchdogs will stop them"
                        );
                    }
                });
            }
        });

    drop(tokio_runtime);
}
