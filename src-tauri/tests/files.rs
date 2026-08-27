//! Files moving between the operator, the agents, and their machines.
//!
//! The thing being tested is not that bytes were stored. It is that the file
//! reaches the model in a form it can act on, and that when it cannot, the
//! model is told rather than left to talk about a document nobody sent. A
//! silently dropped attachment is the worst failure available here: everyone
//! involved, agent and operator, believes the file arrived.
//!
//! What CI can reach is everything up to the sandbox. Placing a document on an
//! agent's machine needs a real E2B key, so those paths are exercised here for
//! their failure behavior and verified for real by `./scripts/evals.sh`.

mod harness;

use guac_lib::domain::envelope::{Envelope, Part, Participant};
use guac_lib::runtime::guard::GuardLimits;

use harness::*;

/// Every file part in an agent's channel, oldest first.
fn files_in(h: &Harness, agent: &str) -> Vec<String> {
    h.runtime
        .store()
        .channel_messages(h.id(agent), 200)
        .unwrap()
        .iter()
        .flat_map(|envelope: &Envelope| {
            envelope.parts.iter().filter_map(|part| match part {
                Part::File(file) => Some(file.name.clone()),
                _ => None,
            })
        })
        .collect()
}

/// Everything the stub was sent, flattened, including image parts.
fn prompts(stub: &Stub) -> String {
    stub.transcript.lock().iter().map(|body| body.to_string()).collect::<Vec<_>>().join("\n")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_text_file_the_operator_drops_is_read_to_the_agent() {
    let stub =
        serve(|_| Script::Say("Three risks, and the second is the one to worry about.".into()))
            .await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());

    let brief = h.runtime.files().put("brief.md", b"# Brief\n\nThe deadline is the 14th.").unwrap();
    let run = h
        .runtime
        .send_from_human_with(h.id("Manager"), "What are the risks?", vec![brief])
        .unwrap();
    h.settle(run).await;

    let sent = prompts(&stub);
    assert!(sent.contains("The deadline is the 14th"), "the model never saw the file:\n{sent}");
    assert!(sent.contains("brief.md"), "and it has to know what the file was called:\n{sent}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_picture_is_handed_over_as_a_picture() {
    // The one thing a model cannot be told about in words. This goes down the
    // same path as a screenshot from `use_screen`, which is the proof that the
    // shape is one a real provider accepts.
    let stub = serve(|_| Script::Say("A red square.".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());

    // A one-pixel PNG, so the bytes are real rather than a string pretending.
    let png: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde,
    ];
    let shot = h.runtime.files().put("chart.png", png).unwrap();
    let run = h.runtime.send_from_human_with(h.id("Manager"), "What is this?", vec![shot]).unwrap();
    h.settle(run).await;

    let sent = prompts(&stub);
    assert!(sent.contains("image_url"), "a picture has to travel as an image part:\n{sent}");
    assert!(sent.contains("data:image/png;base64,"), "{sent}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_picture_is_not_sent_to_a_model_that_cannot_be_shown_one() {
    // The other half of the test above, and the reason any of this is decided
    // rather than assumed. An operator swaps the model in the box for one that
    // takes text only; the picture would be refused by the endpoint, which
    // costs the whole turn rather than the attachment. So it is not sent, and
    // the agent is told in words what it is holding and what it is not.
    let listing = serde_json::json!({
        "data": [{
            "id": "test/model",
            "architecture": { "input_modalities": ["text"], "output_modalities": ["text"] },
        }]
    });
    let stub = serve_publishing(Some(listing), |_| Script::Say("I cannot see it.".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());

    let png: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde,
    ];
    let shot = h.runtime.files().put("chart.png", png).unwrap();
    let run = h.runtime.send_from_human_with(h.id("Manager"), "What is this?", vec![shot]).unwrap();
    h.settle(run).await;

    let sent = prompts(&stub);
    assert!(!sent.contains("image_url"), "the endpoint said it takes text only:\n{sent}");
    assert!(!sent.contains("data:image/png;base64,"), "{sent}");
    // Not silence. A model that finds no picture and no explanation talks about
    // the file from its name, confidently, which is the failure this replaces.
    assert!(sent.contains("chart.png"), "the agent has to know a file arrived:\n{sent}");
    assert!(sent.contains("pictures do not reach the model"), "{sent}");
    // And the prompt agrees with the delivery, rather than telling an agent it
    // can see and then handing it nothing.
    assert!(sent.contains("You read text, and only text"), "{sent}");
    assert!(!sent.contains("use_screen"), "a screen it cannot be shown is not offered:\n{sent}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_picture_is_handed_over_once_and_never_again() {
    // A screenshot off a retina display is over a megabyte, and base64 is a
    // third bigger again. Attaching it to every turn that follows would make
    // each one slower and dearer than the last, and a request big enough
    // eventually fails to send at all. It travels with the message it arrived
    // on; after that the history says only that a file was there.
    let stub = serve(|_| Script::Say("Seen.".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());

    let png: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde,
    ];
    let shot = h.runtime.files().put("screenshot.png", png).unwrap();
    let first =
        h.runtime.send_from_human_with(h.id("Manager"), "what is this?", vec![shot]).unwrap();
    h.settle(first).await;

    let then = h.runtime.send_from_human(h.id("Manager"), "and what should I do?").unwrap();
    h.settle(then).await;

    let carrying = stub
        .transcript
        .lock()
        .iter()
        .filter(|body| body.to_string().contains("data:image/png"))
        .count();
    assert_eq!(carrying, 1, "the picture was sent again on a later turn");

    // The later turn still knows the file existed, which is the point of
    // announcing it in the history rather than re-sending it.
    let last = stub.transcript.lock().last().cloned().unwrap().to_string();
    assert!(last.contains("screenshot.png"), "the history lost the file entirely:\n{last}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_text_file_is_read_out_once_and_not_on_every_turn_after() {
    // The same arithmetic, quieter: 24k characters of a brief re-read into
    // every following prompt is the history window filled with one document.
    let stub = serve(|_| Script::Say("Read.".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());

    let brief = h.runtime.files().put("brief.md", b"THE-CONTENTS-OF-THE-BRIEF").unwrap();
    let first = h.runtime.send_from_human_with(h.id("Manager"), "read this", vec![brief]).unwrap();
    h.settle(first).await;
    let then = h.runtime.send_from_human(h.id("Manager"), "and now?").unwrap();
    h.settle(then).await;

    let carrying = stub
        .transcript
        .lock()
        .iter()
        .filter(|body| body.to_string().contains("THE-CONTENTS-OF-THE-BRIEF"))
        .count();
    assert_eq!(carrying, 1, "the file was read into the prompt again on a later turn");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_document_that_cannot_be_placed_is_admitted_rather_than_described() {
    // No sandbox in CI, so this is the failure path, and the failure path is
    // the one that matters: an agent told nothing would answer about a
    // proposal it has never read.
    let stub = serve(|_| Script::Say("I cannot open it yet.".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());

    let draft = h.runtime.files().put("proposal.docx", b"PK\x03\x04 not really a docx").unwrap();
    let run = h.runtime.send_from_human_with(h.id("Manager"), "Review this.", vec![draft]).unwrap();
    h.settle(run).await;

    let sent = prompts(&stub);
    assert!(sent.contains("proposal.docx"), "{sent}");
    assert!(
        sent.contains("could not be put on your machine"),
        "the model has to be told the file is out of reach:\n{sent}"
    );
    assert!(
        sent.contains("rather than describing a file you have not read"),
        "and what to do about it:\n{sent}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_with_no_covering_note_still_reaches_the_agent() {
    // Handing over a document with nothing typed is the most natural way to
    // send one. Judging a message empty by its text alone dropped it.
    let stub = serve(|_| Script::Say("Read it.".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());

    let notes = h.runtime.files().put("agenda.txt", b"1. budget\n2. hiring").unwrap();
    let run = h.runtime.send_from_human_with(h.id("Manager"), "", vec![notes]).unwrap();
    h.settle(run).await;

    let sent = prompts(&stub);
    assert!(
        sent.contains("agenda.txt"),
        "a message that is only a file is still a message:\n{sent}"
    );
    assert!(sent.contains("2. hiring"), "{sent}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_forwards_a_file_it_was_given_without_needing_a_machine() {
    // The case that started this: a coordinator holding a draft and a colleague
    // that has to send it. Forwarding is host-side, so it works with no
    // sandbox at all.
    let stub = serve(|body| {
        let who = speaker(body);
        if who == "Chef" {
            Script::Say("Got the brief.".into())
        } else if has_tool_result(body) {
            Script::Say("Passed it to Chef.".into())
        } else {
            Script::SendFiles {
                recipients: vec!["Chef".into()],
                text: "here is the brief".into(),
                files: vec!["brief.md".into()],
            }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let brief = h.runtime.files().put("brief.md", b"the deadline is the 14th").unwrap();
    let run = h
        .runtime
        .send_from_human_with(h.id("Manager"), "Send the brief to Chef.", vec![brief])
        .unwrap();
    h.settle(run).await;

    assert_eq!(
        files_in(&h, "Chef"),
        vec!["brief.md"],
        "the file itself has to arrive, not a mention of it:\n{}",
        h.transcript()
    );

    // And Chef was actually shown the contents, not just told a name.
    let chef_prompts: Vec<String> = stub
        .transcript
        .lock()
        .iter()
        .filter(|body| speaker(body) == "Chef")
        .map(|body| body.to_string())
        .collect();
    assert!(
        chef_prompts.iter().any(|p| p.contains("the deadline is the 14th")),
        "Chef was handed a file it could not read: {chef_prompts:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_an_agent_never_had_is_reported_rather_than_imagined() {
    // Without this the sender believes it attached something and goes on to
    // discuss a document the recipient has never seen.
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("It did not go.".into())
        } else {
            Script::SendFiles {
                recipients: vec!["Chef".into()],
                text: "the contract, attached".into(),
                files: vec!["contract.pdf".into()],
            }
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Send Chef the contract.").unwrap();
    h.settle(run).await;

    assert!(files_in(&h, "Chef").is_empty(), "nothing should have been attached");
    let told = tool_results(&stub).join("\n");
    assert!(told.contains("contract.pdf was not attached"), "the model has to be told: {told}");
    assert!(
        told.contains("do not tell them it is on the way"),
        "and told what not to do about it: {told}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_same_file_sent_to_three_agents_is_stored_once() {
    let stub = serve(|body| {
        if speaker(body) == "Manager" && !has_tool_result(body) {
            Script::SendFiles {
                recipients: vec!["Chef".into(), "Baker".into(), "Grocer".into()],
                text: "the menu".into(),
                files: vec!["menu.md".into()],
            }
        } else {
            Script::Say("noted".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Chef", "Baker", "Grocer"], GuardLimits::default());
    let menu = h.runtime.files().put("menu.md", b"soup, then fish").unwrap();
    let digest = menu.digest.clone();
    let run = h
        .runtime
        .send_from_human_with(h.id("Manager"), "Send everyone the menu.", vec![menu])
        .unwrap();
    h.settle(run).await;

    for peer in ["Chef", "Baker", "Grocer"] {
        assert_eq!(files_in(&h, peer), vec!["menu.md"], "{peer} did not get it");
    }
    // Four references, one file: the digest is the address, and a fan-out that
    // copied the bytes per recipient would make a document expensive to share.
    assert_eq!(h.runtime.files().read(&digest).unwrap(), b"soup, then fish");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_never_leaks_into_the_text_of_a_message() {
    // `plain_text` feeds prompt assembly, the guard's duplicate fingerprint and
    // every channel preview. A document that leaked into it would be pasted
    // into all three, and two different files sent with the same note would
    // look like the same message to the guard.
    let stub = serve(|_| Script::Say("Read.".into())).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());

    let file = h.runtime.files().put("secrets.txt", b"the-actual-contents").unwrap();
    let run = h.runtime.send_from_human_with(h.id("Manager"), "read this", vec![file]).unwrap();
    h.settle(run).await;

    let stored = h
        .runtime
        .store()
        .channel_messages(h.id("Manager"), 50)
        .unwrap()
        .into_iter()
        .find(|e| e.from == Participant::Human)
        .expect("the operator's message is in the channel");
    assert_eq!(stored.plain_text(), "read this");
}

/// Every file part on a message this agent sent to the operator, oldest first.
fn handed_over(h: &Harness, agent: &str) -> Vec<String> {
    h.runtime
        .store()
        .channel_messages(h.id(agent), 200)
        .unwrap()
        .iter()
        .filter(|envelope: &&Envelope| envelope.to == Participant::Human)
        .flat_map(|envelope| {
            envelope.parts.iter().filter_map(|part| match part {
                Part::File(file) => Some(file.name.clone()),
                _ => None,
            })
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_document_an_agent_attaches_reaches_the_operator() {
    // The failure this exists for: an agent wrote a brief, saved it, and ended
    // its turn with the path. The operator was handed a location on a machine
    // that is not theirs, in a window with nothing to click, and the document
    // may as well not have been written.
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Tidied up and attached. The second risk is the one to worry about.".into())
        } else {
            Script::Attach { tool: "attach_file".into(), files: vec!["notes.md".into()] }
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let notes = h.runtime.files().put("notes.md", b"# Risks\n\n1. cost\n2. the vendor").unwrap();
    let run = h
        .runtime
        .send_from_human_with(h.id("Manager"), "Tidy this up and give it back.", vec![notes])
        .unwrap();
    h.settle(run).await;

    assert_eq!(
        handed_over(&h, "Manager"),
        vec!["notes.md"],
        "the operator was told about a file rather than given one:\n{}",
        h.transcript()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_document_handed_over_with_nothing_said_still_arrives() {
    // Sending a document with no covering note is a normal thing to do, and
    // models do it constantly. A reply judged empty by its text alone would
    // drop the file the whole turn was spent producing, which is the one
    // failure worse than not having the feature: the agent believes it handed
    // the document over and the operator never sees it.
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("".into())
        } else {
            Script::Attach { tool: "attach".into(), files: vec!["agenda.txt".into()] }
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let agenda = h.runtime.files().put("agenda.txt", b"1. budget\n2. hiring").unwrap();
    let run = h
        .runtime
        .send_from_human_with(h.id("Manager"), "Hand that back to me.", vec![agenda])
        .unwrap();
    h.settle(run).await;

    assert_eq!(
        handed_over(&h, "Manager"),
        vec!["agenda.txt"],
        "a reply with a file and no words is still a reply:\n{}",
        h.transcript()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_document_that_could_not_be_attached_is_admitted_rather_than_claimed() {
    // No sandbox in CI, and no E2B key, so this agent has no computer at all.
    // That is the case this covers, and it is the one that reached an operator:
    // an agent told nothing goes on to say the brief is attached to a message
    // that carries no file, and an agent given advice it cannot act on rewords
    // the claim and tries again.
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("I could not hand the file over.".into())
        } else {
            Script::Attach { tool: "attach_file".into(), files: vec!["/home/user/brief.md".into()] }
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Write me a brief.").unwrap();
    h.settle(run).await;

    assert!(
        handed_over(&h, "Manager").is_empty(),
        "nothing should have been attached:\n{}",
        h.transcript()
    );
    let told = tool_results(&stub).join("\n");
    assert!(told.contains("/home/user/brief.md was not attached"), "{told}");
    assert!(
        told.contains("do not tell them it is attached"),
        "the model has to be told what not to claim: {told}"
    );
    assert!(
        told.contains("nothing to retry"),
        "no path was ever going to resolve, so trying again is a wasted turn: {told}"
    );
    assert!(
        told.contains("put it in your answer as text"),
        "and given a way forward it can actually take: {told}"
    );
    // The bug this closes. The advice used to be "check the path with
    // `run_command` and attach it again", and `run_command` is not offered to
    // an agent with no computer either. One agent followed it twice.
    assert!(
        !told.contains("run_command"),
        "an agent with no computer is not offered that either: {told}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_same_document_attached_twice_in_one_turn_is_one_file() {
    // A model that attaches the brief, writes a paragraph, then attaches the
    // brief again would otherwise put two identical cards under one message.
    let stub = serve(|body| {
        let calls = body["messages"]
            .as_array()
            .map(|m| m.iter().filter(|msg| msg["role"] == "tool").count())
            .unwrap_or(0);
        match calls {
            0 | 1 => Script::Attach { tool: "attach_file".into(), files: vec!["menu.md".into()] },
            _ => Script::Say("Attached.".into()),
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let menu = h.runtime.files().put("menu.md", b"soup, then fish").unwrap();
    let run =
        h.runtime.send_from_human_with(h.id("Manager"), "Give me the menu.", vec![menu]).unwrap();
    h.settle(run).await;

    assert_eq!(
        handed_over(&h, "Manager"),
        vec!["menu.md"],
        "the same document arrived twice:\n{}",
        h.transcript()
    );
    let told = tool_results(&stub).join("\n");
    assert!(told.contains("Nothing new was attached"), "the second call has to say so: {told}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_reads_its_own_attachment_back_on_the_next_turn() {
    // Without this the agent has no record of having handed anything over: it
    // attaches the same document again next turn and tells the operator it is
    // sending it for the first time.
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Attached.".into())
        } else if reading_peer_replies(body) {
            Script::Say("Already sent.".into())
        } else {
            Script::Attach { tool: "attach_file".into(), files: vec!["brief.md".into()] }
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let brief = h.runtime.files().put("brief.md", b"the deadline is the 14th").unwrap();
    let first =
        h.runtime.send_from_human_with(h.id("Manager"), "Hand me the brief.", vec![brief]).unwrap();
    h.settle(first).await;

    let then = h.runtime.send_from_human(h.id("Manager"), "Did you send it?").unwrap();
    h.settle(then).await;

    let last = stub.transcript.lock().last().cloned().unwrap();
    let assistant: Vec<String> = last["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m["role"] == "assistant")
        .map(|m| m["content"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        assistant.iter().any(|turn| turn.contains("brief.md")),
        "its own turn lost the file it attached: {assistant:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_document_is_written_and_handed_over_with_no_machine_anywhere() {
    // The hole under `attach_file`. There is no sandbox in CI and no E2B key,
    // so this agent has no computer, which is the state most agents are in.
    // Until `write_document` there was no way for one to produce a file at all:
    // it had the whole report in the turn and the only route to an attachment
    // ran through a shell command on a machine it did not have. One agent spent
    // four turns on that, twice invented a `/home/user` path, and delivered
    // eight pages as chat text.
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Written up and attached.".into())
        } else {
            Script::Write {
                tool: "write_document".into(),
                name: "readiness.md".into(),
                content: "# Readiness\n\nEverything is UNEXECUTED.".into(),
            }
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Write me the readiness report.").unwrap();
    h.settle(run).await;

    assert_eq!(
        handed_over(&h, "Manager"),
        vec!["readiness.md"],
        "the document has to arrive as a file:\n{}",
        h.transcript()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_document_written_this_turn_can_be_sent_on_in_the_same_turn() {
    // A document a moment old is in no channel yet: the message carrying it has
    // not landed. Resolved against the channel alone, an agent that wrote a
    // brief and passed it to a colleague was told its own document did not
    // exist, which is the failure this whole file is about, one step along.
    let stub = serve(|body| {
        // Counted rather than pattern-matched on the text: what round this is
        // decides what to play, and the tool results are how many have run.
        let results = body["messages"]
            .as_array()
            .map(|m| m.iter().filter(|msg| msg["role"] == "tool").count())
            .unwrap_or(0);
        match results {
            0 => Script::Write {
                tool: "write_document".into(),
                name: "brief.md".into(),
                content: "# Brief\n\nThe short version.".into(),
            },
            1 => Script::SendFiles {
                recipients: vec!["Scribe".into()],
                text: "Here is the brief.".into(),
                files: vec!["brief.md".into()],
            },
            _ => Script::Say("Sent it on.".into()),
        }
    })
    .await;

    let h = harness(&stub, &["Manager", "Scribe"], GuardLimits::default());
    let run =
        h.runtime.send_from_human(h.id("Manager"), "Write a brief and give it to Scribe.").unwrap();
    h.settle(run).await;

    let told = tool_results(&stub).join("\n");
    assert!(
        !told.contains("was not attached") && !told.contains("did not get it"),
        "its own document has to resolve by name: {told}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_document_called_for_with_nothing_in_it_is_refused_with_the_reason() {
    // The mistake this catches is a model calling the tool intending to fill it
    // in on a later round. There is no later round, and an empty file attached
    // to an answer is the same lie as a phantom attachment.
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("I could not produce it.".into())
        } else {
            Script::Write {
                tool: "write_document".into(),
                name: "brief.md".into(),
                content: "   ".into(),
            }
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Write me a brief.").unwrap();
    h.settle(run).await;

    assert!(
        handed_over(&h, "Manager").is_empty(),
        "nothing should have been attached:\n{}",
        h.transcript()
    );
    let told = tool_results(&stub).join("\n");
    assert!(told.contains("no second call"), "it has to say there is no later round: {told}");
}
