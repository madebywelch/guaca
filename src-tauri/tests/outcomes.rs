mod harness;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use guac_lib::db::Store;
use guac_lib::domain::envelope::{Envelope, NoticeKind, Part, Participant};
use guac_lib::domain::ids::RunId;
use guac_lib::domain::now_ms;
use guac_lib::domain::outcome::{Need, Outcome, Status, Trigger as Started};
use guac_lib::domain::routine::{Cadence, Trigger};
use guac_lib::domain::view::{OperatorView, Overlay};
use guac_lib::runtime::events::UiEvent;
use guac_lib::runtime::guard::GuardLimits;
use harness::*;

fn outcomes(h: &Harness) -> Vec<Outcome> {
    h.runtime.store().outcomes(None, None, false, 100).unwrap()
}

fn only(h: &Harness) -> Outcome {
    let all = outcomes(h);
    assert_eq!(all.len(), 1, "one run, one outcome: {all:#?}\n{}", h.transcript());
    all.into_iter().next().unwrap()
}

fn newest(h: &Harness, name: &str, pick: impl Fn(&Envelope) -> bool) -> Envelope {
    h.runtime
        .store()
        .channel_messages(h.id(name), 100)
        .unwrap()
        .into_iter()
        .rev()
        .find(|m| pick(m))
        .unwrap_or_else(|| panic!("nothing in {name}'s channel matched:\n{}", h.transcript()))
}

fn notice(kind: NoticeKind) -> impl Fn(&Envelope) -> bool {
    move |m| m.parts.iter().any(|p| matches!(p, Part::Notice { kind: k, .. } if *k == kind))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_routine_run_ends_in_the_first_line_of_its_note() {
    let stub = serve(|_| {
        Script::Say(
            "Sent the digest to 12 people.\n\nThree bounced, and I sent those again.".into(),
        )
    })
    .await;
    let h = harness(&stub, &["Digest"], GuardLimits::default());
    let routine = h
        .runtime
        .store()
        .create_routine(
            h.id("Digest"),
            "Morning digest",
            "Send the morning digest.",
            Trigger::Clock(Cadence::Daily),
            Some(now_ms() + 24 * 60 * 60 * 1000),
            false,
        )
        .unwrap();

    let run = h.runtime.test_routine(&routine).unwrap();
    h.settle(run).await;

    let outcome = only(&h);
    let note = newest(&h, "Digest", |m| m.to == Participant::Human);
    assert_eq!(outcome.run_id, run);
    assert_eq!(outcome.agent_id, h.id("Digest"));
    assert_eq!(outcome.trigger, Started::Routine);
    assert_eq!(outcome.trigger_ref, Some(routine.id.to_string()));
    assert_eq!(outcome.status, Status::Done);
    assert_eq!(outcome.headline, "Sent the digest to 12 people.");
    assert_eq!(outcome.note_id, Some(note.id), "a click opens the note it came from");
    assert_eq!(outcome.needs, vec![]);
    assert_eq!(outcome.steps, 1);
    assert!(outcome.started_at <= outcome.finished_at);
    assert_eq!(outcome.seen_at, None);

    let events = h.sink.snapshot();
    let settled = events
        .iter()
        .position(|e| matches!(e, UiEvent::RunSettled { run_id, .. } if *run_id == run))
        .unwrap();
    let recorded: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, UiEvent::OutcomeRecorded { .. }))
        .map(|(at, _)| at)
        .collect();
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0] > settled);
    assert!(
        matches!(&events[recorded[0]], UiEvent::OutcomeRecorded { outcome: o } if **o == outcome)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_delegation_is_one_outcome_and_it_is_the_coordinators() {
    let stub = serve(|body| {
        let who = speaker(body);
        if who != "Planner" {
            return Script::Say(format!("{who} is booked for Friday."));
        }
        if reading_peer_replies(body) {
            Script::Say(
                "Venue and Caterer are both booked for Friday.\n\nDetails are in the thread."
                    .into(),
            )
        } else if has_tool_result(body) {
            Script::Say("Asked Venue and Caterer to book Friday.".into())
        } else {
            Script::SendTo {
                recipients: vec!["Venue".into(), "Caterer".into()],
                text: "Please book Friday.".into(),
            }
        }
    })
    .await;
    let h = harness(&stub, &["Planner", "Venue", "Caterer"], GuardLimits::default());

    let run = h.runtime.send_from_human(h.id("Planner"), "Book Friday's offsite.").unwrap();
    h.settle(run).await;

    let outcome = only(&h);
    assert_eq!(outcome.run_id, run);
    assert_eq!(outcome.agent_id, h.id("Planner"), "the agent the operator wrote to owns it");
    assert_eq!(outcome.trigger, Started::Operator);
    assert_eq!(outcome.trigger_ref, None);
    assert_eq!(outcome.status, Status::Done);
    assert_eq!(outcome.headline, "Venue and Caterer are both booked for Friday.");
    assert!(outcome.steps >= 4, "the peers' calls are on the run's bill: {}", outcome.steps);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_whose_model_call_failed_ends_failed_with_the_reason() {
    let stub = serve(|_| Script::Unavailable).await;
    let h = harness(&stub, &["Analyst"], GuardLimits::default());

    let run = h.runtime.send_from_human(h.id("Analyst"), "Summarize the quarter.").unwrap();
    h.settle_within(run, 60).await;

    let outcome = only(&h);
    let failure = newest(&h, "Analyst", notice(NoticeKind::UpstreamError));
    assert_eq!(outcome.status, Status::Failed);
    assert!(
        outcome.headline.starts_with("Analyst could not reply"),
        "the failure's own first sentence: {:?}",
        outcome.headline
    );
    assert!(!outcome.headline.contains('\n'));
    assert_eq!(outcome.note_id, Some(failure.id), "a click opens the notice, where Try again is");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_the_operator_stopped_ends_stopped() {
    let stub = serve(|_| Script::Hang).await;
    let h = harness(&stub, &["Writer"], GuardLimits::default());

    let run = h.runtime.send_from_human(h.id("Writer"), "Draft the launch post.").unwrap();
    h.wait_until("the call to be in the air", |_| stub.calls.load(Ordering::SeqCst) == 1).await;
    assert!(h.runtime.stop_run(run));
    h.settle(run).await;

    let outcome = only(&h);
    assert_eq!(outcome.run_id, run);
    assert_eq!(outcome.status, Status::Stopped);
    assert_eq!(outcome.headline, "Stopped before it finished.");
    assert_eq!(outcome.note_id, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_the_process_died_in_ends_interrupted_after_recovery() {
    let stub = serve(|_| Script::Hang).await;
    let h = harness(&stub, &["Writer"], GuardLimits::default());
    let writer = h.id("Writer");

    let run = h.runtime.send_from_human(writer, "Draft the launch post.").unwrap();
    h.wait_until("the call to be in the air", |_| stub.calls.load(Ordering::SeqCst) == 1).await;
    assert!(outcomes(&h).is_empty(), "nothing has finished yet");

    h.runtime.stop_agent(writer);
    let store = Store::open(&h._dir.path().join("guac.db")).unwrap();
    let recovered = guac_lib::boot::recover(&store).unwrap();
    assert_eq!(recovered.interrupted, 1);

    let all = store.outcomes(None, None, true, 100).unwrap();
    assert_eq!(all.len(), 1, "{all:#?}");
    let notice = store
        .channel_messages(writer, 50)
        .unwrap()
        .into_iter()
        .find(|m| notice(NoticeKind::Interrupted)(m))
        .unwrap();
    assert_eq!(all[0].run_id, run);
    assert_eq!(all[0].agent_id, writer);
    assert_eq!(all[0].status, Status::Interrupted);
    assert_eq!(all[0].headline, "The backend restarted before this conversation finished.");
    assert_eq!(all[0].note_id, Some(notice.id));

    guac_lib::boot::recover(&store).unwrap();
    assert_eq!(store.outcomes(None, None, false, 100).unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_taken_into_another_turn_has_no_outcome_of_its_own() {
    let gate = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let (open, count) = (gate.clone(), calls.clone());
    let stub = serve(move |_| match count.fetch_add(1, Ordering::SeqCst) {
        0 => {
            while !open.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            Script::Progress("Looking up the vendor.".into())
        }
        _ => Script::Say("The vendor opens at 9, and their number is 555-0100.".into()),
    })
    .await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let researcher = h.id("Researcher");

    let first = h.runtime.send_from_human(researcher, "Find the vendor's hours.").unwrap();
    h.wait_until("the first model call", |_| calls.load(Ordering::SeqCst) == 1).await;
    let second = h.runtime.send_from_human(researcher, "And their phone number.").unwrap();
    h.wait_until("the second to queue", |h| h.runtime.inbox_depth(researcher) == 1).await;
    gate.store(true, Ordering::SeqCst);
    h.settle(first).await;
    h.settle(second).await;
    assert!(
        stub.transcript.lock().iter().any(|body| anyone_said(body, "And their phone number.")),
        "the second message was read by the first turn"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2, "and answered there, with no turn of its own");

    let outcome = only(&h);
    assert_eq!(outcome.run_id, first);
    assert_eq!(outcome.headline, "The vendor opens at 9, and their number is 555-0100.");
    assert!(h.runtime.store().outcome(second).unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_that_filed_a_decision_ends_needing_the_operator() {
    let stub = serve(|body| {
        if anyone_said(body, "The operator answered decision") {
            return Script::Say("Booked the 11 AM slot with Alex.".into());
        }
        if has_tool_result(body) {
            return Script::Say("Drafted the reply to Alex.\n\nThe time is your call.".into());
        }
        Script::Plugin {
            name: "decision".into(),
            arguments: serde_json::json!({
                "action": "request",
                "topic": "alex/time",
                "question": "10 AM or 11 AM?",
                "options": ["10 AM", "11 AM"],
                "source": "Alex email"
            }),
        }
    })
    .await;
    let h = harness(&stub, &["Assistant"], GuardLimits::default());

    let run = h.runtime.send_from_human(h.id("Assistant"), "Answer Alex.").unwrap();
    h.settle(run).await;

    let decision = h.runtime.store().decisions(None).unwrap().remove(0);
    let outcome = only(&h);
    assert_eq!(outcome.status, Status::NeedsYou);
    assert_eq!(outcome.needs, vec![Need::Decision { id: decision.id }]);
    assert_eq!(outcome.headline, "Drafted the reply to Alex.");

    let answered = h.runtime.answer_decision(decision.id, "11 AM", false, None).unwrap();
    let follow = answered.delivery_run.unwrap();
    h.settle(follow).await;
    let after = h.runtime.store().outcome(follow).unwrap().unwrap();
    assert_eq!(after.trigger, Started::Decision);
    assert_eq!(after.trigger_ref, Some(decision.id.to_string()));
    assert_eq!(after.status, Status::Done);
    assert_eq!(after.headline, "Booked the 11 AM slot with Alex.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_channel_on_screen_marks_what_its_runs_ended_in_seen() {
    let stub = serve(|body| Script::Say(format!("{} is done.", speaker(body)))).await;
    let h = harness(&stub, &["Ada", "Ben"], GuardLimits::default());
    let ada = h.runtime.send_from_human(h.id("Ada"), "Go.").unwrap();
    let ben = h.runtime.send_from_human(h.id("Ben"), "Go.").unwrap();
    h.settle(ada).await;
    h.settle(ben).await;
    assert_eq!(h.runtime.store().outcomes(None, None, true, 100).unwrap().len(), 2);

    h.runtime.report_view(OperatorView {
        agent_id: Some(h.id("Ben")),
        overlay: Some(Overlay::ForYou),
        ..Default::default()
    });
    assert_eq!(h.runtime.store().outcomes(None, None, true, 100).unwrap().len(), 2);

    h.runtime.report_view(OperatorView { agent_id: Some(h.id("Ada")), ..Default::default() });
    let store = h.runtime.store();
    assert!(store.outcome(ada).unwrap().unwrap().seen_at.is_some());
    assert_eq!(store.outcome(ben).unwrap().unwrap().seen_at, None);
    let unseen: Vec<RunId> =
        store.outcomes(None, None, true, 100).unwrap().iter().map(|o| o.run_id).collect();
    assert_eq!(unseen, vec![ben]);
    let seen = |h: &Harness| {
        h.sink
            .snapshot()
            .into_iter()
            .filter_map(|e| match e {
                UiEvent::OutcomesSeen { run_ids } => Some(run_ids),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(seen(&h), vec![vec![ada]], "every other window is told which");

    h.runtime.report_view(OperatorView { agent_id: Some(h.id("Ada")), ..Default::default() });
    assert_eq!(seen(&h).len(), 1);

    let drawn = store.outcome(ben).unwrap().unwrap().finished_at;
    assert_eq!(h.runtime.mark_outcomes_seen(None, Some(drawn - 1)).unwrap(), vec![]);
    assert_eq!(h.runtime.mark_outcomes_seen(None, Some(drawn)).unwrap(), vec![ben]);
    assert!(store.outcomes(None, None, true, 100).unwrap().is_empty());
    assert_eq!(seen(&h).last(), Some(&vec![ben]));
}
