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

use guac_lib::domain::agent::CleanDraft;
use guac_lib::domain::envelope::Envelope;
use guac_lib::domain::ids::{AgentId, RunId};
use guac_lib::eval::{analyse, faults, Conversation, Fault};
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::trajectory::Trajectory;

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

    // Every channel, not `conversation_flow`: that one is built for the
    // activity view and excludes the system channel, which is where refusals
    // are recorded. An eval that cannot see a refusal cannot tell a guard doing
    // its job from a message that silently went nowhere.
    let mut messages: Vec<Envelope> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in names {
        for envelope in h.runtime.store().channel_messages(h.id(name), 400).unwrap() {
            if seen.insert(envelope.id) {
                messages.push(envelope);
            }
        }
    }
    messages.sort_by_key(|e| (e.created_at, e.id));

    Eval {
        convo: analyse(&messages, &name_of),
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
/// A model that has reported once and is then woken by an acknowledgement
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
        } else if body["messages"]
            .as_array()
            .map(|m| m.iter().filter_map(|m| m["content"].as_str()).any(|c| c.contains("noted")))
            .unwrap_or(false)
        {
            // The habit: say it again, word for word.
            Script::SendTo {
                recipients: vec!["Chef".into(), "Baker".into()],
                text: "the time is 11:03pm".into(),
            }
        } else if body["messages"]
            .as_array()
            .map(|m| m.iter().filter_map(|m| m["content"].as_str()).any(|c| c.contains("11:03pm")))
            .unwrap_or(false)
        {
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
    // an acknowledgement, has nothing to add, and says nothing. The cost of
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
            // Staggered, the way real model calls come back.
            let pause = if text.contains("Baker") {
                500
            } else if text.contains("Grocer") {
                1000
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

// ---- live scenarios ------------------------------------------------------

/// Runs the same instructions against the configured model.
///
/// Ignored by default: it costs money, it needs a key, and it cannot be
/// deterministic. Run it with `./scripts/evals.sh` after changing a prompt,
/// which is the change CI cannot see.
mod live {
    use super::*;

    fn configured() -> Option<guac_lib::config::AppConfig> {
        let path = dirs_config()?;
        let raw = std::fs::read_to_string(path).ok()?;
        let config: guac_lib::config::AppConfig = serde_json::from_str(&raw).ok()?;
        (!config.inference.api_key.trim().is_empty()).then_some(config)
    }

    fn dirs_config() -> Option<std::path::PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(
            std::path::PathBuf::from(home)
                .join("Library/Application Support/com.madebywelch.guac/config.json"),
        )
    }

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
        let config = configured()?;
        let before = machines_now(&config).await;
        let h = live_crew(config.clone(), crew);
        let names: Vec<&str> = crew.iter().map(|a| a.name).collect();
        let run = h.runtime.send_from_human(h.id(names[0]), instruction).unwrap();

        let settled = h.settled_within(run, secs).await;
        // Before the assertions, because an assertion that fails takes the rest
        // of the function with it, and what is left standing is a real machine
        // billing for its idle period. Twenty accumulated this way in an
        // afternoon, and the timeouts left the most behind.
        release_machines(&config, before).await;

        assert!(settled, "run did not settle in {secs}s. messages so far:\n{}", h.transcript());
        let eval = read(&h, run, &names);
        Some((h, eval))
    }

    use guac_lib::computer::provider::ComputerProvider;

    /// Every machine this account holds, by provider id.
    async fn machines_now(config: &guac_lib::config::AppConfig) -> Vec<String> {
        match guac_lib::computer::e2b::E2bProvider::new(&config.e2b.api_key) {
            Some(provider) => provider.list_owned().await.unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// Kills whatever this run brought into existence, and nothing else.
    ///
    /// A diff against a baseline rather than a walk over the crew's rows. An
    /// agent whose first machine does not answer starts another and records
    /// only the newest, so the rows name one of the several a single run can
    /// leave behind; seventeen survived a cleanup written that way.
    ///
    /// It is also why `Runtime::sweep_computers` cannot be borrowed for this.
    /// That kills every Guac machine its own store does not claim, so run from
    /// a throwaway store it would spare this crew's machines and take the
    /// operator's running app apart instead.
    async fn release_machines(config: &guac_lib::config::AppConfig, before: Vec<String>) {
        let Some(provider) = guac_lib::computer::e2b::E2bProvider::new(&config.e2b.api_key) else {
            return;
        };
        let existing: std::collections::HashSet<String> = before.into_iter().collect();
        for machine in provider.list_owned().await.unwrap_or_default() {
            if existing.contains(&machine) {
                continue;
            }
            // The id is the whole address a delete needs; the tokens on a
            // handle reach *into* a machine and this only removes one.
            let handle = guac_lib::computer::provider::ProviderHandle {
                computer: guac_lib::domain::ids::ComputerId::new(),
                provider_id: machine.clone(),
                control_secret: guac_lib::domain::computer::Secret::default(),
                viewer_secret: guac_lib::domain::computer::Secret::default(),
            };
            match provider.delete(&handle).await {
                Ok(()) => println!("released {machine}"),
                Err(err) => eprintln!("could not release {machine}: {err}"),
            }
        }
    }

    /// Which peers were sent something, by name, with a count each.
    fn recipients(h: &Harness, names: &[&str]) -> Vec<(String, usize)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for envelope in h.feed() {
            if let Some(id) = envelope.to.agent_id() {
                if let Some(name) = names.iter().find(|n| h.id(n) == id) {
                    *counts.entry((*name).to_string()).or_default() += 1;
                }
            }
        }
        let mut out: Vec<(String, usize)> = counts.into_iter().collect();
        out.sort();
        out
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
        let names: Vec<&str> = crew.iter().map(|a| a.name).collect();

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

        let got = recipients(&h, &names);
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
}

/// One agent in a live crew.
///
/// Skills are here because a scenario about who should get a piece of work is
/// not testable without them: a crew of four agents with no stated skills gives
/// a coordinator nothing to choose between, and the broadcast it produces is
/// then the right answer.
struct LiveAgent {
    name: &'static str,
    skills: &'static [&'static str],
    /// `None` takes a serviceable default. `Some("")` means the card really
    /// carries no instructions, which is how most agents are created and is
    /// what the workspace rules have to hold up without.
    prompt: Option<&'static str>,
    /// Overrides the configured default. A crew is not obliged to share a
    /// model, and a coordinator on a different one from the agents it directs
    /// is the arrangement that produced the defect these evals were written
    /// for: putting everyone on the default quietly tests a different app.
    model: Option<&'static str>,
}

impl LiveAgent {
    /// An agent for scenarios that are not about who does what.
    fn generic(name: &'static str) -> Self {
        LiveAgent { name, skills: &[], prompt: None, model: None }
    }

    fn system_prompt(&self) -> String {
        match self.prompt {
            Some(prompt) => prompt.to_string(),
            None => format!(
                "You are the {}. Work with your team the way the workspace rules say.",
                self.name
            ),
        }
    }
}

/// A harness pointed at the real configured endpoint rather than a stub.
fn live_crew(config: guac_lib::config::AppConfig, crew: &[LiveAgent]) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let store = guac_lib::db::Store::open(&dir.path().join("guac.db")).unwrap();

    let mut ids = HashMap::new();
    for agent in crew {
        let card = store
            .create_agent(&CleanDraft {
                name: agent.name.to_string(),
                avatar: "plain".into(),
                color: "#c7d96b".into(),
                model: agent
                    .model
                    .map(str::to_string)
                    .unwrap_or_else(|| config.inference.default_model.clone()),
                system_prompt: agent.system_prompt(),
                skills: agent.skills.iter().map(|s| (*s).to_string()).collect(),
                group_id: None,
            })
            .unwrap();
        ids.insert(agent.name.to_string(), card.id);
    }

    let sink = guac_lib::runtime::events::RecordingSink::new();
    let runtime = guac_lib::runtime::Runtime::new(
        store,
        guac_lib::llm::openrouter::LlmClient::new().unwrap(),
        config,
        guac_lib::workspace::Workspace::new(dir.path().join("workspace")),
        guac_lib::files::FileStore::new(dir.path().join("files")),
        sink.clone(),
    );
    runtime.start_all().unwrap();
    Harness { runtime, sink, ids, _dir: dir }
}
