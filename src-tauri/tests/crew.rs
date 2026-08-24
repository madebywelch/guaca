//! One directive, eight agents, a real model, and a record of what happened.
//!
//! The other three suites each ask a question with one right answer. The
//! cascade tests ask whether the runtime did as it was told; the evals ask
//! whether a crew communicated like something worth watching; the trajectory
//! suite asks whether the machinery behaved. All three can be asserted because
//! all three are decidable.
//!
//! This one is not, and that is the point of it. A real model given a real
//! instruction and a team of eight makes a different set of decisions every
//! time, and the interesting failures live in the spread rather than in any one
//! run: the delegation that goes to five agents on the third attempt and one on
//! the first two, the fan-in that reports twice when a reply lands late, the
//! turn that only hangs when two answers arrive in the same batch. A suite that
//! runs once and asserts cannot see any of that. So this runs the same
//! directive as many times as it is asked to, writes down everything each run
//! did, and says what was different.
//!
//! What it does assert is the part that is not a matter of taste: every run
//! settled, no run left the machinery in a state `trajectory.rs` calls broken,
//! and somebody answered the operator. A crew that chose oddly is a finding to
//! read. A placeholder left open is a defect.
//!
//! ```sh
//! ./scripts/crew.sh          # one run, on the default model
//! GUACA_RUNS=5 ./scripts/crew.sh
//! ```

mod harness;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use guac_lib::config::AppConfig;
use guac_lib::domain::envelope::{Envelope, Part, Participant};
use guac_lib::domain::ids::AgentId;
use guac_lib::eval::{analyze, faults, Fault};
use guac_lib::runtime::events::{EventSink, UiEvent};

use harness::live::*;
use harness::*;

/// The agent the operator talks to. Everything else is reached through it.
const CHIEF: &str = "Chief of Staff";

/// Cheap, fast, and a competent tool caller, which is what eight agents making
/// several calls each needs to be. Every run records the model it used, so a
/// comparison between two of them is two runs and a diff rather than a rebuild:
/// `GUACA_MODEL=anthropic/claude-sonnet-5 ./scripts/crew.sh`.
const DEFAULT_MODEL: &str = "openai/gpt-5.6-luna";

/// What an operator types when they have one person they talk to.
///
/// Close to the `ONLY_DELEGATES` instruction the evals use, because the failure
/// it produces is the same one and the two are worth comparing: an agent told
/// not to do the work either answers from its own head, asks everybody, or
/// takes the instruction as a reason to do nothing. The addition is the fan-in,
/// which a crew of two cannot be asked about: four answers have to become one
/// package, and the operator is waiting for the package rather than for the
/// pieces.
const STANDING_INSTRUCTION: &str =
    "You are the Chief of Staff. Work the operator brings you is yours to place: decide who on \
     the team each piece belongs to, send it to them, and answer the operator yourself once you \
     have what you need. You do not do the team's work yourself.";

/// The eight, and the ambiguity is deliberate.
///
/// Four of the directive's asks have an obvious owner and three of the crew
/// have no part in it at all, which is the arrangement that separates a
/// coordinator that chooses from one that hands everybody a piece. The
/// Market Researcher overlaps the Content Marketer on positioning and the Data
/// Analyst on sizing, so there is a real judgment to make rather than a lookup.
/// The Executive Assistant is bait: the directive mentions a meeting on Monday
/// and never asks for anything to be done about it.
///
/// Only the Chief of Staff carries instructions. The other seven are described
/// by their skills and nothing else, because that is how most agents are
/// created, and a crew whose specialists each carry a hand-written brief is a
/// test of the briefs.
fn crew() -> Vec<LiveAgent> {
    vec![
        LiveAgent::told(CHIEF, &["coordination", "prioritization"], STANDING_INSTRUCTION),
        LiveAgent::skilled(
            "Executive Assistant",
            &["calendar", "inbox", "travel booking", "meeting logistics"],
        ),
        LiveAgent::skilled(
            "Sales Development Rep",
            &["outbound prospecting", "lead qualification", "cold email"],
        ),
        LiveAgent::skilled("Content Marketer", &["blog posts", "positioning copy", "editing"]),
        LiveAgent::skilled("Paralegal", &["contract review", "compliance", "filings"]),
        LiveAgent::skilled(
            "Product Manager",
            &["product requirements", "roadmap", "release scoping"],
        ),
        LiveAgent::skilled(
            "Market Researcher",
            &["competitor analysis", "market sizing", "customer interviews"],
        ),
        LiveAgent::skilled("Data Analyst", &["SQL", "metrics", "forecasting"]),
    ]
}

/// Four asks with four owners, in the sentence an operator would type.
///
/// Deliberately not a numbered list: a list is a delegation plan already
/// written, and what is being watched is whether one gets written at all.
const DIRECTIVE: &str = "We're launching the Pro tier on the 15th of next month and I want to \
                         walk into Monday's leadership meeting with one package. I need a \
                         one-page positioning brief, a read on whether our current terms of \
                         service cover usage-based billing, the fifty accounts we should \
                         approach first and the reasoning for those fifty, and what the launch \
                         does to next quarter's numbers. Bring it back to me together.";

// ---- the run -------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "live: costs money, needs a configured key"]
async fn a_directive_to_the_chief_of_staff() {
    let Some(mut config) = configured() else {
        eprintln!("no configured model; skipping");
        return;
    };

    let model = setting("GUACA_MODEL").unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let runs = number("GUACA_RUNS", 1);
    let steps = number("GUACA_STEPS", 120);
    let secs = number("GUACA_SETTLE_SECS", 420) as u64;
    let directive = setting("GUACA_DIRECTIVE").unwrap_or_else(|| DIRECTIVE.to_string());

    config.inference.default_model = model.clone();
    // The operator's own limits otherwise, because the guard is part of what is
    // being watched: a fan-out ceiling low enough to refuse a broadcast would
    // answer the question this suite is asking. The step budget is the
    // exception, and it is the only real spend control: a crew of eight that
    // will not converge is a bill, not a finding.
    config.limits.max_steps_per_run = steps as u32;

    let crew = crew();
    let names: Vec<&str> = crew.iter().map(|agent| agent.name).collect();

    let dir = recordings().join(stamp());
    std::fs::create_dir_all(&dir).unwrap();
    // Printed several times and pasted into a shell at least once, so it is
    // worth being a path rather than a route to one.
    let dir = dir.canonicalize().unwrap_or(dir);
    write(&dir.join("directive.txt"), &format!("to: {CHIEF}\n\n{directive}\n"));
    write(
        &dir.join("setup.json"),
        &serde_json::to_string_pretty(&serde_json::json!({
            "model": model,
            "runs": runs,
            "settleSecs": secs,
            "limits": config.limits,
            "crew": crew.iter().map(|agent| serde_json::json!({
                "name": agent.name,
                "skills": agent.skills,
                "prompt": agent.system_prompt(),
            })).collect::<Vec<_>>(),
        }))
        .unwrap(),
    );

    println!("\n==> {runs} run(s) of one directive on {model}");
    println!("    at most {steps} model call(s) per run, {secs}s to settle in");
    println!("    recording to {}", dir.display());

    let mut reports = Vec::new();
    for run in 1..=runs {
        let report =
            one_run(&config, &crew, &names, &directive, secs, run, &dir.join(format!("run-{run}")))
                .await;
        println!("{}", report.line());
        reports.push(report);
    }

    let across = compare(&reports, &model);
    write(&dir.join("across-runs.md"), &across);
    println!("\n{across}\nrecorded in {}\n", dir.display());

    // Only the three that are not a matter of taste. Everything else about a
    // run is in the comparison above, which is what the suite is for.
    let unsettled = which(&reports, |report| !report.settled);
    assert!(
        unsettled.is_empty(),
        "run(s) {unsettled:?} never settled inside {secs}s. Either the crew did not converge or \
         the runtime lost a turn; the events in {} say which",
        dir.display()
    );
    let broken = which(&reports, |report| !report.anomalies.is_empty());
    assert!(
        broken.is_empty(),
        "run(s) {broken:?} left the machinery in a state trajectory.rs calls broken. This is a \
         defect in Guaca rather than a decision a model made:\n{}",
        reports
            .iter()
            .filter(|report| !report.anomalies.is_empty())
            .flat_map(|report| report
                .anomalies
                .iter()
                .map(|anomaly| format!("  run {}: {anomaly}", report.run)))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let silent = which(&reports, |report| report.silent);
    assert!(
        silent.is_empty(),
        "in run(s) {silent:?} the operator asked for something and was never told anything by \
         anybody, which is the one outcome worse than a bad answer"
    );
}

#[allow(clippy::too_many_arguments)]
async fn one_run(
    config: &AppConfig,
    crew: &[LiveAgent],
    names: &[&str],
    directive: &str,
    secs: u64,
    number: usize,
    dir: &Path,
) -> RunReport {
    std::fs::create_dir_all(dir).unwrap();

    // Opened before the crew exists, so a run that never comes back still
    // leaves everything up to the moment it stopped. `tail -f` works on it.
    let recorder = Recorder::open(&dir.join("events.jsonl"));
    let before = machines_now(config).await;
    let h = live_crew_watched(config.clone(), crew, recorder);
    let (answering, asked) = answer_permission_requests(&h);

    // Written before the first message rather than with the rest, because the
    // events name agents by id and a run that hangs is one whose record has to
    // be readable without it having finished. Per run, since every run builds
    // its crew in a fresh store and none of the ids survive.
    let lookup: HashMap<AgentId, String> =
        names.iter().map(|name| (h.id(name), (*name).to_string())).collect();
    write(
        &dir.join("agents.json"),
        &serde_json::to_string_pretty(
            &lookup.iter().map(|(id, name)| (id.to_string(), name)).collect::<BTreeMap<_, _>>(),
        )
        .unwrap(),
    );

    println!("--> run {number}: {} agents, asking {CHIEF}", crew.len());
    let started = Instant::now();
    let run = h.runtime.send_from_human(h.id(CHIEF), directive).unwrap();
    let settled = h.settled_within(run, secs).await;
    let seconds = started.elapsed().as_secs();
    answering.abort();
    // Before anything that can fail, exactly as the evals do it: a run that
    // overruns is a run whose agents are all still working, and those are the
    // ones that leave the most behind.
    release_machines(config, before).await;

    let messages = h.envelopes(names);
    let name_of = |id: AgentId| lookup.get(&id).cloned().unwrap_or_else(|| "?".into());
    let convo = analyze(&messages, &name_of);
    let found = faults(&messages, &name_of);
    let trajectory = h.trajectory(run);

    let mut tools: BTreeMap<String, usize> = BTreeMap::new();
    for tool in trajectory.tools() {
        *tools.entry(tool.to_string()).or_default() += 1;
    }

    let (prompt_tokens, completion_tokens) = trajectory.tokens();
    let report = RunReport {
        run: number,
        settled,
        seconds,
        calls: trajectory.calls(),
        steps: trajectory.steps(),
        prompt_tokens,
        completion_tokens,
        cost: spend(&h),
        peak_concurrency: trajectory.peak_concurrency(),
        peer_messages: convo.between_agents,
        max_hop: convo.max_hop,
        told_operator: convo
            .to_operator
            .iter()
            .map(|(who, text)| (who.clone(), text.clone()))
            .collect(),
        delegated: h.messaged_by(CHIEF),
        turns: names
            .iter()
            .map(|name| ((*name).to_string(), trajectory.turns(h.id(name))))
            .collect(),
        chased: chases(&messages, &name_of),
        tools,
        refusals: trajectory.refusals(),
        silent: found.contains(&Fault::Silent),
        faults: found.iter().map(Fault::explain).collect(),
        anomalies: trajectory.anomalies().iter().map(|a| a.explain()).collect(),
        approvals: asked.lock().clone(),
    };

    write(&dir.join("transcript.md"), &render(&messages, &lookup, &report));
    write(&dir.join("messages.json"), &serde_json::to_string_pretty(&messages).unwrap());
    write(&dir.join("summary.json"), &serde_json::to_string_pretty(&report).unwrap());
    write(&dir.join("ledger.txt"), &trajectory.ledger);
    report
}

// ---- what one run did ----------------------------------------------------

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RunReport {
    run: usize,
    settled: bool,
    seconds: u64,
    /// Model calls, which is the unit the budget and the bill are both in.
    calls: usize,
    steps: Option<u32>,
    prompt_tokens: u64,
    completion_tokens: u64,
    /// Dollars, when the provider prices a call. Absent rather than zero: a
    /// local endpoint charges nothing and reports nothing, and the two must not
    /// add up the same.
    cost: Option<f64>,
    peak_concurrency: usize,
    peer_messages: usize,
    max_hop: u16,
    told_operator: Vec<(String, String)>,
    /// Who the Chief of Staff sent something to, with a count each. The
    /// decision the whole scenario exists to watch.
    delegated: Vec<(String, usize)>,
    turns: Vec<(String, usize)>,
    /// Work sent to a peer that had not answered the last thing yet, by pair.
    ///
    /// The prompt asks for this in so many words: a send returns once the
    /// message is queued, a reply arrives later on its own, and calling
    /// `send_message` again to check for one is named as the thing not to do.
    /// It is not one of `eval.rs`'s faults, so nothing else counts it, and a
    /// coordinator waiting on four answers is exactly where it happens.
    chased: Vec<String>,
    tools: BTreeMap<String, usize>,
    refusals: Vec<String>,
    silent: bool,
    faults: Vec<String>,
    anomalies: Vec<String>,
    /// Permission requests, all of which were declined.
    approvals: Vec<String>,
}

impl RunReport {
    fn line(&self) -> String {
        format!(
            "    run {}: {} in {}s, {} call(s){}, {} peer message(s), told the operator {} \
             time(s), {} fault(s), {} anomal{}",
            self.run,
            if self.settled { "settled" } else { "DID NOT SETTLE" },
            self.seconds,
            self.calls,
            match self.cost {
                Some(cost) => format!(" (${cost:.4})"),
                None => String::new(),
            },
            self.peer_messages,
            self.told_operator.len(),
            self.faults.len(),
            self.anomalies.len(),
            if self.anomalies.len() == 1 { "y" } else { "ies" },
        )
    }

    /// Who was given work, as one comparable string.
    fn chose(&self) -> String {
        if self.delegated.is_empty() {
            return "nobody".to_string();
        }
        self.delegated.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(", ")
    }
}

/// What every model call this run reported costing.
///
/// Read from the events rather than from the trajectory, which keeps the token
/// counts and drops the price: a comparison between two models is a comparison
/// of what they charged, and tokens alone cannot make it.
fn spend(h: &Harness) -> Option<f64> {
    let mut total = None;
    for event in h.sink.snapshot() {
        let Ok(value) = serde_json::to_value(&event) else { continue };
        if value["type"] != "tokensUsed" {
            continue;
        }
        if let Some(cost) = value["cost"].as_f64() {
            total = Some(total.unwrap_or(0.0) + cost);
        }
    }
    total
}

/// Work sent to a peer that had not answered the last one yet.
///
/// Decidable from the envelopes and nothing else: a pair is waiting from the
/// moment work is sent until anything comes back the other way, and a second
/// piece of work inside that window is a chase. Counted here rather than in
/// `eval.rs` because it has not yet been seen often enough to be a fault: a
/// crew is allowed to send a peer a second, different thing to do, and telling
/// that apart from nagging needs more than one run of evidence.
fn chases(messages: &[Envelope], name_of: &dyn Fn(AgentId) -> String) -> Vec<String> {
    let mut waiting: BTreeSet<(AgentId, AgentId)> = BTreeSet::new();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();

    for envelope in messages {
        let (Participant::Agent { id: from }, Participant::Agent { id: to }) =
            (envelope.from, envelope.to)
        else {
            continue;
        };
        // Anything back the other way ends the wait, whatever it says: an
        // agent that has spoken is an agent that is no longer silent.
        waiting.remove(&(to, from));
        if envelope.intent.is_work() && !waiting.insert((from, to)) {
            *counts.entry(format!("{} chased {}", name_of(from), name_of(to))).or_default() += 1;
        }
    }

    counts.into_iter().map(|(pair, times)| format!("{pair} {times} time(s)")).collect()
}

// ---- what was different --------------------------------------------------

/// The whole reason this suite runs more than once.
///
/// One table of numbers and then, for each thing worth comparing, the distinct
/// answers and which runs gave them. A dimension with one answer was stable; a
/// dimension with five is where to start reading the transcripts.
fn compare(reports: &[RunReport], model: &str) -> String {
    let mut out = format!("# {} run(s) of one directive on `{model}`\n\n", reports.len());

    out.push_str(
        "| run | settled | secs | calls | prompt | completion | cost | peer msgs | hop | told \
         operator | peak |\n|---|---|---|---|---|---|---|---|---|---|---|\n",
    );
    for report in reports {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            report.run,
            if report.settled { "yes" } else { "**no**" },
            report.seconds,
            report.calls,
            report.prompt_tokens,
            report.completion_tokens,
            match report.cost {
                Some(cost) => format!("${cost:.4}"),
                None => "-".to_string(),
            },
            report.peer_messages,
            report.max_hop,
            report.told_operator.len(),
            report.peak_concurrency,
        ));
    }

    out.push_str("\n## What varied\n\n");
    out.push_str(&varied("who the Chief of Staff sent work to", reports, RunReport::chose));
    out.push_str(&varied("peer messages", reports, |r| r.peer_messages.to_string()));
    out.push_str(&varied("model calls", reports, |r| r.calls.to_string()));
    out.push_str(&varied("times the operator was told", reports, |r| {
        r.told_operator.len().to_string()
    }));
    // The two halves of that number, because they are different complaints. A
    // coordinator that answers four times is the one the operator notices; a
    // specialist writing one line in its own channel is the workspace working.
    out.push_str(&varied("times the Chief of Staff answered", reports, |r| {
        r.told_operator.iter().filter(|(who, _)| who == CHIEF).count().to_string()
    }));
    out.push_str(&varied("who answered the operator", reports, |r| {
        set(r.told_operator.iter().map(|(who, _)| who.clone()))
    }));
    out.push_str(&varied("chased for an answer", reports, |r| set(r.chased.iter().cloned())));
    out.push_str(&varied("deepest hop", reports, |r| r.max_hop.to_string()));
    out.push_str(&varied("tools used", reports, |r| set(r.tools.keys().cloned())));
    out.push_str(&varied("guard refusals", reports, |r| set(r.refusals.iter().cloned())));
    out.push_str(&varied("faults", reports, |r| set(r.faults.iter().cloned())));
    out.push_str(&varied("anomalies", reports, |r| set(r.anomalies.iter().cloned())));
    out.push_str(&varied("permission requests", reports, |r| r.approvals.len().to_string()));

    out.push_str("\n## What the operator was told\n");
    for report in reports {
        out.push_str(&format!("\n### run {}\n", report.run));
        if report.told_operator.is_empty() {
            out.push_str("\nNothing.\n");
        }
        for (who, text) in &report.told_operator {
            out.push_str(&format!("\n**{who}**\n\n{text}\n"));
        }
    }
    out
}

/// One dimension, and which runs gave which answer.
fn varied(label: &str, reports: &[RunReport], of: impl Fn(&RunReport) -> String) -> String {
    let mut answers: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for report in reports {
        answers.entry(of(report)).or_default().push(report.run);
    }
    if answers.len() == 1 {
        let (answer, _) = answers.into_iter().next().expect("one answer");
        return format!("- {label}: {answer}, in every run\n");
    }
    let mut out = format!("- **{label}: {} different answers**\n", answers.len());
    for (answer, runs) in answers {
        let runs: Vec<String> = runs.iter().map(usize::to_string).collect();
        out.push_str(&format!("  - {answer} (run {})\n", runs.join(", ")));
    }
    out
}

/// A set of strings, ordered and joined, so two runs that produced the same
/// things in a different order compare equal.
fn set(values: impl Iterator<Item = String>) -> String {
    let ordered: BTreeSet<String> = values.collect();
    if ordered.is_empty() {
        return "none".to_string();
    }
    ordered.into_iter().collect::<Vec<_>>().join("; ")
}

fn which(reports: &[RunReport], fits: impl Fn(&RunReport) -> bool) -> Vec<usize> {
    reports.iter().filter(|report| fits(report)).map(|report| report.run).collect()
}

// ---- the recording -------------------------------------------------------

/// Every event the runtime emitted, timestamped, written as it happened.
///
/// The same stream the webview is drawn from and the same one `trajectory.rs`
/// reads, so a question about what the operator would have seen is answerable
/// from the file rather than from a rerun. Timestamps are here and nowhere near
/// an assertion: a wall clock in a test is a flake, and a wall clock in a
/// recording is how you find the turn that sat for four minutes.
struct Recorder {
    file: parking_lot::Mutex<std::fs::File>,
    started: Instant,
}

impl Recorder {
    fn open(path: &Path) -> Arc<Self> {
        Arc::new(Recorder {
            file: parking_lot::Mutex::new(std::fs::File::create(path).unwrap()),
            started: Instant::now(),
        })
    }
}

impl EventSink for Recorder {
    fn emit(&self, event: UiEvent) {
        let line = serde_json::json!({
            "atMs": self.started.elapsed().as_millis() as u64,
            "event": event,
        });
        let mut file = self.file.lock();
        if let Err(err) = writeln!(file, "{line}") {
            eprintln!("could not record an event: {err}");
        }
    }
}

/// The conversation, as a person would read it.
///
/// `messages.json` beside it is the same thing without a decision taken about
/// what matters, and this is the one that gets read: every envelope in the
/// order it was filed, named rather than numbered, carrying the two fields that
/// decide whether a cascade terminates.
fn render(messages: &[Envelope], names: &HashMap<AgentId, String>, report: &RunReport) -> String {
    let who = |participant: Participant| match participant {
        Participant::Human => "OPERATOR".to_string(),
        Participant::System => "Guaca".to_string(),
        Participant::Agent { id } => {
            names.get(&id).cloned().unwrap_or_else(|| format!("agent {}", id.short()))
        }
    };

    let mut out = format!("# run {}\n\n{}\n\n", report.run, report.line().trim());
    let first = messages.first().map(|envelope| envelope.created_at).unwrap_or_default();
    let at = |envelope: &Envelope| (envelope.created_at - first) as f64 / 1000.0;

    // One line per message before the messages themselves, because a cascade
    // is a shape and a shape cannot be read at the pace of full paragraphs.
    // Everything a runaway is recognized by is in it: work going out to five
    // peers at once, a hop count climbing past where the operator's message
    // started, a specialist answering the operator while its coordinator is
    // still consolidating.
    out.push_str("## The shape\n\n| at | hop | intent | reply | from → to | chars | tools |\n");
    out.push_str("|---|---|---|---|---|---:|---|\n");
    for envelope in messages {
        let tools: Vec<&str> = envelope
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::ToolCall { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        out.push_str(&format!(
            "| +{:.1}s | {} | {} | {} | {} → {} | {} | {} |\n",
            at(envelope),
            envelope.hop,
            envelope.intent.as_str(),
            if envelope.expects_reply { "yes" } else { "" },
            who(envelope.from),
            who(envelope.to),
            envelope.plain_text().chars().count(),
            tools.join(", "),
        ));
    }

    out.push_str("\n## The messages\n");
    for envelope in messages {
        out.push_str(&format!(
            "\n### +{:.1}s  {} → {}  (hop {}, {}{})\n",
            at(envelope),
            who(envelope.from),
            who(envelope.to),
            envelope.hop,
            envelope.intent.as_str(),
            if envelope.expects_reply { ", reply expected" } else { "" },
        ));
        for part in &envelope.parts {
            out.push_str(&match part {
                Part::Text { text } => format!("\n{text}\n"),
                Part::Notice { kind, text } => format!("\n> [{kind:?}] {text}\n"),
                Part::ToolCall { name, arguments, outcome, .. } => {
                    format!("\n`{name}` → {outcome:?}\n\n```json\n{}\n```\n", clip(arguments))
                }
                Part::File(file) => format!("\n[file: {}]\n", file.name),
                Part::Approval { summary, .. } => format!("\n[asks the operator: {summary}]\n"),
                Part::Question { question, options, .. } => {
                    format!(
                        "\n[asks the operator: {question}]\n[options: {}]\n",
                        options.join(" | ")
                    )
                }
                Part::Routine { name, what, .. } => format!("\n[routine {name:?}] {what}\n"),
                Part::Json { name, value } => format!("\n[{name}]\n\n```json\n{value}\n```\n"),
            });
        }
    }
    out
}

/// Tool arguments, shortened. A `send_message` carries the whole message it
/// sent, and the transcript is already the place that message is read.
fn clip(arguments: &serde_json::Value) -> String {
    let rendered = arguments.to_string();
    match rendered.char_indices().nth(400) {
        Some((at, _)) => format!("{}…", &rendered[..at]),
        None => rendered,
    }
}

// ---- plumbing ------------------------------------------------------------

fn setting(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|value| value.trim().to_string()).filter(|v| !v.is_empty())
}

/// A number from the environment, or the default. A value that is not a number
/// is a typo in a command line, and taking the default silently is how a five
/// hundred step budget gets spent on a run somebody thought was bounded.
fn number(name: &str, fallback: usize) -> usize {
    match setting(name) {
        Some(raw) => raw.parse().unwrap_or_else(|_| panic!("{name} must be a number, got {raw:?}")),
        None => fallback,
    }
}

/// Where recordings go: beside the repo, never inside the build directory.
///
/// `cargo clean` is something people run without thinking about it, and the
/// runs are the product of this suite rather than a build artifact of it.
fn recordings() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runs")
}

fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d-%H%M%S").to_string()
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents)
        .unwrap_or_else(|err| panic!("could not write {}: {err}", path.display()));
}
