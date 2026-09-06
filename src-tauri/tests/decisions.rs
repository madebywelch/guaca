//! Durable questions through the real actor and wire, with a scripted model.
mod harness;
use guac_lib::domain::decision::{DecisionRequest, DecisionStatus};
use guac_lib::domain::ids::DecisionId;
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::runtime::Runtime;
use harness::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

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
