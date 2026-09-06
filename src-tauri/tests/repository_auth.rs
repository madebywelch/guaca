//! Real Git smart HTTP, including credentials and linked worktrees. No forge
//! account, model call, or internet connection is used.

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, Uri},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use guac_lib::repo::{self, auth};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
struct Forge {
    root: std::path::PathBuf,
    password: Arc<Mutex<String>>,
}

async fn serve(
    State(forge): State<Forge>,
    method: axum::http::Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request = reqwest::Client::new()
        .get("http://localhost")
        .basic_auth("engineer", Some(forge.password.lock().unwrap().clone()))
        .build()
        .unwrap();
    let expected = request.headers()["authorization"].to_str().unwrap();
    if headers.get("authorization").and_then(|h| h.to_str().ok()) != Some(expected) {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            [("www-authenticate", "Basic realm=forge")],
            "sign in",
        )
            .into_response();
    }
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new("git")
        .arg("http-backend")
        .env("GIT_PROJECT_ROOT", &forge.root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REMOTE_USER", "engineer")
        .env("REQUEST_METHOD", method.as_str())
        .env("PATH_INFO", uri.path())
        .env("QUERY_STRING", uri.query().unwrap_or_default())
        .env(
            "CONTENT_TYPE",
            headers.get("content-type").and_then(|h| h.to_str().ok()).unwrap_or_default(),
        )
        .env("CONTENT_LENGTH", body.len().to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&body).await.unwrap();
    let output = child.wait_with_output().await.unwrap();
    assert!(output.status.success());
    let split = output.stdout.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let mut response = axum::http::Response::builder();
    for line in String::from_utf8_lossy(&output.stdout[..split]).lines() {
        let (name, value) = line.split_once(':').unwrap();
        if name.eq_ignore_ascii_case("status") {
            response =
                response.status(value.trim().split(' ').next().unwrap().parse::<u16>().unwrap());
        } else {
            response = response.header(name, value.trim());
        }
    }
    response.body(axum::body::Body::from(output.stdout[split + 4..].to_vec())).unwrap()
}

fn git(path: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args([
            "-c",
            "user.name=Engineer",
            "-c",
            "user.email=engineer@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
}

fn good(path: &Path, args: &[&str]) -> String {
    let output = git(path, args);
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().into()
}

#[tokio::test(flavor = "multi_thread")]
async fn authenticated_clone_pull_worktree_push_rotation_and_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let bare = dir.path().join("repo.git");
    good(dir.path(), &["init", "--bare", "--initial-branch=main", bare.to_str().unwrap()]);
    good(&bare, &["config", "http.receivepack", "true"]);
    let seed = dir.path().join("seed");
    good(dir.path(), &["clone", bare.to_str().unwrap(), seed.to_str().unwrap()]);
    std::fs::write(seed.join("a.txt"), "initial").unwrap();
    good(&seed, &["add", "."]);
    good(&seed, &["commit", "-m", "initial"]);
    good(&seed, &["push", "origin", "main"]);
    let password = Arc::new(Mutex::new("first/token with spaces".into()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote = format!("http://{}/repo.git", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/*path", any(serve))
        .with_state(Forge { root: dir.path().into(), password: password.clone() });
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let credential =
        dir.path().join("operator's credentials").join(uuid::Uuid::new_v4().to_string());
    auth::keep(&credential, &remote, "engineer", "wrong").await.unwrap();
    let checkout = dir.path().join("clone");
    assert!(repo::clone_remote(&remote, &checkout, Some(&credential)).await.is_err());
    auth::keep(&credential, &remote, "engineer", "first/token with spaces").await.unwrap();
    let path = repo::clone_remote(&remote, &checkout, Some(&credential)).await.unwrap();
    assert_eq!(std::fs::read_to_string(checkout.join("a.txt")).unwrap(), "initial");
    let config = std::fs::read_to_string(checkout.join(".git/config")).unwrap();
    assert!(!config.contains("first/token") && !config.contains("first%2Ftoken"));
    assert!(auth::connection(&path, &credential).await.unwrap().managed_credential);
    // A second setup reuses the first token without a value crossing IPC.
    let second_bare = dir.path().join("second.git");
    good(dir.path(), &["clone", "--bare", bare.to_str().unwrap(), second_bare.to_str().unwrap()]);
    let second_remote = remote.replace("repo.git", "second.git");
    let choices =
        repo::credentials::list(credential.parent().unwrap(), &second_remote).await.unwrap();
    assert_eq!(choices.len(), 1);
    let second_credential = credential.parent().unwrap().join(uuid::Uuid::new_v4().to_string());
    repo::credentials::keep(
        credential.parent().unwrap(),
        &choices[0].id,
        &second_credential,
        &second_remote,
    )
    .await
    .unwrap();
    let second = dir.path().join("second-clone");
    let second_path =
        repo::clone_remote(&second_remote, &second, Some(&second_credential)).await.unwrap();
    auth::clear(&second_path, &second_credential).await.unwrap();
    assert!(!git(&second, &["fetch", "origin"]).status.success());
    good(&checkout, &["fetch", "origin"]);

    let before = good(&bare, &["show-ref"]);
    assert!(auth::check(&path).await.unwrap().contains("No remote refs changed"));
    assert_eq!(good(&bare, &["show-ref"]), before);

    std::fs::write(seed.join("a.txt"), "upstream").unwrap();
    good(&seed, &["commit", "-am", "upstream"]);
    good(&seed, &["push", "origin", "main"]);
    good(&checkout, &["pull", "--ff-only"]);
    assert_eq!(std::fs::read_to_string(checkout.join("a.txt")).unwrap(), "upstream");
    let bench = dir.path().join("engineer worktree");
    good(&checkout, &["worktree", "add", "-b", "engineer", bench.to_str().unwrap()]);
    std::fs::write(bench.join("a.txt"), "engineer change").unwrap();
    good(&bench, &["commit", "-am", "engineer change"]);
    good(&bench, &["push", "origin", "engineer"]);
    assert_eq!(good(&bare, &["show", "engineer:a.txt"]), "engineer change");

    // Same host, different repository: no credential should be offered.
    use std::io::Write;
    let mut fill = std::process::Command::new("git")
        .arg("-C")
        .arg(&checkout)
        .args(["credential", "fill"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    fill.stdin
        .take()
        .unwrap()
        .write_all(format!("url={}\n\n", remote.replace("repo.git", "other.git")).as_bytes())
        .unwrap();
    assert!(!fill.wait().unwrap().success());

    *password.lock().unwrap() = "replacement".into();
    assert!(auth::check(&path).await.unwrap_err().to_string().contains("read check failed"));
    auth::set(&path, &credential, "engineer", "replacement").await.unwrap();
    good(&bench, &["fetch", "origin"]);
    auth::clear(&path, &credential).await.unwrap();
    assert!(!git(&bench, &["fetch", "origin"]).status.success());
    assert!(!auth::connection(&path, &credential).await.unwrap().managed_credential);
    server.abort();
}

#[test]
fn token_origins_reject_plaintext_and_embedded_secrets() {
    for url in [
        "http://forge.example/r.git",
        "https://user:secret@forge.example/r.git",
        "https://forge.example/r.git?token=secret",
    ] {
        let error = auth::https_remote(url).unwrap_err().to_string();
        assert!(!error.contains("secret"));
    }
    assert!(auth::https_remote("https://forge.example:8443/owner/repo.git").is_ok());
}

#[tokio::test]
async fn saved_credentials_are_private_descriptions_and_reuse_checks_the_origin() {
    use repo::credentials;
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("credentials");
    let id = uuid::Uuid::new_v4().to_string();
    let file = store.join(&id);
    let destination = store.join(uuid::Uuid::new_v4().to_string());
    let remote = "https://forge.example/team/first.git";
    auth::keep(&file, remote, "engineer@example.com", "tok/1 + %").await.unwrap();

    for invalid in [
        "https://other.example/team/second.git",
        "https://forge.example:8443/team/second.git",
        "http://forge.example/team/second.git",
        "https://forge.example.evil.test/team/second.git",
        "https://secret@forge.example/team/second.git",
    ] {
        assert!(credentials::list(&store, invalid).await.unwrap().is_empty());
        assert!(credentials::keep(&store, &id, &destination, invalid).await.is_err());
        assert!(!destination.exists());
    }
    for invalid in ["../outside", "/etc/passwd", "", "github-helper.py"] {
        assert!(credentials::keep(&store, invalid, &destination, remote).await.is_err());
    }
    let next = "https://forge.example/team/second.git";
    let saved = credentials::list(&store, next).await.unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].id, id);
    assert_eq!(saved[0].username, "engineer@example.com");
    let metadata = serde_json::to_string(&saved).unwrap();
    assert!(!metadata.contains("tok") && !metadata.contains("%2F"));
    credentials::keep(&store, &id, &destination, next).await.unwrap();
    let copied = std::fs::read_to_string(&destination).unwrap();
    assert!(copied.ends_with("/team/second.git\n"));
    assert!(copied.contains("tok%2F1%20%2B%20%25"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&destination).unwrap().permissions().mode() & 0o777, 0o600);
    }
    std::fs::remove_file(&file).unwrap();
    assert!(credentials::keep(&store, &id, &destination, next).await.is_err());
    assert_eq!(std::fs::read_to_string(&destination).unwrap(), copied);
    assert_eq!(credentials::list(&store, next).await.unwrap().len(), 1);
}

#[tokio::test]
async fn saved_credentials_ignore_non_credentials_and_survive_reopening() {
    use repo::credentials;
    let dir = tempfile::tempdir().unwrap();
    let remote = "https://forge.example/team/repo.git";
    assert!(credentials::list(&dir.path().join("absent"), remote).await.unwrap().is_empty());
    let id = uuid::Uuid::new_v4().to_string();
    auth::keep(&dir.path().join(&id), remote, "git", "secret").await.unwrap();
    std::fs::write(dir.path().join("github-helper.py"), "not a credential").unwrap();
    std::fs::write(dir.path().join(uuid::Uuid::new_v4().to_string()), "malformed secret").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        dir.path().join(&id),
        dir.path().join(uuid::Uuid::new_v4().to_string()),
    )
    .unwrap();
    let saved = credentials::list(dir.path(), remote).await.unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].id, id);
}

#[tokio::test]
async fn the_most_recently_saved_credential_is_offered_first() {
    use repo::credentials;
    let dir = tempfile::tempdir().unwrap();
    let remote = "https://forge.example/team/repo.git";
    let mut ids = Vec::new();
    for seconds in [10, 30, 20] {
        let id = uuid::Uuid::new_v4().to_string();
        let file = dir.path().join(&id);
        auth::keep(&file, remote, "engineer", "token").await.unwrap();
        std::fs::File::open(file)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds),
            ))
            .unwrap();
        ids.push(id);
    }
    let saved = credentials::list(dir.path(), remote).await.unwrap();
    assert_eq!(
        saved.iter().map(|saved| saved.id.as_str()).collect::<Vec<_>>(),
        vec![ids[1].as_str(), ids[2].as_str(), ids[0].as_str()]
    );
}
