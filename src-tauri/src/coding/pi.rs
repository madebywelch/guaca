//! `pi`, as this app starts it and talks to it.
//!
//! Over `--mode rpc`: commands on stdin and events on stdout, one JSON object
//! per line. That is what makes a `pi` job reachable while it runs, the same
//! as the other two: `steer` puts a message in front of the model after the
//! tool calls it is running, `abort` ends the turn and keeps the session, and
//! a follow-up is the same process started again on the same `--session-id`.
//!
//! The push gate is a pi extension, because pi has no permission system of its
//! own and says so. Its `tool_call` hook can block a call, and its
//! `ctx.ui.confirm` arrives here as an `extension_ui_request` that this driver
//! answers. So the hook is three lines that decide nothing: every `bash` call
//! is asked about, and [`super::bridge::outward`] and the operator's desk
//! decide, exactly as they do for `shell` and the other two harnesses.

use std::collections::HashMap;
use std::process::Stdio;

use futures_util::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use super::{first_line, CodingError, Control, Job, Outcome, Progress, Signal};
use crate::domain::terminal::Tuning;

pub(super) const BINARY: &str = "pi";

pub(super) const INSTALL: &str = "npm install -g --ignore-scripts @earendil-works/pi-coding-agent";

/// The oldest `pi` whose RPC mode this build was measured against: `steer`,
/// `abort`, `--session-id` and the extension UI sub-protocol. Below it a job is
/// refused with the update command rather than run on a protocol nothing here
/// has checked.
pub(super) const RPC_FLOOR: (u32, u32) = (0, 84);

/// The title the gate's confirm carries, which is how it is told apart from a
/// dialog one of the operator's own extensions opens.
const GATE_TITLE: &str = "guaca:gate";

/// The command the gate registers, which is how this driver knows it loaded.
const GATE_COMMAND: &str = "guaca-gate";

/// The gate, as pi loads it.
///
/// It decides nothing. Every `bash` call is put to this driver as a confirm,
/// and the answer is the decision the other doors make. A deny blocks the
/// whole call, so `touch a && git push` runs neither half.
const GATE: &str = r#"export default function (pi) {
  pi.registerCommand("guaca-gate", { description: "Guaca's push gate", handler: async () => {} });
  pi.on("tool_call", async (event, ctx) => {
    if (event.toolName !== "bash") return;
    const ok = await ctx.ui.confirm("guaca:gate", String(event.input?.command ?? ""));
    if (!ok) return { block: true, reason: "The operator did not allow this. Nothing ran. Do not try it again or work around it: finish everything else you can, commit it, and say in your last message that this step is waiting on them." };
  });
}
"#;

/// The provider a job paid for with Guaca's key is told about.
///
/// Against OpenRouter, pi's own `openrouter` provider with only its address and
/// key replaced, so every model in pi's catalog keeps its context, thinking and
/// price. Anywhere else, a provider of the one model the job runs, because
/// there is no catalog to keep and pi will not run a model it has no entry for.
/// Measured against pi 0.84.4, whose `registerProvider` takes both forms.
fn provider(lease: &super::relay::Lease, model: &str) -> String {
    provider_source(&lease.base_url(), lease.token(), lease.openrouter(), model)
}

/// The same, from its parts, for a listing that has no lease.
pub(super) fn provider_source(base: &str, key: &str, openrouter: bool, model: &str) -> String {
    let base = serde_json::to_string(base).unwrap_or_default();
    let key = serde_json::to_string(key).unwrap_or_default();
    if openrouter {
        return format!(
            "export default function (pi) {{\n  pi.registerProvider(\"openrouter\", {{ baseUrl: {base}, apiKey: {key} }});\n}}\n"
        );
    }
    let model = serde_json::to_string(model).unwrap_or_default();
    format!(
        "export default function (pi) {{\n  pi.registerProvider(\"{LENT}\", {{ name: \"Guaca\", baseUrl: {base}, apiKey: {key}, api: \"openai-completions\",\n    models: [{{ id: {model}, name: {model}, reasoning: false, input: [\"text\"], cost: {{ input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }}, contextWindow: 128000, maxTokens: 16384 }}] }});\n}}\n"
    )
}

/// The provider id a job on an endpoint other than OpenRouter runs under.
const LENT: &str = "guaca";

/// `--mode rpc`, the session, the appended prompt, and the extensions.
///
/// `--session-id` names the session exactly, creating it the first time and
/// carrying it on after, which is what makes a follow-up and `pi --session`
/// find the same work. `--no-session` is deliberately *not* passed.
///
/// `--model` and `--thinking` only when the operator chose them for this
/// agent, and `--provider` only when Guaca's key pays. Absent, which sign-in
/// pays and which model runs are pi's own settings, exactly as they are when
/// the operator runs `pi` themselves.
pub(super) fn argv(
    session: &str,
    extensions: &[&std::path::Path],
    tuning: &Tuning,
    lease: Option<&super::relay::Lease>,
) -> Vec<String> {
    let mut args: Vec<String> = ["--mode", "rpc", "--append-system-prompt", super::APPENDED_PROMPT]
        .iter()
        .map(|arg| arg.to_string())
        .collect();
    if !session.is_empty() {
        args.push("--session-id".into());
        args.push(session.into());
    }
    for extension in extensions {
        args.push("-e".into());
        args.push(extension.to_string_lossy().into_owned());
    }
    if let Some(lease) = lease {
        args.push("--provider".into());
        args.push(if lease.openrouter() { "openrouter" } else { LENT }.into());
    }
    if let Some(model) = &tuning.model {
        args.push("--model".into());
        args.push(model.clone());
    }
    if let Some(effort) = &tuning.effort {
        args.push("--thinking".into());
        args.push(effort.clone());
    }
    args
}

/// A directory holding one job's extensions, removed when the job ends.
///
/// Beside nothing the job can see. The provider's file holds the job's relay
/// token, which is worthless off this machine and after the job, and is still
/// not left lying in the directory the job works in.
pub(super) struct Scratch {
    dir: std::path::PathBuf,
    files: Vec<std::path::PathBuf>,
}

impl Scratch {
    pub(super) fn new() -> std::io::Result<Self> {
        let dir = std::env::temp_dir().join(format!("guaca-pi-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir)?;
        Ok(Scratch { dir, files: Vec::new() })
    }

    pub(super) fn add(&mut self, name: &str, contents: &str) -> std::io::Result<()> {
        let at = self.dir.join(name);
        std::fs::write(&at, contents)?;
        self.files.push(at);
        Ok(())
    }

    pub(super) fn extensions(&self) -> Vec<&std::path::Path> {
        self.files.iter().map(|file| file.as_path()).collect()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// What a listing asks: every model pi can reach with the sign-ins it has.
pub(super) fn listing() -> Value {
    json!({"id": "models", "type": "get_available_models"})
}

pub(super) fn listed(line: &Value) -> bool {
    line["type"] == "response" && line["id"] == "models"
}

/// The models `/model` would offer. On pi's own sign-ins, each named with its
/// provider, which is what `--model` takes to pick one provider's model out of
/// several. On Guaca's key, only OpenRouter's, named as that provider's own,
/// because the job is started with `--provider openrouter`.
pub(super) fn offers(answer: &Value, lent: bool) -> Vec<super::ModelOffer> {
    const THINKING: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
    answer["data"]["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|model| !lent || model["provider"] == "openrouter")
        .filter_map(|model| {
            let id = model["id"].as_str()?;
            let provider = model["provider"].as_str().unwrap_or_default();
            Some(super::ModelOffer {
                id: if lent { id.to_string() } else { format!("{provider}/{id}") },
                label: model["name"].as_str().unwrap_or(id).to_string(),
                detail: provider.to_string(),
                default: false,
                efforts: match model["reasoning"] == true {
                    true => THINKING.iter().map(|level| level.to_string()).collect(),
                    false => Vec::new(),
                },
            })
        })
        .collect()
}

fn failed(why: impl Into<String>) -> CodingError {
    CodingError::NoAnswer(why.into())
}

async fn write(input: &mut tokio::process::ChildStdin, message: Value) -> Result<(), CodingError> {
    let mut bytes = serde_json::to_vec(&message).map_err(|e| failed(e.to_string()))?;
    bytes.push(b'\n');
    input.write_all(&bytes).await.map_err(|_| failed("pi closed its input pipe"))
}

/// Runs one job to its end over RPC.
pub(super) async fn run(
    job: &Job<'_>,
    mut controls: mpsc::Receiver<Control>,
    signals: mpsc::Sender<Signal>,
    watching: &mut (dyn FnMut(Progress) + Send),
) -> Result<Outcome, CodingError> {
    // The gate's file and the lent provider's, for as long as the job runs.
    let mut scratch = Scratch::new().map_err(|err| CodingError::Start(err.to_string()))?;
    let wrote =
        |result: std::io::Result<()>| result.map_err(|err| CodingError::Start(err.to_string()));
    if job.gate.asks() {
        wrote(scratch.add("gate.ts", GATE))?;
    }
    if let Some(lease) = job.lent {
        let model = job.tuning.model.as_deref().unwrap_or_default();
        wrote(scratch.add("provider.ts", &provider(lease, model)))?;
    }

    let mut command = tokio::process::Command::new(BINARY);
    job.env.apply(&mut command);
    let mut child = command
        .args(argv(job.session, &scratch.extensions(), job.tuning, job.lent))
        .current_dir(job.directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => {
                CodingError::NotInstalled { harness: "pi", install: INSTALL }
            }
            _ => CodingError::Start(err.to_string()),
        })?;
    let mut stdin = child.stdin.take().ok_or_else(|| failed("pi has no input pipe"))?;
    let stdout = child.stdout.take().ok_or_else(|| failed("pi has no output pipe"))?;
    let stderr = child.stderr.take().ok_or_else(|| failed("pi has no error pipe"))?;
    let draining = tokio::spawn(super::drain_stderr(stderr));

    let driving = drive(job, &mut stdin, stdout, &mut controls, signals, watching);
    let result = match tokio::time::timeout(super::CEILING, driving).await {
        Ok(result) => result,
        Err(_) => {
            let _ = child.kill().await;
            return Err(CodingError::TooLong(super::CEILING.as_secs() / 60));
        }
    };
    // RPC mode listens until its input closes, so closing it is how a settled
    // job ends. A process that does not go promptly is killed.
    drop(stdin);
    let status = match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
        Ok(Ok(status)) => status.code(),
        _ => {
            let _ = child.kill().await;
            None
        }
    };
    let stderr = draining.await.unwrap_or_default();
    drop(scratch);
    result.map_err(|error| match error {
        // Said with what the process said about itself, because "exited" alone
        // is the sentence nobody can act on.
        CodingError::NoAnswer(mut why) => {
            if let Some(code) = status {
                why = format!("{why} (exit {code})");
            }
            if !stderr.trim().is_empty() {
                why = format!("{why}: {}", stderr.trim());
            }
            failed(why)
        }
        other => other,
    })
}

async fn drive(
    job: &Job<'_>,
    stdin: &mut tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    controls: &mut mpsc::Receiver<Control>,
    signals: mpsc::Sender<Signal>,
    watching: &mut (dyn FnMut(Progress) + Send),
) -> Result<Outcome, CodingError> {
    let mut lines = BufReader::new(stdout).lines();
    let mut outcome = Outcome { session_id: job.session.to_string(), ..Outcome::default() };
    let mut steers: HashMap<String, oneshot::Sender<Result<(), String>>> = HashMap::new();
    let mut answers: FuturesUnordered<BoxFuture<'static, Value>> = FuturesUnordered::new();
    let mut next = 0u64;
    let mut stopping: Option<tokio::time::Instant> = None;
    let directory = job.directory.to_string();
    let mut drawing = |progress: Progress| {
        watching(match progress {
            Progress::Using { tool, detail } => {
                Progress::Using { tool, detail: super::shown(&directory, &detail) }
            }
            said => said,
        })
    };

    // A chosen model is checked against pi's catalog before anything else,
    // because pi runs one it has no entry for rather than refusing it.
    let mut resolved = Value::Null;
    if job.tuning.model.is_some() {
        write(stdin, json!({"id": "state", "type": "get_state"})).await?;
    } else {
        begin(job, stdin).await?;
    }

    loop {
        let grace = stopping.unwrap_or_else(|| tokio::time::Instant::now() + super::CEILING);
        tokio::select! {
            _ = tokio::time::sleep_until(grace), if stopping.is_some() => {
                return Ok(outcome);
            }
            Some(answer) = answers.next(), if !answers.is_empty() => {
                write(stdin, answer).await?;
            }
            Some(control) = controls.recv() => match control {
                Control::Steer { message, reply } => {
                    if stopping.is_some() {
                        let _ = reply.send(Err("the job is stopping".into()));
                        continue;
                    }
                    next += 1;
                    let id = format!("steer-{next}");
                    write(stdin, json!({"id": id, "type": "steer", "message": message})).await?;
                    steers.insert(id, reply);
                }
                Control::Stop => {
                    outcome.stopped = true;
                    if stopping.is_none() {
                        write(stdin, json!({"type": "abort"})).await?;
                        stopping = Some(tokio::time::Instant::now() + super::STOP_GRACE);
                    }
                }
            },
            line = lines.next_line() => {
                let Some(line) = line.map_err(|_| failed("could not read pi's output"))? else {
                    // Gone without settling. What it said is still an answer; a
                    // job that said nothing is one that did not start.
                    if outcome.stopped || !outcome.said.trim().is_empty() || outcome.failed.is_some() {
                        return Ok(outcome);
                    }
                    return Err(failed("pi exited before it finished the job"));
                };
                let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
                match event["type"].as_str().unwrap_or_default() {
                    "response" => {
                        let id = event["id"].as_str().unwrap_or_default();
                        let ok = event["success"] == Value::Bool(true);
                        let error = event["error"].as_str().unwrap_or("pi refused it").to_string();
                        if id == "state" {
                            resolved = event["data"]["model"].clone();
                            write(stdin, listing()).await?;
                        } else if listed(&event) {
                            match standing(&resolved, &event) {
                                Standing::Listed => begin(job, stdin).await?,
                                Standing::Behind => {
                                    let (provider, model) = (&resolved["provider"], &resolved["id"]);
                                    write(stdin, json!({"id": "model", "type": "set_model", "provider": provider, "modelId": model})).await?;
                                }
                                Standing::Unlisted => {
                                    return Err(CodingError::UnknownModel {
                                        model: job.tuning.model.clone().unwrap_or_default(),
                                    });
                                }
                            }
                        } else if id == "model" {
                            if !ok {
                                return Err(failed(format!(
                                    "pi could not take its catalog's entry for the model: {error}"
                                )));
                            }
                            begin(job, stdin).await?;
                        } else if id == "gate" {
                            let loaded = event["data"]["commands"]
                                .as_array()
                                .is_some_and(|all| all.iter().any(|c| c["name"] == GATE_COMMAND));
                            if !loaded {
                                return Err(failed(
                                    "pi did not load Guaca's push gate, so the job was not started. \
                                     Check that this pi loads extensions from `-e`, or turn off \
                                     Ask me before pushing for this agent",
                                ));
                            }
                            write(stdin, json!({"id": "brief", "type": "prompt", "message": job.brief})).await?;
                        } else if id == "brief" && !ok {
                            return Err(failed(format!("pi refused the brief: {error}")));
                        } else if let Some(reply) = steers.remove(id) {
                            let _ = reply.send(if ok { Ok(()) } else { Err(error) });
                        }
                    }
                    "extension_ui_request" => {
                        if let Some(answer) = answer_dialog(&event, job, &signals) {
                            answers.push(answer);
                        }
                    }
                    "agent_settled" => {
                        for (_, reply) in steers.drain() {
                            let _ = reply.send(Err("the job finished before this reached it".into()));
                        }
                        return Ok(outcome);
                    }
                    _ => absorb(&mut outcome, &event, &mut drawing),
                }
            }
        }
    }
}

/// The gate's check when there is a gate, and otherwise the brief.
///
/// The gate is checked before the brief is sent, so a job the operator asked
/// to be stopped before a push never starts ungated.
async fn begin(job: &Job<'_>, stdin: &mut tokio::process::ChildStdin) -> Result<(), CodingError> {
    if job.gate.asks() {
        write(stdin, json!({"id": "gate", "type": "get_commands"})).await
    } else {
        write(stdin, json!({"id": "brief", "type": "prompt", "message": job.brief})).await
    }
}

/// Where the model pi resolved stands against its catalog.
#[derive(Debug, PartialEq)]
enum Standing {
    /// The catalog's own entry, or no model at all, which pi reports itself.
    Listed,
    /// The catalog has an entry with other limits than the model pi resolved:
    /// it arrived after pi had already settled on a copy. `set_model` takes
    /// the entry, and writes nothing to pi's settings.
    Behind,
    /// The catalog has no entry, so a copy is all pi has.
    Unlisted,
}

/// Reads `get_state`'s model against a `get_available_models` answer.
///
/// pi does not refuse a `--model` it has no entry for under the provider it
/// was given. It copies that provider's default, renames the copy, and runs
/// it with the default's context and output limits. And it settles the model
/// before its first download of pi.dev's catalog lands, which RPC mode starts
/// in the background once the session exists: on a box's first job that
/// resolved `xiaomi/mimo-v2.6-pro` as Kimi K2.6 200 ms before the catalog that
/// listed it arrived. It asked for 209970 output tokens where the model's
/// providers take 131072; the provider OpenRouter routed to first accepted
/// that, and twelve minutes in, the one it fell back to refused it.
fn standing(resolved: &Value, catalog: &Value) -> Standing {
    if resolved.is_null() {
        return Standing::Listed;
    }
    let entry =
        catalog["data"]["models"].as_array().into_iter().flatten().find(|model| {
            model["provider"] == resolved["provider"] && model["id"] == resolved["id"]
        });
    match entry {
        None => Standing::Unlisted,
        Some(entry)
            if entry["contextWindow"] == resolved["contextWindow"]
                && entry["maxTokens"] == resolved["maxTokens"] =>
        {
            Standing::Listed
        }
        Some(_) => Standing::Behind,
    }
}

/// The answer to a dialog pi opened, or `None` for one that expects none.
///
/// The gate's confirm is decided here, from the same `outward` the other doors
/// ask. Any other dialog comes from one of the host user's own extensions, and
/// there is nobody at this end of a job to answer it: it is cancelled rather
/// than left open, because an unanswered dialog is a job that hangs until the
/// ceiling.
fn answer_dialog(
    event: &Value,
    job: &Job<'_>,
    signals: &mpsc::Sender<Signal>,
) -> Option<BoxFuture<'static, Value>> {
    let id = event["id"].as_str()?.to_string();
    let method = event["method"].as_str().unwrap_or_default();
    if !matches!(method, "select" | "confirm" | "input" | "editor") {
        return None;
    }
    if method != "confirm" || event["title"] != GATE_TITLE {
        return Some(
            async move { json!({"type": "extension_ui_response", "id": id, "cancelled": true}) }
                .boxed(),
        );
    }
    let line = event["message"].as_str().unwrap_or_default().to_string();
    let root = std::path::PathBuf::from(job.directory);
    let signals = signals.clone();
    Some(
        async move {
            let allowed = match super::bridge::outward(&line, &root).await {
                None => true,
                Some(reach) => {
                    let (reply, decision) = oneshot::channel();
                    signals.send(Signal::Permission { line, reach, reply }).await.is_ok()
                        && decision.await.unwrap_or(false)
                }
            };
            json!({"type": "extension_ui_response", "id": id, "confirmed": allowed})
        }
        .boxed(),
    )
}

/// Folds one event from pi's stream into the outcome.
///
/// Its own function so the tests drive the real thing. It was written inline
/// and the tests kept a copy of the match beside it, which is how a missing
/// arm passed CI and shipped: the copy captured `stopReason` and the parser
/// never did, so every failed job in a live workspace was reported as a job
/// with nothing to do. A test that mirrors the code under test asserts that
/// the mirror is correct.
pub(super) fn absorb(
    outcome: &mut Outcome,
    event: &serde_json::Value,
    watching: &mut dyn FnMut(Progress),
) {
    match event["type"].as_str().unwrap_or_default() {
        "tool_execution_start" => {
            outcome.tool_calls += 1;
            if let Some(name) = event["toolName"].as_str() {
                watching(Progress::Using {
                    tool: name.to_string(),
                    detail: detail_of(name, &event["args"]),
                });
            }
        }
        // The authoritative message, as opposed to the deltas: pi's own
        // documentation says `message_end` is the final one, and reassembling
        // the text from `text_delta` would be a second copy of the same string
        // that could disagree with it.
        "message_end" if event["message"]["role"] == "assistant" => {
            let said = text_of(&event["message"]);
            if !said.trim().is_empty() {
                watching(Progress::Said(said.clone()));
                // Kept rather than appended. Every assistant message is a round
                // of one turn, and the last one is the answer; joined, a job
                // that narrated its work would report the narration as its
                // result.
                outcome.said = said;
            }
            if let Some(model) = event["message"]["model"].as_str() {
                outcome.model = model.to_string();
            }
            // A turn that ended on an error, which `pi` reports here and then
            // exits zero about. Taken from the last message rather than the
            // first, so a turn that failed and was retried successfully is not
            // a failed job.
            outcome.failed = match event["message"]["stopReason"].as_str() {
                Some("error") => Some(
                    event["message"]["errorMessage"]
                        .as_str()
                        .unwrap_or("the harness did not say why")
                        .to_string(),
                ),
                _ => None,
            };
        }
        "message_update" => {
            // Cumulative rather than additive: pi reports the running total on
            // every update, so adding them up multiplies the bill by the number
            // of updates.
            if let Some(total) = event["usage"]["cost"]["total"].as_f64() {
                if total > 0.0 {
                    outcome.cost = Some(total);
                }
            }
        }
        _ => {}
    }
}

/// The one argument of a tool call worth putting on a line.
///
/// A command, a path, a pattern. Not the whole `args`: a `write` carries the
/// entire file in it, and a watcher wants to know that `src/api.go` is being
/// written rather than to read it going past.
///
/// The names are `pi`'s built-ins, which are lowercase and are not Claude
/// Code's. Anything else falls back to nothing rather than guessing a field,
/// because a wrong guess here prints somebody's file contents into a channel.
fn detail_of(tool: &str, args: &serde_json::Value) -> String {
    let pick = match tool {
        "bash" => "command",
        "read" | "write" | "edit" => "path",
        "grep" | "find" => "pattern",
        _ => return String::new(),
    };
    first_line(args[pick].as_str().unwrap_or_default()).to_string()
}

/// Every text part of a message, joined.
fn text_of(message: &serde_json::Value) -> String {
    message["content"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter(|part| part["type"] == "text")
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives the real parser, not a copy of it.
    fn drive(lines: &[&str]) -> Outcome {
        let mut outcome = Outcome::default();
        let mut seen = Vec::new();
        for line in lines {
            let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            absorb(&mut outcome, &event, &mut |p| seen.push(p));
        }
        outcome
    }

    #[test]
    fn the_last_thing_said_is_the_answer_and_not_the_narration() {
        // A model narrating its work says a sentence before each tool call.
        // Joined, a job reports "Let me look at the tests" as its result.
        let outcome = drive(&[
            r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Let me look at the tests."}]}}"#,
            r#"{"type":"tool_execution_start","toolName":"bash"}"#,
            r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Fixed and pushed."}],"model":"gpt-5.6"}}"#,
        ]);
        assert_eq!(outcome.said, "Fixed and pushed.");
        assert_eq!(outcome.tool_calls, 1);
        assert_eq!(outcome.model, "gpt-5.6");
    }

    #[test]
    fn the_tests_drive_the_parser_the_runtime_uses() {
        // The guard on the whole file. `drive` used to keep its own copy of the
        // match, so an arm the parser was missing passed here against the copy:
        // `stopReason` was read by the test and by nothing else, and every
        // failed coding job in a live workspace came back as a job that found
        // nothing to do. If `absorb` is ever inlined again, this is the test
        // that stops being about anything.
        let progress = std::cell::RefCell::new(Vec::new());
        let mut outcome = Outcome::default();
        let event: serde_json::Value =
            serde_json::from_str(r#"{"type":"tool_execution_start","toolName":"bash"}"#).unwrap();
        absorb(&mut outcome, &event, &mut |p| progress.borrow_mut().push(p));

        assert_eq!(outcome.tool_calls, 1);
        assert_eq!(
            progress.borrow().as_slice(),
            [Progress::Using { tool: "bash".into(), detail: String::new() }]
        );
    }

    #[test]
    fn a_turn_that_ended_on_an_error_is_a_failure_and_not_an_empty_job() {
        // The one that cost an afternoon. `pi` reports a failed turn inside its
        // own stream and exits zero, so an expired credential arrives looking
        // exactly like a job that found nothing to do, and every agent in the
        // workspace dutifully reported that nothing needed doing.
        let outcome = drive(&[
            r#"{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"error","errorMessage":"Provided authentication token is expired."}}"#,
        ]);
        assert_eq!(outcome.failed.as_deref(), Some("Provided authentication token is expired."));
        assert_eq!(outcome.tool_calls, 0);
        assert!(outcome.said.is_empty(), "an errored turn carries no answer");
    }

    #[test]
    fn a_spent_plan_is_a_failure_the_operator_can_act_on() {
        // The shape of the day this was built for: a provider that is signed in
        // and out of quota. It is a 400 inside the stream, not a dead process,
        // and the operator's way out is the other harness.
        let outcome = drive(&[
            r#"{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"error","errorMessage":"400 {\"type\":\"error\",\"error\":{\"message\":\"You're out of extra usage.\"}}"}}"#,
        ]);
        assert!(outcome.failed.unwrap().contains("out of extra usage"));
    }

    #[test]
    fn a_turn_that_failed_and_was_retried_is_not_a_failed_job() {
        // Taken from the last message rather than the first. A harness that
        // retried and then finished has done the work, and reporting the first
        // wobble as the outcome throws the result away.
        let outcome = drive(&[
            r#"{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"error","errorMessage":"overloaded"}}"#,
            r#"{"type":"tool_execution_start","toolName":"edit"}"#,
            r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Fixed and pushed."}],"stopReason":"stop"}}"#,
        ]);
        assert_eq!(outcome.failed, None);
        assert_eq!(outcome.said, "Fixed and pushed.");
    }

    #[test]
    fn an_errored_turn_with_no_message_still_says_something() {
        let outcome = drive(&[
            r#"{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"error"}}"#,
        ]);
        assert!(outcome.failed.is_some(), "silence here is what this exists to prevent");
    }

    #[test]
    fn a_cost_is_the_running_total_and_is_never_added_up() {
        // pi reports cumulative usage on every update. Summed, the bill comes
        // back multiplied by the number of updates.
        let outcome = drive(&[
            r#"{"type":"message_update","usage":{"cost":{"total":0.01}}}"#,
            r#"{"type":"message_update","usage":{"cost":{"total":0.04}}}"#,
            r#"{"type":"message_update","usage":{"cost":{"total":0.09}}}"#,
        ]);
        assert_eq!(outcome.cost, Some(0.09));
    }

    #[test]
    fn a_provider_that_prices_nothing_reports_nothing_rather_than_zero() {
        // Absent is not free, and a zero here would be added into a total the
        // operator reads as what the work cost.
        let outcome = drive(&[r#"{"type":"message_update","usage":{"cost":{"total":0}}}"#]);
        assert_eq!(outcome.cost, None);
    }

    #[test]
    fn a_users_own_message_is_not_mistaken_for_the_answer() {
        let outcome = drive(&[
            r#"{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"fix the test"}]}}"#,
        ]);
        assert_eq!(outcome.said, "");
    }

    #[test]
    fn a_line_that_is_not_json_does_not_end_the_stream() {
        // Anything on stdout that is not an event: a warning, a progress bar, a
        // line from a tool that did not respect the mode.
        let outcome =
            drive(&["npm warn something", r#"{"type":"tool_execution_start","toolName":"edit"}"#]);
        assert_eq!(outcome.tool_calls, 1);
    }

    #[test]
    fn the_session_is_named_and_the_prompt_is_appended() {
        let args = argv("s1", &[], &Tuning::default(), None);
        assert!(args.contains(&"--mode".to_string()) && args.contains(&"rpc".to_string()));
        assert!(args.contains(&super::super::APPENDED_PROMPT.to_string()));
        let after =
            |flag: &str| args.iter().position(|arg| arg == flag).map(|at| args[at + 1].clone());
        // Named rather than read back, so a follow-up and `pi --session` both
        // find this work, and never session-less.
        assert_eq!(after("--session-id").as_deref(), Some("s1"));
        assert!(!args.contains(&"--no-session".to_string()));
        // Which sign-in pays is pi's own setting. Choosing between programs is
        // the agent's harness, and it is not spelled as a provider flag.
        assert!(!args.contains(&"--provider".to_string()));
        // The brief goes over the protocol, not the command line.
        assert!(!args.contains(&"-p".to_string()));
        assert!(!args.contains(&"-e".to_string()), "no gate unless the agent asks for one");
        // Nothing chosen is pi's own model and thinking.
        assert!(!args.contains(&"--model".to_string()));
        assert!(!args.contains(&"--thinking".to_string()));

        let gate = std::path::Path::new("/tmp/guaca-pi-x/gate.ts");
        let gated = argv("s1", &[gate], &Tuning::default(), None);
        let at = gated.iter().position(|arg| arg == "-e").expect("the gate is loaded");
        assert_eq!(gated[at + 1], "/tmp/guaca-pi-x/gate.ts");
    }

    #[test]
    fn a_chosen_model_and_thinking_are_pis_own_flags() {
        let tuning = Tuning {
            model: Some("anthropic/claude-sonnet-4.5".into()),
            effort: Some("high".into()),
            ..Tuning::default()
        };
        let args = argv("s1", &[], &tuning, None);
        let after =
            |flag: &str| args.iter().position(|arg| arg == flag).map(|at| args[at + 1].clone());
        assert_eq!(after("--model").as_deref(), Some("anthropic/claude-sonnet-4.5"));
        assert_eq!(after("--thinking").as_deref(), Some("high"));
        // pi's own sign-in still pays: no provider is named.
        assert!(!args.contains(&"--provider".to_string()));
    }

    #[test]
    fn a_job_on_guacas_key_names_the_lent_provider_and_never_the_key() {
        let tuning = Tuning { model: Some("qwen/qwen3-coder".into()), ..Tuning::default() };
        let openrouter = super::super::relay::Lease::fixed(4100, "tok-1", true);
        let args = argv("s1", &[], &tuning, Some(&openrouter));
        let at = args.iter().position(|arg| arg == "--provider").unwrap();
        assert_eq!(args[at + 1], "openrouter", "pi's own catalog is kept");
        let written = provider(&openrouter, "qwen/qwen3-coder");
        assert!(written.contains(r#"pi.registerProvider("openrouter""#), "{written}");
        assert!(written.contains(r#""http://127.0.0.1:4100/v1""#), "{written}");
        assert!(written.contains(r#""tok-1""#), "{written}");
        assert!(!written.contains("models:"), "the catalog's models are kept: {written}");

        let elsewhere = super::super::relay::Lease::fixed(4101, "tok-2", false);
        let args = argv("s1", &[], &tuning, Some(&elsewhere));
        let at = args.iter().position(|arg| arg == "--provider").unwrap();
        assert_eq!(args[at + 1], LENT);
        let written = provider(&elsewhere, "qwen/qwen3-coder");
        assert!(written.contains(r#"id: "qwen/qwen3-coder""#), "{written}");
        assert!(written.contains(r#"api: "openai-completions""#), "{written}");
    }

    #[test]
    fn the_model_list_names_each_model_the_way_the_job_will_ask_for_it() {
        let answer = json!({"type":"response","id":"models","command":"get_available_models","success":true,"data":{"models":[
            {"id":"claude-opus-4-7","provider":"anthropic","name":"Claude Opus 4.7","reasoning":true},
            {"id":"anthropic/claude-sonnet-4.5","provider":"openrouter","name":"Anthropic: Claude Sonnet 4.5","reasoning":true},
            {"id":"qwen/qwen3-coder","provider":"openrouter","name":"Qwen3 Coder","reasoning":false}
        ]}});
        assert!(listed(&answer));
        let own = offers(&answer, false);
        assert_eq!(own[0].id, "anthropic/claude-opus-4-7", "provider first, for `--model`");
        assert_eq!(own[1].id, "openrouter/anthropic/claude-sonnet-4.5");
        assert_eq!(own[0].efforts.first().map(String::as_str), Some("off"));

        let lent = offers(&answer, true);
        let ids: Vec<&str> = lent.iter().map(|offer| offer.id.as_str()).collect();
        assert_eq!(ids, ["anthropic/claude-sonnet-4.5", "qwen/qwen3-coder"]);
        assert!(lent[1].efforts.is_empty(), "a model that does not think offers no level");
    }

    #[test]
    fn a_model_pi_copied_from_its_default_is_told_apart_from_one_it_knows() {
        let kimi = json!({"id":"moonshotai/kimi-k2.6","provider":"openrouter","contextWindow":262144,"maxTokens":235929});
        // What pi settled on: the default's limits under the id it was asked for.
        let copied = json!({"id":"xiaomi/mimo-v2.6-pro","name":"xiaomi/mimo-v2.6-pro","provider":"openrouter","contextWindow":262144,"maxTokens":235929});
        let before =
            json!({"data":{"models":[kimi, {"id":"claude-opus-4-7","provider":"anthropic"}]}});
        assert_eq!(standing(&copied, &before), Standing::Unlisted);
        let elsewhere = json!({"id":"claude-opus-4-7","provider":"openrouter"});
        assert_eq!(
            standing(&elsewhere, &before),
            Standing::Unlisted,
            "an id is a model only under its provider"
        );
        assert_eq!(standing(&kimi, &before), Standing::Listed);

        // The catalog landed after pi settled: the entry is there, and the
        // copy in use still has the default's limits.
        let mimo = json!({"id":"xiaomi/mimo-v2.6-pro","provider":"openrouter","contextWindow":1048576,"maxTokens":131072});
        let after = json!({"data":{"models":[kimi, mimo]}});
        assert_eq!(standing(&copied, &after), Standing::Behind);
        assert_eq!(standing(&mimo, &after), Standing::Listed);

        // No model at all is pi's to report, in its own words.
        assert_eq!(standing(&Value::Null, &after), Standing::Listed);
    }

    #[test]
    fn the_gate_asks_about_every_bash_call_and_decides_nothing_itself() {
        // What decides is `outward` and the operator's desk, the same as every
        // other door. An extension with its own list of dangerous commands would
        // be a second answer to what counts as outward-facing.
        assert!(GATE.contains(r#"event.toolName !== "bash""#));
        assert!(GATE.contains(GATE_TITLE));
        assert!(GATE.contains(GATE_COMMAND));
        assert!(!GATE.contains("git "), "the gate must not carry its own rule");
        assert!(GATE.contains("block: true"));
    }

    #[test]
    fn a_tool_this_build_does_not_know_prints_nothing_rather_than_a_guess() {
        // A wrong guess here puts somebody's file contents into a channel.
        assert_eq!(detail_of("mcp__whatever", &serde_json::json!({"secret": "hunter2"})), "");
        assert_eq!(detail_of("bash", &serde_json::json!({"command": "npm test"})), "npm test");
        assert_eq!(detail_of("read", &serde_json::json!({"path": "src/api.go"})), "src/api.go");
    }
}
