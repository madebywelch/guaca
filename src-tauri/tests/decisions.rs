//! Durable questions through the real actor and wire, with a scripted model.
mod harness;
use guac_lib::db::Store;
use guac_lib::domain::decision::{DecisionRequest, DecisionStatus};
use guac_lib::domain::envelope::{Envelope, Part};
use guac_lib::domain::ids::DecisionId;
use guac_lib::domain::now_ms;
use guac_lib::llm::openrouter::LlmClient;
use guac_lib::runtime::events::RecordingSink;
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::runtime::{OnDisk, Runtime};
use harness::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

#[tokio::test]
async fn a_decision_releases_the_turn_and_a_late_answer_starts_follow_through() {
    let step = Arc::new(AtomicUsize::new(0));
    let id = Arc::new(OnceLock::<DecisionId>::new());
    let answer_id = id.clone();
    let stub=serve(move|body|match step.fetch_add(1,Ordering::SeqCst) {
        0=>Script::Plugin {name:"decision".into(),arguments:serde_json::json!({"action":"request","topic":"thread/time","question":"10 AM or 11 AM?","options":["10 AM","11 AM"],"source":"Alex email"})},
        1=>Script::Say("The decision is in For you. I finished the other email checks.".into()),
        2=>{
            assert!(anyone_said(body,"The operator answered decision"));
            assert!(anyone_said(body,"11 AM"));
            Script::Plugin { name:"decision".into(),arguments:serde_json::json!({"action":"complete","id":answer_id.get().unwrap(),"outcome":"Meeting confirmed for 11 AM."}) }
        },
        _=>Script::Say("Meeting confirmed for 11 AM.".into())
    }).await;
    let h = harness(&stub, &["Assistant"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Assistant"), "Check my email.").unwrap();
    h.settle(run).await;
    assert!(h.runtime.store().pending_approvals(50).unwrap().is_empty());
    let decisions = h.runtime.store().decisions(None).unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].status, DecisionStatus::Pending);
    id.set(decisions[0].id).unwrap();
    let answered = h.runtime.answer_decision(decisions[0].id, "11 AM", false, None).unwrap();
    h.settle(answered.delivery_run.unwrap()).await;
    let done = h.runtime.store().decisions(None).unwrap();
    assert_eq!(done[0].status, DecisionStatus::Completed);
    assert_eq!(done[0].answer.as_deref(), Some("11 AM"));
    assert!(done[0].outcome.as_deref().unwrap().contains("confirmed"));
}

#[tokio::test]
async fn an_answer_arriving_during_other_work_is_read_at_the_next_round() {
    let hook = Arc::new(OnceLock::<(Runtime, DecisionId)>::new());
    let target = hook.clone();
    let step = Arc::new(AtomicUsize::new(0));
    let stub=serve(move|body|match step.fetch_add(1,Ordering::SeqCst) {
        0=>{
            let (runtime,id)=target.get().unwrap();
            runtime.answer_decision(*id,"11 AM",false,None).unwrap();
            Script::Directory
        },
        1=>{
            assert!(anyone_said(body,"The operator answered decision"),"answer was lost behind a busy turn");
            Script::Plugin {name:"decision".into(),arguments:serde_json::json!({"action":"complete","id":target.get().unwrap().1,"outcome":"11 AM recorded in the plan."})}
        },
        _=>Script::Say("The plan includes 11 AM.".into())
    }).await;
    let h = harness(&stub, &["Assistant"], GuardLimits::default());
    let agent = h.runtime.store().get_agent(h.id("Assistant")).unwrap().unwrap();
    let decision = h
        .runtime
        .store()
        .request_decision(
            agent.id,
            agent.group_id,
            "thread/time",
            DecisionRequest {
                question: "10 AM or 11 AM?".into(),
                context: String::new(),
                recommendation: String::new(),
                source: String::new(),
                options: vec!["10 AM".into(), "11 AM".into()],
            },
            None,
            guac_lib::domain::now_ms(),
        )
        .unwrap();
    assert!(hook.set((h.runtime.clone(), decision.id)).is_ok());
    let run = h.runtime.send_from_human(agent.id, "Work on the rest of the plan.").unwrap();
    h.settle(run).await;
    assert_eq!(h.runtime.store().decisions(None).unwrap()[0].status, DecisionStatus::Completed);
}

// ---- across a restart ------------------------------------------------------

fn time_question() -> DecisionRequest {
    DecisionRequest {
        question: "10 AM or 11 AM?".into(),
        context: String::new(),
        recommendation: String::new(),
        source: String::new(),
        options: vec!["10 AM".into(), "11 AM".into()],
    }
}

fn file_time_question(h: &Harness) -> DecisionId {
    let agent = h.runtime.store().get_agent(h.id("Assistant")).unwrap().unwrap();
    h.runtime
        .store()
        .request_decision(agent.id, agent.group_id, "thread/time", time_question(), None, now_ms())
        .unwrap()
        .id
}

/// Ends this host the way a crash ends it: no drain, every actor stopped
/// wherever it was, and the same workspace opened by a new host through the
/// recovery and the pickup boot runs.
fn crash(h: Harness) -> Harness {
    let Harness { runtime: old, ids, _dir, .. } = h;
    for id in ids.values() {
        old.stop_agent(*id);
    }
    let store = Store::open(&_dir.path().join("guac.db")).unwrap();
    let recovered = guac_lib::boot::recover(&store).unwrap();
    let sink = RecordingSink::new();
    let runtime = Runtime::new(
        store,
        LlmClient::new().unwrap(),
        old.config(),
        OnDisk::under(_dir.path()),
        sink.clone(),
    );
    runtime.start_all().unwrap();
    runtime.pick_up(recovered);
    Harness { runtime, sink, ids, _dir }
}

fn deliveries(h: &Harness, id: DecisionId) -> Vec<Envelope> {
    h.runtime
        .store()
        .channel_messages(h.id("Assistant"), 100)
        .unwrap()
        .into_iter()
        .filter(|m| m.parts.iter().any(|p| matches!(p, Part::Decision { id: d, .. } if *d == id)))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn follow_through_that_finished_before_a_restart_can_still_be_completed() {
    // The answer was acted on and the turn ended, but the work it started is
    // not done: the agent is waiting on Alex. A restart in between cut nothing
    // off, so when Alex confirms, completing the decision is not refused.
    let id = Arc::new(OnceLock::<DecisionId>::new());
    let target = id.clone();
    let stub = serve(move |body| {
        if has_tool_result(body) {
            Script::Say("Closed.".into())
        } else if anyone_said(body, "Alex confirmed") {
            Script::Plugin {
                name: "decision".into(),
                arguments: serde_json::json!({"action": "complete", "id": target.get().unwrap(),
                    "outcome": "Meeting confirmed for 11 AM."}),
            }
        } else {
            Script::Say("Asked Alex to confirm 11 AM.".into())
        }
    })
    .await;
    let h = harness(&stub, &["Assistant"], GuardLimits::default());
    id.set(file_time_question(&h)).unwrap();
    let answered = h.runtime.answer_decision(*id.get().unwrap(), "11 AM", false, None).unwrap();
    h.settle(answered.delivery_run.unwrap()).await;

    let (h, drained) = h.restart(Duration::from_secs(5)).await;
    assert!(drained.is_complete(), "{drained:?}");
    let after = &h.runtime.store().decisions(None).unwrap()[0];
    assert!(!after.interrupted, "follow-through that had settled was reported cut off");

    let run = h.runtime.send_from_human(h.id("Assistant"), "Alex confirmed 11 AM.").unwrap();
    h.settle(run).await;
    let done = &h.runtime.store().decisions(None).unwrap()[0];
    assert_eq!(done.status, DecisionStatus::Completed, "{}", tool_results(&stub).join("\n"));
    assert_eq!(done.outcome.as_deref(), Some("Meeting confirmed for 11 AM."));
}

/// A stub that holds the follow-through of an answer open until the test
/// ends, and answers a delivery that says it was interrupted.
async fn hangs_on_the_answer() -> Stub {
    serve(|body| {
        if anyone_said(body, "This work was interrupted") {
            Script::Say("Checked what was done; booked for 11 AM.".into())
        } else if anyone_said(body, "The operator answered decision") {
            Script::Hang
        } else {
            Script::Say("Nothing to do.".into())
        }
    })
    .await
}

/// A decision answered, and the process gone while its follow-through was
/// inside a model call.
async fn cut_off_during_follow_through(stub: &Stub) -> (Harness, DecisionId, Envelope) {
    let h = harness(stub, &["Assistant"], GuardLimits::default());
    let id = file_time_question(&h);
    h.runtime.answer_decision(id, "11 AM", false, None).unwrap();
    h.wait_until("the follow-through to start", |_| stub.calls.load(Ordering::SeqCst) == 1).await;
    let delivered = deliveries(&h, id);
    assert_eq!(delivered.len(), 1);
    (crash(h), id, delivered[0].clone())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_crash_during_follow_through_still_marks_it_interrupted() {
    let stub = hangs_on_the_answer().await;
    let (h, id, _) = cut_off_during_follow_through(&stub).await;
    let after = &h.runtime.store().decisions(None).unwrap()[0];
    assert_eq!(after.id, id);
    assert!(after.interrupted, "follow-through a crash cut off must be offered to resume");
    assert!(
        h.runtime
            .store()
            .finish_decision(id, h.id("Assistant"), DecisionStatus::Completed, "done", now_ms())
            .is_err(),
        "nothing completes interrupted work before it is resumed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn try_again_on_an_interrupted_delivery_is_the_decisions_resume_and_happens_once() {
    let stub = hangs_on_the_answer().await;
    let (h, id, delivery) = cut_off_during_follow_through(&stub).await;

    // Try again on the restart notice, which is anchored on the delivery.
    let run = h.runtime.retry_turn(h.id("Assistant"), delivery.id).unwrap();
    let decision = h.runtime.store().decisions(None).unwrap().remove(0);
    assert_eq!(decision.delivery_run, Some(run), "the retry is the decision's delivery now");
    assert!(!decision.interrupted, "and it is no longer waiting to be resumed");

    // So For you's Resume has nothing left to resume, and a second press of
    // Try again on the same notice delivers nothing either.
    assert!(h.runtime.answer_decision(id, "", true, None).is_err());
    assert!(h.runtime.retry_turn(h.id("Assistant"), delivery.id).is_err());
    h.settle(run).await;

    let delivered = deliveries(&h, id);
    assert_eq!(delivered.len(), 2, "the answer was delivered again once");
    assert!(
        matches!(delivered[1].parts.as_slice(), [Part::Decision { resumed: true, answer, .. }] if answer == "11 AM"),
        "delivered as Resume delivers it: {:?}",
        delivered[1].parts
    );
    assert_eq!(
        h.channel_texts("Assistant").last().unwrap(),
        "Checked what was done; booked for 11 AM."
    );
}
