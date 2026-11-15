pub mod account;
/// The desktop host. One of the two files that know Tauri exists.
#[cfg(feature = "desktop")]
pub mod app;
pub mod artifact;
pub mod boot;
pub mod cdp;
pub mod claude_signin;
pub mod commands;
pub mod config;
pub mod db;
pub mod domain;
pub mod e2b;
pub mod eval;
pub mod files;
pub mod host;
pub mod ipc;
pub mod kernel;
pub mod llm;
pub mod mcp;
pub mod menubar;
pub mod notebook;
/// Whether macOS will show what a window raises. Only a window raises anything.
#[cfg(feature = "desktop")]
pub mod notifications;
pub mod oauth;
pub mod pickup;
pub mod plugins;
/// A control channel into a test build. Its protocol is compiled into the unit
/// tests everywhere; the socket only into a build with the `probe` feature.
#[cfg(any(test, feature = "probe"))]
pub mod probe;
pub mod programs;
pub mod proxy;
pub mod push;
/// A source build rebuilding itself. Only a window has an app to replace.
#[cfg(feature = "desktop")]
pub mod rebuild;
pub mod runtime;
pub mod secrets;
/// The server host: the same runtime, reached over HTTP and a socket.
#[cfg(feature = "server")]
pub mod server;
pub mod skills;
pub mod skills_sh;
pub mod subscription;
#[cfg(all(test, unix))]
mod testing;
pub mod trajectory;
pub mod transfer;
/// The other one.
#[cfg(feature = "desktop")]
pub mod tray;
#[cfg(feature = "server")]
pub mod updater;
pub mod updates;
pub mod webhook;
pub mod workspace;

#[cfg(feature = "desktop")]
pub use app::run;
