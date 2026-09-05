//! Opening a workspace, which is the same act in both hosts.
//!
//! A workspace is a database, three settings files and two directories, and
//! bringing one up is the same sequence whether the runtime ends up behind a
//! window or behind a socket: open the store, close whatever a restart
//! stranded, read the settings, build the runtime, start the loops that keep
//! their own time, and release anything a previous process left running at a
//! provider.
//!
//! It lives here rather than in `app.rs` because two copies would drift, and
//! the drift would be silent in the worst way: a loop started in one host and
//! forgotten in the other is a workspace where routines never fire, and nothing
//! reports it because nothing is wrong. The desktop is the host that has been
//! shipping, so this is written to be exactly what it already did.
//!
//! What is *not* here is anything either host does alone. A menu bar, a
//! downloads folder and a `guacfile:` scheme are the window's; a listening
//! socket and a bearer token are the daemon's. Both are handed in.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU16;
use std::sync::Arc;

use crate::account::Account;
use crate::artifact::{self, Artifacts};
use crate::config;
use crate::db::Store;
use crate::runtime::events::EventSink;
use crate::runtime::{OnDisk, Runtime};
use crate::subscription::Subscription;

/// Where a workspace keeps itself.
///
/// Two directories rather than one because the desktop app is handed two by the
/// operating system and they are not the same place on macOS. A daemon is given
/// one root and puts them side by side under it, which is the arrangement a
/// volume wants: one thing to snapshot, one thing to restore.
#[derive(Debug, Clone)]
pub struct Paths {
    pub data: PathBuf,
    pub config: PathBuf,
}

impl Paths {
    /// Both under one root, which is what a server is given.
    pub fn under(root: &Path) -> Self {
        Self { data: root.join("data"), config: root.join("config") }
    }

    pub fn db(&self) -> PathBuf {
        self.data.join("guac.db")
    }

    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.json")
    }
}

/// Everything a host needs to serve a workspace.
pub struct Booted {
    pub runtime: Runtime,
    pub subscription: Arc<Subscription>,
    pub account: Arc<Account>,
    pub artifacts: Artifacts,
    pub artifact_port: Arc<AtomicU16>,
    pub config_path: PathBuf,
    /// How many agents came back up. Logged by the host, which knows what to
    /// say about it.
    pub started: usize,
}

/// Opens the workspace at these paths and starts everything that keeps time.
///
/// The `handle` is explicit rather than ambient because the desktop's setup
/// hook runs on the main thread outside any runtime, where `tokio::spawn`
/// panics. A daemon has an ambient one and passes its own.
pub async fn open(
    paths: &Paths,
    handle: tokio::runtime::Handle,
    sink: Arc<dyn EventSink>,
) -> Result<Booted, String> {
    std::fs::create_dir_all(&paths.data)
        .map_err(|err| format!("{}: {err}", paths.data.display()))?;
    std::fs::create_dir_all(&paths.config)
        .map_err(|err| format!("{}: {err}", paths.config.display()))?;

    // One process owns the actors, not just the database. SQLite permits two
    // writers, but two schedulers would perform the same appointment twice.
    let lease = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(paths.data.join("workspace.lock"))
        .map_err(|err| format!("could not open the workspace lock: {err}"))?;
    lease
        .try_lock()
        .map_err(|err| format!("this workspace is already running or cannot be locked: {err}"))?;

    let db_path = paths.db();
    let config_path = paths.config_file();
    // Memories as plain markdown, attachments by content hash, and one git
    // work tree per agent per repository, all under the data directory.
    // `OnDisk::under` is the one place that arrangement is decided.
    let disk = OnDisk::under(&paths.data);
    let workspace_dir = disk.workspace.root().to_path_buf();

    let store =
        Store::open(&db_path).map_err(|err| format!("could not open the workspace: {err}"))?;
    // A permission request is answered by a turn that is holding the line for
    // it, and nothing holds a line across a restart. Anything still pending
    // here is waiting on an agent that no longer exists, so it is closed rather
    // than left drawing live buttons.
    //
    // On a server this stops being the rare case. A container is recycled, a
    // host is drained, a deploy happens, and every one of those lands here.
    match store.expire_pending_approvals() {
        Ok(0) => {}
        Ok(n) => tracing::info!(expired = n, "closed permission requests left by a restart"),
        Err(err) => tracing::warn!(%err, "could not close stale permission requests"),
    }

    let interrupted = store
        .recover_interrupted_runs()
        .map_err(|err| format!("could not recover interrupted conversations: {err}"))?;
    if interrupted > 0 {
        tracing::warn!(interrupted, "conversations interrupted by restart are ready for review");
    }

    let app_config =
        config::load(&config_path).map_err(|err| format!("could not read the settings: {err}"))?;
    // The ChatGPT sign-in, beside the settings rather than inside them.
    // `subscription.rs` says why: the two files have different writers and one
    // of them writes in the background.
    let subscription = Arc::new(Subscription::open(paths.config.join("subscription.json")));
    // The Guaca account, in its own file for the same reason.
    let account = Arc::new(account_store(paths.config.join("account.json")));

    let runtime = Runtime::with_handle(
        handle,
        store,
        crate::llm::openrouter::LlmClient::new()
            .map_err(|err| format!("could not build the model client: {err}"))?
            .with_subscription(subscription.clone()),
        app_config,
        disk,
        sink,
    );

    runtime.hold_workspace_lease(lease);

    let started = runtime.start_all().map_err(|err| format!("could not start the crew: {err}"))?;
    // Agents keep their own appointments.
    runtime.start_scheduler();
    // And find out what their browsers are already signed in to, so the roster
    // is right before anybody asks rather than after.
    runtime.start_signin_sweep();
    // The compost empties itself, whether or not anybody opens it.
    runtime.start_compost();

    // The viewer for agents' computers. Loopback only: it holds the tokens that
    // reach a running machine, and on a server it stays loopback for exactly
    // that reason. What reaches it from outside is the host's business, and the
    // token still never crosses.
    let viewer_port = crate::proxy::start(runtime.store().clone())
        .await
        .map_err(|err| format!("could not start the computer viewer: {err}"))?;
    runtime.set_viewer_port(viewer_port);

    // And the origin a page an agent wrote is allowed to run on, which is
    // separate from the app's for exactly one reason: the app's content policy
    // forbids script and must keep forbidding it.
    let artifacts = Artifacts::new();
    let artifact_port = Arc::new(AtomicU16::new(
        artifact::start(artifacts.clone())
            .await
            .map_err(|err| format!("could not start the artifact viewer: {err}"))?,
    ));

    sweep_providers(&runtime);

    // The account, and where its MCP server is. Both are one write at startup:
    // the Google plugin's server is the operator's own account rather than a
    // vendor's, so a build pointed at a development service has to point that
    // plugin there too or it would sign in to one origin and call another.
    runtime.with_account(account.clone());
    if account.origin() != crate::account::DEFAULT_ORIGIN {
        runtime.plugins_at(std::collections::HashMap::from([(
            crate::domain::plugin::PluginKind::Google.slug().to_string(),
            format!("{}/mcp", account.origin()),
        )]));
    }

    tracing::info!(
        db = %db_path.display(),
        config = %config_path.display(),
        workspace = %workspace_dir.display(),
        agents = started,
        "workspace open"
    );

    Ok(Booted { runtime, subscription, account, artifacts, artifact_port, config_path, started })
}

/// Releases anything a previous process left running at a provider.
///
/// Two sweeps rather than one, because they are separate products on separate
/// bills: an account can have a machine provider configured and not a browser
/// provider, and a single sweep would silently skip whichever half was missing.
/// Both are spawned rather than awaited, because a provider that is having a
/// bad minute must not be what stops a workspace from opening.
fn sweep_providers(runtime: &Runtime) {
    {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            match runtime.sweep_computers().await {
                // Said even when it is nothing, because "no orphans" and "the
                // sweep never ran" look identical from the outside and only one
                // of them is fine.
                Ok(0) => tracing::debug!("swept: no orphaned sandboxes"),
                Ok(n) => tracing::info!(released = n, "released orphaned sandboxes"),
                Err(err) => tracing::warn!(%err, "could not sweep sandboxes"),
            }
        });
    }
    {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            match runtime.sweep_browsers().await {
                Ok(0) => tracing::debug!("swept: no orphaned browsers"),
                Ok(n) => tracing::info!(released = n, "released orphaned browsers"),
                Err(err) => tracing::warn!(%err, "could not sweep browsers"),
            }
        });
    }
}

/// The account store, pointed wherever `GUACA_ACCOUNT_ORIGIN` says.
///
/// An environment variable rather than a setting, for the reason
/// `subscription.rs` gives: a sign-in service an operator can type into a box
/// is a credential sent somewhere nobody chose. An override is logged, because
/// a machine left pointed at a development service must not be a silent state.
pub fn account_store(path: PathBuf) -> Account {
    match std::env::var("GUACA_ACCOUNT_ORIGIN") {
        Ok(origin) if !origin.trim().is_empty() => {
            let origin = origin.trim().to_string();
            tracing::warn!(%origin, "signing in to a Guaca account somewhere other than the default");
            Account::open_at(path, origin)
        }
        _ => Account::open(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_keeps_both_directories_under_one_root() {
        // One thing to snapshot and one thing to restore. The desktop is handed
        // two unrelated directories by the operating system and keeps them; a
        // box has no such convention and a volume wants a single subtree.
        let paths = Paths::under(Path::new("/srv/guaca"));
        assert!(paths.db().starts_with("/srv/guaca"));
        assert!(paths.config_file().starts_with("/srv/guaca"));
        assert_ne!(paths.data, paths.config);
    }

    #[test]
    fn the_database_and_the_settings_are_not_the_same_file() {
        // They have different writers and one of them writes in the background.
        let paths = Paths::under(Path::new("/srv/guaca"));
        assert_ne!(paths.db(), paths.config_file());
    }
}
