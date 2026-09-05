//! Attribution survives cloning, credential changes and an engineer's worktree.
use guac_lib::{
    domain::repository::GitIdentity,
    repo::{self, auth},
};
use std::{path::Path, process::Command};

fn git(path: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .env_remove("GIT_AUTHOR_NAME")
        .env_remove("GIT_AUTHOR_EMAIL")
        .env_remove("GIT_COMMITTER_NAME")
        .env_remove("GIT_COMMITTER_EMAIL")
        .output()
        .unwrap()
}
fn good(path: &Path, args: &[&str]) -> String {
    let result = git(path, args);
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    String::from_utf8(result.stdout).unwrap().trim().into()
}

#[tokio::test]
async fn explicit_author_replaces_legacy_identity_without_changing_history_or_access() {
    let dir = tempfile::tempdir().unwrap();
    // This test binary has one test. No developer config can supply an identity.
    std::env::set_var("GIT_CONFIG_GLOBAL", dir.path().join("global"));
    std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    let seed = dir.path().join("seed");
    std::fs::create_dir(&seed).unwrap();
    good(&seed, &["init", "-b", "main"]);
    good(
        &seed,
        &[
            "-c",
            "user.name=Original",
            "-c",
            "user.email=original@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "original",
        ],
    );
    let path =
        repo::clone_remote(&format!("file://{}", seed.display()), &dir.path().join("clone"), None)
            .await
            .unwrap();
    let checkout = Path::new(&path);
    assert_eq!(auth::identity(&path).await.unwrap().name, "");
    assert!(!git(checkout, &["commit", "--allow-empty", "-m", "must not invent an author"])
        .status
        .success());

    let legacy = GitIdentity { name: "guaca".into(), email: "guaca@localhost".into() };
    auth::set_identity(&path, &legacy).await.unwrap();
    for (name, email) in [
        ("", "person@example.com"),
        ("Person\nInjected", "person@example.com"),
        ("Person", "missing"),
        ("Person", "person@example.com\nInjected"),
        ("Person", "<person@example.com>"),
    ] {
        assert!(auth::set_identity(&path, &GitIdentity { name: name.into(), email: email.into() })
            .await
            .is_err());
        assert_eq!(
            auth::identity(&path).await.unwrap(),
            legacy,
            "invalid updates do not partially write"
        );
    }
    let bench = dir.path().join("engineer");
    good(checkout, &["worktree", "add", "-b", "engineer", bench.to_str().unwrap()]);
    let author = GitIdentity {
        name: "Human Engineer".into(),
        email: "123+engineer@users.noreply.github.com".into(),
    };
    auth::set_identity(&path, &author).await.unwrap();
    good(checkout, &["remote", "set-url", "origin", "https://github.com/team/repo.git"]);
    let credential = dir.path().join("credentials");
    let connected = auth::set(&path, &credential, "x-access-token", "fixture-token").await.unwrap();
    assert_eq!(connected.author, author);
    assert!(connected.managed_credential);
    good(&bench, &["commit", "--allow-empty", "-m", "human attribution"]);
    assert_eq!(good(&bench, &["log", "-1", "--format=%an <%ae>|%cn <%ce>"]), "Human Engineer <123+engineer@users.noreply.github.com>|Human Engineer <123+engineer@users.noreply.github.com>");
    assert_eq!(
        good(&bench, &["log", "-1", "HEAD~1", "--format=%an <%ae>"]),
        "Original <original@example.com>"
    );
    assert_eq!(auth::clear(&path, &credential).await.unwrap().author, author);
    assert_eq!(auth::identity(bench.to_str().unwrap()).await.unwrap(), author);

    // A subsequent clone inherits a configured backend author without overriding it.
    std::fs::write(
        dir.path().join("global"),
        "[user]\nname = Backend User\nemail = backend@example.com\n",
    )
    .unwrap();
    let inherited = repo::clone_remote(
        &format!("file://{}", seed.display()),
        &dir.path().join("inherited"),
        None,
    )
    .await
    .unwrap();
    assert_eq!(auth::identity(&inherited).await.unwrap().name, "Backend User");
}
