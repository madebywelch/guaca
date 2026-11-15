//! Opening a workspace, which is the same act in both hosts.
//!
//! A workspace is a database, three settings files and two directories, and
//! bringing one up is the same sequence whether the runtime ends up behind a
//! window or behind a socket: open the store, close whatever a restart
//! stranded, read the settings, build the runtime, pick up what the last one
//! put down, start the loops that keep their own time, and release anything a
//! previous process left running at a provider.
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
    // Memories as plain markdown and attachments by content hash, all under
    // the data directory.
    // `OnDisk::under` is the one place that arrangement is decided.
    let disk = OnDisk::under(&paths.data);
    let workspace_dir = disk.workspace.root().to_path_buf();
    // The starter skills, once per workspace. A failure here costs the
    // operator a handful of documents they can add by hand, not the workspace.
    match disk.skills.offer(crate::skills::STARTERS) {
        Ok(added) if !added.is_empty() => tracing::info!(?added, "added the starter skills"),
        Ok(_) => {}
        Err(err) => tracing::warn!(%err, "could not add the starter skills"),
    }

    let store =
        Store::open(&db_path).map_err(|err| format!("could not open the workspace: {err}"))?;
    let recovered = recover(&store)?;

    let mut app_config =
        config::load(&config_path).map_err(|err| format!("could not read the settings: {err}"))?;
    // Hosted webhooks arrive through the daemon's public listener. Persist the
    // same dedicated secret the desktop receiver uses, not the workspace token.
    if crate::webhook::prepare(&mut app_config.webhook) {
        config::save(&config_path, &app_config)
            .map_err(|err| format!("could not save webhook settings: {err}"))?;
    }
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

    runtime.keep_settings_at(config_path.clone());
    runtime.hold_workspace_lease(lease);

    let started =
        runtime.start_all().map_err(|err| format!("could not start the agents: {err}"))?;
    // Before the scheduler, so an agent with work to carry on reads as busy
    // to its first sweep.
    runtime.pick_up(recovered);
    // After the pickup, so what recovery wrote for the operator, a run cut
    // off by the restart, is the first thing the dispatcher reads. Before the
    // scheduler, whose first sweep claims the reminders due while the Host was
    // down and wakes it.
    runtime.start_dispatcher();
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

/// Reads back what the last process left in the store, before any actor
/// starts: closes what nothing can answer any more, reports what nobody can
/// vouch for, and returns what the last host put down for `Runtime::pick_up`.
///
/// Its own function so the suites that restart a runtime run exactly this
/// rather than a copy of it.
pub fn recover(store: &Store) -> Result<crate::db::Recovered, String> {
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

    // Before the conversations, because recovering one deletes its recovery
    // point, and that row is the only thing that says a decision's
    // follow-through was cut off rather than finished long ago.
    store
        .recover_decisions()
        .map_err(|err| format!("could not recover decision answers: {err}"))?;

    let recovered = store
        .recover()
        .map_err(|err| format!("could not recover interrupted conversations: {err}"))?;
    if recovered.interrupted > 0 {
        tracing::warn!(
            interrupted = recovered.interrupted,
            "conversations interrupted by restart are ready for review"
        );
    }
    Ok(recovered)
}

/// The account store, pointed wherever `GUACA_ACCOUNT_ORIGIN` says.
///
/// An environment variable rather than a setting, for the reason
/// `subscription.rs` gives: a sign-in service an operator can type into a box
/// is a credential sent somewhere nobody chose. An override is logged, because
/// a machine left pointed at a development service must not be a silent state.
pub fn account_store(path: PathBuf) -> Account {
    let account = match std::env::var("GUACA_ACCOUNT_ORIGIN") {
        Ok(origin) if !origin.trim().is_empty() => {
            let origin = origin.trim().to_string();
            tracing::warn!("using an overridden Guaca account origin");
            Account::open_at(path, origin)
        }
        _ => Account::open(path),
    };
    match std::env::var("GUACA_BROKER_BEARER") {
        Ok(bearer) if !bearer.trim().is_empty() => account.with_broker(bearer),
        _ => account,
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
    fn recovery_marks_only_follow_through_the_restart_cut_off() {
        // Two answered decisions. One's follow-through finished before the
        // restart, the other's was still running. Only the second was cut off.
        //
        // This is also why the decisions are read before the conversations:
        // recovering a conversation deletes its recovery point, and that row
        // is the only thing that tells a run cut off from one that finished.
        // Read after it, the second decision would look finished too.
        use crate::domain::agent::CleanDraft;
        use crate::domain::decision::DecisionRequest;
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();
        let agent = store
            .create_agent(&CleanDraft {
                group_id: None,
                name: "Assistant".into(),
                avatar: "orb".into(),
                color: "#7fb069".into(),
                model: "test".into(),
                subscription_model: None,
                reasoning_effort: None,
                system_prompt: String::new(),
                skills: vec![],
            })
            .unwrap();
        let answered = |topic: &str| {
            let request = DecisionRequest {
                question: "10 or 11?".into(),
                context: String::new(),
                recommendation: String::new(),
                options: vec!["10".into(), "11".into()],
                source: String::new(),
            };
            let filed =
                store.request_decision(agent.id, agent.group_id, topic, request, None, 1).unwrap();
            store.answer_decision(filed.id, "11", false, None, 2).unwrap()
        };
        let (finished, delivery) = answered("finished");
        store.settle_run(delivery.run_id).unwrap();
        let (cut, _) = answered("cut");

        recover(&store).unwrap();
        let after = store.decisions(None).unwrap();
        let interrupted = |id| after.iter().find(|d| d.id == id).unwrap().interrupted;
        assert!(!interrupted(finished.id), "follow-through that had settled was reported cut off");
        assert!(interrupted(cut.id), "follow-through the restart cut off was not reported");
    }

    #[test]
    fn the_database_and_the_settings_are_not_the_same_file() {
        // They have different writers and one of them writes in the background.
        let paths = Paths::under(Path::new("/srv/guaca"));
        assert_ne!(paths.db(), paths.config_file());
    }
}
