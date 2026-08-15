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
use guac_lib::domain::envelope::{Part, Participant};
use guac_lib::domain::group::CleanGroup;
use guac_lib::domain::now_ms;
use guac_lib::llm::openrouter::LlmClient;
use guac_lib::runtime::events::{RecordingSink, UiEvent};
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
async fn an_agent_can_write_notes_and_reads_them_back_next_turn() {
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
async fn deleting_an_agent_takes_its_notes_with_it() {
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
    };
    let sink = RecordingSink::new();
    let runtime = Runtime::new(
        store,
        LlmClient::new().unwrap(),
        config,
        Workspace::new(dir.path().join("workspace")),
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
        .create_routine(h.id("Watcher"), "check the listings", Some(3600), now_ms() - 1000)
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
        routines[0].next_run_at > now_ms(),
        "and must not still be due, or it fires again on the next tick"
    );
    assert!(routines[0].last_run_at.is_some(), "it should record having run");
}

#[tokio::test]
async fn a_one_off_routine_does_not_come_due_twice() {
    let stub = serve(|_| Script::Say("woke up".into())).await;
    let h = harness(&stub, &["Sleeper"], GuardLimits::default());

    h.runtime.store().create_routine(h.id("Sleeper"), "wake up", None, now_ms() - 1000).unwrap();

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
