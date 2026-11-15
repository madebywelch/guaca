mod harness;
use std::time::Duration;

use guac_lib::config::{agent_patch, AppConfig, SettingsPatch};
use guac_lib::domain::approval::Decision;
use guac_lib::runtime::events::UiEvent;
use guac_lib::runtime::guard::GuardLimits;
use harness::*;
use serde_json::json;

fn patch(value: serde_json::Value) -> SettingsPatch {
    serde_json::from_value(value).unwrap()
}

#[test]
fn interleaved_partial_limit_patches_preserve_other_fields_and_last_writer_wins() {
    let mut config = AppConfig::default();
    config.apply(patch(json!({"limits": {"maxHops": 12}}))).unwrap();
    config.apply(patch(json!({"limits": {"maxStepsPerRun": 80}}))).unwrap();
    assert_eq!(config.limits.max_hops, 12);
    assert_eq!(config.limits.max_steps_per_run, 80);
    config.apply(patch(json!({"limits": {"maxHops": 10}}))).unwrap();
    assert_eq!(config.limits.max_hops, 10);
    assert_eq!(config.limits.max_steps_per_run, 80);
}

#[test]
fn old_full_patches_and_persisted_limits_remain_readable() {
    let mut config = AppConfig::default();
    let old = json!({"maxHops": 9, "maxStepsPerRun": 70, "maxFanoutPerCall": 4,
        "maxSendsPerPair": 5, "maxToolRounds": 20});
    config.apply(patch(json!({"limits": old}))).unwrap();
    assert_eq!(serde_json::to_value(config.limits).unwrap(), old);
    let saved = serde_json::to_value(&config).unwrap();
    let reloaded: AppConfig = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(serde_json::to_value(reloaded).unwrap(), saved);
}

#[test]
fn agent_limit_validation_keeps_its_previous_error() {
    let error =
        agent_patch(&json!({"limits": {"maxHops": null}}), &AppConfig::default()).unwrap_err();
    assert_eq!(error, "those values could not be read (invalid type: null, expected u16). `read` shows each setting's type.");
    let error = agent_patch(
        &json!({"limits": {"maxHops": null}, "provider": "compatible"}),
        &AppConfig::default(),
    )
    .unwrap_err();
    assert!(error.starts_with("`provider` is the operator's to change"));
    let error = agent_patch(
        &json!({"browserIdleMinutes": "bad", "limits": {"maxHops": null}}),
        &AppConfig::default(),
    )
    .unwrap_err();
    assert_eq!(error, "those values could not be read (invalid type: string \"bad\", expected u32). `read` shows each setting's type.");
}

// ---- through the runtime, with the card up -------------------------------

/// A stub that asks for a deeper relay on every turn, and says so once it has
/// a result back. Each turn starts without a tool result, so a turn that an
/// answer from For you starts asks again.
async fn asks_for_relay_depth(hops: u16) -> Stub {
    serve(move |body| {
        if has_tool_result(body) {
            Script::Say("Asked.".into())
        } else {
            Script::Settings(json!({
                "action": "update", "changes": { "limits": { "maxHops": hops } },
            }))
        }
    })
    .await
}

#[tokio::test]
async fn an_operator_change_made_while_the_card_is_up_survives_the_allow() {
    // The card can be up for ten minutes. A limit the agent never named, saved
    // by the operator in that time, is theirs, and an Allow for something else
    // must not put it back to what it was when the agent asked.
    let before = GuardLimits::default();
    let stub = asks_for_relay_depth(before.max_hops + 4).await;
    let h = harness(&stub, &["Helper"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Helper"), "Let the group relay further.").unwrap();
    let request = h.awaited_request().await;

    let theirs = before.max_steps_per_run + 20;
    h.runtime
        .change_config(|config| config.apply(patch(json!({"limits": {"maxStepsPerRun": theirs}}))))
        .unwrap();
    h.runtime.decide_approval(request, Decision::Allow).unwrap();
    h.wait_until("the change lands", |h| h.runtime.config().limits.max_hops == before.max_hops + 4)
        .await;
    let now = h.runtime.config().limits;
    assert_eq!(now.max_steps_per_run, theirs, "the Allow undid the operator's change");
    assert_eq!(now.max_fanout_per_call, before.max_fanout_per_call);
    assert_eq!(now.max_sends_per_pair, before.max_sends_per_pair);
    assert_eq!(now.max_tool_rounds, before.max_tool_rounds);
    h.settle(run).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_settings_grant_applies_only_the_limit_it_was_given_for() {
    // A settings change nobody answered is filed, and its Allow is a one-time
    // grant for that diff. The operator moves a limit the agent never named
    // before the agent asks again. The grant still matches, because that
    // limit was never in the diff, and spending it changes the one limit the
    // agent named and leaves the operator's where they put it.
    let before = GuardLimits::default();
    let stub = asks_for_relay_depth(before.max_hops + 3).await;
    let h = harness(&stub, &["Helper"], GuardLimits::default());
    h.runtime.set_approval_window(Duration::from_secs(1));
    let run = h.runtime.send_from_human(h.id("Helper"), "Raise the relay depth.").unwrap();
    h.awaited_request().await;
    h.settle(run).await;
    let desk = h.runtime.store().decisions(Some(h.id("Helper"))).unwrap();
    assert_eq!(desk.len(), 1, "the lapsed change is on the desk: {desk:?}");

    let theirs = before.max_steps_per_run + 20;
    h.runtime
        .change_config(|config| config.apply(patch(json!({"limits": {"maxStepsPerRun": theirs}}))))
        .unwrap();
    h.runtime.set_approval_window(Duration::from_secs(60));
    let answered = h.runtime.answer_decision(desk[0].id, "Allow", false, None).unwrap();
    h.settle(answered.delivery_run.expect("delivered")).await;

    let asked = h.sink.count_of(|e| matches!(e, UiEvent::ApprovalRequested { .. }));
    assert_eq!(asked, 1, "the repeat went through on the grant:\n{}", h.transcript());
    let now = h.runtime.config().limits;
    assert_eq!(now.max_hops, before.max_hops + 3);
    assert_eq!(now.max_steps_per_run, theirs);
}
