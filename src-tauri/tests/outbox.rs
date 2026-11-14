//! The outbox: what reaches a person, through the real runtime with a scripted
//! model. Each scenario drives the cause, steps the dispatcher with a clock it
//! is handed, and then plays the windows: what one asks for when it connects,
//! and what its claim answers.
mod harness;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Barrier};
use std::time::Duration;

use guac_lib::domain::decision::DecisionRequest;
use guac_lib::domain::now_ms;
use guac_lib::domain::outbox::{self, Delivery, Kind, Target, GATHER_MS};
use guac_lib::domain::outcome::Status;
use guac_lib::domain::routine::{Cadence, Trigger};
use guac_lib::domain::view::{OperatorView, AWAY_AFTER_MS};
use guac_lib::runtime::events::UiEvent;
use guac_lib::runtime::guard::GuardLimits;
use harness::*;

/// One outbox row, as much of it as these scenarios read.
#[derive(Debug, Clone)]
struct Row {
    id: String,
    kind: String,
    status: String,
    last_error: Option<String>,
    title: String,
}

fn rows(h: &Harness) -> Vec<Row> {
    let conn = h.runtime.store().conn().unwrap();
    let mut stmt = conn
        .prepare("SELECT id,kind,status,last_error,title FROM outbox ORDER BY created_at, id")
        .unwrap();
    stmt.query_map([], |row| {
        Ok(Row {
            id: row.get(0)?,
            kind: row.get(1)?,
            status: row.get(2)?,
            last_error: row.get(3)?,
            title: row.get(4)?,
        })
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

/// What a window that has just connected is offered.
fn pending(h: &Harness) -> Vec<Delivery> {
    h.runtime.pending_deliveries().unwrap()
}

fn offers(h: &Harness) -> usize {
    h.sink.count_of(|e| matches!(e, UiEvent::Delivery { .. }))
}

/// Files a decision due within the hour, which is due for a reminder at once.
fn file_due_decision(h: &Harness, name: &str, topic: &str) {
    let agent = h.runtime.store().get_agent(h.id(name)).unwrap().unwrap();
    h.runtime
        .store()
        .request_decision(
            agent.id,
            agent.group_id,
            topic,
            DecisionRequest {
                question: "10 AM or 11 AM?".into(),
                context: String::new(),
                recommendation: String::new(),
                source: String::new(),
                options: vec!["10 AM".into(), "11 AM".into()],
            },
            Some(now_ms() + 30 * 60 * 1000),
            now_ms(),
        )
        .unwrap();
}

/// A window with focus on this agent's channel.
fn looking_at(h: &Harness, name: &str, window: &str) {
    let view = OperatorView { agent_id: Some(h.id(name)), ..Default::default() };
    h.runtime.report_window(view, true, Some(window.into()));
}

fn escalating() -> impl Fn(&serde_json::Value) -> Script + Clone + Send + Sync + 'static {
    |body| {
        let said = body["messages"]
            .as_array()
            .map(|m| m.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        if said.contains("on the operator's desk now") || said.contains("You already had that up") {
            Script::Say("Flagged it. I did what I could without the deploy key.".into())
        } else {
            Script::Escalate("The deploy needs a key only you have.".into())
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reminder_due_with_no_window_open_is_delivered_once_when_one_connects() {
    let stub = serve(|_| Script::Say("Nothing to do.".into())).await;
    let h = harness(&stub, &["Assistant"], GuardLimits::default());
    file_due_decision(&h, "Assistant", "thread/time");

    // Due now, with no window connected: the claim used to be the only
    // record, and the event it raised went to nobody.
    h.runtime.sweep_decisions(now_ms());
    assert_eq!(
        h.sink.count_of(|e| matches!(e, UiEvent::DecisionReminder { count: 1 })),
        1,
        "still said, for the windows that raise their own banner from it"
    );
    assert_eq!(h.runtime.dispatch_deliveries(now_ms()), 1);

    // A window connects and asks what it missed.
    let waiting = pending(&h);
    assert_eq!(waiting.len(), 1, "{waiting:#?}");
    assert_eq!(waiting[0].kind, Kind::Reminder);
    assert_eq!(waiting[0].title, "For you");
    assert_eq!(waiting[0].body, "1 decision needs a review. Open For you to answer or follow up.");
    assert_eq!(waiting[0].target, Target::ForYou);

    assert!(h.runtime.claim_delivery(&waiting[0].id).unwrap(), "the first window shows it");
    assert!(!h.runtime.claim_delivery(&waiting[0].id).unwrap(), "and no window shows it again");
    assert!(pending(&h).is_empty());

    // The next sweep finds nothing due, and writes nothing.
    h.runtime.sweep_decisions(now_ms());
    h.runtime.dispatch_deliveries(now_ms());
    assert!(pending(&h).is_empty());
    let all = rows(&h);
    assert_eq!(all.len(), 1, "{all:#?}");
    assert_eq!(all[0].status, "delivered");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reminder_due_while_the_host_was_down_is_not_lost_at_boot() {
    let stub = serve(|_| Script::Say("Nothing to do.".into())).await;
    let h = harness(&stub, &["Assistant"], GuardLimits::default());
    file_due_decision(&h, "Assistant", "thread/time");

    // The next host boots the way boot::open does: pickup, dispatcher, then
    // the scheduler, whose first sweep runs before any window can connect.
    let (h, _) = h.restart(Duration::from_secs(5)).await;
    h.runtime.start_dispatcher();
    h.runtime.start_scheduler();

    h.wait_until("the reminder to be offered", |h| pending(h).len() == 1).await;
    let waiting = pending(&h);
    assert_eq!(waiting[0].kind, Kind::Reminder);
    assert_eq!(offers(&h), 1, "offered once, to nobody, and kept");
    assert!(h.runtime.claim_delivery(&waiting[0].id).unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_windows_claiming_one_delivery_show_it_once() {
    let stub = serve(|_| Script::Say("Nothing to do.".into())).await;
    let h = harness(&stub, &["Assistant"], GuardLimits::default());
    file_due_decision(&h, "Assistant", "thread/time");
    h.runtime.sweep_decisions(now_ms());
    h.runtime.dispatch_deliveries(now_ms());
    let id = pending(&h)[0].id.clone();

    // Every window heard the same offer and claims it at the same moment.
    let windows = 8;
    let start = Arc::new(Barrier::new(windows));
    let claims: Vec<_> = (0..windows)
        .map(|_| {
            let (runtime, start, id) = (h.runtime.clone(), start.clone(), id.clone());
            std::thread::spawn(move || {
                start.wait();
                runtime.claim_delivery(&id).unwrap()
            })
        })
        .collect();
    let won = claims.into_iter().map(|claim| claim.join().unwrap()).filter(|won| *won).count();
    assert_eq!(won, 1, "exactly one window shows the banner");
    assert!(pending(&h).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_that_ends_in_the_channel_the_operator_is_reading_interrupts_nobody() {
    let stub = serve(|_| Script::Say("Sent the digest to 12 people.".into())).await;
    let h = harness(&stub, &["Digest", "Triage"], GuardLimits::default());
    looking_at(&h, "Digest", "desk");

    let run = h.runtime.send_from_human(h.id("Digest"), "Send the digest.").unwrap();
    h.settle(run).await;
    h.runtime.dispatch_deliveries(now_ms());

    let all = rows(&h);
    assert_eq!(all.len(), 1, "{all:#?}");
    assert_eq!(all[0].kind, "outcome");
    assert_eq!(all[0].status, "suppressed");
    assert_eq!(all[0].last_error.as_deref(), Some("on screen"));

    // The control: the same ending in a channel the operator is not reading
    // is not decided against, only held until they would count as away.
    let run = h.runtime.send_from_human(h.id("Triage"), "Triage the inbox.").unwrap();
    h.settle(run).await;
    h.runtime.dispatch_deliveries(now_ms());
    let triage = rows(&h).into_iter().find(|row| row.title == "Triage").unwrap();
    assert_eq!(triage.status, "pending", "{triage:#?}");
    assert!(pending(&h).is_empty(), "nothing finished is offered while the operator is here");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_routines_finishing_while_the_operator_is_away_are_one_item() {
    let stub = serve(|body| Script::Say(format!("{} is done.", speaker(body)))).await;
    let names = ["R1", "R2", "R3"];
    let h = harness(&stub, &names, GuardLimits::default());
    for name in names {
        let routine = h
            .runtime
            .store()
            .create_routine(
                h.id(name),
                "Morning check",
                "Check the morning.",
                Trigger::Clock(Cadence::Daily),
                Some(now_ms() + 24 * 60 * 60 * 1000),
                false,
            )
            .unwrap();
        let run = h.runtime.test_routine(&routine).unwrap();
        h.settle(run).await;
    }

    // No window has said anything: nobody is there. The three are gathered.
    let now = now_ms();
    assert_eq!(h.runtime.dispatch_deliveries(now), 0);
    assert!(pending(&h).is_empty(), "nothing is offered while it gathers");
    assert_eq!(h.runtime.dispatch_deliveries(now + GATHER_MS - 1), 0);

    assert_eq!(h.runtime.dispatch_deliveries(now + GATHER_MS), 1, "one item for all three");
    let waiting = pending(&h);
    assert_eq!(waiting.len(), 1, "{waiting:#?}");
    assert_eq!(waiting[0].kind, Kind::Outcome);
    assert_eq!(waiting[0].title, "3 finished");
    let lines: Vec<&str> = waiting[0].body.lines().collect();
    assert_eq!(lines.len(), 3, "{}", waiting[0].body);
    for name in names {
        assert!(lines.iter().any(|line| line.starts_with(&format!("{name}: "))), "{lines:?}");
    }
    assert_eq!(waiting[0].target, Target::ForYou);

    assert!(h.runtime.claim_delivery(&waiting[0].id).unwrap());
    assert!(rows(&h).iter().all(|row| row.status == "delivered"), "{:#?}", rows(&h));
    assert_eq!(offers(&h), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_with_nothing_to_tell_writes_nothing() {
    // A stop is the operator's own act.
    let stub = serve(|_| Script::Hang).await;
    let h = harness(&stub, &["Writer"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Writer"), "Draft the launch post.").unwrap();
    h.wait_until("the call to be in the air", |_| stub.calls.load(Ordering::SeqCst) == 1).await;
    assert!(h.runtime.stop_run(run));
    h.settle(run).await;
    let outcome = h.runtime.store().outcome(run).unwrap().unwrap();
    assert_eq!(outcome.status, Status::Stopped);
    h.runtime.dispatch_deliveries(now_ms() + GATHER_MS);
    assert!(rows(&h).is_empty(), "{:#?}", rows(&h));

    // A check that found nothing is the other ending nobody is told about.
    // No run ends that way until the prompt lets an agent say so, so it is
    // asked of the one function every outcome's row comes from.
    let nothing_new = guac_lib::domain::outcome::Outcome { status: Status::NothingNew, ..outcome };
    assert_eq!(outbox::for_outcome(&nothing_new, "Writer"), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restart_between_the_offer_and_the_claim_delivers_once() {
    let stub = serve(escalating()).await;
    let h = harness(&stub, &["Analyst"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Analyst"), "Ship it.").unwrap();
    h.settle(run).await;

    assert_eq!(h.runtime.dispatch_deliveries(now_ms()), 1, "the escalation, at once");
    let before = pending(&h);
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].kind, Kind::NeedsYou);
    assert_eq!(before[0].title, "Analyst is stuck");

    // The host goes down before any window claims it.
    let (h, _) = h.restart(Duration::from_secs(5)).await;
    assert_eq!(h.runtime.dispatch_deliveries(now_ms()), 0, "an offer is not made twice");
    let after = pending(&h);
    assert_eq!(after.len(), 1, "{after:#?}");
    assert_eq!(after[0].id, before[0].id);
    assert!(h.runtime.claim_delivery(&after[0].id).unwrap());
    assert!(!h.runtime.claim_delivery(&after[0].id).unwrap());
    assert!(pending(&h).is_empty());
    let stuck: Vec<Row> = rows(&h).into_iter().filter(|row| row.kind == "needs_you").collect();
    assert_eq!(stuck.len(), 1, "{stuck:#?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_banner_about_something_already_dealt_with_is_withdrawn() {
    let stub = serve(escalating()).await;
    let h = harness(&stub, &["Analyst"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Analyst"), "Ship it.").unwrap();
    h.settle(run).await;

    // Raised twice in one day is one interruption.
    let again = h.runtime.send_from_human(h.id("Analyst"), "Try again.").unwrap();
    h.settle(again).await;
    let open = h.runtime.store().open_escalations(10).unwrap();
    assert_eq!(open[0].times, 2, "raised again: {}", h.transcript());
    let stuck: Vec<Row> = rows(&h).into_iter().filter(|row| row.kind == "needs_you").collect();
    assert_eq!(stuck.len(), 1, "{stuck:#?}");

    h.runtime.dispatch_deliveries(now_ms());
    let id = pending(&h).into_iter().find(|d| d.kind == Kind::NeedsYou).unwrap().id;

    // Cleared in one window before another claims the banner.
    h.runtime.clear_escalation(open[0].id).unwrap();
    assert!(!h.runtime.claim_delivery(&id).unwrap(), "nothing to show about a cleared wall");
    let row = rows(&h).into_iter().find(|row| row.id == id).unwrap();
    assert_eq!((row.status.as_str(), row.last_error.as_deref()), ("suppressed", Some("answered")));

    // Seen is the same for a finished run: opening the channel withdraws it.
    let finished: Vec<Row> = rows(&h).into_iter().filter(|row| row.kind == "outcome").collect();
    assert_eq!(finished.len(), 2, "{finished:#?}");
    assert!(finished.iter().all(|row| row.status == "pending"), "gathering: {finished:#?}");
    looking_at(&h, "Analyst", "desk");
    let finished: Vec<Row> = rows(&h).into_iter().filter(|row| row.kind == "outcome").collect();
    assert!(
        finished
            .iter()
            .all(|row| row.status == "suppressed" && row.last_error.as_deref() == Some("seen")),
        "{finished:#?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn finished_work_while_the_operator_is_elsewhere_goes_out_only_if_they_leave() {
    let stub = serve(|_| Script::Say("Booked Friday.".into())).await;
    let h = harness(&stub, &["Planner", "Notes"], GuardLimits::default());
    looking_at(&h, "Notes", "desk");
    let seen_at = now_ms();

    let run = h.runtime.send_from_human(h.id("Planner"), "Book Friday.").unwrap();
    h.settle(run).await;
    assert_eq!(h.runtime.dispatch_deliveries(now_ms()), 0, "held while they are here");

    // They leave: the window loses focus and says so.
    let view = OperatorView { agent_id: Some(h.id("Notes")), ..Default::default() };
    h.runtime.report_window(view, false, Some("desk".into()));
    let away = now_ms().max(seen_at) + AWAY_AFTER_MS;
    assert_eq!(h.runtime.dispatch_deliveries(away), 0, "gathered once they count as away");
    assert_eq!(h.runtime.dispatch_deliveries(away + GATHER_MS), 1);
    let waiting = pending(&h);
    assert_eq!(
        (waiting[0].title.as_str(), waiting[0].body.as_str()),
        ("Planner", "Booked Friday.")
    );
    assert_eq!(waiting[0].target, Target::Channel { agent_id: h.id("Planner") });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_window_losing_focus_changes_nothing_an_agent_reads() {
    let stub = serve(|_| Script::Say("Hello.".into())).await;
    let h = harness(&stub, &["Pip", "Ada"], GuardLimits::default());
    looking_at(&h, "Pip", "desk");
    let before = h.runtime.describe_view();

    let view = OperatorView { agent_id: Some(h.id("Ada")), ..Default::default() };
    h.runtime.report_window(view, false, Some("laptop".into()));
    assert_eq!(h.runtime.describe_view(), before, "presence is not part of what an agent reads");
    assert!(before.contains("Pip"), "{before}");
}
