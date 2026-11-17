use std::path::PathBuf;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("GUAC_LOG")
                .unwrap_or_else(|_| "guac=info,warn".into()),
        )
        .init();

    tracing::info!(
        zone = guac_lib::host::zone().as_deref().unwrap_or("unknown"),
        offset = %chrono::Local::now().offset(),
        "keeping local time"
    );

    let root =
        PathBuf::from(std::env::var("GUACA_ROOT").unwrap_or_else(|_| "/var/lib/guaca".to_string()));
    let bind = std::env::var("GUACA_BIND").unwrap_or_else(|_| "127.0.0.1:8787".to_string());
    let bind = match bind.parse() {
        Ok(bind) => bind,
        Err(err) => {
            tracing::error!(%bind, %err, "GUACA_BIND is not an address to listen on");
            std::process::exit(2);
        }
    };

    let token = match token_for(&root) {
        Ok(token) => token,
        Err(err) => {
            tracing::error!(%err, "could not settle on a token");
            std::process::exit(1);
        }
    };

    let settings = guac_lib::server::Settings {
        root,
        bind,
        token,
        web: std::env::var_os("GUACA_WEB").map(PathBuf::from),
        origin: std::env::var("GUACA_ORIGIN")
            .ok()
            .map(|o| o.trim().to_string())
            .filter(|o| !o.is_empty()),
        updater: std::env::var_os("GUACA_UPDATER_SOCKET")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from),
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("guac-agent")
        .build()
        .expect("failed to start the async runtime");

    if let Err(err) = runtime.block_on(guac_lib::server::serve(settings)) {
        tracing::error!(%err, "guacad stopped");
        std::process::exit(1);
    }
}

fn token_for(root: &std::path::Path) -> Result<String, String> {
    if let Ok(token) = std::env::var("GUACA_TOKEN") {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }

    let dir = root.join("config");
    std::fs::create_dir_all(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let path = dir.join("token");

    if let Ok(existing) = std::fs::read_to_string(&path) {
        let existing = existing.trim().to_string();
        if !existing.is_empty() {
            return Ok(existing);
        }
    }

    let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    write_private(&path, &token)?;
    tracing::warn!(
        path = %path.display(),
        token = %token,
        "generated a token for this workspace; it is in this log once and in that file"
    );
    Ok(token)
}

fn write_private(path: &std::path::Path, contents: &str) -> Result<(), String> {
    use std::io::Write;

    let temp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp).map_err(|err| format!("{}: {err}", temp.display()))?;
    file.write_all(contents.as_bytes()).map_err(|err| format!("{}: {err}", temp.display()))?;
    file.sync_all().map_err(|err| format!("{}: {err}", temp.display()))?;
    drop(file);
    std::fs::rename(&temp, path).map_err(|err| format!("{}: {err}", path.display()))
}
