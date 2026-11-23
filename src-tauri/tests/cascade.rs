mod harness;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;

use guac_lib::config::{AppConfig, InferenceConfig};
use guac_lib::db::Store;
use guac_lib::domain::agent::{Lifecycle, COMPOST_MS};
use guac_lib::domain::approval::{ApprovalState, Decision, ProtectedAction, Request};
use guac_lib::domain::connector::CleanConnector;
use guac_lib::domain::envelope::{Envelope, Part, Participant, ToolOutcome, TIDY};
use guac_lib::domain::group::{CleanGroup, GroupLimits, InferenceOverrides, Tidy};
use guac_lib::domain::now_ms;
use guac_lib::domain::outcome::Status as OutcomeStatus;
use guac_lib::domain::routine::{Cadence, EventTrigger, RunKind, Trigger};
use guac_lib::domain::signin::{Signin, Surface};
use guac_lib::llm::openrouter::LlmClient;
use guac_lib::runtime::events::{Activity, RecordingSink, UiEvent};
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::runtime::{prompt, OnDisk, Runtime, SCHEDULE_TICK};

use harness::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queued_work_preserves_approval_activity_until_the_request_is_answered() {
    let requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stub = serve(move |_body| {
        if !requested.swap(true, Ordering::SeqCst) {
            Script::Settings(serde_json::json!({
                "action": "update", "changes": { "limits": { "maxHops": 12 } },
            }))
        } else {
            Script::Say("Done.".into())
        }
    })
    .await;
    let h = harness(&stub, &["Helper"], GuardLimits::default());
    let id = h.id("Helper");
    let first = h.runtime.send_from_human(id, "Change the limit.").unwrap();
    let request = h.awaited_request().await;
    let checkpoint = h.sink.snapshot().len();
    let second = h.runtime.send_from_human(id, "Another task.").unwrap();
    assert_eq!(h.runtime.activity_snapshot().get(&id), Some(&Activity::AwaitingApproval));
    assert_eq!(
        h.runtime.store().get_approval(request).unwrap().unwrap().state,
        ApprovalState::Pending
    );
    assert!(!h.sink.snapshot().iter().skip(checkpoint).any(|event| matches!(event,
        UiEvent::ActivityChanged { agent_id, activity: Activity::Queued { .. } } if *agent_id == id)));
    h.runtime.decide_approval(request, Decision::Allow).unwrap();
    h.settle(first).await;
    h.settle(second).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queued_work_preserves_paused_activity_until_resume() {
    let stub = quiet().await;
    let h = harness(&stub, &["Writer"], GuardLimits::default());
    let id = h.id("Writer");
    let first = h.park("Writer", "First task.").await;
    let checkpoint = h.sink.snapshot().len();
    let second = h.runtime.send_from_human(id, "Second task.").unwrap();
    assert_eq!(h.runtime.activity_snapshot().get(&id), Some(&Activity::Paused));
    assert!(!h.sink.snapshot().iter().skip(checkpoint).any(|event| matches!(event,
        UiEvent::ActivityChanged { agent_id, activity: Activity::Queued { .. } } if *agent_id == id)));
    h.resume("Writer");
    h.settle(first).await;
    h.settle(second).await;
}

#[tokio::test]
async fn a_peer_remembers_the_answer_it_sent_on_its_previous_turn() {
    let remembered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = remembered.clone();
    let stub = serve(move |body| {
        if speaker(body) == "Chef" {
            if anyone_said(body, "Approve your previous quote") {
                let has_quote = body["messages"].as_array().unwrap().iter().any(|message| {
                    message["role"] == "assistant"
                        && message["content"].as_str().is_some_and(|text| text.contains("$137"))
                });
                observed.store(has_quote, Ordering::SeqCst);
                return Script::Say("Approval recorded.".into());
            }
            return Script::Say("My quote is $137.".into());
        }
        if has_tool_result(body) || reading_peer_replies(body) {
            return Script::Say("Recorded.".into());
        }
        Script::Instruct {
            recipients: vec!["Chef".into()],
            text: if anyone_said(body, "Approve the quote") {
                "Approve your previous quote."
            } else {
                "Quote the menu."
            }
            .into(),
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let first = h.runtime.send_from_human(h.id("Manager"), "Get a quote.").unwrap();
    h.settle(first).await;
    let second = h.runtime.send_from_human(h.id("Manager"), "Approve the quote.").unwrap();
    h.settle(second).await;

    assert!(
        remembered.load(Ordering::SeqCst),
        "Chef lost its own answer because it was filed in Manager's channel:\n{}",
        h.transcript()
    );
}

#[tokio::test]
async fn an_exhausted_turn_leaves_new_work_its_own_budget() {
    let hook = Arc::new(std::sync::OnceLock::new());
    let followup = Arc::new(std::sync::OnceLock::new());
    let armed: Arc<std::sync::OnceLock<(Runtime, guac_lib::domain::ids::AgentId)>> = hook.clone();
    let sent = followup.clone();
    let stub = serve(move |body| {
        if anyone_said(body, "Check the new numbers") {
            return Script::Say("New numbers checked.".into());
        }
        let (runtime, writer) = armed.get().unwrap();
        sent.set(runtime.send_from_human(*writer, "Check the new numbers.").unwrap()).unwrap();
        Script::Directory
    })
    .await;

    let h =
        harness(&stub, &["Writer"], GuardLimits { max_steps_per_run: 1, ..GuardLimits::default() });
    let _ = hook.set((h.runtime.clone(), h.id("Writer")));
    let first = h.runtime.send_from_human(h.id("Writer"), "Check the directory.").unwrap();
    h.settle(first).await;
    h.settle(*followup.get().unwrap()).await;

    assert_eq!(stub.calls.load(Ordering::SeqCst), 2, "each run has one call to spend");
    assert!(
        h.channel_texts("Writer").iter().any(|text| text == "New numbers checked."),
        "the exhausted turn consumed fresh work without showing it to the model"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn manager_introduces_itself_to_every_other_agent() {
    let peers = ["Chef", "Host", "Barista", "Sommelier"];
    let stub = serve(move |body| {
        let who = speaker(body);
        if who == "Manager" {
            if reading_peer_replies(body) {
                Script::Say("Everyone has introduced themselves.".into())
            } else if has_tool_result(body) {
                Script::Say("Introductions are done.".into())
            } else {
                Script::SendTo {
                    recipients: peers.iter().map(|s| s.to_string()).collect(),
                    text: "Hi, I'm Manager. I coordinate this workspace.".into(),
                }
            }
        } else {
            Script::Say(format!("Hi Manager, I'm {who}."))
        }
    })
    .await;

    let h = harness(
        &stub,
        &["Manager", "Chef", "Host", "Barista", "Sommelier"],
        GuardLimits::default(),
    );
    let run = h
        .runtime
        .send_from_human(h.id("Manager"), "Introduce yourself to all the other agents.")
        .unwrap();
    h.settle(run).await;

    for peer in peers {
        let texts = h.channel_texts(peer);
        assert!(
            texts.iter().any(|t| t.contains("I'm Manager")),
            "{peer} never received the introduction. Channel was: {texts:?}"
        );
    }

    let manager_channel = h.channel_texts("Manager").join("\n");
    for peer in peers {
        assert!(
            manager_channel.contains(&format!("I'm {peer}")),
            "{peer}'s reply never reached the Manager channel. Channel was:\n{manager_channel}"
        );
    }

    assert_eq!(h.feed().len(), 8, "expected 4 sends and 4 replies in the activity feed");

    let calls = stub.calls.load(Ordering::SeqCst);
    assert!(
        (6..=10).contains(&calls),
        "the cascade should converge in 6-10 model calls, took {calls}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_that_arrives_mid_turn_reaches_the_turn_that_is_working() {
    let hook: Arc<std::sync::OnceLock<(Runtime, guac_lib::domain::ids::AgentId)>> =
        Arc::new(std::sync::OnceLock::new());
    let typed = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let armed = hook.clone();
    let once = typed.clone();
    let stub = serve(move |body| {
        if speaker(body) != "Writer" {
            return Script::Say("ok".into());
        }
        if anyone_said(body, "Call it we") {
            return Script::Say("Changed it to we.".into());
        }
        if !once.swap(true, Ordering::SeqCst) {
            if let Some((runtime, writer)) = armed.get() {
                runtime.send_from_human(*writer, "Stop writing I. Call it we.").unwrap();
            }
            return Script::Progress("drafting the post".into());
        }
        Script::Say("Draft done.".into())
    })
    .await;

    let h = harness(&stub, &["Writer"], GuardLimits::default());
    let writer = h.id("Writer");
    let _ = hook.set((h.runtime.clone(), writer));

    let run = h.runtime.send_from_human(writer, "Write the post.").unwrap();
    h.settle(run).await;

    let texts = h.channel_texts("Writer");
    assert!(
        texts.iter().any(|t| t.contains("Changed it to we")),
        "the turn never read what the operator typed while it worked: {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.contains("Draft done")),
        "the turn finished on its own answer and read the correction afterwards: {texts:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn work_that_lands_mid_turn_is_reported_even_when_the_turn_owed_nobody_words() {
    let hook: Arc<std::sync::OnceLock<(Runtime, guac_lib::domain::ids::AgentId)>> =
        Arc::new(std::sync::OnceLock::new());

    let armed = hook.clone();
    let stub = serve(move |body| {
        if speaker(body) == "Manager" {
            return Script::Say("Here is the answer.".into());
        }
        if anyone_said(body, "fix the headline") {
            return Script::Say(String::new());
        }
        if reading_peer_replies(body) {
            if let Some((runtime, writer)) = armed.get() {
                runtime.send_from_human(*writer, "Also fix the headline.").unwrap();
            }
            return Script::Progress("tidying up".into());
        }
        if has_tool_result(body) {
            return Script::Say("Asked.".into());
        }
        Script::SendTo { recipients: vec!["Manager".into()], text: "Quick question.".into() }
    })
    .await;

    let h = harness(&stub, &["Manager", "Writer"], GuardLimits::default());
    let _ = hook.set((h.runtime.clone(), h.id("Writer")));

    let run =
        h.runtime.send_from_human(h.id("Writer"), "Ask the Manager about the launch.").unwrap();
    h.settle(run).await;

    h.wait_until("the silent turn to be reported", |h| {
        h.runtime
            .store()
            .channel_messages(h.id("Writer"), 50)
            .unwrap()
            .into_iter()
            .flat_map(|e| e.parts)
            .any(|part| {
                matches!(part, Part::Notice { ref text, .. } if text.contains("without reporting anything"))
            })
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_the_prompt_already_carries_is_taken_in_once() {
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = seen.clone();
    let stub = serve(move |body| {
        let times = body["messages"]
            .as_array()
            .map(|messages| {
                messages
                    .iter()
                    .filter(|m| m["role"] != "system")
                    .filter_map(|m| m["content"].as_str())
                    .filter(|c| c.contains("Second thing"))
                    .count()
            })
            .unwrap_or(0);
        counter.fetch_max(times, Ordering::SeqCst);
        Script::Say("Both noted.".into())
    })
    .await;

    let h = harness(&stub, &["Writer"], GuardLimits::default());
    let writer = h.id("Writer");

    h.pause("Writer");
    let run = h.runtime.send_from_human(writer, "First thing to do.").unwrap();
    h.runtime.send_from_human(writer, "Second thing, while you are at it.").unwrap();
    h.wait_until("both messages to be filed", |h| h.channel_texts("Writer").len() == 2).await;
    h.resume("Writer");
    h.settle(run).await;

    assert_eq!(
        seen.load(Ordering::SeqCst),
        1,
        "the second message reached the model more than once in the same call"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_answering_a_peer_leaves_the_operator_a_turn_of_their_own() {
    let hook: Arc<std::sync::OnceLock<(Runtime, guac_lib::domain::ids::AgentId)>> =
        Arc::new(std::sync::OnceLock::new());
    let typed = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let armed = hook.clone();
    let once = typed.clone();
    let stub = serve(move |body| {
        let who = speaker(body);
        if who == "Manager" {
            return if has_tool_result(body) {
                Script::Say("Asked.".into())
            } else {
                Script::Instruct {
                    recipients: vec!["Writer".into()],
                    text: "Draft the announcement.".into(),
                }
            };
        }
        if anyone_said(body, "Call it we") {
            return Script::Say("Noted for next time: we.".into());
        }
        if !once.swap(true, Ordering::SeqCst) {
            if let Some((runtime, writer)) = armed.get() {
                runtime.send_from_human(*writer, "Stop writing I. Call it we.").unwrap();
            }
            return Script::Progress("drafting".into());
        }
        Script::Say("Here is the announcement.".into())
    })
    .await;

    let h = harness(&stub, &["Manager", "Writer"], GuardLimits::default());
    let _ = hook.set((h.runtime.clone(), h.id("Writer")));

    let run = h.runtime.send_from_human(h.id("Manager"), "Get the announcement drafted.").unwrap();
    h.settle(run).await;

    let manager = h.channel_texts("Manager");
    assert!(
        manager.iter().any(|t| t.contains("Here is the announcement")),
        "the peer that asked never got its answer: {manager:?}"
    );

    h.wait_until("the operator's message to be answered", |h| {
        h.channel_texts("Writer").iter().any(|t| t.contains("Noted for next time"))
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn replies_queued_together_are_read_in_a_single_turn() {
    let peers = ["Chef", "Host", "Barista", "Sommelier"];
    let stub = serve(move |body| {
        let who = speaker(body);
        if who == "Manager" {
            if reading_peer_replies(body) {
                Script::Say("Noted.".into())
            } else if has_tool_result(body) {
                Script::Say("Sent.".into())
            } else {
                Script::SendTo {
                    recipients: peers.iter().map(|s| s.to_string()).collect(),
                    text: "Hello there.".into(),
                }
            }
        } else {
            std::thread::sleep(Duration::from_millis(250));
            Script::Say(format!("Acknowledged, from {who}."))
        }
    })
    .await;

    let h = harness(
        &stub,
        &["Manager", "Chef", "Host", "Barista", "Sommelier"],
        GuardLimits::default(),
    );
    let manager = h.id("Manager");
    let run = h.runtime.send_from_human(manager, "Say hello to everyone.").unwrap();

    h.wait_until("the introductions to be sent", |h| {
        h.feed().iter().filter(|e| e.from == Participant::Agent { id: manager }).count() == 4
    })
    .await;
    h.pause("Manager");

    h.wait_until("all four replies to be queued", |h| {
        let delivered =
            h.feed().iter().filter(|e| e.to == Participant::Agent { id: manager }).count();
        delivered == 4 && h.runtime.inbox_depth(manager) >= 3
    })
    .await;

    h.resume("Manager");
    h.settle(run).await;

    let prompts_reading_replies: Vec<usize> = stub
        .transcript
        .lock()
        .iter()
        .filter(|body| speaker(body) == "Manager" && reading_peer_replies(body))
        .map(|body| {
            body["messages"]
                .as_array()
                .and_then(|m| m.last())
                .and_then(|m| m["content"].as_str())
                .map(|c| c.matches("[AGENT").count())
                .unwrap_or(0)
        })
        .collect();

    assert_eq!(
        prompts_reading_replies.len(),
        1,
        "four queued replies must cost one turn, not {}",
        prompts_reading_replies.len()
    );
    assert_eq!(prompts_reading_replies[0], 4, "all four replies must appear in that single prompt");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reply_still_being_worked_on_is_waited_for_rather_than_read_alone() {
    let stub = serve(move |body| {
        let who = speaker(body);
        if who == "Manager" {
            if reading_peer_replies(body) {
                Script::Say("Both heard from.".into())
            } else if has_tool_result(body) {
                Script::Say("Sent.".into())
            } else {
                Script::SendTo {
                    recipients: vec!["Chef".into(), "Grocer".into()],
                    text: "Hello there.".into(),
                }
            }
        } else {
            if who == "Grocer" {
                std::thread::sleep(Duration::from_millis(3200));
            }
            Script::Say(format!("Acknowledged, from {who}."))
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef", "Grocer"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Say hello to both.").unwrap();
    h.settle(run).await;

    let prompts_reading_replies: Vec<usize> = stub
        .transcript
        .lock()
        .iter()
        .filter(|body| speaker(body) == "Manager" && reading_peer_replies(body))
        .map(|body| {
            body["messages"]
                .as_array()
                .and_then(|m| m.last())
                .and_then(|m| m["content"].as_str())
                .map(|c| c.matches("[AGENT").count())
                .unwrap_or(0)
        })
        .collect();
    assert_eq!(
        prompts_reading_replies.len(),
        1,
        "a slow reply must be waited for, not read in a turn of its own\n{}",
        h.transcript()
    );
    assert_eq!(
        prompts_reading_replies[0], 2,
        "both replies must appear in the single reading prompt"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_answer_owed_by_a_peer_that_failed_is_not_waited_out() {
    let stub = serve(move |body| {
        let who = speaker(body);
        if who == "Manager" {
            if reading_peer_replies(body) {
                Script::Say("Heard from Chef.".into())
            } else if has_tool_result(body) {
                Script::Say("Sent.".into())
            } else {
                Script::SendTo {
                    recipients: vec!["Chef".into(), "Grocer".into()],
                    text: "Hello there.".into(),
                }
            }
        } else if who == "Grocer" {
            Script::Unavailable
        } else {
            Script::Say("Acknowledged, from Chef.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef", "Grocer"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Say hello to both.").unwrap();
    h.settle(run).await;

    assert!(
        h.channel_texts("Manager").iter().any(|t| t.contains("Acknowledged, from Chef.")),
        "Chef's reply still has to arrive:\n{}",
        h.transcript()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_operator_line_reaches_the_model_after_the_first_rather_than_above_it() {
    let stub = serve(|body| {
        let _ = body;
        Script::Say("Both noted.".into())
    })
    .await;

    let h = harness(&stub, &["Writer"], GuardLimits::default());
    let writer = h.id("Writer");

    h.pause("Writer");
    let run = h.runtime.send_from_human(writer, "First thing: book the 4pm slot.").unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    h.runtime.send_from_human(writer, "Second thing: actually make that 5pm.").unwrap();
    h.wait_until("both lines to be filed", |h| h.channel_texts("Writer").len() == 2).await;
    h.resume("Writer");
    h.settle(run).await;

    let order: Vec<(usize, usize)> = stub
        .transcript
        .lock()
        .iter()
        .filter(|body| speaker(body) == "Writer")
        .map(|body| {
            let contents: Vec<&str> = body["messages"]
                .as_array()
                .map(|m| {
                    m.iter()
                        .filter(|m| m["role"] != "system")
                        .filter_map(|m| m["content"].as_str())
                        .collect()
                })
                .unwrap_or_default();
            let position = |needle: &str| {
                contents.iter().position(|c| c.contains(needle)).unwrap_or(usize::MAX)
            };
            (position("First thing"), position("Second thing"))
        })
        .collect();

    assert!(!order.is_empty(), "the turn never reached the model");
    for (first, second) in order {
        assert!(first != usize::MAX && second != usize::MAX, "both lines must reach the model");
        assert!(
            first < second,
            "the second line was rendered above the first: correction before the thing corrected\n{}",
            h.transcript()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_agents_told_to_talk_forever_stop_on_their_own() {
    let stub = serve(|body| {
        let who = speaker(body);
        let other = if who == "Ping" { "Pong" } else { "Ping" };
        Script::SendTo {
            recipients: vec![other.to_string()],
            text: format!(
                "message {} from {who}",
                body["messages"].as_array().map(|m| m.len()).unwrap_or(0)
            ),
        }
    })
    .await;

    let h = harness(
        &stub,
        &["Ping", "Pong"],
        GuardLimits {
            max_hops: 4,
            max_steps_per_run: 12,
            max_fanout_per_call: 8,
            max_sends_per_pair: 3,
            max_tool_rounds: 24,
        },
    );
    let run = h.runtime.send_from_human(h.id("Ping"), "Talk to Pong forever.").unwrap();
    h.settle(run).await;

    let calls = stub.calls.load(Ordering::SeqCst);
    assert!(calls <= 14, "runaway loop: {calls} inference calls");

    let feed = h.feed();
    assert!(!feed.is_empty(), "the agents should have exchanged something before stopping");
    assert!(feed.len() <= 8, "too much traffic before the guard bit: {}", feed.len());

    let all: String = h
        .runtime
        .store()
        .channel_messages(h.id("Ping"), 200)
        .unwrap()
        .iter()
        .chain(h.runtime.store().channel_messages(h.id("Pong"), 200).unwrap().iter())
        .map(|e| serde_json::to_string(&e.parts).unwrap())
        .collect();
    assert!(
        all.contains("limit") || all.contains("budget") || all.contains("Refused"),
        "the operator must be told why the conversation stopped"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_repeated_identical_message_is_refused() {
    let stub = serve(|body| {
        let who = speaker(body);
        if who == "Ping" {
            Script::SendTo { recipients: vec!["Pong".into()], text: "identical text".into() }
        } else {
            Script::Say("ok".into())
        }
    })
    .await;

    let h = harness(&stub, &["Ping", "Pong"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Ping"), "Message Pong repeatedly.").unwrap();
    h.settle(run).await;

    let delivered = h.feed().iter().filter(|e| e.plain_text() == "identical text").count();
    assert_eq!(delivered, 1, "the same message to the same peer must only go once");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn messaging_an_agent_that_does_not_exist_is_reported_to_the_model() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("That agent does not exist.".into())
        } else {
            Script::SendTo { recipients: vec!["Nobody".into()], text: "hello?".into() }
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Message Nobody.").unwrap();
    h.settle(run).await;

    let tool_results: Vec<String> = stub
        .transcript
        .lock()
        .iter()
        .flat_map(|body| {
            body["messages"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|m| m["role"] == "tool")
                .map(|m| m["content"].as_str().unwrap_or_default().to_string())
                .collect::<Vec<_>>()
        })
        .collect();

    assert!(
        tool_results.iter().any(|r| r.contains("no agent named Nobody")),
        "the model must learn the recipient does not exist, got {tool_results:?}"
    );
    assert!(h.feed().is_empty(), "nothing should have been delivered");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agents_tool_trail_stays_in_its_own_channel() {
    let stub = serve(|body| {
        let who = speaker(body);
        if who == "Manager" {
            if has_tool_result(body) {
                Script::Say("Asked Chef to take a look.".into())
            } else {
                Script::SendTo { recipients: vec!["Chef".into()], text: "please review".into() }
            }
        } else {
            Script::Say("Reviewed.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "get Chef on it").unwrap();
    h.settle(run).await;

    let tool_parts_in = |name: &str| {
        h.runtime
            .store()
            .channel_messages(h.id(name), 200)
            .unwrap()
            .iter()
            .flat_map(|e| e.parts.clone())
            .filter(|p| matches!(p, Part::ToolCall { .. }))
            .count()
    };

    assert!(tool_parts_in("Manager") > 0, "the acting agent keeps its own record");
    assert_eq!(tool_parts_in("Chef"), 0, "Chef's channel must not hold Manager's tool calls");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_guard_refusal_is_reported_to_the_agent_that_hit_it() {
    let stub = serve(|body| {
        let who = speaker(body);
        let other = if who == "Ping" { "Pong" } else { "Ping" };
        Script::SendTo {
            recipients: vec![other.to_string()],
            text: format!("relay {}", body["messages"].as_array().map(|m| m.len()).unwrap_or(0)),
        }
    })
    .await;

    let h = harness(
        &stub,
        &["Ping", "Pong"],
        GuardLimits { max_hops: 2, max_steps_per_run: 10, ..GuardLimits::default() },
    );
    let run = h.runtime.send_from_human(h.id("Ping"), "talk forever").unwrap();
    h.settle(run).await;

    let notices_in = |name: &str| {
        h.runtime
            .store()
            .channel_messages(h.id(name), 200)
            .unwrap()
            .iter()
            .flat_map(|e| e.parts.clone())
            .filter(|p| matches!(p, Part::Notice { .. }))
            .count()
    };
    assert!(
        notices_in("Ping") + notices_in("Pong") > 0,
        "the operator must be told why it stopped"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_directory_tool_lists_peers_but_never_their_prompts() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Found them.".into())
        } else {
            Script::Directory
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Who else is here?").unwrap();
    h.settle(run).await;

    let tool_results: Vec<String> = stub
        .transcript
        .lock()
        .iter()
        .flat_map(|body| {
            body["messages"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|m| m["role"] == "tool")
                .map(|m| m["content"].as_str().unwrap_or_default().to_string())
                .collect::<Vec<_>>()
        })
        .collect();

    let joined = tool_results.join("\n");
    assert!(joined.contains("Chef"), "directory should list Chef: {joined}");
    assert!(!joined.contains("Manager"), "an agent should not see itself in the directory");
    assert!(
        !joined.contains("You are the Chef."),
        "the directory must never expose another agent's system prompt: {joined}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_deleted_agent_stops_receiving_but_keeps_its_transcript() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Understood.".into())
        } else {
            Script::SendTo { recipients: vec!["Chef".into()], text: "are you there?".into() }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());

    let first = h.runtime.send_from_human(h.id("Chef"), "hello Chef").unwrap();
    h.settle(first).await;
    assert!(!h.channel_texts("Chef").is_empty());

    h.runtime.store().set_lifecycle(h.id("Chef"), Lifecycle::Terminated).unwrap();
    h.runtime.stop_agent(h.id("Chef"));

    let run = h.runtime.send_from_human(h.id("Manager"), "Message Chef.").unwrap();
    h.settle(run).await;

    assert!(
        h.feed().iter().all(|e| e.to != Participant::Agent { id: h.id("Chef") }),
        "a deleted agent must not receive new messages"
    );
    assert!(
        !h.channel_texts("Chef").is_empty(),
        "deleting an agent must not destroy the record of what it said"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deleting_a_paused_agent_stops_its_actor() {
    let stub = serve(|_| Script::Say("ok".into())).await;
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    assert_eq!(h.runtime.live_actors(), 2);

    h.park("Chef", "you are paused").await;

    h.runtime.store().set_lifecycle(h.id("Chef"), Lifecycle::Terminated).unwrap();
    h.runtime.stop_agent(h.id("Chef"));

    h.wait_until("Chef's actor to exit", |h| h.runtime.live_actors() == 1).await;
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upstream_failure_is_reported_in_the_channel_rather_than_swallowed() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|| async {
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    axum::Json(
                        serde_json::json!({"error": {"message": "No auth credentials found"}}),
                    ),
                )
                    .into_response()
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let stub = Stub {
        base_url: format!("http://{addr}/v1"),
        calls: Arc::new(AtomicUsize::new(0)),
        transcript: Arc::new(parking_lot::Mutex::new(Vec::new())),
    };
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "hello").unwrap();
    h.settle(run).await;

    let parts: String = h
        .runtime
        .store()
        .channel_messages(h.id("Manager"), 50)
        .unwrap()
        .iter()
        .map(|e| serde_json::to_string(&e.parts).unwrap())
        .collect();
    assert!(
        parts.contains("No auth credentials") || parts.contains("rejected the API key"),
        "an auth failure must show up in the channel, got {parts}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_model_call_is_told_plainly_and_keeps_the_provider_words_as_detail() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|| async {
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(
                        serde_json::json!({"error": {"message": "The model is overloaded."}}),
                    ),
                )
                    .into_response()
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let stub = Stub {
        base_url: format!("http://{addr}/v1"),
        calls: Arc::new(AtomicUsize::new(0)),
        transcript: Arc::new(parking_lot::Mutex::new(Vec::new())),
    };
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "hello").unwrap();
    h.settle(run).await;

    let notices: Vec<(String, Option<String>)> = h
        .runtime
        .store()
        .channel_messages(h.id("Manager"), 50)
        .unwrap()
        .into_iter()
        .flat_map(|e| e.parts)
        .filter_map(|part| match part {
            Part::Notice { kind: NoticeKind::UpstreamError, text, detail } => Some((text, detail)),
            _ => None,
        })
        .collect();
    let [(text, detail)] = notices.as_slice() else {
        panic!("one failure notice, got {notices:?}");
    };
    assert_eq!(
        text,
        "Manager could not reply: the provider had a problem on its side. Wait a minute, then try again."
    );
    let detail = detail.as_deref().unwrap_or_default();
    assert!(
        detail.contains("503") && detail.contains("The model is overloaded."),
        "the provider's words stay one click away: {detail:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_progress_note_is_in_the_next_turn_with_how_long_ago_it_was_written() {
    let stub = serve(|body| {
        let system = body["messages"][0]["content"].as_str().unwrap_or_default();
        if system.contains("waiting on the legal read") {
            let age = if system.contains("just now") { "fresh" } else { "stale" };
            Script::Say(format!("still waiting, and the note reads {age}."))
        } else if has_tool_result(body) {
            Script::Say("Noted.".into())
        } else {
            Script::Progress("waiting on the legal read".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let first = h.runtime.send_from_human(h.id("Manager"), "chase the legal read").unwrap();
    h.settle(first).await;

    let notes = h.runtime.store().working_notes(h.id("Manager")).unwrap();
    assert_eq!(notes.len(), 1, "the note should be stored");
    assert_eq!(notes[0].body, "waiting on the legal read");
    assert_eq!(
        h.runtime.workspace().read(h.id("Manager")),
        "",
        "a progress note must not reach the memory file"
    );

    let second = h.runtime.send_from_human(h.id("Manager"), "any news?").unwrap();
    h.settle(second).await;

    let said = h.channel_texts("Manager").join("\n");
    assert!(said.contains("still waiting"), "the note was not in the second prompt: {said}");
    assert!(said.contains("reads fresh"), "the note reached the prompt without its age: {said}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_progress_note_tells_the_agent_how_many_it_now_holds() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            let results = serde_json::to_string(&body["messages"]).unwrap_or_default();
            Script::Say(results)
        } else {
            Script::Progress("handed the scope document to Robert".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "where are we?").unwrap();
    h.settle(run).await;

    let said = h.channel_texts("Manager").join("\n");
    assert!(said.contains("1 of 16 working notes"), "the count did not come back: {said}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn noting_the_same_line_twice_stores_it_once_and_says_how_old_the_first_is() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say(serde_json::to_string(&body["messages"]).unwrap_or_default())
        } else {
            Script::Progress("waiting on the legal read".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let first = h.runtime.send_from_human(h.id("Manager"), "chase the legal read").unwrap();
    h.settle(first).await;
    let second = h.runtime.send_from_human(h.id("Manager"), "any news?").unwrap();
    h.settle(second).await;

    let notes = h.runtime.store().working_notes(h.id("Manager")).unwrap();
    assert_eq!(notes.len(), 1, "the repeat was stored as a second note");

    let said = h.channel_texts("Manager").join("\n");
    assert!(
        said.contains("You noted that already"),
        "the repeat came back as an ordinary note: {said}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_can_write_its_memory_and_reads_it_back_next_turn() {
    let stub = serve(|body| {
        let system = body["messages"][0]["content"].as_str().unwrap_or_default();
        if system.contains("Operator prefers terse replies") {
            Script::Say("I remember.".into())
        } else if has_tool_result(body) {
            Script::Say("Noted.".into())
        } else {
            Script::Notes("Operator prefers terse replies.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let first = h.runtime.send_from_human(h.id("Manager"), "be terse from now on").unwrap();
    h.settle(first).await;

    assert_eq!(
        h.runtime.workspace().read(h.id("Manager")),
        "Operator prefers terse replies.",
        "the note should be on disk"
    );

    let second = h.runtime.send_from_human(h.id("Manager"), "what do you remember?").unwrap();
    h.settle(second).await;

    let said = h.channel_texts("Manager").join("\n");
    assert!(said.contains("I remember."), "the note was not in the second prompt: {said}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_calling_the_old_name_for_its_memory_still_writes_it() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Remembered.".into())
        } else {
            Script::Memory("Operator prefers terse replies.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "update your memory: be terse").unwrap();
    h.settle(run).await;

    assert_eq!(
        h.runtime.workspace().read(h.id("Manager")),
        "Operator prefers terse replies.",
        "a memory written under the operator's word for it went nowhere"
    );
    let said = h.channel_texts("Manager").join("\n");
    assert!(said.contains("Remembered."), "the turn should have carried on, got {said}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_memory_write_records_the_version_it_replaced() {
    let stub = serve(|body| {
        let system = body["messages"][0]["content"].as_str().unwrap_or_default();
        if has_tool_result(body) {
            Script::Say("Done.".into())
        } else if system.contains("Smith handles verification.") {
            Script::Notes("Smith handles verification.\nJones signs off.".into())
        } else {
            Script::Notes("Smith handles verification.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let first = h.runtime.send_from_human(h.id("Manager"), "remember who verifies").unwrap();
    h.settle(first).await;
    let second = h.runtime.send_from_human(h.id("Manager"), "and who signs off").unwrap();
    h.settle(second).await;

    let replaced: Vec<Option<String>> = h
        .runtime
        .store()
        .channel_messages(h.id("Manager"), 200)
        .unwrap()
        .iter()
        .flat_map(|m| m.parts.clone())
        .filter_map(|part| match part {
            Part::ToolCall { name, replaced, .. } if name == "update_memory" => Some(replaced),
            _ => None,
        })
        .collect();

    assert_eq!(
        replaced,
        vec![Some(String::new()), Some("Smith handles verification.".to_string())],
        "the first write replaced nothing and the second replaced the first"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_memory_write_tells_the_panel_drawing_it() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Done.".into())
        } else {
            Script::Notes("Smith handles verification.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "remember who verifies").unwrap();
    h.settle(run).await;

    let manager = h.id("Manager");
    assert_eq!(
        h.sink
            .count_of(|e| matches!(e, UiEvent::MemoryChanged { agent_id } if *agent_id == manager)),
        1,
        "the agent rewrote its memory and nothing said so"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_memory_write_that_failed_tells_nobody_it_changed() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Done.".into())
        } else {
            Script::Notes("Smith handles verification.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    std::fs::write(h.runtime.workspace().root(), b"not a directory").unwrap();

    let run = h.runtime.send_from_human(h.id("Manager"), "remember who verifies").unwrap();
    h.settle(run).await;

    let failed = h
        .runtime
        .store()
        .channel_messages(h.id("Manager"), 200)
        .unwrap()
        .iter()
        .flat_map(|m| m.parts.clone())
        .any(|part| {
            matches!(part, Part::ToolCall { name, outcome, .. }
                if name == "update_memory" && matches!(outcome, ToolOutcome::Failed { .. }))
        });
    assert!(failed, "the write was supposed to fail, so the assertion below proves nothing");

    assert_eq!(
        h.sink.count_of(|e| matches!(e, UiEvent::MemoryChanged { .. })),
        0,
        "nothing was stored, so nothing changed"
    );
}

fn answers_so_far(body: &serde_json::Value) -> Vec<String> {
    body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|message| message["role"] == "tool")
        .map(|message| message["content"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn operator_saves(workspace: &guac_lib::workspace::Workspace, id: guac_lib::domain::ids::AgentId) {
    let shown = workspace.read_versioned(id);
    let saved = workspace
        .write_versioned(id, "Manager", "Jones handles verification.", Some(&shown.version))
        .unwrap();
    assert!(matches!(saved, guac_lib::workspace::WriteResult::Stored(_)));
}

const MEMORY_CONFLICT: &str =
    "Not saved: the operator edited your memory while you were working, and \
     your rewrite started from the page as it was before. Their version is below. Make your \
     change again on top of it.\n\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_operator_save_during_a_turn_is_not_overwritten_by_the_agents_rewrite() {
    let workspace = Arc::new(std::sync::OnceLock::new());
    let saving = workspace.clone();
    let stub = serve(move |body| {
        if has_tool_result(body) {
            return Script::Say("Done.".into());
        }
        let (workspace, id) = saving.get().expect("the harness is built before the turn runs");
        operator_saves(workspace, *id);
        Script::Memory("Smith handles verification.\nThe operator likes tables.".into())
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let manager = h.id("Manager");
    h.runtime.workspace().write(manager, "Manager", "Smith handles verification.").unwrap();
    workspace.set((h.runtime.workspace().clone(), manager)).unwrap();

    let run = h.runtime.send_from_human(manager, "remember that I like tables").unwrap();
    h.settle(run).await;

    assert_eq!(
        h.runtime.workspace().read(manager),
        "Jones handles verification.",
        "the operator's correction was overwritten by a page made before it"
    );
    let bodies = stub.transcript.lock().clone();
    let told = bodies.last().map(answers_so_far).unwrap_or_default();
    assert_eq!(told, vec![format!("{MEMORY_CONFLICT}Jones handles verification.")]);
    let refused = h
        .runtime
        .store()
        .channel_messages(manager, 200)
        .unwrap()
        .iter()
        .flat_map(|m| m.parts.clone())
        .any(|part| {
            matches!(part, Part::ToolCall { name, outcome, replaced: None, .. }
                if name == "update_memory" && matches!(outcome, ToolOutcome::Refused { .. }))
        });
    assert!(refused, "the transcript has to show the write that was not saved as not saved");
    assert_eq!(
        h.sink.count_of(|e| matches!(e, UiEvent::MemoryChanged { .. })),
        0,
        "nothing the agent wrote was stored, so the panel has nothing new to read"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_a_refused_rewrite_the_agent_can_make_its_change_on_the_operators_page() {
    let workspace = Arc::new(std::sync::OnceLock::new());
    let saving = workspace.clone();
    let stub = serve(move |body| match answers_so_far(body).len() {
        0 => {
            let (workspace, id) = saving.get().expect("the harness is built before the turn runs");
            operator_saves(workspace, *id);
            Script::Memory("Smith handles verification.\nThe operator likes tables.".into())
        }
        1 => Script::Memory("Jones handles verification.\nThe operator likes tables.".into()),
        _ => Script::Say("Done.".into()),
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let manager = h.id("Manager");
    h.runtime.workspace().write(manager, "Manager", "Smith handles verification.").unwrap();
    workspace.set((h.runtime.workspace().clone(), manager)).unwrap();

    let run = h.runtime.send_from_human(manager, "remember that I like tables").unwrap();
    h.settle(run).await;

    let bodies = stub.transcript.lock().clone();
    let told = bodies.last().map(answers_so_far).unwrap_or_default();
    assert_eq!(told.len(), 2, "{told:?}");
    assert!(told[0].starts_with(MEMORY_CONFLICT), "{told:?}");
    assert_eq!(told[1], "Memory saved (54 characters).");
    assert_eq!(
        h.runtime.workspace().read(manager),
        "Jones handles verification.\nThe operator likes tables."
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_nobody_else_writing_every_rewrite_in_a_turn_is_saved() {
    let stub = serve(|body| match answers_so_far(body).len() {
        0 => Script::Memory("Smith handles verification.".into()),
        1 => Script::Memory("Smith handles verification.\nJones signs off.".into()),
        _ => Script::Say("Done.".into()),
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let manager = h.id("Manager");
    let run = h.runtime.send_from_human(manager, "remember who verifies and signs").unwrap();
    h.settle(run).await;

    let bodies = stub.transcript.lock().clone();
    let told = bodies.last().map(answers_so_far).unwrap_or_default();
    assert_eq!(
        told,
        vec![
            "Memory saved (27 characters).".to_string(),
            "Memory saved (44 characters).".to_string()
        ]
    );
    assert_eq!(
        h.runtime.workspace().read(manager),
        "Smith handles verification.\nJones signs off."
    );
    assert_eq!(
        h.sink
            .count_of(|e| matches!(e, UiEvent::MemoryChanged { agent_id } if *agent_id == manager)),
        2
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_one_agent_remembers_is_never_shown_to_another() {
    let stub = serve(|body| {
        if speaker(body) != "Manager" {
            return Script::Say("Nothing to add.".into());
        }
        if has_tool_result(body) {
            Script::Say("Kept.".into())
        } else {
            Script::Notes("The operator's home address is 12 Rowan Street.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let written = h.runtime.send_from_human(h.id("Manager"), "remember where I live").unwrap();
    h.settle(written).await;
    let again = h.runtime.send_from_human(h.id("Manager"), "still there?").unwrap();
    h.settle(again).await;
    let asked = h.runtime.send_from_human(h.id("Chef"), "anything I should know?").unwrap();
    h.settle(asked).await;

    let prompts = prompts_by_agent(&stub);
    assert!(
        prompts.get("Manager").expect("the Manager ran").contains("12 Rowan Street"),
        "the agent that wrote it has to be able to read it back, or this proves nothing"
    );
    assert!(
        !prompts.get("Chef").expect("Chef ran").contains("Rowan Street"),
        "one agent's memory reached another's prompt: {}",
        prompts["Chef"]
    );
    assert!(
        h.runtime.workspace().read(h.id("Chef")).is_empty(),
        "and nothing was written to the file of an agent that wrote nothing"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deleting_an_agent_takes_its_memory_with_it() {
    let stub = serve(|_| Script::Notes("private".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "remember something").unwrap();
    h.settle(run).await;
    assert_eq!(h.runtime.workspace().read(h.id("Manager")), "private");

    h.runtime.workspace().remove(h.id("Manager")).unwrap();
    assert_eq!(h.runtime.workspace().read(h.id("Manager")), "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streamed_text_matches_the_persisted_message() {
    let stub = serve(|_| Script::Say("The quick brown fox jumps over the lazy dog.".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "say something").unwrap();
    h.settle(run).await;

    let stream_id = h
        .sink
        .snapshot()
        .into_iter()
        .find_map(|e| match e {
            UiEvent::StreamStarted { message_id, .. } => Some(message_id),
            _ => None,
        })
        .expect("a stream should have started");

    let streamed = h.sink.streamed_text(stream_id);
    let persisted = h.channel_texts("Manager").pop().unwrap();
    assert_eq!(streamed, persisted, "what the operator watched appear must equal what was saved");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_that_speaks_twice_is_watched_with_the_break_between_its_rounds() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Two of us.".into())
        } else {
            Script::Narrate {
                text: "Checking who is here.".into(),
                then: Box::new(Script::Directory),
            }
        }
    })
    .await;
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "who else is here?").unwrap();
    h.settle(run).await;

    let stream_id = h
        .sink
        .snapshot()
        .into_iter()
        .find_map(|e| match e {
            UiEvent::StreamStarted { message_id, .. } => Some(message_id),
            _ => None,
        })
        .expect("a stream should have started");

    let persisted = h.channel_texts("Manager").pop().unwrap();
    assert_eq!(persisted, "Checking who is here.\n\nTwo of us.");
    assert_eq!(
        h.sink.streamed_text(stream_id),
        persisted,
        "what the operator watched appear must equal what was saved"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_that_ends_on_a_promise_is_given_the_round_back() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Drive's newest file is still 27 Aug.".into())
        } else if anyone_said(body, "You ended your message with work you had not done") {
            Script::Progress("Drive listing is stale".into())
        } else {
            Script::Say("Both answered, so the plugin is back. Checking both properly.".into())
        }
    })
    .await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "is the plugin back?").unwrap();
    h.settle(run).await;

    let said = h.channel_texts("Manager").pop().unwrap();
    assert!(
        said.contains("still 27 Aug"),
        "the work the turn promised has to happen in the turn that promised it: {said:?}"
    );
    assert_eq!(
        said,
        "Both answered, so the plugin is back. Checking both properly.\n\n\
         Drive's newest file is still 27 Aug."
    );
    assert_eq!(
        calls_by_agent(&stub).get("Manager"),
        Some(&3),
        "the nudge is one model call: the promise, the work, the answer"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_that_promises_again_is_not_argued_with() {
    let stub = serve(|_| Script::Say("Checking now.".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "is the plugin back?").unwrap();
    h.settle(run).await;

    assert_eq!(
        calls_by_agent(&stub).get("Manager"),
        Some(&2),
        "one nudge per turn, however many times the model promises"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_that_says_what_it_found_is_left_alone() {
    let stub =
        serve(|_| Script::Say("Checked both. Drive is stale and Gmail has nothing.".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "is the plugin back?").unwrap();
    h.settle(run).await;

    assert_eq!(calls_by_agent(&stub).get("Manager"), Some(&1), "a report is not a promise");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_retried_round_opens_its_replacement_at_the_left_margin() {
    let answers = Arc::new(AtomicUsize::new(0));
    let stub = serve(move |body| {
        if !has_tool_result(body) {
            return Script::Narrate {
                text: "Checking who is here.".into(),
                then: Box::new(Script::Directory),
            };
        }
        if answers.fetch_add(1, Ordering::SeqCst) == 0 {
            Script::Unavailable
        } else {
            Script::Say("Two of us.".into())
        }
    })
    .await;
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "who else is here?").unwrap();
    h.settle(run).await;

    let opened: Vec<_> = h
        .sink
        .snapshot()
        .into_iter()
        .filter_map(|e| match e {
            UiEvent::StreamStarted { message_id, .. } => Some(message_id),
            _ => None,
        })
        .collect();
    assert_eq!(opened.len(), 2, "the failed attempt should have been replaced, not appended to");

    assert_eq!(
        h.sink.streamed_text(*opened.last().unwrap()),
        "Two of us.",
        "the replacement bubble held nothing to be separated from"
    );
    assert_eq!(h.channel_texts("Manager").pop().unwrap(), "Checking who is here.\n\nTwo of us.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_model_that_thinks_out_loud_is_watched_without_being_recorded() {
    let stub = serve(|_| Script::Thinking {
        about: "They want the total, so 17 times 23.".into(),
        say: "391.".into(),
    })
    .await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "what is 17 times 23").unwrap();
    h.settle(run).await;

    let stream_id = h
        .sink
        .snapshot()
        .into_iter()
        .find_map(|e| match e {
            UiEvent::StreamStarted { message_id, .. } => Some(message_id),
            _ => None,
        })
        .expect("a stream should have started");

    assert_eq!(
        h.sink.streamed_reasoning(stream_id),
        "They want the total, so 17 times 23.",
        "the operator should have seen it working"
    );
    assert_eq!(h.sink.streamed_text(stream_id), "391.");

    let persisted = h.channel_texts("Manager").pop().unwrap();
    assert_eq!(persisted, "391.");
    assert!(!persisted.contains("17 times 23"), "reasoning reached the transcript: {persisted}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_says_what_it_reached_for_before_it_knows_how_it_went() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Two of us.".into())
        } else {
            Script::Directory
        }
    })
    .await;
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "who else is here?").unwrap();
    h.settle(run).await;

    let events = h.sink.snapshot();
    let stream_id = events
        .iter()
        .find_map(|e| match e {
            UiEvent::StreamStarted { message_id, .. } => Some(*message_id),
            _ => None,
        })
        .expect("a stream should have started");

    let started = events
        .iter()
        .position(|e| {
            matches!(e, UiEvent::ToolStarted { message_id, name, .. }
                if *message_id == stream_id && name == "directory")
        })
        .expect("the call should have been reported as it was made");
    let finished = events
        .iter()
        .position(
            |e| matches!(e, UiEvent::ToolFinished { message_id, .. } if *message_id == stream_id),
        )
        .expect("and again when it came back");
    assert!(started < finished, "the call was reported finished before it was reported started");

    let (
        UiEvent::ToolStarted { call_id: opened, .. },
        UiEvent::ToolFinished { call_id: closed, part: watched, .. },
    ) = (&events[started], &events[finished])
    else {
        unreachable!("both were matched above")
    };
    assert_eq!(opened, closed);

    let recorded: Vec<_> = h
        .runtime
        .store()
        .channel_messages(h.id("Manager"), 200)
        .unwrap()
        .iter()
        .flat_map(|e| e.parts.clone())
        .filter(|part| matches!(part, Part::ToolCall { name, .. } if name == "directory"))
        .collect();
    assert_eq!(
        recorded.as_slice(),
        std::slice::from_ref(watched),
        "the record must agree with what was drawn"
    );

    assert_eq!(
        events.iter().filter(|e| matches!(e, UiEvent::ToolFinished { .. })).count(),
        recorded.len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_thought_is_coalesced_on_the_same_clock_as_the_text() {
    let thinking = "weighing the options carefully. ".repeat(40);
    let expected = thinking.clone();
    let stub =
        serve(move |_| Script::Thinking { about: thinking.clone(), say: "Yes.".into() }).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "decide").unwrap();
    h.settle(run).await;

    let deltas = h.sink.count_of(|e| matches!(e, UiEvent::ReasoningDelta { .. }));
    assert!(
        deltas <= 12,
        "{deltas} events for reasoning the provider sent in {} pieces",
        expected.len().div_ceil(9)
    );

    let stream_id = h
        .sink
        .snapshot()
        .into_iter()
        .find_map(|e| match e {
            UiEvent::StreamStarted { message_id, .. } => Some(message_id),
            _ => None,
        })
        .expect("a stream should have started");
    assert_eq!(h.sink.streamed_reasoning(stream_id), expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_that_thinks_twice_is_watched_with_the_break_between_its_rounds() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Thinking { about: "**Answering**".into(), say: "Two of us.".into() }
        } else {
            Script::Mull {
                about: "**Checking who is here**".into(),
                then: Box::new(Script::Directory),
            }
        }
    })
    .await;
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "who else is here?").unwrap();
    h.settle(run).await;

    let stream_id = h
        .sink
        .snapshot()
        .into_iter()
        .find_map(|e| match e {
            UiEvent::StreamStarted { message_id, .. } => Some(message_id),
            _ => None,
        })
        .expect("a stream should have started");
    assert_eq!(h.sink.streamed_reasoning(stream_id), "**Checking who is here**\n\n**Answering**");
    assert_eq!(h.channel_texts("Manager").pop().unwrap(), "Two of us.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_retried_round_opens_its_thinking_at_the_left_margin() {
    let answers = Arc::new(AtomicUsize::new(0));
    let stub = serve(move |body| {
        if !has_tool_result(body) {
            return Script::Mull {
                about: "**Checking who is here**".into(),
                then: Box::new(Script::Directory),
            };
        }
        if answers.fetch_add(1, Ordering::SeqCst) == 0 {
            Script::Unavailable
        } else {
            Script::Thinking { about: "**Answering**".into(), say: "Two of us.".into() }
        }
    })
    .await;
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "who else is here?").unwrap();
    h.settle(run).await;

    let opened: Vec<_> = h
        .sink
        .snapshot()
        .into_iter()
        .filter_map(|e| match e {
            UiEvent::StreamStarted { message_id, .. } => Some(message_id),
            _ => None,
        })
        .collect();
    assert_eq!(opened.len(), 2, "the failed attempt should have been replaced, not appended to");
    assert_eq!(h.sink.streamed_reasoning(opened[1]), "**Answering**");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_long_reply_reaches_the_window_in_far_fewer_events_than_tokens() {
    let reply = "the quick brown fox jumps over the lazy dog.".repeat(40);
    let expected = reply.clone();
    let stub = serve(move |_| Script::Say(reply.clone())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "say a lot").unwrap();
    h.settle(run).await;

    let deltas = h.sink.count_of(|e| matches!(e, UiEvent::StreamDelta { .. }));
    assert!(
        deltas <= 12,
        "{deltas} events for a reply the provider sent in {} pieces",
        expected.len().div_ceil(7)
    );

    let stream_id = h
        .sink
        .snapshot()
        .into_iter()
        .find_map(|e| match e {
            UiEvent::StreamStarted { message_id, .. } => Some(message_id),
            _ => None,
        })
        .expect("a stream should have started");
    assert_eq!(h.sink.streamed_text(stream_id), expected);
    assert_eq!(h.channel_texts("Manager").pop().unwrap(), expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agents_run_concurrently_rather_than_one_after_another() {
    let peers = ["Chef", "Host", "Barista", "Sommelier"];
    let stub = serve(move |body| {
        let who = speaker(body);
        if who == "Manager" {
            if has_tool_result(body) {
                Script::Say("done".into())
            } else {
                Script::SendTo {
                    recipients: peers.iter().map(|s| s.to_string()).collect(),
                    text: "go".into(),
                }
            }
        } else {
            std::thread::sleep(Duration::from_millis(300));
            Script::Say(format!("{who} finished."))
        }
    })
    .await;

    let h = harness(
        &stub,
        &["Manager", "Chef", "Host", "Barista", "Sommelier"],
        GuardLimits::default(),
    );
    let started = Instant::now();
    let run = h.runtime.send_from_human(h.id("Manager"), "go").unwrap();
    h.settle(run).await;
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(900),
        "four 300ms peers took {elapsed:?}; they are not running concurrently"
    );
}

#[tokio::test]
async fn a_skill_one_agent_writes_is_offered_to_its_crew_on_the_next_turn() {
    let stub = serve(|body| {
        let who = speaker(body);
        if who == "Writer" {
            if has_tool_result(body) {
                Script::Say("Written down.".into())
            } else {
                Script::Skill(serde_json::json!({
                    "action": "write",
                    "name": "release-notes",
                    "description": "When writing release notes",
                    "content": "# Release notes\n\nLead with what changed for the reader.",
                }))
            }
        } else if has_tool_result(body) {
            Script::Say("Read it.".into())
        } else {
            Script::Skill(serde_json::json!({ "action": "view", "name": "release-notes" }))
        }
    })
    .await;

    let h = harness(&stub, &["Writer", "Reader"], GuardLimits::default());
    let run =
        h.runtime.send_from_human(h.id("Writer"), "Write down how we do release notes.").unwrap();
    h.settle(run).await;
    let run = h.runtime.send_from_human(h.id("Reader"), "Draft the notes.").unwrap();
    h.settle(run).await;

    let prompts = prompts_by_agent(&stub);
    let reader = prompts.get("Reader").expect("the reader was prompted");
    assert!(
        reader.contains("- release-notes (your group): When writing release notes"),
        "the crewmate is not offered the skill: {reader}"
    );
    assert!(reader.contains("- guaca (Guaca):"), "Guaca's own is always offered: {reader}");
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("Wrote `release-notes` for your group"), "{results}");
    assert!(results.contains("Lead with what changed for the reader."), "{results}");
}

#[tokio::test]
async fn an_agent_cannot_write_guacas_skill_or_delete_the_operators() {
    let stub = serve(|body| {
        let who = speaker(body);
        if has_tool_result(body) {
            Script::Say("Understood.".into())
        } else if who == "Writer" {
            Script::Skill(serde_json::json!({
                "action": "write", "name": "guaca", "description": "Mine now", "content": "x",
            }))
        } else {
            Script::Skill(serde_json::json!({ "action": "delete", "name": "house-style" }))
        }
    })
    .await;

    let h = harness(&stub, &["Writer", "Deleter"], GuardLimits::default());
    let house =
        guac_lib::domain::skill::Clean::new("house-style", "When writing", "Plain.").unwrap();
    h.runtime.skills().write(guac_lib::domain::skill::Scope::Workspace, &house).unwrap();
    for name in ["Writer", "Deleter"] {
        let run = h.runtime.send_from_human(h.id(name), "Go.").unwrap();
        h.settle(run).await;
    }

    let results = tool_results(&stub).join("\n");
    assert!(results.contains("`guaca` is Guaca's own skill and is read-only"), "{results}");
    assert!(results.contains("`house-style` belongs to the operator"), "{results}");
    assert!(h.runtime.skills().read(None, "house-style").is_ok(), "the operator's skill survives");
}

#[tokio::test]
async fn a_skill_names_the_files_it_carries_and_an_agent_reads_only_those() {
    let stub = serve(|body| {
        let who = speaker(body);
        if has_tool_result(body) {
            Script::Say("Done.".into())
        } else if who == "Reader" {
            Script::Skill(serde_json::json!({ "action": "view", "name": "pdf" }))
        } else if who == "Filer" {
            Script::Skill(
                serde_json::json!({ "action": "view", "name": "pdf", "file": "forms.md" }),
            )
        } else {
            Script::Skill(
                serde_json::json!({ "action": "view", "name": "pdf", "file": "../../guac.db" }),
            )
        }
    })
    .await;

    let h = harness(&stub, &["Reader", "Filer", "Climber"], GuardLimits::default());
    let package = guac_lib::domain::skill::Package::new(
        "pdf",
        vec![
            (
                "SKILL.md".into(),
                "---\nname: pdf\ndescription: When a PDF is involved\n---\nFor forms, read forms.md."
                    .into(),
            ),
            ("forms.md".into(), "Check for fillable fields first. THE-FORM-DETAIL".into()),
        ],
    )
    .unwrap();
    h.runtime.skills().install(guac_lib::domain::skill::Scope::Workspace, &package, None).unwrap();
    for name in ["Reader", "Filer", "Climber"] {
        let run = h.runtime.send_from_human(h.id(name), "A PDF arrived.").unwrap();
        h.settle(run).await;
    }

    let results = tool_results(&stub).join("\n");
    assert!(results.contains("For forms, read forms.md."), "{results}");
    assert!(results.contains("Files beside it, read with `view` and `file`"), "{results}");
    assert!(results.contains("THE-FORM-DETAIL"), "the named file is read: {results}");
    assert!(results.contains("is not a file beside a skill"), "the climb is refused: {results}");
    assert_eq!(
        results.matches("THE-FORM-DETAIL").count(),
        1,
        "a file is read only when asked for: {results}"
    );
}

#[tokio::test]
async fn a_file_an_agent_keeps_is_named_on_its_next_turn_and_read_only_when_asked() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Kept.".into())
        } else if anyone_said(body, "Track the lead") {
            Script::Notebook(serde_json::json!({
                "action": "write",
                "path": "leads/acme",
                "content": "# Acme\nWants a demo in October. THE-PRIVATE-DETAIL",
            }))
        } else {
            Script::Say("Nothing to do.".into())
        }
    })
    .await;
    let h = harness(&stub, &["Seller", "Peer"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Seller"), "Track the lead from Acme.").unwrap();
    h.settle(run).await;
    let run = h.runtime.send_from_human(h.id("Seller"), "Anything new?").unwrap();
    h.settle(run).await;

    let prompts = prompts_by_agent(&stub);
    let seller = prompts.get("Seller").expect("the seller was prompted");
    assert!(seller.contains("## Your notebook"), "{seller}");
    assert!(seller.contains("- leads/acme.md ("), "{seller}");
    assert!(!seller.contains("THE-PRIVATE-DETAIL"), "a file is named, never read out: {seller}");
    assert_eq!(h.runtime.notebooks().list(h.id("Peer")), vec![], "the folder is the agent's own");
    assert!(h
        .runtime
        .notebooks()
        .read(h.id("Seller"), "leads/acme.md")
        .unwrap()
        .contains("October"));
}

#[tokio::test]
async fn an_agent_reads_the_settings_and_what_the_operator_is_looking_at_and_never_a_key() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("That pane holds the limits.".into())
        } else {
            Script::Settings(serde_json::json!({ "action": "read" }))
        }
    })
    .await;
    let h = harness(&stub, &["Helper"], GuardLimits::default());
    h.runtime.report_view(guac_lib::domain::view::OperatorView {
        agent_id: Some(h.id("Helper")),
        overlay: Some(guac_lib::domain::view::Overlay::Settings),
        section: Some("limits".into()),
        group_id: None,
    });
    let run = h.runtime.send_from_human(h.id("Helper"), "What is this pane?").unwrap();
    h.settle(run).await;

    let results = tool_results(&stub).join("\n");
    assert!(
        results.contains(
            "The operator is looking at the channel with Helper, with Settings open on Limits"
        ),
        "{results}"
    );
    assert!(results.contains("\"maxHops\""), "{results}");
    let key = h.runtime.config().inference.api_key;
    assert!(
        !key.is_empty() && !results.contains(key.expose()),
        "a key reached the model: {results}"
    );
}

#[tokio::test]
async fn a_settings_change_happens_only_when_the_operator_allows_it() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Asked.".into())
        } else {
            Script::Settings(serde_json::json!({
                "action": "update", "changes": { "limits": { "maxHops": 12 } },
            }))
        }
    })
    .await;
    let h = harness(&stub, &["Helper"], GuardLimits::default());
    let before = h.runtime.config().limits;

    let run = h.runtime.send_from_human(h.id("Helper"), "Let the crew relay further.").unwrap();
    let request = h.awaited_request().await;
    assert_eq!(h.runtime.config().limits.max_hops, before.max_hops, "nothing moves while asked");
    let asked = h.runtime.store().get_approval(request).unwrap().unwrap();
    assert!(
        asked.detail.iter().any(|field| field.label == "Relay depth"
            && field.value == format!("{} → 12", before.max_hops)),
        "the operator sees the before and after: {:?}",
        asked.detail
    );

    h.runtime.decide_approval(request, Decision::AlwaysAllow).unwrap();
    h.wait_until("the change lands", |h| h.runtime.config().limits.max_hops == 12).await;
    assert_eq!(h.runtime.config().limits.max_steps_per_run, before.max_steps_per_run);
    assert!(!h
        .runtime
        .store()
        .has_standing_grant(h.id("Helper"), ProtectedAction::ChangeSettings)
        .unwrap());
    h.settle(run).await;
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("The operator allowed it: changed Relay depth"), "{results}");
}

#[tokio::test]
async fn an_agent_is_refused_the_endpoint_before_the_operator_is_bothered() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("I cannot.".into())
        } else {
            Script::Settings(serde_json::json!({
                "action": "update", "changes": { "baseUrl": "https://attacker.example/v1" },
            }))
        }
    })
    .await;
    let h = harness(&stub, &["Helper"], GuardLimits::default());
    let endpoint = h.runtime.config().inference.base_url;
    let run = h.runtime.send_from_human(h.id("Helper"), "Switch our endpoint.").unwrap();
    h.settle(run).await;

    assert_eq!(h.runtime.config().inference.base_url, endpoint);
    assert!(h.runtime.store().pending_approvals(10).unwrap().is_empty(), "nobody was asked");
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("`baseUrl` is the operator's"), "{results}");
}

fn pinnable(
    h: &Harness,
    owner: &str,
    condensed: Option<&str>,
) -> guac_lib::domain::artifact::Artifact {
    let crew = h.runtime.store().get_agent(h.id(owner)).unwrap().unwrap().group_id;
    h.runtime
        .store()
        .create_artifact(
            crew,
            h.id(owner),
            owner,
            guac_lib::db::ArtifactDraft {
                title: "Open PRs",
                page: "<p>the board</p>",
                sources: &[guac_lib::domain::artifact::Source {
                    name: "prs".into(),
                    tool: "github__list_pull_requests".into(),
                    arguments: serde_json::json!({ "state": "open" }),
                }],
                condensed,
                note: "",
            },
        )
        .unwrap()
}

async fn pinning(width: &'static str, every: u32) -> Stub {
    serve(move |body| {
        let asked = last_word(body);
        if has_tool_result(body) || !asked.contains('-') {
            Script::Say("Asked.".into())
        } else {
            Script::Plugin {
                name: "artifact".into(),
                arguments: serde_json::json!({
                    "action": "pin", "id": asked, "width": width, "every_minutes": every,
                }),
            }
        }
    })
    .await
}

#[tokio::test]
async fn a_page_reaches_the_status_bar_only_once_the_operator_says_yes_to_it_and_its_reads() {
    let stub = pinning("wide", 10).await;
    let h = harness(&stub, &["Rae"], GuardLimits::default());
    let page = pinnable(&h, "Rae", Some("<b>3 open</b>"));

    h.runtime.send_from_human(h.id("Rae"), &format!("Put it on my bar, id {}", page.id)).unwrap();
    let request = h.awaited_request().await;
    assert!(h.runtime.config().widgets.is_empty(), "nothing is pinned while asked");
    let asked = h.runtime.store().get_approval(request).unwrap().unwrap();
    assert!(asked.summary.contains("\"Open PRs\" on your status bar"), "{}", asked.summary);
    let reads = asked
        .detail
        .iter()
        .find(|field| field.label == "Reads, allowed with this")
        .unwrap_or_else(|| panic!("the card shows the reads: {:?}", asked.detail));
    assert!(
        reads.value.contains("github__list_pull_requests {\"state\":\"open\"}"),
        "{}",
        reads.value
    );
    assert!(reads.value.contains("every 10 min, as Rae"), "{}", reads.value);

    h.runtime.decide_approval(request, Decision::Allow).unwrap();
    h.wait_until("the page lands", |h| !h.runtime.config().widgets.is_empty()).await;
    let pinned = &h.runtime.config().widgets[0];
    assert_eq!(pinned.artifact_id, page.id);
    assert_eq!(pinned.width, guac_lib::domain::widget::Width::Wide);
    assert_eq!((pinned.every_minutes, pinned.added_by.as_str()), (10, "Rae"));
    assert!(
        h.runtime.store().any_artifact(page.id).unwrap().unwrap().sources_allowed,
        "the yes allowed the reads it showed"
    );

    let run = h.runtime.send_from_human(h.id("Rae"), "Anything new?").unwrap();
    h.settle(run).await;
    let prompt = prompts_by_agent(&stub).remove("Rae").unwrap_or_default();
    assert!(prompt.contains("on the operator's status bar"), "{prompt}");
}

#[tokio::test]
async fn a_page_without_a_condensed_view_is_refused_before_anybody_is_asked() {
    let stub = pinning("narrow", 5).await;
    let h = harness(&stub, &["Rae"], GuardLimits::default());
    let page = pinnable(&h, "Rae", None);

    let run =
        h.runtime.send_from_human(h.id("Rae"), &format!("Pin the board, id {}", page.id)).unwrap();
    h.settle(run).await;

    assert!(h.runtime.store().pending_approvals(10).unwrap().is_empty(), "nobody was asked");
    assert!(h.runtime.config().widgets.is_empty());
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("has no condensed view"), "the refusal says what to add: {results}");
    assert!(results.contains("`condensed`"), "{results}");
}

#[tokio::test]
async fn an_agent_cannot_pin_another_crews_page() {
    let stub = pinning("narrow", 5).await;
    let h = harness_in_groups(
        &stub,
        &[("Manager", Some("Front")), ("Chef", Some("Back"))],
        GuardLimits::default(),
    );
    let theirs = pinnable(&h, "Chef", Some("<b>2</b>"));

    let run = h
        .runtime
        .send_from_human(h.id("Manager"), &format!("Pin the menu, id {}", theirs.id))
        .unwrap();
    h.settle(run).await;

    assert!(h.runtime.store().pending_approvals(10).unwrap().is_empty(), "nobody was asked");
    assert!(h.runtime.config().widgets.is_empty());
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("no artifact with the id"), "{results}");
}

#[tokio::test]
async fn a_deleted_page_leaves_the_status_bar_with_it() {
    let stub = serve(|_| Script::Say("Done.".into())).await;
    let h = harness(&stub, &["Rae"], GuardLimits::default());
    let (gone, kept) =
        (pinnable(&h, "Rae", Some("<b>1</b>")), pinnable(&h, "Rae", Some("<b>2</b>")));
    h.runtime
        .change_config(|config| {
            for id in [gone.id, kept.id] {
                let pin = guac_lib::domain::widget::Widget::new(
                    id,
                    guac_lib::domain::widget::Width::Narrow,
                    None,
                    "the operator",
                )?;
                guac_lib::domain::widget::pin(&mut config.widgets, pin)?;
            }
            Ok::<_, guac_lib::config::ConfigError>(())
        })
        .unwrap();

    h.runtime.store().delete_artifact(gone.id).unwrap();
    h.runtime.unpin_missing();

    let left: Vec<_> = h.runtime.config().widgets.iter().map(|w| w.artifact_id).collect();
    assert_eq!(left, vec![kept.id], "a pin never outlives its page, and the others stay put");
}

#[tokio::test]
async fn an_agent_writes_its_crews_calendar_and_is_told_back_the_date_it_stored() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("On the calendar.".into())
        } else {
            Script::Note { title: "Board call".into(), starts_at: "2099-09-14 15:00".into() }
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Board call on the 14th at 3.").unwrap();
    h.settle(run).await;

    let results = tool_results(&stub).join("\n");
    assert!(results.contains("Board call"), "got {results:?}");
    assert!(results.contains("3:00 PM"), "the stored wall clock has to come back: {results:?}");
    assert!(results.contains("Mon 14 Sep 2099"), "with the day on it: {results:?}");

    let stored = h.runtime.store().occasions(0, i64::MAX, None, 50).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].title, "Board call");
    assert_eq!(stored[0].agent_id, Some(h.id("Manager")), "who wrote it is kept");
}

#[tokio::test]
async fn a_date_a_model_invented_is_refused_with_the_two_shapes_that_work() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Fixing that.".into())
        } else {
            Script::Note { title: "Board call".into(), starts_at: "next tuesday".into() }
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Put the call on next Tuesday.").unwrap();
    h.settle(run).await;

    let results = tool_results(&stub).join("\n");
    assert!(results.contains("Refused"), "got {results:?}");
    assert!(results.contains("2026-09-14 15:00"), "with a working time: {results:?}");
    assert!(
        h.runtime.store().occasions(0, i64::MAX, None, 50).unwrap().is_empty(),
        "nothing was written"
    );
}

#[tokio::test]
async fn an_agent_cannot_move_another_crews_meeting() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Understood.".into())
        } else {
            let asked = body["messages"]
                .as_array()
                .and_then(|turns| turns.last())
                .and_then(|turn| turn["content"].as_str())
                .unwrap_or("")
                .to_string();
            let id = asked.rsplit(' ').next().unwrap_or("").trim_end_matches('.').to_string();
            Script::Move { action: "update".into(), id, starts_at: "2099-12-25 09:00".into() }
        }
    })
    .await;

    let h = harness_in_groups(
        &stub,
        &[("Manager", Some("Front")), ("Chef", Some("Back"))],
        GuardLimits::default(),
    );

    let theirs = h.runtime.store().get_agent(h.id("Chef")).unwrap().unwrap().group_id;
    let when = guac_lib::domain::occasion::parse_when("2099-11-02 10:00").unwrap();
    let clean =
        guac_lib::domain::occasion::Clean::new(theirs, None, "Deposition", "", "", when, Some(60))
            .unwrap();
    let hidden = h.runtime.store().create_occasion(&clean).unwrap();

    let run = h
        .runtime
        .send_from_human(h.id("Manager"), &format!("Move the deposition, id {}", hidden.id))
        .unwrap();
    h.settle(run).await;

    let results = tool_results(&stub).join("\n");
    assert!(
        results.contains("no occasion with the id"),
        "an occasion in another crew must be indistinguishable from one that never existed, \
         otherwise the refusal itself says whose it is, got {results:?}"
    );

    let untouched = h.runtime.store().any_occasion(hidden.id).unwrap().unwrap();
    assert_eq!(untouched.starts_at, hidden.starts_at, "and it must not have moved");
}

fn last_word(body: &serde_json::Value) -> String {
    body["messages"]
        .as_array()
        .and_then(|turns| turns.last())
        .and_then(|turn| turn["content"].as_str())
        .unwrap_or("")
        .rsplit(' ')
        .next()
        .unwrap_or("")
        .trim_end_matches('.')
        .to_string()
}

fn artifact_calls(
    h: &Harness,
    name: &str,
) -> Vec<(ToolOutcome, Option<guac_lib::domain::artifact::Made>)> {
    h.runtime
        .store()
        .channel_messages(h.id(name), 200)
        .unwrap()
        .into_iter()
        .flat_map(|message| message.parts)
        .filter_map(|part| match part {
            Part::ToolCall { name, outcome, artifact, .. } if name == "artifact" => {
                Some((outcome, artifact))
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn an_agent_keeps_a_page_and_its_turn_records_a_card_for_it() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("The board is in your Artifacts.".into())
        } else {
            Script::Plugin {
                name: "artifact".into(),
                arguments: serde_json::json!({
                    "action": "create",
                    "title": "Pipeline by stage",
                    "page": "<!doctype html><p>Pipeline</p>",
                }),
            }
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Make me a pipeline board.").unwrap();
    h.settle(run).await;

    let kept = h.runtime.store().artifacts(None, 10).unwrap();
    assert_eq!(kept.len(), 1, "one page kept");
    assert_eq!(kept[0].owner.as_ref().map(|o| o.id), Some(h.id("Manager")), "its maker owns it");

    let results = tool_results(&stub).join("\n");
    assert!(results.contains("You own it"), "the maker is told it owns it: {results:?}");

    let calls = artifact_calls(&h, "Manager");
    let made = calls.iter().find_map(|(_, made)| made.clone());
    let made = made.expect("the call records what it made, which is what the card is drawn from");
    assert_eq!((made.id, made.version), (kept[0].id, 1));
    assert_eq!(made.title, "Pipeline by stage");
}

#[tokio::test]
async fn a_page_made_with_a_condensed_view_is_told_it_can_be_pinned() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Made it.".into())
        } else {
            Script::Plugin {
                name: "artifact".into(),
                arguments: serde_json::json!({
                    "action": "create",
                    "title": "Open PRs",
                    "page": "<!doctype html><p>the board</p>",
                    "condensed": "<!doctype html><b>3 open</b>",
                }),
            }
        }
    })
    .await;
    let h = harness(&stub, &["Rae"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Rae"), "Track my PRs.").unwrap();
    h.settle(run).await;

    let kept = &h.runtime.store().artifacts(None, 10).unwrap()[0];
    assert!(kept.condensed);
    assert_eq!(
        h.runtime.store().artifact_condensed(kept.id).unwrap().as_deref(),
        Some("<!doctype html><b>3 open</b>")
    );
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("`pin` asks them"), "{results}");
    assert!(h.runtime.config().widgets.is_empty(), "made is not pinned: that is the operator's");
}

#[tokio::test]
async fn viewing_a_page_shows_its_condensed_view_so_an_edit_to_it_is_not_blind() {
    let stub = serve(|body| {
        let asked = last_word(body);
        if has_tool_result(body) || !asked.contains('-') {
            Script::Say("Read it.".into())
        } else {
            Script::Plugin {
                name: "artifact".into(),
                arguments: serde_json::json!({ "action": "view", "id": asked }),
            }
        }
    })
    .await;
    let h = harness(&stub, &["Rae"], GuardLimits::default());
    let page = pinnable(&h, "Rae", Some("<b>3 open</b>"));
    let run = h.runtime.send_from_human(h.id("Rae"), &format!("Look at {}", page.id)).unwrap();
    h.settle(run).await;

    let results = tool_results(&stub).join("\n");
    assert!(results.contains("Its condensed view, not on the status bar"), "{results}");
    assert!(results.contains("<b>3 open</b>"), "{results}");
}

#[tokio::test]
async fn a_crewmate_edits_without_messaging_the_owner_who_reads_it_on_its_next_turn() {
    let stub = serve(|body| {
        let asked = last_word(body);
        if has_tool_result(body) || !asked.contains('-') {
            Script::Say("Done.".into())
        } else {
            Script::Plugin {
                name: "artifact".into(),
                arguments: serde_json::json!({
                    "action": "update",
                    "id": asked,
                    "page": "<p>with Q4</p>",
                    "note": "Added a Q4 column",
                }),
            }
        }
    })
    .await;

    let h = harness(&stub, &["Rae", "Milo"], GuardLimits::default());
    let crew = h.runtime.store().get_agent(h.id("Rae")).unwrap().unwrap().group_id;
    let page = h
        .runtime
        .store()
        .create_artifact(
            crew,
            h.id("Rae"),
            "Rae",
            guac_lib::db::ArtifactDraft {
                title: "Pipeline",
                page: "<p>v1</p>",
                sources: &[],
                condensed: None,
                note: "First cut",
            },
        )
        .unwrap();

    let run = h
        .runtime
        .send_from_human(h.id("Milo"), &format!("Add Q4 to the pipeline, id {}", page.id))
        .unwrap();
    h.settle(run).await;

    let after = h.runtime.store().artifact(page.id, crew).unwrap().unwrap();
    assert_eq!(after.version, 2);
    assert_eq!(after.owner.map(|o| o.id), Some(h.id("Rae")), "an edit is not a take-over");
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("no need to message it"), "got {results:?}");
    assert!(
        h.runtime.store().channel_messages(h.id("Rae"), 50).unwrap().is_empty(),
        "nothing reached the owner's channel"
    );

    let run = h.runtime.send_from_human(h.id("Rae"), "Anything new?").unwrap();
    h.settle(run).await;
    let prompt = prompts_by_agent(&stub).remove("Rae").unwrap_or_default();
    assert!(prompt.contains("\"Pipeline\" v2, yours, updated"), "got {prompt}");
    assert!(prompt.contains("by Milo"), "the owner reads who changed it: {prompt}");
}

#[tokio::test]
async fn an_edit_without_a_note_is_refused_and_changes_nothing() {
    let stub = serve(|body| {
        let asked = last_word(body);
        if has_tool_result(body) {
            Script::Say("I will say what changed.".into())
        } else {
            Script::Plugin {
                name: "artifact".into(),
                arguments: serde_json::json!({ "action": "update", "id": asked, "page": "<p/>" }),
            }
        }
    })
    .await;

    let h = harness(&stub, &["Rae"], GuardLimits::default());
    let crew = h.runtime.store().get_agent(h.id("Rae")).unwrap().unwrap().group_id;
    let page = h
        .runtime
        .store()
        .create_artifact(
            crew,
            h.id("Rae"),
            "Rae",
            guac_lib::db::ArtifactDraft {
                title: "Pipeline",
                page: "<p>v1</p>",
                sources: &[],
                condensed: None,
                note: "",
            },
        )
        .unwrap();

    let run = h.runtime.send_from_human(h.id("Rae"), &format!("Tidy it, id {}", page.id)).unwrap();
    h.settle(run).await;

    let results = tool_results(&stub).join("\n");
    assert!(results.contains("needs a `note`"), "the refusal says what to send: {results:?}");
    assert_eq!(h.runtime.store().artifact(page.id, crew).unwrap().unwrap().version, 1);
    assert!(
        matches!(artifact_calls(&h, "Rae")[..], [(ToolOutcome::Refused { .. }, None)]),
        "drawn as a refusal, and with no card: {:?}",
        artifact_calls(&h, "Rae")
    );
}

#[tokio::test]
async fn an_agent_cannot_touch_another_crews_artifact() {
    let stub = serve(|body| {
        let asked = last_word(body);
        if has_tool_result(body) {
            Script::Say("Understood.".into())
        } else {
            Script::Plugin {
                name: "artifact".into(),
                arguments: serde_json::json!({
                    "action": "update",
                    "id": asked,
                    "page": "<p>mine now</p>",
                    "note": "Took it",
                }),
            }
        }
    })
    .await;

    let h = harness_in_groups(
        &stub,
        &[("Manager", Some("Front")), ("Chef", Some("Back"))],
        GuardLimits::default(),
    );
    let theirs = h.runtime.store().get_agent(h.id("Chef")).unwrap().unwrap().group_id;
    let hidden = h
        .runtime
        .store()
        .create_artifact(
            theirs,
            h.id("Chef"),
            "Chef",
            guac_lib::db::ArtifactDraft {
                title: "Menu",
                page: "<p>menu</p>",
                sources: &[],
                condensed: None,
                note: "",
            },
        )
        .unwrap();

    let run = h
        .runtime
        .send_from_human(h.id("Manager"), &format!("Edit the menu, id {}", hidden.id))
        .unwrap();
    h.settle(run).await;

    let results = tool_results(&stub).join("\n");
    assert!(results.contains("no artifact with the id"), "got {results:?}");
    let untouched = h.runtime.store().any_artifact(hidden.id).unwrap().unwrap();
    assert_eq!(untouched.version, 1, "and it must not have changed");
}

#[tokio::test]
async fn an_agent_reads_what_its_own_crew_already_has() {
    let stub = serve(|_| Script::Say("Nothing else that week.".into())).await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let mine = h.runtime.store().get_agent(h.id("Manager")).unwrap().unwrap().group_id;
    let soon = now_ms() + 2 * 86_400_000;
    let clean = guac_lib::domain::occasion::Clean::new(
        mine,
        None,
        "Board call",
        "",
        "",
        guac_lib::domain::occasion::When { starts_at: soon, all_day: false },
        Some(60),
    )
    .unwrap();
    h.runtime.store().create_occasion(&clean).unwrap();

    let run = h.runtime.send_from_human(h.id("Manager"), "What is on this week?").unwrap();
    h.settle(run).await;

    let prompt = prompts_by_agent(&stub).remove("Manager").unwrap_or_default();
    assert!(prompt.contains("Board call"), "got {prompt}");
    assert!(prompt.contains("in 2 days"), "with how far off it is: {prompt}");
}

#[tokio::test]
async fn an_agent_cannot_message_a_peer_in_another_group() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Understood.".into())
        } else {
            Script::SendTo { recipients: vec!["Chef".into()], text: "hello".into() }
        }
    })
    .await;

    let h = harness_in_groups(
        &stub,
        &[("Manager", Some("Front")), ("Chef", Some("Back"))],
        GuardLimits::default(),
    );
    let run = h.runtime.send_from_human(h.id("Manager"), "Message Chef.").unwrap();
    h.settle(run).await;

    let results = tool_results(&stub);
    assert!(
        results.iter().any(|r| r.contains("no agent named Chef")),
        "a peer in another group must be indistinguishable from one that never existed, \
         otherwise the refusal itself leaks the roster across the boundary, got {results:?}"
    );
    assert!(h.feed().is_empty(), "nothing may cross a group boundary");
    assert!(
        h.channel_texts("Chef").is_empty(),
        "the recipient's channel must not have received the message"
    );
}

#[tokio::test]
async fn agents_in_the_same_group_still_reach_each_other() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Sent.".into())
        } else {
            Script::SendTo { recipients: vec!["Chef".into()], text: "service".into() }
        }
    })
    .await;

    let h = harness_in_groups(
        &stub,
        &[("Manager", Some("Front")), ("Chef", Some("Front"))],
        GuardLimits::default(),
    );
    let run = h.runtime.send_from_human(h.id("Manager"), "Message Chef.").unwrap();
    h.settle(run).await;

    assert!(
        h.channel_texts("Chef").iter().any(|t| t.contains("service")),
        "an agent must still reach a peer inside its own group, got {:?}",
        h.channel_texts("Chef")
    );
}

#[tokio::test]
async fn the_directory_never_lists_agents_from_another_group() {
    let stub =
        serve(
            |body| {
                if has_tool_result(body) {
                    Script::Say("Noted.".into())
                } else {
                    Script::Directory
                }
            },
        )
        .await;

    let h = harness_in_groups(
        &stub,
        &[("Manager", Some("Front")), ("Sous", Some("Front")), ("Chef", Some("Back"))],
        GuardLimits::default(),
    );
    let run = h.runtime.send_from_human(h.id("Manager"), "Who else is here?").unwrap();
    h.settle(run).await;

    let joined = tool_results(&stub).join("\n");
    assert!(joined.contains("Sous"), "a peer in the same group must be listed, got {joined:?}");
    assert!(
        !joined.contains("Chef"),
        "an agent in another group must not appear in the directory, got {joined:?}"
    );
}

#[tokio::test]
async fn a_reply_sent_through_the_tool_does_not_demand_another() {
    let stub = serve(|body| {
        let last = body["messages"]
            .as_array()
            .and_then(|m| m.last())
            .and_then(|m| m["content"].as_str())
            .unwrap_or_default()
            .to_string();

        if has_tool_result(body) {
            Script::Say("noted".into())
        } else if last.contains("hello from manager") {
            Script::SendTo { recipients: vec!["Manager".into()], text: "hello back".into() }
        } else if last.contains("hello back") {
            Script::Say("ok".into())
        } else {
            Script::SendTo { recipients: vec!["Chef".into()], text: "hello from manager".into() }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Introduce yourself to Chef.").unwrap();
    h.settle(run).await;

    let feed = h.feed();
    let deepest = feed.iter().map(|e| e.hop).max().unwrap_or(0);
    assert_eq!(
        feed.len(),
        2,
        "one approach and one answer is the whole exchange; anything more means the \
         answer demanded an answer. Got {:?}",
        feed.iter().map(|e| (e.hop, e.expects_reply)).collect::<Vec<_>>()
    );
    assert_eq!(deepest, 2, "the exchange must not travel past the answer");

    let answer = feed.iter().find(|e| e.hop == 2).expect("the answer must have been delivered");
    assert!(
        !answer.expects_reply,
        "an answer sent through the tool must not demand another, or the cascade re-arms"
    );
}

#[tokio::test]
async fn a_peer_that_answered_through_the_tool_does_not_then_write_to_the_operator() {
    let stub = serve(|body| {
        if speaker(body) == "Chef" {
            if has_tool_result(body) {
                Script::Say("Replied to Manager confirming how I work.".into())
            } else {
                Script::SendTo {
                    recipients: vec!["Manager".into()],
                    text: "Glad to be aboard.".into(),
                }
            }
        } else if has_tool_result(body) || reading_peer_replies(body) {
            Script::Say("Chef is introduced.".into())
        } else {
            Script::SendTo { recipients: vec!["Chef".into()], text: "hello from manager".into() }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Introduce yourself to Chef.").unwrap();
    h.settle(run).await;

    let chef = h.runtime.store().channel_messages(h.id("Chef"), 200).unwrap();
    let to_operator: Vec<String> = chef
        .iter()
        .filter(|e| e.from.is_agent() && e.to == Participant::Human)
        .map(guac_lib::domain::envelope::Envelope::plain_text)
        .collect();
    assert!(
        to_operator.is_empty(),
        "Chef was answering Manager and wrote to the operator, who was never in it: \
         {to_operator:?}"
    );

    let recorded: Vec<String> = chef
        .iter()
        .filter(|e| e.to == Participant::System)
        .map(guac_lib::domain::envelope::Envelope::plain_text)
        .collect();
    assert!(
        recorded.iter().any(|t| t.contains("Replied to Manager")),
        "the trailing text has to be written down somewhere: {recorded:?}"
    );
}

#[tokio::test]
async fn the_agent_the_operator_wrote_to_still_reports_after_it_answers_its_crew() {
    let stub = serve(|body| {
        if speaker(body) == "Chef" {
            if anyone_said(body, "Now cost the fish") {
                Script::Say("Fish is costed.".into())
            } else if has_tool_result(body) {
                Script::Say("Told Manager about the soup.".into())
            } else {
                Script::Instruct {
                    recipients: vec!["Manager".into()],
                    text: "Soup is costed.".into(),
                }
            }
        } else if anyone_said(body, "Fish is costed") {
            Script::Say("Both costed.".into())
        } else if anyone_said(body, "Soup is costed") {
            if has_tool_result(body) {
                Script::Say("Robert, the soup is costed and the fish is under way.".into())
            } else {
                Script::Instruct {
                    recipients: vec!["Chef".into()],
                    text: "Now cost the fish.".into(),
                }
            }
        } else if has_tool_result(body) {
            Script::Say("Chef is on it.".into())
        } else {
            Script::Instruct { recipients: vec!["Chef".into()], text: "Cost the soup.".into() }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Cost the menu.").unwrap();
    h.settle(run).await;

    let reported: Vec<String> = h
        .runtime
        .store()
        .channel_messages(h.id("Manager"), 200)
        .unwrap()
        .iter()
        .filter(|e| {
            e.from == Participant::Agent { id: h.id("Manager") } && e.to == Participant::Human
        })
        .map(guac_lib::domain::envelope::Envelope::plain_text)
        .collect();
    assert!(
        reported.iter().any(|t| t.contains("the soup is costed")),
        "the report the operator was waiting for reached nobody:\n{}",
        h.transcript()
    );

    let from_chef: Vec<String> = h
        .runtime
        .store()
        .channel_messages(h.id("Chef"), 200)
        .unwrap()
        .iter()
        .filter(|e| e.from.is_agent() && e.to == Participant::Human)
        .map(guac_lib::domain::envelope::Envelope::plain_text)
        .collect();
    assert!(
        from_chef.is_empty(),
        "a worker the operator never wrote to still reported to them: {from_chef:?}"
    );
}

#[tokio::test]
async fn a_document_attached_after_the_answer_still_reaches_the_peer_that_asked() {
    let stub = serve(|body| {
        let calls = body["messages"]
            .as_array()
            .map(|m| m.iter().filter(|msg| msg["role"] == "tool").count())
            .unwrap_or(0);

        if speaker(body) == "Chef" {
            match calls {
                0 => Script::SendTo { recipients: vec!["Manager".into()], text: "On it.".into() },
                1 => Script::Attach { tool: "attach_file".into(), files: vec!["menu.md".into()] },
                _ => Script::Say("Tidied and attached.".into()),
            }
        } else if calls > 0 || reading_peer_replies(body) {
            Script::Say("Chef has it.".into())
        } else {
            Script::SendFiles {
                recipients: vec!["Chef".into()],
                text: "tidy this up".into(),
                files: vec!["menu.md".into()],
            }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let menu = h.runtime.files().put("menu.md", b"soup, then fish").unwrap();
    let run = h
        .runtime
        .send_from_human_with(h.id("Manager"), "Have Chef tidy the menu.", vec![menu])
        .unwrap();
    h.settle(run).await;

    let handed_back: Vec<String> = h
        .runtime
        .store()
        .channel_messages(h.id("Manager"), 200)
        .unwrap()
        .iter()
        .filter(|e| e.from == Participant::Agent { id: h.id("Chef") })
        .flat_map(|e| e.parts.clone())
        .filter_map(|part| match part {
            Part::File(file) => Some(file.name),
            _ => None,
        })
        .collect();
    assert_eq!(
        handed_back,
        vec!["menu.md".to_string()],
        "the document never reached the agent that asked for it:\n{}",
        h.transcript()
    );
}

#[tokio::test]
async fn a_refused_courtesy_tells_the_agent_what_to_do_instead() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorder = seen.clone();
    let stub = serve(move |body| {
        let text = body["messages"]
            .as_array()
            .map(|m| m.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();

        if has_tool_result(body) {
            recorder.lock().unwrap().push(text);
            Script::Say("understood".into())
        } else if text.contains("good to meet you") {
            Script::SendTo { recipients: vec!["Chef".into()], text: "thanks Chef".into() }
        } else if text.contains("hello from manager") {
            Script::SendTo { recipients: vec!["Manager".into()], text: "good to meet you".into() }
        } else {
            Script::SendTo { recipients: vec!["Chef".into()], text: "hello from manager".into() }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Introduce yourself to Chef.").unwrap();
    h.settle(run).await;

    let told = seen.lock().unwrap().join("\n");
    assert!(
        told.contains("neither of you is waiting on the other"),
        "the agent has to be told why, or it rewords and tries again: {told}"
    );
    assert!(
        told.contains("Put what you still need in your final message instead"),
        "a refusal without a way forward is a dead end: {told}"
    );
    assert!(
        told.contains(r#"intent "work""#),
        "and when the message really was work, the way through has to be named: {told}"
    );
}

#[tokio::test]
async fn a_second_instruction_to_a_peer_that_already_answered_is_delivered() {
    let stub = serve(|body| {
        let who = speaker(body);
        let text = body["messages"]
            .as_array()
            .map(|m| m.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();

        if who == "Chef" {
            if text.contains("go ahead and send it") {
                Script::Say("Sent.".into())
            } else {
                Script::Say("Ready, but I need you to confirm before I send.".into())
            }
        } else if text.contains("Sent.") {
            Script::Say("Done: Chef sent the mailing.".into())
        } else if text.contains("I need you to confirm") {
            Script::Instruct {
                recipients: vec!["Chef".into()],
                text: "Confirmed by the operator: go ahead and send it.".into(),
            }
        } else if has_tool_result(body) {
            Script::Say("Chef has been told to send it.".into())
        } else {
            Script::SendTo { recipients: vec!["Chef".into()], text: "prepare the mailing".into() }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Have Chef send the mailing.").unwrap();
    h.settle(run).await;

    let to_chef: Vec<String> = h
        .feed()
        .into_iter()
        .filter(|e| e.to == Participant::Agent { id: h.id("Chef") })
        .map(|e| e.plain_text())
        .collect();
    assert!(
        to_chef.iter().any(|t| t.contains("go ahead and send it")),
        "the follow-up instruction never reached Chef: {to_chef:?}"
    );
    assert!(
        h.channel_texts("Manager").iter().any(|t| t.contains("Sent.")),
        "Chef's answer never reached the Manager that instructed it:\n{}",
        h.transcript()
    );
    assert!(
        h.channel_texts("Manager").iter().any(|t| t.contains("Done: Chef sent the mailing.")),
        "and the Manager never closed the loop for the operator:\n{}",
        h.transcript()
    );
}

#[tokio::test]
async fn a_peer_instructed_after_it_answered_does_the_work_rather_than_going_quiet() {
    let stub = serve(|body| {
        let who = speaker(body);
        let text = body["messages"]
            .as_array()
            .map(|m| m.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();

        if who == "Chef" {
            if !text.contains("go ahead and send it") {
                Script::Say("I have the file open, confirm before I send.".into())
            } else if text.contains("Nothing here needs an answer") {
                Script::Say(String::new())
            } else {
                Script::Say("Sent it.".into())
            }
        } else if text.contains("Sent it.") {
            Script::Say("Mailing confirmed sent.".into())
        } else if text.contains("confirm before I send") {
            Script::Instruct {
                recipients: vec!["Chef".into()],
                text: "Confirmed by the operator: go ahead and send it.".into(),
            }
        } else if has_tool_result(body) {
            Script::Say("Chef has been told.".into())
        } else {
            Script::SendTo { recipients: vec!["Chef".into()], text: "prepare the mailing".into() }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Have Chef send the mailing.").unwrap();
    h.settle(run).await;

    assert!(
        h.channel_texts("Manager").iter().any(|t| t.contains("Sent it.")),
        "the instruction landed and the agent went quiet, or its answer stranded:\n{}",
        h.transcript()
    );
}

#[tokio::test]
async fn work_and_a_reply_are_different_questions_on_the_wire() {
    let stub = serve(|body| {
        if speaker(body) == "Chef" {
            Script::Say("Done.".into())
        } else if has_tool_result(body) {
            Script::Say("Told.".into())
        } else {
            Script::Instruct { recipients: vec!["Chef".into()], text: "send it".into() }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Tell Chef to send it.").unwrap();
    h.settle(run).await;

    let instruction = h
        .runtime
        .store()
        .channel_messages(h.id("Chef"), 50)
        .unwrap()
        .into_iter()
        .find(|e| e.plain_text() == "send it")
        .expect("the instruction reached Chef");
    assert!(instruction.intent.is_work(), "what the sender declared has to survive the wire");
    assert!(
        instruction.expects_reply,
        "work expects an answer wherever in the exchange it lands; only a courtesy into a \
         settled pair does not"
    );
}

#[tokio::test]
async fn a_courtesy_to_a_peer_that_already_answered_is_still_refused() {
    let stub = serve(|body| {
        let text = body["messages"]
            .as_array()
            .map(|m| m.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();

        if has_tool_result(body) {
            Script::Say("understood".into())
        } else if text.contains("good to meet you") {
            Script::SendTo { recipients: vec!["Chef".into()], text: "thanks Chef".into() }
        } else if text.contains("hello from manager") {
            Script::SendTo { recipients: vec!["Manager".into()], text: "good to meet you".into() }
        } else {
            Script::SendTo { recipients: vec!["Chef".into()], text: "hello from manager".into() }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Introduce yourself to Chef.").unwrap();
    h.settle(run).await;

    let to_chef: Vec<String> = h
        .feed()
        .into_iter()
        .filter(|e| e.to == Participant::Agent { id: h.id("Chef") })
        .map(|e| e.plain_text())
        .collect();
    assert!(
        !to_chef.iter().any(|t| t.contains("thanks")),
        "a courtesy reached a peer that had already answered: {to_chef:?}"
    );
}

#[tokio::test]
async fn an_introduction_ends_when_everyone_has_answered() {
    let stub = serve(|body| {
        let text = body["messages"]
            .as_array()
            .map(|m| m.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();

        if has_tool_result(body) {
            Script::Say("introductions done".into())
        } else if text.contains("good to meet you") {
            Script::SendTo {
                recipients: vec!["Chef".into(), "Baker".into(), "Grocer".into()],
                text: "thanks all".into(),
            }
        } else if text.contains("hello from manager") {
            Script::SendTo { recipients: vec!["Manager".into()], text: "good to meet you".into() }
        } else {
            Script::SendTo {
                recipients: vec!["Chef".into(), "Baker".into(), "Grocer".into()],
                text: "hello from manager".into(),
            }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef", "Baker", "Grocer"], GuardLimits::default());
    let run =
        h.runtime.send_from_human(h.id("Manager"), "Introduce yourself to your team.").unwrap();
    h.settle(run).await;

    let feed = h.feed();
    let deepest = feed.iter().map(|e| e.hop).max().unwrap_or(0);
    assert_eq!(
        deepest,
        2,
        "three approaches and three answers is the whole exchange, got {:?}",
        feed.iter().map(|e| (e.hop, e.expects_reply)).collect::<Vec<_>>()
    );
    assert!(
        !feed.iter().any(|e| e.plain_text().contains("thanks all")),
        "an exchange where nobody is waiting cannot be restarted with a courtesy"
    );
}

#[tokio::test]
async fn a_group_can_pin_a_model_without_touching_the_other_group() {
    let stub = serve(|_| Script::Say("ok".into())).await;

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("guac.db")).unwrap();
    let pinned = store
        .create_group(&CleanGroup {
            name: "Local".into(),
            inference: Some(InferenceOverrides {
                default_model: Some("local/qwen".into()),
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();

    let mut inherits = draft("Inherits", &["testing"]);
    inherits.group_id = Some(pinned.id);
    inherits.model = String::new();
    let inherits = store.create_agent(&inherits).unwrap();

    let mut overrides = draft("Overrides", &["testing"]);
    overrides.group_id = Some(pinned.id);
    overrides.model = "explicit/model".into();
    let overrides = store.create_agent(&overrides).unwrap();

    let mut elsewhere = draft("Elsewhere", &["testing"]);
    elsewhere.model = String::new();
    let elsewhere = store.create_agent(&elsewhere).unwrap();

    let config = AppConfig {
        version: guac_lib::config::CURRENT_VERSION,
        operator_name: String::new(),
        inference: InferenceConfig {
            base_url: stub.base_url.clone(),
            api_key: "sk-test".into(),
            default_model: "app/default".into(),
            request_timeout_secs: 10,
            ..Default::default()
        },
        limits: GuardLimits::default(),
        e2b: Default::default(),
        kernel: Default::default(),
        search: Default::default(),
        webhook: Default::default(),
        quick_actions: Vec::new(),
        widgets: Vec::new(),
        attention: Default::default(),
        delivery: Default::default(),
    };
    let sink = RecordingSink::new();
    let runtime = Runtime::new(
        store,
        LlmClient::new().unwrap(),
        config,
        OnDisk::under(dir.path()),
        sink.clone(),
    );
    runtime.start_all().unwrap();
    let h = Harness { runtime, sink, ids: HashMap::new(), _dir: dir };

    for id in [inherits.id, overrides.id, elsewhere.id] {
        let run = h.runtime.send_from_human(id, "hello").unwrap();
        h.settle(run).await;
    }

    let models: Vec<String> = stub
        .transcript
        .lock()
        .iter()
        .filter_map(|body| body["model"].as_str().map(|s| s.to_string()))
        .collect();

    assert!(models.contains(&"local/qwen".to_string()), "group model must apply, got {models:?}");
    assert!(
        models.contains(&"explicit/model".to_string()),
        "an agent must still be able to override its group, got {models:?}"
    );
    assert!(
        models.contains(&"app/default".to_string()),
        "a group with no model must still inherit the app default, got {models:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_group_runs_on_its_own_budget_and_leaves_the_next_group_alone() {
    let stub = serve(|_| Script::Notes("still working".into())).await;

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("guac.db")).unwrap();

    let careful = store
        .create_group(&CleanGroup {
            name: "Careful".into(),
            inference: Some(InferenceOverrides {
                default_model: Some("careful/model".into()),
                ..Default::default()
            }),
            limits: Some(GroupLimits { max_steps_per_run: Some(2), ..Default::default() }),
            ..Default::default()
        })
        .unwrap();
    let roomy = store
        .create_group(&CleanGroup {
            name: "Roomy".into(),
            inference: Some(InferenceOverrides {
                default_model: Some("roomy/model".into()),
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();

    let mut thrifty = draft("Thrifty", &["testing"]);
    thrifty.group_id = Some(careful.id);
    thrifty.model = String::new();
    let thrifty = store.create_agent(&thrifty).unwrap();

    let mut busy = draft("Busy", &["testing"]);
    busy.group_id = Some(roomy.id);
    busy.model = String::new();
    let busy = store.create_agent(&busy).unwrap();

    let config = AppConfig {
        version: guac_lib::config::CURRENT_VERSION,
        operator_name: String::new(),
        inference: InferenceConfig {
            base_url: stub.base_url.clone(),
            api_key: "sk-test".into(),
            default_model: "app/default".into(),
            request_timeout_secs: 10,
            ..Default::default()
        },
        limits: GuardLimits { max_steps_per_run: 6, ..GuardLimits::default() },
        e2b: Default::default(),
        kernel: Default::default(),
        search: Default::default(),
        webhook: Default::default(),
        quick_actions: Vec::new(),
        widgets: Vec::new(),
        attention: Default::default(),
        delivery: Default::default(),
    };
    let sink = RecordingSink::new();
    let runtime = Runtime::new(
        store,
        LlmClient::new().unwrap(),
        config,
        OnDisk::under(dir.path()),
        sink.clone(),
    );
    runtime.start_all().unwrap();
    let h = Harness { runtime, sink, ids: HashMap::new(), _dir: dir };

    for id in [thrifty.id, busy.id] {
        let run = h.runtime.send_from_human(id, "get to work").unwrap();
        h.settle(run).await;
    }

    let models: Vec<String> = stub
        .transcript
        .lock()
        .iter()
        .filter_map(|body| body["model"].as_str().map(|s| s.to_string()))
        .collect();
    let calls = |model: &str| models.iter().filter(|m| m.as_str() == model).count();

    assert_eq!(
        calls("careful/model"),
        2,
        "the group's budget is the one that binds, got {models:?}"
    );
    assert_eq!(calls("roomy/model"), 6, "a group that sets no budget runs on the app's");

    let said: String = h
        .runtime
        .store()
        .channel_messages(thrifty.id, 200)
        .unwrap()
        .iter()
        .map(|e| serde_json::to_string(&e.parts).unwrap())
        .collect();
    assert!(said.contains("budget of 2 model calls"), "the operator must be told: {said}");
}

#[tokio::test]
async fn a_routine_that_is_due_wakes_its_agent() {
    let stub = serve(|_| Script::Say("checked the listings".into())).await;
    let h = harness(&stub, &["Watcher"], GuardLimits::default());

    h.runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Listings sweep",
            "check the listings",
            Trigger::Clock(Cadence::Every(3600)),
            Some(now_ms() - 1000),
            false,
        )
        .unwrap();

    h.start_scheduler();
    h.wait_until("the routine to fire", |h| {
        h.channel_texts("Watcher").iter().any(|t| t.contains("checked the listings"))
    })
    .await;

    let routines = h.runtime.store().agent_routines(h.id("Watcher")).unwrap();
    assert_eq!(routines.len(), 1, "a repeat must survive its own run");
    assert!(
        routines[0].next_run_at.expect("a clock routine holds a slot") > now_ms(),
        "and must not still be due, or it fires again on the next tick"
    );
    assert!(routines[0].last_run_at.is_some(), "it should record having run");
}

#[tokio::test]
async fn a_one_off_routine_does_not_come_due_twice() {
    let stub = serve(|_| Script::Say("woke up".into())).await;
    let h = harness(&stub, &["Sleeper"], GuardLimits::default());

    h.runtime
        .store()
        .create_routine(
            h.id("Sleeper"),
            "",
            "wake up",
            Trigger::Clock(Cadence::Once),
            Some(now_ms() - 1000),
            false,
        )
        .unwrap();

    h.start_scheduler();
    h.wait_until("the alarm to go off", |h| {
        h.channel_texts("Sleeper").iter().any(|t| t.contains("woke up"))
    })
    .await;

    assert!(
        h.runtime.store().agent_routines(h.id("Sleeper")).unwrap().is_empty(),
        "a one-off must be gone once it has run, not left with a time in the past"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_routine_that_skips_lands_on_the_idle_agent_and_not_on_the_working_one() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("done".into())
        } else if speaker(body) == "Busy" {
            Script::Hire {
                name: "Chief of Product".into(),
                instructions: "You own the roadmap.".into(),
                notes: String::new(),
            }
        } else {
            Script::Say("checked the listings".into())
        }
    })
    .await;

    let h = harness(&stub, &["Busy", "Idle"], GuardLimits::default());
    h.runtime.send_from_human(h.id("Busy"), "Create a chief of product.").unwrap();
    h.awaited_request().await;

    let mut booked = HashMap::new();
    for who in ["Busy", "Idle"] {
        booked.insert(
            who,
            h.runtime
                .store()
                .create_routine(
                    h.id(who),
                    "Listings sweep",
                    "check the listings",
                    Trigger::Clock(Cadence::Every(3600)),
                    Some(now_ms() - 1000),
                    true,
                )
                .unwrap(),
        );
    }

    h.start_scheduler();
    h.wait_until("both routines to come due", |h| {
        booked.values().all(|r| !h.runtime.store().routine_runs(r.id, 20).unwrap().is_empty())
    })
    .await;

    let skipped = h.runtime.store().routine_runs(booked["Busy"].id, 20).unwrap();
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].kind, RunKind::Skipped);
    assert_eq!(skipped[0].run_id, None, "nothing ran, so there is nothing to thread back to");
    assert!(
        !h.runtime
            .store()
            .channel_messages(h.id("Busy"), 50)
            .unwrap()
            .iter()
            .any(|m| m.parts.iter().any(|p| matches!(p, Part::Routine { .. }))),
        "the instruction must not be queued behind the turn it was skipped for:\n{}",
        h.transcript()
    );

    let after = h.runtime.store().get_routine(booked["Busy"].id).unwrap().unwrap();
    assert!(
        after.next_run_at.expect("a clock routine holds a slot") > now_ms(),
        "a skipped firing must not still be due, or it fires the moment the agent goes quiet"
    );
    assert_eq!(after.last_run_at, Some(after.next_run_at.unwrap() - 3_600_000));

    h.wait_until("the idle agent to do the work", |h| {
        h.channel_texts("Idle").iter().any(|t| t.contains("checked the listings"))
    })
    .await;
    let ran = h.runtime.store().routine_runs(booked["Idle"].id, 20).unwrap();
    assert_eq!(ran[0].kind, RunKind::Scheduled);
    assert!(ran[0].run_id.is_some());
}

#[tokio::test]
async fn a_routine_left_on_the_ordinary_rule_waits_its_turn_rather_than_being_dropped() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("done".into())
        } else {
            Script::Hire {
                name: "Chief of Product".into(),
                instructions: "You own the roadmap.".into(),
                notes: String::new(),
            }
        }
    })
    .await;

    let h = harness(&stub, &["Watcher"], GuardLimits::default());
    h.runtime.send_from_human(h.id("Watcher"), "Create a chief of product.").unwrap();
    h.awaited_request().await;

    let routine = h
        .runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Listings sweep",
            "check the listings",
            Trigger::Clock(Cadence::Every(3600)),
            Some(now_ms() - 1000),
            false,
        )
        .unwrap();

    h.start_scheduler();
    h.wait_until("the routine to be delivered", |h| {
        h.runtime
            .store()
            .channel_messages(h.id("Watcher"), 50)
            .unwrap()
            .iter()
            .any(|m| m.parts.iter().any(|p| matches!(p, Part::Routine { .. })))
    })
    .await;

    let history = h.runtime.store().routine_runs(routine.id, 20).unwrap();
    assert_eq!(history[0].kind, RunKind::Scheduled, "an agent being busy is not a reason to skip");
}

#[tokio::test]
async fn a_fired_routine_reaches_the_model_as_its_instruction_and_the_operator_as_one_line() {
    let stub = serve(|_| Script::Say("checked".into())).await;
    let h = harness(&stub, &["Watcher"], GuardLimits::default());

    let routine = h
        .runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Listings sweep",
            "Check the listings and say what is new.",
            Trigger::Clock(Cadence::Daily),
            Some(now_ms() + 60_000),
            false,
        )
        .unwrap();

    let run = h.runtime.test_routine(&routine).unwrap();
    h.settle(run).await;

    let sent = h.runtime.store().channel_messages(h.id("Watcher"), 20).unwrap();
    let fired = sent
        .iter()
        .find(|m| m.from == Participant::System)
        .expect("the routine was delivered from the system");
    assert_eq!(
        fired.plain_text(),
        "Check the listings and say what is new.",
        "the prompt reads a routine's instruction the same way it reads any text"
    );

    match fired.parts.as_slice() {
        [Part::Routine { routine_id, name, what, payload }] => {
            assert_eq!(*routine_id, routine.id, "so the operator can open the routine it names");
            assert_eq!(name, "Listings sweep");
            assert_eq!(what, "Check the listings and say what is new.");
            assert_eq!(*payload, None, "the clock carries no body");
        }
        other => panic!("a fired routine is one routine part, got {other:?}"),
    }

    assert!(
        h.channel_texts("Watcher").iter().any(|t| t.contains("checked")),
        "the routine fired and the agent said nothing:\n{}",
        h.transcript()
    );
}

#[tokio::test]
async fn testing_a_routine_delivers_it_without_spending_the_schedule() {
    let stub = serve(|_| Script::Say("checked the listings".into())).await;
    let h = harness(&stub, &["Watcher"], GuardLimits::default());

    let due_next_week = now_ms() + 7 * 24 * 60 * 60 * 1000;
    let routine = h
        .runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Sweep",
            "check the listings",
            Trigger::Clock(Cadence::Once),
            Some(due_next_week),
            false,
        )
        .unwrap();

    h.runtime.test_routine(&routine).unwrap();
    h.wait_until("the test run to reach the agent", |h| {
        h.channel_texts("Watcher").iter().any(|t| t.contains("checked the listings"))
    })
    .await;

    let after = h.runtime.store().agent_routines(h.id("Watcher")).unwrap();
    assert_eq!(after.len(), 1, "a one-shot must survive being tested");
    assert_eq!(after[0].next_run_at, Some(due_next_week), "and must still be due when it was");
    assert!(after[0].last_run_at.is_none(), "a test is not the routine having run");

    let history = h.runtime.store().routine_runs(routine.id, 20).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].kind, RunKind::Test);
}

#[cfg(feature = "server")]
async fn receiver(h: &Harness) -> (String, String) {
    let secret = "test-secret-that-is-long-enough-to-be-one";
    let mut config = h.runtime.config();
    config.webhook.secret = secret.into();
    h.runtime.set_config(config);
    let listener =
        tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("loopback is bindable");
    let addr = listener.local_addr().unwrap();
    let events = guac_lib::server::events::<()>(h.runtime.clone());
    tokio::spawn(async move { axum::serve(listener, events).await });
    (format!("http://{addr}"), secret.to_string())
}

fn dunning(h: &Harness, who: &str, skip_if_working: bool) -> guac_lib::domain::routine::Routine {
    h.runtime
        .store()
        .create_routine(
            h.id(who),
            "Dunning",
            "Chase the invoice named in the event.",
            Trigger::Event(EventTrigger {
                service: "stripe".into(),
                topic: "invoice.payment_failed".into(),
            }),
            None,
            skip_if_working,
        )
        .unwrap()
}

#[cfg(feature = "server")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_retries_of_one_event_claim_each_routine_only_once() {
    let stub = quiet().await;
    let h = harness(&stub, &["Clerk"], GuardLimits::default());
    let routine = dunning(&h, "Clerk", false);
    let (base, secret) = receiver(&h).await;
    let mut tasks = Vec::new();
    for _ in 0..12 {
        let url = format!("{base}/events/stripe/invoice.payment_failed");
        let secret = secret.clone();
        tasks.push(tokio::spawn(async move {
            reqwest::Client::new()
                .post(url)
                .bearer_auth(secret)
                .header("Idempotency-Key", "same delivery")
                .body("same payload")
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap()
        }));
    }
    let mut delivered = 0;
    for task in tasks {
        delivered += task.await.unwrap()["delivered"].as_u64().unwrap();
    }
    assert_eq!(delivered, 1);
    assert_eq!(h.runtime.store().routine_runs(routine.id, 20).unwrap().len(), 1);
}

#[cfg(feature = "server")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unkeyed_events_and_repeated_test_fires_are_independent() {
    let stub = quiet().await;
    let h = harness(&stub, &["Clerk"], GuardLimits::default());
    let routine = dunning(&h, "Clerk", false);
    let (base, secret) = receiver(&h).await;
    let url = format!("{base}/events/stripe/invoice.payment_failed");
    let client = reqwest::Client::new();
    for _ in 0..2 {
        assert_eq!(client.post(&url).bearer_auth(&secret).send().await.unwrap().status(), 200);
    }
    let first = h.runtime.test_routine(&routine).unwrap();
    let second = h.runtime.test_routine(&routine).unwrap();
    assert_ne!(first, second);
    let history = h.runtime.store().routine_runs(routine.id, 20).unwrap();
    assert_eq!(history.iter().filter(|row| row.kind == RunKind::Event).count(), 2);
    assert_eq!(history.iter().filter(|row| row.kind == RunKind::Test).count(), 2);
}

#[cfg(feature = "server")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_same_delivery_id_is_scoped_to_each_routine() {
    let stub = quiet().await;
    let h = harness(&stub, &["Clerk"], GuardLimits::default());
    let first = dunning(&h, "Clerk", false);
    let second = dunning(&h, "Clerk", false);
    let (base, secret) = receiver(&h).await;
    let url = format!("{base}/events/stripe/invoice.payment_failed");
    let client = reqwest::Client::new();
    for _ in 0..2 {
        assert_eq!(
            client
                .post(&url)
                .bearer_auth(&secret)
                .header("webhook-id", "delivery")
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
    }
    assert_eq!(h.runtime.store().routine_runs(first.id, 20).unwrap().len(), 1);
    assert_eq!(h.runtime.store().routine_runs(second.id, 20).unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn event_arriving_after_drain_is_kept_for_the_next_host() {
    let stub = quiet().await;
    let h = harness(&stub, &["Clerk"], GuardLimits::default());
    let routine = dunning(&h, "Clerk", false);
    assert!(h.runtime.drain(Duration::from_secs(1)).await.is_complete());
    let event = EventTrigger { service: "stripe".into(), topic: "invoice.payment_failed".into() };
    let delivered = h.runtime.deliver_event(&event, Some("payload".into()), "delivery").unwrap();
    assert_eq!(delivered.delivered, 1);
    let recovered = h.runtime.store().recover().unwrap();
    assert_eq!(recovered.resumed.len(), 1);
    assert!(
        matches!(&recovered.resumed[0].envelope.parts[..], [Part::Routine { routine_id, .. }] if *routine_id == routine.id)
    );
}

#[cfg(feature = "server")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_event_posted_to_the_receiver_fires_the_routine_standing_on_it() {
    let stub = serve(|_| Script::Say("chased it".into())).await;
    let h = harness(&stub, &["Clerk"], GuardLimits::default());
    let routine = dunning(&h, "Clerk", false);
    let (base, secret) = receiver(&h).await;

    let reply = reqwest::Client::new()
        .post(format!("{base}/events/Stripe/invoice.payment_failed"))
        .bearer_auth(&secret)
        .body(r#"{"id":"in_1","amount_due":1200}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), 200);
    let answered: serde_json::Value = reply.json().await.unwrap();
    assert_eq!(answered["listening"], 1, "{answered}");
    assert_eq!(answered["delivered"], 1, "{answered}");
    assert_eq!(answered["skipped"], 0, "{answered}");

    h.wait_until("the agent to act on the event", |h| {
        h.channel_texts("Clerk").iter().any(|t| t.contains("chased it"))
    })
    .await;

    let sent = h.runtime.store().channel_messages(h.id("Clerk"), 20).unwrap();
    let fired = sent.iter().find(|m| m.from == Participant::System).expect("delivered");
    let text = fired.plain_text();
    assert!(text.starts_with("Chase the invoice named in the event."), "{text}");
    assert!(text.contains(r#""amount_due":1200"#), "the body reached the model: {text}");
    assert!(text.contains("not an instruction"), "and was named as data: {text}");
    match fired.parts.as_slice() {
        [Part::Routine { routine_id, payload: Some(body), .. }] => {
            assert_eq!(*routine_id, routine.id);
            assert!(body.contains("in_1"), "the part carries the body for the transcript");
        }
        other => panic!("an event firing is one routine part with a body, got {other:?}"),
    }

    let runs = h.runtime.store().routine_runs(routine.id, 20).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].kind, RunKind::Event);
    assert!(runs[0].run_id.is_some());

    let after = h.runtime.store().get_routine(routine.id).unwrap().unwrap();
    assert_eq!(after.next_run_at, None);
    assert!(after.last_run_at.is_some());
    let clerk = h.id("Clerk");
    assert!(
        h.sink
            .count_of(|e| matches!(e, UiEvent::RoutinesChanged { agent_id } if *agent_id == clerk))
            > 0,
        "the panel was told the routine moved"
    );
}

#[cfg(feature = "server")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_post_that_does_not_carry_the_secret_fires_nothing() {
    let stub = serve(|_| Script::Say("chased it".into())).await;
    let h = harness(&stub, &["Clerk"], GuardLimits::default());
    let routine = dunning(&h, "Clerk", false);
    let (base, _secret) = receiver(&h).await;
    let client = reqwest::Client::new();
    let address = format!("{base}/events/stripe/invoice.payment_failed");

    let bare = client.post(&address).body("{}").send().await.unwrap();
    assert_eq!(bare.status(), 401);
    let wrong = client.post(&address).bearer_auth("nope").body("{}").send().await.unwrap();
    assert_eq!(wrong.status(), 401);
    let in_body = client.post(&address).body("secret=test-secret").send().await.unwrap();
    assert_eq!(in_body.status(), 401);
    let get = client.get(&address).send().await.unwrap();
    assert_eq!(get.status(), 405);
    let elsewhere = client.post(format!("{base}/hooks/stripe")).send().await.unwrap();
    assert_eq!(elsewhere.status(), 404);

    let refused: serde_json::Value = wrong.json().await.unwrap();
    assert!(
        refused["error"].as_str().unwrap_or_default().contains("Authorization: Bearer"),
        "a refusal says what to send: {refused}"
    );
    assert!(h.runtime.store().routine_runs(routine.id, 20).unwrap().is_empty());
    assert!(h.runtime.store().channel_messages(h.id("Clerk"), 20).unwrap().is_empty());
}

#[cfg(feature = "server")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_event_nobody_stands_on_is_answered_with_a_404() {
    let stub = serve(|_| Script::Say("chased it".into())).await;
    let h = harness(&stub, &["Clerk"], GuardLimits::default());
    dunning(&h, "Clerk", false);
    let (base, secret) = receiver(&h).await;

    let reply = reqwest::Client::new()
        .post(format!("{base}/events/stripe/invoice.paid"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), 404);
    let answered: serde_json::Value = reply.json().await.unwrap();
    assert_eq!(answered["listening"], 0);
    assert!(
        answered["error"].as_str().unwrap_or_default().contains("stripe/invoice.paid"),
        "names the event nobody waits on: {answered}"
    );
    assert!(h.runtime.store().channel_messages(h.id("Clerk"), 20).unwrap().is_empty());
}

#[cfg(feature = "server")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_event_that_lands_on_a_working_agent_is_skipped_when_the_routine_says_so() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("done".into())
        } else if speaker(body) == "Busy" {
            Script::Hire {
                name: "Chief of Product".into(),
                instructions: "You own the roadmap.".into(),
                notes: String::new(),
            }
        } else {
            Script::Say("chased it".into())
        }
    })
    .await;
    let h = harness(&stub, &["Busy", "Idle"], GuardLimits::default());
    h.runtime.send_from_human(h.id("Busy"), "Create a chief of product.").unwrap();
    h.awaited_request().await;
    let busy = dunning(&h, "Busy", true);
    let idle = dunning(&h, "Idle", true);
    let (base, secret) = receiver(&h).await;

    let reply = reqwest::Client::new()
        .post(format!("{base}/events/stripe/invoice.payment_failed"))
        .bearer_auth(&secret)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), 200);
    let answered: serde_json::Value = reply.json().await.unwrap();
    assert_eq!(answered["listening"], 2, "{answered}");
    assert_eq!(answered["delivered"], 1, "{answered}");
    assert_eq!(answered["skipped"], 1, "{answered}");

    let skipped = h.runtime.store().routine_runs(busy.id, 20).unwrap();
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].kind, RunKind::Skipped);
    assert_eq!(skipped[0].run_id, None);
    assert!(
        !h.runtime
            .store()
            .channel_messages(h.id("Busy"), 50)
            .unwrap()
            .iter()
            .any(|m| m.parts.iter().any(|p| matches!(p, Part::Routine { .. }))),
        "the event must not queue behind the turn it was skipped for:\n{}",
        h.transcript()
    );

    h.wait_until("the idle agent to act on the event", |h| {
        h.channel_texts("Idle").iter().any(|t| t.contains("chased it"))
    })
    .await;
    assert_eq!(h.runtime.store().routine_runs(idle.id, 20).unwrap()[0].kind, RunKind::Event);
}

#[cfg(feature = "server")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bearer_scheme_in_lower_case_is_accepted() {
    let stub = serve(|_| Script::Say("chased it".into())).await;
    let h = harness(&stub, &["Clerk"], GuardLimits::default());
    let routine = dunning(&h, "Clerk", false);
    let (base, secret) = receiver(&h).await;

    for scheme in ["bearer", "BEARER"] {
        let reply = reqwest::Client::new()
            .post(format!("{base}/events/stripe/invoice.payment_failed"))
            .header("Authorization", format!("{scheme} {secret}"))
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status(), 200, "{scheme}");
        let answered: serde_json::Value = reply.json().await.unwrap();
        assert_eq!(answered["delivered"], 1, "{scheme}: {answered}");
        assert_eq!(answered["failed"], 0, "{scheme}: {answered}");
        assert_eq!(answered["paused"], 0, "{scheme}: {answered}");
    }
    assert_eq!(h.runtime.store().routine_runs(routine.id, 20).unwrap().len(), 2);
}

#[cfg(feature = "server")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_event_for_a_paused_agents_routine_says_the_agent_is_paused() {
    let stub = serve(|_| Script::Say("chased it".into())).await;
    let h = harness(&stub, &["Clerk", "Spare"], GuardLimits::default());
    dunning(&h, "Clerk", false);
    h.pause("Clerk");
    let (base, secret) = receiver(&h).await;
    let post = || {
        reqwest::Client::new()
            .post(format!("{base}/events/stripe/invoice.payment_failed"))
            .bearer_auth(&secret)
            .body("{}")
            .send()
    };

    let reply = post().await.unwrap();
    assert_eq!(reply.status(), 409);
    let answered: serde_json::Value = reply.json().await.unwrap();
    assert_eq!(
        answered,
        serde_json::json!({
            "error": "The routine is there, but its agent is paused.",
            "listening": 0,
            "delivered": 0,
            "skipped": 0,
            "failed": 0,
            "paused": 1,
        })
    );
    assert!(h.runtime.store().channel_messages(h.id("Clerk"), 20).unwrap().is_empty());

    let spare = dunning(&h, "Spare", false);
    let reply = post().await.unwrap();
    assert_eq!(reply.status(), 200);
    let answered: serde_json::Value = reply.json().await.unwrap();
    assert_eq!(answered["listening"], 1, "{answered}");
    assert_eq!(answered["delivered"], 1, "{answered}");
    assert_eq!(answered["paused"], 1, "{answered}");
    assert!(answered.get("error").is_none(), "{answered}");
    assert_eq!(h.runtime.store().routine_runs(spare.id, 20).unwrap()[0].kind, RunKind::Event);
}

#[cfg(feature = "server")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_secret_is_checked_before_the_event_is_read() {
    let stub = quiet().await;
    let h = harness(&stub, &["Clerk"], GuardLimits::default());
    let (base, secret) = receiver(&h).await;
    let client = reqwest::Client::new();

    let refused = client
        .post(format!("{base}/events/not an/event"))
        .header("Authorization", "Bearer nope")
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 401);

    let missing = client.post(format!("{base}/events/stripe/invoice.paid")).send().await.unwrap();
    assert_eq!(missing.status(), 401);

    let near = secret.replace('e', "f");
    assert_eq!(near.len(), secret.len());
    let refused = client
        .post(format!("{base}/events/stripe/invoice.paid"))
        .bearer_auth(&near)
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 401);
}

#[cfg(feature = "server")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delivery_id_is_checked_only_after_the_secret() {
    let stub = quiet().await;
    let h = harness(&stub, &["Clerk"], GuardLimits::default());
    let (base, secret) = receiver(&h).await;
    let client = reqwest::Client::new();
    let url = format!("{base}/events/github/push");
    let bad = "x".repeat(201);

    let unsigned = client.post(&url).header("Idempotency-Key", &bad).send().await.unwrap();
    assert_eq!(unsigned.status(), 401);
    let signed = client
        .post(&url)
        .bearer_auth(&secret)
        .header("Idempotency-Key", &bad)
        .send()
        .await
        .unwrap();
    assert_eq!(signed.status(), 400);
}

#[cfg(feature = "server")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_client_waiting_to_be_told_to_continue_is_noticed() {
    let stub = quiet().await;
    let h = harness(&stub, &["Clerk"], GuardLimits::default());
    let routine = h
        .runtime
        .store()
        .create_routine(
            h.id("Clerk"),
            "Reviews",
            "Review the pull request named in the event.",
            Trigger::Event(EventTrigger {
                service: "github".into(),
                topic: "pull_request.opened".into(),
            }),
            None,
            false,
        )
        .unwrap();
    let (base, secret) = receiver(&h).await;

    let reply = reqwest::Client::new()
        .post(format!("{base}/events/github/pull_request.opened?delivery=1"))
        .bearer_auth(&secret)
        .header("Expect", "100-continue")
        .body(vec![b'x'; 2048])
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), 200);
    let answered: serde_json::Value = reply.json().await.unwrap();
    assert_eq!(answered["delivered"], 1, "{answered}");
    assert_eq!(h.runtime.store().routine_runs(routine.id, 20).unwrap()[0].kind, RunKind::Event);
}

#[cfg(feature = "server")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_request_still_says_how_much_body_is_on_its_way() {
    let stub = quiet().await;
    let h = harness(&stub, &["Clerk"], GuardLimits::default());
    let (base, _secret) = receiver(&h).await;

    let reply = reqwest::Client::new()
        .post(format!("{base}/events/stripe/invoice.paid"))
        .body(vec![b'x'; 40])
        .send()
        .await
        .expect("a refusal, not a reset");
    assert_eq!(reply.status(), 401);
    let refused: serde_json::Value = reply.json().await.unwrap();
    assert!(
        refused["error"].as_str().unwrap_or_default().contains("Authorization: Bearer"),
        "{refused}"
    );
}

fn at_the_cap(n: usize) -> String {
    format!(
        "Refused: you already keep {n} routines, which is the most one agent may keep. Cancel \
         one you no longer need with action \"cancel\", or change one with \"update\" so it \
         covers this."
    )
}

fn keep_routines(h: &Harness, who: &str, n: usize) -> Vec<guac_lib::domain::routine::Routine> {
    (0..n)
        .map(|i| {
            let routine = h
                .runtime
                .store()
                .create_routine(
                    h.id(who),
                    &format!("Sweep {i}"),
                    &format!("Check source number {i} and report what changed."),
                    Trigger::Clock(Cadence::Daily),
                    Some(now_ms() + 3_600_000),
                    false,
                )
                .unwrap();
            if i % 3 == 0 {
                h.runtime.store().set_routine_active(routine.id, false).unwrap()
            } else {
                routine
            }
        })
        .collect()
}

fn book_another(body: &serde_json::Value) -> Script {
    if has_tool_result(body) {
        return Script::Say("Done.".into());
    }
    Script::Book {
        name: "Newsletter".into(),
        what: "Draft the weekly newsletter from this week's notes.".into(),
        repeat: "weekly".into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_twenty_first_routine_an_agent_adds_is_refused_and_nothing_is_written() {
    let stub = serve(book_another).await;
    let h = harness(&stub, &["Watcher"], GuardLimits::default());
    keep_routines(&h, "Watcher", 20);
    let checkpoint = h.sink.snapshot().len();

    let asked =
        h.runtime.send_from_human(h.id("Watcher"), "Draft the newsletter every week.").unwrap();
    h.settle(asked).await;

    let told = tool_results(&stub);
    assert_eq!(told, vec![at_the_cap(20)], "the refusal, word for word");
    let standing = h.runtime.store().agent_routines(h.id("Watcher")).unwrap();
    assert_eq!(standing.len(), 20, "nothing was written");
    assert!(!standing.iter().any(|r| r.name == "Newsletter"));
    let watcher = h.id("Watcher");
    assert!(
        !h.sink.snapshot()[checkpoint..]
            .iter()
            .any(|e| matches!(e, UiEvent::RoutinesChanged { agent_id } if *agent_id == watcher)),
        "and nothing told the panel a routine changed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_operator_can_still_add_a_routine_past_the_cap() {
    let stub = serve(book_another).await;
    let h = harness(&stub, &["Watcher"], GuardLimits::default());
    keep_routines(&h, "Watcher", 21);
    assert_eq!(h.runtime.store().agent_routines(h.id("Watcher")).unwrap().len(), 21);

    let asked =
        h.runtime.send_from_human(h.id("Watcher"), "Draft the newsletter every week.").unwrap();
    h.settle(asked).await;

    assert_eq!(tool_results(&stub), vec![at_the_cap(21)]);
    assert_eq!(h.runtime.store().agent_routines(h.id("Watcher")).unwrap().len(), 21);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn update_and_cancel_still_work_at_the_cap() {
    let ids: Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let script_ids = ids.clone();
    let stub = serve(move |body| {
        let done = body["messages"]
            .as_array()
            .map(|m| m.iter().filter(|msg| msg["role"] == "tool").count())
            .unwrap_or(0);
        let ids = script_ids.lock().unwrap();
        match done {
            0 => Script::Retime { id: ids[0].clone(), repeat: "weekly".into() },
            1 => Script::Plugin {
                name: "schedule".into(),
                arguments: serde_json::json!({ "action": "cancel", "id": ids[1] }),
            },
            _ => Script::Say("Changed one and dropped one.".into()),
        }
    })
    .await;
    let h = harness(&stub, &["Watcher"], GuardLimits::default());
    let kept = keep_routines(&h, "Watcher", 20);
    *ids.lock().unwrap() = kept.iter().map(|r| r.id.to_string()).collect();

    let asked = h
        .runtime
        .send_from_human(h.id("Watcher"), "Make the first sweep weekly and drop the second.")
        .unwrap();
    h.settle(asked).await;

    let told = last_tool_results(&stub);
    assert!(told[0].starts_with("Updated "), "{told:?}");
    assert!(told[1].starts_with("Canceled "), "{told:?}");
    let standing = h.runtime.store().agent_routines(h.id("Watcher")).unwrap();
    assert_eq!(standing.len(), 19);
    let first = h.runtime.store().get_routine(kept[0].id).unwrap().unwrap();
    assert_eq!(first.trigger, Trigger::Clock(Cadence::Weekly));
    assert!(h.runtime.store().get_routine(kept[1].id).unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_change_an_agent_makes_to_its_schedule_redraws_the_panel() {
    let ids: Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let script_ids = ids.clone();
    let stub = serve(move |body| {
        let done = body["messages"]
            .as_array()
            .map(|m| m.iter().filter(|msg| msg["role"] == "tool").count())
            .unwrap_or(0);
        let ids = script_ids.lock().unwrap();
        match done {
            0 => Script::Book {
                name: "Newsletter".into(),
                what: "Draft the weekly newsletter from this week's notes.".into(),
                repeat: "weekly".into(),
            },
            1 => Script::Retime { id: ids[0].clone(), repeat: "weekly".into() },
            2 => Script::Plugin {
                name: "schedule".into(),
                arguments: serde_json::json!({ "action": "cancel", "id": ids[1] }),
            },
            _ => Script::Say("Added one, moved one and dropped one.".into()),
        }
    })
    .await;
    let h = harness(&stub, &["Watcher"], GuardLimits::default());
    let kept = keep_routines(&h, "Watcher", 2);
    *ids.lock().unwrap() = kept.iter().map(|r| r.id.to_string()).collect();
    let watcher = h.id("Watcher");
    let checkpoint = h.sink.snapshot().len();

    let asked = h
        .runtime
        .send_from_human(
            watcher,
            "Add the newsletter, make the first sweep weekly, drop the second.",
        )
        .unwrap();
    h.settle(asked).await;

    let told = last_tool_results(&stub);
    assert!(told[0].starts_with("Scheduled: "), "{told:?}");
    assert!(told[1].starts_with("Updated "), "{told:?}");
    assert!(told[2].starts_with("Canceled "), "{told:?}");
    let redrawn = h.sink.snapshot()[checkpoint..]
        .iter()
        .filter(|e| matches!(e, UiEvent::RoutinesChanged { agent_id } if *agent_id == watcher))
        .count();
    assert_eq!(
        redrawn, 3,
        "each write the agent made has to reach the panel beside its transcript"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fired_one_off_and_a_cancelled_routine_do_not_count_toward_the_cap() {
    let stub = serve(|body| {
        if anyone_said(body, "every week") {
            book_another(body)
        } else {
            Script::Say("Woke up.".into())
        }
    })
    .await;
    let h = harness(&stub, &["Watcher"], GuardLimits::default());
    keep_routines(&h, "Watcher", 18);
    let cancelled = h
        .runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Old sweep",
            "Check the old source.",
            Trigger::Clock(Cadence::Daily),
            Some(now_ms() + 3_600_000),
            false,
        )
        .unwrap();
    assert!(h.runtime.store().delete_routine(cancelled.id).unwrap());
    h.runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Alarm",
            "wake up",
            Trigger::Clock(Cadence::Once),
            Some(now_ms() - 1000),
            false,
        )
        .unwrap();
    h.runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Standing",
            "Check the standing source.",
            Trigger::Clock(Cadence::Daily),
            Some(now_ms() + 3_600_000),
            false,
        )
        .unwrap();

    h.start_scheduler();
    h.wait_until("the one-off to fire and go", |h| {
        h.channel_texts("Watcher").iter().any(|t| t.contains("Woke up."))
            && h.runtime.store().agent_routines(h.id("Watcher")).unwrap().len() == 19
    })
    .await;

    let asked =
        h.runtime.send_from_human(h.id("Watcher"), "Draft the newsletter every week.").unwrap();
    h.settle(asked).await;

    let told = tool_results(&stub);
    assert!(told.iter().any(|r| r.starts_with("Scheduled: ")), "{told:?}");
    assert_eq!(h.runtime.store().agent_routines(h.id("Watcher")).unwrap().len(), 20);
}

#[tokio::test]
async fn a_weekday_routine_slept_through_a_weekend_records_the_two_slots_it_missed() {
    let stub = serve(|_| Script::Say("swept".into())).await;
    let h = harness(&stub, &["Watcher"], GuardLimits::default());
    let local = |d: u32, hour: u32| {
        let date = chrono::NaiveDate::from_ymd_opt(2025, 1, d).unwrap();
        guac_lib::domain::instant(date.and_hms_opt(hour, 0, 0).unwrap()).unwrap()
    };
    let routine = h
        .runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Morning sweep",
            "Sweep the inbox.",
            Trigger::Clock(Cadence::Weekdays),
            Some(local(3, 9)),
            false,
        )
        .unwrap();
    h.runtime.set_up_since(local(6, 15));

    h.start_scheduler();
    h.wait_until("the catch-up firing", |h| {
        h.channel_texts("Watcher").iter().any(|t| t.contains("swept"))
    })
    .await;

    let history = h.runtime.store().routine_runs(routine.id, 20).unwrap();
    assert_eq!(history.len(), 1, "one firing, not one per missed day");
    assert_eq!(history[0].kind, RunKind::Scheduled);
    assert_eq!(history[0].missed, 2);
    let after = h.runtime.store().get_routine(routine.id).unwrap().unwrap();
    assert!(after.next_run_at.unwrap() > now_ms(), "and it is not due again");
}

#[tokio::test]
async fn a_routine_that_comes_due_while_the_host_is_up_missed_nothing() {
    let stub = serve(|_| Script::Say("swept".into())).await;
    let h = harness(&stub, &["Watcher"], GuardLimits::default());
    let routine = h
        .runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Sweep",
            "Sweep the inbox.",
            Trigger::Clock(Cadence::Every(3600)),
            Some(now_ms() - 1000),
            false,
        )
        .unwrap();
    h.runtime.set_up_since(now_ms() - 5000);

    h.start_scheduler();
    h.wait_until("the firing", |h| {
        !h.runtime.store().routine_runs(routine.id, 20).unwrap().is_empty()
    })
    .await;
    assert_eq!(h.runtime.store().routine_runs(routine.id, 20).unwrap()[0].missed, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_routine_that_fires_is_work_to_do_rather_than_something_to_note() {
    let stub = serve(|body| {
        let prompt = body["messages"][0]["content"].as_str().unwrap_or_default();
        if prompt.contains("Nothing here needs an answer") {
            Script::Say(String::new())
        } else {
            Script::Say("Swept the listings: three new ones.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Watcher"], GuardLimits::default());
    let routine = h
        .runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Listings sweep",
            "Check the listings and say what is new.",
            Trigger::Clock(Cadence::Daily),
            Some(now_ms() - 1000),
            false,
        )
        .unwrap();

    let run = h.runtime.test_routine(&routine).unwrap();
    h.settle(run).await;

    let prompt = prompts_by_agent(&stub).remove("Watcher").expect("the Watcher ran");
    assert!(
        prompt.contains("You have been given something to do"),
        "a routine coming due is work, and the turn has to be told so: {prompt}"
    );
    assert!(
        !prompt.contains("Saying nothing is allowed here"),
        "the silence permission belongs to the mode where nothing was asked: {prompt}"
    );
    assert!(
        h.channel_texts("Watcher").iter().any(|t| t.contains("three new ones")),
        "the routine fired and the agent said nothing:\n{}",
        h.transcript()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_routine_can_hand_its_work_to_the_specialist_it_belongs_to() {
    let stub = serve(|body| {
        let who = speaker(body);
        if who == "Researcher" {
            return Script::Say("Two new filings this week.".into());
        }
        let text = body["messages"]
            .as_array()
            .map(|m| m.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        if text.contains("Two new filings") {
            Script::Say("This week: two new filings.".into())
        } else if has_tool_result(body) {
            Script::Say(String::new())
        } else {
            Script::Instruct {
                recipients: vec!["Researcher".into()],
                text: "Check this week's filings.".into(),
            }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Researcher"], GuardLimits::default());
    let routine = h
        .runtime
        .store()
        .create_routine(
            h.id("Manager"),
            "Weekly filings",
            "Have the filings checked and report what is new.",
            Trigger::Clock(Cadence::Weekly),
            Some(now_ms() - 1000),
            false,
        )
        .unwrap();

    let run = h.runtime.test_routine(&routine).unwrap();
    h.settle(run).await;

    assert!(
        h.channel_texts("Researcher").iter().any(|t| t.contains("this week's filings")),
        "the routine's work never reached the agent it belongs to:\n{}",
        h.transcript()
    );
    assert!(
        h.channel_texts("Manager").iter().any(|t| t.contains("two new filings")),
        "and the answer never came back to the channel the operator reads:\n{}",
        h.transcript()
    );
    h.expect_normal(run, "a routine that delegates");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_standing_request_becomes_one_routine_that_does_the_job_when_it_fires() {
    let stub = serve(|body| {
        let woken_by_the_schedule = body["messages"]
            .as_array()
            .and_then(|m| m.last())
            .and_then(|m| m["content"].as_str())
            .unwrap_or_default()
            .contains("[SYSTEM]");

        if woken_by_the_schedule {
            Script::Say("Three new listings this morning.".into())
        } else if has_tool_result(body) {
            Script::Say("Booked for every weekday.".into())
        } else {
            Script::Book {
                name: "Listings sweep".into(),
                what: "Check the listings and say what is new.".into(),
                repeat: "weekdays".into(),
            }
        }
    })
    .await;

    let h = harness(&stub, &["Watcher"], GuardLimits::default());
    let asked = h
        .runtime
        .send_from_human(
            h.id("Watcher"),
            "Check the listings every weekday and tell me what's new.",
        )
        .unwrap();
    h.settle(asked).await;

    let booked = h.runtime.store().agent_routines(h.id("Watcher")).unwrap();
    assert_eq!(booked.len(), 1, "one standing job is one routine, got {booked:?}");
    assert_eq!(
        booked[0].trigger,
        Trigger::Clock(Cadence::Weekdays),
        "a weekday repeat is a shape on the calendar, not a gap in seconds"
    );
    assert_eq!(
        booked[0].title(),
        "Listings sweep",
        "and the operator's list shows the name it chose rather than the whole instruction"
    );
    assert!(
        tool_results(&stub).iter().any(|r| r.contains("Scheduled:") && r.contains("weekday")),
        "the agent has to be told what it set, because the reply is its only record of it: {:?}",
        tool_results(&stub)
    );

    let fired = h.runtime.test_routine(&booked[0]).unwrap();
    h.settle(fired).await;
    assert!(
        h.channel_texts("Watcher").iter().any(|t| t.contains("Three new listings")),
        "the routine fired and did nothing:\n{}",
        h.transcript()
    );
    assert_eq!(
        h.runtime.store().agent_routines(h.id("Watcher")).unwrap()[0].next_run_at,
        booked[0].next_run_at,
        "trying a routine out must not spend the slot it was holding"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_change_to_a_routine_changes_the_one_that_stands_rather_than_adding_a_second() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            return Script::Say("Moved it to every morning.".into());
        }
        match standing_id(body) {
            Some(id) => Script::Retime { id, repeat: "daily".into() },
            None => Script::Book {
                name: "Listings check".into(),
                what: "Check the listings and say what is new.".into(),
                repeat: "daily".into(),
            },
        }
    })
    .await;

    let h = harness(&stub, &["Watcher"], GuardLimits::default());
    h.runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Listings sweep",
            "Check the listings and say what is new.",
            Trigger::Clock(Cadence::Weekdays),
            Some(now_ms() + 3_600_000),
            false,
        )
        .unwrap();

    let asked = h
        .runtime
        .send_from_human(h.id("Watcher"), "Actually make that every day, not just weekdays.")
        .unwrap();
    h.settle(asked).await;

    let standing = h.runtime.store().agent_routines(h.id("Watcher")).unwrap();
    assert_eq!(
        standing.len(),
        1,
        "a change to a standing job is one routine, not two competing ones: {:?}",
        standing.iter().map(|r| (r.title().to_string(), r.describe())).collect::<Vec<_>>()
    );
    assert_eq!(
        standing[0].trigger,
        Trigger::Clock(Cadence::Daily),
        "and it is the change asked for"
    );
    assert_eq!(
        standing[0].name, "Listings sweep",
        "a field nobody sent is left alone, or retiming a routine loses its name"
    );
    assert!(
        tool_results(&stub).iter().any(|r| r.contains("Updated")),
        "the reply is the agent's only record of what it did: {:?}",
        tool_results(&stub)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_agent_cannot_retime_or_cancel_another_agents_routine() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            return Script::Say("Could not change it.".into());
        }
        let id = body["messages"]
            .as_array()
            .and_then(|m| m.last())
            .and_then(|m| m["content"].as_str())
            .and_then(|text| text.split("id ").nth(1))
            .map(|rest| rest.trim().trim_end_matches('.').to_string())
            .unwrap_or_default();
        Script::Retime { id, repeat: "monthly".into() }
    })
    .await;

    let h = harness(&stub, &["Watcher", "Scribe"], GuardLimits::default());
    let theirs = h
        .runtime
        .store()
        .create_routine(
            h.id("Scribe"),
            "Filing",
            "File yesterday's notes.",
            Trigger::Clock(Cadence::Weekdays),
            Some(now_ms() + 3_600_000),
            false,
        )
        .unwrap();

    let asked = h
        .runtime
        .send_from_human(h.id("Watcher"), &format!("Move the filing routine, id {}.", theirs.id))
        .unwrap();
    h.settle(asked).await;

    assert_eq!(
        h.runtime.store().get_routine(theirs.id).unwrap().unwrap().trigger,
        Trigger::Clock(Cadence::Weekdays),
        "one agent retimed another's routine"
    );
    assert!(
        tool_results(&stub).iter().any(|r| r.contains("no routine with the id")),
        "and the refusal has to say so rather than reporting success: {:?}",
        tool_results(&stub)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_routine_for_a_job_already_standing_is_named_while_the_turn_can_still_fix_it() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            return Script::Say("Booked.".into());
        }
        Script::Book {
            name: "Listings check".into(),
            what: "Check Zillow listings and email me a summary of anything new.".into(),
            repeat: "daily".into(),
        }
    })
    .await;

    let h = harness(&stub, &["Watcher"], GuardLimits::default());
    let first = h
        .runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Listings sweep",
            "Check the new listings and email me a summary.",
            Trigger::Clock(Cadence::Weekdays),
            Some(now_ms() + 3_600_000),
            false,
        )
        .unwrap();

    let asked = h
        .runtime
        .send_from_human(h.id("Watcher"), "Check the listings daily and email me.")
        .unwrap();
    h.settle(asked).await;

    let told = tool_results(&stub).join("\n");
    assert!(
        told.contains("same job"),
        "the turn has to be told it now has two routines doing one job: {told}"
    );
    assert!(
        told.contains(&first.id.to_string()),
        "with the id of the one it already had, or there is nothing it can act on: {told}"
    );
    assert!(told.contains("cancel"), "and the way out: {told}");
}

#[tokio::test]
async fn a_routine_that_is_switched_off_stays_quiet_and_starts_again_when_asked() {
    let stub = serve(|_| Script::Say("checked".into())).await;
    let h = harness(&stub, &["Watcher"], GuardLimits::default());

    let routine = h
        .runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Sweep",
            "check",
            Trigger::Clock(Cadence::Daily),
            Some(now_ms() - 1000),
            false,
        )
        .unwrap();
    h.runtime.store().set_routine_active(routine.id, false).unwrap();

    h.start_scheduler();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        h.channel_texts("Watcher").is_empty(),
        "an inactive routine must not fire, however overdue it is"
    );

    h.runtime.store().set_routine_active(routine.id, true).unwrap();
    tokio::time::pause();
    tokio::time::advance(SCHEDULE_TICK).await;
    tokio::time::resume();
    h.wait_until("it to fire once switched back on", |h| {
        h.channel_texts("Watcher").iter().any(|t| t.contains("checked"))
    })
    .await;
}

#[test]
fn a_schedule_that_cannot_be_read_parks_until_the_next_tick() {
    let (set_up, wait_for_set_up) = std::sync::mpsc::channel();
    let (finished, watch) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap();
        rt.block_on(async {
            let stub = serve(|_| Script::Say("never called".into())).await;
            let h = harness(&stub, &["Watcher"], GuardLimits::default());
            h.runtime.store().conn().unwrap().execute_batch("DROP TABLE routines").unwrap();

            let _ = set_up.send(());
            h.start_scheduler();

            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        let _ = finished.send(());
    });

    assert!(wait_for_set_up.recv().is_ok(), "the harness failed before the scheduler started");
    assert!(
        watch.recv_timeout(Duration::from_secs(10)).is_ok(),
        "the clock never moved, so the scheduler never parked: a schedule it cannot read has to \
         wait out the tick like one it can"
    );
}

fn signin_on(agent: guac_lib::domain::ids::AgentId, service: &str) -> Signin {
    Signin {
        agent_id: agent,
        surface: Surface::Browser,
        domain: format!("{}.example", service.to_lowercase()),
        service: service.into(),
        recognized: true,
        first_seen_at: 0,
        last_seen_at: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_is_told_what_its_browser_holds_and_what_its_peers_hold() {
    let stub = serve(|_| Script::Say("Noted.".into())).await;
    let h = harness_with_browser(&stub, &["Manager", "Researcher"], GuardLimits::default());

    let group = h.runtime.store().get_agent(h.id("Manager")).unwrap().unwrap().group_id;
    h.runtime
        .store()
        .replace_signins(
            h.id("Researcher"),
            Surface::Browser,
            &[signin_on(h.id("Researcher"), "LinkedIn")],
        )
        .unwrap();
    h.runtime
        .store()
        .create_connector(&CleanConnector {
            group_id: group,
            service: "GitHub".into(),
            account: "madebywelch".into(),
            env_var: "GITHUB_TOKEN".into(),
            note: String::new(),
            agents: vec![h.id("Manager"), h.id("Researcher")],
            secret: "ghp_hunter2".into(),
        })
        .unwrap();

    for agent in ["Manager", "Researcher"] {
        let run = h.runtime.send_from_human(h.id(agent), "hello").unwrap();
        h.settle(run).await;
    }

    let prompts = prompts_by_agent(&stub);
    let researcher = prompts.get("Researcher").expect("Researcher ran");
    let manager = prompts.get("Manager").expect("Manager ran");

    assert!(
        researcher.contains("You are signed in to these already")
            && researcher.contains("- LinkedIn in your browser"),
        "the agent whose browser is signed in must be told so: {researcher}"
    );
    assert!(
        !manager.contains("You are signed in to these already"),
        "cookies are in one jar; claiming otherwise produces a login wall: {manager}"
    );
    assert!(
        manager.contains("Researcher") && manager.contains("signed in to LinkedIn"),
        "the roster has to name the agent that can do it: {manager}"
    );

    for prompt in [researcher, manager] {
        assert!(prompt.contains("$GITHUB_TOKEN"), "the variable is named: {prompt}");
        assert!(!prompt.contains("ghp_hunter2"), "a secret reached a model: {prompt}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_account_on_a_place_that_was_taken_back_is_named_to_nobody() {
    let stub = serve(|_| Script::Say("Noted.".into())).await;
    let h = harness_with_browser(&stub, &["Manager", "Researcher"], GuardLimits::default());

    h.runtime
        .store()
        .replace_signins(
            h.id("Researcher"),
            Surface::Browser,
            &[signin_on(h.id("Researcher"), "LinkedIn")],
        )
        .unwrap();
    h.runtime.store().set_has_browser(h.id("Researcher"), false).unwrap();

    for agent in ["Manager", "Researcher"] {
        let run = h.runtime.send_from_human(h.id(agent), "hello").unwrap();
        h.settle(run).await;
    }

    let prompts = prompts_by_agent(&stub);
    let researcher = prompts.get("Researcher").expect("Researcher ran");
    let manager = prompts.get("Manager").expect("Manager ran");

    assert!(
        !researcher.contains("LinkedIn"),
        "an agent must not be told it holds an account it can no longer reach: {researcher}"
    );
    assert!(
        !manager.contains("signed in to LinkedIn"),
        "and a peer must not be sent to it: {manager}"
    );
    assert_eq!(h.runtime.store().agent_signins(h.id("Researcher")).unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_directory_says_which_peer_is_signed_in_to_what() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Researcher can.".into())
        } else {
            Script::Directory
        }
    })
    .await;

    let h = harness_with_browser(&stub, &["Manager", "Researcher"], GuardLimits::default());
    h.runtime
        .store()
        .replace_signins(
            h.id("Researcher"),
            Surface::Browser,
            &[signin_on(h.id("Researcher"), "LinkedIn")],
        )
        .unwrap();

    let run = h.runtime.send_from_human(h.id("Manager"), "who can post to LinkedIn?").unwrap();
    h.settle(run).await;

    let listing: String = stub
        .transcript
        .lock()
        .iter()
        .flat_map(|body| {
            body["messages"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|m| m["role"] == "tool")
                .map(|m| m["content"].as_str().unwrap_or_default().to_string())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        listing.contains("LinkedIn"),
        "the directory has to say what a peer is signed in to: {listing}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_cannot_add_a_colleague_without_the_operator_saying_so() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Chief of Product is set up.".into())
        } else {
            Script::Hire {
                name: "Chief of Product".into(),
                instructions: "You own the roadmap.".into(),
                notes: "# Context\nB2B services, founder-led sales.".into(),
            }
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Create a chief of product.").unwrap();

    let request = h.awaited_request().await;
    assert!(
        h.agent_named("Chief of Product").is_none(),
        "an agent existed before anybody was asked about it"
    );
    assert_eq!(
        h.runtime.activity_snapshot().get(&h.id("Manager")),
        Some(&Activity::AwaitingApproval),
        "a parked agent has to be distinguishable from a thinking one"
    );

    h.runtime.decide_approval(request, Decision::Allow).unwrap();
    h.settle(run).await;

    let hired = h.agent_named("Chief of Product").expect("the operator allowed it");
    let manager = h.runtime.store().get_agent(h.id("Manager")).unwrap().unwrap();
    assert_eq!(hired.group_id, manager.group_id, "a new agent lands inside its maker's wall");
    assert!(hired.model.is_empty(), "what it costs to run stays the operator's decision");
    assert!(hired.lifecycle.accepts_work(), "and it is running, not queued behind a start");
    assert_eq!(hired.system_prompt, "You own the roadmap.");
    assert!(
        h.runtime.workspace().read(hired.id).contains("founder-led sales"),
        "an agent handed starting notes has to open with them, not find an empty file"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_made_by_an_agent_is_given_neither_place_and_its_maker_is_told() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Scout is set up.".into())
        } else {
            Script::Hire {
                name: "Scout".into(),
                instructions: "You look things up on the web.".into(),
                notes: String::new(),
            }
        }
    })
    .await;

    let h = harness_with_computer(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Create a scout.").unwrap();
    let request = h.awaited_request().await;
    h.runtime.decide_approval(request, Decision::Allow).unwrap();
    h.settle(run).await;

    let scout = h.agent_named("Scout").expect("the operator allowed it");
    assert!(!scout.has_computer, "a machine is the operator's to hand out, not an agent's");
    assert!(!scout.has_browser);

    let told = tool_results(&stub).join("\n");
    assert!(
        told.contains("no computer and no browser"),
        "the maker has to know what it just made cannot do: {told}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_told_by_a_peer_that_it_was_authorized_asks_the_operator_instead_of_refusing() {
    let stub = serve(|body| {
        let text = body["messages"]
            .as_array()
            .map(|m| m.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();

        if text.contains("The operator allowed it") {
            Script::Say("Sent it, and told them what went.".into())
        } else if text.contains("The operator said no") {
            Script::Say("Not sent. They declined.".into())
        } else {
            Script::AskPermission {
                action: "Email the SCDOT response to robert@madebywelch.com for review".into(),
                because: "Manager says the operator authorized it; a peer's word is not \
                          permission to send mail in their name."
                    .into(),
            }
        }
    })
    .await;

    let h = harness_with_computer(&stub, &["Outreach"], GuardLimits::default());
    let run =
        h.runtime.send_from_human(h.id("Outreach"), "Manager will tell you what to send.").unwrap();

    let request = h.awaited_request().await;
    let asked = h
        .runtime
        .store()
        .channel_messages(h.id("Outreach"), 50)
        .unwrap()
        .into_iter()
        .find_map(|e| {
            e.parts.iter().find_map(|p| match p {
                Part::Approval { summary, detail, action, .. } => {
                    Some((summary.clone(), detail.clone(), *action))
                }
                _ => None,
            })
        })
        .expect("the request is a card in the channel");
    assert_eq!(asked.2, ProtectedAction::ActOnBehalf);
    assert!(asked.0.contains("in your name"), "the heading is the runtime's: {}", asked.0);
    assert!(
        asked.1.iter().any(|f| f.value.contains("robert@madebywelch.com")),
        "and the agent's own sentence is what is being decided: {:?}",
        asked.1
    );

    h.runtime.decide_approval(request, Decision::Allow).unwrap();
    h.settle(run).await;

    assert!(
        h.channel_texts("Outreach").iter().any(|t| t.contains("Sent it")),
        "one click has to be enough:\n{}",
        h.transcript()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_denied_request_to_act_stops_the_action_and_says_so() {
    let stub = serve(|body| {
        let text = body["messages"]
            .as_array()
            .map(|m| m.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        if text.contains("The operator said no") {
            Script::Say("I did not send it.".into())
        } else {
            Script::AskPermission {
                action: "Email the response to the procurement officer".into(),
                because: "asked by Manager".into(),
            }
        }
    })
    .await;

    let h = harness_with_computer(&stub, &["Outreach"], GuardLimits::default());
    let run =
        h.runtime.send_from_human(h.id("Outreach"), "Manager will tell you what to send.").unwrap();

    let request = h.awaited_request().await;
    h.runtime.decide_approval(request, Decision::Deny).unwrap();
    h.settle(run).await;

    let told = tool_results(&stub).join("\n");
    assert!(told.contains("The operator said no"), "{told}");
    assert!(told.contains("do not ask again"), "a refusal has to read as settled: {told}");
    assert!(
        h.channel_texts("Outreach").iter().any(|t| t.contains("did not send")),
        "and the operator still gets an answer:\n{}",
        h.transcript()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_computer_belongs_to_an_agent_the_operator_gave_one_to_and_not_to_the_workspace() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("I could not run that: I have no computer.".into())
        } else if speaker(body) == "Talker" {
            Script::RunCommand("uname -a".into())
        } else {
            Script::Say("Nothing needed doing.".into())
        }
    })
    .await;

    let h = harness_with_computer(&stub, &["Runner", "Talker"], GuardLimits::default());
    h.runtime.store().set_has_computer(h.id("Talker"), false).unwrap();

    let refused =
        h.runtime.send_from_human(h.id("Talker"), "What kernel is that machine on?").unwrap();
    h.settle(refused).await;
    let allowed = h.runtime.send_from_human(h.id("Runner"), "Anything to do?").unwrap();
    h.settle(allowed).await;

    let offered = tools_by_agent(&stub);
    assert!(
        offered["Runner"].contains(&"run_command".to_string()),
        "the agent that was given a computer still has one: {offered:?}"
    );
    for tool in ["run_command", "open_on_desktop", "use_screen"] {
        assert!(
            !offered["Talker"].contains(&tool.to_string()),
            "{tool} was offered to an agent with no computer: {offered:?}"
        );
    }

    let told = tool_results(&stub).join("\n");
    assert!(told.contains("you have no computer"), "{told}");
    assert!(
        told.contains("nothing to retry"),
        "a refusal that reads as a broken machine gets tried again: {told}"
    );
    assert!(
        h.agent_named("Talker").unwrap().sandbox_id.is_none(),
        "a refused call must not have rented a machine anyway"
    );

    assert!(
        h.channel_texts("Talker").iter().any(|t| t.contains("no computer")),
        "and the turn still ends in an answer:\n{}",
        h.transcript()
    );
    h.expect_normal(refused, "a refused tool call is an ordinary turn");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_screen_is_neither_offered_nor_served_to_a_model_that_cannot_be_shown_one() {
    let listing = serde_json::json!({
        "data": [{
            "id": "test/model",
            "architecture": { "input_modalities": ["text"], "output_modalities": ["text"] },
        }]
    });
    let stub = serve_publishing(Some(listing), |body| {
        if has_tool_result(body) {
            Script::Say("I cannot look at that screen, so here is what I did instead.".into())
        } else {
            Script::Look
        }
    })
    .await;

    let h = harness_with_computer(&stub, &["Runner"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Runner"), "What is on the screen?").unwrap();
    h.settle(run).await;

    let offered = tools_by_agent(&stub);
    assert!(!offered["Runner"].contains(&"use_screen".to_string()), "{offered:?}");
    assert!(offered["Runner"].contains(&"run_command".to_string()), "{offered:?}");
    assert!(offered["Runner"].contains(&"open_on_desktop".to_string()), "{offered:?}");

    let told = tool_results(&stub).join("\n");
    assert!(told.contains("a picture cannot reach the model"), "{told}");
    assert!(told.contains("`run_command`"), "a refusal needs a way forward: {told}");
    assert!(
        h.agent_named("Runner").unwrap().sandbox_id.is_none(),
        "a refused call must not have rented a machine anyway"
    );
    h.expect_normal(run, "a refused tool call is an ordinary turn");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_browser_is_given_the_same_way_and_refused_the_same_way() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("I could not open that: I have no browser.".into())
        } else if speaker(body) == "Talker" {
            Script::Open("https://example.com".into())
        } else {
            Script::Say("Nothing needed doing.".into())
        }
    })
    .await;

    let h = harness_with_browser(&stub, &["Reader", "Talker"], GuardLimits::default());
    h.runtime.store().set_has_browser(h.id("Talker"), false).unwrap();

    let refused = h.runtime.send_from_human(h.id("Talker"), "What is on example.com?").unwrap();
    h.settle(refused).await;
    let allowed = h.runtime.send_from_human(h.id("Reader"), "Anything to do?").unwrap();
    h.settle(allowed).await;

    let offered = tools_by_agent(&stub);
    assert!(offered["Reader"].contains(&"browse".to_string()), "{offered:?}");
    assert!(
        !offered["Talker"].contains(&"browse".to_string()),
        "browse was offered to an agent with no browser: {offered:?}"
    );

    let told = tool_results(&stub).join("\n");
    assert!(told.contains("you have no browser"), "{told}");
    assert!(
        h.agent_named("Talker").unwrap().browser_id.is_none(),
        "a refused call must not have opened a browser anyway"
    );
    h.expect_normal(refused, "a refused browse is an ordinary turn");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn asking_to_act_with_nowhere_to_act_is_refused_without_troubling_the_operator() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say(
                "I cannot reach your calendar from here. Nothing is connected to it.".into(),
            )
        } else {
            Script::AskPermission {
                action: "Read the operator's calendar for this week".into(),
                because: "the task needs their schedule and I have no access to it".into(),
            }
        }
    })
    .await;

    let h = harness(&stub, &["Assistant"], GuardLimits::default());
    let run =
        h.runtime.send_from_human(h.id("Assistant"), "What is on my calendar tomorrow?").unwrap();
    h.settle(run).await;

    assert_eq!(
        h.sink.count_of(|e| matches!(e, UiEvent::ApprovalRequested { .. })),
        0,
        "nobody should be asked a question their answer cannot settle"
    );

    let told = tool_results(&stub).join("\n");
    assert!(told.contains("missing is access, not permission"), "{told}");
    assert!(
        told.contains("give you a computer or a browser"),
        "a refusal with no way forward gets reworded and retried: {told}"
    );

    assert!(
        h.channel_texts("Assistant").iter().any(|t| t.contains("cannot reach your calendar")),
        "and the operator gets the sentence rather than the modal:\n{}",
        h.transcript()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refusal_reaches_the_model_as_a_decision_rather_than_a_failure() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Understood, no new agent.".into())
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
    let run = h.runtime.send_from_human(h.id("Manager"), "Create a scout.").unwrap();

    let request = h.awaited_request().await;
    h.runtime.decide_approval(request, Decision::Deny).unwrap();
    h.settle(run).await;

    assert!(h.agent_named("Scout").is_none(), "a denial has to mean nothing was created");

    let told = tool_results(&stub).join("\n");
    assert!(told.contains("said no"), "the model has to be told what happened: {told}");
    assert!(told.contains("do not ask again"), "and that it is settled: {told}");

    let said = h.channel_texts("Manager");
    assert!(said.iter().any(|t| t.contains("no new agent")), "{said:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn always_allow_means_the_same_agent_is_not_asked_twice() {
    let hires = Arc::new(AtomicUsize::new(0));
    let counter = hires.clone();
    let stub = serve(move |body| {
        if has_tool_result(body) {
            Script::Say("Done.".into())
        } else {
            let nth = counter.fetch_add(1, Ordering::SeqCst);
            Script::Hire {
                name: format!("Scout {nth}"),
                instructions: "You look things up.".into(),
                notes: String::new(),
            }
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());

    let first = h.runtime.send_from_human(h.id("Manager"), "Create a scout.").unwrap();
    let request = h.awaited_request().await;
    h.runtime.decide_approval(request, Decision::AlwaysAllow).unwrap();
    h.settle(first).await;

    let second = h.runtime.send_from_human(h.id("Manager"), "Create another scout.").unwrap();
    h.settle(second).await;

    assert!(h.agent_named("Scout 0").is_some(), "the approved one");
    assert!(h.agent_named("Scout 1").is_some(), "and the one that needed no asking");
    assert_eq!(
        h.sink.count_of(|e| matches!(e, UiEvent::ApprovalRequested { .. })),
        1,
        "the operator answered once and should not have been asked again"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_name_already_taken_is_refused_without_troubling_the_operator() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("There is already a Chef.".into())
        } else {
            Script::Hire {
                name: "Chef".into(),
                instructions: "You cook.".into(),
                notes: String::new(),
            }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Create a chef.").unwrap();
    h.settle(run).await;

    assert_eq!(
        h.sink.count_of(|e| matches!(e, UiEvent::ApprovalRequested { .. })),
        0,
        "nobody should have been asked"
    );
    let told = tool_results(&stub).join("\n");
    assert!(told.contains("already an agent called Chef"), "{told}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_connection_that_drops_once_is_retried_rather_than_reported() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = attempts.clone();
    let stub = serve(move |_body| {
        if counter.fetch_add(1, Ordering::SeqCst) == 0 {
            Script::Unavailable
        } else {
            Script::Say("Answered on the second attempt.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "hello").unwrap();
    h.settle(run).await;

    assert!(attempts.load(Ordering::SeqCst) >= 2, "the failed call was never tried again");
    let said = h.channel_texts("Manager");
    assert!(said.iter().any(|t| t.contains("second attempt")), "{said:?}");
    assert!(
        !said.iter().any(|t| t.contains("could not reach")),
        "a failure the runtime recovered from must not reach the operator: {said:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_that_keeps_failing_is_reported_with_the_message_to_send_again() {
    let stub = serve(|_body| Script::Unavailable).await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "what is the plan?").unwrap();
    h.settle(run).await;

    let failure = h
        .runtime
        .store()
        .channel_messages(h.id("Manager"), 50)
        .unwrap()
        .into_iter()
        .find(|m| {
            m.parts.iter().any(|p| {
                matches!(
                    p,
                    Part::Notice {
                        kind: guac_lib::domain::envelope::NoticeKind::UpstreamError,
                        ..
                    }
                )
            })
        })
        .expect("the operator has to be told the call failed");

    let cause = failure.cause.expect("the notice must name what to send again");
    assert_eq!(
        h.runtime.store().get_message(cause).unwrap().unwrap().plain_text(),
        "what is the plan?",
        "retrying has to re-deliver the message the turn was answering"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_that_fails_after_tool_calls_keeps_the_record_of_them() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let manager_calls = Arc::new(AtomicUsize::new(0));
    let counter = manager_calls.clone();
    tokio::spawn(async move {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move |body: axum::extract::Json<serde_json::Value>| {
                let counter = counter.clone();
                async move {
                    if speaker(&body.0) == "Chef" {
                        let said = render(&Script::Say(String::new()));
                        return ([("content-type", "text/event-stream")], said).into_response();
                    }
                    let script = match counter.fetch_add(1, Ordering::SeqCst) {
                        0 => Script::Narrate {
                            text: "Noting where this stands first.".into(),
                            then: Box::new(Script::Progress("menu quote requested".into())),
                        },
                        1 => Script::SendTo {
                            recipients: vec!["Chef".into()],
                            text: "Quote the menu.".into(),
                        },
                        _ => {
                            return (
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                "upstream exploded",
                            )
                                .into_response()
                        }
                    };
                    ([("content-type", "text/event-stream")], render(&script)).into_response()
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let stub = Stub {
        base_url: format!("http://{addr}/v1"),
        calls: Arc::new(AtomicUsize::new(0)),
        transcript: Arc::new(parking_lot::Mutex::new(Vec::new())),
    };
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Get a quote.").unwrap();
    h.settle(run).await;

    let manager = Participant::Agent { id: h.id("Manager") };
    let channel = h.runtime.store().channel_messages(h.id("Manager"), 50).unwrap();
    let records: Vec<usize> = channel
        .iter()
        .enumerate()
        .filter(|(_, m)| m.from == manager && m.to == Participant::System)
        .map(|(at, _)| at)
        .collect();
    assert_eq!(records.len(), 1, "one record of the turn:\n{}", h.transcript());
    let record = &channel[records[0]];
    let called: Vec<&str> = record
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::ToolCall { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(called, ["note_progress", "send_message"], "{:?}", record.parts);
    assert!(
        record.parts.iter().any(|part| matches!(part,
            Part::Text { text } if text.contains("Noting where this stands"))),
        "the words written before the failure are kept on the record: {:?}",
        record.parts
    );
    assert!(
        !channel.iter().any(|m| m.from == manager && m.to == Participant::Human),
        "half a turn is not a reply:\n{}",
        h.transcript()
    );

    let notices: Vec<(usize, &str)> = channel
        .iter()
        .enumerate()
        .flat_map(|(at, m)| {
            m.parts.iter().filter_map(move |part| match part {
                Part::Notice {
                    kind: guac_lib::domain::envelope::NoticeKind::UpstreamError,
                    text,
                    ..
                } => Some((at, text.as_str())),
                _ => None,
            })
        })
        .collect();
    assert_eq!(notices.len(), 1, "{notices:?}");
    let (at, said) = notices[0];
    assert!(at > records[0], "the notice follows the calls it warns about");
    assert!(said.contains("already made 2 calls"), "{said}");
    assert!(said.contains("Check them before you send this again"), "{said}");

    assert_eq!(h.messaged_by("Manager"), vec![("Chef".to_string(), 1)]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retrying_delivers_the_same_message_again_under_a_fresh_budget() {
    let stub = serve(|_body| Script::Say("Answered.".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());

    let first = h.runtime.send_from_human(h.id("Manager"), "the question").unwrap();
    h.settle(first).await;

    let original = h
        .runtime
        .store()
        .channel_messages(h.id("Manager"), 50)
        .unwrap()
        .into_iter()
        .find(|m| m.plain_text() == "the question")
        .unwrap();

    let again = h.runtime.retry_turn(h.id("Manager"), original.id).unwrap();
    assert_ne!(again, first, "a retry is the operator acting, so it gets a run of its own");
    h.settle(again).await;

    let asked = h
        .runtime
        .store()
        .channel_messages(h.id("Manager"), 50)
        .unwrap()
        .into_iter()
        .filter(|m| m.plain_text() == "the question")
        .count();
    assert_eq!(asked, 2, "the agent has to read what it read before, not a summary of it");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retrying_something_that_is_no_longer_there_says_so() {
    let stub = serve(|_body| Script::Say("hi".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());

    let gone = guac_lib::domain::ids::MessageId::new();
    assert!(matches!(
        h.runtime.retry_turn(h.id("Manager"), gone),
        Err(guac_lib::runtime::RuntimeError::NothingToRetry)
    ));
}

#[tokio::test]
async fn an_agent_given_work_that_says_nothing_is_reported_rather_than_vanishing() {
    let stub = serve(|body| {
        let text = body["messages"]
            .as_array()
            .map(|m| m.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();

        if speaker(body) == "Chef" {
            if text.contains("send the invoice now") {
                Script::Say(String::new())
            } else {
                Script::Say("yes, I can send invoices".into())
            }
        } else if has_tool_result(body) {
            Script::Say("Chef has been told.".into())
        } else if text.contains("yes, I can send invoices") {
            Script::Instruct {
                recipients: vec!["Chef".into()],
                text: "send the invoice now".into(),
            }
        } else {
            Script::SendTo {
                recipients: vec!["Chef".into()],
                text: "can you send invoices?".into(),
            }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Have Chef send the invoice.").unwrap();
    h.settle(run).await;

    let instruction = h
        .runtime
        .store()
        .channel_messages(h.id("Chef"), 50)
        .unwrap()
        .into_iter()
        .find(|e| e.plain_text() == "send the invoice now")
        .expect("the second instruction reached Chef");
    assert!(instruction.intent.is_work(), "the scenario depends on this arriving as work");
    assert!(
        instruction.expects_reply,
        "work re-arms the reply path even for a peer that has already answered"
    );

    let reported = h
        .runtime
        .store()
        .channel_messages(h.id("Chef"), 50)
        .unwrap()
        .into_iter()
        .flat_map(|e| e.parts)
        .any(|part| {
            matches!(part, Part::Notice { ref text, .. } if text.contains("without reporting anything"))
        });
    assert!(
        reported,
        "Chef was given work, said nothing, and the operator was not told:\n{}",
        h.transcript()
    );
}

#[tokio::test]
async fn an_agent_reading_an_acknowledgment_may_still_say_nothing_quietly() {
    let stub = serve(|body| {
        if speaker(body) == "Chef" {
            Script::Say("good to meet you".into())
        } else if has_tool_result(body) {
            Script::Say(String::new())
        } else {
            Script::SendTo { recipients: vec!["Chef".into()], text: "hello from manager".into() }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Introduce yourself to Chef.").unwrap();
    h.settle(run).await;

    assert!(
        !h.transcript().contains("without reporting anything"),
        "nothing gave Manager work, so its silence is the design working:\n{}",
        h.transcript()
    );
}

fn said(body: &serde_json::Value) -> String {
    body["messages"]
        .as_array()
        .map(|m| m.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default()
}

fn escalated(body: &serde_json::Value) -> bool {
    let said = said(body);
    said.contains("on the operator's desk now") || said.contains("You already had that up")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_that_cannot_go_on_puts_it_on_the_desk_and_finishes_its_turn() {
    let stub = serve(|body| {
        if escalated(body) {
            Script::Say("Flagged it. I have done what I can without the deploy key.".into())
        } else {
            Script::Escalate("The deploy needs a key only you have.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Analyst"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Analyst"), "Ship it.").unwrap();
    h.settle(run).await;

    assert_eq!(
        h.sink.count_of(|e| matches!(e, UiEvent::ApprovalRequested { .. })),
        0,
        "an escalation must not park a turn"
    );
    assert_eq!(h.sink.count_of(|e| matches!(e, UiEvent::EscalationRaised { .. })), 1);

    let open = h.runtime.store().open_escalations(50).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].summary, "The deploy needs a key only you have.");
    assert_eq!(open[0].agent_id, h.id("Analyst"));
    assert_eq!(open[0].times, 1);

    assert!(
        h.transcript().contains("done what I can"),
        "the turn has to finish rather than stop on it:\n{}",
        h.transcript()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_turn_that_hits_the_same_wall_counts_rather_than_queues() {
    let stub = serve(|body| {
        if escalated(body) {
            Script::Say("Nothing else I can do here.".into())
        } else {
            Script::Escalate("The workspace tooling is down.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Analyst"], GuardLimits::default());
    for _ in 0..2 {
        let run = h.runtime.send_from_human(h.id("Analyst"), "Any progress?").unwrap();
        h.settle(run).await;
    }

    let open = h.runtime.store().open_escalations(50).unwrap();
    assert_eq!(open.len(), 1, "one agent, one open row");
    assert_eq!(open[0].times, 2, "and the count is what says how much has been lost to it");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_open_escalation_is_in_the_next_turns_prompt() {
    let stub = serve(|body| {
        if said(body).contains("What you have already escalated") {
            Script::Say("Still waiting on the key. Nothing has moved.".into())
        } else if escalated(body) {
            Script::Say("Flagged it.".into())
        } else {
            Script::Escalate("The deploy needs a key only you have.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Analyst"], GuardLimits::default());
    let first = h.runtime.send_from_human(h.id("Analyst"), "Ship it.").unwrap();
    h.settle(first).await;
    let second = h.runtime.send_from_human(h.id("Analyst"), "Any progress?").unwrap();
    h.settle(second).await;

    assert!(
        h.transcript().contains("Nothing has moved"),
        "the second turn has to have been shown its own open escalation:\n{}",
        h.transcript()
    );
    assert_eq!(
        h.runtime.store().open_escalations(50).unwrap()[0].times,
        1,
        "and having been shown it, must not have raised it again as news"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clearing_one_says_so_and_leaves_the_agent_free_to_raise_another() {
    let stub = serve(|body| {
        if escalated(body) {
            Script::Say("Nothing else I can do here.".into())
        } else {
            Script::Escalate("The workspace tooling is down.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Analyst"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Analyst"), "Ship it.").unwrap();
    h.settle(run).await;

    let one = h.runtime.store().open_escalations(50).unwrap().remove(0);
    h.runtime.clear_escalation(one.id).unwrap();

    assert_eq!(h.sink.count_of(|e| matches!(e, UiEvent::EscalationCleared { .. })), 1);
    assert!(h.runtime.store().open_escalations(50).unwrap().is_empty());

    let again = h.runtime.send_from_human(h.id("Analyst"), "And now?").unwrap();
    h.settle(again).await;
    assert_eq!(h.runtime.store().open_escalations(50).unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_that_cannot_decide_asks_the_operator_and_carries_on_with_the_answer() {
    let stub = serve(|body| {
        let text = body["messages"]
            .as_array()
            .map(|m| m.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();

        if text.contains("The operator answered") {
            Script::Say("Going with Northwind, as you said.".into())
        } else {
            Script::AskQuestion {
                question: "Both vendors clear the bar on price. Which do you want?".into(),
                options: vec!["Northwind".into(), "Contoso".into()],
            }
        }
    })
    .await;

    let h = harness(&stub, &["Analyst"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Analyst"), "Pick a vendor.").unwrap();

    let request = h.awaited_request().await;
    assert_eq!(
        h.runtime.activity_snapshot().get(&h.id("Analyst")),
        Some(&Activity::AwaitingApproval),
        "a turn waiting on a person is parked whichever kind of thing it asked"
    );

    let asked = h.runtime.store().get_approval(request).unwrap().unwrap();
    assert_eq!(
        asked.request,
        Request::Question { options: vec!["Northwind".into(), "Contoso".into()] },
        "the choices the agent offered have to survive to the operator"
    );

    h.runtime.answer_question(request, "Northwind").unwrap();
    h.settle(run).await;

    let settled = h.runtime.store().get_approval(request).unwrap().unwrap();
    assert_eq!(settled.state, ApprovalState::Answered);
    assert_eq!(settled.answer.as_deref(), Some("Northwind"));
    assert!(
        h.transcript().contains("Going with Northwind"),
        "the answer has to reach the turn that stopped for it:\n{}",
        h.transcript()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_settled_question_says_more_on_the_page_and_nothing_more_to_the_agent() {
    let stub = serve(|body| {
        if anyone_said(body, "The operator answered") {
            Script::Say("Going with Northwind, as you said.".into())
        } else {
            Script::AskQuestion {
                question: "Both vendors clear the bar on price. Which do you want?".into(),
                options: vec!["Northwind".into(), "Contoso".into()],
            }
        }
    })
    .await;

    let h = harness(&stub, &["Analyst"], GuardLimits::default());
    let analyst = h.id("Analyst");
    let run = h.runtime.send_from_human(analyst, "Pick a vendor.").unwrap();
    let question = h.awaited_request().await;

    h.runtime.answer_question(question, "Northwind").unwrap();
    h.settle(run).await;

    let resumed = stub.transcript.lock().last().cloned().expect("the turn resumed");
    let handed_back: Vec<String> = resumed["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter(|message| message["role"] == "tool")
        .filter_map(|message| message["content"].as_str().map(str::to_string))
        .collect();
    assert_eq!(handed_back.len(), 1, "one tool result, and it is the answer");
    assert_eq!(
        handed_back[0],
        "The operator answered: Northwind\n\nThat is their decision, so take it and carry on in \
         this turn. Do not ask them the same thing again and do not ask a colleague to confirm \
         it. Say in your reply what you did with it.",
        "what the model is told is exactly what it was told before the card changed"
    );

    let stored = h.runtime.store().channel_messages(analyst, 50).unwrap();
    let asked = stored
        .iter()
        .find_map(|message| {
            message
                .parts
                .iter()
                .find(|part| matches!(part, Part::Question { id, .. } if *id == question))
        })
        .expect("the question is in the channel");
    assert_eq!(
        asked,
        &Part::Question {
            id: question,
            question: "Both vendors clear the bar on price. Which do you want?".into(),
            options: vec!["Northwind".into(), "Contoso".into()],
        },
        "the stored question carries the question and nothing learned since"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_question_nobody_answers_records_no_answer_and_releases_the_turn() {
    let stub = serve(|_| Script::AskQuestion {
        question: "Which vendor?".into(),
        options: vec!["Northwind".into(), "Contoso".into()],
    })
    .await;

    let h = harness(&stub, &["Analyst"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Analyst"), "Pick a vendor.").unwrap();

    let request = h.awaited_request().await;
    h.runtime.stop_run(run);
    h.settle(run).await;

    let settled = h.runtime.store().get_approval(request).unwrap().unwrap();
    assert_eq!(settled.state, ApprovalState::Expired);
    assert_eq!(settled.answer, None, "nobody said anything, so nothing may be recorded as said");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_question_cannot_be_answered_with_a_verdict_and_a_permission_cannot_be_answered_with_a_word(
) {
    let stub = serve(|_| Script::AskQuestion {
        question: "Which vendor?".into(),
        options: vec!["Northwind".into(), "Contoso".into()],
    })
    .await;

    let h = harness(&stub, &["Analyst"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Analyst"), "Pick a vendor.").unwrap();
    let question = h.awaited_request().await;

    assert!(
        h.runtime.decide_approval(question, Decision::Allow).is_err(),
        "a verdict is not an answer to a question"
    );
    assert!(
        h.runtime.answer_question(question, "   ").is_err(),
        "an empty answer settles the row and tells the agent nothing"
    );
    assert_eq!(
        h.runtime.store().get_approval(question).unwrap().unwrap().state,
        ApprovalState::Pending,
        "a refused answer must leave the request answerable"
    );

    h.runtime.answer_question(question, "Northwind").unwrap();
    h.runtime.stop_run(run);
    h.settle(run).await;
}

async fn quiet() -> Stub {
    serve(|_| Script::Say("ok".into())).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_deleted_agent_keeps_its_memory_while_it_waits() {
    let stub = quiet().await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let id = h.id("Researcher");
    h.runtime.workspace().write(id, "Researcher", "The vendor answers on Tuesdays.").unwrap();

    let card = h.runtime.store().get_agent(id).unwrap().unwrap();
    h.runtime.discard_agent(&card).await.unwrap();

    assert!(h.runtime.workspace().read(id).contains("Tuesdays"), "the memory has to wait with it");
    assert!(h.runtime.store().get_agent(id).unwrap().unwrap().discarded());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_deleted_agent_cannot_be_messaged_and_comes_back_able_to_be() {
    let stub = quiet().await;
    let h = harness(&stub, &["Manager", "Researcher"], GuardLimits::default());
    let id = h.id("Researcher");

    let card = h.runtime.store().get_agent(id).unwrap().unwrap();
    h.runtime.discard_agent(&card).await.unwrap();
    assert!(
        h.runtime.send_from_human(id, "still there?").is_err(),
        "an agent in the compost is as unreachable as one that is gone"
    );

    let back = h.runtime.restore_agent(&card).unwrap();
    assert_eq!(back.lifecycle, Lifecycle::Paused, "it comes back stopped, not working");
    let run = h.runtime.send_from_human(id, "welcome back").unwrap();
    h.runtime.stop_run(run);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restore_steps_around_a_name_the_crew_gave_away() {
    let stub = quiet().await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let card = h.runtime.store().get_agent(h.id("Researcher")).unwrap().unwrap();

    h.runtime.discard_agent(&card).await.unwrap();
    h.runtime.store().create_agent(&draft("Researcher", &[])).unwrap();

    let back = h.runtime.restore_agent(&card).unwrap();
    assert_eq!(back.name, "Researcher copy");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn emptying_the_compost_takes_the_memory_and_leaves_the_transcript() {
    let stub = quiet().await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let id = h.id("Researcher");
    h.runtime.workspace().write(id, "Researcher", "The vendor answers on Tuesdays.").unwrap();
    let run = h.runtime.send_from_human(id, "hello").unwrap();
    h.settle(run).await;

    let card = h.runtime.store().get_agent(id).unwrap().unwrap();
    h.runtime.discard_agent(&card).await.unwrap();
    h.runtime.purge_agent(&card).await.unwrap();

    assert!(h.runtime.workspace().read(id).is_empty(), "the memory goes with it");
    assert!(
        !h.runtime.store().get_agent(id).unwrap().unwrap().discarded(),
        "and the offer of a restore goes with the memory, or it is an offer nothing can keep"
    );
    assert!(
        !h.runtime.store().channel_messages(id, 50).unwrap().is_empty(),
        "what it said stays readable: a delete must not punch holes in transcripts"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_sweep_takes_whoever_is_past_the_deadline_and_nobody_else() {
    let stub = quiet().await;
    let h = harness(&stub, &["Researcher", "Scribe"], GuardLimits::default());
    let (due, waiting) = (h.id("Researcher"), h.id("Scribe"));

    for id in [due, waiting] {
        h.runtime.workspace().write(id, "whoever", "something worth keeping").unwrap();
    }
    h.runtime.store().discard_agent(due, now_ms() - COMPOST_MS - 1).unwrap();
    h.runtime.store().discard_agent(waiting, now_ms()).unwrap();

    h.runtime.sweep_compost().await;

    assert!(h.runtime.workspace().read(due).is_empty(), "its thirty days were up");
    assert!(
        h.runtime.workspace().read(waiting).contains("worth keeping"),
        "and the other one has twenty-nine days left"
    );
    assert!(h.runtime.store().get_agent(waiting).unwrap().unwrap().discarded());
}

#[derive(Clone, Copy, Debug)]
enum PurgeStep {
    Memory,
    Notebook,
    Routines,
    WorkingNotes,
    Signins,
    Approvals,
    PluginAccess,
    Stamp,
}

const HELD_TABLE: &str = "held_by_test";

fn purge_table(step: PurgeStep) -> Option<&'static str> {
    match step {
        PurgeStep::Routines => Some("routines"),
        PurgeStep::WorkingNotes => Some("working_notes"),
        PurgeStep::Signins => Some("signins"),
        PurgeStep::Approvals => Some("approvals"),
        PurgeStep::PluginAccess => Some("plugin_agents"),
        PurgeStep::Memory | PurgeStep::Notebook | PurgeStep::Stamp => None,
    }
}

fn notebook_dir(h: &Harness, id: guac_lib::domain::ids::AgentId) -> std::path::PathBuf {
    h._dir.path().join("notebooks").join(id.to_string())
}

fn break_purge(h: &Harness, id: guac_lib::domain::ids::AgentId, name: &str, step: PurgeStep) {
    let conn = h.runtime.store().conn().unwrap();
    if let Some(table) = purge_table(step) {
        conn.execute_batch(&format!("ALTER TABLE {table} RENAME TO {HELD_TABLE}")).unwrap();
        return;
    }
    match step {
        PurgeStep::Memory => {
            let path = h.runtime.workspace().preferred_path(id, name);
            std::fs::remove_file(&path).unwrap();
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("in-the-way"), "a folder where the memory was").unwrap();
        }
        PurgeStep::Notebook => {
            let dir = notebook_dir(h, id);
            std::fs::remove_dir_all(&dir).unwrap();
            std::fs::write(&dir, "a file where the notebook was").unwrap();
        }
        PurgeStep::Stamp => conn
            .execute_batch(
                "CREATE TRIGGER held_stamp BEFORE UPDATE OF discarded_at ON agents \
                 WHEN NEW.discarded_at IS NULL BEGIN SELECT RAISE(ABORT, 'held'); END",
            )
            .unwrap(),
        _ => unreachable!(),
    }
}

fn mend_purge(h: &Harness, id: guac_lib::domain::ids::AgentId, name: &str, step: PurgeStep) {
    let conn = h.runtime.store().conn().unwrap();
    if let Some(table) = purge_table(step) {
        conn.execute_batch(&format!("ALTER TABLE {HELD_TABLE} RENAME TO {table}")).unwrap();
        return;
    }
    match step {
        PurgeStep::Memory => {
            std::fs::remove_dir_all(h.runtime.workspace().preferred_path(id, name)).unwrap()
        }
        PurgeStep::Notebook => std::fs::remove_file(notebook_dir(h, id)).unwrap(),
        PurgeStep::Stamp => conn.execute_batch("DROP TRIGGER held_stamp").unwrap(),
        _ => unreachable!(),
    }
}

fn compost_due(h: &Harness, name: &str, at: i64) -> guac_lib::domain::ids::AgentId {
    let id = h.id(name);
    h.runtime.workspace().write(id, name, "The vendor answers on Tuesdays.").unwrap();
    h.runtime.notebooks().write(id, "leads.md", "Acme, call back Friday").unwrap();
    h.runtime.store().discard_agent(id, at).unwrap();
    id
}

fn still_composted(h: &Harness, id: guac_lib::domain::ids::AgentId) -> bool {
    h.runtime.store().get_agent(id).unwrap().unwrap().discarded()
}

async fn a_purge_that_fails_at(step: PurgeStep) {
    let stub = quiet().await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let id = compost_due(&h, "Researcher", now_ms() - COMPOST_MS - 1);
    let card = h.runtime.store().get_agent(id).unwrap().unwrap();

    break_purge(&h, id, "Researcher", step);
    let failed = h.runtime.purge_agent(&card).await;
    assert!(failed.is_err(), "{step:?}: a purge that could not finish said it had");
    assert!(
        still_composted(&h, id),
        "{step:?}: the stamp went while something it held was left behind, so nothing will retry"
    );

    mend_purge(&h, id, "Researcher", step);
    h.runtime.sweep_compost().await;
    assert!(
        !still_composted(&h, id),
        "{step:?}: the next sweep finishes what the failed one began"
    );
    assert!(h.runtime.workspace().read(id).is_empty(), "{step:?}: the memory is gone");
    assert!(h.runtime.notebooks().list(id).is_empty(), "{step:?}: the notebook is gone");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_purge_that_cannot_remove_the_memory_keeps_the_agent_in_the_compost() {
    a_purge_that_fails_at(PurgeStep::Memory).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_purge_that_cannot_remove_the_notebook_keeps_the_agent_in_the_compost() {
    a_purge_that_fails_at(PurgeStep::Notebook).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_purge_that_cannot_delete_the_schedule_keeps_the_agent_in_the_compost() {
    a_purge_that_fails_at(PurgeStep::Routines).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_purge_that_cannot_clear_the_working_notes_keeps_the_agent_in_the_compost() {
    a_purge_that_fails_at(PurgeStep::WorkingNotes).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_purge_that_cannot_delete_the_signins_keeps_the_agent_in_the_compost() {
    a_purge_that_fails_at(PurgeStep::Signins).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_purge_that_cannot_delete_the_approvals_keeps_the_agent_in_the_compost() {
    a_purge_that_fails_at(PurgeStep::Approvals).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_purge_that_cannot_revoke_plugin_access_keeps_the_agent_in_the_compost() {
    a_purge_that_fails_at(PurgeStep::PluginAccess).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_purge_that_cannot_clear_the_stamp_reports_it() {
    a_purge_that_fails_at(PurgeStep::Stamp).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_agent_the_sweep_cannot_purge_does_not_hold_up_the_rest() {
    let stub = quiet().await;
    let h = harness(&stub, &["Researcher", "Scribe"], GuardLimits::default());
    let stuck = compost_due(&h, "Researcher", now_ms() - COMPOST_MS - 2);
    let next = compost_due(&h, "Scribe", now_ms() - COMPOST_MS - 1);
    break_purge(&h, stuck, "Researcher", PurgeStep::Memory);

    h.runtime.sweep_compost().await;

    assert!(still_composted(&h, stuck), "the one that failed waits for the next sweep");
    assert!(!still_composted(&h, next), "the one after it is purged all the same");
    assert!(h.runtime.workspace().read(next).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deleting_forever_reports_a_purge_that_could_not_finish() {
    let stub = quiet().await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let id = compost_due(&h, "Researcher", now_ms());
    let state = command_state(&h);
    break_purge(&h, id, "Researcher", PurgeStep::Memory);

    assert!(guac_lib::commands::purge_agent(&state, id).await.is_err());
    assert!(still_composted(&h, id), "the operator can still restore it, or press again");

    mend_purge(&h, id, "Researcher", PurgeStep::Memory);
    guac_lib::commands::purge_agent(&state, id).await.unwrap();
    assert!(!still_composted(&h, id));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clearing_a_group_reports_memory_it_could_not_remove() {
    let stub = quiet().await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let id = h.id("Researcher");
    h.runtime.workspace().write(id, "Researcher", "The vendor answers on Tuesdays.").unwrap();
    let group = h.runtime.store().get_agent(id).unwrap().unwrap().group_id;
    let state = command_state(&h);
    break_purge(&h, id, "Researcher", PurgeStep::Memory);

    assert!(
        guac_lib::commands::clear_group(&state, group).await.is_err(),
        "a clear that left the memory behind said it had removed it"
    );
}

use guac_lib::domain::envelope::NoticeKind;

fn picked_up(body: &serde_json::Value) -> bool {
    anyone_said(body, "restarted partway through your turn")
}

fn notices_of(h: &Harness, name: &str, kind: NoticeKind) -> Vec<String> {
    h.runtime
        .store()
        .channel_messages(h.id(name), 200)
        .unwrap()
        .iter()
        .flat_map(|m| m.parts.clone())
        .filter_map(|part| match part {
            Part::Notice { kind: found, text, .. } if found == kind => Some(text),
            _ => None,
        })
        .collect()
}

async fn guard_after_restart(
    second_message: &'static str,
    max_sends_per_pair: u32,
    edit_limits: bool,
) -> Vec<String> {
    let calls = Arc::new(AtomicUsize::new(0));
    let manager_calls = calls.clone();
    let stub = serve(move |body| {
        if speaker(body) != "Manager" {
            return Script::Say("Done.".into());
        }
        match manager_calls.fetch_add(1, Ordering::SeqCst) {
            0 => {
                Script::Instruct { recipients: vec!["Chef".into()], text: "Prepare lunch.".into() }
            }
            1 => Script::Hang,
            2 => Script::Instruct { recipients: vec!["Chef".into()], text: second_message.into() },
            _ => Script::Say("Finished.".into()),
        }
    })
    .await;
    let h = harness(
        &stub,
        &["Manager", "Chef"],
        GuardLimits { max_sends_per_pair, ..GuardLimits::default() },
    );
    h.pause("Chef");
    let run = h.runtime.send_from_human(h.id("Manager"), "Ask Chef to prepare lunch.").unwrap();
    h.wait_until("the first send and the next model call", |h| {
        calls.load(Ordering::SeqCst) >= 2
            && h.feed().iter().any(|message| message.plain_text() == "Prepare lunch.")
    })
    .await;

    if edit_limits {
        use guac_lib::domain::group::{CleanGroup, GroupLimits};
        let store = h.runtime.store();
        let agent = store.get_agent(h.id("Manager")).unwrap().unwrap();
        let group = store.get_group(agent.group_id).unwrap().unwrap();
        store
            .update_group(
                group.id,
                &CleanGroup {
                    name: group.name,
                    limits: Some(GroupLimits { max_sends_per_pair: Some(6), ..Default::default() }),
                    ..Default::default()
                },
            )
            .unwrap();
    }

    let (h, drained) = h.restart(Duration::from_secs(5)).await;
    assert!(drained.is_complete(), "{drained:?}");
    h.wait_until("the resumed send to return", |_| calls.load(Ordering::SeqCst) >= 4).await;
    let results = tool_results(&stub);
    h.resume("Chef");
    h.settle(run).await;
    results
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pair_quota_spent_before_an_update_is_still_spent_after_it() {
    let results = guard_after_restart("Prepare dinner.", 1, false).await;
    assert!(
        results.iter().any(|result| result.contains("already sent Chef 1 messages")),
        "{results:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_sent_before_an_update_is_still_a_duplicate_after_it() {
    let results = guard_after_restart("Prepare lunch.", 6, false).await;
    assert!(
        results.iter().any(|result| result.contains("already sent Chef this exact message")),
        "{results:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_group_limit_edited_before_an_update_does_not_change_the_resumed_run() {
    let results = guard_after_restart("Prepare dinner.", 1, true).await;
    assert!(
        results.iter().any(|result| result.contains("already sent Chef 1 messages")),
        "{results:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reply_owed_before_an_update_does_not_request_another_reply() {
    let manager_calls = Arc::new(AtomicUsize::new(0));
    let calls = manager_calls.clone();
    let stub = serve(move |body| {
        if speaker(body) == "Chef" {
            if has_tool_result(body) {
                return Script::Say("Done.".into());
            }
            return Script::SendTo {
                recipients: vec!["Manager".into()],
                text: "Lunch is ready.".into(),
            };
        }
        match calls.fetch_add(1, Ordering::SeqCst) {
            0 => {
                Script::Instruct { recipients: vec!["Chef".into()], text: "Prepare lunch.".into() }
            }
            1 => Script::Hang,
            _ => Script::Say("Noted.".into()),
        }
    })
    .await;
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    h.pause("Chef");
    let run = h.runtime.send_from_human(h.id("Manager"), "Arrange lunch.").unwrap();
    h.wait_until("the instruction to queue", |h| {
        manager_calls.load(Ordering::SeqCst) >= 2
            && h.feed().iter().any(|message| message.plain_text() == "Prepare lunch.")
    })
    .await;
    let (h, drained) = h.restart(Duration::from_secs(5)).await;
    assert!(drained.is_complete(), "{drained:?}");
    h.resume("Chef");
    h.settle(run).await;
    let feed = h.feed();
    let answer = feed
        .iter()
        .find(|message| message.plain_text() == "Lunch is ready.")
        .expect("Chef's answer was delivered");
    assert!(
        !answer.expects_reply,
        "the outstanding instruction made this an answer, not a new request"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_an_update_stops_carries_on_after_it_without_making_its_calls_again() {
    let stub = serve(|body| {
        if picked_up(body) {
            Script::Say("Noted, and done.".into())
        } else if has_tool_result(body) {
            Script::Hang
        } else {
            Script::Progress("Checking the vendor's hours.".into())
        }
    })
    .await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let researcher = h.id("Researcher");
    let run = h.runtime.send_from_human(researcher, "Find the vendor's hours.").unwrap();
    h.wait_until("the second model call to be in the air", |_| {
        stub.calls.load(Ordering::SeqCst) >= 2
    })
    .await;

    let (h, drained) = h.restart(Duration::from_secs(5)).await;
    assert!(drained.is_complete(), "{drained:?}");
    h.settle(run).await;

    let fresh =
        stub.transcript.lock().iter().filter(|b| !picked_up(b) && !has_tool_result(b)).count();
    assert_eq!(fresh, 1, "asked once as though nothing had been done, and never again");
    assert_eq!(h.runtime.store().working_notes(researcher).unwrap().len(), 1);
    let resumed: Vec<serde_json::Value> =
        stub.transcript.lock().iter().filter(|b| picked_up(b)).cloned().collect();
    assert_eq!(resumed.len(), 1);
    assert!(anyone_said(&resumed[0], "note_progress"), "told which call it made");
    assert!(anyone_said(&resumed[0], "Find the vendor's hours."), "and what it was answering");

    let channel = h.runtime.store().channel_messages(researcher, 50).unwrap();
    let answer = channel.last().unwrap();
    assert_eq!(answer.to, Participant::Human, "the answer goes where it was always going");
    assert_eq!(answer.plain_text(), "Noted, and done.");
    assert!(
        notices_of(&h, "Researcher", NoticeKind::Lifecycle)
            .iter()
            .any(|text| text.contains("the host is restarting")),
        "the transcript says why it stopped: {}",
        h.transcript()
    );
    assert!(notices_of(&h, "Researcher", NoticeKind::Interrupted).is_empty(), "nothing to review");

    let steps = h.sink.snapshot().into_iter().find_map(|event| match event {
        UiEvent::RunSettled { run_id, steps_used } if run_id == run => Some(steps_used),
        _ => None,
    });
    assert_eq!(steps, Some(2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn absorbed_routine_resumes_with_its_consuming_turn_and_records() {
    let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let gate = release.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let stub = serve(move |body| {
        if picked_up(body) {
            return Script::Say("Finished.".into());
        }
        match seen.fetch_add(1, Ordering::SeqCst) {
            0 => {
                hold_until_open(
                    &gate,
                    "absorbed_routine_resumes_with_its_consuming_turn_and_records",
                );
                Script::Progress("Original work.".into())
            }
            1 => Script::Progress("Routine work happened once.".into()),
            _ => Script::Hang,
        }
    })
    .await;
    let h = harness(&stub, &["Clerk"], GuardLimits::default());
    let agent = h.id("Clerk");
    let routine = dunning(&h, "Clerk", false);
    let run = h.runtime.send_from_human(agent, "Do the original work.").unwrap();
    h.wait_until("the first model call", |_| calls.load(Ordering::SeqCst) == 1).await;
    h.runtime.test_routine(&routine).unwrap();
    release.store(true, Ordering::SeqCst);
    h.wait_until("the routine to be absorbed and acted on", |_| calls.load(Ordering::SeqCst) >= 3)
        .await;
    assert!(stub.transcript.lock().iter().any(|body| anyone_said(body, &routine.what)));
    assert_eq!(h.runtime.store().routine_runs(routine.id, 20).unwrap()[0].run_id, Some(run));
    let drained = h.runtime.drain(Duration::from_secs(5)).await;
    assert!(drained.is_complete());
    h.runtime.stop_agent(agent);
    let store = Store::open(&h._dir.path().join("guac.db")).unwrap();
    let recovered = guac_lib::boot::recover(&store).unwrap();
    assert_eq!(recovered.resumed.len(), 1, "the absorbed routine belongs to its consuming turn");
    store.set_lifecycle(agent, Lifecycle::Paused).unwrap();
    let sink = RecordingSink::new();
    let runtime = Runtime::new(
        store,
        LlmClient::new().unwrap(),
        h.runtime.config(),
        OnDisk::under(h._dir.path()),
        sink.clone(),
    );
    let started = runtime.start_all().unwrap();
    runtime.pick_up(&started, recovered);
    assert!(runtime.drain(Duration::from_secs(5)).await.is_complete());
    runtime.stop_agent(agent);
    let store = Store::open(&h._dir.path().join("guac.db")).unwrap();
    let recovered = guac_lib::boot::recover(&store).unwrap();
    assert_eq!(recovered.resumed.len(), 1);
    assert_eq!(recovered.resumed[0].absorbed.len(), 1);
    assert!(!recovered.resumed[0].records.is_empty());
    store.set_lifecycle(agent, Lifecycle::Active).unwrap();
    let runtime = Runtime::new(
        store,
        LlmClient::new().unwrap(),
        h.runtime.config(),
        OnDisk::under(h._dir.path()),
        sink.clone(),
    );
    let started = runtime.start_all().unwrap();
    runtime.pick_up(&started, recovered);
    let h = Harness { runtime, sink, ids: h.ids, _dir: h._dir };
    h.settle(run).await;
    h.wait_until("the resumed call", |_| stub.transcript.lock().iter().any(picked_up)).await;
    h.wait_until("all resumed work to settle", |h| h.runtime.live_runs() == 0).await;
    let transcript = stub.transcript.lock();
    let resumed: Vec<_> = transcript.iter().filter(|body| picked_up(body)).collect();
    assert_eq!(resumed.len(), 1, "the routine must not become a separate turn");
    assert!(anyone_said(resumed[0], &routine.what));
    assert!(anyone_said(resumed[0], "Routine work happened once."));
    assert_eq!(calls.load(Ordering::SeqCst), 3, "the routine must not run again");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn work_queued_behind_a_busy_agent_is_carried_across_an_update() {
    let restarted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let after = restarted.clone();
    let stub = serve(move |_| {
        if after.load(Ordering::SeqCst) {
            Script::Say("Both handled.".into())
        } else {
            Script::Hang
        }
    })
    .await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let researcher = h.id("Researcher");
    let first = h.runtime.send_from_human(researcher, "The first thing.").unwrap();
    h.wait_until("the first turn to be in the air", |_| stub.calls.load(Ordering::SeqCst) >= 1)
        .await;
    let second = h.runtime.send_from_human(researcher, "The second thing.").unwrap();
    h.wait_until("the second to queue", |h| h.runtime.inbox_depth(researcher) == 1).await;

    restarted.store(true, Ordering::SeqCst);
    let (h, drained) = h.restart(Duration::from_secs(5)).await;
    assert!(drained.is_complete(), "{drained:?}");
    h.settle(first).await;
    h.settle(second).await;

    let after_restart: Vec<serde_json::Value> = stub.transcript.lock()[1..].to_vec();
    assert_eq!(after_restart.len(), 1, "the first turn was dropped mid-call and never answered");
    let prompt = after_restart[0].to_string();
    let (one, two) =
        (prompt.find("The first thing.").unwrap(), prompt.find("The second thing.").unwrap());
    assert!(one < two, "in the order they were sent");
    assert!(notices_of(&h, "Researcher", NoticeKind::Interrupted).is_empty());
    assert_eq!(h.channel_texts("Researcher").last().unwrap(), "Both handled.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_sent_while_the_host_is_stopping_is_answered_by_the_next_one() {
    let stub = serve(|_| Script::Say("Here.".into())).await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let researcher = h.id("Researcher");

    assert!(h.runtime.drain(Duration::from_secs(5)).await.is_complete());
    let run = h.runtime.send_from_human(researcher, "Are you there?").unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0, "nothing starts on a host that is stopping");

    let (h, _) = h.restart(Duration::from_secs(5)).await;
    h.settle(run).await;
    assert_eq!(h.channel_texts("Researcher"), ["Are you there?", "Here."]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_conversation_the_operator_stopped_is_not_carried_on_by_the_next_host() {
    let stub = serve(|_| Script::Hang).await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let researcher = h.id("Researcher");
    let run = h.runtime.send_from_human(researcher, "Start something.").unwrap();
    h.wait_until("the call to be in the air", |_| stub.calls.load(Ordering::SeqCst) >= 1).await;
    h.runtime.stop_run(run);
    h.settle(run).await;

    let (h, _) = h.restart(Duration::from_secs(5)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(stub.calls.load(Ordering::SeqCst), 1, "called off is never picked up");
    assert!(notices_of(&h, "Researcher", NoticeKind::Interrupted).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_agents_held_work_waits_for_it_on_the_next_host() {
    let stub = serve(|_| Script::Say("Done as asked.".into())).await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let researcher = h.id("Researcher");
    let run = h.park("Researcher", "When you are back, do this.").await;

    let (h, drained) = h.restart(Duration::from_secs(5)).await;
    assert!(drained.is_complete(), "{drained:?}");
    h.wait_until("it to hold the message again", |h| {
        h.sink.snapshot().iter().any(|event| {
            matches!(event, UiEvent::ActivityChanged { agent_id, activity: Activity::Paused }
                if *agent_id == researcher)
        })
    })
    .await;
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0, "still paused: the card says so");

    h.resume("Researcher");
    h.settle(run).await;
    assert_eq!(h.channel_texts("Researcher").last().unwrap(), "Done as asked.");
}

use guac_lib::domain::ids::RunId;

async fn taken_in_mid_turn(
    h: &Harness,
    stub: &Stub,
    gate: &std::sync::atomic::AtomicBool,
    calls: &AtomicUsize,
) -> (RunId, RunId) {
    let researcher = h.id("Researcher");
    let first = h.runtime.send_from_human(researcher, "Find the vendor's hours.").unwrap();
    h.wait_until("the first model call", |_| calls.load(Ordering::SeqCst) == 1).await;
    let second = h.runtime.send_from_human(researcher, "And their phone number.").unwrap();
    h.wait_until("the second to queue", |h| h.runtime.inbox_depth(researcher) == 1).await;
    gate.store(true, Ordering::SeqCst);
    h.wait_until("the second to be taken into the running turn", |_| {
        stub.transcript.lock().iter().any(|body| anyone_said(body, "And their phone number."))
    })
    .await;
    (first, second)
}

fn settled_on(sink: &RecordingSink, run: RunId) -> bool {
    sink.snapshot()
        .iter()
        .any(|event| matches!(event, UiEvent::RunSettled { run_id, .. } if *run_id == run))
}

async fn gated_researcher(
    gate: Arc<std::sync::atomic::AtomicBool>,
    calls: Arc<AtomicUsize>,
) -> Stub {
    serve(move |body| {
        if picked_up(body) {
            return Script::Say("Hours and number found.".into());
        }
        match calls.fetch_add(1, Ordering::SeqCst) {
            0 => {
                hold_until_open(&gate, "gated_researcher");
                Script::Progress("Looking up the vendor.".into())
            }
            _ => Script::Hang,
        }
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_taken_into_a_turn_the_process_dies_in_is_reported_with_it() {
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let stub = gated_researcher(gate.clone(), calls.clone()).await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let researcher = h.id("Researcher");
    let (first, second) = taken_in_mid_turn(&h, &stub, &gate, &calls).await;
    let settled_while_read = settled_on(&h.sink, second);

    h.runtime.stop_agent(researcher);
    let store = Store::open(&h._dir.path().join("guac.db")).unwrap();
    let recovered = guac_lib::boot::recover(&store).unwrap();
    assert_eq!(recovered.interrupted, 2, "both messages the dead turn held are reported");
    let reported: std::collections::HashSet<RunId> = store
        .channel_messages(researcher, 50)
        .unwrap()
        .into_iter()
        .filter(|m| {
            matches!(m.parts.first(), Some(Part::Notice { kind: NoticeKind::Interrupted, .. }))
        })
        .map(|m| m.run_id)
        .collect();
    assert_eq!(reported, [first, second].into_iter().collect());
    assert!(!settled_while_read, "read is not answered: the turn that read it was still working");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_taken_into_a_turn_an_update_stops_is_handed_on_not_settled() {
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let stub = gated_researcher(gate.clone(), calls.clone()).await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let (first, second) = taken_in_mid_turn(&h, &stub, &gate, &calls).await;

    let stopping = h.sink.clone();
    let (h, drained) = h.restart(Duration::from_secs(5)).await;
    assert!(drained.is_complete(), "{drained:?}");
    assert!(
        !settled_on(&stopping, second),
        "the host that stopped settled a message its turn had read and not answered"
    );
    h.settle(first).await;
    h.settle(second).await;
    assert_eq!(h.channel_texts("Researcher").last().unwrap(), "Hours and number found.");
    assert!(notices_of(&h, "Researcher", NoticeKind::Interrupted).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_decision_answered_while_the_host_is_stopping_is_followed_through_by_the_next_one() {
    use guac_lib::domain::decision::DecisionRequest;

    let stub = serve(|body| {
        if anyone_said(body, "11 AM") {
            Script::Say("Booked for 11 AM.".into())
        } else {
            Script::Say("Nothing to do.".into())
        }
    })
    .await;
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
            now_ms(),
        )
        .unwrap();

    assert!(h.runtime.drain(Duration::from_secs(5)).await.is_complete());
    h.runtime.stop_agent(agent.id);
    let answered = h.runtime.answer_decision(decision.id, "11 AM", false, None).unwrap();
    let run = answered.delivery_run.unwrap();
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0, "nothing starts on a host that is stopping");

    let (h, _) = h.restart(Duration::from_secs(5)).await;
    h.settle(run).await;
    assert_eq!(h.channel_texts("Assistant").last().unwrap(), "Booked for 11 AM.");
    assert!(notices_of(&h, "Assistant", NoticeKind::Interrupted).is_empty());
    assert!(!h.runtime.store().decisions(None).unwrap()[0].interrupted);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_keyed_message_sent_while_the_host_is_stopping_is_answered_by_the_next_one() {
    let stub = serve(|_| Script::Say("Here.".into())).await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let researcher = h.id("Researcher");

    assert!(h.runtime.drain(Duration::from_secs(5)).await.is_complete());
    let (message, run) = h
        .runtime
        .send_from_human_keyed(researcher, "Are you there?", Vec::new(), "send-1", "print")
        .unwrap();
    assert_eq!(
        h.runtime
            .send_from_human_keyed(researcher, "Are you there?", Vec::new(), "send-1", "print")
            .unwrap(),
        (message, run),
        "a retry of the same send is the same message"
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0, "nothing starts on a host that is stopping");

    let (h, _) = h.restart(Duration::from_secs(5)).await;
    h.settle(run).await;
    assert_eq!(h.channel_texts("Researcher"), ["Are you there?", "Here."]);
    assert!(notices_of(&h, "Researcher", NoticeKind::Interrupted).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_recurring_check_that_found_nothing_ends_quiet_with_an_outcome_and_no_notice() {
    let stub = serve(|_| Script::Say(String::new())).await;
    let h = harness(&stub, &["Watcher"], GuardLimits::default());
    let routine = h
        .runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Listings sweep",
            "Check the listings and say what is new.",
            Trigger::Clock(Cadence::Daily),
            Some(now_ms() + 3_600_000),
            false,
        )
        .unwrap();
    let run = h.runtime.test_routine(&routine).unwrap();
    h.settle(run).await;

    let prompt = prompts_by_agent(&stub).remove("Watcher").expect("the Watcher ran");
    assert!(prompt.contains("This is a check that runs again"), "{prompt}");
    let outcome = h.runtime.store().outcome(run).unwrap().expect("a finished run has an outcome");
    assert_eq!(outcome.status, OutcomeStatus::NothingNew, "{}", h.transcript());
    assert_eq!(outcome.headline, "Ran and found nothing new.");
    assert!(
        !guard_stopped(&h, "Watcher"),
        "a check allowed to be silent was reported as stopping:\n{}",
        h.transcript()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_one_off_that_ends_in_silence_is_still_reported() {
    let stub = serve(|_| Script::Say(String::new())).await;
    let h = harness(&stub, &["Watcher"], GuardLimits::default());
    let routine = h
        .runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Renewal",
            "Renew the domain.",
            Trigger::Clock(Cadence::Once),
            Some(now_ms() + 3_600_000),
            false,
        )
        .unwrap();
    let run = h.runtime.test_routine(&routine).unwrap();
    h.settle(run).await;

    let prompt = prompts_by_agent(&stub).remove("Watcher").expect("the Watcher ran");
    assert!(prompt.contains("Saying nothing here is the one wrong answer"), "{prompt}");
    assert!(!prompt.contains("This is a check that runs again"), "{prompt}");
    assert!(guard_stopped(&h, "Watcher"), "the silence went unremarked:\n{}", h.transcript());
    let outcome = h.runtime.store().outcome(run).unwrap().expect("a finished run has an outcome");
    assert_eq!(outcome.status, OutcomeStatus::Done, "{}", h.transcript());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_operator_message_read_mid_turn_is_answered_under_the_prompt_for_it() {
    let hook: Arc<std::sync::OnceLock<(Runtime, guac_lib::domain::ids::AgentId)>> =
        Arc::new(std::sync::OnceLock::new());
    let armed = hook.clone();
    let rounds: Arc<parking_lot::Mutex<Vec<String>>> = Arc::default();
    let seen = rounds.clone();
    let stub = serve(move |body| {
        let system = body["messages"][0]["content"].as_str().unwrap_or_default().to_string();
        seen.lock().push(system);
        if anyone_said(body, "What did you find") {
            return Script::Say("Found two new listings.".into());
        }
        if let Some((runtime, watcher)) = armed.get() {
            runtime.send_from_human(*watcher, "What did you find?").unwrap();
        }
        Script::Progress("sweeping".into())
    })
    .await;
    let h = harness(&stub, &["Watcher"], GuardLimits::default());
    let _ = hook.set((h.runtime.clone(), h.id("Watcher")));
    let routine = h
        .runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Listings sweep",
            "Check the listings and say what is new.",
            Trigger::Clock(Cadence::Once),
            Some(now_ms() + 3_600_000),
            false,
        )
        .unwrap();
    let run = h.runtime.test_routine(&routine).unwrap();
    h.settle(run).await;

    let rounds = rounds.lock().clone();
    assert!(rounds.len() >= 2, "{rounds:?}");
    assert!(rounds[0].contains("You have been given something to do"), "{}", rounds[0]);
    let last = rounds.last().unwrap();
    assert!(
        last.contains("Your final message goes to the human operator"),
        "the round that read the operator's message still had the routine's prompt:\n{last}"
    );
    assert!(!last.contains("You have been given something to do"), "{last}");
    assert!(last.trim_end().ends_with("they come back."), "presence stays last:\n{last}");
    assert!(
        h.channel_texts("Watcher").iter().any(|t| t.contains("two new listings")),
        "{}",
        h.transcript()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_turn_that_reads_the_last_answer_closes_the_delegation_and_its_line_is_the_outcome() {
    let closing: Arc<parking_lot::Mutex<Option<String>>> = Arc::default();
    let kept = closing.clone();
    let stub = serve(move |body| {
        let who = speaker(body);
        let system = body["messages"][0]["content"].as_str().unwrap_or_default();
        if who == "Chef" {
            return Script::Say("Booked for 10.".into());
        }
        if reading_peer_replies(body) {
            if system.contains("this turn finishes it") {
                *kept.lock() = Some(system.to_string());
                return Script::Say(
                    "Chef booked the room for 10.\n\nChef confirmed it; nothing is still open."
                        .into(),
                );
            }
            return Script::Say(String::new());
        }
        if has_tool_result(body) {
            return Script::Say("Asked Chef to book the room.".into());
        }
        Script::Instruct { recipients: vec!["Chef".into()], text: "Book the room.".into() }
    })
    .await;
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Get the room booked.").unwrap();
    h.settle(run).await;

    let system = closing.lock().clone().unwrap_or_else(|| {
        panic!(
            "the turn reading Chef's answer was never told it closes the work:\n{}",
            h.transcript()
        )
    });
    assert!(system.contains("The work came from the operator."), "{system}");
    assert!(
        system.trim_end().ends_with("a question in your note waits until they come back."),
        "where the operator is comes last:\n{system}"
    );
    let outcome = h.runtime.store().outcome(run).unwrap().expect("a finished run has an outcome");
    assert_eq!(outcome.headline, "Chef booked the room for 10.", "{}", h.transcript());
    assert_eq!(outcome.status, OutcomeStatus::Done);
}

fn guard_stopped(h: &Harness, name: &str) -> bool {
    h.runtime
        .store()
        .channel_messages(h.id(name), 50)
        .unwrap()
        .into_iter()
        .flat_map(|e| e.parts)
        .any(|part| matches!(part, Part::Notice { kind: NoticeKind::GuardStop, .. }))
}

const LAPSED_PERMISSION: &str = "Nobody answered in ten minutes, so you do not have permission and must not act now. Your request is now on the operator's For you desk, and their answer will reach you as a new message. If they allow it, ask again for the same action and it is granted without waiting. Carry on with anything that does not depend on it, and say in your note that it is waiting on them.";
const LAPSED_QUESTION: &str = "Nobody answered in ten minutes, so you are on your own for this turn. Your question is now on the operator's For you desk, and their answer will reach you as a new message. Do not ask again this turn. Take the most defensible option, do as much of the work as that lets you, and in your note say what you asked, what you assumed instead, and what would change if the assumption is wrong.";
const LAPSED_SETTINGS: &str = "Nobody answered in ten minutes, so nothing changed. The request is now on the operator's For you desk with its before and after, and their answer will reach you as a new message. If they allow it, ask again for the same change and it is applied without waiting, unless one of those settings has changed since.";
const ENDED_BY_RESTART: &str = "The host restarted while this was waiting for the operator, so nobody decided anything. Ask again if it still needs doing.";

fn lapsed_create(name: &str) -> String {
    format!("Nobody answered the request to create {name} in ten minutes, so nothing was created. The request is now on the operator's For you desk, and their answer will reach you as a new message. If they allow it, ask again with the same details and it goes through without waiting. Finish what you can without it.")
}

fn filed_for(h: &Harness, name: &str) -> guac_lib::domain::decision::WorkDecision {
    let desk = h.runtime.store().decisions(Some(h.id(name))).unwrap();
    assert_eq!(desk.len(), 1, "one decision on the desk: {desk:?}");
    desk.into_iter().next().unwrap()
}

fn asked_count(h: &Harness) -> usize {
    h.sink.count_of(|e| matches!(e, UiEvent::ApprovalRequested { .. }))
}

async fn asks_every_turn(ask: Script) -> Stub {
    serve(move |body| {
        if anyone_said(body, "restarted partway") {
            Script::Say("I will ask again if it still needs doing.".into())
        } else if has_tool_result(body) {
            Script::Say("Noted.".into())
        } else {
            ask.clone()
        }
    })
    .await
}

fn weekly_report() -> Script {
    Script::AskPermission {
        action: "Email the weekly report to the board".into(),
        because: "It goes out every Friday.".into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_question_nobody_answers_in_time_moves_to_for_you_and_its_answer_reaches_the_agent() {
    let stub = serve(|body| {
        if anyone_said(body, "The operator answered decision") {
            Script::Say("Switching to Northwind, as you chose.".into())
        } else if has_tool_result(body) {
            Script::Say("Went with Contoso for now.".into())
        } else {
            Script::AskQuestion {
                question: "Both vendors clear the bar on price. Which do you want?".into(),
                options: vec!["Northwind".into(), "Contoso".into()],
            }
        }
    })
    .await;
    let h = harness(&stub, &["Analyst"], GuardLimits::default());
    h.runtime.set_approval_window(Duration::from_secs(1));
    let run = h.runtime.send_from_human(h.id("Analyst"), "Pick a vendor.").unwrap();
    let request = h.awaited_request().await;
    h.settle(run).await;

    assert_eq!(last_tool_results(&stub), [LAPSED_QUESTION], "{}", h.transcript());
    assert_eq!(
        h.runtime.store().get_approval(request).unwrap().unwrap().state,
        ApprovalState::Expired,
        "the card is no longer answerable"
    );
    let filed = filed_for(&h, "Analyst");
    assert_eq!(h.runtime.store().decision_origin(filed.id).unwrap(), Some(request));
    assert_eq!(filed.request.question, "Both vendors clear the bar on price. Which do you want?");
    assert_eq!(filed.request.options, ["Northwind", "Contoso"]);
    let outcome = h.runtime.store().outcome(run).unwrap().expect("an outcome");
    assert_eq!(outcome.status, OutcomeStatus::NeedsYou, "the run left something on the desk");

    let answered = h.runtime.answer_decision(filed.id, "Northwind", false, None).unwrap();
    h.settle(answered.delivery_run.expect("delivered")).await;
    assert_eq!(
        h.channel_texts("Analyst").last().unwrap(),
        "Switching to Northwind, as you chose.",
        "a late answer reaches the agent as a new message:\n{}",
        h.transcript()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lapsed_permission_allowed_later_lets_the_same_request_through_once() {
    let stub = asks_every_turn(weekly_report()).await;
    let h = harness_with_computer(&stub, &["Outreach"], GuardLimits::default());
    h.runtime.set_approval_window(Duration::from_secs(1));
    let run = h.runtime.send_from_human(h.id("Outreach"), "Send the weekly report.").unwrap();
    let request = h.awaited_request().await;
    h.settle(run).await;

    assert_eq!(last_tool_results(&stub), [LAPSED_PERMISSION], "{}", h.transcript());
    let filed = filed_for(&h, "Outreach");
    assert_eq!(h.runtime.store().decision_origin(filed.id).unwrap(), Some(request));
    assert_eq!(filed.request.options, ["Allow", "Deny"]);
    assert!(
        filed.request.context.contains("Email the weekly report to the board"),
        "the desk says what it would allow: {:?}",
        filed.request
    );

    h.runtime.set_approval_window(Duration::from_secs(60));
    let answered = h.runtime.answer_decision(filed.id, "Allow", false, None).unwrap();
    h.settle(answered.delivery_run.expect("delivered")).await;
    assert_eq!(asked_count(&h), 1, "the repeat went through without parking:\n{}", h.transcript());
    assert!(
        last_tool_results(&stub)[0].starts_with("The operator allowed it. Do it now"),
        "{:?}",
        last_tool_results(&stub)
    );
    assert!(
        stub.transcript
            .lock()
            .iter()
            .any(|body| anyone_said(body, "This allows that one action once, until ")),
        "the agent was told what the yes covers"
    );

    let again = h.runtime.send_from_human(h.id("Outreach"), "Send it again.").unwrap();
    h.wait_until("the second repeat to park", |h| asked_count(h) == 2).await;
    let parked = h
        .sink
        .snapshot()
        .into_iter()
        .filter_map(|event| match event {
            UiEvent::ApprovalRequested { approval_id, .. } if approval_id != request => {
                Some(approval_id)
            }
            _ => None,
        })
        .next()
        .unwrap();
    h.runtime.decide_approval(parked, Decision::Deny).unwrap();
    h.settle(again).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lapsed_request_to_create_an_agent_allowed_later_creates_it_without_asking() {
    let stub = asks_every_turn(Script::Hire {
        name: "Scout".into(),
        instructions: "Find venues.".into(),
        notes: String::new(),
    })
    .await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    h.runtime.set_approval_window(Duration::from_secs(1));
    let run = h.runtime.send_from_human(h.id("Manager"), "Hire a scout.").unwrap();
    h.awaited_request().await;
    h.settle(run).await;

    assert_eq!(last_tool_results(&stub), [lapsed_create("Scout")], "{}", h.transcript());
    assert!(h.agent_named("Scout").is_none(), "nothing was created");
    let filed = filed_for(&h, "Manager");

    h.runtime.set_approval_window(Duration::from_secs(60));
    let answered = h.runtime.answer_decision(filed.id, "Allow", false, None).unwrap();
    h.settle(answered.delivery_run.expect("delivered")).await;
    assert_eq!(asked_count(&h), 1, "{}", h.transcript());
    assert!(h.agent_named("Scout").is_some(), "the same request went through:\n{}", h.transcript());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lapsed_settings_change_allowed_later_applies_once_unless_a_setting_moved_since() {
    let was = GuardLimits::default().max_hops;
    for moved in [false, true] {
        let stub = asks_every_turn(Script::Settings(serde_json::json!({
            "action": "update", "changes": { "limits": { "maxHops": was + 3 } },
        })))
        .await;
        let h = harness(&stub, &["Helper"], GuardLimits::default());
        h.runtime.set_approval_window(Duration::from_secs(1));
        let run = h.runtime.send_from_human(h.id("Helper"), "Raise the relay depth.").unwrap();
        h.awaited_request().await;
        h.settle(run).await;
        assert_eq!(last_tool_results(&stub), [LAPSED_SETTINGS], "{}", h.transcript());
        let filed = filed_for(&h, "Helper");
        assert!(filed.request.context.contains(&format!("{was} → {}", was + 3)), "{filed:?}");

        if moved {
            let mut config = h.runtime.config();
            config.limits.max_hops = was + 1;
            h.runtime.set_config(config);
        }
        h.runtime.set_approval_window(Duration::from_secs(60));
        let answered = h.runtime.answer_decision(filed.id, "Allow", false, None).unwrap();
        let delivery = answered.delivery_run.expect("delivered");
        if moved {
            h.wait_until("the repeat to park", |h| asked_count(h) == 2).await;
            assert_eq!(h.runtime.config().limits.max_hops, was + 1, "nothing applied unasked");
            h.runtime.stop_run(delivery);
            h.settle(delivery).await;
        } else {
            h.settle(delivery).await;
            assert_eq!(asked_count(&h), 1, "{}", h.transcript());
            assert_eq!(h.runtime.config().limits.max_hops, was + 3);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_a_restart_ended_is_reported_as_ended_by_the_restart() {
    let stub = asks_every_turn(weekly_report()).await;
    let h = harness_with_computer(&stub, &["Outreach"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Outreach"), "Send the weekly report.").unwrap();
    let request = h.awaited_request().await;

    let (h, drained) = h.restart(Duration::from_secs(5)).await;
    assert!(drained.is_complete(), "{drained:?}");
    h.settle(run).await;

    let resumed = stub.transcript.lock().last().cloned().expect("picked up");
    let resumed = resumed.to_string();
    assert!(
        resumed.contains(&format!("refused: {ENDED_BY_RESTART}")),
        "the next host says the restart ended it: {resumed}"
    );
    assert!(!resumed.contains("nobody answered"), "nobody was asked to answer: {resumed}");
    assert_eq!(
        h.runtime.store().get_approval(request).unwrap().unwrap().state,
        ApprovalState::Expired
    );
    assert!(h.runtime.store().decisions(None).unwrap().is_empty(), "nothing waits on the desk");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_a_stop_ended_says_nobody_decided_and_files_nothing() {
    let stub = asks_every_turn(weekly_report()).await;
    let h = harness_with_computer(&stub, &["Outreach"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Outreach"), "Send the weekly report.").unwrap();
    h.awaited_request().await;
    h.runtime.stop_run(run);
    h.settle(run).await;

    let outcome = h
        .runtime
        .store()
        .channel_messages(h.id("Outreach"), 50)
        .unwrap()
        .into_iter()
        .flat_map(|e| e.parts)
        .find_map(|part| match part {
            Part::ToolCall { name, outcome, .. } if name == "request_permission" => Some(outcome),
            _ => None,
        })
        .expect("the call is on the record");
    assert_eq!(outcome, ToolOutcome::Refused { reason: ENDED_BY_RESTART.into() });
    assert!(h.runtime.store().decisions(None).unwrap().is_empty());
}

fn crash(h: Harness, paused: &[&str]) -> (Harness, Vec<RunId>, usize) {
    for id in h.ids.values() {
        h.runtime.stop_agent(*id);
    }
    let store = Store::open(&h._dir.path().join("guac.db")).unwrap();
    let recovered = guac_lib::boot::recover(&store).unwrap();
    let again = recovered.resumed.iter().map(|put| put.envelope.run_id).collect();
    let interrupted = recovered.interrupted;
    for (name, id) in &h.ids {
        let lifecycle =
            if paused.contains(&name.as_str()) { Lifecycle::Paused } else { Lifecycle::Active };
        store.set_lifecycle(*id, lifecycle).unwrap();
    }
    let sink = RecordingSink::new();
    let runtime = Runtime::new(
        store,
        LlmClient::new().unwrap(),
        h.runtime.config(),
        OnDisk::under(h._dir.path()),
        sink.clone(),
    );
    let started = runtime.start_all().unwrap();
    runtime.pick_up(&started, recovered);
    (Harness { runtime, sink, ids: h.ids, _dir: h._dir }, again, interrupted)
}

async fn hangs_until(crashed: Arc<std::sync::atomic::AtomicBool>, answer: &'static str) -> Stub {
    serve(
        move |_| {
            if crashed.load(Ordering::SeqCst) {
                Script::Say(answer.into())
            } else {
                Script::Hang
            }
        },
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_stand_in_model_has_chosen_its_answer_before_a_test_can_count_the_call() {
    let counter: Arc<std::sync::OnceLock<Arc<AtomicUsize>>> = Arc::default();
    let counted_when_chosen = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let stub = serve({
        let counter = counter.clone();
        let counted_when_chosen = counted_when_chosen.clone();
        move |_| {
            let calls = counter.get().expect("the counter is known before any call");
            counted_when_chosen.lock().push(calls.load(Ordering::SeqCst));
            Script::Say("Sent.".into())
        }
    })
    .await;
    counter.set(stub.calls.clone()).expect("set once");
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Researcher"), "Send the invoice.").unwrap();
    h.settle(run).await;

    let counted_when_chosen = counted_when_chosen.lock().clone();
    assert!(!counted_when_chosen.is_empty());
    assert_eq!(
        counted_when_chosen,
        (0..counted_when_chosen.len()).collect::<Vec<_>>(),
        "a call a test can see counted already has the answer it will get"
    );
}

async fn queued_behind_a_busy_turn(h: &Harness, stub: &Stub) -> (RunId, RunId) {
    let researcher = h.id("Researcher");
    let first = h.runtime.send_from_human(researcher, "The first thing.").unwrap();
    h.wait_until("the first turn to be in the air", |_| stub.calls.load(Ordering::SeqCst) >= 1)
        .await;
    let second = h.runtime.send_from_human(researcher, "The second thing.").unwrap();
    h.wait_until("the second to queue", |h| h.runtime.inbox_depth(researcher) == 1).await;
    (first, second)
}

fn interrupted_runs(h: &Harness, name: &str) -> Vec<RunId> {
    h.runtime
        .store()
        .channel_messages(h.id(name), 200)
        .unwrap()
        .into_iter()
        .filter(|m| {
            matches!(m.parts.first(), Some(Part::Notice { kind: NoticeKind::Interrupted, .. }))
        })
        .map(|m| m.run_id)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_queued_behind_a_busy_turn_is_answered_after_a_crash() {
    let crashed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stub = hangs_until(crashed.clone(), "Second handled.").await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let (first, second) = queued_behind_a_busy_turn(&h, &stub).await;

    crashed.store(true, Ordering::SeqCst);
    let (h, again, interrupted) = crash(h, &[]);
    assert_eq!(again, [second], "delivered again: no turn had started on it");
    assert_eq!(interrupted, 1);
    h.settle(second).await;

    assert_eq!(stub.calls.load(Ordering::SeqCst), 2, "answered once, by one call");
    let texts = h.channel_texts("Researcher");
    assert_eq!(texts.iter().filter(|text| *text == "The second thing.").count(), 1);
    assert_eq!(texts.iter().filter(|text| *text == "Second handled.").count(), 1);
    assert_eq!(interrupted_runs(&h, "Researcher"), [first], "only what a turn had begun");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_whose_turn_had_called_the_model_is_reported_not_delivered_again() {
    let crashed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stub = hangs_until(crashed.clone(), "Should not be asked.").await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Researcher"), "Send the invoice.").unwrap();
    h.wait_until("the call to be in the air", |_| stub.calls.load(Ordering::SeqCst) >= 1).await;

    crashed.store(true, Ordering::SeqCst);
    let (h, again, interrupted) = crash(h, &[]);
    assert!(again.is_empty(), "a started turn is never repeated on its own");
    assert_eq!(interrupted, 1);
    assert_eq!(h.runtime.live_runs(), 0, "nothing was handed to an agent");
    assert_eq!(stub.calls.load(Ordering::SeqCst), 1);
    assert_eq!(interrupted_runs(&h, "Researcher"), [run]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_two_crashes_in_a_row_find_unstarted_is_answered_once() {
    let crashed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stub = hangs_until(crashed.clone(), "Second handled.").await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let researcher = h.id("Researcher");
    let (first, second) = queued_behind_a_busy_turn(&h, &stub).await;

    let (h, again, interrupted) = crash(h, &["Researcher"]);
    assert_eq!(again, [second]);
    assert_eq!(interrupted, 1);
    h.wait_until("it to hold the message", |h| {
        h.sink.snapshot().iter().any(|event| {
            matches!(event, UiEvent::ActivityChanged { agent_id, activity: Activity::Paused }
                if *agent_id == researcher)
        })
    })
    .await;
    assert_eq!(stub.calls.load(Ordering::SeqCst), 1, "paused: nothing started on it");

    crashed.store(true, Ordering::SeqCst);
    let (h, again, interrupted) = crash(h, &[]);
    assert_eq!(again, [second], "still unstarted, so delivered again");
    assert_eq!(interrupted, 0, "the first was reported once, by the first crash");
    h.settle(second).await;

    assert_eq!(stub.calls.load(Ordering::SeqCst), 2, "answered once, by one call");
    let texts = h.channel_texts("Researcher");
    assert_eq!(texts.iter().filter(|text| *text == "The second thing.").count(), 1);
    assert_eq!(texts.iter().filter(|text| *text == "Second handled.").count(), 1);
    assert_eq!(interrupted_runs(&h, "Researcher"), [first]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_decision_answer_queued_behind_a_busy_turn_is_followed_through_after_a_crash() {
    use guac_lib::domain::decision::{DecisionRequest, DecisionStatus};

    let crashed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let after = crashed.clone();
    let stub = serve(move |body| {
        if !after.load(Ordering::SeqCst) {
            Script::Hang
        } else if anyone_said(body, "11 AM") {
            Script::Say("Booked for 11 AM.".into())
        } else {
            Script::Say("Nothing to do.".into())
        }
    })
    .await;
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
            now_ms(),
        )
        .unwrap();
    let busy = h.runtime.send_from_human(agent.id, "Tidy the calendar.").unwrap();
    h.wait_until("the busy turn to be in the air", |_| stub.calls.load(Ordering::SeqCst) >= 1)
        .await;
    let answered = h.runtime.answer_decision(decision.id, "11 AM", false, None).unwrap();
    let delivery = answered.delivery_run.unwrap();
    h.wait_until("the answer to queue", |h| h.runtime.inbox_depth(agent.id) == 1).await;

    crashed.store(true, Ordering::SeqCst);
    let (h, again, interrupted) = crash(h, &[]);
    assert_eq!(again, [delivery], "the answer is delivered again: nothing had read it");
    assert_eq!(interrupted, 1);
    let row = h.runtime.store().decisions(None).unwrap().remove(0);
    assert!(!row.interrupted, "follow-through on its way is not follow-through cut off");
    assert_eq!(row.delivery_run, Some(delivery));
    h.settle(delivery).await;

    assert_eq!(h.channel_texts("Assistant").last().unwrap(), "Booked for 11 AM.");
    assert_eq!(interrupted_runs(&h, "Assistant"), [busy], "nothing offers to send it again");
    assert!(
        h.runtime.answer_decision(decision.id, "", true, None).is_err(),
        "nothing to resume: it was followed through"
    );
    let row = h
        .runtime
        .store()
        .finish_decision(decision.id, agent.id, DecisionStatus::Completed, "Booked.", now_ms())
        .unwrap();
    assert_eq!(row.status, DecisionStatus::Completed);
}

const REPORTED: &str = "## What you have already reported";

fn answering(body: &serde_json::Value) -> String {
    body["messages"]
        .as_array()
        .and_then(|messages| messages.iter().rev().find(|m| m["role"] == "user"))
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string()
}

fn number_after(text: &str, word: &str) -> Option<u32> {
    let rest = &text[text.find(word)? + word.len()..];
    rest.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()
}

fn reported_lines(system: &str) -> Vec<String> {
    let Some(at) = system.find(REPORTED) else { return Vec::new() };
    let rest = &system[at + REPORTED.len()..];
    let body = rest.split("\n## ").next().unwrap_or_default();
    body.lines().filter(|line| line.starts_with("- ")).map(str::to_string).collect()
}

fn prompt_answering(stub: &Stub, name: &str, needle: &str) -> serde_json::Value {
    stub.transcript
        .lock()
        .iter()
        .rev()
        .find(|body| speaker(body) == name && answering(body).contains(needle))
        .cloned()
        .unwrap_or_else(|| panic!("{name} never answered {needle:?}"))
}

fn drawn(body: &serde_json::Value) -> Vec<String> {
    body["messages"].as_array().unwrap()[1..]
        .iter()
        .filter_map(|m| m["content"].as_str().map(str::to_string))
        .collect()
}

async fn reporter() -> Stub {
    serve(|body| {
        let asked = answering(body);
        match number_after(&asked, "Topic ") {
            Some(topic) => Script::Say(format!("{} reported on topic {topic}.", speaker(body))),
            None => Script::Say("Nothing else is open.".into()),
        }
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_is_shown_the_ten_newest_lines_it_ended_with_before_its_window() {
    let stub = reporter().await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    for topic in 1..=32 {
        let run = h.runtime.send_from_human(h.id("Manager"), &format!("Topic {topic}.")).unwrap();
        h.settle(run).await;
    }
    let last = h.runtime.send_from_human(h.id("Manager"), "Anything else?").unwrap();
    h.settle(last).await;

    for topic in 1..=15 {
        let body = prompt_answering(&stub, "Manager", &format!("Topic {topic}."));
        let system = body["messages"][0]["content"].as_str().unwrap();
        assert!(!system.contains(REPORTED), "topic {topic} saw the section:\n{system}");
    }

    let body = prompt_answering(&stub, "Manager", "Anything else?");
    let system = body["messages"][0]["content"].as_str().unwrap();
    let on_screen: Vec<u32> =
        drawn(&body).iter().filter_map(|turn| number_after(turn, "Topic ")).collect();
    let expected: Vec<String> = (1..=32)
        .rev()
        .filter(|topic| !on_screen.contains(topic))
        .take(10)
        .map(|topic| format!("Manager reported on topic {topic}."))
        .collect();
    let listed: Vec<String> = reported_lines(system)
        .iter()
        .map(|line| line.split_once(": ").expect("an age, then the line").1.to_string())
        .collect();
    assert!(on_screen.contains(&32), "the newest topic is in the window: {on_screen:?}");
    assert!(32 - on_screen.len() >= 12, "twelve or more fell out: {on_screen:?}");
    assert_eq!(listed, expected, "{system}");
    for line in reported_lines(system) {
        assert!(line.starts_with("- just now: ") || line.contains(" ago: "), "{line}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_coordinator_still_reads_the_line_that_started_a_long_delegation() {
    let stub = serve(|body| {
        let asked = answering(body);
        let step = number_after(&asked, "step ");
        if speaker(body) == "Chef" {
            return Script::Say(match step {
                Some(step) => format!("Done with step {step}."),
                None => "The launch menu is written.".into(),
            });
        }
        if reading_peer_replies(body) {
            return Script::Say(match step {
                Some(step) => format!("Step {step} of the launch is finished."),
                None => "Launch planned: Chef wrote the menu and owns the steps.".into(),
            });
        }
        if has_tool_result(body) {
            return Script::Say("Asked Chef.".into());
        }
        Script::Instruct {
            recipients: vec!["Chef".into()],
            text: match step {
                Some(step) => format!("Please do step {step} of the launch."),
                None => "Write the launch menu.".into(),
            },
        }
    })
    .await;
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let manager = h.id("Manager");
    let first = h.runtime.send_from_human(manager, "Plan the launch with Chef.").unwrap();
    h.settle(first).await;
    let started = h.runtime.store().outcome(first).unwrap().expect("the first run ended");
    assert_eq!(started.headline, "Launch planned: Chef wrote the menu and owns the steps.");

    let before = h.runtime.store().agent_history(manager, i64::MAX, 1_000).unwrap().len();
    let mut step = 0;
    while h.runtime.store().agent_history(manager, i64::MAX, 1_000).unwrap().len() < before + 50 {
        step += 1;
        let run = h.runtime.send_from_human(manager, &format!("Next, step {step}.")).unwrap();
        h.settle(run).await;
    }
    let last = h.runtime.send_from_human(manager, "How is the launch going?").unwrap();
    h.settle(last).await;

    let body = prompt_answering(&stub, "Manager", "How is the launch going?");
    let system = body["messages"][0]["content"].as_str().unwrap();
    assert!(
        !drawn(&body).iter().any(|turn| turn.contains("Plan the launch")),
        "the brief is still on screen, so this proves nothing"
    );
    assert!(
        reported_lines(system)
            .iter()
            .any(|line| line.ends_with(": Launch planned: Chef wrote the menu and owns the steps.")),
        "{system}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_is_shown_only_its_own_lines_and_never_another_groups() {
    let stub = reporter().await;
    let h = harness_in_groups(
        &stub,
        &[("Manager", None), ("Chef", None), ("Scout", Some("Field"))],
        GuardLimits::default(),
    );
    for topic in 1..=24 {
        for name in ["Manager", "Chef", "Scout"] {
            let run = h.runtime.send_from_human(h.id(name), &format!("Topic {topic}.")).unwrap();
            h.settle(run).await;
        }
    }
    for name in ["Manager", "Chef", "Scout"] {
        let run = h.runtime.send_from_human(h.id(name), "Anything else?").unwrap();
        h.settle(run).await;
        let body = prompt_answering(&stub, name, "Anything else?");
        let lines = reported_lines(body["messages"][0]["content"].as_str().unwrap());
        assert!(!lines.is_empty(), "{name} has lines older than its window");
        for line in &lines {
            assert!(line.contains(&format!(": {name} reported on topic ")), "{name}: {line}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cleared_channel_takes_what_was_reported_in_it_along() {
    let stub = reporter().await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let manager = h.id("Manager");
    for topic in 1..=24 {
        let run = h.runtime.send_from_human(manager, &format!("Topic {topic}.")).unwrap();
        h.settle(run).await;
    }
    let shown = h.runtime.send_from_human(manager, "Anything else?").unwrap();
    h.settle(shown).await;
    let body = prompt_answering(&stub, "Manager", "Anything else?");
    assert!(!reported_lines(body["messages"][0]["content"].as_str().unwrap()).is_empty());

    h.runtime.store().delete_channel_messages(manager).unwrap();
    let fresh = h.runtime.send_from_human(manager, "Anything new?").unwrap();
    h.settle(fresh).await;
    let body = prompt_answering(&stub, "Manager", "Anything new?");
    let system = body["messages"][0]["content"].as_str().unwrap();
    assert!(!system.contains(REPORTED), "{system}");
}

fn tidy_overnight(h: &Harness, name: &str) {
    let store = h.runtime.store();
    let agent = store.get_agent(h.id(name)).unwrap().unwrap();
    let group = store.get_group(agent.group_id).unwrap().unwrap();
    store
        .update_group(
            group.id,
            &CleanGroup {
                name: group.name,
                tidy: Some(Tidy { on: true, hour: 3 }),
                ..Default::default()
            },
        )
        .unwrap();
}

fn night(nights: i64) -> i64 {
    Tidy::last_struck(3, now_ms() + nights * 86_400_000).unwrap() + 60_000
}

fn tidies(h: &Harness, name: &str) -> Vec<Envelope> {
    let channel = h.runtime.store().channel_messages(h.id(name), 500).unwrap();
    channel.into_iter().filter(Envelope::is_tidy).collect()
}

fn last_handed(body: &serde_json::Value) -> String {
    body["messages"]
        .as_array()
        .and_then(|messages| messages.iter().rev().find(|m| m["role"] == "user"))
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string()
}

fn a_tidy(body: &serde_json::Value) -> bool {
    last_handed(body).starts_with(&format!("[SYSTEM]\n{TIDY}"))
}

fn offered(body: &serde_json::Value) -> Vec<String> {
    let mut names: Vec<String> = body["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_string))
        .collect();
    names.sort();
    names
}

fn system_of(body: &serde_json::Value) -> String {
    body["messages"][0]["content"].as_str().unwrap_or_default().to_string()
}

async fn sweep(h: &Harness, name: &str, at: i64) -> Option<Envelope> {
    let before = tidies(h, name).len();
    h.runtime.sweep_tidy(at);
    let handed = tidies(h, name);
    if handed.len() == before {
        return None;
    }
    let tidy = handed.last().cloned().unwrap();
    h.settle(tidy.run_id).await;
    Some(tidy)
}

fn outbox(h: &Harness) -> std::collections::BTreeSet<String> {
    let store = h.runtime.store();
    let due = store.due_deliveries(i64::MAX).unwrap().into_iter().map(|row| row.id);
    due.chain(store.pending_deliveries(None).unwrap().into_iter().map(|row| row.id)).collect()
}

fn unseen(h: &Harness) -> usize {
    h.runtime.store().outcomes(None, None, true, 500).unwrap().len()
}

fn warnings() -> String {
    struct Into(Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Into {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    static LOG: std::sync::OnceLock<Arc<std::sync::Mutex<Vec<u8>>>> = std::sync::OnceLock::new();
    let log = LOG.get_or_init(|| {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .with_writer(move || Into(sink.clone()))
            .finish();
        tracing::subscriber::set_global_default(subscriber).expect("nothing else here logs");
        log
    });
    String::from_utf8_lossy(&log.lock().unwrap()).into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_opted_in_agent_is_tidied_once_at_the_hour_on_what_its_work_ended_with() {
    let stub = serve(|body| {
        if a_tidy(body) {
            Script::Say(String::new())
        } else if has_tool_result(body) {
            Script::Say("Booked the venue.".into())
        } else {
            Script::Progress("Waiting on the caterer's quote.".into())
        }
    })
    .await;
    let h = harness(&stub, &["Planner"], GuardLimits::default());
    let planner = h.id("Planner");
    tidy_overnight(&h, "Planner");

    let run = h.runtime.send_from_human(planner, "Book the venue.").unwrap();
    h.settle(run).await;
    let ended = h.runtime.store().outcome(run).unwrap().expect("the run finished");
    let notes = h.runtime.store().working_notes(planner).unwrap();
    let normal = stub.transcript.lock().last().cloned().unwrap();

    let at = night(1);
    let tidy = sweep(&h, "Planner", at).await.expect("work ended since the switch went on");
    let asked = stub.transcript.lock().last().cloned().unwrap();
    assert!(a_tidy(&asked));
    assert_eq!(
        last_handed(&asked),
        format!(
            "[SYSTEM]\n{TIDY}\n\n{}\n\n{}",
            prompt::reported_line(ended.finished_at, &ended.headline, ended.status, at),
            prompt::note_line(&notes[0], at).trim_end(),
        ),
        "the tidy, then what the work ended with, then the working notes"
    );
    assert_eq!(offered(&asked), vec!["note_progress", "notebook", "update_memory"]);
    for heading in ["## Your reply", "## Where the operator is"] {
        assert!(system_of(&normal).contains(heading), "a normal turn reads {heading}");
        assert!(!system_of(&asked).contains(heading), "a tidy does not read {heading}");
    }

    assert_eq!(sweep(&h, "Planner", at + 3_600_000).await, None);
    assert_eq!(tidies(&h, "Planner").len(), 1);
    assert_eq!(h.runtime.store().outcome(tidy.run_id).unwrap(), None);
    assert!(h.runtime.store().agent_routines(planner).unwrap().is_empty());
    assert_eq!(sweep(&h, "Planner", night(2)).await, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nobody_is_tidied_without_the_switch_or_without_work_since() {
    let stub = serve(|body| {
        assert!(!a_tidy(body), "no tidy was owed:\n{}", last_handed(body));
        Script::Say("Done.".into())
    })
    .await;
    let h = harness_in_groups(
        &stub,
        &[("Idle", Some("Kitchen")), ("Busy", Some("Office"))],
        GuardLimits::default(),
    );
    tidy_overnight(&h, "Idle");
    let run = h.runtime.send_from_human(h.id("Busy"), "File the receipts.").unwrap();
    h.settle(run).await;
    let calls = stub.calls.load(Ordering::SeqCst);

    assert_eq!(sweep(&h, "Idle", night(1)).await, None);
    assert_eq!(sweep(&h, "Busy", night(1)).await, None);
    assert_eq!(stub.calls.load(Ordering::SeqCst), calls, "a tidy owed nothing costs nothing");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tidy_that_names_a_tool_it_was_not_offered_ends_there() {
    let _ = warnings();
    let stub = serve(|body| {
        if a_tidy(body) {
            Script::SendTo { recipients: vec!["Chef".into()], text: "Morning.".into() }
        } else {
            Script::Say("Done.".into())
        }
    })
    .await;
    let h = harness(&stub, &["Strayer", "Chef"], GuardLimits::default());
    tidy_overnight(&h, "Strayer");
    let run = h.runtime.send_from_human(h.id("Strayer"), "Plan the menu.").unwrap();
    h.settle(run).await;

    let tidy = sweep(&h, "Strayer", night(1)).await.expect("owed a tidy");
    let tidy_calls = stub.transcript.lock().iter().filter(|body| a_tidy(body)).count();
    assert_eq!(tidy_calls, 1, "not answered, so not asked again");
    assert!(h.said_to_peers("Strayer").is_empty(), "{}", h.transcript());
    assert!(h.channel_texts("Chef").is_empty(), "{}", h.transcript());
    let filed = h.runtime.store().channel_messages(h.id("Strayer"), 500).unwrap();
    assert!(
        !filed.iter().flat_map(|m| &m.parts).any(|part| matches!(part, Part::ToolCall { .. })),
        "a call that was not run is not filed as one"
    );
    assert_eq!(h.runtime.store().outcome(tidy.run_id).unwrap(), None);
    let logged = warnings();
    assert!(
        logged.lines().any(|line| line.contains("a tidy named a tool it is not offered")
            && line.contains("Strayer")
            && line.contains("send_message")),
        "{logged}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_operator_save_during_a_tidy_is_not_overwritten() {
    let workspace = Arc::new(std::sync::OnceLock::new());
    let saving = workspace.clone();
    let stub = serve(move |body| {
        if !a_tidy(body) {
            return Script::Say("Done.".into());
        }
        if has_tool_result(body) {
            return Script::Say(String::new());
        }
        let (workspace, id) = saving.get().expect("the harness is built before the turn runs");
        operator_saves(workspace, *id);
        Script::Memory("Smith handles verification.\nThe venue is booked.".into())
    })
    .await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let manager = h.id("Manager");
    tidy_overnight(&h, "Manager");
    h.runtime.workspace().write(manager, "Manager", "Smith handles verification.").unwrap();
    workspace.set((h.runtime.workspace().clone(), manager)).unwrap();
    let run = h.runtime.send_from_human(manager, "Book the venue.").unwrap();
    h.settle(run).await;

    sweep(&h, "Manager", night(1)).await.expect("owed a tidy");
    assert_eq!(
        h.runtime.workspace().read(manager),
        "Jones handles verification.",
        "the operator's correction was overwritten by a tidy"
    );
    let told = stub.transcript.lock().last().map(answers_so_far).unwrap_or_default();
    assert_eq!(told, vec![format!("{MEMORY_CONFLICT}Jones handles verification.")]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tidy_records_nothing_reaches_nobody_and_is_not_read_again() {
    let stub = serve(|body| {
        if a_tidy(body) {
            Script::Say("All tidy, nothing to change.".into())
        } else {
            Script::Say("Booked the venue.".into())
        }
    })
    .await;
    let h = harness(&stub, &["Planner"], GuardLimits::default());
    tidy_overnight(&h, "Planner");
    let run = h.runtime.send_from_human(h.id("Planner"), "Book the venue.").unwrap();
    h.settle(run).await;
    let (outbox_before, unseen_before) = (outbox(&h), unseen(&h));
    assert!(!outbox_before.is_empty(), "a finished run is in the outbox, so its absence is news");

    let tidy = sweep(&h, "Planner", night(1)).await.expect("owed a tidy");
    assert_eq!(h.runtime.store().outcome(tidy.run_id).unwrap(), None);
    assert!(outbox(&h).is_subset(&outbox_before), "a tidy put something in the outbox");
    assert_eq!(unseen(&h), unseen_before, "a tidy is counted as unseen work");
    assert!(
        !h.transcript().contains("All tidy"),
        "what a tidy says is filed nowhere:\n{}",
        h.transcript()
    );

    let next = h.runtime.send_from_human(h.id("Planner"), "And the caterer?").unwrap();
    h.settle(next).await;
    let body = stub.transcript.lock().last().cloned().unwrap();
    assert!(!a_tidy(&body));
    let read = serde_json::to_string(&body["messages"]).unwrap();
    for gone in ["Time to tidy your memory", "All tidy"] {
        assert!(!read.contains(gone), "{gone} came back in a later turn");
    }
    let card = h.runtime.store().get_agent(h.id("Planner")).unwrap().unwrap();
    let reported = h.runtime.store().reported(card.id, card.group_id, i64::MAX, 50).unwrap();
    let mut runs: Vec<_> = reported.iter().map(|one| one.run_id).collect();
    runs.sort();
    let mut expected = vec![run, next];
    expected.sort();
    assert_eq!(runs, expected, "{reported:?}");
    assert!(!reported.iter().any(|one| one.run_id == tidy.run_id));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tidy_that_rewrites_memory_files_the_call_and_nothing_else() {
    let stub = serve(|body| match (a_tidy(body), has_tool_result(body)) {
        (true, false) => Script::Memory("The venue is the Hall.".into()),
        (true, true) => Script::Say(String::new()),
        _ => Script::Say("Booked the venue.".into()),
    })
    .await;
    let h = harness(&stub, &["Planner"], GuardLimits::default());
    tidy_overnight(&h, "Planner");
    let run = h.runtime.send_from_human(h.id("Planner"), "Book the venue.").unwrap();
    h.settle(run).await;
    let (outbox_before, unseen_before) = (outbox(&h), unseen(&h));

    let tidy = sweep(&h, "Planner", night(1)).await.expect("owed a tidy");
    assert_eq!(h.runtime.workspace().read(h.id("Planner")), "The venue is the Hall.");
    let channel = h.runtime.store().channel_messages(h.id("Planner"), 500).unwrap();
    let at = channel.iter().position(|m| m.id == tidy.id).unwrap();
    let record = &channel[at + 1..];
    assert_eq!(record.len(), 1, "{}", h.transcript());
    assert!(matches!(record[0].from, Participant::Agent { .. }));
    assert!(matches!(record[0].to, Participant::System));
    assert!(
        matches!(&record[0].parts[..], [Part::ToolCall { name, outcome: ToolOutcome::Ok { .. }, .. }]
            if name == "update_memory"),
        "the rewrite, under the tidy line, as the transcript draws any: {:?}",
        record[0].parts
    );
    assert_eq!(h.runtime.store().outcome(tidy.run_id).unwrap(), None);
    assert!(outbox(&h).is_subset(&outbox_before));
    assert_eq!(unseen(&h), unseen_before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_tidy_says_so_reaches_nobody_and_is_owed_again_and_retried_as_a_tidy() {
    let healthy = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let well = healthy.clone();
    let stub = serve(move |body| {
        if !a_tidy(body) {
            Script::Say("Booked the venue.".into())
        } else if well.load(Ordering::SeqCst) {
            Script::Say(String::new())
        } else {
            Script::Unavailable
        }
    })
    .await;
    let h = harness(&stub, &["Planner"], GuardLimits::default());
    let planner = h.id("Planner");
    tidy_overnight(&h, "Planner");
    let run = h.runtime.send_from_human(planner, "Book the venue.").unwrap();
    h.settle(run).await;
    let (outbox_before, unseen_before) = (outbox(&h), unseen(&h));

    let failed = sweep(&h, "Planner", night(1)).await.expect("owed a tidy");
    let channel = h.runtime.store().channel_messages(planner, 500).unwrap();
    let notice = channel
        .iter()
        .find(|m| {
            m.parts.iter().any(|part| {
                matches!(
                    part,
                    Part::Notice {
                        kind: guac_lib::domain::envelope::NoticeKind::UpstreamError,
                        ..
                    }
                )
            })
        })
        .expect("the failure is said, as it is for any turn");
    assert_eq!(notice.cause, Some(failed.id), "and offers to try the tidy again");
    assert_eq!(h.runtime.store().outcome(failed.run_id).unwrap(), None);
    assert!(outbox(&h).is_subset(&outbox_before), "a failed tidy reached the outbox");
    assert_eq!(unseen(&h), unseen_before, "a failed tidy is counted as unseen");

    healthy.store(true, Ordering::SeqCst);
    let again = h.runtime.retry_turn(planner, failed.id).unwrap();
    h.settle(again).await;
    let retried = tidies(&h, "Planner");
    assert_eq!(retried.len(), 2);
    assert_eq!(retried[1].parts, failed.parts, "it reads what the failed one would have");
    assert_eq!(retried[1].cause, None);
    let asked = stub.transcript.lock().last().cloned().unwrap();
    assert!(a_tidy(&asked));
    assert_eq!(offered(&asked), vec!["note_progress", "notebook", "update_memory"]);
    for heading in ["## Your reply", "## Where the operator is"] {
        assert!(!system_of(&asked).contains(heading), "a retried tidy reads {heading}");
    }
    assert_eq!(h.runtime.store().outcome(again).unwrap(), None);
    assert!(outbox(&h).is_subset(&outbox_before));
    assert_eq!(unseen(&h), unseen_before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tidy_that_never_read_its_lines_is_handed_them_the_next_night() {
    let stub = serve(|body| {
        if a_tidy(body) {
            Script::Unavailable
        } else {
            Script::Say("Booked the venue.".into())
        }
    })
    .await;
    let h = harness(&stub, &["Planner"], GuardLimits::default());
    tidy_overnight(&h, "Planner");
    let run = h.runtime.send_from_human(h.id("Planner"), "Book the venue.").unwrap();
    h.settle(run).await;

    let first = sweep(&h, "Planner", night(1)).await.expect("owed a tidy");
    assert_eq!(sweep(&h, "Planner", night(1) + 60_000).await, None, "not tried again that night");
    let second = sweep(&h, "Planner", night(2)).await.expect("its lines were never read");
    let lines = |tidy: &Envelope| match &tidy.parts[0] {
        Part::Tidy { reported, .. } => reported.len(),
        _ => unreachable!(),
    };
    assert_eq!((lines(&first), lines(&second)), (1, 1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tidy_that_stored_its_rewrite_before_failing_has_read_its_lines() {
    let stub = serve(|body| match (a_tidy(body), has_tool_result(body)) {
        (true, false) => Script::Memory("The venue is the Hall.".into()),
        (true, true) => Script::Unavailable,
        _ => Script::Say("Booked the venue.".into()),
    })
    .await;
    let h = harness(&stub, &["Planner"], GuardLimits::default());
    tidy_overnight(&h, "Planner");
    let run = h.runtime.send_from_human(h.id("Planner"), "Book the venue.").unwrap();
    h.settle(run).await;

    sweep(&h, "Planner", night(1)).await.expect("owed a tidy");
    assert_eq!(h.runtime.workspace().read(h.id("Planner")), "The venue is the Hall.");
    assert_eq!(sweep(&h, "Planner", night(2)).await, None);
}

fn over_the_limit(limit: usize, after: usize) -> String {
    format!(
        "Not saved: a tidy may not leave your memory page longer than {limit} characters, and \
         your rewrite is {after}. Cut what is no longer needed, or move detail to your notebook, \
         then save again."
    )
}

async fn tidy_writing(
    page: &str,
    first: String,
    then: Option<String>,
) -> (Stub, Harness, guac_lib::domain::ids::AgentId) {
    let stub = serve(move |body| {
        if !a_tidy(body) {
            return Script::Say("Booked the venue.".into());
        }
        match (answers_so_far(body).len(), &then) {
            (0, _) => Script::Memory(first.clone()),
            (1, Some(then)) => Script::Memory(then.clone()),
            _ => Script::Say(String::new()),
        }
    })
    .await;
    let h = harness(&stub, &["Planner"], GuardLimits::default());
    let planner = h.id("Planner");
    tidy_overnight(&h, "Planner");
    h.runtime.workspace().write(planner, "Planner", page).unwrap();
    let run = h.runtime.send_from_human(planner, "Book the venue.").unwrap();
    h.settle(run).await;
    (stub, h, planner)
}

fn told(stub: &Stub) -> Vec<String> {
    stub.transcript.lock().last().map(answers_so_far).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tidy_may_grow_a_short_page_up_to_two_thousand_characters_and_no_further() {
    let (stub, h, planner) =
        tidy_writing("The venue is the Hall.", "a".repeat(2001), Some("b".repeat(2000))).await;

    sweep(&h, "Planner", night(1)).await.expect("owed a tidy");
    assert_eq!(
        told(&stub),
        vec![over_the_limit(2000, 2001), "Memory saved (2000 characters).".to_string()]
    );
    assert_eq!(h.runtime.workspace().read(planner), "b".repeat(2000));
    assert_eq!(
        sweep(&h, "Planner", night(2)).await,
        None,
        "it stored a page, so it has read its lines"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tidy_may_not_grow_a_page_already_over_the_limit() {
    let (stub, h, planner) =
        tidy_writing(&"c".repeat(2500), "c".repeat(2501), Some("d".repeat(2500))).await;

    sweep(&h, "Planner", night(1)).await.expect("owed a tidy");
    assert_eq!(
        told(&stub),
        vec![over_the_limit(2500, 2501), "Memory saved (2500 characters).".to_string()]
    );
    assert_eq!(h.runtime.workspace().read(planner), "d".repeat(2500));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tidy_whose_last_save_was_refused_is_owed_its_lines_again() {
    let (stub, h, planner) = tidy_writing(&"c".repeat(2500), "c".repeat(2600), None).await;

    let first = sweep(&h, "Planner", night(1)).await.expect("owed a tidy");
    assert_eq!(told(&stub), vec![over_the_limit(2500, 2600)]);
    assert_eq!(h.runtime.workspace().read(planner), "c".repeat(2500));
    assert_eq!(sweep(&h, "Planner", night(1) + 60_000).await, None, "not again that night");
    let second = sweep(&h, "Planner", night(2)).await.expect("its lines were never tidied");
    let lines = |tidy: &Envelope| match &tidy.parts[0] {
        Part::Tidy { reported, .. } => reported.len(),
        _ => unreachable!(),
    };
    assert_eq!((lines(&first), lines(&second)), (1, 1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_normal_turn_may_make_the_page_as_long_as_it_likes() {
    let stub = serve(|body| match answers_so_far(body).len() {
        0 => Script::Memory("e".repeat(5000)),
        _ => Script::Say("Noted.".into()),
    })
    .await;
    let h = harness(&stub, &["Planner"], GuardLimits::default());
    let planner = h.id("Planner");
    tidy_overnight(&h, "Planner");
    h.runtime.workspace().write(planner, "Planner", &"c".repeat(2500)).unwrap();
    let run = h.runtime.send_from_human(planner, "Remember all of this.").unwrap();
    h.settle(run).await;

    assert_eq!(told(&stub), vec!["Memory saved (5000 characters).".to_string()]);
    assert_eq!(h.runtime.workspace().read(planner), "e".repeat(5000));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_operator_save_during_a_tidy_is_answered_as_a_conflict_not_as_the_limit() {
    let workspace = Arc::new(std::sync::OnceLock::new());
    let saving = workspace.clone();
    let stub = serve(move |body| {
        if !a_tidy(body) {
            return Script::Say("Booked the venue.".into());
        }
        if has_tool_result(body) {
            return Script::Say(String::new());
        }
        let (workspace, id) = saving.get().expect("the harness is built before the tidy runs");
        operator_saves(workspace, *id);
        Script::Memory("f".repeat(2001))
    })
    .await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let manager = h.id("Manager");
    tidy_overnight(&h, "Manager");
    h.runtime.workspace().write(manager, "Manager", "Smith handles verification.").unwrap();
    workspace.set((h.runtime.workspace().clone(), manager)).unwrap();
    let run = h.runtime.send_from_human(manager, "Book the venue.").unwrap();
    h.settle(run).await;

    sweep(&h, "Manager", night(1)).await.expect("owed a tidy");
    assert_eq!(told(&stub), vec![format!("{MEMORY_CONFLICT}Jones handles verification.")]);
    assert_eq!(h.runtime.workspace().read(manager), "Jones handles verification.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tidy_the_operator_stops_before_it_stores_anything_is_owed_again() {
    let stub =
        serve(|body| if a_tidy(body) { Script::Hang } else { Script::Say("Done.".into()) }).await;
    let h = harness(&stub, &["Planner"], GuardLimits::default());
    tidy_overnight(&h, "Planner");
    let run = h.runtime.send_from_human(h.id("Planner"), "Book the venue.").unwrap();
    h.settle(run).await;

    let before = tidies(&h, "Planner").len();
    h.runtime.sweep_tidy(night(1));
    let tidy = tidies(&h, "Planner").pop().unwrap();
    assert_eq!(before, 0);
    h.wait_until("the tidy's call to be in the air", |_| stub.transcript.lock().iter().any(a_tidy))
        .await;
    assert!(h.runtime.stop_run(tidy.run_id));
    h.settle(tidy.run_id).await;
    assert_eq!(h.runtime.store().outcome(tidy.run_id).unwrap(), None);
    assert_eq!(sweep(&h, "Planner", night(1) + 60_000).await, None, "not again that night");
    h.runtime.sweep_tidy(night(2));
    let owed = tidies(&h, "Planner");
    assert_eq!(owed.len(), 2, "its lines were never read, so the next night has them");
    let lines = |tidy: &Envelope| match &tidy.parts[0] {
        Part::Tidy { reported, .. } => reported.len(),
        _ => unreachable!(),
    };
    assert_eq!((lines(&tidy), lines(&owed[1])), (1, 1), "aged afresh, and the same line");
    h.runtime.stop_run(owed[1].run_id);
    h.settle(owed[1].run_id).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_host_that_was_off_for_three_nights_tidies_once() {
    let stub =
        serve(
            |body| {
                if a_tidy(body) {
                    Script::Say(String::new())
                } else {
                    Script::Say("Done.".into())
                }
            },
        )
        .await;
    let h = harness(&stub, &["Planner"], GuardLimits::default());
    tidy_overnight(&h, "Planner");
    let run = h.runtime.send_from_human(h.id("Planner"), "Book the venue.").unwrap();
    h.settle(run).await;

    assert!(sweep(&h, "Planner", night(3)).await.is_some());
    assert_eq!(sweep(&h, "Planner", night(3) + 3_600_000).await, None);
    assert_eq!(tidies(&h, "Planner").len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_at_work_at_the_hour_is_tidied_when_it_is_free() {
    let stub = serve(|body| {
        if a_tidy(body) {
            Script::Say(String::new())
        } else if last_handed(body).contains("Book the venue.") {
            Script::Say("Booked.".into())
        } else {
            Script::Hang
        }
    })
    .await;
    let h = harness(&stub, &["Planner"], GuardLimits::default());
    tidy_overnight(&h, "Planner");
    let first = h.runtime.send_from_human(h.id("Planner"), "Book the venue.").unwrap();
    h.settle(first).await;
    let working = h.runtime.send_from_human(h.id("Planner"), "Find a caterer.").unwrap();
    h.wait_until("the turn to be in the air", |_| stub.calls.load(Ordering::SeqCst) >= 2).await;

    assert_eq!(sweep(&h, "Planner", night(1)).await, None, "a working agent is not interrupted");
    h.runtime.stop_run(working);
    h.settle(working).await;
    assert!(sweep(&h, "Planner", night(1) + 60_000).await.is_some(), "free, so tidied that night");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_agent_is_tidied_once_when_it_is_resumed() {
    let stub =
        serve(
            |body| {
                if a_tidy(body) {
                    Script::Say(String::new())
                } else {
                    Script::Say("Done.".into())
                }
            },
        )
        .await;
    let h = harness(&stub, &["Planner"], GuardLimits::default());
    tidy_overnight(&h, "Planner");
    let run = h.runtime.send_from_human(h.id("Planner"), "Book the venue.").unwrap();
    h.settle(run).await;

    h.pause("Planner");
    assert_eq!(sweep(&h, "Planner", night(1)).await, None);
    assert_eq!(sweep(&h, "Planner", night(2)).await, None);
    h.resume("Planner");
    assert!(sweep(&h, "Planner", night(2) + 60_000).await.is_some());
    assert_eq!(sweep(&h, "Planner", night(2) + 120_000).await, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restart_the_same_night_does_not_tidy_again() {
    let stub =
        serve(
            |body| {
                if a_tidy(body) {
                    Script::Say(String::new())
                } else {
                    Script::Say("Done.".into())
                }
            },
        )
        .await;
    let h = harness(&stub, &["Planner"], GuardLimits::default());
    tidy_overnight(&h, "Planner");
    let run = h.runtime.send_from_human(h.id("Planner"), "Book the venue.").unwrap();
    h.settle(run).await;
    assert!(sweep(&h, "Planner", night(1)).await.is_some());

    let (h, drained) = h.restart(Duration::from_secs(5)).await;
    assert!(drained.is_complete(), "{drained:?}");
    assert_eq!(sweep(&h, "Planner", night(1) + 60_000).await, None);
    let asked = stub.transcript.lock().iter().filter(|body| a_tidy(body)).count();
    assert_eq!(asked, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_a_turn_hands_back_a_message_it_took_in_and_never_acted_on() {
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let stub = {
        let (gate, calls) = (gate.clone(), calls.clone());
        serve(move |_| match calls.fetch_add(1, Ordering::SeqCst) {
            0 => {
                hold_until_open(
                    &gate,
                    "stopping_a_turn_hands_back_a_message_it_took_in_and_never_acted_on",
                );
                Script::Progress("Looking up the vendor.".into())
            }
            1 => Script::Hang,
            _ => Script::Say("Their number is 555-0100.".into()),
        })
        .await
    };
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let (first, second) = taken_in_mid_turn(&h, &stub, &gate, &calls).await;

    assert!(h.runtime.stop_run(first));
    h.settle(first).await;
    h.settle(second).await;

    let answered =
        h.runtime.store().channel_messages(h.id("Researcher"), 200).unwrap().into_iter().any(|m| {
            m.run_id == second && m.to == Participant::Human && m.plain_text().contains("555-0100")
        });
    assert!(answered, "the second conversation was never answered:\n{}", h.transcript());
    let outcome =
        h.runtime.store().outcome(second).unwrap().expect("the second run has an outcome");
    assert_eq!(outcome.status, OutcomeStatus::Done, "{}", h.transcript());
    assert_eq!(h.runtime.store().outcome(first).unwrap().unwrap().status, OutcomeStatus::Stopped);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_a_turn_tells_a_message_it_acted_on_at_once_and_settles_it() {
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let stub = {
        let (gate, calls) = (gate.clone(), calls.clone());
        serve(move |_| match calls.fetch_add(1, Ordering::SeqCst) {
            0 => {
                hold_until_open(
                    &gate,
                    "stopping_a_turn_tells_a_message_it_acted_on_at_once_and_settles_it",
                );
                Script::Progress("Looking up the vendor.".into())
            }
            1 => Script::Progress("Looking up the number.".into()),
            _ => Script::Hang,
        })
        .await
    };
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let researcher = h.id("Researcher");
    let (first, second) = taken_in_mid_turn(&h, &stub, &gate, &calls).await;
    h.wait_until("the call after the second was taken in", |_| calls.load(Ordering::SeqCst) == 3)
        .await;

    assert!(h.runtime.stop_run(first));
    h.settle(first).await;
    h.wait_until("the turn to end", |h| h.runtime.live_runs() == 0).await;

    assert_eq!(calls.load(Ordering::SeqCst), 3, "the message a call acted on was carried on blind");
    h.settle(second).await;
    assert_cut_short(&h, second, STOPPED_ANOTHER);

    h.runtime.stop_agent(researcher);
    let store = Store::open(&h._dir.path().join("guac.db")).unwrap();
    let recovered = guac_lib::boot::recover(&store).unwrap();
    assert!(recovered.resumed.is_empty(), "a message a turn acted on was delivered again");
    assert_eq!(recovered.interrupted, 0, "the conversation was reported a second time");
}

const STOPPED_ANOTHER: &str = "You stopped another conversation while Researcher was working on \
     this one too, so this was not finished. Review any actions already taken before retrying.";

const FAILED_TOGETHER: &str = "Researcher was working on this and another conversation together \
     when a model call failed, so this was not finished. Review any actions already taken before \
     retrying.";

const SPENT_TOGETHER: &str = "Researcher was working on this and another conversation together \
     when the other conversation hit its budget, so this was not finished. Review any actions \
     already taken before retrying.";

fn assert_cut_short(h: &Harness, run: RunId, text: &str) {
    let messages = h.runtime.store().channel_messages(h.id("Researcher"), 200).unwrap();
    let asked = messages.iter().find(|m| m.run_id == run && m.from == Participant::Human).unwrap();
    let notices: Vec<&Envelope> = messages
        .iter()
        .filter(|m| m.run_id == run && m.parts.iter().any(|p| matches!(p, Part::Notice { .. })))
        .collect();
    assert_eq!(notices.len(), 1, "{}", h.transcript());
    assert_eq!(notices[0].cause, Some(asked.id), "Try again would not send this message");
    assert_eq!(
        notices[0].parts,
        vec![Part::Notice { kind: NoticeKind::Interrupted, text: text.into(), detail: None }]
    );
    let outcome = h.runtime.store().outcome(run).unwrap().expect("the conversation has an outcome");
    assert_eq!(outcome.status, OutcomeStatus::Interrupted);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stopped_message_taken_into_a_turn_is_not_put_down_for_the_next_host() {
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let stub = gated_researcher(gate.clone(), calls.clone()).await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let (first, second) = taken_in_mid_turn(&h, &stub, &gate, &calls).await;
    let asked = h
        .runtime
        .store()
        .channel_messages(h.id("Researcher"), 50)
        .unwrap()
        .into_iter()
        .find(|m| m.run_id == second)
        .unwrap()
        .id;

    assert!(h.runtime.stop_run(second));
    let drained = h.runtime.drain(Duration::from_secs(5)).await;
    assert!(drained.is_complete(), "{drained:?}");

    let conn = h.runtime.store().conn().unwrap();
    let put: i64 = conn
        .query_row("SELECT count(*) FROM put_down WHERE message_id=?1", [asked.to_string()], |r| {
            r.get(0)
        })
        .unwrap();
    let whole: i64 = conn
        .query_row(
            "SELECT count(*) FROM put_down_runs WHERE run_id=?1",
            [second.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    drop(conn);
    assert_eq!((put, whole), (0, 0), "the stopped conversation was put down for the next host");
    assert_eq!(h.runtime.store().outcome(second).unwrap().unwrap().status, OutcomeStatus::Stopped);

    let (h, _) = h.restart(Duration::from_secs(5)).await;
    h.settle(first).await;
    assert!(!settled_on(&h.sink, second), "the next host picked up a stopped conversation");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_start_mark_leaves_the_message_owed_and_the_next_host_answers_it() {
    let stub = serve(|_| Script::Say("The room is booked.".into())).await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    h.runtime
        .store()
        .conn()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_start BEFORE UPDATE OF started_at ON pending_runs \
             BEGIN SELECT RAISE(ABORT,'disk failure'); END;",
        )
        .unwrap();
    let run = h.runtime.send_from_human(h.id("Researcher"), "Book the room.").unwrap();
    h.wait_until("the start mark to fail and the run to leave the process", |h| {
        h.runtime.live_runs() == 0
    })
    .await;

    assert_eq!(stub.calls.load(Ordering::SeqCst), 0);
    assert!(!settled_on(&h.sink, run), "a run that never started was reported settled");
    assert!(h.runtime.store().outcome(run).unwrap().is_none());
    let pending: i64 = h
        .runtime
        .store()
        .conn()
        .unwrap()
        .query_row("SELECT count(*) FROM pending_runs WHERE run_id=?1", [run.to_string()], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(pending, 1, "the conversation is no longer owed anywhere");

    h.runtime.store().conn().unwrap().execute_batch("DROP TRIGGER reject_start;").unwrap();
    let (h, _) = h.restart(Duration::from_secs(5)).await;
    h.settle(run).await;
    assert_eq!(stub.calls.load(Ordering::SeqCst), 1);
    assert!(h.channel_texts("Researcher").iter().any(|t| t.contains("The room is booked.")));
    assert_eq!(h.runtime.store().outcome(run).unwrap().unwrap().status, OutcomeStatus::Done);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_handed_on_beside_the_turn_that_acted_on_it_never_runs_without_that_record() {
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let stub = {
        let (gate, calls) = (gate.clone(), calls.clone());
        serve(move |_| match calls.fetch_add(1, Ordering::SeqCst) {
            0 => {
                hold_until_open(&gate, "a_message_handed_on_beside_the_turn_that_acted_on_it_never_runs_without_that_record");
                Script::Progress("Looking up the vendor.".into())
            }
            1 => Script::Progress("Looking up the number.".into()),
            2 => Script::Hang,
            _ => Script::Say("Calling the vendor again.".into()),
        })
        .await
    };
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let researcher = h.id("Researcher");
    let (first, second) = taken_in_mid_turn(&h, &stub, &gate, &calls).await;
    h.wait_until("the call after the second was taken in", |_| calls.load(Ordering::SeqCst) == 3)
        .await;

    h.runtime.store().set_lifecycle(researcher, Lifecycle::Paused).unwrap();
    let (h, drained) = h.restart(Duration::from_secs(5)).await;
    assert!(drained.is_complete(), "{drained:?}");
    assert!(h.runtime.stop_run(first));
    h.settle(first).await;
    h.resume("Researcher");
    h.wait_until("the picked-up work to leave the process", |h| h.runtime.live_runs() == 0).await;

    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "the second conversation ran again without the record of the call made for it:\n{}",
        h.transcript()
    );
    assert!(!h.channel_texts("Researcher").iter().any(|t| t.contains("Calling the vendor again.")));
    assert!(settled_on(&h.sink, second), "the second conversation waited for another host");
    assert_cut_short(&h, second, STOPPED_ANOTHER);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_handed_on_beside_the_turn_that_acted_on_it_is_carried_on_in_that_turn() {
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let stub = {
        let (gate, calls) = (gate.clone(), calls.clone());
        serve(move |_| match calls.fetch_add(1, Ordering::SeqCst) {
            0 => {
                hold_until_open(&gate, "a_message_handed_on_beside_the_turn_that_acted_on_it_is_carried_on_in_that_turn");
                Script::Progress("Looking up the vendor.".into())
            }
            1 => Script::Progress("Looking up the number.".into()),
            2 => Script::Hang,
            _ => Script::Say("Hours and number found.".into()),
        })
        .await
    };
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let (first, second) = taken_in_mid_turn(&h, &stub, &gate, &calls).await;
    h.wait_until("the call after the second was taken in", |_| calls.load(Ordering::SeqCst) == 3)
        .await;

    let (h, drained) = h.restart(Duration::from_secs(5)).await;
    assert!(drained.is_complete(), "{drained:?}");
    h.settle(first).await;
    h.settle(second).await;

    assert_eq!(calls.load(Ordering::SeqCst), 4, "{}", h.transcript());
    let resumed = stub.transcript.lock().last().cloned().unwrap();
    assert!(picked_up(&resumed), "the turn did not carry on with its record");
    assert!(
        anyone_said(&resumed, "And their phone number."),
        "the second message was not carried with it"
    );
    assert!(h.channel_texts("Researcher").iter().any(|t| t.contains("Hours and number found.")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_whose_start_cannot_be_recorded_says_so_at_once() {
    let stub = serve(|_| Script::Say("The room is booked.".into())).await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let researcher = h.id("Researcher");
    h.runtime
        .store()
        .conn()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_start BEFORE UPDATE OF started_at ON pending_runs \
             BEGIN SELECT RAISE(ABORT,'disk failure'); END;",
        )
        .unwrap();
    let run = h.runtime.send_from_human(researcher, "Book the room.").unwrap();
    h.wait_until("the start mark to fail and the run to leave the process", |h| {
        h.runtime.live_runs() == 0
    })
    .await;

    let messages = h.runtime.store().channel_messages(researcher, 50).unwrap();
    let asked = messages.iter().find(|m| m.run_id == run && m.from == Participant::Human).unwrap();
    let notices: Vec<&Envelope> = messages
        .iter()
        .filter(|m| m.run_id == run && m.parts.iter().any(|p| matches!(p, Part::Notice { .. })))
        .collect();
    assert_eq!(notices.len(), 1, "{}", h.transcript());
    assert_eq!(notices[0].cause, Some(asked.id));
    assert_eq!(
        notices[0].parts,
        vec![Part::Notice {
            kind: NoticeKind::Lifecycle,
            text: "Researcher could not start on this because the workspace could not record it. \
                   Nothing was done. It will be delivered again when the host next starts."
                .into(),
            detail: None,
        }]
    );
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0);

    h.runtime.store().conn().unwrap().execute_batch("DROP TRIGGER reject_start;").unwrap();
    let (h, _) = h.restart(Duration::from_secs(5)).await;
    h.settle(run).await;
    assert_eq!(stub.calls.load(Ordering::SeqCst), 1);
    assert_eq!(h.runtime.store().outcome(run).unwrap().unwrap().status, OutcomeStatus::Done);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_whose_start_cannot_be_recorded_is_delivered_after_two_crashes_in_a_row() {
    let stub = serve(|_| Script::Say("The room is booked.".into())).await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let researcher = h.id("Researcher");
    h.runtime
        .store()
        .conn()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_start BEFORE UPDATE OF started_at ON pending_runs \
             BEGIN SELECT RAISE(ABORT,'disk failure'); END;",
        )
        .unwrap();
    let run = h.runtime.send_from_human(researcher, "Book the room.").unwrap();
    h.wait_until("the start mark to fail and the run to leave the process", |h| {
        h.runtime.live_runs() == 0
    })
    .await;
    h.runtime.store().conn().unwrap().execute_batch("DROP TRIGGER reject_start;").unwrap();

    h.runtime.stop_agent(researcher);
    let store = Store::open(&h._dir.path().join("guac.db")).unwrap();
    let first = guac_lib::boot::recover(&store).unwrap();
    assert_eq!(first.resumed.iter().map(|put| put.envelope.run_id).collect::<Vec<_>>(), vec![run]);
    let second = guac_lib::boot::recover(&store).unwrap();
    assert_eq!(second.interrupted, 0, "a message no turn started was reported interrupted");
    assert_eq!(second.resumed.iter().map(|put| put.envelope.run_id).collect::<Vec<_>>(), vec![run]);
}

async fn acted_on_then(gate: Arc<std::sync::atomic::AtomicBool>, calls: Arc<AtomicUsize>) -> Stub {
    serve(move |_| match calls.fetch_add(1, Ordering::SeqCst) {
        0 => {
            hold_until_open(&gate, "acted_on_then");
            Script::Progress("Looking up the vendor.".into())
        }
        1 => Script::Progress("Looking up the number.".into()),
        _ => Script::Unavailable,
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_model_call_tells_a_conversation_the_turn_acted_on_at_once() {
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let stub = acted_on_then(gate.clone(), calls.clone()).await;
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let (first, second) = taken_in_mid_turn(&h, &stub, &gate, &calls).await;

    h.settle(first).await;
    h.settle(second).await;

    assert_cut_short(&h, second, FAILED_TOGETHER);
    assert_eq!(h.runtime.store().outcome(first).unwrap().unwrap().status, OutcomeStatus::Failed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_exhausted_budget_tells_a_conversation_the_turn_acted_on_at_once() {
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let stub = acted_on_then(gate.clone(), calls.clone()).await;
    let h = harness(
        &stub,
        &["Researcher"],
        GuardLimits { max_steps_per_run: 2, ..GuardLimits::default() },
    );
    let (first, second) = taken_in_mid_turn(&h, &stub, &gate, &calls).await;

    h.settle(first).await;
    h.settle(second).await;

    assert_eq!(calls.load(Ordering::SeqCst), 2, "the budget allowed a third model call");
    assert_cut_short(&h, second, SPENT_TOGETHER);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_model_call_hands_back_a_message_the_turn_never_acted_on() {
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let stub = {
        let (gate, calls) = (gate.clone(), calls.clone());
        serve(move |body| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                hold_until_open(
                    &gate,
                    "a_failed_model_call_hands_back_a_message_the_turn_never_acted_on",
                );
                return Script::Progress("Looking up the vendor.".into());
            }
            if has_tool_result(body) {
                return Script::Unavailable;
            }
            Script::Say("Their number is 555-0100.".into())
        })
        .await
    };
    let h = harness(&stub, &["Researcher"], GuardLimits::default());
    let (first, second) = taken_in_mid_turn(&h, &stub, &gate, &calls).await;

    h.settle(first).await;
    h.settle(second).await;

    let answered =
        h.runtime.store().channel_messages(h.id("Researcher"), 200).unwrap().into_iter().any(|m| {
            m.run_id == second && m.to == Participant::Human && m.plain_text().contains("555-0100")
        });
    assert!(
        answered,
        "the unread conversation was not answered in a turn of its own:
{}",
        h.transcript()
    );
    assert_eq!(h.runtime.store().outcome(second).unwrap().unwrap().status, OutcomeStatus::Done);
}
