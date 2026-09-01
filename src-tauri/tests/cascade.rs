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
use guac_lib::domain::agent::{Lifecycle, COMPOST_MS};
use guac_lib::domain::approval::{ApprovalState, Decision, ProtectedAction, Request};
use guac_lib::domain::connector::CleanConnector;
use guac_lib::domain::envelope::{Part, Participant, ToolOutcome};
use guac_lib::domain::group::{CleanGroup, GroupLimits, InferenceOverrides};
use guac_lib::domain::now_ms;
use guac_lib::domain::routine::{Cadence, RunKind, Trigger};
use guac_lib::domain::signin::{Signin, Surface};
use guac_lib::llm::openrouter::LlmClient;
use guac_lib::runtime::events::{Activity, RecordingSink, UiEvent};
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::runtime::{OnDisk, Runtime};

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
    // many batches they land in. That last part is scheduling, not behavior,
    // so this bounds amplification rather than pinning a number: batching
    // itself is asserted deterministically in
    // `replies_queued_together_are_read_in_a_single_turn`.
    let calls = stub.calls.load(Ordering::SeqCst);
    assert!(
        (6..=10).contains(&calls),
        "the cascade should converge in 6-10 model calls, took {calls}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_that_arrives_mid_turn_reaches_the_turn_that_is_working() {
    // The operator types a correction while an agent is working. Before the
    // turn could see its own inbox, that correction waited for the turn to end,
    // which on a long turn is forty minutes: the work it was meant to steer was
    // finished and published first.
    //
    // The message is sent from inside the stub rather than from the test, so
    // there is no race to lose. Answering the first model call is provably
    // inside the turn, and provably before the round that follows it.
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
    // The discriminator. Left queued, the correction becomes a turn of its own
    // and this line is what the working turn finishes with first.
    assert!(
        !texts.iter().any(|t| t.contains("Draft done")),
        "the turn finished on its own answer and read the correction afterwards: {texts:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn work_that_lands_mid_turn_is_reported_even_when_the_turn_owed_nobody_words() {
    // A turn woken by a peer's answer owes nobody words, which is `NoteOnly`,
    // and `NoteOnly` tells an agent that silence is usually right. That is true
    // of what it woke up to and false of work that lands while it runs, so
    // anything taken in that carries work moves the turn to `Assigned`, where a
    // note the agent never writes is reported rather than looking like an agent
    // that quietly stopped. Both modes write to the same place, so nothing the
    // UI has already drawn moves.
    let hook: Arc<std::sync::OnceLock<(Runtime, guac_lib::domain::ids::AgentId)>> =
        Arc::new(std::sync::OnceLock::new());

    let armed = hook.clone();
    let stub = serve(move |body| {
        if speaker(body) == "Manager" {
            // An answer, which is the one thing that never expects one back.
            return Script::Say("Here is the answer.".into());
        }
        // The correction has been taken in. Say nothing at all, which is what
        // `NoteOnly` invites and what must not pass unremarked now that work
        // has landed in this turn.
        if anyone_said(body, "fix the headline") {
            return Script::Say(String::new());
        }
        // Woken by the Manager's answer: nobody is waiting on this agent's
        // words. The operator types while it is working.
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

    // A notice, not text: this is Guaca speaking into the Writer's channel,
    // which is what `plain_text` filters out.
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
    // `deliver` writes to the store before it touches the inbox, so a message
    // queued behind the one a turn is answering is already in the history that
    // turn reads while still sitting in the inbox waiting to be answered.
    // Taken in without checking, it is written into the prompt twice, and a
    // model reading the same instruction twice is being told it was said twice.
    //
    // Pausing is what makes the window certain rather than likely: both
    // messages are provably in the store before the agent is free to read
    // either.
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
    // The other half of the rule, and the reason it is not "read everything".
    // A turn in `ToPeer` is addressed to the agent that asked, so an operator
    // message folded into it would be read and never answered. It waits, and
    // gets the turn it is owed.
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

    // The peer got its answer, undiluted.
    let manager = h.channel_texts("Manager");
    assert!(
        manager.iter().any(|t| t.contains("Here is the announcement")),
        "the peer that asked never got its answer: {manager:?}"
    );

    // And the operator got theirs, in a turn of its own.
    h.wait_until("the operator's message to be answered", |h| {
        h.channel_texts("Writer").iter().any(|t| t.contains("Noted for next time"))
    })
    .await;
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
async fn a_reply_still_being_worked_on_is_waited_for_rather_than_read_alone() {
    // The gather used to run on a 2.5-second clock sized to this suite's own
    // stub, which answers in milliseconds. Real model calls land tens of
    // seconds apart, so in production the window expired before the second
    // answer of every fan-out and a coordinator read its replies one whole
    // prompt and one model call at a time. The wait now follows the work: as
    // long as a peer that owes an answer is still on it, the gatherer holds
    // out. Grocer's answer here takes longer than the whole of the old window,
    // and it must still be read in the same turn as Chef's.
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
    // The other half of the generous window, and the reason it can be generous
    // at all. A peer whose model call fails every attempt never answers, and
    // an agent that waited the full window for it would hold its own turn, and
    // the run's settlement, open for two minutes of nothing. The gather ends
    // when nobody owing an answer is still working, so the failed peer ends it
    // within this test's ordinary settle window rather than at the ceiling.
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
    // The whole assertion: 20 seconds covers Grocer's retries and nothing like
    // the gather ceiling, so settling here means the wait ended with the peer.
    h.settle(run).await;

    assert!(
        h.channel_texts("Manager").iter().any(|t| t.contains("Acknowledged, from Chef.")),
        "Chef's reply still has to arrive:\n{}",
        h.transcript()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_operator_line_reaches_the_model_after_the_first_rather_than_above_it() {
    // `deliver` writes to the store before the inbox, so an operator's second
    // line typed while the first is being picked up is in the history the turn
    // reads while still queued behind the batch. Rendered from the history it
    // sat *above* the message it follows, because the batch is rendered last:
    // a correction read before the thing it corrects. It is kept out of the
    // history now and reaches the model through intake, in the order it was
    // said.
    let stub = serve(|body| {
        let _ = body;
        Script::Say("Both noted.".into())
    })
    .await;

    let h = harness(&stub, &["Writer"], GuardLimits::default());
    let writer = h.id("Writer");

    h.pause("Writer");
    let run = h.runtime.send_from_human(writer, "First thing: book the 4pm slot.").unwrap();
    // Distinct millisecond stamps, so the second line is provably newer than
    // the batch. Two lines in one millisecond have no order to restore.
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
    // agent's channel rather than traveling to whoever it was aimed at.
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
async fn a_progress_note_is_in_the_next_turn_with_how_long_ago_it_was_written() {
    // The other half of the write-manage-read loop, and the half that expires.
    // An agent notes what it is waiting on, and finds it on its next turn under
    // an age, which is what tells it whether to keep waiting.
    let stub = serve(|body| {
        let system = body["messages"][0]["content"].as_str().unwrap_or_default();
        if system.contains("waiting on the legal read") {
            // Said back so the assertion below is about the prompt rather than
            // about anything this stub knows.
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
    // And nowhere near the memory, which is the entire point of the split.
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
    // The count is how an agent learns the list is bounded without being
    // lectured about it in the tool description on every turn, and it is what
    // says an older note has just gone.
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
    // The one mechanical brake on a bounded list filling with a single fact.
    // "Still waiting on the legal read", noted on four turns, is four of
    // sixteen slots spent on one thing and three notes that record nothing.
    // Acknowledging the repeat is what teaches an agent that restating is how
    // you say something is still true, so it is not acknowledged: it is told
    // how old the note it already has is, which is what it was reaching for and
    // what it needs to decide whether to chase.
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
async fn an_agent_calling_the_old_name_for_its_memory_still_writes_it() {
    // `update_notes` is what this tool was called for a year. A model that
    // learned Guaca from an older transcript still reaches for it, and refusing
    // it would spend a turn on a rename the agent had no way to hear about.
    // What is recorded is the current name either way, so the rename does not
    // fork the transcript from today onward.
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
    // What the operator wants from the transcript is what changed, and a
    // rewrite is the whole file either way: two pages of near-identical
    // markdown compared by eye. So the call carries what it overwrote, and it
    // is carried rather than looked up because by the time anybody reads the
    // channel the file says something else again.
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
    // The operator reads an agent's memory in the column beside it, and the
    // moment they are most likely to be doing that is while the agent is
    // working, which is the moment the agent rewrites the file. Without this
    // the page on screen is the one that was true when the panel was drawn and
    // the only way to find out otherwise is to click away and back.
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
    // The panel reads the file again on this event, so one emitted for a write
    // that did not land is a read that reports the same page it already had,
    // in front of an operator who has just been told something changed.
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Done.".into())
        } else {
            Script::Notes("Smith handles verification.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    // A file where the workspace directory has to go: every write fails.
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
async fn a_turn_that_speaks_twice_is_watched_with_the_break_between_its_rounds() {
    // A turn is several model calls under one placeholder, and a model that
    // narrates its work says something before each tool call. The message that
    // lands puts a blank line between those sentences; the bubble the operator
    // watched being written has to put one there too, or every round after the
    // first begins in the middle of the sentence the last one ended with:
    // "...who is here.Two of us."
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
    // The failure, verbatim: a plugin that had just started answering again,
    // two checks left to run, rounds and budget to run them in, and a closing
    // line of "Checking both properly". A turn ends when the model stops
    // calling tools, so that sentence was the end of it. The operator read a
    // promise, waited, and had to send another message to get the work done.
    //
    // The prompt now says a message ends the turn, which is the fix for a model
    // that reads its prompt. This is the fix for the rest of them.
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
    // The promise stays in the message rather than being retracted. The
    // operator watched it being written, so a finished message without it would
    // disagree with the bubble they read; left where it is, it reads as the
    // sentence before the answer, which is what it turned out to be.
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
    // The bound. A model that answers the nudge with the same sentence is not
    // going to be talked into working, and without a cap this is a turn that
    // spends every round it has and the budget behind them on one promise.
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
    // What the check costs when it is wrong, which is what decides how tight it
    // has to be. A turn closing on a report is the overwhelmingly common case
    // and it must not buy a second model call.
    let stub =
        serve(|_| Script::Say("Checked both. Drive is stale and Gmail has nothing.".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "is the plugin back?").unwrap();
    h.settle(run).await;

    assert_eq!(calls_by_agent(&stub).get("Manager"), Some(&1), "a report is not a promise");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_retried_round_opens_its_replacement_at_the_left_margin() {
    // The other half of that break. A retry throws away what was drawn and
    // starts a new placeholder, so the round it reopens for is the first thing
    // in that bubble however many rounds came before it. Deciding the break
    // from what has been collected rather than from what is on screen puts a
    // blank line at the top of every recovered turn.
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
    // And the record is unaffected: it kept the round the screen threw away.
    assert_eq!(h.channel_texts("Manager").pop().unwrap(), "Checking who is here.\n\nTwo of us.");
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
async fn a_turn_says_what_it_reached_for_before_it_knows_how_it_went() {
    // The wait this exists for: a turn can spend ten minutes working through
    // tool results and publish nothing about any of them until it ends. The
    // operator watching had a pulsing avatar and a line of prose the model
    // wrote about itself, which is what a turn says it is doing rather than
    // what it did.
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

    // Reported before the call is made, which is the half that matters: a
    // command can sit for a minute, and a call only reported once it comes back
    // is silence for exactly as long as the wait it was meant to explain.
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
    // Paired by the provider's own id. Two identical calls in one turn are two
    // calls, and an outcome filed against the wrong one is a chip that says a
    // call succeeded when its twin is the one that did.
    assert_eq!(opened, closed);

    // And what the operator watched land is what the transcript holds: the same
    // part, not a second reading of it. One function draws the chip from this
    // value at both ends, so they cannot disagree about a call, and whatever a
    // call has to say for itself next reaches both of them or neither.
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

    // One call, one record. What was watched happening is the same work as what
    // the transcript keeps, not a second entry beside it.
    assert_eq!(
        events.iter().filter(|e| matches!(e, UiEvent::ToolFinished { .. })).count(),
        recorded.len()
    );
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
async fn the_agent_the_operator_wrote_to_still_reports_after_it_answers_its_crew() {
    // The other half of the rule above, and what it shipped as. The same turn
    // shape means the opposite thing for the one agent the operator actually
    // wrote to: it answers its crew with `send_message` and closes the turn
    // addressing the operator, because delegating and then saying where the
    // work got to is the whole of its job.
    //
    // Every turn it runs after the first is woken by a peer, so `ToPeer` is the
    // only mode it is ever in again and this is the only path its report has:
    // `send_message` reaches agents, and there is no name for the operator.
    // Filed as commentary, ten reports in one real run reached nobody while the
    // operator asked why nothing was happening and four agents worked.
    let stub = serve(|body| {
        if speaker(body) == "Chef" {
            if anyone_said(body, "Now cost the fish") {
                Script::Say("Fish is costed.".into())
            } else if has_tool_result(body) {
                // The worker half, in the same run: Chef has already answered
                // and the operator was never in this exchange.
                Script::Say("Told Manager about the soup.".into())
            } else {
                // An answer declared as work, which is what comes back up a
                // delegation and what puts the manager in `ToPeer` for the
                // rest of the run.
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

    // And the rule it is carved out of still holds inside the same run: Chef
    // answered Manager and the operator was never in that exchange.
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
    // The carve-out in the rule above, and the reason it is not "drop whatever
    // a turn trails". Text written after the answer went is a restatement of
    // it; a file is not. `send_message` carries its own files, so one attached
    // afterward has reached nobody at all, and the agent that asked for the
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
                // over afterward, then say so.
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
    // Found in a real session. The operator authorized an external send, the
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
        } else if text.contains("Sent.") {
            // The answer to the second instruction came back, so the Manager
            // can close the loop it opened.
            Script::Say("Done: Chef sent the mailing.".into())
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
    // The instruction was work, so its answer comes back to the agent that
    // gave it rather than stranding in Chef's own channel: the Manager can
    // assemble what came of the work it placed.
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
    // Found in a real session, and the exact shape of it. The operator asked
    // for an email, the coordinator relayed it, the peer opened the document
    // and reported back, and the coordinator then issued the actual send
    // instruction. That instruction was delivered, and the peer said nothing.
    //
    // Nothing was waiting on a reply, so the turn ran in the mode that tells an
    // agent nobody is asking it for anything and silence is usually right. It
    // spent a model call and complied. From the operator's side an agent had
    // simply stopped. Work re-arms the reply path now, so the second
    // instruction runs as an ordinary reply turn: the prompt says someone is
    // waiting, and the answer lands with them.
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
                // The old failure, kept as a tripwire: if the second
                // instruction ever runs in the silent mode again, this arm
                // plays the model that reads its prompt and complies, and the
                // assertion below fails on an agent that went quiet.
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
        "work expects an answer wherever in the exchange it lands; only a courtesy into a \
         settled pair does not"
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
            inference: Some(InferenceOverrides {
                default_model: Some("local/qwen".into()),
                ..Default::default()
            }),
            ..Default::default()
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
    // A limit is a statement about one crew's work, not about the app. The stub
    // answers every call with another tool call, so each agent runs until
    // something stops it, and the model name on the request is what says which
    // group's ceiling did.
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

    // And the crew that stopped is told why, in the group's own numbers.
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
            false,
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
            false,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_routine_that_skips_lands_on_the_idle_agent_and_not_on_the_working_one() {
    // Two agents, one sweep, the same routine, opposite answers. The point of
    // the option is a sweep that must not stack: an agent still working through
    // the last one does not want this one waiting behind it, and the next slot
    // is an hour away regardless.
    //
    // Busy is parked on a permission request, which is a turn genuinely in
    // flight and held open until somebody answers it. Idle has been sent
    // nothing.
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

    h.runtime.start_scheduler();
    h.wait_until("both routines to come due", |h| {
        booked.values().all(|r| !h.runtime.store().routine_runs(r.id, 20).unwrap().is_empty())
    })
    .await;

    // The working one was passed over, and the history says so rather than
    // leaving a gap: a firing that vanishes without trace reads exactly like a
    // scheduler that has stopped running.
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

    // Dropped, not deferred: the slot moved on exactly as it would have if the
    // firing had happened, so nothing comes due again on the next tick.
    let after = h.runtime.store().get_routine(booked["Busy"].id).unwrap().unwrap();
    assert!(
        after.next_run_at.expect("a clock routine holds a slot") > now_ms(),
        "a skipped firing must not still be due, or it fires the moment the agent goes quiet"
    );
    assert_eq!(after.last_run_at, Some(after.next_run_at.unwrap() - 3_600_000));

    // And the same routine on an agent with nothing in hand is delivered
    // normally, which is what stops this being a way to switch a schedule off.
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
    // The default, and the one that must not change. An agent busy at nine is
    // still the agent that has to send the report, so its firing queues and is
    // read when the turn in front of it finishes.
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

    h.runtime.start_scheduler();
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
            false,
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
            false,
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
        recognized: true,
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
    // A crew with browsers, because a session is only worth naming to an agent
    // that could go and use it: what an agent is told it can reach is filtered
    // by the places it actually has.
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
async fn an_account_on_a_place_that_was_taken_back_is_named_to_nobody() {
    // A sign-in outlives the place it was found on, deliberately: the profile
    // holding those cookies is kept when a browser is taken back, so the
    // account is still there if it is given back. Naming it meanwhile is the
    // overclaim that is worse than saying nothing — the agent decides it has
    // access and hits a login wall, or a peer routes the work to it.
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
    // The row is still there for when the browser is given back.
    assert_eq!(h.runtime.store().agent_signins(h.id("Researcher")).unwrap().len(), 1);
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
async fn an_agent_made_by_an_agent_is_given_neither_place_and_its_maker_is_told() {
    // A crew that could staff itself with machines would route around the
    // decision entirely: one agent with a computer makes three more and the
    // operator finds out from the bill. And the maker has to hear it, or it
    // hands the web to a peer that cannot reach it and reports the work as
    // delegated.
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
    // The live failure this exists for. The operator authorized an email, the
    // coordinator relayed the authorization, and the sending agent declined:
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
            Script::AskPermission {
                action: "Email the SCDOT response to robert@madebywelch.com for review".into(),
                because: "Manager says the operator authorized it; a peer's word is not \
                          permission to send mail in their name."
                    .into(),
            }
        }
    })
    .await;

    // A workspace with a machine, because an agent with no way out of the
    // workspace has nothing for the operator to authorize and is refused
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
            Script::AskPermission {
                action: "Email the response to the procurement officer".into(),
                because: "asked by Manager".into(),
            }
        }
    })
    .await;

    // A workspace with a machine, because an agent with no way out of the
    // workspace has nothing for the operator to authorize and is refused
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
async fn a_computer_belongs_to_an_agent_the_operator_gave_one_to_and_not_to_the_workspace() {
    // One key, one workspace, two agents, and only one of them was given a
    // machine. The key used to decide on its own, which meant an operator who
    // wanted a crew where one agent runs commands and the rest only talk had no
    // way to say so: every agent was offered `run_command` and the first one to
    // think of it rented itself a sandbox.
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
    // The harness gives the crew whatever the provider makes possible, so this
    // is the operator's decision under test: Talker is not to have one.
    h.runtime.store().set_has_computer(h.id("Talker"), false).unwrap();

    let refused =
        h.runtime.send_from_human(h.id("Talker"), "What kernel is that machine on?").unwrap();
    h.settle(refused).await;
    let allowed = h.runtime.send_from_human(h.id("Runner"), "Anything to do?").unwrap();
    h.settle(allowed).await;

    // Decided before the first token, which is the half that costs nothing: a
    // tool offered for a place an agent does not have is a model call and a
    // turn spent finding out.
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

    // And the half that is load-bearing: a model calling a name it was never
    // offered is refused rather than served, or the tool list is decoration.
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

    // The operator hears about it in words, in the channel they asked in.
    assert!(
        h.channel_texts("Talker").iter().any(|t| t.contains("no computer")),
        "and the turn still ends in an answer:\n{}",
        h.transcript()
    );
    h.expect_normal(refused, "a refused tool call is an ordinary turn");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_screen_is_neither_offered_nor_served_to_a_model_that_cannot_be_shown_one() {
    // The agent has the machine. What it does not have is a model that can be
    // sent a picture, and `use_screen` hands back nothing else: the coordinates
    // to click next are in the picture and nowhere else. Both halves are
    // tested, because a model names tools it was never offered and the tool
    // list alone is decoration.
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

    // Decided before the first token, which is the half that costs nothing.
    let offered = tools_by_agent(&stub);
    assert!(!offered["Runner"].contains(&"use_screen".to_string()), "{offered:?}");
    // And the rest of the machine is untouched: a shell answers in text, and a
    // program on the desktop is there for the operator to watch.
    assert!(offered["Runner"].contains(&"run_command".to_string()), "{offered:?}");
    assert!(offered["Runner"].contains(&"open_on_desktop".to_string()), "{offered:?}");

    // The load-bearing half. Served, the machine would be worked, the screen
    // captured and the picture thrown away, which reads to the model as a
    // screen that came back blank.
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
    // The other place, and a separate decision: a crew where one agent reads
    // the web and nobody else leaves the workspace is the ordinary shape. The
    // gate is the same three lines with one boolean changed, which is exactly
    // the shape a copy-paste error hides in.
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
    // The live failure. An agent worked out that it could not reach the
    // operator's calendar, and asked them for permission to have it. This
    // workspace has no computer and no browser, so there was no action to
    // authorize and nothing a click could have handed over: what was missing
    // was access. The operator got a decision that changed nothing instead of a
    // sentence saying what they would have to add.
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
    // a peer that has already answered is instructed again. Work re-arms the
    // reply path, so the second instruction expects an answer and the turn
    // runs as an ordinary reply turn — whose silence used to be the invisible
    // kind. A turn that produced no text produced no envelope either, so there
    // was nothing in Chef's channel, nothing in the feed, nothing for the
    // Manager that was owed the answer, and an agent that to the operator had
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
        instruction.expects_reply,
        "work re-arms the reply path even for a peer that has already answered"
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
async fn an_agent_reading_an_acknowledgment_may_still_say_nothing_quietly() {
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

// ---- saying it has stopped, without stopping ------------------------------

/// Everything the model was sent this call, as one string.
///
/// Deliberately every message rather than only the last: what these stubs
/// branch on is whether a tool has already answered, and that answer arrives as
/// a message of its own. It reads the system prompt too, which is what one of
/// them is asking about.
fn said(body: &serde_json::Value) -> String {
    body["messages"]
        .as_array()
        .map(|m| m.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default()
}

/// Whether `escalate` has already answered this turn.
///
/// Two phrases because there are deliberately two answers, and the difference
/// between them is the feature: a first raise is told it is on the desk, and a
/// repeat is told how long the operator has had it and how many turns have gone
/// into the same wall. A stub keyed on one of them loops on the other.
fn escalated(body: &serde_json::Value) -> bool {
    let said = said(body);
    said.contains("on the operator's desk now") || said.contains("You already had that up")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_that_cannot_go_on_puts_it_on_the_desk_and_finishes_its_turn() {
    // The case neither of the two parking tools could cover. Nothing here can
    // be answered inside a turn: the operator has to go and do something, and
    // ten minutes of a parked turn would end with the wall still there and a
    // run booking spent. So the agent says so on its way out and carries on,
    // and what it said becomes a row rather than a paragraph in a channel
    // nobody has open.
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

    // Nothing parked, and that is the point rather than a gap: the turn ran to
    // the end and the agent was never held anywhere.
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
    // Six turns into one wall is one thing to deal with, and the operator has
    // to be able to see that it is six. A row each would be a desk that reads
    // as six problems; a silent second raise would be a desk that reads as one
    // bad afternoon.
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
    // An agent that cannot see what it has already escalated raises it again as
    // news every turn, which is the behavior in a channel that this replaced.
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

    // Cleared is not answered. Nothing was waiting on it, so nothing resumed,
    // and the agent is not told: what unblocks it is a message in its channel.
    let again = h.runtime.send_from_human(h.id("Analyst"), "And now?").unwrap();
    h.settle(again).await;
    assert_eq!(h.runtime.store().open_escalations(50).unwrap().len(), 1);
}

// ---- asking the operator what, rather than whether ------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_that_cannot_decide_asks_the_operator_and_carries_on_with_the_answer() {
    // The case `request_permission` could never cover. Nothing here needs
    // authorizing: the agent could take either road and does not know which the
    // operator wants. Before this existed its only moves were to guess, or to
    // write the question into a channel nobody was watching and stop.
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
async fn a_question_nobody_answers_records_no_answer_and_releases_the_turn() {
    // What this proves and what it does not, said plainly. A stop is the one
    // unanswered end an offline test can reach: the ten minute window is a real
    // wall clock and the stub here is a real server, so a paused clock is not
    // available. So this covers the row and the release — nothing is recorded
    // as said, and the turn is not left parked — and it does not cover what the
    // agent does next, which is a model's behavior and belongs to the evals.
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
    // Two surfaces draw these, and both draw both kinds. A card that offered
    // Allow and Deny for "which vendor" would settle the row without saying
    // anything, and the turn would resume having been told nothing: it reads
    // back the answer, and there would not be one.
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

// ---- the compost ---------------------------------------------------------

/// A crew whose model is never asked anything. The compost is entirely runtime
/// machinery, so what the model would have said is beside the point.
async fn quiet() -> Stub {
    serve(|_| Script::Say("ok".into())).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_deleted_agent_keeps_its_memory_while_it_waits() {
    // The whole argument for the compost. Deleting used to take the memory on
    // the click, and a restore that came back with an agent that had forgotten
    // its work would not be worth offering.
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
    // Addressable again: paused queues rather than refuses, which is the
    // difference between a restore and a resume.
    let run = h.runtime.send_from_human(id, "welcome back").unwrap();
    h.runtime.stop_run(run);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restore_steps_around_a_name_the_crew_gave_away() {
    // A composted agent frees its name at once, which is what lets the crew
    // hire a replacement. Without settling it, the restore dies on the unique
    // index and the operator is shown a database error for a button whose job
    // is to succeed.
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
    // The promise the panel draws a countdown against. Nothing else in the
    // build empties the compost, so without this the feature is a list that
    // grows forever and a memory that is never actually deleted.
    let stub = quiet().await;
    let h = harness(&stub, &["Researcher", "Scribe"], GuardLimits::default());
    let (due, waiting) = (h.id("Researcher"), h.id("Scribe"));

    for id in [due, waiting] {
        h.runtime.workspace().write(id, "whoever", "something worth keeping").unwrap();
    }
    // Stamped through the store rather than through the runtime, because one of
    // them has to be dated a month ago and that is the one thing a test cannot
    // wait for. The stamp is all the sweep reads.
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
