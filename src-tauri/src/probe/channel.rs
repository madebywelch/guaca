//! The socket a probe build opens, and what it does with each request.
//!
//! Owner-only and opt-in twice over. A build without the `probe` feature does
//! not contain this file, and a probe build opens nothing unless the launch
//! named a socket in `GUACA_PROBE_SOCKET`. The socket's directory has to be
//! one only this account can enter, the socket itself is made readable by
//! nobody else, and every connection is asked for its account again.

// Optimizations on is the shape of a build someone ships: `tauri build`
// without `--debug`, `install.sh`, the release workflow. Refused here so that
// no flag, profile or script can put a control socket in one.
#[cfg(not(debug_assertions))]
compile_error!(
    "the probe feature is for test builds only; build it with --debug, never for release"
);

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use super::{Envelope, Reply, Request, State, Window, SOCKET_ENV};
use crate::tray::Tray;

/// Set by `refuseNextBanner`, taken by the next banner.
static REFUSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the banner being handed over now is the one the probe asked to be
/// refused. Asking takes the refusal, so exactly one banner is refused.
pub fn refused() -> bool {
    REFUSE.swap(false, std::sync::atomic::Ordering::SeqCst)
}

/// How long a page has to answer. A page that is loading answers as soon as
/// it can; one that has not answered in this long is not going to.
const PAGE: Duration = Duration::from_secs(10);

/// Opens the socket, if this launch named one. Called once the tray exists.
pub fn start(app: &AppHandle) {
    let Some(path) = std::env::var_os(SOCKET_ENV).map(PathBuf::from) else { return };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(err) = serve(app, &path).await {
            tracing::error!(%err, "the probe channel did not open");
        }
    });
}

async fn serve(app: AppHandle, path: &Path) -> Result<(), String> {
    let dir = path.parent().ok_or("the probe socket needs a directory")?;
    let owner = std::fs::metadata(dir).map_err(|err| format!("{}: {err}", dir.display()))?.uid();
    super::private_dir(dir, owner)?;
    if path.exists() {
        return Err(format!("{} already exists; the probe will not replace it", path.display()));
    }
    let listener = UnixListener::bind(path).map_err(|err| err.to_string())?;
    // The socket is this process's own file, so its owner is the account the
    // app runs as, which is the only one the directory may belong to.
    let me = std::fs::metadata(path).map_err(|err| err.to_string())?.uid();
    if me != owner {
        let _ = std::fs::remove_file(path);
        return Err(format!("{} belongs to another account", dir.display()));
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|err| err.to_string())?;
    tracing::info!(socket = %path.display(), "the probe channel is open");
    loop {
        let (stream, _) = listener.accept().await.map_err(|err| err.to_string())?;
        let admitted = stream.peer_cred().is_ok_and(|peer| super::admits(peer.uid(), me));
        if !admitted {
            tracing::warn!("the probe channel turned away a connection from another account");
            continue;
        }
        tauri::async_runtime::spawn(connection(app.clone(), stream));
    }
}

async fn connection(app: AppHandle, stream: UnixStream) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let reply = match super::parse(&line) {
            Ok(Envelope { id, request }) => Reply::to(id, handle(&app, request).await),
            Err(reply) => reply,
        };
        if write.write_all(reply.line().as_bytes()).await.is_err() {
            return;
        }
    }
}

async fn handle(app: &AppHandle, request: Request) -> Result<Value, String> {
    match request {
        Request::Hello => Ok(json!({
            "pid": std::process::id(),
            "identifier": app.config().identifier,
        })),
        Request::State => state(app).await,
        Request::Page { window, call } => page(app, &window, &call).await,
        Request::ClickIcon => {
            let tray = app.try_state::<Arc<Tray>>().ok_or("this launch has no menu bar icon")?;
            let tray = Arc::clone(&tray);
            blocking(move || tray.probe_click()).await?.map(|()| Value::Null)
        }
        Request::Close { window } => {
            let window = app
                .get_webview_window(&window)
                .ok_or_else(|| format!("no window named {window}"))?;
            blocking(move || window.close().map_err(|err| err.to_string()))
                .await?
                .map(|()| Value::Null)
        }
        Request::Hide => {
            let app = app.clone();
            blocking(move || app.hide().map_err(|err| err.to_string())).await?.map(|()| Value::Null)
        }
        Request::Quit => {
            // After the answer is on its way, or the client hears a closed
            // socket and cannot tell a quit from a crash.
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                app.exit(0);
            });
            Ok(Value::Null)
        }
        Request::RefuseNextBanner => {
            REFUSE.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(Value::Null)
        }
        Request::Busy { ms } => {
            occupy(app.clone(), Instant::now() + Duration::from_millis(ms));
            Ok(Value::Null)
        }
    }
}

/// Holds the main thread in 20 ms slices until `until`. Each slice is queued
/// from a thread of its own once the one before it has finished, so between
/// slices the run loop goes back to whatever else is waiting, IPC included,
/// and is idle only for the instant before the next slice wakes it. Queued
/// before the last one finished, slices would be drained in one go and the
/// window would stall instead; queued from the main thread, a slice would run
/// inline and the next would nest inside it.
fn occupy(app: AppHandle, until: Instant) {
    let spawned = std::thread::Builder::new().name("probe-busy".into()).spawn(move || {
        let (finished, finishes) = std::sync::mpsc::channel::<()>();
        while Instant::now() < until {
            let finished = finished.clone();
            let queued = app.run_on_main_thread(move || {
                std::thread::sleep(Duration::from_millis(20));
                let _ = finished.send(());
            });
            if let Err(err) = queued {
                tracing::warn!(%err, "could not keep the main thread busy");
                return;
            }
            if finishes.recv().is_err() {
                return;
            }
        }
    });
    if let Err(err) = spawned {
        tracing::warn!(%err, "could not start keeping the main thread busy");
    }
}

async fn state(app: &AppHandle) -> Result<Value, String> {
    let notifications = serde_json::to_value(crate::notifications::current().await)
        .map_err(|err| err.to_string())?;
    let app = app.clone();
    blocking(move || {
        let windows = app
            .webview_windows()
            .into_iter()
            .map(|(label, window)| Window {
                label,
                visible: window.is_visible().unwrap_or(false),
                focused: window.is_focused().unwrap_or(false),
                minimized: window.is_minimized().unwrap_or(false),
            })
            .collect();
        let state = State {
            identifier: app.config().identifier.clone(),
            pid: std::process::id(),
            windows,
            notifications,
            menubar: app.try_state::<Arc<Tray>>().map(|tray| tray.probe_view()),
        };
        serde_json::to_value(state).map_err(|err| err.to_string())
    })
    .await?
}

async fn page(app: &AppHandle, label: &str, call: &Value) -> Result<Value, String> {
    let window = app.get_webview_window(label).ok_or_else(|| format!("no window named {label}"))?;
    let (send, answer) = tokio::sync::oneshot::channel::<String>();
    // The webview calls back once; the callback's type allows more.
    let send = Mutex::new(Some(send));
    window
        .eval_with_callback(super::page_script(call), move |reported| {
            if let Some(send) = send.lock().take() {
                let _ = send.send(reported);
            }
        })
        .map_err(|err| err.to_string())?;
    match tokio::time::timeout(PAGE, answer).await {
        Ok(Ok(reported)) => super::page_answer(&reported),
        Ok(Err(_)) => Err(format!("the {label} window dropped the call")),
        Err(_) => Err(format!("the {label} window did not answer in {}s", PAGE.as_secs())),
    }
}

/// Window and tray calls hop to the main thread and wait for it, so they are
/// made off the async pool.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|err| err.to_string())
}
