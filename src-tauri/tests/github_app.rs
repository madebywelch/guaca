//! Real credential-helper and gh processes against an authenticated local broker.
#![cfg(unix)]
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use guac_lib::repo::{auth, github};
use serde_json::{json, Value};
use std::{
    os::unix::fs::PermissionsExt,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn git_and_gh_share_app_access_across_worktrees_and_disconnect_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let requests = Arc::new(AtomicUsize::new(0));
    async fn token(
        State(requests): State<Arc<AtomicUsize>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Result<Json<Value>, StatusCode> {
        if headers.get("authorization").and_then(|h| h.to_str().ok())
            != Some("Bearer test-broker-authorization")
            || body["repository"] != "owner/repo"
        {
            return Err(StatusCode::FORBIDDEN);
        }
        let count = requests.fetch_add(1, Ordering::SeqCst);
        Ok(Json(
            json!({"token":if count < 2 {"first-fixture-token"} else {"replacement-fixture-token"}, "expiresAt":"2099-01-01T00:00:00Z"}),
        ))
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/v1/token", post(token))
                .route("/v1/user/token", post(token))
                .with_state(requests),
        )
        .await
        .unwrap();
    });
    let broker_token = root.join("broker-token");
    std::fs::write(&broker_token, "test-broker-authorization").unwrap();
    std::env::set_var("GUACA_GITHUB_BROKER", url);
    std::env::set_var("GUACA_GITHUB_BROKER_TOKEN_FILE", &broker_token);
    let checkout = root.join("checkout");
    std::fs::create_dir(&checkout).unwrap();
    let git = |args: &[&str]| {
        let result =
            std::process::Command::new("git").arg("-C").arg(&checkout).args(args).output().unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.name", "Fixture"]);
    git(&["config", "user.email", "fixture@example.com"]);
    git(&["config", "commit.gpgsign", "false"]);
    git(&["remote", "add", "origin", "https://github.com/owner/repo.git"]);
    git(&["commit", "--allow-empty", "-m", "fixture"]);
    let credential = root.join("operator's credentials/repository");
    let file = github::file(&credential);
    github::prepare(&file, "https://github.com/owner/repo.git").await.unwrap();
    github::attach(checkout.to_str().unwrap(), &file).await.unwrap();
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(!saved.contains("fixture-token") && !saved.contains("authorization"));
    let bench = root.join("bench");
    git(&["worktree", "add", "-b", "engineer", bench.to_str().unwrap()]);
    async fn fill(path: &std::path::Path, remote: &str) -> std::process::Output {
        use tokio::io::AsyncWriteExt;
        let mut process = tokio::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["credential", "fill"])
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        process
            .stdin
            .take()
            .unwrap()
            .write_all(format!("url={remote}\n\n").as_bytes())
            .await
            .unwrap();
        process.wait_with_output().await.unwrap()
    }
    let first = fill(&bench, "https://github.com/owner/repo.git").await;
    assert!(first.status.success());
    assert!(String::from_utf8_lossy(&first.stdout).contains("first-fixture-token"));
    assert!(!fill(&bench, "https://github.com/owner/other.git").await.status.success());
    let replacement = fill(&bench, "https://github.com/owner/repo.git").await;
    assert!(String::from_utf8_lossy(&replacement.stdout).contains("replacement-fixture-token"));
    let bin = root.join("bin");
    std::fs::create_dir(&bin).unwrap();
    std::fs::write(bin.join("gh"), "#!/bin/sh\n[ \"$GH_TOKEN\" = replacement-fixture-token ] && [ \"$GH_REPO\" = owner/repo ] && echo authenticated\n").unwrap();
    std::fs::set_permissions(bin.join("gh"), std::fs::Permissions::from_mode(0o700)).unwrap();
    let old_path = std::env::var_os("PATH").unwrap();
    std::env::set_var("PATH", format!("{}:{}", bin.display(), old_path.to_string_lossy()));
    let command = guac_lib::shell::run(
        bench.to_str().unwrap(),
        "gh pr list",
        std::time::Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(command.stdout.trim(), "authenticated", "{command:?}");
    let other = guac_lib::shell::run(
        bench.to_str().unwrap(),
        "gh pr list --repo owner/other",
        std::time::Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_ne!(other.exit_code, Some(0));
    let revoked = auth::clear(checkout.to_str().unwrap(), &credential).await.unwrap();
    assert!(!revoked.github_app);
    assert!(!fill(&bench, "https://github.com/owner/repo.git").await.status.success());
    let stopped = guac_lib::shell::run(
        bench.to_str().unwrap(),
        "gh pr list",
        std::time::Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_ne!(stopped.exit_code, Some(0));
    // Explicitly replacing App access with a PAT restores the normal gh path.
    auth::set(checkout.to_str().unwrap(), &credential, "git", "pat-fixture").await.unwrap();
    assert!(github::attached(bench.to_str().unwrap()).await.is_none());
    assert!(!file.exists());
    assert!(String::from_utf8_lossy(
        &fill(&bench, "https://github.com/owner/repo.git").await.stdout
    )
    .contains("pat-fixture"));
    std::env::set_var("PATH", old_path);
    server.abort();
}
