//! Codex's app-server protocol. One CLI process owns one thread and turn.
//! Steering uses the active turn id and is acknowledged by the CLI. Approval
//! callbacks spend the same repository gate as Claude hooks and `shell`.

use std::{collections::HashSet, path::Path, process::Stdio, time::Duration};

use futures_util::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{mpsc, oneshot},
};

use super::{CodingError, Outcome, Progress, Signal};
use crate::domain::repository::Gate;

pub(super) const BINARY: &str = "codex";
pub(super) const INSTALL: &str = "npm install -g @openai/codex";
const RESPONSE_LIMIT: Duration = Duration::from_secs(30);

/// The response belongs to the operator's call, so a late or rejected steer
/// never gets the same success message as an accepted correction.
pub struct Steer {
    pub message: String,
    pub reply: oneshot::Sender<Result<(), String>>,
}

pub struct Control {
    pub gate: Gate,
    pub steering: mpsc::Receiver<Steer>,
    pub signals: mpsc::Sender<Signal>,
}

fn failed(why: impl Into<String>) -> CodingError {
    CodingError::NoAnswer(why.into())
}

pub async fn run(
    repository: &str,
    task: &str,
    control: Option<Control>,
    mut watching: impl FnMut(Progress),
) -> Result<Outcome, CodingError> {
    let mut command = tokio::process::Command::new(BINARY);
    crate::repo::github::environment(repository, &mut command).await;
    let mut child = command
        .args(["app-server", "--listen", "stdio://"])
        .current_dir(repository)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                CodingError::NotInstalled { harness: "Codex", install: INSTALL }
            } else {
                CodingError::Start(err.to_string())
            }
        })?;
    let stdin = child.stdin.take().ok_or_else(|| failed("Codex has no input pipe"))?;
    let stdout = child.stdout.take().ok_or_else(|| failed("Codex has no output pipe"))?;
    let stderr = child.stderr.take().ok_or_else(|| failed("Codex has no error pipe"))?;
    let finishing = async {
        let protocol = async {
            let result = drive(repository, task, control, stdin, stdout, &mut watching).await;
            // An app-server keeps listening after turn/completed. End this
            // job's server and reap it before releasing the worktree lock.
            let _ = child.kill().await;
            let _ = child.wait().await;
            result
        };
        let (result, stderr) = tokio::join!(protocol, super::drain_stderr(stderr));
        result.map_err(|error| match error {
            CodingError::NoAnswer(why) if !stderr.trim().is_empty() => {
                failed(format!("{why}: {}", stderr.trim()))
            }
            other => other,
        })
    };
    match tokio::time::timeout(super::CEILING, finishing).await {
        Ok(result) => result,
        Err(_) => {
            let _ = child.kill().await;
            Err(CodingError::TooLong(super::CEILING.as_secs() / 60))
        }
    }
}

async fn write(input: &mut tokio::process::ChildStdin, message: Value) -> Result<(), CodingError> {
    let mut bytes = serde_json::to_vec(&message).map_err(|e| failed(e.to_string()))?;
    bytes.push(b'\n');
    input.write_all(&bytes).await.map_err(|_| failed("Codex closed its input pipe"))
}

fn input(text: &str) -> Value {
    json!([{ "type": "text", "text": text }])
}

async fn drive(
    repository: &str,
    task: &str,
    control: Option<Control>,
    mut stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    watching: &mut impl FnMut(Progress),
) -> Result<Outcome, CodingError> {
    let (gate, mut steering, signals) = match control {
        Some(control) => (control.gate, control.steering, control.signals),
        None => {
            let (_, steering) = mpsc::channel(1);
            let (signals, _) = mpsc::channel(1);
            (Gate::Open, steering, signals)
        }
    };
    write(
        &mut stdin,
        json!({"id":1,"method":"initialize","params":{
            "clientInfo":{"name":"guaca","title":"Guaca","version":env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi":false}
        }}),
    )
    .await?;
    let mut lines = BufReader::new(stdout).lines();
    let mut outcome = Outcome::default();
    let mut thread = String::new();
    let mut turn = String::new();
    let mut ready = false;
    let mut complete = false;
    let mut accepting = true;
    let mut next_id = 4u64;
    let mut pending: Option<(u64, oneshot::Sender<Result<(), String>>)> = None;
    let mut deadline = tokio::time::Instant::now() + RESPONSE_LIMIT;
    let mut approvals: FuturesUnordered<BoxFuture<'static, Value>> = FuturesUnordered::new();
    let mut items = HashSet::new();

    loop {
        if complete && pending.is_none() {
            return Ok(outcome);
        }
        tokio::select! {
            _ = tokio::time::sleep_until(deadline), if !ready || pending.is_some() => {
                if let Some((_, reply)) = pending.take() {
                    let _ = reply.send(Err("Codex did not acknowledge the correction. The job was stopped; review its changes before retrying.".into()));
                }
                return Err(failed("Codex did not answer a control request within 30 seconds"));
            }
            Some(answer) = approvals.next(), if !approvals.is_empty() => {
                write(&mut stdin, answer).await?;
            }
            correction = steering.recv(), if ready && accepting && pending.is_none() && !complete => {
                let Some(correction) = correction else { accepting = false; continue; };
                if correction.reply.is_closed() { continue; }
                if correction.message.trim().is_empty() {
                    let _ = correction.reply.send(Err("Enter a correction to send".into()));
                    continue;
                }
                let id = next_id;
                next_id += 1;
                write(&mut stdin, json!({"id":id,"method":"turn/steer","params":{
                    "threadId":thread,"expectedTurnId":turn,"input":input(&correction.message)
                }})).await?;
                pending = Some((id, correction.reply));
                deadline = tokio::time::Instant::now() + RESPONSE_LIMIT;
            }
            line = lines.next_line() => {
                let Some(line) = line.map_err(|_| failed("Could not read Codex output"))? else {
                    return Err(failed("Codex exited before finishing the job"));
                };
                let Ok(event) = serde_json::from_str::<Value>(&line) else { continue; };
                if let Some(method) = event["method"].as_str() {
                    let params = &event["params"];
                    if event.get("id").is_some() {
                        approvals.push(answer_request(event.clone(), gate, repository.into(), thread.clone(), turn.clone(), signals.clone()).boxed());
                        continue;
                    }
                    if params["threadId"].as_str() != Some(thread.as_str()) { continue; }
                    match method {
                        "turn/started" if turn.is_empty() => {
                            turn = params["turn"]["id"].as_str().unwrap_or_default().into();
                        }
                        "item/started" | "item/completed" if params["turnId"].as_str() == Some(turn.as_str()) => {
                            absorb(&mut outcome, &params["item"], method == "item/completed", &mut items, watching, repository);
                        }
                        "turn/completed" if params["turn"]["id"].as_str() == Some(turn.as_str()) => {
                            complete = true;
                            steering.close();
                            while let Ok(correction) = steering.try_recv() {
                                let _ = correction.reply.send(Err("The job finished before this correction was sent. Start a new coding job.".into()));
                            }
                            match params["turn"]["status"].as_str() {
                                Some("completed") => {}
                                Some("failed") => outcome.failed = Some(params["turn"]["error"]["message"].as_str().unwrap_or("Codex reported a failed turn").into()),
                                Some("interrupted") => outcome.failed = Some("Codex interrupted the turn; partial changes may remain".into()),
                                _ => return Err(failed("Codex returned an unknown completion status")),
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                let Some(id) = event["id"].as_u64() else { continue; };
                if pending.as_ref().is_some_and(|(waiting, _)| *waiting == id) {
                    let (_, reply) = pending.take().unwrap();
                    let accepted = event.get("error").is_none() && event["result"]["turnId"].as_str() == Some(turn.as_str());
                    let answer = if accepted { Ok(()) } else {
                        Err(event["error"]["message"].as_str().unwrap_or("Codex did not accept the correction for this turn").into())
                    };
                    let _ = reply.send(answer);
                    continue;
                }
                if let Some(error) = event.get("error") {
                    return Err(failed(error["message"].as_str().unwrap_or("Codex refused to start the job")));
                }
                let result = &event["result"];
                match id {
                    1 => {
                        write(&mut stdin, json!({"method":"initialized"})).await?;
                        write(&mut stdin, json!({"id":2,"method":"thread/start","params":{
                            "cwd":repository, "approvalPolicy": if gate == Gate::AskBeforePushing { "untrusted" } else { "never" },
                            "approvalsReviewer":"user", "sandbox":"danger-full-access",
                            "developerInstructions":super::APPENDED_PROMPT,
                            "serviceName":"guaca"
                        }})).await?;
                        deadline = tokio::time::Instant::now() + RESPONSE_LIMIT;
                    }
                    2 => {
                        thread = result["thread"]["id"].as_str().filter(|id| !id.is_empty()).ok_or_else(|| failed("Codex returned no thread id"))?.into();
                        if gate == Gate::AskBeforePushing && (result["approvalPolicy"] != "untrusted" || result["approvalsReviewer"] != "user") {
                            return Err(failed("Codex did not enable Guaca's approval policy; the coding job was not started"));
                        }
                        outcome.session_id = thread.clone();
                        outcome.model = result["model"].as_str().unwrap_or_default().into();
                        write(&mut stdin, json!({"id":3,"method":"turn/start","params":{
                            "threadId":thread, "input":input(task)
                        }})).await?;
                        deadline = tokio::time::Instant::now() + RESPONSE_LIMIT;
                    }
                    3 => {
                        let started = result["turn"]["id"].as_str().filter(|id| !id.is_empty()).ok_or_else(|| failed("Codex returned no active turn id"))?;
                        if !turn.is_empty() && turn != started { return Err(failed("Codex returned conflicting turn ids")); }
                        turn = started.into();
                        ready = true;
                    }
                    _ => {}
                }
            }
        }
    }
}

/// The CLI handles its ordinary edits. Only outward shell actions spend the
/// repository gate, through the same signal and decision as Claude's hooks.
async fn answer_request(
    event: Value,
    gate: Gate,
    repository: String,
    thread: String,
    turn: String,
    signals: mpsc::Sender<Signal>,
) -> Value {
    let id = &event["id"];
    let params = &event["params"];
    if params["threadId"].as_str() != Some(thread.as_str())
        || params["turnId"].as_str() != Some(turn.as_str())
    {
        return json!({"id":id,"error":{"code":-32602,"message":"Request does not belong to the active coding turn"}});
    }
    let result = match event["method"].as_str().unwrap_or_default() {
        "item/commandExecution/requestApproval" => {
            let allowed = if gate == Gate::Open {
                true
            } else if let Some(line) = params["command"].as_str() {
                let cwd = params["cwd"].as_str().unwrap_or(&repository);
                if let Some(reach) = super::bridge::outward(line, Path::new(cwd)).await {
                    let (reply, decision) = oneshot::channel();
                    signals
                        .send(Signal::Permission { line: line.into(), reach, reply })
                        .await
                        .is_ok()
                        && decision.await.unwrap_or(false)
                } else {
                    true
                }
            } else {
                false
            };
            // Never grant a session-wide exemption that would skip later gates.
            json!({"decision":if allowed {"accept"} else {"decline"}})
        }
        "item/fileChange/requestApproval" => json!({"decision":"accept"}),
        "item/tool/requestUserInput" => json!({"answers":{}}),
        "mcpServer/elicitation/request" => json!({"action":"decline","content":null}),
        "item/permissions/requestApproval" => json!({"permissions":{},"scope":"turn"}),
        _ => {
            return json!({"id":id,"error":{"code":-32601,"message":"Guaca does not support this Codex callback"}})
        }
    };
    json!({"id":id,"result":result})
}

fn absorb(
    outcome: &mut Outcome,
    item: &Value,
    completed: bool,
    seen: &mut HashSet<String>,
    watching: &mut dyn FnMut(Progress),
    repository: &str,
) {
    let kind = item["type"].as_str().unwrap_or_default();
    if kind == "agentMessage" && completed {
        let text = item["text"].as_str().unwrap_or_default();
        if !text.trim().is_empty() {
            outcome.said = text.into();
            watching(Progress::Said(text.into()));
        }
    } else if matches!(kind, "commandExecution" | "fileChange" | "mcpToolCall" | "webSearch") {
        let Some(id) = item["id"].as_str() else {
            return;
        };
        if !seen.insert(id.into()) {
            return;
        }
        outcome.tool_calls += 1;
        let (tool, detail) = match kind {
            "commandExecution" => ("shell", item["command"].as_str().unwrap_or_default()),
            "fileChange" => ("edit", ""),
            "mcpToolCall" => (item["tool"].as_str().unwrap_or("MCP"), ""),
            _ => ("search", ""),
        };
        watching(Progress::Using { tool: tool.into(), detail: super::shown(repository, detail) });
    }
}
