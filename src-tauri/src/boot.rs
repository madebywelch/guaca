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

#[derive(Debug, Clone)]
pub struct Paths {
    pub data: PathBuf,
    pub config: PathBuf,
}

impl Paths {
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

pub struct Booted {
    pub runtime: Runtime,
    pub subscription: Arc<Subscription>,
    pub account: Arc<Account>,
    pub artifacts: Artifacts,
    pub artifact_port: Arc<AtomicU16>,
    pub config_path: PathBuf,
    pub started: usize,
}

pub async fn open(
    paths: &Paths,
    handle: tokio::runtime::Handle,
    sink: Arc<dyn EventSink>,
) -> Result<Booted, String> {
    std::fs::create_dir_all(&paths.data)
        .map_err(|err| format!("{}: {err}", paths.data.display()))?;
    std::fs::create_dir_all(&paths.config)
        .map_err(|err| format!("{}: {err}", paths.config.display()))?;

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
    let disk = OnDisk::under(&paths.data);
    let workspace_dir = disk.workspace.root().to_path_buf();
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
    if crate::webhook::prepare(&mut app_config.webhook) {
        config::save(&config_path, &app_config)
            .map_err(|err| format!("could not save webhook settings: {err}"))?;
    }
    let subscription = Arc::new(Subscription::open(paths.config.join("subscription.json")));
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
        runtime.resume(recovered).map_err(|err| format!("could not start the agents: {err}"))?;
    runtime.start_signin_sweep();
    runtime.start_compost();

    let viewer_port = crate::proxy::start(runtime.store().clone())
        .await
        .map_err(|err| format!("could not start the computer viewer: {err}"))?;
    runtime.set_viewer_port(viewer_port);

    let artifacts = Artifacts::new();
    let artifact_port = Arc::new(AtomicU16::new(
        artifact::start(artifacts.clone())
            .await
            .map_err(|err| format!("could not start the artifact viewer: {err}"))?,
    ));

    sweep_providers(&runtime);

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

fn sweep_providers(runtime: &Runtime) {
    {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            match runtime.sweep_computers().await {
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

pub fn recover(store: &Store) -> Result<crate::db::Recovered, String> {
    match store.expire_pending_approvals() {
        Ok(0) => {}
        Ok(n) => tracing::info!(expired = n, "closed permission requests left by a restart"),
        Err(err) => tracing::warn!(%err, "could not close stale permission requests"),
    }

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
    use crate::db::RunLock;

    #[test]
    fn the_native_client_builds_no_runtime() {
        let client = include_str!("app.rs");
        for reach in ["Runtime::new", "Runtime::with_handle", "boot::open", "start_all"] {
            assert!(
                !client.contains(reach),
                "app.rs names {reach}, but agents run only in guacad: the window asks the host \
                 through its commands instead"
            );
        }
    }

    #[test]
    fn a_server_keeps_both_directories_under_one_root() {
        let paths = Paths::under(Path::new("/srv/guaca"));
        assert!(paths.db().starts_with("/srv/guaca"));
        assert!(paths.config_file().starts_with("/srv/guaca"));
        assert_ne!(paths.data, paths.config);
    }

    #[test]
    fn recovery_marks_only_follow_through_the_restart_cut_off() {
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
            store.answer_decision(&RunLock::new(()).lock(), filed.id, "11", false, None, 2).unwrap()
        };
        let (finished, delivery) = answered("finished");
        store.settle_run(delivery.run_id).unwrap();
        let (cut, delivery) = answered("cut");
        store.start_deliveries(&[delivery.id], delivery.run_id).unwrap();

        recover(&store).unwrap();
        let after = store.decisions(None).unwrap();
        let interrupted = |id| after.iter().find(|d| d.id == id).unwrap().interrupted;
        assert!(!interrupted(finished.id), "follow-through that had settled was reported cut off");
        assert!(interrupted(cut.id), "follow-through the restart cut off was not reported");
    }

    #[test]
    fn the_database_and_the_settings_are_not_the_same_file() {
        let paths = Paths::under(Path::new("/srv/guaca"));
        assert_ne!(paths.db(), paths.config_file());
    }
}
