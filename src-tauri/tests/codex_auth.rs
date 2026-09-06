//! The real CLI, isolated from the operator's credentials. Run this test binary
//! alone: its environment belongs to the test process, never the running app.
use guac_lib::{coding, domain::repository::Harness};

#[tokio::test]
#[ignore = "live: requires the official Codex CLI; no credentials or model spend"]
async fn unsigned_codex_reports_setup_before_inference() {
    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("config.toml"), "cli_auth_credentials_store = \"file\"\n")
        .unwrap();
    std::env::set_var("CODEX_HOME", home.path());
    std::env::remove_var("OPENAI_API_KEY");
    std::env::remove_var("CODEX_API_KEY");
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(40),
        coding::run(
            Harness::Codex,
            repo.path().to_str().unwrap(),
            "Do not use tools.",
            None,
            |_| {},
        ),
    )
    .await
    .expect("the authentication check must finish without inference");
    let error = result.unwrap_err().to_string();
    assert!(error.contains("Codex is not signed in on this backend"), "{error}");
    assert!(error.contains("codex login --device-auth"), "{error}");
}
