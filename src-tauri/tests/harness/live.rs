//! A crew on the operator's own endpoint, with a real model answering.
//!
//! Everything here reaches the network and spends money, so nothing that uses
//! it runs in CI. It is shared rather than owned by one suite because two ask
//! live questions now: `evals.rs` asks whether a crew communicates like
//! something worth watching, and `crew.rs` asks what a whole team does with one
//! directive and what is different when you ask again. A second copy of this
//! would drift, and the half that drifts without anybody noticing is the
//! cleanup: a machine left running bills for its idle period whether or not the
//! scenario that started it passed.

use std::collections::HashMap;
use std::sync::Arc;

use guac_lib::config::{AppConfig, Provider};
use guac_lib::db::Store;
use guac_lib::domain::agent::CleanDraft;
use guac_lib::domain::approval::Decision;
use guac_lib::llm::openrouter::LlmClient;
use guac_lib::runtime::events::{EventSink, NullSink, RecordingSink, UiEvent};
use guac_lib::runtime::{OnDisk, Runtime};

use super::Harness;

/// One agent in a live crew.
///
/// Skills are here because a scenario about who should get a piece of work is
/// not testable without them: a crew of four agents with no stated skills gives
/// a coordinator nothing to choose between, and the broadcast it produces is
/// then the right answer.
pub struct LiveAgent {
    pub name: &'static str,
    pub skills: &'static [&'static str],
    /// `None` takes a serviceable default. `Some("")` means the card really
    /// carries no instructions, which is how most agents are created and is
    /// what the workspace rules have to hold up without.
    pub prompt: Option<&'static str>,
    /// Overrides the configured default. A crew is not obliged to share a
    /// model, and a coordinator on a different one from the agents it directs
    /// is the arrangement that produced the defect these evals were written
    /// for: putting everyone on the default quietly tests a different app.
    pub model: Option<&'static str>,
}

impl LiveAgent {
    /// An agent for scenarios that are not about who does what.
    pub fn generic(name: &'static str) -> Self {
        LiveAgent { name, skills: &[], prompt: None, model: None }
    }

    /// A specialist, described only by what it does.
    ///
    /// `Some("")` rather than a default sentence, because a card with no
    /// instructions is how most agents are created and is what the workspace
    /// rules have to hold up without: a scenario whose specialists each carry a
    /// hand-written brief is testing the brief.
    pub fn skilled(name: &'static str, skills: &'static [&'static str]) -> Self {
        LiveAgent { name, skills, prompt: Some(""), model: None }
    }

    /// The same, carrying the standing instruction an operator would type on
    /// the one agent they talk to.
    pub fn told(name: &'static str, skills: &'static [&'static str], prompt: &'static str) -> Self {
        LiveAgent { name, skills, prompt: Some(prompt), model: None }
    }

    pub fn system_prompt(&self) -> String {
        match self.prompt {
            Some(prompt) => prompt.to_string(),
            None => format!(
                "You are the {}. Work with your team the way the workspace rules say.",
                self.name
            ),
        }
    }
}

/// The operator's own settings, or nothing when no key has been pasted.
///
/// Defaults come from the app's settings. Shell overrides select a test model
/// on compatible endpoints and a fresh key on OpenRouter without saving either.
pub fn configured() -> Option<AppConfig> {
    let path = dirs_config()?;
    let raw = std::fs::read_to_string(path).ok()?;
    let mut config: AppConfig = serde_json::from_str(&raw).ok()?;
    use_openrouter_key(&mut config, std::env::var("OPENROUTER_API_KEY").ok().as_deref());
    if config.inference.provider == Provider::Compatible {
        if let Some(model) = std::env::var("GUACA_TEST_MODEL")
            .ok()
            .map(|model| model.trim().to_owned())
            .filter(|model| !model.is_empty())
        {
            config.inference.default_model = model;
        }
    }
    (!config.inference.api_key.trim().is_empty()).then_some(config)
}

fn use_openrouter_key(config: &mut AppConfig, key: Option<&str>) {
    // A shell credential for OpenRouter must never replace a local endpoint's
    // key or travel to another provider configured in the app.
    let openrouter = reqwest::Url::parse(&config.inference.base_url)
        .is_ok_and(|url| url.origin().ascii_serialization() == "https://openrouter.ai");
    if config.inference.provider == Provider::Compatible && openrouter {
        if let Some(key) = key.map(str::trim).filter(|key| !key.is_empty()) {
            config.inference.api_key = key.to_owned();
        }
    }
}

#[test]
fn a_shell_key_overrides_only_the_openrouter_origin() {
    for (endpoint, expected) in [
        ("https://openrouter.ai/api/v1", "shell-key"),
        ("https://openrouter.ai:443/api/v1", "shell-key"),
        ("http://openrouter.ai/api/v1", "saved-key"),
        ("https://openrouter.ai:8443/api/v1", "saved-key"),
        ("https://openrouter.ai.example.com/api/v1", "saved-key"),
        ("http://localhost:1234/v1", "saved-key"),
        ("invalid", "saved-key"),
    ] {
        let mut config = AppConfig::default();
        config.inference.base_url = endpoint.into();
        config.inference.api_key = "saved-key".into();
        use_openrouter_key(&mut config, Some(" shell-key "));
        assert_eq!(config.inference.api_key, expected, "{endpoint}");
    }
}

#[test]
fn an_absent_shell_key_or_subscription_keeps_the_saved_key() {
    for (provider, key) in [
        (Provider::Compatible, None),
        (Provider::Compatible, Some("  ")),
        (Provider::Chatgpt, Some("shell-key")),
        (Provider::Claude, Some("shell-key")),
    ] {
        let mut config = AppConfig::default();
        config.inference.provider = provider;
        config.inference.base_url = "https://openrouter.ai/api/v1".into();
        config.inference.api_key = "saved-key".into();
        use_openrouter_key(&mut config, key);
        assert_eq!(config.inference.api_key, "saved-key");
    }
}

fn dirs_config() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        std::path::PathBuf::from(home)
            .join("Library/Application Support/com.madebywelch.guac/config.json"),
    )
}

/// A harness pointed at the real configured endpoint rather than at a stub.
pub fn live_crew(config: AppConfig, crew: &[LiveAgent]) -> Harness {
    live_crew_watched(config, crew, Arc::new(NullSink))
}

/// The same, with a second reader on every event the runtime emits.
///
/// The recording sink the harness keeps is memory, and memory is gone when a
/// run overruns and takes the process with it. A watcher that writes as events
/// arrive is what leaves a record of exactly the runs worth reading: the ones
/// that did not finish.
pub fn live_crew_watched(
    config: AppConfig,
    crew: &[LiveAgent],
    watching: Arc<dyn EventSink>,
) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("guac.db")).unwrap();

    let mut ids = HashMap::new();
    for agent in crew {
        let card = store
            .create_agent(&CleanDraft {
                name: agent.name.to_string(),
                avatar: "plain".into(),
                color: "#c7d96b".into(),
                model: agent
                    .model
                    .map(str::to_string)
                    .unwrap_or_else(|| config.inference.default_model.clone()),
                system_prompt: agent.system_prompt(),
                skills: agent.skills.iter().map(|s| (*s).to_string()).collect(),
                group_id: None,
            })
            .unwrap();
        ids.insert(agent.name.to_string(), card.id);
    }

    let kept = RecordingSink::new();
    let runtime = Runtime::new(
        store,
        LlmClient::new().unwrap(),
        config,
        OnDisk::under(dir.path()),
        Arc::new(Tee { watching, kept: kept.clone() }),
    );
    runtime.start_all().unwrap();
    Harness { runtime, sink: kept, ids, _dir: dir }
}

/// One event, two readers.
struct Tee {
    watching: Arc<dyn EventSink>,
    kept: Arc<RecordingSink>,
}

impl EventSink for Tee {
    fn emit(&self, event: UiEvent) {
        // The watcher first: it is the one that might be a file, and a record
        // on disk is worth more than the order these two are called in.
        self.watching.emit(event.clone());
        self.kept.emit(event);
    }
}

/// Answers whatever the crew asks the operator, with no.
///
/// A live crew has no operator, and a parked turn holds its run for the ten
/// minutes the runtime waits before giving up on one. A scenario about
/// delegation that happens to trip a permission request would otherwise spend
/// its whole settle window waiting for a click that is never coming, and fail
/// as though the crew had never stopped talking. That is exactly how a knee
/// question in a crew of three read: two messages, one of them blank, and five
/// minutes of nothing.
///
/// Declined rather than allowed, and not negotiable: the actions behind this
/// tool are the ones that reach outside the workspace and cannot be taken back.
/// An eval is not a good enough reason to send mail in the operator's name.
/// What was asked is returned so the scenario can print it, because an answer
/// nobody sees still shapes the run.
pub fn answer_permission_requests(
    h: &Harness,
) -> (tokio::task::JoinHandle<()>, Arc<parking_lot::Mutex<Vec<String>>>) {
    let asked = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let runtime = h.runtime.clone();
    let sink = h.sink.clone();
    let recorded = asked.clone();

    let handle = tokio::spawn(async move {
        let mut answered = std::collections::HashSet::new();
        loop {
            let requests: Vec<_> = sink
                .snapshot()
                .into_iter()
                .filter_map(|event| match event {
                    UiEvent::ApprovalRequested { approval_id, .. } => Some(approval_id),
                    _ => None,
                })
                .collect();
            for id in requests {
                if !answered.insert(id) {
                    continue;
                }
                recorded.lock().push(id.short());
                if let Err(err) = runtime.decide_approval(id, Decision::Deny) {
                    eprintln!("could not decline {}: {err}", id.short());
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    });

    (handle, asked)
}

/// Every sandbox this account holds, by id.
pub async fn machines_now(config: &AppConfig) -> Vec<String> {
    match guac_lib::e2b::E2bClient::new(&config.e2b.api_key) {
        Some(client) => client.list_ours().await.unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Kills whatever this run brought into existence, and nothing else.
///
/// A diff against a baseline rather than a walk over the crew's rows. An agent
/// whose first machine does not answer starts another and records only the
/// newest, so the rows name one of the several a single run can leave behind;
/// seventeen survived a cleanup written that way.
///
/// It is also why `Runtime::sweep_computers` cannot be borrowed for this. That
/// kills every Guac sandbox its own store does not claim, so run from a
/// throwaway store it would spare this crew's machines and take the operator's
/// running app apart instead.
pub async fn release_machines(config: &AppConfig, before: Vec<String>) {
    let Some(client) = guac_lib::e2b::E2bClient::new(&config.e2b.api_key) else {
        return;
    };
    let existing: std::collections::HashSet<String> = before.into_iter().collect();
    for sandbox in client.list_ours().await.unwrap_or_default() {
        if existing.contains(&sandbox) {
            continue;
        }
        match client.kill(&sandbox).await {
            Ok(()) => println!("released {sandbox}"),
            Err(err) => eprintln!("could not release {sandbox}: {err}"),
        }
    }
}
