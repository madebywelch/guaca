mod harness;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use guac_lib::domain::agent::Lifecycle;
use guac_lib::domain::approval::{ApprovalState, Decision};
use guac_lib::domain::envelope::NoticeKind;
use guac_lib::runtime::events::UiEvent;
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::trajectory::{Anomaly, Record};

use harness::*;

fn placeholders(t: &guac_lib::trajectory::Trajectory) -> usize {
    t.records.iter().filter(|r| matches!(r, Record::StreamOpened { .. })).count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_turn_leaves_nothing_open_and_nothing_running() {
    let stub = serve(|_| Script::Say("The kitchen is ready.".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Is the kitchen ready?").unwrap();
    h.settle(run).await;

    let t = h.expect_normal(run, "answering one question");
    assert_eq!(t.calls(), 1, "one question, one model call:\n{}", t.ledger);
    assert_eq!(t.steps(), Some(1), "and the run reports what it spent:\n{}", t.ledger);
    assert_eq!(t.turns(h.id("Manager")), 1);
    assert_eq!(t.tokens(), (CALL_TOKENS.0 as u64, CALL_TOKENS.1 as u64));
    assert_eq!(placeholders(&t), 1, "one message, one placeholder:\n{}", t.ledger);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_that_works_through_a_tool_result_bills_every_call_it_made() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("There are three of us.".into())
        } else {
            Script::Directory
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef", "Baker"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Who else works here?").unwrap();
    h.settle(run).await;

    let t = h.expect_normal(run, "looking something up before answering");
    assert_eq!(t.turns(h.id("Manager")), 1, "it is one turn:\n{}", t.ledger);
    assert_eq!(t.calls(), 2, "and two model calls:\n{}", t.ledger);
    assert_eq!(t.steps(), Some(2), "the budget counts the calls, not the turn:\n{}", t.ledger);
    assert_eq!(t.tools(), vec!["directory"]);
    assert_eq!(placeholders(&t), 1, "both calls fill one placeholder:\n{}", t.ledger);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_delegation_settles_after_the_peer_has_finished_rather_than_before() {
    let stub = serve(|body| {
        let who = speaker(body);
        if who == "Chef" {
            Script::Say("Twelve covers.".into())
        } else if has_tool_result(body) {
            Script::Say("Chef says twelve covers.".into())
        } else {
            Script::SendTo { recipients: vec!["Chef".into()], text: "how many covers".into() }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Ask Chef how many covers.").unwrap();
    h.settle(run).await;

    let t = h.expect_normal(run, "delegating a question");
    assert!(t.turns(h.id("Chef")) >= 1, "Chef has to have taken a turn:\n{}", t.ledger);
    assert_eq!(
        t.steps().map(|s| s as usize),
        Some(t.calls()),
        "every call the crew made is on the run's bill:\n{}",
        t.ledger
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_fan_out_has_every_peer_working_at_the_same_moment() {
    let peers = ["Chef", "Host", "Barista", "Sommelier"];
    let together = Arc::new((std::sync::Mutex::new(0), std::sync::Condvar::new()));
    let manager_arrived = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stub = serve(move |body| {
        let wait_for_peers = || {
            let (count, ready) = &*together;
            let mut count = count.lock().unwrap();
            *count += 1;
            ready.notify_all();
            let (count, timeout) = ready
                .wait_timeout_while(count, Duration::from_secs(10), |count| *count < 5)
                .unwrap();
            assert!(!timeout.timed_out(), "all five turns must reach the model together");
            drop(count);
        };
        let who = speaker(body);
        if who == "Manager" {
            if has_tool_result(body) {
                if !manager_arrived.swap(true, Ordering::SeqCst) {
                    wait_for_peers();
                }
                Script::Say("All four are on it.".into())
            } else {
                Script::SendTo {
                    recipients: peers.iter().map(|s| s.to_string()).collect(),
                    text: "service starts at six".into(),
                }
            }
        } else {
            wait_for_peers();
            Script::Say(format!("{who} is ready."))
        }
    })
    .await;

    let h = harness(
        &stub,
        &["Manager", "Chef", "Host", "Barista", "Sommelier"],
        GuardLimits::default(),
    );
    let run =
        h.runtime.send_from_human(h.id("Manager"), "Tell the team service starts at six.").unwrap();
    h.settle(run).await;

    let t = h.expect_normal(run, "telling four peers one thing");
    assert_eq!(
        t.peak_concurrency(),
        peers.len() + 1,
        "every peer and the manager that woke them should have been mid-turn together:\n{}",
        t.ledger
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dropped_connection_starts_a_new_placeholder_rather_than_appending_to_the_broken_one() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = attempts.clone();
    let stub = serve(move |_| {
        if counter.fetch_add(1, Ordering::SeqCst) == 0 {
            Script::Unavailable
        } else {
            Script::Say("Service resumed.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Are we back?").unwrap();
    h.settle(run).await;

    let t = h.expect_normal(run, "a connection that dropped once");
    assert_eq!(
        placeholders(&t),
        2,
        "the retry has to open its own placeholder, not reuse the broken one:\n{}",
        t.ledger
    );
    assert_eq!(t.calls(), 1, "two attempts, one call:\n{}", t.ledger);
    assert_eq!(t.steps(), Some(1), "and one step, however many times the network dropped it");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_provider_that_never_answers_is_named_and_still_spent_its_step() {
    let stub = serve(|_| Script::Unavailable).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Anyone there?").unwrap();
    h.settle(run).await;

    let t = h.trajectory(run);
    assert_eq!(
        t.anomalies(),
        vec![Anomaly::CallFailed { agent: "Manager".into() }],
        "the failure is the only thing wrong with this run:\n{}",
        t.ledger
    );
    assert_eq!(t.calls(), 0, "nothing was counted, because nothing answered");
    assert_eq!(t.steps(), Some(1), "the attempt still cost the run a step:\n{}", t.ledger);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_retry_is_its_own_run_and_carries_nothing_from_the_one_that_failed() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = attempts.clone();
    let stub = serve(move |_| {
        if counter.fetch_add(1, Ordering::SeqCst) < 3 {
            Script::Unavailable
        } else {
            Script::Say("The plan is unchanged.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let failed = h.runtime.send_from_human(h.id("Manager"), "what is the plan?").unwrap();
    h.settle(failed).await;

    let asked = h
        .runtime
        .store()
        .channel_messages(h.id("Manager"), 50)
        .unwrap()
        .into_iter()
        .find(|m| m.plain_text() == "what is the plan?")
        .expect("the operator's question is in the channel");
    let again = h.runtime.retry_turn(h.id("Manager"), asked.id).unwrap();
    h.settle(again).await;

    let broken = h.trajectory(failed);
    assert_eq!(
        broken.anomalies(),
        vec![Anomaly::CallFailed { agent: "Manager".into() }],
        "the failed run is the one that failed, and only that:\n{}",
        broken.ledger
    );
    assert_eq!(broken.calls(), 0, "nothing answered, so nothing was counted:\n{}", broken.ledger);

    let t = h.expect_normal(again, "the operator pressing try again");
    assert_eq!(t.calls(), 1, "the retry's spend is its own:\n{}", t.ledger);
    assert_eq!(t.steps(), Some(1), "under a budget of its own:\n{}", t.ledger);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_parked_on_the_operator_shows_the_wait_and_the_release() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Scout is set up.".into())
        } else {
            Script::Hire {
                name: "Scout".into(),
                instructions: "You look things up.".into(),
                notes: String::new(),
            }
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Hire a scout.").unwrap();

    let request = h.awaited_request().await;
    h.runtime.decide_approval(request, Decision::Allow).unwrap();
    h.settle(run).await;

    let t = h.expect_normal(run, "an agent asking to add a colleague");
    assert!(
        t.records.iter().any(|r| matches!(r, Record::Parked { .. })),
        "the wait is part of what happened and belongs in the ledger:\n{}",
        t.ledger
    );
    assert!(
        t.records.iter().any(|r| matches!(r, Record::Answered { state: ApprovalState::Allow, .. })),
        "and so is the answer:\n{}",
        t.ledger
    );
    assert_eq!(
        t.turns(h.id("Manager")),
        1,
        "parking and resuming is one turn, not two:\n{}",
        t.ledger
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_stopped_by_its_budget_stops_cleanly_and_spends_exactly_its_allowance() {
    let stub = serve(|_| Script::SendTo {
        recipients: vec!["Chef".into()],
        text: "and another thing".into(),
    })
    .await;

    let budget = 6;
    let limits = GuardLimits { max_steps_per_run: budget, ..GuardLimits::default() };
    let h = harness(&stub, &["Manager", "Chef"], limits);
    let run = h.runtime.send_from_human(h.id("Manager"), "Talk to Chef.").unwrap();
    h.settle(run).await;

    let t = h.expect_normal(run, "a model that will not stop");
    assert_eq!(t.steps(), Some(budget), "the budget is a ceiling, not a suggestion:\n{}", t.ledger);
    assert_eq!(t.calls(), budget as usize, "and every step was a real call:\n{}", t.ledger);
    assert!(
        !t.refusals().is_empty(),
        "something has to have told the models to stop:\n{}",
        t.ledger
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_the_operator_watched_is_what_the_run_was_billed_for() {
    let stub = serve(|body| {
        let who = speaker(body);
        if who == "Chef" {
            Script::Say("Six.".into())
        } else if has_tool_result(body) {
            Script::Say("Chef says six.".into())
        } else {
            Script::SendTo { recipients: vec!["Chef".into()], text: "how many".into() }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Ask Chef how many.").unwrap();
    h.settle(run).await;

    let t = h.expect_normal(run, "a delegation, counted twice");
    let billed = h.runtime.store().usage_by_run(&[run]).unwrap();
    let billed = billed.get(&run).copied().expect("a run that made calls has rows");

    assert_eq!(
        billed.calls as usize,
        t.calls(),
        "the store and the screen disagree:\n{}",
        t.ledger
    );
    assert_eq!(
        (billed.prompt, billed.completion),
        t.tokens(),
        "the tokens the operator watched arrive are not the ones on the bill:\n{}",
        t.ledger
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deleting_an_agent_that_was_holding_work_still_ends_its_run() {
    let stub = serve(|_| Script::Say("on it".into())).await;
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());

    let held = h.park("Chef", "start the prep").await;
    let queued = h.runtime.send_from_human(h.id("Chef"), "and the stock").unwrap();

    h.runtime.store().set_lifecycle(h.id("Chef"), Lifecycle::Terminated).unwrap();
    h.runtime.stop_agent(h.id("Chef"));

    for (run, what) in [(held, "the message it was holding"), (queued, "the one queued behind it")]
    {
        assert!(
            h.settled_within(run, 5).await,
            "{what} outlived the agent:\n{}",
            h.trajectory(run).ledger
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_nobody_is_left_to_read_ends_its_run_rather_than_hanging() {
    let stub = serve(|_| Script::Say("nobody home".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());

    h.runtime.stop_agent(h.id("Manager"));
    let run = h.runtime.send_from_human(h.id("Manager"), "are you there?").unwrap();

    assert!(
        h.settled_within(run, 5).await,
        "a message with no reader left its run open:\n{}",
        h.trajectory(run).ledger
    );
    assert_eq!(h.trajectory(run).steps(), Some(0), "and nothing was spent on it");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_a_run_ends_it_and_bills_only_the_calls_it_made() {
    let stub = serve(|_| Script::SendTo {
        recipients: vec!["Chef".into()],
        text: "and another thing".into(),
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Talk to Chef.").unwrap();

    h.wait_until("a call has been made and paid for", |h| {
        h.sink.count_of(|e| matches!(e, UiEvent::TokensUsed { run_id, .. } if *run_id == run)) > 0
    })
    .await;

    assert!(h.runtime.stop_run(run), "a run with work outstanding can be stopped");
    assert!(
        h.settled_within(run, 10).await,
        "a stopped run still owes the operator an ending:\n{}",
        h.trajectory(run).ledger
    );

    let t = h.expect_normal(run, "a conversation the operator stopped");
    assert_eq!(
        t.steps().map(|steps| steps as usize),
        Some(t.calls()),
        "a stopped run must not report a call it never made:\n{}",
        t.ledger
    );
    assert!(
        t.records.iter().any(|r| matches!(
            r,
            Record::Noticed { kind: NoticeKind::GuardStop, text, .. } if text.contains("stopped")
        )),
        "the transcript has to say why it ended, in the agent's own channel:\n{}",
        t.ledger
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_a_run_abandons_the_model_call_it_was_waiting_on() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Hang
        } else {
            Script::Progress("looking into it".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "How is the prep going?").unwrap();

    h.wait_until("the second call has been sent and not answered", |_| {
        stub.calls.load(Ordering::SeqCst) >= 2
    })
    .await;

    assert!(h.runtime.stop_run(run), "the turn is still outstanding");
    assert!(
        h.settled_within(run, 5).await,
        "a stop waited out the call in flight, which {HANG:?} of provider is too long to call \
         stopping:\n{}",
        h.trajectory(run).ledger
    );

    let t = h.expect_normal(run, "a stop that landed while a call was in flight");
    assert_eq!(t.calls(), 1, "the abandoned call answered nothing, so nothing was counted");
    assert_eq!(
        t.steps().map(|steps| steps as usize),
        Some(t.calls()),
        "the step claimed for the abandoned call was not given back:\n{}",
        t.ledger
    );
    assert!(
        t.records.iter().any(|r| matches!(
            r,
            Record::Noticed { kind: NoticeKind::GuardStop, text, .. } if text.contains("stopped")
        )),
        "and the channel says why it ended where it did:\n{}",
        t.ledger
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_a_run_a_paused_agent_is_holding_still_ends_it() {
    let stub = serve(|_| Script::Say("on it".into())).await;
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());

    let run = h.park("Chef", "start the prep").await;

    assert!(h.runtime.stop_run(run), "the run is outstanding, so there is something to stop");
    assert!(
        h.settled_within(run, 5).await,
        "work a paused agent was holding outlived the stop:\n{}",
        h.trajectory(run).ledger
    );

    let t = h.expect_normal(run, "stopping work a paused agent was holding");
    assert_eq!(t.calls(), 0, "nothing ran, so nothing was billed:\n{}", t.ledger);
    assert!(
        t.records.iter().any(|r| matches!(
            r,
            Record::Noticed { kind: NoticeKind::GuardStop, text, .. } if text.contains("paused")
        )),
        "and the channel says why nothing happened:\n{}",
        t.ledger
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_a_run_closes_the_request_it_was_waiting_on() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Scout is set up.".into())
        } else {
            Script::Hire {
                name: "Scout".into(),
                instructions: "You look things up.".into(),
                notes: String::new(),
            }
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Hire a scout.").unwrap();

    let request = h.awaited_request().await;
    assert!(h.runtime.stop_run(run), "a turn parked on the operator is still outstanding work");
    assert!(
        h.settled_within(run, 10).await,
        "a stopped run sat waiting on a request nobody was going to answer:\n{}",
        h.trajectory(run).ledger
    );

    let t = h.expect_normal(run, "stopping a turn that was waiting on the operator");
    assert!(
        t.records
            .iter()
            .any(|r| matches!(r, Record::Answered { state: ApprovalState::Expired, .. })),
        "the request has to be closed, and as expired rather than refused:\n{}",
        t.ledger
    );
    assert_eq!(
        h.runtime.store().get_approval(request).unwrap().map(|a| a.state),
        Some(ApprovalState::Expired),
        "the row is the record, and a row left pending draws live buttons that answer nothing"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_everything_ends_every_run_exactly_once() {
    let stub = serve(|body| {
        let peer = match speaker(body).as_str() {
            "Manager" => "Chef",
            "Chef" => "Manager",
            "Host" => "Barista",
            _ => "Host",
        };
        Script::SendTo { recipients: vec![peer.into()], text: "and another thing".into() }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef", "Host", "Barista"], GuardLimits::default());
    assert_eq!(h.runtime.live_runs(), 0);
    assert_eq!(h.runtime.stop_everything(), 0, "a stop with nothing running stops nothing");

    let first = h.runtime.send_from_human(h.id("Manager"), "Talk to Chef.").unwrap();
    let second = h.runtime.send_from_human(h.id("Host"), "Talk to Barista.").unwrap();

    h.wait_until("both conversations have paid for a call", |h| {
        [first, second].iter().all(|run| {
            h.sink.count_of(|e| matches!(e, UiEvent::TokensUsed { run_id, .. } if run_id == run))
                > 0
        })
    })
    .await;
    assert_eq!(h.runtime.live_runs(), 2, "the strip counts conversations, not agents");

    assert_eq!(h.runtime.stop_everything(), 2);

    for (run, which) in [(first, "the first"), (second, "the second")] {
        assert!(
            h.settled_within(run, 10).await,
            "{which} conversation owes an ending:\n{}",
            h.trajectory(run).ledger
        );
        let t = h.expect_normal(run, "every conversation stopped at once");
        assert_eq!(
            t.steps().map(|steps| steps as usize),
            Some(t.calls()),
            "{which} conversation must not report a call it never made:\n{}",
            t.ledger
        );
    }

    assert_eq!(h.runtime.live_runs(), 0, "and nothing is left outstanding");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_everything_ends_each_run_once_when_they_share_both_inboxes() {
    const PROBE: &str = "Is anything left?";
    let said_it_was_stopped = |text: &str| text.starts_with("You stopped this conversation");
    let stub = serve(|body| {
        if anyone_said(body, PROBE) {
            return Script::Say("Nothing.".into());
        }
        Script::SendTo {
            recipients: vec![if speaker(body) == "Manager" { "Chef" } else { "Manager" }.into()],
            text: "and another thing".into(),
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let first = h.runtime.send_from_human(h.id("Manager"), "Talk to Chef.").unwrap();
    let second = h.runtime.send_from_human(h.id("Chef"), "Talk to Manager.").unwrap();

    h.wait_until("both conversations have paid for a call", |h| {
        [first, second].iter().all(|run| {
            h.sink.count_of(|e| matches!(e, UiEvent::TokensUsed { run_id, .. } if run_id == run))
                > 0
        })
    })
    .await;
    assert_eq!(h.runtime.live_runs(), 2);

    assert_eq!(h.runtime.stop_everything(), 2);
    for (run, which) in [(first, "the first"), (second, "the second")] {
        assert!(
            h.settled_within(run, 10).await,
            "{which} conversation owes an ending:\n{}",
            h.trajectory(run).ledger
        );
    }

    for name in ["Manager", "Chef"] {
        let probe = h.runtime.send_from_human(h.id(name), PROBE).unwrap();
        h.settle(probe).await;
    }
    assert_eq!(h.runtime.live_runs(), 0, "and nothing is left outstanding");

    for (run, which) in [(first, "the first"), (second, "the second")] {
        let t = h.trajectory(run);
        let settles =
            h.sink.count_of(|e| matches!(e, UiEvent::RunSettled { run_id, .. } if *run_id == run));
        assert_eq!(
            settles, 1,
            "{which} conversation was reported finished {settles} times:\n{}",
            t.ledger
        );

        let wrong: Vec<String> = t
            .anomalies()
            .into_iter()
            .filter(|a| !matches!(a, Anomaly::StillWorkingWhenTheRunSettled { .. }))
            .map(|a| format!("  - {}", a.explain()))
            .collect();
        assert!(wrong.is_empty(), "{which} conversation:\n{}\n{}", t.ledger, wrong.join("\n"));
        assert_eq!(
            t.steps().map(|steps| steps as usize),
            Some(t.calls()),
            "{which} conversation must not report a call it never made:\n{}",
            t.ledger
        );
        assert!(
            t.records.iter().any(|r| matches!(
                r,
                Record::Noticed { kind: NoticeKind::GuardStop, text, .. } if said_it_was_stopped(text)
            )),
            "{which} conversation has to say it was stopped:\n{}",
            t.ledger
        );

        let mut told = std::collections::HashSet::new();
        for record in &t.records {
            match record {
                Record::Noticed { agent, kind: NoticeKind::GuardStop, text }
                    if said_it_was_stopped(text) =>
                {
                    told.insert(*agent);
                }
                Record::Called { agent, .. } | Record::StreamOpened { agent, .. } => assert!(
                    !told.contains(agent),
                    "{which} conversation took another step after its own stop:\n{}",
                    t.ledger
                ),
                _ => {}
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_a_run_that_has_already_finished_changes_nothing() {
    let stub = serve(|_| Script::Say("The kitchen is ready.".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "are we set?").unwrap();
    h.settle(run).await;

    assert!(!h.runtime.stop_run(run), "there is nothing left to stop");
    assert!(h.settled_within(run, 1).await, "and the run still ended exactly once");

    let t = h.expect_normal(run, "a stop that arrived after the answer");
    assert!(
        !t.records.iter().any(|r| matches!(r, Record::Noticed { kind: NoticeKind::GuardStop, .. })),
        "a run that finished on its own must not gain a line saying it was stopped:\n{}",
        t.ledger
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_a_run_mid_call_does_not_let_the_answer_reach_the_peer() {
    let chef_calls = Arc::new(AtomicUsize::new(0));
    let counted = chef_calls.clone();
    let stub = serve(move |body| {
        if speaker(body) == "Chef" {
            if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                return Script::Unavailable;
            }
            return Script::Say("the prep is done".into());
        }
        if has_tool_result(body) {
            Script::Say("asked Chef.".into())
        } else {
            Script::SendTo { recipients: vec!["Chef".into()], text: "how is the prep".into() }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Ask Chef about the prep.").unwrap();

    h.wait_until("Chef's first attempt has been refused", |_| {
        chef_calls.load(Ordering::SeqCst) >= 1
    })
    .await;

    assert!(h.runtime.stop_run(run), "Chef's envelope is still outstanding");
    assert!(
        h.settled_within(run, 10).await,
        "a run stopped between a turn's attempts never ended:\n{}",
        h.trajectory(run).ledger
    );

    let t = h.expect_normal(run, "a stop that landed while an agent was talking");
    assert!(
        !h.channel_texts("Manager").iter().any(|line| line.contains("the prep is done")),
        "Chef's answer was sent on after the stop:\n{:?}",
        h.channel_texts("Manager")
    );
    assert!(
        t.records.iter().any(|r| matches!(
            r,
            Record::Noticed { kind: NoticeKind::GuardStop, text, .. } if text.contains("stopped")
        )),
        "and the transcript has to say why nothing was sent on:\n{}",
        t.ledger
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_a_run_queued_behind_another_at_a_paused_agent_still_ends_it() {
    let stub = serve(|_| Script::Say("on it".into())).await;
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());

    let held = h.park("Chef", "start the prep").await;

    let behind = h.runtime.send_from_human(h.id("Chef"), "and the stock").unwrap();

    assert!(h.runtime.stop_run(behind), "the queued run is outstanding");
    assert!(
        h.settled_within(behind, 5).await,
        "work queued behind a held envelope outlived the stop:\n{}",
        h.trajectory(behind).ledger
    );

    assert!(
        !h.settled_within(held, 1).await,
        "the run nobody stopped ended anyway:\n{}",
        h.trajectory(held).ledger
    );
    h.resume("Chef");
    assert!(
        h.settled_within(held, 10).await,
        "the held run never resumed:\n{}",
        h.trajectory(held).ledger
    );

    h.expect_normal(behind, "stopping work queued behind a held envelope");
    h.expect_normal(held, "the conversation that was not stopped");
}
