mod harness;

use std::time::Duration;

use guac_lib::domain::approval::{
    ApprovalState, Decision as ApprovalDecision, ProtectedAction, Request as ApprovalRequest,
};
use guac_lib::domain::ids::RunId;
use guac_lib::domain::now_ms;
use guac_lib::domain::policy::{Class, Condition, Effect, Field, Predicate, RuleDraft};
use guac_lib::runtime::events::UiEvent;
use guac_lib::runtime::guard::GuardLimits;
use harness::*;
use serde_json::{json, Value};

fn policy(effect: Effect, conditions: Vec<Condition>) -> RuleDraft {
    RuleDraft {
        group_id: None,
        effect,
        class: Class::ActOnBehalf,
        tool: None,
        agents: vec![],
        conditions,
    }
}

fn request(arguments: Value) -> Script {
    Script::Plugin { name: "request_permission".into(), arguments }
}

fn standing_grant(h: &Harness, name: &str) {
    let card = h.agent_named(name).unwrap();
    let store = h.runtime.store();
    let request = store
        .create_approval(
            card.id,
            card.group_id,
            RunId::new(),
            ApprovalRequest::Permission { action: ProtectedAction::ActOnBehalf },
            "Wants to send mail",
            &[],
        )
        .unwrap();
    store.settle_approval(request.id, ApprovalState::AlwaysAllow).unwrap();
    assert!(store.has_standing_grant(card.id, ProtectedAction::ActOnBehalf).unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_block_fact_parks_even_with_a_real_standing_grant() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Done".into())
        } else {
            request(json!({"action":"Send the message", "because":"Requested"}))
        }
    })
    .await;
    let h = harness_with_computer(&stub, &["Sender"], GuardLimits::default());
    let first = h.runtime.send_from_human(h.id("Sender"), "First").unwrap();
    let initial = h.awaited_request().await;
    let decided = h.runtime.decide_approval(initial, ApprovalDecision::AlwaysAllow).unwrap();
    assert_eq!(decided.state, ApprovalState::Allow);
    h.settle(first).await;
    assert!(!h
        .runtime
        .store()
        .has_standing_grant(h.id("Sender"), ProtectedAction::ActOnBehalf)
        .unwrap());
    standing_grant(&h, "Sender");

    h.runtime
        .store()
        .create_policy(policy(
            Effect::Block,
            vec![Condition {
                field: Field::Amount,
                predicate: Predicate::GreaterThan { value: "50".into(), currency: "USD".into() },
            }],
        ))
        .unwrap();
    let second = h.runtime.send_from_human(h.id("Sender"), "Second").unwrap();
    h.wait_until("a new permission despite the standing grant", |h| {
        h.sink.count_of(|e| matches!(e, UiEvent::ApprovalRequested { .. })) == 2
    })
    .await;
    let new = h
        .sink
        .snapshot()
        .into_iter()
        .filter_map(|event| match event {
            UiEvent::ApprovalRequested { approval_id, .. } if approval_id != initial => {
                Some(approval_id)
            }
            _ => None,
        })
        .next()
        .unwrap();
    h.runtime.decide_approval(new, ApprovalDecision::Deny).unwrap();
    h.settle(second).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_standing_grant_to_act_in_the_operators_name_is_not_honored() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Done".into())
        } else {
            request(json!({"action":"Send the message", "because":"Requested"}))
        }
    })
    .await;
    let h = harness_with_computer(&stub, &["Sender"], GuardLimits::default());
    standing_grant(&h, "Sender");

    let run = h.runtime.send_from_human(h.id("Sender"), "Send it").unwrap();
    let asked = tokio::time::timeout(Duration::from_secs(10), h.awaited_request())
        .await
        .expect("a standing grant to act in the operator's name let the request through unasked");
    h.runtime.decide_approval(asked, ApprovalDecision::Deny).unwrap();
    h.settle(run).await;
}

async fn predicate_case(predicate: Predicate, field: Field, facts: Value, outcome: &str) {
    let mut args = json!({"action":"Send the message", "because":"Requested"});
    args.as_object_mut().unwrap().extend(facts.as_object().unwrap().clone());
    let stub = serve(move |body| {
        if has_tool_result(body) {
            Script::Say("Done".into())
        } else {
            request(args.clone())
        }
    })
    .await;
    let h = harness_with_computer(&stub, &["Sender"], GuardLimits::default());
    h.runtime.store().create_policy(policy(Effect::Allow, vec![])).unwrap();
    h.runtime
        .store()
        .create_policy(policy(Effect::Block, vec![Condition { field, predicate }]))
        .unwrap();
    let run = h.runtime.send_from_human(h.id("Sender"), "Send it").unwrap();
    if outcome == "ask" {
        let approval = tokio::time::timeout(Duration::from_secs(10), h.awaited_request())
            .await
            .expect("missing or invalid block fact must ask");
        h.runtime.decide_approval(approval, ApprovalDecision::Deny).unwrap();
        h.settle(run).await;
    } else {
        h.settle(run).await;
        assert_eq!(h.sink.count_of(|e| matches!(e, UiEvent::ApprovalRequested { .. })), 0);
        let text = tool_results(&stub).join("\n").to_lowercase();
        if outcome == "block" {
            assert!(text.contains("refused"), "{text}");
        } else {
            assert!(text.contains("allowed"), "{text}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn outside_domain_and_greater_than_block_allow_and_ask() {
    for (facts, outcome) in [
        (json!({"to":["a@evil.com"]}), "block"),
        (json!({"to":["a@sub.example.com"]}), "allow"),
        (json!({}), "ask"),
    ] {
        predicate_case(
            Predicate::OutsideDomain { value: "example.com".into() },
            Field::Recipients,
            facts,
            outcome,
        )
        .await;
    }
    for (facts, outcome) in [
        (json!({"amount":"50.01", "currency":"USD"}), "block"),
        (json!({"amount":"50", "currency":"USD"}), "allow"),
        (json!({"amount":"51", "currency":"EUR"}), "ask"),
    ] {
        predicate_case(
            Predicate::GreaterThan { value: "50".into(), currency: "USD".into() },
            Field::Amount,
            facts,
            outcome,
        )
        .await;
    }
}

async fn allowed_after_a_lapse(
    arguments: Value,
    rule: RuleDraft,
) -> (Stub, Harness, String, RunId) {
    let stub = serve(move |body| {
        if has_tool_result(body) {
            Script::Say("Done".into())
        } else {
            request(arguments.clone())
        }
    })
    .await;
    let h = harness_with_computer(&stub, &["Sender"], GuardLimits::default());
    h.runtime.set_approval_window(Duration::from_secs(1));
    let first = h.runtime.send_from_human(h.id("Sender"), "Send it").unwrap();
    h.awaited_request().await;
    h.settle(first).await;
    let filed = h.runtime.store().decisions(Some(h.id("Sender"))).unwrap().remove(0);
    let fingerprint: String = h
        .runtime
        .store()
        .conn()
        .unwrap()
        .query_row(
            "SELECT fingerprint FROM permission_grants WHERE decision_id=?1",
            [filed.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();

    h.runtime.store().create_policy(rule).unwrap();
    h.runtime.set_approval_window(Duration::from_secs(60));
    let answered = h.runtime.answer_decision(filed.id, "Allow", false, None).unwrap();
    (stub, h, fingerprint, answered.delivery_run.expect("delivered"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_block_rule_beats_an_allow_given_on_for_you() {
    let block = policy(
        Effect::Block,
        vec![Condition {
            field: Field::Recipients,
            predicate: Predicate::OutsideDomain { value: "example.com".into() },
        }],
    );
    let (stub, h, fingerprint, delivery) = allowed_after_a_lapse(
        json!({"action":"Send the message", "because":"Requested", "to":["a@evil.com"]}),
        block,
    )
    .await;
    h.settle(delivery).await;
    assert_eq!(h.sink.count_of(|e| matches!(e, UiEvent::ApprovalRequested { .. })), 1);
    let told = last_tool_results(&stub).join("\n");
    assert!(told.starts_with("Refused: the operator's rules never allow this"), "{told}");
    assert!(
        h.runtime.store().take_grant(h.id("Sender"), &fingerprint, now_ms()).unwrap(),
        "the block refused it without spending the grant"
    );

    let unsure = policy(
        Effect::Block,
        vec![Condition {
            field: Field::Amount,
            predicate: Predicate::GreaterThan { value: "50".into(), currency: "USD".into() },
        }],
    );
    let (_stub, h, fingerprint, delivery) = allowed_after_a_lapse(
        json!({"action":"Pay the invoice", "because":"Requested", "amount":"51", "currency":"EUR"}),
        unsure,
    )
    .await;
    h.wait_until("the repeat to park despite the grant", |h| {
        h.sink.count_of(|e| matches!(e, UiEvent::ApprovalRequested { .. })) == 2
    })
    .await;
    h.runtime.stop_run(delivery);
    h.settle(delivery).await;
    assert!(h.runtime.store().take_grant(h.id("Sender"), &fingerprint, now_ms()).unwrap());
}
