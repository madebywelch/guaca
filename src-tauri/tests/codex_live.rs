//! Explicitly opted-in contract check against the official CLI. A dedicated
//! test binary isolates its PATH wrapper from all offline stand-ins.
#![cfg(unix)]

#[tokio::test]
#[ignore = "live: uses Codex login with gpt-5.4-mini, never the configured default model"]
async fn codex_mini_accepts_steering_and_honors_a_denied_push() {
    use guac_lib::{coding, domain::repository::Harness};
    use std::{os::unix::fs::PermissionsExt, process::Command, time::Duration};
    let path = std::env::var_os("PATH").unwrap();
    let binary = std::env::split_paths(&path)
        .map(|p| p.join("codex"))
        .find(|p| p.is_file())
        .expect("install Codex first");
    let signed_in = Command::new(&binary).args(["login", "status"]).output().unwrap();
    assert!(signed_in.status.success(), "sign in with codex login --device-auth first");
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    let repo = dir.path().join("repo");
    std::fs::create_dir(&bin).unwrap();
    std::fs::create_dir(&repo).unwrap();
    let wrapper = bin.join("codex");
    // Keep the operator's authentication, override the model explicitly, and
    // disable their MCP servers for this disposable contract test. The wrapper
    // never reads or prints auth.json and never edits the operator's config.
    let real = serde_json::to_string(&binary.to_string_lossy()).unwrap();
    std::fs::write(&wrapper, format!(r#"#!/usr/bin/env python3
import os, pathlib, sys, tomllib
real = {real}
root = pathlib.Path(os.environ.get('CODEX_HOME', str(pathlib.Path.home() / '.codex')))
config = root / 'config.toml'
settings = tomllib.loads(config.read_text()) if config.exists() else {{}}
args = [real, 'app-server', '-c', 'model="gpt-5.4-mini"', '-c', 'model_reasoning_effort="low"', '-c', 'features.multi_agent=false']
for name in settings.get('mcp_servers', {{}}):
    args += ['-c', 'mcp_servers.' + name + '.enabled=false']
for key in ['CODEX_API_KEY', 'OPENAI_API_KEY']:
    os.environ.pop(key, None)
os.execv(real, args + sys.argv[2:])
"#)).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git").arg("-C").arg(&repo).args(args).output().unwrap();
        assert!(out.status.success(), "git failed: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join("README.md"), "A tiny CLI contract test.\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "fixture"]);
    let before = git(&["rev-parse", "HEAD"]);
    std::env::set_var("PATH", format!("{}:{}", bin.display(), path.to_string_lossy()));
    let bare = dir.path().join("origin.git");
    git(&["init", "--bare", bare.to_str().unwrap()]);
    git(&["remote", "add", "origin", bare.to_str().unwrap()]);
    let (corrections, steering) = tokio::sync::mpsc::channel(8);
    let (signals, mut heard) = tokio::sync::mpsc::channel(8);
    let (progress, mut watching) = tokio::sync::mpsc::unbounded_channel();
    let running = coding::run_with_control(
        Harness::Codex, repo.to_str().unwrap(),
        "First run sleep 3. Then create smoke.txt containing exactly original followed by a newline. Check its contents and commit it with message test: verify Codex runner. Finally push HEAD to origin as branch steering-check. Origin is a disposable local bare repository. If push is denied, do not retry. Do not contact external services, spawn subagents, or change other files. Reply briefly.",
        None, Some(coding::codex::Control { gate: guac_lib::domain::repository::Gate::AskBeforePushing, steering, signals }),
        move |event| { let _ = progress.send(event); },
    );
    tokio::pin!(running);
    let mut steered = false;
    let mut gated = false;
    let mut acknowledgment = None;
    let checking = async {
        loop {
            tokio::select! {
                done = &mut running => break done,
                Some(event) = watching.recv() => {
                    if matches!(&event, coding::Progress::Using { detail, .. } if detail.contains("sleep 3")) && !steered && acknowledgment.is_none() {
                        let (reply, accepted) = tokio::sync::oneshot::channel();
                        corrections.send(coding::codex::Steer { message: "Change of plan: smoke.txt must contain exactly steered followed by a newline. Verify and commit that content before the planned push.".into(), reply }).await.unwrap();
                        acknowledgment = Some(accepted);
                    }
                }
                accepted = async { acknowledgment.as_mut().unwrap().await }, if acknowledgment.is_some() => {
                    accepted.unwrap().unwrap();
                    steered = true;
                    acknowledgment = None;
                }
                Some(signal) = heard.recv() => {
                    if let coding::Signal::Permission { line, reply, .. } = signal {
                        assert!(line.contains("push"), "unexpected gated command: {line}");
                        gated = true;
                        let _ = reply.send(false);
                    }
                }
            }
        }
    };
    let result = tokio::time::timeout(Duration::from_secs(120), checking).await;
    std::env::set_var("PATH", &path);
    let outcome = result.expect("Codex exceeded two minutes").expect("Codex could not run");
    assert!(steered, "the live turn never accepted steering");
    assert!(gated, "the live CLI never asked before pushing");
    assert_eq!(outcome.model, "gpt-5.4-mini");
    assert!(
        git(&["--git-dir", bare.to_str().unwrap(), "for-each-ref"]).is_empty(),
        "a denied push changed the remote"
    );
    assert!(outcome.failed.is_none(), "{:?}", outcome.failed);
    assert_eq!(std::fs::read_to_string(repo.join("smoke.txt")).unwrap(), "steered\n");
    assert_ne!(git(&["rev-parse", "HEAD"]), before);
    assert!(git(&["status", "--porcelain"]).is_empty());
    assert!(!outcome.session_id.is_empty());
    assert!(outcome.tool_calls > 0);
    assert!(!outcome.said.is_empty());
    assert!(outcome.cost.is_none());
}
