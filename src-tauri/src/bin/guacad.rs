//! The Guaca daemon: one workspace, on a machine that stays awake.
//!
//! Everything it needs comes from the environment, because it is started by
//! systemd or a container runtime rather than by a person with a terminal. A
//! flag nobody can see is a flag nobody sets correctly.
//!
//!   GUACA_ROOT     where the workspace lives. Default `/var/lib/guaca`.
//!   GUACA_BIND     what to listen on. Default `127.0.0.1:8787`, which is
//!                  loopback on purpose: a box is reached through a tunnel, and
//!                  a default that binds every interface is one operator's
//!                  firewall away from a public workspace.
//!   GUACA_TOKEN    the bearer token. Generated and written beside the settings
//!                  when it is absent, so a first run needs nothing prepared.
//!   GUACA_WEB      the frontend bundle to serve at `/`. Absent serves no page,
//!                  which is right for a box the desktop app connects to.
//!   GUACA_ORIGIN   the address a browser reaches this box at, e.g. the
//!                  tunnel's https URL. Only a sign-in needs it, for the
//!                  redirect; absent, the origin of the last call is used.
//!   ANTHROPIC_API_KEY  not Guaca's own setting, but read for one decision:
//!                  with it set, Claude Code is offered as a coding harness
//!                  and spends the key.
//!   GUAC_LOG       the tracing filter, spelled as the desktop app spells it.

use std::path::PathBuf;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("GUAC_LOG")
                .unwrap_or_else(|_| "guac=info,warn".into()),
        )
        .init();

    let root =
        PathBuf::from(std::env::var("GUACA_ROOT").unwrap_or_else(|_| "/var/lib/guaca".to_string()));
    let bind = std::env::var("GUACA_BIND").unwrap_or_else(|_| "127.0.0.1:8787".to_string());
    let bind = match bind.parse() {
        Ok(bind) => bind,
        Err(err) => {
            // Said rather than defaulted. A daemon that silently listens
            // somewhere other than where it was told is one nobody can reach
            // and nobody can diagnose.
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
        claude_key: std::env::var("ANTHROPIC_API_KEY").is_ok_and(|key| !key.trim().is_empty()),
        origin: std::env::var("GUACA_ORIGIN")
            .ok()
            .map(|o| o.trim().to_string())
            .filter(|o| !o.is_empty()),
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

/// The token this workspace answers to.
///
/// Read from the environment, else from the file a previous run wrote, else
/// generated. Generating is what makes a first run need nothing prepared: an
/// operator who has just installed this has a working token before they have
/// read any documentation, and it is in the logs where they are already looking.
///
/// It is not in `config.json`. That file is rewritten wholesale whenever the
/// operator presses Save in the app, and a credential in it would be one a
/// settings change could drop. The same argument `subscription.rs` makes.
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

    // Two v4 UUIDs with the hyphens taken out: 256 bits from the same generator
    // every id in this app already comes from. Not a password, and never typed
    // by a person, so it is long rather than memorable.
    let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    write_private(&path, &token)?;
    tracing::warn!(
        path = %path.display(),
        token = %token,
        "generated a token for this workspace; it is in this log once and in that file"
    );
    Ok(token)
}

/// Writes a credential so only this user can read it.
///
/// Temp-then-rename, and the mode is set before the bytes are written rather
/// than after: a file created world-readable and tightened a moment later is
/// readable for that moment, and on a shared box that moment is the whole of
/// the vulnerability.
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
