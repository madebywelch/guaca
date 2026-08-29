//! Evals: whether a crew communicates like something worth watching.
//!
//! The cascade suite asks whether the runtime does what it was told. This asks
//! a different question: given an instruction an operator would actually type,
//! is the resulting traffic reasonable? Every cascade defect this app has had
//! passed the first suite and failed the second, because each individual
//! message was correct and the shape was not.
//!
//! Scenarios come in two kinds.
//!
//! The scripted ones run against a stub model that plays a specific bad habit
//! on purpose: the eternally polite agent, the one that answers three times,
//! the one that broadcasts twice. They are deterministic, free, and they check
//! that the runtime contains the habit rather than that a model happens not to
//! have it today. These run in CI.
//!
//! The live ones run against whatever model is configured and ask whether the
//! prompts hold up. They cost money and cannot be deterministic, so they are
//! `#[ignore]`d and run with `./scripts/evals.sh`. A prompt change that makes
//! agents chattier will not fail CI; it will fail here, which is the point.

mod harness;

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use guac_lib::domain::ids::{AgentId, RunId};
use guac_lib::eval::{analyze, faults, Conversation, Fault};
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::trajectory::Trajectory;

use harness::live::*;
use harness::*;

/// One instruction, run end to end, read for what it did to the operator.
struct Eval {
    convo: Conversation,
    faults: Vec<Fault>,
    /// What the machinery under the conversation did. The two are read
    /// together because they fail apart: a crew can say exactly the right
    /// things through a runtime that left a placeholder open forever.
    trajectory: Trajectory,
}

impl Eval {
    /// Fails naming the fault and printing the whole conversation.
    ///
    /// A count on its own is unactionable: the reason these are worth having is
    /// that a failure hands over the transcript that caused it.
    fn expect_clean(&self, scenario: &str) {
        assert!(
            self.faults.is_empty(),
            "{scenario}\n\n{}\n\nwhat went wrong:\n{}",
            self.convo.script,
            self.faults
                .iter()
                .map(|f| format!("  - {}", f.explain()))
                .collect::<Vec<_>>()
                .join("\n")
        );
        // And the machinery under it: a crew that said exactly the right
        // things through a runtime that left a placeholder open is not a
        // passing eval. See `guac_lib::trajectory`.
        expect_normal(&self.trajectory, scenario);
    }

    /// How many times one agent spoke to the operator.
    ///
    /// Per agent, because each has its own channel: the agent that was given
    /// the instruction is the one whose channel should hold the answer, and a
    /// peer noting what it did in its own channel is not noise in that one.
    fn expect_told_operator(&self, agent: &str, times: usize, scenario: &str) {
        let said = self.convo.to_operator.iter().filter(|(who, _)| who == agent).count();
        assert_eq!(
            said, times,
            "{scenario}: expected {agent} to tell the operator {times} time(s)\n\n{}",
            self.convo.script
        );
    }

    /// The same as a ceiling rather than a count.
    ///
    /// A scripted crew says exactly what its stub says, so those scenarios can
    /// pin the number. A real one is allowed the shape `eval.rs` already calls
    /// reasonable: an update when the work goes out, then the result. What is
    /// not allowed is a third, which is the crew narrating itself.
    fn expect_at_most_told_operator(&self, agent: &str, most: usize, scenario: &str) {
        let said: Vec<&String> = self
            .convo
            .to_operator
            .iter()
            .filter(|(who, _)| who == agent)
            .map(|(_, text)| text)
            .collect();
        assert!(
            said.len() <= most,
            "{scenario}: {agent} told the operator {} time(s), expected at most {most}: {said:?}\
             \n\n{}",
            said.len(),
            self.convo.script
        );
        assert!(
            !said.is_empty(),
            "{scenario}: {agent} never answered the operator\n\n{}",
            self.convo.script
        );
    }

    fn expect_at_most_peer_messages(&self, most: usize, scenario: &str) {
        assert!(
            self.convo.between_agents <= most,
            "{scenario}: {} peer messages, expected at most {most}\n\n{}",
            self.convo.between_agents,
            self.convo.script
        );
    }
}

fn read(h: &Harness, run: RunId, names: &[&str]) -> Eval {
    let lookup: HashMap<AgentId, String> =
        names.iter().map(|n| (h.id(n), (*n).to_string())).collect();
    let name_of = move |id: AgentId| lookup.get(&id).cloned().unwrap_or_else(|| "?".into());

    let messages = h.envelopes(names);

    Eval {
        convo: analyze(&messages, &name_of),
        faults: faults(&messages, &name_of),
        trajectory: h.trajectory(run),
    }
}

// ---- scripted scenarios --------------------------------------------------

/// Everything said so far in one request, agent turns included.
fn history(body: &serde_json::Value) -> String {
    body["messages"]
        .as_array()
        .map(|m| m.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default()
}

/// Whether this agent has already told the operator this.
///
/// A model that has reported once and is then woken by an acknowledgment
/// should say nothing the second time. A stub that repeats itself every turn
/// is only testing the stub, and every scenario here would fail on its own
/// noise rather than on the runtime's.
fn already_said(body: &serde_json::Value, phrase: &str) -> bool {
    body["messages"]
        .as_array()
        .map(|m| {
            m.iter().any(|msg| {
                msg["role"] == "assistant"
                    && msg["content"].as_str().unwrap_or_default().contains(phrase)
            })
        })
        .unwrap_or(false)
}

/// Reports once, then holds its peace.
fn report_once(body: &serde_json::Value, summary: &str) -> Script {
    if already_said(body, summary) {
        Script::Say(String::new())
    } else {
        Script::Say(summary.to_string())
    }
}

/// A model that answers every peer message it sees, forever. The most common
/// real failure: nothing it does is wrong, and it never stops.
fn eternally_polite() -> impl Fn(&serde_json::Value) -> Script + Clone + Send + Sync + 'static {
    |body: &serde_json::Value| {
        let text = history(body);

        if has_tool_result(body) {
            return report_once(body, "the introduction is made");
        }
        if text.contains("thanks") || text.contains("good to meet you") {
            // Thanks for the thanks, and round we go.
            Script::SendTo { recipients: vec!["Chef".into()], text: "thanks again".into() }
        } else if text.contains("hello from manager") {
            Script::SendTo { recipients: vec!["Manager".into()], text: "good to meet you".into() }
        } else {
            Script::SendTo { recipients: vec!["Chef".into()], text: "hello from manager".into() }
        }
    }
}

#[tokio::test]
async fn an_introduction_to_one_agent_is_two_messages() {
    let stub = serve(eternally_polite()).await;
    let h = harness(&stub, &["Manager", "Chef"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Introduce yourself to Chef.").unwrap();
    h.settle(run).await;

    let eval = read(&h, run, &["Manager", "Chef"]);
    eval.expect_clean("introducing yourself to one agent");
    eval.expect_at_most_peer_messages(2, "introducing yourself to one agent");
    eval.expect_told_operator("Manager", 1, "introducing yourself to one agent");
}

#[tokio::test]
async fn an_introduction_to_a_whole_team_does_not_scale_into_a_conversation() {
    // Three peers, all answering at once. The batch an actor sees depends on
    // arrival timing, which is what made this the hardest of these to get
    // right: two of three replies landing late used to make their senders look
    // like strangers.
    let stub = serve(|body: &serde_json::Value| {
        let text = history(body);

        if has_tool_result(body) {
            return report_once(body, "the team knows who I am");
        }
        if text.contains("good to meet you") {
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

    let names = ["Manager", "Chef", "Baker", "Grocer"];
    let h = harness(&stub, &names, GuardLimits::default());
    let run =
        h.runtime.send_from_human(h.id("Manager"), "Introduce yourself to your team.").unwrap();
    h.settle(run).await;

    let eval = read(&h, run, &names);
    eval.expect_clean("introducing yourself to three agents");
    // Three out, three back. Anything more is the crew talking to itself.
    eval.expect_at_most_peer_messages(6, "introducing yourself to three agents");
    eval.expect_told_operator("Manager", 1, "introducing yourself to three agents");
}

#[tokio::test]
async fn an_announcement_is_not_repeated_to_anyone() {
    // Observed: told to announce something, a manager announced it, was thanked
    // three times, and announced it again to all three.
    let stub = serve(|body: &serde_json::Value| {
        if has_tool_result(body) {
            Script::Say("everyone has been told".into())
        } else if anyone_said(body, "noted") {
            // The habit: say it again, word for word.
            Script::SendTo {
                recipients: vec!["Chef".into(), "Baker".into()],
                text: "the time is 11:03pm".into(),
            }
        } else if anyone_said(body, "11:03pm") {
            Script::Say("noted".into())
        } else {
            Script::SendTo {
                recipients: vec!["Chef".into(), "Baker".into()],
                text: "the time is 11:03pm".into(),
            }
        }
    })
    .await;

    let names = ["Manager", "Chef", "Baker"];
    let h = harness(&stub, &names, GuardLimits::default());
    let run = h
        .runtime
        .send_from_human(h.id("Manager"), "Tell everybody the current time is 11:03pm.")
        .unwrap();
    h.settle(run).await;

    let eval = read(&h, run, &names);
    eval.expect_clean("announcing something to everyone");
    eval.expect_at_most_peer_messages(2, "announcing something to everyone");
}

#[tokio::test]
async fn delegating_and_reporting_back_is_one_round_trip() {
    // The flow the app exists for. It must survive everything above: a crew
    // that cannot ask a question is quieter and useless.
    let stub = serve(|body: &serde_json::Value| {
        let text = history(body);

        if has_tool_result(body) {
            return report_once(body, "Chef says the answer is 42");
        }
        if text.contains("what is the answer") {
            Script::SendTo { recipients: vec!["Manager".into()], text: "the answer is 42".into() }
        } else {
            Script::SendTo { recipients: vec!["Chef".into()], text: "what is the answer".into() }
        }
    })
    .await;

    let names = ["Manager", "Chef"];
    let h = harness(&stub, &names, GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Ask Chef for the answer.").unwrap();
    h.settle(run).await;

    let eval = read(&h, run, &names);
    eval.expect_clean("delegating a question and reporting the answer");
    eval.expect_at_most_peer_messages(2, "delegating a question and reporting the answer");
    eval.expect_told_operator("Manager", 1, "delegating a question and reporting the answer");
    assert!(
        eval.convo.to_operator.iter().any(|(_, t)| t.contains("42")),
        "the operator has to be told the answer, not that it was asked for:\n{}",
        eval.convo.script
    );
}

#[tokio::test]
async fn a_model_that_will_not_stop_is_stopped_and_the_operator_still_gets_an_answer() {
    // The guard is the backstop for exactly this. What matters is not only that
    // it fires, but that the operator is not left with nothing.
    let stub = serve(|_: &serde_json::Value| Script::SendTo {
        recipients: vec!["Chef".into()],
        text: "and another thing".into(),
    })
    .await;

    let names = ["Manager", "Chef"];
    let h = harness(&stub, &names, GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Talk to Chef.").unwrap();
    h.settle(run).await;

    let eval = read(&h, run, &names);
    assert!(
        !eval.convo.refusals.is_empty(),
        "a model that only ever sends must be refused by something:\n{}",
        eval.convo.script
    );
    assert!(
        eval.convo.max_hop <= GuardLimits::default().max_hops,
        "the hop limit is the outer wall:\n{}",
        eval.convo.script
    );
}

#[tokio::test]
async fn agents_in_separate_groups_produce_no_traffic_at_all() {
    let stub = serve(|body: &serde_json::Value| {
        if has_tool_result(body) {
            Script::Say("I could not reach them".into())
        } else {
            Script::SendTo { recipients: vec!["Chef".into()], text: "hello".into() }
        }
    })
    .await;

    let names = ["Manager", "Chef"];
    let h = harness_in_groups(
        &stub,
        &[("Manager", Some("Kitchen")), ("Chef", Some("Pantry"))],
        GuardLimits::default(),
    );
    let run = h.runtime.send_from_human(h.id("Manager"), "Say hello to Chef.").unwrap();
    h.settle(run).await;

    let eval = read(&h, run, &names);
    eval.expect_at_most_peer_messages(0, "messaging across a group boundary");
    eval.expect_told_operator("Manager", 1, "messaging across a group boundary");
    assert!(
        !eval.convo.refusals.is_empty(),
        "the sender has to be told why nothing was delivered:\n{}",
        eval.convo.script
    );
}

#[tokio::test]
async fn an_agent_asked_something_it_cannot_do_still_answers_the_operator() {
    // Silence is the worst outcome of all: the operator cannot tell it from a
    // crash, and there is nothing to act on.
    let stub = serve(|_: &serde_json::Value| {
        Script::Say("I cannot do that: I have no way to reach the mainframe.".into())
    })
    .await;

    let names = ["Manager"];
    let h = harness(&stub, &names, GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Reboot the mainframe.").unwrap();
    h.settle(run).await;

    let eval = read(&h, run, &names);
    eval.expect_clean("being asked for something impossible");
    eval.expect_told_operator("Manager", 1, "being asked for something impossible");
}

#[tokio::test]
async fn a_lone_agent_answers_without_inventing_anyone_to_talk_to() {
    let stub = serve(|body: &serde_json::Value| {
        if has_tool_result(body) {
            Script::Say("nobody else is here".into())
        } else {
            Script::Say("It is quiet. I am the only agent in this workspace.".into())
        }
    })
    .await;

    let h = harness(&stub, &["Manager"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Who else is here?").unwrap();
    h.settle(run).await;

    let eval = read(&h, run, &["Manager"]);
    eval.expect_clean("asking a lone agent who else is here");
    eval.expect_at_most_peer_messages(0, "asking a lone agent who else is here");
    eval.expect_told_operator("Manager", 1, "asking a lone agent who else is here");
}

#[tokio::test]
async fn a_crew_told_to_stay_quiet_costs_one_model_call_per_agent() {
    // What a well-behaved wake-up looks like from the outside: an agent reads
    // an acknowledgment, has nothing to add, and says nothing. The cost of
    // being polite is measured in model calls, so it is asserted in them.
    let stub = serve(|body: &serde_json::Value| {
        let text = history(body);

        if has_tool_result(body) {
            return report_once(body, "Chef has been told");
        }
        // The manager is the one holding the operator's instruction; Chef only
        // ever sees the announcement itself. Chef, having been told something
        // that wants no answer, says nothing at all.
        if text.contains("Tell Chef") {
            Script::SendTo { recipients: vec!["Chef".into()], text: "the kitchen is closed".into() }
        } else {
            Script::Say(String::new())
        }
    })
    .await;

    let names = ["Manager", "Chef"];
    let h = harness(&stub, &names, GuardLimits::default());
    let run =
        h.runtime.send_from_human(h.id("Manager"), "Tell Chef the kitchen is closed.").unwrap();
    h.settle(run).await;

    let eval = read(&h, run, &names);
    eval.expect_clean("telling one agent something that needs no answer");
    eval.expect_at_most_peer_messages(1, "telling one agent something that needs no answer");

    // Manager's turn, Manager's tool result, Chef's turn. A fourth would mean
    // somebody was woken by silence.
    let calls = stub.calls.load(Ordering::SeqCst);
    assert!(calls <= 3, "{calls} model calls to tell one agent one thing\n{}", eval.convo.script);
}

#[tokio::test]
async fn replies_that_arrive_apart_are_still_read_together() {
    // Three peers answering one broadcast do not answer together: each takes
    // as long as its own model call. Read one at a time, that is three turns,
    // three prompts and three notes in the operator's channel for one
    // instruction, which is what an operator sees as the crew narrating
    // itself.
    let stub = serve(|body: &serde_json::Value| {
        let text = history(body);
        if has_tool_result(body) {
            return report_once(body, "the team knows");
        }
        if text.contains("hello from manager") {
            // Staggered, the way real model calls come back — and both gaps
            // are past what the old fixed window covered, because that window
            // was sized to this stub and expired before the second answer of
            // every real fan-out. With one slow peer instead of two, a run
            // that reads one reply late still slips under the call ceiling.
            let pause = if text.contains("Baker") {
                2800
            } else if text.contains("Grocer") {
                5600
            } else {
                0
            };
            std::thread::sleep(std::time::Duration::from_millis(pause));
            Script::SendTo { recipients: vec!["Manager".into()], text: "noted".into() }
        } else {
            Script::SendTo {
                recipients: vec!["Chef".into(), "Baker".into(), "Grocer".into()],
                text: "hello from manager".into(),
            }
        }
    })
    .await;

    let names = ["Manager", "Chef", "Baker", "Grocer"];
    let h = harness(&stub, &names, GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Tell the team hello.").unwrap();
    h.settle(run).await;

    let eval = read(&h, run, &names);
    eval.expect_clean("three peers answering a broadcast at different speeds");
    eval.expect_told_operator("Manager", 1, "three peers answering at different speeds");

    // Measured in model calls, because that is what reading them one at a time
    // actually costs: a whole prompt each, and a note each in a real run.
    // Two turns for the manager, two for each peer.
    let calls = stub.calls.load(Ordering::SeqCst);
    // Twelve when each reply is read on its own, ten when they are read
    // together, on a stub whose peers cost two calls each.
    assert!(
        calls <= 10,
        "{calls} model calls to say hello to three agents; replies read one at a time\n{}",
        eval.convo.script
    );
}

// ---- a coordinator with a large team --------------------------------------
//
// Everything above is two or three agents, where "who should do this" has at
// most one wrong answer and a broadcast is nearly the right shape anyway. A
// crew of eight is where delegating is a decision: one peer the work belongs
// to, six who answer from outside their competence if they are asked, and a
// coordinator whose whole job is to tell them apart. Every one of these is
// scored on who was messaged, not on how much was said, because a well-worded
// message to the wrong agent is the failure.

/// A coordinator and seven specialists, each for something different.
///
/// The Manager carries the instruction an operator actually types, because the
/// scenarios below are about what a coordinator does with it.
fn big_crew() -> Vec<Member<'static>> {
    vec![
        Member::told(
            "Manager",
            &["coordination"],
            "You are the Manager. Your job is to delegate to the team. You do not do the work \
             yourself.",
        ),
        Member::new("Researcher", &["web research", "finding sources"]),
        Member::new("Mathematician", &["arithmetic", "statistics"]),
        Member::new("Writer", &["drafting", "editing"]),
        Member::new("Designer", &["layout", "illustration"]),
        Member::new("Lawyer", &["contracts", "compliance"]),
        Member::new("Accountant", &["bookkeeping", "invoices"]),
        Member::new("Scientist", &["experiments", "lab work"]),
    ]
}

fn crew_names(crew: &[Member<'static>]) -> Vec<&'static str> {
    crew.iter().map(|m| m.name).collect()
}

/// How many tool results this conversation already holds.
///
/// A scripted model emits one tool call per round, so this is how a stub says
/// "the third thing I do", which is what a coordinator working through several
/// specialists looks like from inside one turn.
fn rounds_done(body: &serde_json::Value) -> usize {
    body["messages"]
        .as_array()
        .map(|m| m.iter().filter(|m| m["role"] == "tool").count())
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn work_for_one_specialist_costs_the_other_six_nothing() {
    // The argument for delegating rather than broadcasting, in the unit an
    // operator pays in. Six agents that had no part in this must not appear in
    // the bill at all: not one model call, not one line in a channel.
    let crew = big_crew();
    let names = crew_names(&crew);
    let stub = serve(|body| {
        let who = speaker(body);
        if who != "Manager" {
            return Script::Say(format!("{who} here: 17 x 23 is 391."));
        }
        if history(body).contains("391") {
            // The answer is back. This is the one thing the operator is told.
            return report_once(body, "391, from the Mathematician.");
        }
        if has_tool_result(body) {
            // Queued, and nothing has come back yet. There is nothing to say.
            return Script::Say(String::new());
        }
        Script::Instruct {
            recipients: vec!["Mathematician".into()],
            text: "What is 17 x 23?".into(),
        }
    })
    .await;

    let h = harness_of(&stub, &crew, GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "What is 17 times 23?").unwrap();
    h.settle(run).await;

    let eval = read(&h, run, &names);
    eval.expect_clean("a numbers question in a crew of eight");
    eval.expect_told_operator("Manager", 1, "a numbers question in a crew of eight");
    assert_eq!(
        h.messaged_by("Manager"),
        vec![("Mathematician".to_string(), 1)],
        "one fitting agent means one message\n{}",
        eval.convo.script
    );

    // And the six the work was not for never woke up. This is the assertion
    // that scales: a crew of fifty is only affordable if a task costs what the
    // task needs rather than what the roster is long.
    let calls = calls_by_agent(&stub);
    for idle in ["Researcher", "Writer", "Designer", "Lawyer", "Accountant", "Scientist"] {
        assert!(!calls.contains_key(idle), "{idle} was woken by a numbers question: {calls:?}");
        assert!(h.channel_texts(idle).is_empty(), "and it has a channel to show for it");
    }

    // The decision was informed: the roster the Manager read names every peer
    // and what each is for. Without that the broadcast is the right answer.
    let manager = prompts_by_agent(&stub).remove("Manager").expect("the Manager ran");
    assert!(
        manager.contains("- Mathematician (arithmetic, statistics)"),
        "the coordinator has to be able to tell its peers apart: {manager}"
    );
    assert!(manager.contains("- Lawyer (contracts, compliance)"), "{manager}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn a_task_in_three_parts_reaches_three_specialists_and_nobody_else() {
    // The failure this exists for is not the broadcast: it is the split. Asked
    // for something with three parts, a coordinator that will not choose cuts
    // it into a piece per available body and every message looks like work.
    // Here the crew genuinely has three of the seven that fit, so the shape of
    // a correct answer and the shape of the failure are the same size.
    let crew = big_crew();
    let names = crew_names(&crew);
    let stub = serve(|body| {
        let who = speaker(body);
        match who.as_str() {
            "Researcher" => return Script::Say("Sources found: three of them.".into()),
            "Mathematician" => return Script::Say("The number is 391.".into()),
            "Writer" => return Script::Say("Draft ready.".into()),
            "Manager" => {}
            other => return Script::Say(format!("{other} has nothing to do with this.")),
        }

        // Woken by an answer rather than working through its own sends. The
        // three instructions are already out; what is left is to wait for the
        // rest and then say one thing.
        if reading_peer_replies(body) {
            let text = history(body);
            let everything_back = ["Sources found", "The number is 391", "Draft ready"]
                .iter()
                .all(|part| text.contains(part));
            return if everything_back {
                report_once(body, "Sources, the number and a draft: all three are in.")
            } else {
                Script::Say(String::new())
            };
        }

        match rounds_done(body) {
            0 => Script::Instruct {
                recipients: vec!["Researcher".into()],
                text: "Find the sources.".into(),
            },
            1 => Script::Instruct {
                recipients: vec!["Mathematician".into()],
                text: "Work out the number.".into(),
            },
            2 => Script::Instruct {
                recipients: vec!["Writer".into()],
                text: "Draft the write-up.".into(),
            },
            // Everything is out and nothing is back. Waiting is not a message.
            _ => Script::Say(String::new()),
        }
    })
    .await;

    let h = harness_of(&stub, &crew, GuardLimits::default());
    let run = h
        .runtime
        .send_from_human(
            h.id("Manager"),
            "I need the sources for this, the arithmetic checked, and the whole thing written up.",
        )
        .unwrap();
    h.settle(run).await;

    let eval = read(&h, run, &names);
    eval.expect_clean("a three-part task in a crew of eight");
    eval.expect_told_operator("Manager", 1, "a three-part task in a crew of eight");
    assert_eq!(
        h.messaged_by("Manager"),
        vec![
            ("Mathematician".to_string(), 1),
            ("Researcher".to_string(), 1),
            ("Writer".to_string(), 1),
        ],
        "three parts, three agents, one message each\n{}",
        eval.convo.script
    );

    let calls = calls_by_agent(&stub);
    for idle in ["Designer", "Lawyer", "Accountant", "Scientist"] {
        assert!(!calls.contains_key(idle), "{idle} was given a piece of a task it has no part in");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn a_step_that_needs_the_previous_answer_reaches_the_next_specialist() {
    // The shape a real piece of work has and the introduction demo does not:
    // three phases where each one needs what the last one produced, so the
    // coordinator is woken by an answer and has to turn it into the next
    // instruction. That turn is the one where nobody is waiting on the
    // coordinator's words, which used to be read as "nothing is being asked of
    // you", and a pipeline died on its second leg with the operator watching an
    // agent that had apparently stopped.
    let crew = big_crew();
    let names = crew_names(&crew);
    let stub = serve(|body| {
        let who = speaker(body);
        let text = history(body);
        match who.as_str() {
            "Researcher" => return Script::Say("Three sources: A, B and C.".into()),
            // Each specialist is given the previous answer, and says so. If the
            // handover carried nothing, this is where it shows.
            "Mathematician" => {
                return Script::Say(if text.contains("A, B and C") {
                    "Across A, B and C the total is 391.".into()
                } else {
                    "I was sent nothing to add up.".to_string()
                })
            }
            "Writer" => {
                return Script::Say(if text.contains("391") {
                    "Written up: three sources, total 391.".into()
                } else {
                    "I was sent nothing to write up.".to_string()
                })
            }
            "Manager" => {}
            other => return Script::Say(format!("{other} is not part of this.")),
        }

        // One leg per turn: once the send is away there is nothing to add until
        // an answer comes back, and the next leg is decided by which answer it
        // was.
        if has_tool_result(body) {
            Script::Say(String::new())
        } else if text.contains("Written up") {
            report_once(body, "Done: three sources, total 391, written up.")
        } else if text.contains("total is 391") {
            Script::Instruct {
                recipients: vec!["Writer".into()],
                text: "Write this up: three sources, total 391.".into(),
            }
        } else if text.contains("A, B and C") {
            Script::Instruct {
                recipients: vec!["Mathematician".into()],
                text: "Add up the figures in A, B and C.".into(),
            }
        } else {
            Script::Instruct {
                recipients: vec!["Researcher".into()],
                text: "Find the sources.".into(),
            }
        }
    })
    .await;

    let h = harness_of(&stub, &crew, GuardLimits::default());
    let run = h
        .runtime
        .send_from_human(h.id("Manager"), "Research it, add up the figures, and write it up.")
        .unwrap();
    h.settle(run).await;

    let eval = read(&h, run, &names);
    eval.expect_clean("three phases, each needing the last");
    eval.expect_told_operator("Manager", 1, "three phases, each needing the last");

    // Every leg arrived carrying what the leg before it produced, and the proof
    // is in the answers rather than in the instructions: each specialist says
    // one thing when it was given the previous finding and another when it was
    // not. A pipeline that hands on nothing still looks like three well-formed
    // delegations from the sending side.
    assert!(
        h.said_to_peers("Mathematician").iter().any(|t| t.contains("Across A, B and C")),
        "the second leg was not given the first one's answer:\n{}",
        eval.convo.script
    );
    assert!(
        h.said_to_peers("Writer").iter().any(|t| t.contains("three sources, total 391")),
        "and the third was not given the second's:\n{}",
        eval.convo.script
    );
    assert_eq!(
        h.messaged_by("Manager"),
        vec![
            ("Mathematician".to_string(), 1),
            ("Researcher".to_string(), 1),
            ("Writer".to_string(), 1),
        ],
        "one message per phase and no chatter between them\n{}",
        eval.convo.script
    );

    // What a pipeline costs in depth, which is the limit it will actually meet.
    // Each phase is two hops even though every message is one hop from the
    // coordinator: the answer carries the hop back, and the next instruction
    // starts from there. Eight hops is therefore four phases, not eight.
    assert_eq!(
        eval.convo.max_hop, 6,
        "three phases is six hops, and this is the arithmetic the hop limit is read against:\n{}",
        eval.convo.script
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn a_pipeline_deeper_than_the_hop_limit_stops_at_the_wall_and_says_where() {
    // The other side of the arithmetic above. A coordinator working through
    // specialists in sequence spends two hops per phase, so the default limit
    // of eight is four phases: the fifth instruction is refused. That is the
    // guard doing its job, and what it must not do is leave the operator with
    // a pipeline that stopped without saying where.
    let stub = serve(|body| {
        let who = speaker(body);
        if who != "Manager" {
            return Script::Say(format!("{who} is done."));
        }
        if has_tool_result(body) {
            let text = history(body);
            return if text.contains("hops from the operator") {
                Script::Say("Stopped: the chain hit its depth limit before the last step.".into())
            } else {
                Script::Say(String::new())
            };
        }
        // One specialist per phase, in order, driven by who has answered.
        let done = ["Alpha", "Bravo", "Charlie", "Delta", "Echo"]
            .iter()
            .filter(|name| history(body).contains(&format!("{name} is done")))
            .count();
        let next = ["Alpha", "Bravo", "Charlie", "Delta", "Echo"][done.min(4)];
        Script::Instruct { recipients: vec![next.into()], text: format!("Your turn, {next}.") }
    })
    .await;

    let crew = [
        Member::new("Manager", &["coordination"]),
        Member::new("Alpha", &["first"]),
        Member::new("Bravo", &["second"]),
        Member::new("Charlie", &["third"]),
        Member::new("Delta", &["fourth"]),
        Member::new("Echo", &["fifth"]),
    ];
    let h = harness_of(&stub, &crew, GuardLimits::default());
    let run = h
        .runtime
        .send_from_human(h.id("Manager"), "Take this through all five of them in order.")
        .unwrap();
    h.settle(run).await;

    let names = ["Manager", "Alpha", "Bravo", "Charlie", "Delta", "Echo"];
    let eval = read(&h, run, &names);
    assert!(
        eval.convo.max_hop <= GuardLimits::default().max_hops,
        "the hop limit is the outer wall:\n{}",
        eval.convo.script
    );
    assert_eq!(
        h.messaged_by("Manager").len(),
        4,
        "four phases fit inside eight hops and the fifth does not\n{}",
        eval.convo.script
    );
    assert!(
        tool_results(&stub).iter().any(|r| r.contains("hops from the operator")),
        "the coordinator has to be told which wall it hit, not merely refused"
    );
    eval.expect_told_operator("Manager", 1, "a pipeline deeper than the hop limit");
    assert!(
        eval.convo.to_operator.iter().any(|(_, t)| t.contains("depth limit")),
        "and the operator has to learn the work stopped early:\n{}",
        eval.convo.script
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_specialist_that_cannot_do_it_says_so_and_the_operator_hears_why() {
    // Failure path. The work was routed correctly and the agent it belongs to
    // cannot do it, which is the ordinary outcome of a real task. What must not
    // happen is the coordinator quietly trying somebody else, or the operator
    // being told the job is done.
    let crew = big_crew();
    let names = crew_names(&crew);
    let stub = serve(|body| {
        let who = speaker(body);
        if who == "Researcher" {
            return Script::Say(
                "I cannot: the archive is locked and nobody here has a login.".into(),
            );
        }
        if who != "Manager" {
            return Script::Say(format!("{who} was not asked."));
        }
        let text = history(body);
        if text.contains("the archive is locked") {
            report_once(body, "Blocked: the archive is locked and Researcher cannot get in.")
        } else if has_tool_result(body) {
            Script::Say(String::new())
        } else {
            Script::Instruct {
                recipients: vec!["Researcher".into()],
                text: "Pull the figures from the archive.".into(),
            }
        }
    })
    .await;

    let h = harness_of(&stub, &crew, GuardLimits::default());
    let run =
        h.runtime.send_from_human(h.id("Manager"), "Get me the figures from the archive.").unwrap();
    h.settle(run).await;

    let eval = read(&h, run, &names);
    eval.expect_clean("work the right agent cannot do");
    eval.expect_told_operator("Manager", 1, "work the right agent cannot do");
    assert!(
        eval.convo.to_operator.iter().any(|(_, t)| t.contains("locked")),
        "the operator has to be told why it is stuck, not that it is:\n{}",
        eval.convo.script
    );
    assert_eq!(
        h.messaged_by("Manager"),
        vec![("Researcher".to_string(), 1)],
        "a refusal is not a reason to go asking the rest of the crew\n{}",
        eval.convo.script
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_coordinator_that_will_not_stop_asking_one_specialist_is_stopped_and_still_answers() {
    // The per-pair limit, from the coordinator's side. A large task is exactly
    // where an agent talks itself into one more round with the same peer, and
    // the wall it hits has to leave the operator with an answer rather than
    // with a run that went quiet.
    let stub = serve(|body| {
        if speaker(body) == "Researcher" {
            return Script::Say(format!("Nothing further from me ({}).", rounds_done(body)));
        }
        // Answers arriving are not a reason to start again.
        if reading_peer_replies(body) {
            return Script::Say(String::new());
        }
        if history(body).contains("already sent Researcher") {
            // The refusal is read mid-turn. This is what a model does with it.
            return Script::Say("Researcher has given me all it can. Stopping there.".into());
        }
        Script::Instruct {
            recipients: vec!["Researcher".into()],
            // Varied on purpose: this is the pair limit, not the dedup rule.
            text: format!("One more thing, number {}.", rounds_done(body)),
        }
    })
    .await;

    let crew =
        [Member::new("Manager", &["coordination"]), Member::new("Researcher", &["research"])];
    let limits = GuardLimits { max_sends_per_pair: 2, ..GuardLimits::default() };
    let h = harness_of(&stub, &crew, limits);
    let run = h.runtime.send_from_human(h.id("Manager"), "Get everything Researcher has.").unwrap();
    h.settle(run).await;

    let eval = read(&h, run, &["Manager", "Researcher"]);
    let to_researcher = h
        .messaged_by("Manager")
        .into_iter()
        .find(|(name, _)| name == "Researcher")
        .map(|(_, n)| n)
        .unwrap_or(0);
    assert!(
        to_researcher <= 2,
        "the pair limit is 2 and {to_researcher} were delivered\n{}",
        eval.convo.script
    );
    assert!(
        tool_results(&stub).iter().any(|r| r.contains("already sent Researcher")),
        "the coordinator has to read the wall it hit, or it rewords and tries again"
    );
    eval.expect_told_operator(
        "Manager",
        1,
        "a coordinator stopped by the per-pair limit still owes an answer",
    );
    // The machinery, but not `expect_clean`: whether the second instruction
    // demanded an answer depends on whether the first reply had landed when it
    // was sent, and a crew being cut off mid-round is exactly where that race
    // is live. Pinning the conversation-level shape here would be pinning the
    // scheduler.
    h.expect_normal(run, "a coordinator stopped by the per-pair limit");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn an_announcement_to_a_team_wider_than_the_fan_out_limit_still_reaches_everybody() {
    // Twelve agents and a limit of eight recipients per call. The refusal is
    // the guard doing its job, and on its own it is also a crew of twelve that
    // cannot be told anything: what makes it survivable is that the refusal
    // says how to get through, and that the second call does.
    let peers: Vec<String> = (1..=12).map(|n| format!("Agent {n}")).collect();
    let everyone = peers.clone();
    let stub = serve(move |body| {
        if speaker(body) != "Manager" {
            // An announcement asks for nothing back, and a crew of twelve
            // acknowledging one is twelve model calls and twelve lines.
            return Script::Say(String::new());
        }
        let text = history(body);
        let announcement = "The office closes at four on Friday.";
        if text.contains("Queued for delivery to: Agent 9") {
            return report_once(body, "All twelve have been told.");
        }
        if text.contains("Queued for delivery to: Agent 1,") {
            // The first eight are away. The rest go in a second call.
            return Script::SendTo {
                recipients: everyone[8..].to_vec(),
                text: announcement.into(),
            };
        }
        if text.contains("exceeds the limit") {
            // Read the refusal, split the list, and go again.
            return Script::SendTo {
                recipients: everyone[..8].to_vec(),
                text: announcement.into(),
            };
        }
        Script::SendTo { recipients: everyone.clone(), text: announcement.into() }
    })
    .await;

    let mut crew = vec![Member::new("Manager", &["coordination"])];
    crew.extend(peers.iter().map(|name| Member::new(name.as_str(), &["testing"])));
    let h = harness_of(&stub, &crew, GuardLimits::default());
    let run = h
        .runtime
        .send_from_human(h.id("Manager"), "Tell everyone the office closes at four on Friday.")
        .unwrap();
    h.settle(run).await;

    let refusals = tool_results(&stub).join("\n");
    assert!(
        refusals.contains("12 recipients in one call exceeds the limit of 8"),
        "the wall has to be named with its numbers: {refusals}"
    );
    assert!(
        refusals.contains("Send to at most 8 at a time"),
        "and with the way through, or a crew of twelve is a crew that cannot be told anything"
    );

    for peer in &peers {
        assert!(
            h.channel_texts(peer).iter().any(|t| t.contains("closes at four")),
            "{peer} never heard the announcement"
        );
    }

    let names: Vec<&str> =
        std::iter::once("Manager").chain(peers.iter().map(String::as_str)).collect();
    let eval = read(&h, run, &names);
    eval.expect_told_operator("Manager", 1, "announcing something to twelve agents");
    eval.expect_at_most_peer_messages(12, "announcing something to twelve agents");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_coordinator_delegates_from_what_it_remembered_on_an_earlier_run() {
    // Memory as the thing it is for: not a fact recited back, but a decision
    // made differently because of it. The operator says who does what once, in
    // one run; the routing that follows happens in another run, where the only
    // surviving trace of that conversation is the agent's own file.
    let stub = serve(|body| {
        let who = speaker(body);
        if who != "Manager" {
            return Script::Say(format!("{who} has it."));
        }

        let system = body["messages"][0]["content"].as_str().unwrap_or_default();
        let remembered = system.contains("Ada does the numbers");
        let asked_to_add = history(body).contains("Add up last quarter");

        if !asked_to_add {
            // The first run: the operator says who does what, and it is worth
            // keeping because it will outlive this conversation.
            return if has_tool_result(body) {
                Script::Say("Noted.".into())
            } else {
                Script::Notes("- Ada does the numbers. Bo does the history.".into())
            };
        }
        if has_tool_result(body) || reading_peer_replies(body) {
            return report_once(body, "Sent to the one that does numbers.");
        }
        // The second run, where the only trace of the first is the file above.
        // Without it there is nothing to choose on: neither name says anything.
        Script::Instruct {
            recipients: vec![if remembered { "Ada" } else { "Bo" }.into()],
            text: "Add up last quarter.".into(),
        }
    })
    .await;

    // No skills on either, deliberately. If the roster could answer this, the
    // memory would not have to.
    let crew = [
        Member::new("Manager", &["coordination"]),
        Member::new("Ada", &[]),
        Member::new("Bo", &[]),
    ];
    let h = harness_of(&stub, &crew, GuardLimits::default());

    let told = h
        .runtime
        .send_from_human(h.id("Manager"), "Ada does the numbers and Bo does the history.")
        .unwrap();
    h.settle(told).await;
    assert_eq!(
        h.runtime.workspace().read(h.id("Manager")),
        "- Ada does the numbers. Bo does the history.",
        "nothing was kept, so the run below cannot be about memory"
    );

    let run = h.runtime.send_from_human(h.id("Manager"), "Add up last quarter for me.").unwrap();
    h.settle(run).await;

    let eval = read(&h, run, &["Manager", "Ada", "Bo"]);
    assert_eq!(
        h.messaged_by("Manager"),
        vec![("Ada".to_string(), 1)],
        "the routing had to come out of memory: neither card says who does numbers\n{}",
        eval.convo.script
    );
    assert!(h.channel_texts("Bo").is_empty(), "and the other one was never troubled");
    eval.expect_clean("delegating from what an earlier run remembered");
}

// ---- live scenarios ------------------------------------------------------

/// Runs the same instructions against the configured model.
///
/// Ignored by default: it costs money, it needs a key, and it cannot be
/// deterministic. Run it with `./scripts/evals.sh` after changing a prompt,
/// which is the change CI cannot see.
mod live {
    use super::*;

    /// Everything a live scenario needs: real agents, real model, real prompt.
    async fn run_live(names: &[&'static str], instruction: &str) -> Option<Eval> {
        let crew: Vec<LiveAgent> = names.iter().map(|n| LiveAgent::generic(n)).collect();
        // Generous: several sequential model calls, over a network, and a
        // failure here should mean "this crew never stopped talking", not
        // "the endpoint was slow".
        run_live_crew(&crew, instruction, 180).await.map(|(_, eval)| eval)
    }

    /// The same, for scenarios that have to look at who was messaged rather
    /// than at how much was said. The harness comes back because the answer is
    /// in the envelopes, and `Conversation` counts peer traffic without
    /// recording where any of it went.
    ///
    /// `secs` is per scenario because the settle window is a property of the
    /// work, not of the network. These run against the operator's own limits,
    /// which allow a hundred tool rounds, so an agent handed a real task starts
    /// a machine and uses it: the first run of this took longer than three
    /// minutes with nothing wrong.
    async fn run_live_crew(
        crew: &[LiveAgent],
        instruction: &str,
        secs: u64,
    ) -> Option<(Harness, Eval)> {
        run_live_prepared(crew, instruction, secs, |_| {}).await
    }

    /// The same, with something already true of the workspace before the crew
    /// is asked anything.
    ///
    /// A scenario about what an agent does with what it already keeps cannot
    /// arrange that with an instruction: it would be asking for the thing under
    /// test. `setup` runs against the live store, and everything after it is
    /// the ordinary path, including the machine cleanup an early assertion must
    /// not be able to skip.
    async fn run_live_prepared(
        crew: &[LiveAgent],
        instruction: &str,
        secs: u64,
        setup: impl FnOnce(&Harness),
    ) -> Option<(Harness, Eval)> {
        let config = configured()?;
        let before = machines_now(&config).await;
        let h = live_crew(config.clone(), crew);
        let names: Vec<&str> = crew.iter().map(|a| a.name).collect();
        setup(&h);

        // Started before the instruction, because the first turn can park.
        let (answering, asked) = answer_permission_requests(&h);
        let run = h.runtime.send_from_human(h.id(names[0]), instruction).unwrap();

        let settled = h.settled_within(run, secs).await;
        answering.abort();
        // Before the assertions, because an assertion that fails takes the rest
        // of the function with it, and what is left standing is a real machine
        // billing for its idle period. Twenty accumulated this way in an
        // afternoon, and the timeouts left the most behind.
        release_machines(&config, before).await;

        let asked = asked.lock().clone();
        if !asked.is_empty() {
            println!("permission requests, all declined: {asked:?}");
        }
        assert!(settled, "run did not settle in {secs}s. messages so far:\n{}", h.transcript());
        let eval = read(&h, run, &names);
        Some((h, eval))
    }

    fn report(scenario: &str, eval: &Eval) {
        println!("\n=== {scenario} ===\n{}", eval.convo.script);
        println!(
            "told the operator {} time(s), {} peer message(s), deepest hop {}",
            eval.convo.to_operator.len(),
            eval.convo.between_agents,
            eval.convo.max_hop
        );
        for fault in &eval.faults {
            println!("  FAULT: {}", fault.explain());
        }
    }

    #[tokio::test]
    #[ignore = "live: costs money, needs a configured key"]
    async fn live_introduction_to_a_team() {
        let names = ["Manager", "Researcher", "Mathematician", "Scientist"];
        let Some(eval) = run_live(&names, "Introduce yourself to your team.").await else {
            eprintln!("no configured model; skipping");
            return;
        };
        report("introduce yourself to your team", &eval);
        eval.expect_clean("introduce yourself to your team");
        eval.expect_at_most_peer_messages(6, "introduce yourself to your team");
    }

    #[tokio::test]
    #[ignore = "live: costs money, needs a configured key"]
    async fn live_announcement() {
        let names = ["Manager", "Researcher", "Mathematician"];
        let Some(eval) =
            run_live(&names, "Tell everybody that the current time is 11:03 p.m. Eastern.").await
        else {
            eprintln!("no configured model; skipping");
            return;
        };
        report("tell everybody the time", &eval);
        eval.expect_clean("tell everybody the time");
        eval.expect_at_most_peer_messages(4, "tell everybody the time");
    }

    #[tokio::test]
    #[ignore = "live: costs money, needs a configured key"]
    async fn live_delegation() {
        let names = ["Manager", "Mathematician"];
        let Some(eval) =
            run_live(&names, "Ask the Mathematician what 17 times 23 is, then tell me the answer.")
                .await
        else {
            eprintln!("no configured model; skipping");
            return;
        };
        report("delegate a calculation", &eval);
        eval.expect_clean("delegate a calculation");
        assert!(
            eval.convo.to_operator.iter().any(|(_, t)| t.contains("391")),
            "the operator has to end up with the answer:\n{}",
            eval.convo.script
        );
    }

    #[tokio::test]
    #[ignore = "live: costs money, needs a configured key"]
    async fn live_question_needing_no_delegation() {
        let names = ["Manager", "Researcher"];
        let Some(eval) = run_live(&names, "In one sentence, what do you do?").await else {
            eprintln!("no configured model; skipping");
            return;
        };
        report("a question the agent can answer itself", &eval);
        eval.expect_clean("a question the agent can answer itself");
        eval.expect_at_most_peer_messages(0, "a question the agent can answer itself");
        eval.expect_told_operator("Manager", 1, "a question the agent can answer itself");
    }

    #[tokio::test]
    #[ignore = "live: costs money, needs a configured key"]
    async fn live_work_goes_only_to_the_agent_it_is_for() {
        // The observed failure, reproduced with the crew that produced it. A
        // coordinator told it does not do the work itself was asked a research
        // question and sent it to the Researcher, the Mathematician and the
        // Scientist. Every message was well formed. Nothing refused it: three
        // recipients is inside a fan-out limit of eight, so the guard is not
        // the thing that was supposed to catch this and lowering it would only
        // break announcements.
        //
        // What was missing was a reason to choose. The roster printed each
        // peer's skills and never said what they were for, and `directory` was
        // described as a way to check a name. So this eval asks the one
        // question CI cannot: reading the prompts as they now stand, does a
        // coordinator narrow?
        // The crew is copied off the machine this happened on, because every
        // detail that looked incidental turned out to matter. The skills really
        // are one word each. No card carries a system prompt: the standing
        // instruction arrived as the operator's message, which is why it is at
        // the front of the instruction below rather than on the Manager. And
        // the coordinator runs a different model from the crew it directs,
        // which is the agent under test here, so a crew put entirely on the
        // default model is not this scenario.
        //
        // Three questions and three peers is what makes this the hard case. The
        // easy reading is one question per agent, and it is wrong: all three
        // questions are history, and only one of these agents does history.
        let crew = [
            LiveAgent {
                name: "Manager",
                skills: &["Manager"],
                prompt: Some(""),
                model: Some("openai/gpt-5.6-luna"),
            },
            LiveAgent {
                name: "Researcher",
                skills: &["researcher"],
                prompt: Some(""),
                model: None,
            },
            LiveAgent {
                name: "Mathematician",
                skills: &["mathematics"],
                prompt: Some(""),
                model: None,
            },
            LiveAgent { name: "Scientist", skills: &["scientist"], prompt: Some(""), model: None },
        ];

        // Generous, because all three specialists start machines and read
        // Wikipedia before answering, exactly as they did on the day. A
        // timeout here would fail the eval for the wrong reason.
        let Some((h, eval)) = run_live_crew(
            &crew,
            "You are the Manager, you do not work, you delegate and escalate to me only when \
             necessary. Now, I want research done. How many Japanese died fighting in China? Why \
             did Japan invade China? What was the U.S.'s involvement?",
            420,
        )
        .await
        else {
            eprintln!("no configured model; skipping");
            return;
        };
        report("research delegated to a mixed crew", &eval);

        let got = h.messaged_by("Manager");
        println!("messaged: {got:?}");

        let strangers: Vec<&(String, usize)> =
            got.iter().filter(|(name, _)| name == "Mathematician" || name == "Scientist").collect();
        assert!(
            strangers.is_empty(),
            "a research question reached {strangers:?}, who have no research skill between \
             them. Delegating to everyone is the failure this eval exists for.\n\n{}",
            eval.convo.script
        );
        assert!(
            got.iter().any(|(name, _)| name == "Researcher"),
            "the work still has to be delegated, and the Researcher is who it is for\n\n{}",
            eval.convo.script
        );
        eval.expect_clean("research delegated to a mixed crew");

        // The second half of the same turn. A fired routine starts a fresh run
        // with a fresh step budget, so a coordinator that schedules a check for
        // replies spends outside every limit applied to the run that scheduled
        // it. The one observed booked two, 19 and 34 seconds out, both ahead of
        // any reply.
        let booked = h.runtime.store().agent_routines(h.id("Manager")).unwrap();
        assert!(
            booked.is_empty(),
            "the Manager scheduled {} routine(s) instead of waiting for replies that arrive on \
             their own: {:?}\n\n{}",
            booked.len(),
            booked.iter().map(|r| r.what.clone()).collect::<Vec<_>>(),
            eval.convo.script
        );
    }

    #[tokio::test]
    #[ignore = "live: costs money, needs a configured key"]
    async fn live_two_step_delegation() {
        // The hard one for the current design: a manager that needs two
        // different specialists in sequence.
        let names = ["Manager", "Mathematician", "Researcher"];
        let Some(eval) = run_live(
            &names,
            "Ask the Mathematician to double 21, then ask the Researcher to say whether that \
             number is a famous one. Tell me both answers.",
        )
        .await
        else {
            eprintln!("no configured model; skipping");
            return;
        };
        report("two specialists in sequence", &eval);
        eval.expect_clean("two specialists in sequence");
        eval.expect_at_most_peer_messages(4, "two specialists in sequence");
    }

    // ---- a coordinator with a large team ---------------------------------
    //
    // The scripted half of these proves the runtime carries a delegation to one
    // agent and charges nothing for the rest. Only a real model can be asked
    // the question underneath: given a roster of seven and a task, does it
    // choose. A crew this size is also where the failure is cheapest to make
    // and most expensive to have, because every wrong recipient is a model call
    // and an answer from outside its competence.

    /// Seven specialists whose skills do not overlap, and a coordinator told
    /// what an operator actually types.
    ///
    /// No card carries anything else: the standing instruction is on the
    /// Manager because that is where an operator puts it, and the specialists
    /// are described only by what they do, because that is all the roster
    /// carries.
    fn large_crew(manager_instruction: &'static str) -> Vec<LiveAgent> {
        vec![
            LiveAgent {
                name: "Manager",
                skills: &["coordination"],
                prompt: Some(manager_instruction),
                model: None,
            },
            LiveAgent::skilled("Researcher", &["web research", "finding sources"]),
            LiveAgent::skilled("Mathematician", &["arithmetic", "statistics"]),
            LiveAgent::skilled("Writer", &["drafting", "editing"]),
            LiveAgent::skilled("Designer", &["layout", "illustration"]),
            LiveAgent::skilled("Lawyer", &["contract review", "compliance"]),
            LiveAgent::skilled("Accountant", &["bookkeeping", "invoices"]),
        ]
    }

    const ONLY_DELEGATES: &str = "You are the Manager. Your job is to delegate work to the team \
                                  and to report back to the operator. You do not do the work \
                                  yourself.";

    /// Fails naming every peer that was messaged and had no business being.
    fn expect_only(h: &Harness, from: &str, fits: &[&str], eval: &Eval) {
        let got = h.messaged_by(from);
        println!("messaged: {got:?}");
        let strangers: Vec<&(String, usize)> =
            got.iter().filter(|(name, _)| !fits.contains(&name.as_str())).collect();
        assert!(
            strangers.is_empty(),
            "{from} sent work to {strangers:?}, who have no skill this task needs. Spreading a \
             task over everyone available is the decision not being made.\n\n{}",
            eval.convo.script
        );
        for fit in fits {
            assert!(
                got.iter().any(|(name, _)| name == fit),
                "the work still has to reach {fit}\n\n{}",
                eval.convo.script
            );
        }
    }

    #[tokio::test]
    #[ignore = "live: costs money, needs a configured key"]
    async fn live_a_manager_that_only_delegates_still_gets_one_thing_done() {
        // The plainest form of the question: a coordinator that has been told
        // in so many words not to do the work, a crew of seven, and a task that
        // belongs to exactly one of them. Three ways this fails and all three
        // have been seen: it answers from its own head, it asks everybody, or
        // it takes the instruction as a reason to do nothing at all.
        let crew = large_crew(ONLY_DELEGATES);
        let Some((h, eval)) =
            run_live_crew(&crew, "What is 17% of 4,820? I need the number, not a method.", 300)
                .await
        else {
            eprintln!("no configured model; skipping");
            return;
        };
        report("one task, a crew of seven, a manager that only delegates", &eval);

        expect_only(&h, "Manager", &["Mathematician"], &eval);
        eval.expect_clean("one task, a crew of seven");
        eval.expect_at_most_told_operator("Manager", 2, "one task, a crew of seven");
        assert!(
            eval.convo.to_operator.iter().any(|(who, t)| who == "Manager" && t.contains("819")),
            "delegating is not the deliverable; the operator asked for a number:\n{}",
            eval.convo.script
        );
    }

    #[tokio::test]
    #[ignore = "live: costs money, needs a configured key"]
    async fn live_a_task_with_three_parts_reaches_three_of_seven() {
        // The failure this exists for is the split rather than the broadcast.
        // Asked for something in parts, a coordinator that will not choose cuts
        // it into a piece per available body, and every one of those messages
        // is well formed and has a rationale. Here three of the seven genuinely
        // fit, so the correct answer and the failure are the same size and only
        // the names tell them apart.
        let crew = large_crew(ONLY_DELEGATES);
        let Some((h, eval)) = run_live_crew(
            &crew,
            "I'm putting together a one-page brief on the UK's 2024 renters' reform bill. I need \
             the facts checked against a source, the arithmetic on the commencement dates sanity \
             checked, and the whole thing written up in plain English. Come back to me when you \
             have all three.",
            600,
        )
        .await
        else {
            eprintln!("no configured model; skipping");
            return;
        };
        report("a three-part brief in a crew of seven", &eval);

        expect_only(&h, "Manager", &["Researcher", "Mathematician", "Writer"], &eval);
        eval.expect_clean("a three-part brief in a crew of seven");
        eval.expect_at_most_told_operator("Manager", 2, "a three-part brief in a crew of seven");
    }

    #[tokio::test]
    #[ignore = "live: costs money, needs a configured key"]
    async fn live_a_crew_with_nobody_for_the_job_says_so_rather_than_picking_the_nearest_name() {
        // The rule the prompt states as "the nearest available name is not a
        // fit". An agent under pressure to delegate will delegate to somebody,
        // and the cost of that is a specialist answering from outside its
        // competence in a voice that sounds exactly like an answer.
        let crew = vec![
            LiveAgent {
                name: "Manager",
                skills: &["coordination"],
                prompt: Some(ONLY_DELEGATES),
                model: None,
            },
            LiveAgent::skilled("Designer", &["layout", "illustration"]),
            LiveAgent::skilled("Accountant", &["bookkeeping", "invoices"]),
        ];
        let Some((h, eval)) = run_live_crew(
            &crew,
            "Diagnose why my knee hurts when I run downhill, and tell me whether to see anyone \
             about it.",
            300,
        )
        .await
        else {
            eprintln!("no configured model; skipping");
            return;
        };
        report("work the crew has nobody for", &eval);

        assert_eq!(
            h.messaged_by("Manager"),
            vec![],
            "a task nobody here fits was handed to somebody anyway\n\n{}",
            eval.convo.script
        );
        eval.expect_at_most_told_operator("Manager", 2, "work the crew has nobody for");
    }

    #[tokio::test]
    #[ignore = "live: costs money, needs a configured key"]
    async fn live_a_recurring_instruction_becomes_one_routine_on_the_calendar() {
        // Standing work, which is the other thing an operator asks a crew for.
        // Two failures, and the prompt argues against both: several routines
        // that each do a piece of one job, and a daily job stored as a gap in
        // seconds, which drifts an hour twice a year and cannot mean weekdays.
        let crew = vec![LiveAgent::skilled("Watcher", &["monitoring", "reporting"])];
        let Some((h, eval)) = run_live_crew(
            &crew,
            "Every weekday at 8am, check the top stories on Hacker News and send me anything \
             about local-first software. Just set it up; don't do it now.",
            240,
        )
        .await
        else {
            eprintln!("no configured model; skipping");
            return;
        };
        report("a standing weekday job", &eval);

        let booked = h.runtime.store().agent_routines(h.id("Watcher")).unwrap();
        println!(
            "booked: {:?}",
            booked.iter().map(|r| (r.title().to_string(), r.describe())).collect::<Vec<_>>()
        );
        assert_eq!(
            booked.len(),
            1,
            "one standing job is one routine; {} were booked: {:?}",
            booked.len(),
            booked.iter().map(|r| r.what.clone()).collect::<Vec<_>>()
        );
        assert_eq!(
            booked[0].trigger,
            guac_lib::domain::routine::Trigger::Clock(guac_lib::domain::routine::Cadence::Weekdays),
            "a weekday job is a shape on the calendar. Stored as a gap it fires on Saturday, and \
             stored as a day in seconds it loses an hour twice a year"
        );
        assert!(
            booked[0].what.len() > 40,
            "the instruction has to stand on its own when it fires, with no conversation behind \
             it: {:?}",
            booked[0].what
        );
        eval.expect_told_operator("Watcher", 1, "a standing weekday job");
    }

    #[tokio::test]
    #[ignore = "live: costs money, needs a configured key"]
    async fn live_a_change_to_a_standing_job_changes_it_rather_than_adding_a_second() {
        // The one this whole path exists for, and it is a question about
        // judgment rather than about machinery: half an hour after booking
        // something, the operator asks for it differently without saying which
        // routine they mean. An agent that cannot see what it keeps writes a
        // second one, tells the operator it has made the change, and both fire
        // from then on. Nothing in the scripted suite can see this: the model
        // decides it.
        let crew = vec![LiveAgent::skilled("Watcher", &["monitoring", "reporting"])];
        let Some((h, eval)) = run_live_prepared(
            &crew,
            "Actually, do the listings sweep every day rather than just weekdays.",
            240,
            |h| {
                // Booked in an earlier conversation, which is the only place
                // this can come from: the turn under test must not be the turn
                // that created it.
                h.runtime
                    .store()
                    .create_routine(
                        h.id("Watcher"),
                        "Listings sweep",
                        "Check the new listings on both sites and email the operator anything \
                         new, with the asking price.",
                        guac_lib::domain::routine::Trigger::Clock(
                            guac_lib::domain::routine::Cadence::Weekdays,
                        ),
                        Some(guac_lib::domain::now_ms() + 3_600_000),
                        false,
                    )
                    .unwrap();
            },
        )
        .await
        else {
            eprintln!("no configured model; skipping");
            return;
        };
        report("a change to a standing job", &eval);

        let standing = h.runtime.store().agent_routines(h.id("Watcher")).unwrap();
        println!(
            "standing: {:?}",
            standing.iter().map(|r| (r.title().to_string(), r.describe())).collect::<Vec<_>>()
        );
        assert_eq!(
            standing.len(),
            1,
            "a change to a standing job is one routine, not two that both fire: {:?}",
            standing.iter().map(|r| (r.title().to_string(), r.describe())).collect::<Vec<_>>()
        );
        assert_eq!(
            standing[0].trigger,
            guac_lib::domain::routine::Trigger::Clock(guac_lib::domain::routine::Cadence::Daily),
            "and it is the change that was asked for"
        );
        assert!(
            standing[0].what.len() > 40,
            "the instruction it already had must survive being retimed: {:?}",
            standing[0].what
        );
        eval.expect_told_operator("Watcher", 1, "a change to a standing job");
    }

    #[tokio::test]
    #[ignore = "live: costs money, needs a configured key"]
    async fn live_a_standing_preference_is_kept_in_memory_rather_than_in_the_conversation() {
        // Memory is the only thing that survives a conversation, and the one
        // thing nobody else maintains. An agent that treats "from now on" as
        // something to agree to has lost it by the next run, and will not know
        // it has.
        let crew = vec![LiveAgent::skilled("Assistant", &["drafting", "scheduling"])];
        let Some((h, eval)) = run_live_crew(
            &crew,
            "From now on, always give me prices in pounds rather than dollars, and never book \
             anything before 10am.",
            240,
        )
        .await
        else {
            eprintln!("no configured model; skipping");
            return;
        };
        report("a standing preference", &eval);

        let kept = h.runtime.workspace().read(h.id("Assistant"));
        println!("memory:\n{kept}");
        let lowered = kept.to_lowercase();
        assert!(
            lowered.contains("pound") && lowered.contains("10"),
            "a preference given for every future conversation was left in this one:\n{kept}"
        );
        assert!(
            h.runtime.store().agent_routines(h.id("Assistant")).unwrap().is_empty(),
            "a preference is not a routine: nothing here has to happen at a time"
        );
        eval.expect_told_operator("Assistant", 1, "a standing preference");
    }
}
