#![cfg(feature = "server")]

use serde_json::json;

#[tokio::test]
async fn legacy_client_cannot_approve_on_a_new_host() {
    let dir = tempfile::tempdir().unwrap();
    let bound = guac_lib::server::bind(guac_lib::server::Settings {
        root: dir.path().to_path_buf(),
        bind: "127.0.0.1:0".parse().unwrap(),
        token: "test-token".into(),
        web: None,
        origin: None,
        updater: None,
    })
    .await
    .unwrap();
    let addr = bound.addr;
    tokio::spawn(bound.serve());
    let response: serde_json::Value = reqwest::Client::new()
        .post(format!("http://{addr}/v1/call"))
        .bearer_auth("test-token")
        .json(&json!({"name": "allow_artifact_sources", "args": {"id": "00000000-0000-4000-8000-000000000001"}}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(response["err"]["kind"], "clientOutdated");
    assert_eq!(response["err"]["message"], "Update Guaca on this device to approve this page.");
}
