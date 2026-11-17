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

const CHIEF: &str = "Chief of Staff";

const DEFAULT_MODEL: &str = "openai/gpt-5.6-luna";

const STANDING_INSTRUCTION: &str =
    "You are the Chief of Staff. Work the operator brings you is yours to place: decide who on \
     the team each piece belongs to, send it to them, and answer the operator yourself once you \
     have what you need. You do not do the team's work yourself.";

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

const DIRECTIVE: &str = "We're launching the Pro tier on the 15th of next month and I want to \
                         walk into Monday's leadership meeting with one package. I need a \
                         one-page positioning brief, a read on whether our current terms of \
                         service cover usage-based billing, the fifty accounts we should \
                         approach first and the reasoning for those fifty, and what the launch \
                         does to next quarter's numbers. Bring it back to me together.";

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
    config.limits.max_steps_per_run = steps as u32;

    let crew = crew();
    let names: Vec<&str> = crew.iter().map(|agent| agent.name).collect();

    let dir = recordings().join(stamp());
    std::fs::create_dir_all(&dir).unwrap();
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

    let recorder = Recorder::open(&dir.join("events.jsonl"));
    let machines = machines_now(config).await;
    let h = live_crew_watched(config.clone(), crew, recorder);
    let (answering, asked) = answer_permission_requests(&h);

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
    let settled = settle_and_release(&h, run, secs, answering, machines).await;
    let seconds = started.elapsed().as_secs();

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

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RunReport {
    run: usize,
    settled: bool,
    seconds: u64,
    calls: usize,
    steps: Option<u32>,
    prompt_tokens: u64,
    completion_tokens: u64,
    cost: Option<f64>,
    peak_concurrency: usize,
    peer_messages: usize,
    max_hop: u16,
    told_operator: Vec<(String, String)>,
    delegated: Vec<(String, usize)>,
    turns: Vec<(String, usize)>,
    chased: Vec<String>,
    tools: BTreeMap<String, usize>,
    refusals: Vec<String>,
    silent: bool,
    faults: Vec<String>,
    anomalies: Vec<String>,
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

    fn chose(&self) -> String {
        if self.delegated.is_empty() {
            return "nobody".to_string();
        }
        self.delegated.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(", ")
    }
}

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

fn chases(messages: &[Envelope], name_of: &dyn Fn(AgentId) -> String) -> Vec<String> {
    let mut waiting: BTreeSet<(AgentId, AgentId)> = BTreeSet::new();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();

    for envelope in messages {
        let (Participant::Agent { id: from }, Participant::Agent { id: to }) =
            (envelope.from, envelope.to)
        else {
            continue;
        };
        waiting.remove(&(to, from));
        if envelope.intent.is_work() && !waiting.insert((from, to)) {
            *counts.entry(format!("{} chased {}", name_of(from), name_of(to))).or_default() += 1;
        }
    }

    counts.into_iter().map(|(pair, times)| format!("{pair} {times} time(s)")).collect()
}

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
                Part::Decision { request, answer, resumed, .. } => format!(
                    "\n[{} {:?}]\n{answer}\n",
                    if *resumed { "resumes decision" } else { "answers decision" },
                    request.question
                ),
                Part::Json { name, value } => format!("\n[{name}]\n\n```json\n{value}\n```\n"),
                Part::Tidy { reported, notes } => {
                    format!("\n[overnight tidy]\n{}\n{}\n", reported.join("\n"), notes.join("\n"))
                }
            });
        }
    }
    out
}

fn clip(arguments: &serde_json::Value) -> String {
    let rendered = arguments.to_string();
    match rendered.char_indices().nth(400) {
        Some((at, _)) => format!("{}…", &rendered[..at]),
        None => rendered,
    }
}

fn setting(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|value| value.trim().to_string()).filter(|v| !v.is_empty())
}

fn number(name: &str, fallback: usize) -> usize {
    match setting(name) {
        Some(raw) => raw.parse().unwrap_or_else(|_| panic!("{name} must be a number, got {raw:?}")),
        None => fallback,
    }
}

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
