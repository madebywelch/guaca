use guac_lib::secrets::ProcessEnv;

use guac_lib::db::Store;
use guac_lib::e2b::E2bClient;

const DOCUMENTS: [(&str, &str, &str); 2] = [
    ("beyond-browsing.pdf", "https://arxiv.org/pdf/2410.16464", "Beyond Browsing"),
    ("attention.pdf", "https://arxiv.org/pdf/1706.03762", "Attention Is All You Need"),
];

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
    (!config.e2b.api_key.expose().trim().is_empty()).then_some(config)
}

fn credentials_holding(var: &str) -> Option<ProcessEnv> {
    let store = Store::open(&app_dir()?.join("guac.db")).ok()?;
    store
        .list_agents()
        .ok()?
        .into_iter()
        .filter_map(|group| store.connector_env(group.id).ok())
        .find(|env| env.contains(var))
}

struct Machine {
    client: E2bClient,
    id: String,
    token: guac_lib::secrets::Secret,
}

impl Machine {
    async fn start(env: ProcessEnv, key: &guac_lib::secrets::Secret) -> Machine {
        let client = E2bClient::new(key).expect("an E2B key is configured").with_env(env);
        let sandbox = client.create("connector-test", 300).await.expect("a machine to work on");
        Machine { client, id: sandbox.id, token: sandbox.envd_token }
    }

    async fn run(&self, command: &str) -> String {
        let out = self
            .client
            .run(&self.id, &self.token, command)
            .await
            .unwrap_or_else(|err| panic!("could not run on the machine: {err}"));
        assert_eq!(
            out.exit_code, 0,
            "command failed: {command}\nstdout: {}\nstderr: {}",
            out.stdout, out.stderr
        );
        out.stdout
    }

    async fn release(&self) {
        if let Err(err) = self.client.kill(&self.id).await {
            eprintln!("could not release {}: {err}", self.id);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "live: costs money, needs the E2B and Mistral keys the app is configured with"]
async fn an_agent_can_read_a_folder_of_documents_it_downloaded() {
    let Some(config) = configured() else {
        eprintln!("no E2B key configured; skipping");
        return;
    };
    let Some(env) = credentials_holding("MISTRAL_API_KEY") else {
        eprintln!("no Mistral credential in any group; add one and run this again");
        return;
    };

    let machine = Machine::start(env, &config.e2b.api_key).await;

    let mut downloaded = String::new();
    for (name, url) in DOCUMENTS.map(|(name, url, _)| (name, url)) {
        machine
            .run(&format!("mkdir -p ~/docs && curl -sSL --max-time 120 -o ~/docs/{name} {url}"))
            .await;
        downloaded = machine.run("ls -1 ~/docs && du -sh ~/docs | cut -f1").await;
    }
    assert!(downloaded.contains("beyond-browsing.pdf"), "the folder is not there: {downloaded}");

    let present = machine.run("test -n \"$MISTRAL_API_KEY\" && echo set || echo missing").await;
    assert_eq!(present.trim(), "set", "the credential never reached the machine's environment");

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

    assert!(!text.contains("MISTRAL_API_KEY="), "the environment leaked into the output");
}
