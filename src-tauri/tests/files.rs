mod harness;

use guac_lib::domain::envelope::{Envelope, Part, Participant};
use guac_lib::runtime::guard::GuardLimits;

use harness::*;

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
    let stub = serve(|_| Script::Say("A red square.".into())).await;
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
    assert!(sent.contains("image_url"), "a picture has to travel as an image part:\n{sent}");
    assert!(sent.contains("data:image/png;base64,"), "{sent}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_picture_is_not_sent_to_a_model_that_cannot_be_shown_one() {
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
    assert!(sent.contains("chart.png"), "the agent has to know a file arrived:\n{sent}");
    assert!(sent.contains("pictures do not reach the model"), "{sent}");
    assert!(sent.contains("You read text, and only text"), "{sent}");
    assert!(!sent.contains("use_screen"), "a screen it cannot be shown is not offered:\n{sent}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_picture_is_handed_over_once_and_never_again() {
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

    let last = stub.transcript.lock().last().cloned().unwrap().to_string();
    assert!(last.contains("screenshot.png"), "the history lost the file entirely:\n{last}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_text_file_is_read_out_once_and_not_on_every_turn_after() {
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
    assert_eq!(h.runtime.files().read(&digest).unwrap(), b"soup, then fish");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_never_leaks_into_the_text_of_a_message() {
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
    assert!(
        !told.contains("run_command"),
        "an agent with no computer is not offered that either: {told}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_same_document_attached_twice_in_one_turn_is_one_file() {
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
    let stub = serve(|body| {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_document_created_on_an_earlier_turn_can_be_reopened_without_a_computer() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Done.".into())
        } else if anyone_said(body, "Reopen") {
            Script::Plugin {
                name: "read_file".into(),
                arguments: serde_json::json!({"name": "pilot-playbook.md"}),
            }
        } else {
            Script::Write {
                tool: "write_document".into(),
                name: "pilot-playbook.md".into(),
                content: "# Pilot\n\nThe verification code is ORCHID-4729.".into(),
            }
        }
    })
    .await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let first = h.runtime.send_from_human(h.id("Manager"), "Write the playbook.").unwrap();
    h.settle(first).await;
    let second = h.runtime.send_from_human(h.id("Manager"), "Reopen the playbook.").unwrap();
    h.settle(second).await;
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("ORCHID-4729"), "saved contents did not reach the model: {results}");
    assert_eq!(handed_over(&h, "Manager"), vec!["pilot-playbook.md"]);
}

fn saved_reply(h: &Harness, agent: &str, name: &str, bytes: &[u8]) -> Envelope {
    use guac_lib::domain::{
        envelope::{Intent, Trust},
        ids::{MessageId, RunId},
        now_ms,
    };
    Envelope {
        id: MessageId::new(),
        run_id: RunId::new(),
        channel_id: h.id(agent),
        from: Participant::Agent { id: h.id(agent) },
        to: Participant::Human,
        parts: vec![Part::File(h.runtime.files().put(name, bytes).unwrap())],
        trust: Trust::Peer,
        hop: 0,
        expects_reply: false,
        intent: Intent::Courtesy,
        cause: None,
        created_at: now_ms(),
    }
}

fn read_saved(body: &serde_json::Value, name: &str, offset: usize) -> Script {
    if has_tool_result(body) {
        Script::Say("Done.".into())
    } else {
        Script::Plugin {
            name: "read_file".into(),
            arguments: serde_json::json!({"name": name, "offset": offset}),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reading_a_saved_file_distinguishes_missing_inaccessible_and_unreadable() {
    let stub = serve(|body| read_saved(body, "brief.md", 0)).await;
    let h = harness(&stub, &["Manager", "Scribe"], GuardLimits::default());
    let secret = saved_reply(&h, "Scribe", "brief.md", b"PRIVATE-CONTENTS");
    h.runtime.store().append(&secret).unwrap();
    let run = h.runtime.send_from_human(h.id("Manager"), "Read the brief.").unwrap();
    h.settle(run).await;
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("No saved attachment"), "{results}");
    assert!(!prompts(&stub).contains("PRIVATE-CONTENTS"));

    let mut missing = saved_reply(&h, "Manager", "brief.md", b"lost");
    if let Part::File(file) = &mut missing.parts[0] {
        file.digest = "0".repeat(64);
    }
    h.runtime.store().append(&missing).unwrap();
    let run = h.runtime.send_from_human(h.id("Manager"), "Try reading the brief again.").unwrap();
    h.settle(run).await;
    let results = tool_results(&stub).join("\n");
    assert!(
        results.contains("could not be read") && results.contains("Ask for a new copy"),
        "{results}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_saved_file_outlives_history_and_includes_replies_filed_under_a_peer() {
    use guac_lib::domain::ids::MessageId;
    let stub = serve(|body| read_saved(body, "BRIEF.md", 0)).await;
    let h = harness(&stub, &["Manager", "Scribe"], GuardLimits::default());
    let mut old = saved_reply(&h, "Manager", "brief.md", b"OLD-VERSION");
    old.created_at = 1;
    h.runtime.store().append(&old).unwrap();
    let mut latest = saved_reply(&h, "Manager", "brief.md", b"LATEST-VERSION");
    latest.to = Participant::Agent { id: h.id("Scribe") };
    latest.channel_id = h.id("Scribe");
    latest.created_at = 2;
    h.runtime.store().append(&latest).unwrap();
    for i in 0..200 {
        let mut chatter = old.clone();
        chatter.id = MessageId::new();
        chatter.parts = vec![Part::text("Unrelated conversation.")];
        chatter.created_at = 3 + i;
        h.runtime.store().append(&chatter).unwrap();
    }
    let run = h.runtime.send_from_human(h.id("Manager"), "Read the brief.").unwrap();
    h.settle(run).await;
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("LATEST-VERSION") && !results.contains("OLD-VERSION"), "{results}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn long_saved_text_can_be_read_to_the_end_without_a_computer() {
    let stub = serve(|body| {
        let results =
            body["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "tool").count();
        match results {
            0 | 1 => Script::Plugin {
                name: "read_file".into(),
                arguments: serde_json::json!({"name": "long.md", "offset": results * 24_000}),
            },
            _ => Script::Say("Done.".into()),
        }
    })
    .await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let contents = format!("{}LAST-PARAGRAPH", "é".repeat(24_000));
    h.runtime.store().append(&saved_reply(&h, "Manager", "long.md", contents.as_bytes())).unwrap();
    let run = h.runtime.send_from_human(h.id("Manager"), "Read the entire document.").unwrap();
    h.settle(run).await;
    let results = tool_results(&stub);
    assert!(results[0].contains("offset 24000"), "{results:?}");
    assert!(!results[0].contains("LAST-PARAGRAPH"));
    assert!(
        results.last().unwrap().contains("LAST-PARAGRAPH")
            && results.last().unwrap().contains("End of file")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_saved_image_is_shown_again_only_when_requested() {
    let stub = serve(|body| read_saved(body, "chart.png", 0)).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    h.runtime.store().append(&saved_reply(&h, "Manager", "chart.png", b"image-bytes")).unwrap();
    let run = h.runtime.send_from_human(h.id("Manager"), "Inspect the chart again.").unwrap();
    h.settle(run).await;
    let transcript = stub.transcript.lock();
    assert!(!transcript[0].to_string().contains("data:image/png"));
    assert!(transcript.last().unwrap().to_string().contains("data:image/png;base64,"));
    assert!(!transcript
        .last()
        .unwrap()
        .to_string()
        .contains("This is what your screen looks like now."));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unsupported_saved_format_reports_why_it_cannot_be_opened() {
    let stub = serve(|body| read_saved(body, "brief.docx", 0)).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    h.runtime.store().append(&saved_reply(&h, "Manager", "brief.docx", b"PK fake docx")).unwrap();
    let run = h.runtime.send_from_human(h.id("Manager"), "Read the document.").unwrap();
    h.settle(run).await;
    let results = tool_results(&stub).join("\n");
    assert!(
        results.contains("could not be opened") && results.contains("ask for a text copy"),
        "{results}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "live: costs money, needs a configured model"]
async fn live_an_agent_reopens_its_saved_playbook() {
    use harness::live::{configured, live_crew, LiveAgent};
    let config = configured().expect("this live eval requires a configured model");
    let h = live_crew(config, &[LiveAgent::generic("Manager")]);
    let name = "virtual-try-on-boutique-pilot-playbook.md";
    let code = guac_lib::domain::ids::MessageId::new().to_string();
    h.runtime
        .store()
        .append(&saved_reply(
            &h,
            "Manager",
            name,
            format!("# Pilot playbook\n\nPilot verification code: {code}").as_bytes(),
        ))
        .unwrap();
    let run = h
        .runtime
        .send_from_human(
            h.id("Manager"),
            &format!(
                "What is the exact pilot verification code in the {name} you attached earlier?"
            ),
        )
        .unwrap();
    h.settle(run).await;
    let messages = h.runtime.store().channel_messages(h.id("Manager"), 50).unwrap();
    assert!(
        messages.iter().any(|message| message.from == Participant::Agent { id: h.id("Manager") }
            && message.plain_text().contains(&code)),
        "{}",
        h.transcript()
    );
    assert!(
        messages
            .iter()
            .flat_map(|message| &message.parts)
            .any(|part| matches!(part, Part::ToolCall {name, ..} if name == "read_file")),
        "{}",
        h.transcript()
    );
    println!("{}", h.transcript());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_long_attachment_points_to_the_rest_without_a_computer() {
    let stub = serve(|body| read_saved(body, "long.md", 24_000)).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let file = h
        .runtime
        .files()
        .put("long.md", format!("{}THE-END", "x".repeat(24_000)).as_bytes())
        .unwrap();
    let run = h
        .runtime
        .send_from_human_with(h.id("Manager"), "Read the whole brief.", vec![file])
        .unwrap();
    h.settle(run).await;
    assert!(prompts(&stub).contains("and offset 24000 to read the rest"));
    assert!(tool_results(&stub).join("\n").contains("THE-END"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_saved_picture_is_not_sent_to_a_text_only_model() {
    let listing = serde_json::json!({"data": [{"id": "test/model", "architecture": {"input_modalities": ["text"]}}]});
    let stub = serve_publishing(Some(listing), |body| read_saved(body, "chart.png", 0)).await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    h.runtime.store().append(&saved_reply(&h, "Manager", "chart.png", b"picture")).unwrap();
    let run = h.runtime.send_from_human(h.id("Manager"), "Read the picture.").unwrap();
    h.settle(run).await;
    assert!(!prompts(&stub).contains("data:image/png"));
    assert!(tool_results(&stub).join("\n").contains("could not be opened"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_written_this_turn_is_read_before_its_older_saved_version() {
    let stub = serve(|body| {
        let results =
            body["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "tool").count();
        match results {
            0 => Script::Write {
                tool: "write_document".into(),
                name: "brief.md".into(),
                content: "NEW-CONTENTS".into(),
            },
            1 => Script::Plugin {
                name: "read_file".into(),
                arguments: serde_json::json!({"name":"brief.md"}),
            },
            _ => Script::Say("Done.".into()),
        }
    })
    .await;
    let h = harness(&stub, &["Manager"], GuardLimits::default());
    h.runtime.store().append(&saved_reply(&h, "Manager", "brief.md", b"OLD-CONTENTS")).unwrap();
    let run =
        h.runtime.send_from_human(h.id("Manager"), "Revise the brief and read it back.").unwrap();
    h.settle(run).await;
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("NEW-CONTENTS") && !results.contains("OLD-CONTENTS"), "{results}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_missing_attachment_stops_the_whole_send_and_preserves_the_retry() {
    let stub = serve(|body| {
        if speaker(body) != "Manager" {
            return Script::Say("Received.".into());
        }
        let results =
            body["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "tool").count();
        match results {
            0 => Script::SendFiles {
                recipients: vec!["Chef".into(), "Scribe".into()],
                text: "The logo and brief are attached.".into(),
                files: vec!["brief.md".into(), "logo.png".into()],
            },
            1 => Script::Write {
                tool: "write_document".into(),
                name: "logo.png".into(),
                content: "recovered logo".into(),
            },
            2 => Script::SendFiles {
                recipients: vec!["Chef".into(), "Scribe".into()],
                text: "The logo and brief are attached.".into(),
                files: vec!["brief.md".into(), "logo.png".into()],
            },
            _ => Script::Say("Done.".into()),
        }
    })
    .await;
    let h = harness(
        &stub,
        &["Manager", "Chef", "Scribe"],
        GuardLimits { max_sends_per_pair: 1, ..GuardLimits::default() },
    );
    let brief = h.runtime.files().put("brief.md", b"brief").unwrap();
    let run =
        h.runtime.send_from_human_with(h.id("Manager"), "Send the assets.", vec![brief]).unwrap();
    h.settle(run).await;
    for peer in ["Chef", "Scribe"] {
        let messages = h.runtime.store().channel_messages(h.id(peer), 200).unwrap();
        let sends: Vec<_> = messages
            .iter()
            .filter(|m| {
                m.from == Participant::Agent { id: h.id("Manager") }
                    && m.plain_text() == "The logo and brief are attached."
            })
            .collect();
        assert_eq!(sends.len(), 1, "{}", h.transcript());
        assert_eq!(
            sends[0].parts.iter().filter(|p| matches!(p, Part::File(_))).count(),
            2,
            "the initial send must not deliver text or a partial file set"
        );
    }
    let messages = h.runtime.store().channel_messages(h.id("Manager"), 200).unwrap();
    assert!(messages.iter().flat_map(|m| &m.parts).any(|p| matches!(p, Part::ToolCall { name, outcome: guac_lib::domain::envelope::ToolOutcome::Failed { error }, .. } if name == "send_message" && error.contains("logo.png"))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn saved_metadata_without_bytes_cannot_send_a_phantom_attachment() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Could not send.".into())
        } else {
            Script::SendFiles {
                recipients: vec!["Chef".into()],
                text: "Attached.".into(),
                files: vec!["logo.png".into()],
            }
        }
    })
    .await;
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let mut saved = saved_reply(&h, "Manager", "logo.png", b"lost");
    if let Part::File(file) = &mut saved.parts[0] {
        file.digest = "0".repeat(64);
    }
    h.runtime.store().append(&saved).unwrap();
    let run = h.runtime.send_from_human(h.id("Manager"), "Send Chef the logo.").unwrap();
    h.settle(run).await;
    assert!(h.runtime.store().channel_messages(h.id("Chef"), 200).unwrap().is_empty());
    let results = tool_results(&stub).join("\n");
    assert!(
        results.contains("Ask for a new copy") && results.contains("Message not sent"),
        "{results}"
    );
}
