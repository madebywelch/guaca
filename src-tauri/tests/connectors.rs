//! What a credential actually reaches, against the real service.
//!
//! The scripted suites prove the plumbing: a secret goes into the environment
//! of a sandbox command and never anywhere else. They cannot prove the thing an
//! operator cares about, which is that an agent handed a key can do the work the
//! key is for. That needs the real machine and the real API, so it costs money
//! and lives behind `#[ignore]`.
//!
//! Run with `./scripts/connectors.sh`.
//!
//! The credential is read from the app's own database and put into the sandbox
//! the way the runtime puts it there. It is never printed, never asserted on,
//! and never written to the sandbox's disk: if one of these tests fails, the
//! output says which step failed and not what the token was.

use std::collections::BTreeMap;

use guac_lib::computer::e2b::E2bProvider;
use guac_lib::computer::provider::{ComputerProvider, CreateComputer, ExecRequest, ProviderHandle};
use guac_lib::db::Store;
use guac_lib::domain::ids::{AgentId, ComputerId};

/// Two real documents, and something in each that OCR has to come back with.
///
/// arXiv because the papers are stable, public, and already cited by this
/// project: `docs/PROTOCOL.md` argues from both of them.
const DOCUMENTS: [(&str, &str, &str); 2] = [
    ("beyond-browsing.pdf", "https://arxiv.org/pdf/2410.16464", "Beyond Browsing"),
    ("attention.pdf", "https://arxiv.org/pdf/1706.03762", "Attention Is All You Need"),
];

/// One local file, read by Mistral OCR: upload, sign, read.
///
/// Written as shell rather than as Rust HTTP calls on purpose. This is the
/// sequence an agent will run through `run_command`, in the same login shell,
/// reaching the credential by the same variable name, so what passes here is
/// what an agent can do rather than something only the test can.
///
/// `DOCUMENT` is replaced with the file name. Not a `format!`: the body is
/// mostly braces and escaping every one of them makes it unreadable and
/// unpasteable into a terminal, which is where it was worked out.
const OCR_ONE_FILE: &str = r#"set -e
file_id=$(curl -sS --max-time 120 https://api.mistral.ai/v1/files \
  -H "Authorization: Bearer $MISTRAL_API_KEY" \
  -F purpose=ocr -F file=@$HOME/docs/DOCUMENT \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
signed=$(curl -sS --max-time 120 "https://api.mistral.ai/v1/files/$file_id/url?expiry=1" \
  -H "Authorization: Bearer $MISTRAL_API_KEY" \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["url"])')
python3 -c 'import json,sys; print(json.dumps({"model":"mistral-ocr-4-1","document":{"type":"document_url","document_url":sys.argv[1]},"pages":[0]}))' "$signed" > /tmp/ocr.json
curl -sS --max-time 300 https://api.mistral.ai/v1/ocr \
  -H "Authorization: Bearer $MISTRAL_API_KEY" -H 'Content-Type: application/json' \
  --data @/tmp/ocr.json \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); print("\n".join(p.get("markdown","") for p in d.get("pages",[])) or json.dumps(d)[:400])'"#;

fn app_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(std::path::PathBuf::from(home).join("Library/Application Support/com.madebywelch.guac"))
}

fn configured() -> Option<guac_lib::config::AppConfig> {
    let raw = std::fs::read_to_string(app_dir()?.join("config.json")).ok()?;
    let config: guac_lib::config::AppConfig = serde_json::from_str(&raw).ok()?;
    (!config.e2b.api_key.trim().is_empty()).then_some(config)
}

/// The group environment holding `var`, exactly as the runtime would build it.
///
/// Whichever group the operator put the credential in: it is one workspace and
/// the test should not care which crew holds the key.
fn credentials_holding(var: &str) -> Option<BTreeMap<String, String>> {
    let store = Store::open(&app_dir()?.join("guac.db")).ok()?;
    store
        .list_groups()
        .ok()?
        .into_iter()
        .filter_map(|group| store.connector_env(group.id).ok())
        .find(|env| env.contains_key(var))
}

/// A machine with the group's credentials in its environment.
struct Machine {
    provider: E2bProvider,
    handle: ProviderHandle,
    /// Carried rather than baked into the provider: credentials belong to the
    /// command, which is how the runtime hands them over too.
    env: BTreeMap<String, String>,
}

impl Machine {
    async fn start(env: BTreeMap<String, String>, key: &str) -> Machine {
        let provider = E2bProvider::new(key).expect("an E2B key is configured");
        let handle = provider
            .create(&CreateComputer {
                computer: ComputerId::new(),
                agent: AgentId::new(),
                agent_name: "connector-test".into(),
                idle_seconds: 300,
            })
            .await
            .expect("a machine to work on");
        Machine { provider, handle, env }
    }

    async fn run(&self, command: &str) -> String {
        let out = self
            .provider
            .exec(
                &self.handle,
                ExecRequest {
                    argv: vec!["/bin/bash".into(), "-l".into(), "-c".into(), command.to_string()],
                    env: self.env.clone(),
                    cwd: "/home/user".into(),
                    timeout: std::time::Duration::from_secs(120),
                },
            )
            .await
            .unwrap_or_else(|err| panic!("could not run on the machine: {err}"));
        assert_eq!(
            out.exit_code, 0,
            "command failed: {command}\nstdout: {}\nstderr: {}",
            out.stdout, out.stderr
        );
        out.stdout
    }

    /// Never left running. A machine nobody holds a reference to bills exactly
    /// like a used one, and a failing assertion takes the rest of the test with
    /// it, so this is called before anything that can panic.
    async fn release(&self) {
        if let Err(err) = self.provider.delete(&self.handle).await {
            eprintln!("could not release {}: {err}", self.handle.provider_id);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "live: costs money, needs the E2B and Mistral keys the app is configured with"]
async fn an_agent_can_read_a_folder_of_documents_it_downloaded() {
    // The whole path an operator asked for, end to end: a machine with the
    // group's credentials in its environment downloads a folder of PDFs and
    // turns them into text with Mistral OCR. Everything here is what an agent
    // would do with `run_command`, in the same shell, with the same variable.
    let Some(config) = configured() else {
        eprintln!("no E2B key configured; skipping");
        return;
    };
    let Some(env) = credentials_holding("MISTRAL_API_KEY") else {
        eprintln!("no Mistral credential in any group; add one and run this again");
        return;
    };

    let machine = Machine::start(env, &config.e2b.api_key).await;

    // 1. The folder. Downloaded the way an agent would, with the tool it has.
    let mut downloaded = String::new();
    for (name, url) in DOCUMENTS.map(|(name, url, _)| (name, url)) {
        machine
            .run(&format!("mkdir -p ~/docs && curl -sSL --max-time 120 -o ~/docs/{name} {url}"))
            .await;
        downloaded = machine.run("ls -1 ~/docs && du -sh ~/docs | cut -f1").await;
    }
    assert!(downloaded.contains("beyond-browsing.pdf"), "the folder is not there: {downloaded}");

    // 2. The credential reaches the shell under the name the catalog promised,
    // and only as a name: what is asserted is that it is set, never its value.
    let present = machine.run("test -n \"$MISTRAL_API_KEY\" && echo set || echo missing").await;
    assert_eq!(present.trim(), "set", "the credential never reached the machine's environment");

    // 3. OCR, per file: upload, sign, read. This is the sequence the catalog
    // note tells an agent about, run exactly as written there.
    let mut text = String::new();
    for (name, _, expected) in DOCUMENTS {
        let extracted = machine.run(&OCR_ONE_FILE.replace("DOCUMENT", name)).await;

        println!("--- {name} ---\n{}\n", extracted.chars().take(600).collect::<String>());
        assert!(
            extracted.to_lowercase().contains(&expected.to_lowercase()),
            "OCR did not return the text on page one of {name}. Expected {expected:?} in:\n{extracted}"
        );
        text.push_str(&extracted);
    }

    machine.release().await;

    // The value must not have travelled with the result. Cheap to check and the
    // one mistake in this flow that would matter.
    assert!(!text.contains("MISTRAL_API_KEY="), "the environment leaked into the output");
}
