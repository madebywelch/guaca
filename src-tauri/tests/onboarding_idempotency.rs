#![cfg(feature = "server")]

use serde_json::{json, Value};

#[tokio::test]
async fn lost_hire_and_send_replies_return_the_original_receipts() {
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
    let client = reqwest::Client::new();
    let call = |name: &'static str, args: Value| {
        let client = client.clone();
        async move {
            client
                .post(format!("http://{addr}/v1/call"))
                .bearer_auth("test-token")
                .json(&json!({"name": name, "args": args}))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    let groups = call("list_groups", json!({})).await;
    let group = groups["ok"][0]["id"].as_str().unwrap();
    let hire = json!({"groupId": group, "idempotencyKey": "first-task", "drafts": [{
        "name": "Helper", "avatar": "avocado", "color": "#7ab55c", "model": "", "systemPrompt": "Help."
    }]});
    let (first, retry) = tokio::join!(call("hire_agents", hire.clone()), call("hire_agents", hire));
    assert_eq!(first["ok"][0]["id"], retry["ok"][0]["id"]);
    let agent = first["ok"][0]["id"].as_str().unwrap();
    let send =
        json!({"agentId": agent, "text": "Plan our launch.", "idempotencyKey": "first-task"});
    let (first, retry) =
        tokio::join!(call("send_message", send.clone()), call("send_message", send));
    assert_eq!(first["ok"], retry["ok"]);
    assert!(first["ok"]["messageId"].is_string());
    assert!(first["ok"]["runId"].is_string());
    let messages = call("channel_messages", json!({"channelId": agent})).await;
    assert_eq!(
        messages["ok"].as_array().unwrap().iter().filter(|m| m["from"]["kind"] == "human").count(),
        1
    );
    let roster = call("list_agents", json!({})).await;
    assert_eq!(roster["ok"].as_array().unwrap().len(), 1);
    let legacy =
        call("send_message", json!({"agentId": agent, "text": "A second instruction."})).await;
    assert!(legacy["ok"].is_string());
}
