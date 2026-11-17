use std::collections::HashMap;
use std::sync::Arc;

use guac_lib::config::{AppConfig, Provider};
use guac_lib::db::Store;
use guac_lib::domain::agent::CleanDraft;
use guac_lib::domain::approval::Decision;
use guac_lib::domain::ids::RunId;
use guac_lib::llm::openrouter::LlmClient;
use guac_lib::runtime::events::{EventSink, NullSink, RecordingSink, UiEvent};
use guac_lib::runtime::{OnDisk, Runtime};

use super::Harness;

pub struct LiveAgent {
    pub name: &'static str,
    pub skills: &'static [&'static str],
    pub prompt: Option<&'static str>,
    pub model: Option<&'static str>,
}

impl LiveAgent {
    pub fn generic(name: &'static str) -> Self {
        LiveAgent { name, skills: &[], prompt: None, model: None }
    }

    pub fn skilled(name: &'static str, skills: &'static [&'static str]) -> Self {
        LiveAgent { name, skills, prompt: Some(""), model: None }
    }

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

pub fn live_crew(config: AppConfig, crew: &[LiveAgent]) -> Harness {
    live_crew_watched(config, crew, Arc::new(NullSink))
}

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
                subscription_model: None,
                reasoning_effort: None,
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

struct Tee {
    watching: Arc<dyn EventSink>,
    kept: Arc<RecordingSink>,
}

impl EventSink for Tee {
    fn emit(&self, event: UiEvent) {
        self.watching.emit(event.clone());
        self.kept.emit(event);
    }
}

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

pub struct Machines {
    config: AppConfig,
    before: Vec<String>,
}

pub async fn machines_now(config: &AppConfig) -> Machines {
    let before = match guac_lib::e2b::E2bClient::new(&config.e2b.api_key) {
        Some(client) => client.list_ours().await.unwrap_or_default(),
        None => Vec::new(),
    };
    Machines { config: config.clone(), before }
}

pub async fn settle_and_release(
    h: &Harness,
    run: RunId,
    secs: u64,
    answering: tokio::task::JoinHandle<()>,
    machines: Machines,
) -> bool {
    let settled = h.settled_within(run, secs).await;
    answering.abort();
    release_machines(machines).await;
    settled
}

async fn release_machines(Machines { config, before }: Machines) {
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
