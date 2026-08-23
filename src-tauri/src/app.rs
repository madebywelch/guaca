//! Tauri application wiring.
//!
//! One of the two files that know Tauri exists, the other being `tray.rs`.
//! Everything below them is a plain Rust library with plain tests, which is why
//! the cascade tests can drive the real runtime without a window.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use tauri::http::{Request, Response};
use tauri::{Emitter, Manager, UriSchemeContext};

use crate::account::Account;
use crate::commands::{self, AppState};
use crate::config;
use crate::db::Store;
use crate::files::FileStore;
use crate::llm::openrouter::LlmClient;
use crate::proxy;
use crate::runtime::events::{EventSink, UiEvent, CHANNEL};
use crate::runtime::Runtime;
use crate::subscription::Subscription;
use crate::tray::{self, Tray};
use crate::workspace::Workspace;

/// Bridges runtime events onto the webview's event bus, and onto the menu bar.
///
/// Two subscribers rather than one, because the two are not alternatives: the
/// window can be closed for a week while routines fire, and the strip is what
/// is left saying so. Neither can be a call the other has to make.
struct TauriSink {
    app: tauri::AppHandle,
    /// Filled once the menu bar exists.
    ///
    /// The sink has to be handed to the runtime before the runtime exists to be
    /// given to the strip, so one of the two has to be late. This one, because
    /// an event that arrives in the gap is one the strip would have redrawn
    /// itself for anyway on the next: it reads the world rather than adding
    /// events up.
    tray: Arc<OnceLock<Arc<Tray>>>,
}

impl EventSink for TauriSink {
    fn emit(&self, event: UiEvent) {
        if let Some(tray) = self.tray.get() {
            tray.observe(&event);
        }
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
    let own_origin = app_origin(context.app_handle().config());

    // A request can in principle arrive before `setup` has managed the state,
    // and a window that asks too early should be told to come back rather than
    // take the app down with it.
    let Some(state) = context.app_handle().try_state::<AppState>() else {
        return refuse(&own_origin, 503, "The file store is not open yet.");
    };

    file_response(state.runtime.files(), &request, &own_origin)
}

/// One stored file, or the reason there is not one.
///
/// Every answer carries `access-control-allow-origin`, because a response on a
/// custom scheme is cross-origin to the page that asked for it and WebKit checks
/// it. Without the header a `fetch` rejects with `TypeError: Load failed`,
/// which is what an operator reads in place of a document, and a refusal cannot
/// even say which refusal it was: the status is unreadable too, so the three
/// sentences `whyNot` exists to tell apart all arrive as the same one. An `img`
/// is exempt from the check, which is why a picture drew and nothing else did.
///
/// It names the origin rather than allowing any, and refuses a page that is not
/// this app's own outright. This webview also holds a cross-origin frame showing
/// an agent's browser, and a wildcard would let script in that frame read any
/// file whose digest it could name.
fn file_response(
    files: &FileStore,
    request: &Request<Vec<u8>>,
    own_origin: &str,
) -> Response<Vec<u8>> {
    // Sent on a `fetch` and absent on an `img` or a frame load. Absent is not
    // suspicious: WebKit omits it exactly where it also does not police the
    // answer, and the header below is then ignored rather than needed.
    let asking = request.headers().get("origin").and_then(|value| value.to_str().ok());
    if let Some(asking) = asking.filter(|asking| *asking != own_origin) {
        tracing::warn!(asking, own_origin, "refused a file to a page that is not this app's");
        return refuse(own_origin, 403, "This file is not readable from here.");
    }

    let range = request.headers().get("range").and_then(|value| value.to_str().ok());
    let target = request.uri().path().trim_start_matches('/');
    match files.serve(target, range) {
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
                // The same header whether or not the request carried an origin,
                // so the one cached answer is valid for either. A response that
                // varied on the request would let an `img` fill the cache with
                // a copy a later `fetch` of the same file cannot read.
                .header("access-control-allow-origin", own_origin)
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
            refuse(own_origin, 404, "No file here with that content.")
        }
    }
}

fn refuse(own_origin: &str, status: u16, why: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .header("access-control-allow-origin", own_origin)
        .body(why.as_bytes().to_vec())
        .expect("a refusal is a valid response")
}

/// The origin the app's own page is served from.
///
/// Dev serves it off Vite and a bundle off Tauri's own scheme. Both are the
/// value Tauri itself puts in `Access-Control-Allow-Origin` for the protocols it
/// registers, and both are the `Origin` WebKit then sends back here, so this has
/// to keep agreeing with `Manager::get_app_url`.
fn app_origin(config: &tauri::Config) -> String {
    match tauri::is_dev().then_some(config.build.dev_url.as_ref()).flatten() {
        Some(dev) => dev.origin().ascii_serialization(),
        None if cfg!(windows) => "http://tauri.localhost".to_string(),
        None => "tauri://localhost".to_string(),
    }
}

/// Closing the window puts Guaca in the menu bar instead of ending it.
///
/// Tauri exits when the last window closes, and for this app that is the wrong
/// default: agents keep their own appointments, so quitting on a close means a
/// routine set for every morning stops firing the first time somebody tidies
/// their screen, with nothing said. A hidden window is not a closed one, so
/// preventing the close is the whole mechanism and no exit handling goes with
/// it. Command-Q and the strip's own Quit still quit.
///
/// Conditional on the strip being there, and that is the point rather than
/// caution: an app with no window and no menu bar icon is one the operator
/// cannot see, cannot reach and cannot stop. If the tray did not build, closing
/// the window quits exactly as it used to.
fn hide_rather_than_quit(window: &tauri::Window, event: &tauri::WindowEvent) {
    let tauri::WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };
    if window.app_handle().tray_by_id(tray::TRAY_ID).is_none() {
        return;
    }
    api.prevent_close();
    if let Err(err) = window.hide() {
        tracing::warn!(%err, "could not hide the window, so it closes instead");
        return;
    }
    tracing::info!("window closed; Guaca is still running in the menu bar");
}

/// The Guaca account store, pointed at the service this build ships with.
///
/// `GUACA_ACCOUNT_ORIGIN` moves it, and exists so the sign-in can be run end to
/// end against a Worker on this machine. It is an environment variable rather
/// than a setting on purpose, and `account.rs` says why: a sign-in service an
/// operator can type into a box is a credential sent somewhere nobody chose.
/// `Account` refuses anything that is neither HTTPS nor loopback, and an
/// override is logged at startup so a machine left pointed at a development
/// service is not a silent state.
fn account_store(path: PathBuf) -> Account {
    match std::env::var("GUACA_ACCOUNT_ORIGIN") {
        Ok(origin) if !origin.trim().is_empty() => {
            let origin = origin.trim().to_string();
            tracing::warn!(%origin, "signing in to a Guaca account somewhere other than the default");
            Account::open_at(path, origin)
        }
        _ => Account::open(path),
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
        .plugin(tauri_plugin_notification::init())
        .register_uri_scheme_protocol(FILE_SCHEME, serve_file)
        .on_window_event(hide_rather_than_quit)
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
            // The ChatGPT sign-in, beside the settings rather than inside them.
            // `subscription.rs` says why: the two files have different writers
            // and one of them writes in the background.
            let subscription = Arc::new(Subscription::open(config_dir.join("subscription.json")));
            // The Guaca account, in its own file for the same reason. Optional,
            // and an install that never signs in never talks to the service.
            let account = Arc::new(account_store(config_dir.join("account.json")));
            let menubar = Arc::new(OnceLock::new());
            let sink = Arc::new(TauriSink { app: app.handle().clone(), tray: menubar.clone() });

            let runtime = Runtime::with_handle(
                handle.clone(),
                store,
                LlmClient::new()?.with_subscription(subscription.clone()),
                app_config,
                Workspace::new(workspace_dir.clone()),
                FileStore::new(files_dir),
                sink,
            );

            // Before anything is started, so the first thing an agent does on
            // the way up has somewhere to be shown. A menu bar that will not
            // build is logged and lived without: the window is the record and
            // the strip is the copy, and taking the app down over the copy
            // would be the wrong way round.
            match Tray::install(app.handle(), runtime.clone()) {
                Ok(tray) => {
                    let _ = menubar.set(tray);
                }
                Err(err) => tracing::warn!(%err, "no menu bar presence this session"),
            }

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

            // And the same for browsers, which are a separate provider with a
            // separate bill: an account can have one configured and not the
            // other, so a single sweep would leave whichever half it skipped.
            {
                let runtime = runtime.clone();
                tauri::async_runtime::spawn(async move {
                    match runtime.sweep_browsers().await {
                        Ok(0) => tracing::debug!("swept: no orphaned browsers"),
                        Ok(n) => tracing::info!(released = n, "released orphaned browsers"),
                        Err(err) => tracing::warn!(%err, "could not sweep browsers"),
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

            app.manage(AppState { runtime, config_path, downloads, subscription, account });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::agent_computer,
            commands::give_agent_computer,
            commands::take_agent_computer,
            commands::start_agent_computer,
            commands::stop_agent_computer,
            commands::delete_agent_computer,
            commands::agent_browser,
            commands::give_agent_browser,
            commands::take_agent_browser,
            commands::start_agent_browser,
            commands::stop_agent_browser,
            commands::group_connectors,
            commands::create_connector,
            commands::delete_connector,
            commands::plugin_catalogue,
            commands::group_plugins,
            commands::set_plugin_access,
            commands::set_plugin_tool,
            commands::connect_plugin,
            commands::disconnect_plugin,
            commands::scan_agent_signins,
            commands::agent_signins,
            commands::list_groups,
            commands::create_group,
            commands::update_group,
            commands::test_group_connection,
            commands::delete_group,
            commands::disband_group,
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
            commands::stop_run,
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
            commands::account_status,
            commands::sign_in_account,
            commands::account_connectors,
            commands::sign_out_account,
            commands::subscription_status,
            commands::begin_subscription_signin,
            commands::complete_subscription_signin,
            commands::sign_out_subscription,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Guac");

    drop(tokio_runtime);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The origin the webview asks from while `pnpm app` is running.
    const DEV: &str = "http://localhost:1420";

    fn asked(origin: Option<&str>, range: Option<&str>, target: &str) -> Request<Vec<u8>> {
        let mut builder = Request::builder().uri(format!("guacfile://localhost/{target}"));
        if let Some(origin) = origin {
            builder = builder.header("origin", origin);
        }
        if let Some(range) = range {
            builder = builder.header("range", range);
        }
        builder.body(Vec::new()).unwrap()
    }

    fn store() -> (FileStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (FileStore::new(dir.path().join("files")), dir)
    }

    /// A document the operator can read, rather than one they have to download.
    ///
    /// Every preview but a picture reaches the file store through `fetch`, and a
    /// `fetch` whose answer does not name an allowed origin rejects before the
    /// caller sees a status. This is the header that was missing, and its
    /// absence turned every markdown brief an agent wrote into a widget saying
    /// "Load failed".
    #[test]
    fn a_file_the_app_asks_for_can_be_read_by_the_app() {
        let (files, _dir) = store();
        let brief = files.put("brief.md", b"# Findings\n\nThe map is drawn.\n").unwrap();

        let response = file_response(
            &files,
            &asked(Some(DEV), None, &format!("{}/brief.md", brief.digest)),
            DEV,
        );

        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["access-control-allow-origin"], DEV);
        assert_eq!(response.headers()["content-type"], "text/markdown");
        assert_eq!(response.body(), b"# Findings\n\nThe map is drawn.\n");
    }

    /// The snippet under a message asks for the first bytes and nothing more.
    #[test]
    fn the_front_of_a_file_is_allowed_too() {
        let (files, _dir) = store();
        let log = files.put("run.txt", b"0123456789").unwrap();

        let response = file_response(
            &files,
            &asked(Some(DEV), Some("bytes=0-3"), &format!("{}/run.txt", log.digest)),
            DEV,
        );

        assert_eq!(response.status(), 206);
        assert_eq!(response.headers()["access-control-allow-origin"], DEV);
        assert_eq!(response.body(), b"0123");
    }

    /// An `img` sends no origin, and is answered anyway.
    #[test]
    fn a_request_with_no_origin_is_still_answered() {
        let (files, _dir) = store();
        let shot = files.put("screen.png", b"\x89PNG not really").unwrap();

        let response =
            file_response(&files, &asked(None, None, &format!("{}/screen.png", shot.digest)), DEV);

        assert_eq!(response.status(), 200);
        assert_eq!(response.body(), b"\x89PNG not really");
    }

    /// The frame showing an agent's browser is another origin in this webview.
    #[test]
    fn a_page_that_is_not_this_app_is_refused_the_bytes() {
        let (files, _dir) = store();
        let brief = files.put("brief.md", b"# Findings\n").unwrap();

        let response = file_response(
            &files,
            &asked(
                Some("https://sessions.onkernel.com:8443"),
                None,
                &format!("{}/brief.md", brief.digest),
            ),
            DEV,
        );

        assert_eq!(response.status(), 403);
        assert!(!response.body().windows(8).any(|w| w == b"Findings"), "the bytes stay here");
    }

    /// A refusal has to be readable, or it is the same "Load failed" again and
    /// the operator cannot tell a missing file from a store that was not open.
    #[test]
    fn a_refusal_says_which_refusal_it_was() {
        let (files, _dir) = store();
        let missing = "0".repeat(64);

        let response =
            file_response(&files, &asked(Some(DEV), None, &format!("{missing}/gone.md")), DEV);

        assert_eq!(response.status(), 404);
        assert_eq!(response.headers()["access-control-allow-origin"], DEV);

        let opening = refuse(DEV, 503, "The file store is not open yet.");
        assert_eq!(opening.headers()["access-control-allow-origin"], DEV);
    }

    #[test]
    fn the_allowed_origin_is_where_the_page_came_from() {
        let mut config = tauri::Config::default();
        config.build.dev_url = Some("http://localhost:1420/".parse().unwrap());

        assert_eq!(
            app_origin(&config),
            if tauri::is_dev() { DEV } else { "tauri://localhost" },
            "the dev server while `pnpm app` runs, Tauri's own scheme in a bundle"
        );
    }
}
