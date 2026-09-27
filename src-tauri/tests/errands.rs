//! Errands, end to end: the agent's own model sent out with a brief, inside
//! the turn that sent it.
//!
//! The same scripted server the cascade suite drives, answering as whichever
//! speaker is asking. An errand's prompt opens "You are on an errand for
//! Writer", so `speaker` reads it as "on an errand for Writer" and never as
//! Writer: a stub that could not tell the two apart could not script either.
//!
//! What these hold is the part of the design that is not wording: who is
//! offered errands, what an errand may not do, what it spends and from where,
//! and that a stop reaches it. What a real model does with the tool is the
//! evals' question.

mod harness;

use std::sync::atomic::Ordering;
use std::time::Duration;

use guac_lib::domain::envelope::{NoticeKind, Part, ToolOutcome};
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::trajectory::Anomaly;

use harness::*;

const ERRAND: &str = "on an errand for Writer";

fn errand_requests(stub: &Stub) -> Vec<serde_json::Value> {
    stub.transcript.lock().iter().filter(|body| speaker(body) == ERRAND).cloned().collect()
}

fn grant(h: &Harness, name: &str) {
    h.runtime.store().set_runs_errands(h.id(name), true).unwrap();
}

/// Every tool call on the record of the given agent's channel, by name and
/// outcome, in order.
fn recorded_calls(h: &Harness, name: &str) -> Vec<(String, ToolOutcome)> {
    h.envelopes(&[name])
        .iter()
        .flat_map(|envelope| envelope.parts.clone())
        .filter_map(|part| match part {
            Part::ToolCall { name, outcome, .. } => Some((name, outcome)),
            _ => None,
        })
        .collect()
}

// ---- refused before anything is spent -------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_not_given_errands_is_not_offered_them_and_is_refused_one() {
    // Off is the default and the operator's to change. A model that calls the
    // tool anyway, having seen it in some other harness, spends nothing.
    let stub = serve(|body| {
        if speaker(body) == ERRAND {
            return Script::Say("I should not exist.".into());
        }
        if has_tool_result(body) {
            return Script::Say("I counted them myself.".into());
        }
        Script::Errands(vec!["Count the staff in the north office.".into()])
    })
    .await;
    let h = harness(&stub, &["Writer"], GuardLimits::default());

    let run = h.runtime.send_from_human(h.id("Writer"), "Count the staff.").unwrap();
    h.settle(run).await;

    assert!(
        !tools_by_agent(&stub)["Writer"].contains(&"errand".to_string()),
        "offered a tool the operator never switched on"
    );
    assert!(errand_requests(&stub).is_empty(), "an errand ran without the grant");
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("errands are not switched on for you"), "{results}");
    h.expect_normal(run, "an errand refused for want of the grant");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fourth_errand_is_refused_and_none_is_sent() {
    // Refused rather than queued, and all of them rather than the first three:
    // a model told "at most three" picks the three that matter, which a
    // runtime dropping the last one cannot do for it.
    let stub = serve(|body| {
        if speaker(body) == ERRAND {
            return Script::Say("I should not exist.".into());
        }
        if has_tool_result(body) {
            return Script::Say("Too many at once.".into());
        }
        Script::Errands((1..=4).map(|n| format!("Check office {n}.")).collect())
    })
    .await;
    let h = harness(&stub, &["Writer"], GuardLimits::default());
    grant(&h, "Writer");

    let run = h.runtime.send_from_human(h.id("Writer"), "Check every office.").unwrap();
    h.settle(run).await;

    assert!(errand_requests(&stub).is_empty(), "an errand ran from a refused call");
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("you asked for 4 errands and at most 3 run at once"), "{results}");
    assert!(matches!(
        recorded_calls(&h, "Writer").last(),
        Some((name, ToolOutcome::Refused { .. })) if name == "errand"
    ));
    h.expect_normal(run, "four errands refused");
}

// ---- what an errand is --------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn errands_work_their_briefs_and_their_answers_reach_the_turn_that_sent_them() {
    let stub = serve(|body| {
        if speaker(body) == ERRAND {
            if anyone_said(body, "north office") {
                return Script::Say("North office: 12 staff.".into());
            }
            return Script::Say("South office: 9 staff.".into());
        }
        if has_tool_result(body) {
            return Script::Say("21 staff across both offices.".into());
        }
        Script::Errands(vec![
            "Count the staff in the north office.".into(),
            "Count the staff in the south office.".into(),
        ])
    })
    .await;
    let h = harness(&stub, &["Writer", "Chef"], GuardLimits::default());
    grant(&h, "Writer");

    let run = h.runtime.send_from_human(h.id("Writer"), "Count the staff.").unwrap();
    h.settle(run).await;

    // The answers came back as the result of the call, into the turn that
    // asked, and that turn answered the operator from them.
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("North office: 12 staff."), "{results}");
    assert!(results.contains("South office: 9 staff."), "{results}");
    assert!(results.contains("the errand's own account"), "framed as its findings: {results}");
    assert!(
        h.channel_texts("Writer").iter().any(|text| text == "21 staff across both offices."),
        "{}",
        h.transcript()
    );

    // Two errands, one call each, and the turn's own two: four model calls,
    // each claimed from this run's budget and each on its bill.
    assert_eq!(stub.calls.load(Ordering::SeqCst), 4);
    h.expect_normal(run, "two errands and an answer");

    // An errand knows its brief and nothing else: not the agent's standing
    // instructions, not the crew, not the conversation it was sent from.
    let sent = errand_requests(&stub);
    assert_eq!(sent.len(), 2);
    for body in &sent {
        let system = body["messages"][0]["content"].as_str().unwrap();
        assert!(!system.contains("You are the Writer."), "the card's prompt reached it: {system}");
        assert!(!system.contains("Chef"), "the crew reached it: {system}");
        assert!(!anyone_said(body, "Count the staff."), "the operator's words reached it");

        let tools: Vec<&str> = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["function"]["name"].as_str())
            .collect();
        for withheld in ["errand", "send_message", "update_memory"] {
            assert!(!tools.contains(&withheld), "an errand was offered {withheld}: {tools:?}");
        }
        assert!(tools.contains(&"directory"), "the rest are the sender's own: {tools:?}");
    }

    // The crew heard nothing: an errand is not a message to anybody.
    assert!(h.channel_texts("Chef").is_empty(), "{}", h.transcript());

    // And the operator can read what each one came back with.
    let (_, outcome) = recorded_calls(&h, "Writer")
        .into_iter()
        .find(|(name, _)| name == "errand")
        .expect("the errand call is on the record");
    let ToolOutcome::Ok { summary } = outcome else { panic!("{outcome:?}") };
    assert!(summary.starts_with("2 errands: 2 finished"), "{summary}");
    assert!(summary.contains("North office: 12 staff."), "{summary}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_errand_cannot_message_the_crew_rewrite_memory_or_send_errands_of_its_own() {
    // Each one refused where the operator can see it, and each errand still
    // answers: a refusal is something to route around, not an ending.
    let stub = serve(|body| {
        if speaker(body) == ERRAND {
            if has_tool_result(body) {
                return Script::Say("Could not do that part; the rest is done.".into());
            }
            if anyone_said(body, "Tell Chef") {
                return Script::SendTo {
                    recipients: vec!["Chef".into()],
                    text: "Hello from an errand.".into(),
                };
            }
            if anyone_said(body, "Remember") {
                return Script::Memory("Overwritten by an errand.".into());
            }
            return Script::Errands(vec!["Go deeper.".into()]);
        }
        if has_tool_result(body) {
            return Script::Say("Done.".into());
        }
        Script::Errands(vec![
            "Tell Chef the menu is final.".into(),
            "Remember that the menu is final.".into(),
            "Split this into smaller errands.".into(),
        ])
    })
    .await;
    let h = harness(&stub, &["Writer", "Chef"], GuardLimits::default());
    grant(&h, "Writer");
    h.runtime.workspace().write(h.id("Writer"), "Writer", "What Writer knows.").unwrap();

    let run = h.runtime.send_from_human(h.id("Writer"), "Finalize the menu.").unwrap();
    h.settle(run).await;

    assert!(h.channel_texts("Chef").is_empty(), "an errand reached a peer: {}", h.transcript());
    assert_eq!(h.runtime.workspace().read(h.id("Writer")), "What Writer knows.");
    assert_eq!(
        errand_requests(&stub).len(),
        6,
        "three errands, two calls each, and no fourth sent from inside one"
    );

    let results = tool_results(&stub).join("\n");
    assert!(results.contains("an errand cannot message anyone"), "{results}");
    assert!(results.contains("an errand cannot rewrite Writer's memory"), "{results}");
    assert!(results.contains("an errand cannot send errands of its own"), "{results}");

    // On the agent's own trail, refused, with the reason.
    let refused: Vec<String> = recorded_calls(&h, "Writer")
        .into_iter()
        .filter(|(_, outcome)| matches!(outcome, ToolOutcome::Refused { .. }))
        .map(|(name, _)| name)
        .collect();
    for tool in ["send_message", "update_memory", "errand"] {
        assert!(refused.contains(&tool.to_string()), "{tool} is not on the trail: {refused:?}");
    }
    h.expect_normal(run, "three refusals inside errands");
}

// ---- what an errand spends -------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn errands_never_spend_the_calls_kept_for_the_turn_that_sent_them() {
    // An errand that never finishes, in a run with five calls to spend. The
    // turn spends one sending it; the errand may take the budget down to the
    // two kept back and no further; the turn answers with one of those.
    let stub = serve(|body| {
        if speaker(body) == ERRAND {
            return Script::Directory;
        }
        if has_tool_result(body) {
            return Script::Say("Here is what came back.".into());
        }
        Script::Errands(vec!["Keep looking.".into()])
    })
    .await;
    let h = harness(&stub, &["Writer"], GuardLimits { max_steps_per_run: 5, ..Default::default() });
    grant(&h, "Writer");

    let run = h.runtime.send_from_human(h.id("Writer"), "Look into it.").unwrap();
    h.settle(run).await;

    assert_eq!(errand_requests(&stub).len(), 2, "the errand stopped at the kept steps");
    assert!(
        h.channel_texts("Writer").iter().any(|text| text == "Here is what came back."),
        "the turn had no call left to answer with: {}",
        h.transcript()
    );
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("the last 2 model calls are kept for you"), "{results}");
    h.expect_normal(run, "errands held off the kept steps");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_errand_that_runs_out_of_rounds_says_so_rather_than_finishing() {
    let stub = serve(|body| {
        if speaker(body) == ERRAND {
            return Script::Narrate {
                text: "Still looking.".into(),
                then: Box::new(Script::Directory),
            };
        }
        if has_tool_result(body) {
            return Script::Say("It ran out.".into());
        }
        Script::Errands(vec!["Keep looking.".into()])
    })
    .await;
    let h = harness(&stub, &["Writer"], GuardLimits { max_tool_rounds: 3, ..Default::default() });
    grant(&h, "Writer");

    let run = h.runtime.send_from_human(h.id("Writer"), "Look into it.").unwrap();
    h.settle(run).await;

    assert_eq!(errand_requests(&stub).len(), 3, "bounded by the crew's own round limit");
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("stopped at its limit of 3 rounds before it finished"), "{results}");
    assert!(results.contains("What it had written by then:\nStill looking."), "{results}");
    h.expect_normal(run, "an errand out of rounds");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_errand_failing_leaves_the_others_answers_and_is_on_the_bill() {
    let stub = serve(|body| {
        if speaker(body) == ERRAND {
            if anyone_said(body, "broken") {
                return Script::Unavailable;
            }
            return Script::Say("The working one answered.".into());
        }
        if has_tool_result(body) {
            return Script::Say("One of two came back.".into());
        }
        Script::Errands(vec!["Ask the broken source.".into(), "Ask the good source.".into()])
    })
    .await;
    let h = harness(&stub, &["Writer"], GuardLimits::default());
    grant(&h, "Writer");

    let run = h.runtime.send_from_human(h.id("Writer"), "Ask both sources.").unwrap();
    h.settle(run).await;

    let results = tool_results(&stub).join("\n");
    assert!(results.contains("The working one answered."), "{results}");
    assert!(results.contains("failed:"), "{results}");
    assert!(
        h.envelopes(&["Writer"]).iter().flat_map(|e| e.parts.clone()).any(|part| matches!(
            part,
            Part::Notice { kind: NoticeKind::UpstreamError, ref text }
                if text.contains("An errand Writer sent could not reach the model")
        )),
        "the operator was not told: {}",
        h.transcript()
    );
    // The failed call is the one thing wrong with this run. It claimed its step
    // and is counted by its notice, so the bill still equals the calls made and
    // nothing reads as a budget that miscounted.
    assert_eq!(h.trajectory(run).anomalies(), vec![Anomaly::CallFailed { agent: "Writer".into() }]);
}

// ---- what reaches an errand from outside -------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_the_conversation_calls_its_errands_off() {
    let stub = serve(|body| {
        if speaker(body) == ERRAND {
            return Script::Hang;
        }
        if has_tool_result(body) {
            return Script::Say("I should not be asked again.".into());
        }
        Script::Errands(vec!["Think for a very long time.".into()])
    })
    .await;
    let h = harness(&stub, &["Writer"], GuardLimits::default());
    grant(&h, "Writer");

    let run = h.runtime.send_from_human(h.id("Writer"), "Think.").unwrap();
    h.wait_until("the errand's call is in flight", |_| stub.calls.load(Ordering::SeqCst) == 2)
        .await;
    h.runtime.stop_run(run);

    // Well inside the stub's hang: a stop that waited for the call to come
    // back would sit here for the whole of it.
    h.settle_within(run, (HANG - Duration::from_secs(20)).as_secs()).await;
    assert_eq!(stub.calls.load(Ordering::SeqCst), 2, "nothing was called after the stop");
    // The abandoned call gave its step back, so the bill names no call that
    // was never answered.
    h.expect_normal(run, "a stop reaching an errand mid-call");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn errands_that_need_the_terminal_take_turns_with_it() {
    // Two errands each writing a start and an end line into one file, with a
    // pause between. Run at once in one terminal they interleave; an errand
    // that touches a place holds it until it is done, so they cannot.
    let stub = serve(|body| {
        if speaker(body) == ERRAND {
            if has_tool_result(body) {
                return Script::Say("Marked.".into());
            }
            let mark = if anyone_said(body, "Mark A") { "A" } else { "B" };
            return Script::Shell(format!(
                "echo {mark}-start >> order.log && sleep 0.4 && echo {mark}-end >> order.log"
            ));
        }
        if has_tool_result(body) {
            return Script::Say("Both marked.".into());
        }
        Script::Errands(vec!["Mark A in the log.".into(), "Mark B in the log.".into()])
    })
    .await;
    let h = harness(&stub, &["Writer"], GuardLimits::default());
    grant(&h, "Writer");
    let root = terminal(&h, "Writer");

    let run = h.runtime.send_from_human(h.id("Writer"), "Mark both.").unwrap();
    h.settle(run).await;

    let log = std::fs::read_to_string(root.join("order.log")).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 4, "{log}");
    let first = lines[0].split('-').next().unwrap();
    assert_eq!(
        lines,
        if first == "A" {
            ["A-start", "A-end", "B-start", "B-end"]
        } else {
            ["B-start", "B-end", "A-start", "A-end"]
        },
        "two errands worked one terminal at once"
    );
    h.expect_normal(run, "errands taking turns with a terminal");
}

/// Gives the named agent a terminal, and returns where it is.
fn terminal(h: &Harness, name: &str) -> std::path::PathBuf {
    let card = h.agent_named(name).unwrap();
    h.runtime.store().set_has_terminal(card.id, true).unwrap();
    h.runtime.terminals().ensure(card.id).unwrap()
}
