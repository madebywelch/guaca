//! End-to-end runtime tests.
//!
//! These drive the real actor runtime against a scripted OpenAI-compatible
//! server, so everything between "operator presses enter" and "four agents have
//! replied" is exercised: tool-call assembly, the guard, channel routing, batch
//! coalescing, and settle detection. The only thing swapped out is the model.

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
use guac_lib::domain::agent::Lifecycle;
use guac_lib::domain::approval::{Decision, ProtectedAction};
use guac_lib::domain::connector::CleanConnector;
use guac_lib::domain::envelope::{Part, Participant};
use guac_lib::domain::group::CleanGroup;
use guac_lib::domain::now_ms;
use guac_lib::domain::routine::{Cadence, RunKind, Trigger};
use guac_lib::domain::signin::{Signin, Surface};
use guac_lib::llm::openrouter::LlmClient;
use guac_lib::runtime::events::{Activity, RecordingSink, UiEvent};
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::runtime::Runtime;
use guac_lib::workspace::Workspace;

use harness::*;

// ---- tests ---------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn manager_introduces_itself_to_every_other_agent() {
    // The scenario from the brief, start to finish.
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

    // Every peer received the introduction in its own channel.
    for peer in peers {
        let texts = h.channel_texts(peer);
        assert!(
            texts.iter().any(|t| t.contains("I'm Manager")),
            "{peer} never received the introduction. Channel was: {texts:?}"
        );
    }

    // Every reply came back to the Manager's channel.
    let manager_channel = h.channel_texts("Manager").join("\n");
    for peer in peers {
        assert!(
            manager_channel.contains(&format!("I'm {peer}")),
            "{peer}'s reply never reached the Manager channel. Channel was:\n{manager_channel}"
        );
    }

    // Four introductions out, four replies back.
    assert_eq!(h.feed().len(), 8, "expected 4 sends and 4 replies in the activity feed");

    // Two Manager calls to send and then speak, four peer calls, and between
    // one and four more for the Manager to read the replies, depending on how
    // many batches they land in. That last part is scheduling, not behaviour,
    // so this bounds amplification rather than pinning a number: batching
    // itself is asserted deterministically in
    // `replies_queued_together_are_read_in_a_single_turn`.
    let calls = stub.calls.load(Ordering::SeqCst);
    assert!(
        (6..=10).contains(&calls),
        "the cascade should converge in 6-10 model calls, took {calls}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn replies_queued_together_are_read_in_a_single_turn() {
    // Batching is what stops four replies from costing four Manager turns.
    // Rather than racing the scheduler, this pauses the Manager so all four
    // replies are provably sitting in its inbox before it wakes up.
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
            // Long enough that the Manager is idle and pausable before any
            // reply is on the wire.
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

    // Once all four introductions are out, the peers are still sleeping.
    h.wait_until("the introductions to be sent", |h| {
        h.feed().iter().filter(|e| e.from == Participant::Agent { id: manager }).count() == 4
    })
    .await;
    h.pause("Manager");

    // Waiting on the transcript alone is not enough: a message is persisted
    // before it is enqueued, so resuming in that window catches only three.
    // At most one reply can have been pulled already, hence three still queued.
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
async fn two_agents_told_to_talk_forever_stop_on_their_own() {
    // Both agents are scripted to always message the other. Without the guard
    // this never terminates.
    let stub = serve(|body| {
        let who = speaker(body);
        let other = if who == "Ping" { "Pong" } else { "Ping" };
        Script::SendTo {
            recipients: vec![other.to_string()],
            // Varying text defeats the dedup check on purpose, so this test
            // exercises the hop and pair limits rather than dedup.
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

    // The transcript must say why it stopped, not just go quiet.
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
    // Regression: the trail used to ride along on the reply envelope, and a
    // reply to a peer files in the recipient's channel. Opening one agent's
    // channel therefore showed you every other agent's private working notes.
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
    // The refusal is about what this agent tried to do, so it belongs in this
    // agent's channel rather than travelling to whoever it was aimed at.
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

    // Give Chef some history first.
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
    // Regression: a paused actor parks on a notifier whose only other handle
    // lives in the inbox. Deleting the agent drops that inbox, so the actor
    // would wait forever, leaking the task and the message it was holding.
    let stub = serve(|_| Script::Say("ok".into())).await;
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    assert_eq!(h.runtime.live_actors(), 2);

    h.pause("Chef");
    // Give Chef something to hold, so it is parked mid-message rather than
    // idle on an empty inbox.
    h.runtime.send_from_human(h.id("Chef"), "you are paused").unwrap();
    h.wait_until("Chef to park", |h| {
        matches!(
            h.runtime.activity_snapshot().get(&h.id("Chef")),
            Some(guac_lib::runtime::events::Activity::Paused)
        )
    })
    .await;

    h.runtime.store().set_lifecycle(h.id("Chef"), Lifecycle::Terminated).unwrap();
    // Deleting must wake the parked actor by itself. Nothing else can: this
    // call drops the last handle to the notifier it is waiting on.
    h.runtime.stop_agent(h.id("Chef"));

    h.wait_until("Chef's actor to exit", |h| h.runtime.live_actors() == 1).await;
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
async fn an_agent_can_write_its_memory_and_reads_it_back_next_turn() {
    // The write-manage-read loop, end to end: an agent records something on one
    // turn and finds it in its own prompt on the next.
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
async fn an_agent_asked_for_its_memory_writes_the_same_file_as_its_notes() {
    // What the operator calls this file is memory; the tool is `update_notes`.
    // An agent that takes the operator at their word and calls `update_memory`
    // has written the right thing to the right place, and refusing it would
    // spend a turn on the difference between two words for one file.
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
async fn what_one_agent_remembers_is_never_shown_to_another() {
    // A memory is written by an agent for itself, in whatever shorthand suits
    // it, and it holds whatever the operator has told that one agent in
    // confidence. It is also the only thing that survives a conversation, so a
    // crew that pooled memories would grow a shared file nobody wrote and every
    // agent acts on. One file per agent, and the boundary is the prompt.
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
    // A second turn, so the Manager's own prompt is one built after the write.
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

    h.runtime.workspace().remove(h.id("Manager"));
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
async fn a_model_that_thinks_out_loud_is_watched_without_being_recorded() {
    // The whole of the feature: an operator can see what a turn is doing while
    // it does it, and nothing about it survives the turn. Reasoning that leaked
    // into the message would be replayed into every later prompt, hashed by the
    // loop guard, and read back by a peer as something the agent had said.
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

    // And the transcript holds the answer alone.
    let persisted = h.channel_texts("Manager").pop().unwrap();
    assert_eq!(persisted, "391.");
    assert!(!persisted.contains("17 times 23"), "reasoning reached the transcript: {persisted}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_thought_is_coalesced_on_the_same_clock_as_the_text() {
    // Reasoning is produced as fast as text and costs the same IPC hop and
    // render. A thinking model streaming its working one token at a time is
    // the freeze the pen exists to prevent, arriving through a second door.
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
    // Coalesced, not truncated: the tail is flushed when the call ends.
    assert_eq!(h.sink.streamed_reasoning(stream_id), expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_long_reply_reaches_the_window_in_far_fewer_events_than_tokens() {
    // Every event is an IPC hop and a re-render in the operator's window, and a
    // model writes faster than a screen refreshes. Emitting one per token spent
    // the main thread on work no eye could resolve; with five agents answering
    // at once it stopped painting altogether, which is what an operator
    // reported as the app freezing and the text arriving in a lump.
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

    // And not one character was coalesced away. The buffer is flushed when the
    // call ends, or the tail of every reply would be lost.
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
    // Each peer's response is delayed. If the runtime were serial, five agents
    // at 300ms each would take 1.5s; concurrent should be closer to one delay.
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

// ---- group isolation -----------------------------------------------------

#[tokio::test]
async fn an_agent_cannot_message_a_peer_in_another_group() {
    // Chef exists, is live, and is addressed by its exact name. The only thing
    // stopping the message is the group boundary.
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
    // The control for the test above: the boundary must not be a blanket ban.
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
    // The boundary has to hold at discovery too. Refusing the send but naming
    // the peer in the directory would hand the model a roster it can only be
    // frustrated by, and leak who exists elsewhere.
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
    // Models answer their correspondent by calling `send_message` at them
    // rather than replying in plain text. If that counts as a fresh approach,
    // the answer demands an answer and the exchange only ends when the guard
    // fires: a two-agent introduction was observed reaching hop 7 of 8, stopped
    // by the dedup rule rather than by anyone running out of things to say.
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
            // Chef answers Manager the way the real models do.
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
    // Found in a real workspace. The operator asked one agent to introduce
    // itself to the crew, it messaged seven, and three of the seven then wrote
    // to the operator: "I replied to the Chief of Staff confirming how I work".
    // The operator had spoken to one agent and got mail from four.
    //
    // The turn shape that produces it is ordinary: answer the peer with
    // `send_message`, then keep typing. The trailing text is not a second
    // answer, so it was sent to the operator instead, who is the one
    // participant an envelope can always be addressed to. Being addressable is
    // not the same as being the audience.
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

    // Not delivered is not the same as thrown away. It is on the record of the
    // turn that wrote it, in Chef's own channel, where the operator can read
    // what their agent did without being handed a message about it.
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
async fn a_document_attached_after_the_answer_still_reaches_the_peer_that_asked() {
    // The carve-out in the rule above, and the reason it is not "drop whatever
    // a turn trails". Text written after the answer went is a restatement of
    // it; a file is not. `send_message` carries its own files, so one attached
    // afterwards has reached nobody at all, and the agent that asked for the
    // work is who it belongs to. Silently filing it as commentary would lose
    // the only thing the turn produced.
    let stub = serve(|body| {
        let calls = body["messages"]
            .as_array()
            .map(|m| m.iter().filter(|msg| msg["role"] == "tool").count())
            .unwrap_or(0);

        if speaker(body) == "Chef" {
            match calls {
                // Answer the way the real models do, then hand the document
                // over afterwards, then say so.
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

    // In Manager's channel and from Chef: that is a document delivered to the
    // agent that asked, rather than one filed in the sender's own record.
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
    // The refusal is read mid-turn by the model that caused it, and it is the
    // only thing standing between "stop being polite at each other" and a
    // model that tries the same send again with different words.
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
        told.contains("Reply to the operator instead"),
        "a refusal without a way forward is a dead end: {told}"
    );
    assert!(
        told.contains(r#"intent "work""#),
        "and when the message really was work, the way through has to be named: {told}"
    );
}

#[tokio::test]
async fn a_second_instruction_to_a_peer_that_already_answered_is_delivered() {
    // Found in a real session. The operator authorised an external send, the
    // coordinator relayed it, read the answer, and was refused when it tried to
    // instruct again: the guard cannot tell a second instruction from a thank
    // you, and it was aimed at the thank you. Every delegation needing two
    // rounds died there, so the sender now says which it is.
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
        } else if text.contains("I need you to confirm") {
            // The turn this test exists for: woken by an answer, nobody is
            // waiting on the Manager, and it has genuinely new work to give.
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
        h.channel_texts("Chef").iter().any(|t| t.contains("Sent.")),
        "and Chef never acted on it:\n{}",
        h.transcript()
    );
}

#[tokio::test]
async fn a_peer_instructed_after_it_answered_does_the_work_rather_than_going_quiet() {
    // Found in a real session, and the exact shape of it. The operator asked
    // for an email, the coordinator relayed it, the peer opened the document
    // and reported back, and the coordinator then issued the actual send
    // instruction. That instruction was delivered, and the peer said nothing.
    //
    // Nothing was waiting on a reply, so the turn ran in the mode that tells an
    // agent nobody is asking it for anything and silence is usually right. It
    // spent a model call and complied. From the operator's side an agent had
    // simply stopped.
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
                // A model that reads its prompt. Told nothing is being asked of
                // it and that silence is usually right, it stays silent, which
                // is exactly what the live agent did with a real instruction to
                // send an email in front of it.
                Script::Say(String::new())
            } else {
                Script::Say("Sent it.".into())
            }
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
        h.channel_texts("Chef").iter().any(|t| t.contains("Sent it.")),
        "the instruction landed and the agent went quiet:\n{}",
        h.transcript()
    );
}

#[tokio::test]
async fn work_and_a_reply_are_different_questions_on_the_wire() {
    // The two used to be the same field. An instruction carries work and wants
    // no reply, which is the combination that had nowhere to live.
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
        "the first message of an exchange still expects an answer; only a settled pair does not"
    );
}

#[tokio::test]
async fn a_courtesy_to_a_peer_that_already_answered_is_still_refused() {
    // The other half. Declaring intent is not a way around the guard: the
    // thank-you that used to run a crew in circles is turned away exactly as
    // before, and only a message that says it carries work gets through.
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
    // The whole shape, as an operator ran it: introduce yourself to three
    // agents, all three answer, and the manager goes back round thanking them.
    //
    // The replies land milliseconds apart, so the manager's turn sees one of
    // them and the other two arrive after. Deciding "am I answering this peer"
    // from the batch made those two strangers, and strangers get messages that
    // demand answers: the exchange went to hop 4 and produced four summaries
    // for one instruction.
    let stub = serve(|body| {
        let text = body["messages"]
            .as_array()
            .map(|m| m.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();

        if has_tool_result(body) {
            Script::Say("introductions done".into())
        } else if text.contains("good to meet you") {
            // The manager thanks all three, whatever the batch happened to hold.
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
    // Settings resolve agent over group over app. The point of the chain is
    // that two crews can run on different models at once, so this checks what
    // the model actually received rather than what was configured.
    let stub = serve(|_| Script::Say("ok".into())).await;

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("guac.db")).unwrap();
    let pinned = store
        .create_group(&CleanGroup {
            name: "Local".into(),
            base_url: None,
            default_model: Some(Some("local/qwen".into())),
            api_key: None,
        })
        .unwrap();

    // One agent inherits its group's model, one names its own, one is in the
    // default group and inherits the app default.
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
    };
    let sink = RecordingSink::new();
    let runtime = Runtime::new(
        store,
        LlmClient::new().unwrap(),
        config,
        Workspace::new(dir.path().join("workspace")),
        guac_lib::files::FileStore::new(dir.path().join("files")),
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

// ---- routines ------------------------------------------------------------

#[tokio::test]
async fn a_routine_that_is_due_wakes_its_agent() {
    // The whole point of a schedule is that something happens without anyone
    // typing, so this drives the real scheduler rather than calling the
    // delivery path directly.
    let stub = serve(|_| Script::Say("checked the listings".into())).await;
    let h = harness(&stub, &["Watcher"], GuardLimits::default());

    // Due a moment ago, the state the scheduler actually finds things in.
    h.runtime
        .store()
        .create_routine(
            h.id("Watcher"),
            "Listings sweep",
            "check the listings",
            Trigger::Clock(Cadence::Every(3600)),
            Some(now_ms() - 1000),
        )
        .unwrap();

    h.runtime.start_scheduler();
    h.wait_until("the routine to fire", |h| {
        h.channel_texts("Watcher").iter().any(|t| t.contains("checked the listings"))
    })
    .await;

    // A repeating routine stays, moved forward rather than firing again.
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
        )
        .unwrap();

    h.runtime.start_scheduler();
    h.wait_until("the alarm to go off", |h| {
        h.channel_texts("Sleeper").iter().any(|t| t.contains("woke up"))
    })
    .await;

    assert!(
        h.runtime.store().agent_routines(h.id("Sleeper")).unwrap().is_empty(),
        "a one-off must be gone once it has run, not left with a time in the past"
    );
}

#[tokio::test]
async fn a_fired_routine_reaches_the_model_as_its_instruction_and_the_operator_as_one_line() {
    // Both halves of the same delivery. The model has to be told exactly what
    // the routine says, or a schedule does nothing. The operator was being
    // shown the same several sentences as a chat bubble from Guaca, in the
    // middle of their own conversation with the agent: the system prompting
    // their agent, drawn as though somebody had typed it to them.
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
        )
        .unwrap();

    let run = h.runtime.test_routine(&routine).unwrap();
    h.settle(run).await;

    // What the model was sent: the instruction, unchanged.
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

    // What the transcript holds: one part naming the routine, and no loose
    // text for a bubble to draw.
    match fired.parts.as_slice() {
        [Part::Routine { routine_id, name, what }] => {
            assert_eq!(*routine_id, routine.id, "so the operator can open the routine it names");
            assert_eq!(name, "Listings sweep");
            assert_eq!(what, "Check the listings and say what is new.");
        }
        other => panic!("a fired routine is one routine part, got {other:?}"),
    }

    // And the agent still did the work, which is the whole point of the change
    // being only about how it is drawn.
    assert!(
        h.channel_texts("Watcher").iter().any(|t| t.contains("checked")),
        "the routine fired and the agent said nothing:\n{}",
        h.transcript()
    );
}

#[tokio::test]
async fn testing_a_routine_delivers_it_without_spending_the_schedule() {
    // The button exists so an operator can find out what a routine does before
    // Tuesday morning. Firing it through the scheduler's own path would move
    // the slot, and on a one-shot it would consume the only firing it had.
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

    // It is in the history, marked as the test it was.
    let history = h.runtime.store().routine_runs(routine.id, 20).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].kind, RunKind::Test);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_routine_that_fires_is_work_to_do_rather_than_something_to_note() {
    // A fired routine arrives from the system, and nobody is waiting on the
    // agent's words: the answer goes into its own channel. That combination is
    // the one that used to mean "nothing is being asked of you, and silence is
    // usually right", because the instruction came from neither the operator
    // nor a peer and so matched neither arm. A model that reads its prompt then
    // does exactly what it was told, and the operator watches a schedule they
    // set produce nothing at all, which is indistinguishable from a broken one.
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
        )
        .unwrap();

    let run = h.runtime.test_routine(&routine).unwrap();
    h.settle(run).await;

    // The mode itself, which is the claim: a routine hands over work.
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
    // A standing job in a crew is rarely one agent's to do alone: the schedule
    // belongs to whoever owns the outcome, and the work belongs to whoever has
    // the skill. This is the whole delegation path with nobody typing anything,
    // and it runs under a budget of its own because a fired routine is a fresh
    // run.
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
    // The whole loop an operator asks for when they say "every weekday": the
    // agent books it, what it booked is what it was asked for, and the thing
    // that fires later is something it can act on with nothing else in front of
    // it.
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

    // And what it booked works: the same delivery the scheduler makes.
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
    // The failure this exists for. An operator books something, comes back half
    // an hour later and asks for it at a different time without saying which
    // routine they mean. The agent had no way to know it already kept one, and
    // no verb for changing it: it wrote a second beside the first, said it had
    // made the change, and both fired from then on.
    //
    // The id is read out of the agent's own prompt, so this asserts the whole
    // path rather than the tool: what it keeps is in front of it, and the way
    // to change it takes that id.
    let stub = serve(|body| {
        if has_tool_result(body) {
            return Script::Say("Moved it to every morning.".into());
        }
        match standing_id(body) {
            Some(id) => Script::Retime { id, repeat: "daily".into() },
            // No id in the prompt is the bug: an agent that cannot see what it
            // keeps books a second routine instead.
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
    // A schedule is not shared. `update` reaches a row by id, and an id that
    // arrives from anywhere other than this agent's own list has to miss.
    let stub = serve(|body| {
        if has_tool_result(body) {
            return Script::Say("Could not change it.".into());
        }
        // The id is passed in through the instruction, which is the only way an
        // agent could come by a peer's: read out of a message.
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
    // The backstop for the same failure, for a turn that books anyway. Not a
    // refusal: nothing here can tell "move the sweep" from "sweep twice a day",
    // and refusing the second would refuse honest work. Said, with both ids,
    // while the turn that knows which it meant is still running.
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
        )
        .unwrap();
    h.runtime.store().set_routine_active(routine.id, false).unwrap();

    h.runtime.start_scheduler();
    // Overdue and inactive: the one state where the scheduler must do nothing.
    // Two ticks' worth, since proving a negative here is proving it stayed
    // silent through the sweep that would otherwise have caught it.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        h.channel_texts("Watcher").is_empty(),
        "an inactive routine must not fire, however overdue it is"
    );

    h.runtime.store().set_routine_active(routine.id, true).unwrap();
    h.wait_until("it to fire once switched back on", |h| {
        h.channel_texts("Watcher").iter().any(|t| t.contains("checked"))
    })
    .await;
}

#[test]
fn a_schedule_that_cannot_be_read_parks_until_the_next_tick() {
    // The scheduler shares its runtime with every agent, so a pass that returns
    // without waiting is not a slow scheduler: it is a worker nobody else gets
    // back. Paused time is what makes that observable rather than merely slow.
    // Tokio only advances a paused clock once every task is parked on a timer,
    // so a scheduler spinning on a store it cannot read holds the clock still
    // and the minute below never elapses. One thread, because on a multi-thread
    // runtime the spin has somewhere else to go and the bug hides.
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
            // The one failure the scheduler cannot read its way out of, and the
            // one that stays broken for as long as the process runs.
            h.runtime.store().conn().unwrap().execute_batch("DROP TABLE routines").unwrap();

            h.runtime.start_scheduler();

            // Several ticks' worth, and it costs no real time: the clock jumps
            // the moment this task and the scheduler are both on a timer.
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        let _ = finished.send(());
    });

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
        recognised: true,
        first_seen_at: 0,
        last_seen_at: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_is_told_what_its_browser_holds_and_what_its_peers_hold() {
    // The whole point, end to end. Two failures it exists to stop, and they are
    // opposites: an agent whose browser is signed in and does not know, so it
    // declines; and an agent told about a session on another machine, so it
    // tries and hits a login wall.
    let stub = serve(|_| Script::Say("Noted.".into())).await;
    let h = harness(&stub, &["Manager", "Researcher"], GuardLimits::default());

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

    // The agent that holds the session knows it holds it, and knows which of
    // its two places holds it: a session in one is unreachable from the other.
    assert!(
        researcher.contains("You are signed in to these already")
            && researcher.contains("- LinkedIn in your browser"),
        "the agent whose browser is signed in must be told so: {researcher}"
    );
    // The one that is not, is not told it is.
    assert!(
        !manager.contains("You are signed in to these already"),
        "cookies are in one jar; claiming otherwise produces a login wall: {manager}"
    );
    // But it is told who to ask, which is the answer it should give instead.
    assert!(
        manager.contains("Researcher") && manager.contains("signed in to LinkedIn"),
        "the roster has to name the agent that can do it: {manager}"
    );

    // A credential is a string, so both machines have it, and neither prompt
    // has the value.
    for prompt in [researcher, manager] {
        assert!(prompt.contains("$GITHUB_TOKEN"), "the variable is named: {prompt}");
        assert!(!prompt.contains("ghp_hunter2"), "a secret reached a model: {prompt}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_directory_says_which_peer_is_signed_in_to_what() {
    // The roster in the prompt is one path; `directory` is the other, and an
    // agent that calls it mid-turn to decide who to delegate to reads this one.
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Researcher can.".into())
        } else {
            Script::Directory
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Researcher"], GuardLimits::default());
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

// ---- adding an agent -----------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_cannot_add_a_colleague_without_the_operator_saying_so() {
    // The whole point of the mechanism: the turn stops, nothing exists yet, and
    // a person decides. An agent that could staff its own workspace could spend
    // the operator's money in a shape they never chose.
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
async fn an_agent_told_by_a_peer_that_it_was_authorised_asks_the_operator_instead_of_refusing() {
    // The live failure this exists for. The operator authorised an email, the
    // coordinator relayed the authorisation, and the sending agent declined:
    // correctly, because a peer's word is a claim. It then asked the operator
    // to repeat the instruction in another channel, which is the operator
    // doing the routing by hand for a decision they had already made. Asking
    // them, with two buttons, is the whole difference.
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
            Script::AskOperator {
                action: "Email the SCDOT response to robert@madebywelch.com for review".into(),
                because: "Manager says the operator authorised it; a peer's word is not \
                          permission to send mail in their name."
                    .into(),
            }
        }
    })
    .await;

    // A workspace with a machine, because an agent with no way out of the
    // workspace has nothing for the operator to authorise and is refused
    // before they are asked.
    let h = harness_with_computer(&stub, &["Outreach"], GuardLimits::default());
    let run =
        h.runtime.send_from_human(h.id("Outreach"), "Manager will tell you what to send.").unwrap();

    let request = h.awaited_request().await;
    // The question is in the transcript, where the operator is already looking,
    // and it says what will happen rather than that something wants doing.
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
            Script::AskOperator {
                action: "Email the response to the procurement officer".into(),
                because: "asked by Manager".into(),
            }
        }
    })
    .await;

    // A workspace with a machine, because an agent with no way out of the
    // workspace has nothing for the operator to authorise and is refused
    // before they are asked.
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
async fn asking_to_act_with_nowhere_to_act_is_refused_without_troubling_the_operator() {
    // The live failure. An agent worked out that it could not reach the
    // operator's calendar, and asked them for permission to have it. This
    // workspace has no computer and no browser, so there was no action to
    // authorise and nothing a click could have handed over: what was missing
    // was access. The operator got a decision that changed nothing instead of a
    // sentence saying what they would have to add.
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say(
                "I cannot reach your calendar from here. Nothing is connected to it.".into(),
            )
        } else {
            Script::AskOperator {
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
        told.contains("add a provider"),
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
    // A refusal worded as an error gets retried. This one has to read as
    // settled, or the next turn asks the operator the same question again.
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

    // The operator still gets an answer rather than a dead turn.
    let said = h.channel_texts("Manager");
    assert!(said.iter().any(|t| t.contains("no new agent")), "{said:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn always_allow_means_the_same_agent_is_not_asked_twice() {
    // The second request is the one that must not appear. Asking again after
    // "always" is how a permission prompt becomes something to click through.
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
    // Approving a create that then fails on a duplicate name spends the
    // operator's attention on nothing at all.
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

// ---- surviving a bad connection ------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_connection_that_drops_once_is_retried_rather_than_reported() {
    // The failure an operator actually hits: a laptop changes network, one
    // request never lands, and an agent that would have answered fine reports
    // that it could not reach the endpoint. Retrying is what the operator would
    // have done, so the runtime does it first.
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
    // Once retries are spent the operator has to be told, and told next to the
    // thing that failed, or the only way back is to retype what they asked.
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
    // The shipped bug, from the operator's side, on the path that produces it:
    // a peer that has already answered is instructed again, so the message
    // carries work and expects no reply. That is `ReplyMode::Assigned`, whose
    // own prompt says silence is the one wrong answer. A turn that produced no
    // text used to produce no envelope either, so there was nothing in Chef's
    // channel, nothing in the feed, and an agent that to the operator had
    // simply stopped. The turn may still be silent; it may no longer be silent
    // invisibly.
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
        !instruction.expects_reply,
        "work with nobody waiting is the combination that produces Assigned"
    );

    // Read as a notice part rather than as text: this is Guaca speaking into
    // Chef's channel, which is exactly what `plain_text` filters out.
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
async fn an_agent_reading_an_acknowledgement_may_still_say_nothing_quietly() {
    // The other side of the same line, and the one that must not regress. The
    // asymmetry that terminates cascades depends on a peer being able to read a
    // courtesy and write nothing at all. Reporting that as a failure would put
    // a chip in a channel after every well-behaved broadcast.
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
