//! Explicitly opted-in contract check against the official CLI. A dedicated
//! test binary isolates its PATH wrapper from all offline stand-ins.
#![cfg(unix)]

#[tokio::test]
#[ignore = "live: uses Codex login with gpt-5.4-mini, never the configured default model"]
async fn codex_mini_edits_and_commits_through_the_real_runner() {
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
    let real = binary.to_string_lossy().replace('\'', "'\\''");
    let wrapper = bin.join("codex");
    std::fs::write(&wrapper, format!("#!/bin/sh\nshift\nunset CODEX_API_KEY OPENAI_API_KEY\nexec '{real}' exec --ignore-user-config --ignore-rules --model gpt-5.4-mini -c 'model_reasoning_effort=\"low\"' \"$@\"\n")).unwrap();
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
    let result = tokio::time::timeout(Duration::from_secs(120), coding::run(
        Harness::Codex, repo.to_str().unwrap(),
        "Only create smoke.txt containing exactly verified followed by a newline. Check its contents and commit it with message test: verify Codex runner. Do not contact any remote service, spawn subagents, or change any other file. Reply with one short sentence.",
        None, |_| {},
    )).await;
    std::env::set_var("PATH", &path);
    let outcome = result.expect("Codex exceeded two minutes").expect("Codex could not run");
    assert!(outcome.failed.is_none(), "{:?}", outcome.failed);
    assert_eq!(std::fs::read_to_string(repo.join("smoke.txt")).unwrap(), "verified\n");
    assert_ne!(git(&["rev-parse", "HEAD"]), before);
    assert!(git(&["status", "--porcelain"]).is_empty());
    assert!(!outcome.session_id.is_empty());
    assert!(outcome.tool_calls > 0);
    assert!(!outcome.said.is_empty());
    assert!(outcome.cost.is_none());
}
