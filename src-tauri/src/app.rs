//! Tauri application wiring.
//!
//! The only file that knows Tauri exists. Everything below it is a plain Rust
//! library with plain tests, which is why the cascade tests can drive the real
//! runtime without a window.

use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::commands::{self, AppState};
use crate::config;
use crate::daytona;
use crate::db::Store;
use crate::llm::openrouter::LlmClient;
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
        // Everything an agent's computer is made of comes through here.
        //
        // Daytona puts an "are you sure" interstitial in front of preview URLs
        // and serves it for every request, not just the first document, so
        // noVNC's own stylesheet and scripts come back as copies of the warning
        // page and the desktop renders as unstyled text. The only documented way
        // past it is a request header, and an iframe cannot set one. So the
        // webview asks us, and we ask Daytona with the header attached.
        .register_asynchronous_uri_scheme_protocol(
            daytona::COMPUTER_SCHEME,
            move |app, request, responder| {
                let app = app.app_handle().clone();
                let uri = request.uri().clone();

                tauri::async_runtime::spawn(async move {
                    responder.respond(proxy_computer_request(&app, &uri).await);
                });
            },
        )
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

/// Forwards one `guaccomputer://localhost/{sandbox}/{port}/{path}` request to
/// the sandbox it names.
///
/// Failures are answered as a readable page rather than a dropped connection:
/// an iframe that receives nothing shows a blank rectangle, which is
/// indistinguishable from a computer that is simply asleep.
async fn proxy_computer_request(
    app: &tauri::AppHandle,
    uri: &tauri::http::Uri,
) -> tauri::http::Response<Vec<u8>> {
    let Some((sandbox, port, path)) = split_computer_path(uri.path()) else {
        return plain(400, "Not a computer address.");
    };

    let Some(state) = app.try_state::<AppState>() else {
        return plain(503, "Guac is still starting.");
    };
    let Some(client) = daytona::DaytonaClient::new(&state.runtime.config().daytona.api_key) else {
        return plain(503, "No Daytona API key is set.");
    };

    match client.proxy_get(&sandbox, port, &path, uri.query()).await {
        Ok(file) => tauri::http::Response::builder()
            .status(file.status)
            .header("content-type", file.content_type)
            // The document loads from this scheme but talks to Daytona directly
            // over a WebSocket, so it has to be allowed to.
            .header(
                "content-security-policy",
                "default-src 'self' 'unsafe-inline' 'unsafe-eval' data: blob:; \
                 connect-src 'self' wss://*.daytonaproxy01.net https://*.daytonaproxy01.net",
            )
            .body(file.body)
            .unwrap_or_else(|_| plain(500, "Could not build the reply.")),
        Err(err) => plain(502, &format!("Could not reach that computer: {err}")),
    }
}

/// `/{sandbox}/{port}/{rest...}` -> the three parts, or nothing if it is not that.
fn split_computer_path(path: &str) -> Option<(String, u16, String)> {
    let mut parts = path.trim_start_matches('/').splitn(3, '/');
    let sandbox = parts.next().filter(|s| !s.is_empty())?;
    let port: u16 = parts.next()?.parse().ok()?;
    // An empty tail is the port's index page, which is how the terminal loads.
    let rest = parts.next().unwrap_or("");
    Some((sandbox.to_string(), port, rest.to_string()))
}

fn plain(status: u16, message: &str) -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(message.as_bytes().to_vec())
        .expect("a plain text reply always builds")
}

#[cfg(test)]
mod tests {
    use super::split_computer_path;

    #[test]
    fn a_computer_address_splits_into_sandbox_port_and_file() {
        let (sandbox, port, path) = split_computer_path("/abc-123/6080/app/ui.js").unwrap();
        assert_eq!(sandbox, "abc-123");
        assert_eq!(port, 6080);
        assert_eq!(path, "app/ui.js", "the tail keeps its slashes");
    }

    #[test]
    fn an_empty_tail_is_the_index_page() {
        // The web terminal is served from the port root, so this is not an
        // edge case: it is how one of the two views loads.
        let (_, port, path) = split_computer_path("/abc/22222/").unwrap();
        assert_eq!(port, 22222);
        assert_eq!(path, "");
    }

    #[test]
    fn anything_that_is_not_a_computer_address_is_refused() {
        assert!(split_computer_path("/").is_none());
        assert!(split_computer_path("/abc").is_none());
        assert!(split_computer_path("/abc/not-a-port/x").is_none());
    }
}
