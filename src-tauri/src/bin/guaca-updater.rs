//! The process beside a box's host that can replace it. `updater.rs` says why
//! it is a second container and what it may be asked.
//!
//!   guaca-updater                 serve: start the host, answer its socket
//!   guaca-updater install IMAGE   make the updater container from IMAGE
//!
//! `install` is what `deploy/box/install.sh` runs once, from a throwaway
//! container, and what a serving updater runs on itself after an update. The
//! box is configured by the environment `install` is given, and it carries
//! that to the updater and the updater to the host:
//!
//!   GUACA_PORT     the loopback port the host is published on. Default 8787.
//!   GUACA_VOLUME   the workspace volume. Default `guacad-data`; name an
//!                  existing one to adopt a workspace.
//!   GUACA_CHANNEL  `release`, the default, or `main`: which builds an update
//!                  installs. Fixed here; nothing the host sends changes it.
//!   and every variable guacad reads, listed in `updater::HOST_ENV`.

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("GUAC_LOG")
                .unwrap_or_else(|_| "guac=info,warn".into()),
        )
        .init();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to start the async runtime");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => runtime.block_on(serve()),
        ["install", image] => {
            runtime.block_on(guac_lib::updater::install(&guac_lib::host::docker_binary(), image))
        }
        _ => Err("usage: guaca-updater [install IMAGE]".into()),
    };
    if let Err(error) = result {
        tracing::error!(%error, "guaca-updater stopped");
        std::process::exit(1);
    }
}

#[cfg(unix)]
async fn serve() -> Result<(), String> {
    guac_lib::updater::serve().await
}

#[cfg(not(unix))]
async fn serve() -> Result<(), String> {
    Err("guaca-updater runs on Linux, beside a box's Docker.".into())
}
