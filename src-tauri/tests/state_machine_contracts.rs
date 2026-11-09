use guac_lib::db::Store;
use guac_lib::domain::agent::CleanDraft;
use guac_lib::domain::envelope::{Envelope, Intent, Part, Participant, Trust};
use guac_lib::domain::ids::{MessageId, RunId};
use guac_lib::runtime::events::Activity;

#[test]
fn queued_work_blocks_immediate_delivery_at_every_depth() {
    for depth in [0, 1, 2, usize::MAX] {
        assert!(Activity::Queued { depth }.is_working());
    }
    assert!(!Activity::Idle.is_working());
}

#[test]
fn settling_a_run_does_not_forget_its_operator_send_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("guac.db");
    let store = Store::open(&path).unwrap();
    let agent = store
        .create_agent(&CleanDraft {
            group_id: None,
            name: "Scout".into(),
            avatar: "orb".into(),
            color: "#7fb069".into(),
            model: "test".into(),
            subscription_model: None,
            reasoning_effort: None,
            system_prompt: "test".into(),
            skills: vec![],
        })
        .unwrap();
    let message = Envelope {
        id: MessageId::new(),
        run_id: RunId::new(),
        channel_id: agent.id,
        from: Participant::Human,
        to: Participant::Agent { id: agent.id },
        parts: vec![Part::text("Start")],
        trust: Trust::Operator,
        hop: 0,
        expects_reply: true,
        intent: Intent::Work,
        cause: None,
        created_at: 1000,
    };
    assert_eq!(store.operator_send_receipt("key", "fingerprint").unwrap(), None);
    assert_eq!(
        store.claim_operator_send("key", "fingerprint", &message, true).unwrap(),
        (message.id, message.run_id, true)
    );
    store.settle_run(message.run_id).unwrap();
    drop(store);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(
        reopened.operator_send_receipt("key", "fingerprint").unwrap(),
        Some((message.id, message.run_id))
    );
    assert!(reopened.operator_send_receipt("key", "other").is_err());
    assert_eq!(
        reopened.claim_operator_send("key", "fingerprint", &message, false).unwrap(),
        (message.id, message.run_id, false)
    );
}
