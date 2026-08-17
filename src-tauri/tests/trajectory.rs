//! Trajectory evals: whether the machinery under a run behaved.
//!
//! The cascade suite asks whether the runtime did what it was told. The eval
//! suite asks whether the resulting traffic is something an operator would want
//! to watch. Both read the messages, and there is a class of defect neither can
//! see: the messages are correct and the machine around them is not.
//!
//! A placeholder that opens and never closes leaves a message half-arrived on
//! screen forever. A run that reports itself finished while an agent is still
//! thinking stops the spinner and then keeps talking. A budget that counts
//! turns rather than model calls bills a bounded run several times over, and
//! every message it produced looks exactly the same as one that did not.
//!
//! These drive the real runtime against the scripted model and read the event
//! stream the UI is drawn from. `guac_lib::trajectory` is the analyser;
//! everything it reports is decidable from the record, so a failure here names
//! a defect rather than a suspicion.

mod harness;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use guac_lib::domain::agent::Lifecycle;
use guac_lib::domain::approval::{ApprovalState, Decision};
use guac_lib::runtime::events::Activity;
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::trajectory::{Anomaly, Record};

use harness::*;

/// How many placeholders the operator watched open.
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
    // The claim the budget rests on, end to end: one turn, two model calls,
    // two steps. A budget counting turns would report one here and let a
    // tool-looping agent spend the run's whole allowance against it.
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
    // The failure this rules out is the operator-visible one: the spinner
    // stops, and then the answer arrives.
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
    // The architectural claim, read from the interleaving rather than a clock.
    // "Four peers finished inside a second" is an inference about a machine
    // that was not busy; four turns open at one point in the ledger is the
    // thing itself.
    let peers = ["Chef", "Host", "Barista", "Sommelier"];
    let stub = serve(move |body| {
        let who = speaker(body);
        if who == "Manager" {
            if has_tool_result(body) {
                Script::Say("All four are on it.".into())
            } else {
                Script::SendTo {
                    recipients: peers.iter().map(|s| s.to_string()).collect(),
                    text: "service starts at six".into(),
                }
            }
        } else {
            // Long enough that a serial runtime could not have every peer
            // mid-call at once, short enough not to slow the suite down.
            std::thread::sleep(Duration::from_millis(200));
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
    // Text from a first attempt is text the operator has already read. The
    // second attempt starts from the beginning, so it needs a box of its own,
    // and the trajectory is where that is visible: the messages are identical
    // either way.
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
    // A run that reached nobody is not a normal run, and the analyser has to
    // say so rather than pass it as quiet. The step still went: the request
    // was made, and a budget that refunded it would let a broken endpoint be
    // retried without limit.
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
    // "Try again" is the operator acting, so it gets a run and a budget of its
    // own. Both runs are in one event stream, which is the only place the two
    // can be confused: reading the retry's spend as the failed run's would
    // bill an operator for a call that never landed.
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = attempts.clone();
    let stub = serve(move |_| {
        // Three attempts is what one call is allowed. The fourth request is
        // the retry the operator asked for.
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
    // A guard doing its job is normal operation. What would not be is a run
    // that reported a different number than it spent, or one that left an
    // agent mid-turn when the money ran out.
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
    // Two independent accounts of the same run: the events the UI counts as
    // they arrive, and the rows the usage screen reads back afterwards. A run
    // is only observable if they agree.
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

// ---- work that will never be done -----------------------------------------
//
// A run settles when nothing is outstanding, and the turn that reads an
// envelope is what releases it. Every path where an envelope is delivered and
// then never read has to release it too, or the run waits on a turn that
// cannot happen: no settle, no reconciliation of what it spent, and one entry
// stuck in the runtime's in-flight table for the life of the process.

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deleting_an_agent_that_was_holding_work_still_ends_its_run() {
    // Found by this suite. A paused agent parks holding the message it was
    // woken by; deleting it there dropped that message and the run's only
    // outstanding piece of work with it, and the run never ended.
    let stub = serve(|_| Script::Say("on it".into())).await;
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());

    h.pause("Chef");
    let held = h.runtime.send_from_human(h.id("Chef"), "start the prep").unwrap();
    h.wait_until("Chef parks holding the message", |h| {
        matches!(h.runtime.activity_snapshot().get(&h.id("Chef")), Some(Activity::Paused))
    })
    .await;
    // A second run's work, still queued behind the one being held. It goes the
    // same way and has to be released the same way.
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
    // The same accounting from the other side: the agent is gone by the time
    // the envelope reaches its inbox. The operator is owed an ending either
    // way, and a run that can never start should end immediately rather than
    // stay in flight forever.
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
