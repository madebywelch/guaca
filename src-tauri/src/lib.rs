pub mod account;
/// The desktop host. One of the two files that know Tauri exists.
#[cfg(feature = "desktop")]
pub mod app;
pub mod artifact;
pub mod boot;
pub mod cdp;
pub mod coding;
pub mod commands;
pub mod config;
pub mod db;
pub mod domain;
pub mod e2b;
pub mod eval;
pub mod files;
pub mod ipc;
pub mod kernel;
pub mod llm;
pub mod mcp;
pub mod menubar;
pub mod oauth;
pub mod plugins;
pub mod programs;
pub mod proxy;
pub mod repo;
pub mod runtime;
/// The server host: the same runtime, reached over HTTP and a socket.
#[cfg(feature = "server")]
pub mod server;
pub mod shell;
pub mod subscription;
pub mod trajectory;
/// The other one.
#[cfg(feature = "desktop")]
pub mod tray;
pub mod workspace;

#[cfg(feature = "desktop")]
pub use app::run;
