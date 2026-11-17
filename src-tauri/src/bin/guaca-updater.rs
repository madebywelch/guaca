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
