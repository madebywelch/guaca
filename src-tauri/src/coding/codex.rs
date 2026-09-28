//! Codex's app-server protocol. One CLI process owns one thread and one turn.
//!
//! A new job starts a thread and a follow-up resumes it, so the work carries on
//! with everything it had already read. Steering uses the active turn id and is
//! acknowledged by the CLI; a stop is `turn/interrupt`, which ends the turn and
//! leaves the thread for the next follow-up. Approval callbacks spend the same
//! agent gate as Claude hooks, `pi`'s extension and `shell`.

use std::{collections::HashSet, path::Path, process::Stdio, time::Duration};

use futures_util::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{mpsc, oneshot},
};

use super::{CodingError, Control, Job, Outcome, Progress, Signal};
use crate::domain::terminal::Gate;

pub(super) const BINARY: &str = "codex";
pub(super) const INSTALL: &str = "npm install -g @openai/codex";
const RESPONSE_LIMIT: Duration = Duration::from_secs(30);

fn failed(why: impl Into<String>) -> CodingError {
    CodingError::NoAnswer(why.into())
}

pub(super) async fn run(
    job: &Job<'_>,
    controls: mpsc::Receiver<Control>,
    signals: mpsc::Sender<Signal>,
    watching: &mut (dyn FnMut(Progress) + Send),
) -> Result<Outcome, CodingError> {
    let mut command = tokio::process::Command::new(BINARY);
    job.env.apply(&mut command);
    let mut child = command
        .args(["app-server", "--listen", "stdio://"])
        .current_dir(job.directory)
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
            let result = drive(job, controls, signals, stdin, stdout, watching).await;
            // An app-server keeps listening after turn/completed, until its
            // input closes, which `drive` returning just did. Reaped before the
            // agent's job slot is freed, and killed only if it does not go.
            if tokio::time::timeout(Duration::from_secs(5), child.wait()).await.is_err() {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
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

/// What a listing says to the app-server: the handshake and one page of
/// models, large enough to be every one.
pub(super) fn listing() -> [Value; 3] {
    [
        json!({"id":1,"method":"initialize","params":{
            "clientInfo":{"name":"guaca","title":"Guaca","version":env!("CARGO_PKG_VERSION")}
        }}),
        json!({"method":"initialized"}),
        json!({"id":2,"method":"model/list","params":{"limit":200}}),
    ]
}

pub(super) fn listed(line: &Value) -> bool {
    line["id"] == 2
}

/// The models the picker shows, with the efforts each one advertises.
/// Hidden ones are left out, as the picker leaves them out.
pub(super) fn offers(answer: &Value) -> Vec<super::ModelOffer> {
    answer["result"]["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|model| model["hidden"] != true)
        .filter_map(|model| {
            let id = model["id"].as_str().or_else(|| model["model"].as_str())?;
            Some(super::ModelOffer {
                id: id.to_string(),
                label: model["displayName"].as_str().unwrap_or(id).to_string(),
                detail: model["description"].as_str().unwrap_or_default().to_string(),
                default: model["isDefault"] == true,
                efforts: model["supportedReasoningEfforts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|effort| effort["reasoningEffort"].as_str().map(str::to_string))
                    .collect(),
            })
        })
        .collect()
}

fn input(text: &str) -> Value {
    json!([{ "type": "text", "text": text }])
}

async fn drive(
    job: &Job<'_>,
    mut controls: mpsc::Receiver<Control>,
    signals: mpsc::Sender<Signal>,
    mut stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    watching: &mut (dyn FnMut(Progress) + Send),
) -> Result<Outcome, CodingError> {
    let gate = job.gate;
    let directory = job.directory;
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
    let mut stopping = false;
    let mut next_id = 5u64;
    let mut pending: Option<(u64, oneshot::Sender<Result<(), String>>)> = None;
    let mut deadline = tokio::time::Instant::now() + RESPONSE_LIMIT;
    let mut approvals: FuturesUnordered<BoxFuture<'static, Value>> = FuturesUnordered::new();
    let mut items = HashSet::new();
    // Corrections that arrived before the turn could take one, or while the
    // last was still being acknowledged. Held rather than refused: a
    // correction typed a second after `code` returned is the ordinary case.
    let mut queued: std::collections::VecDeque<(String, oneshot::Sender<Result<(), String>>)> =
        std::collections::VecDeque::new();

    loop {
        if complete && pending.is_none() {
            for (_, reply) in queued.drain(..) {
                let _ = reply.send(Err("The job finished before this correction was sent. Send it again to continue the session.".into()));
            }
            return Ok(outcome);
        }
        if ready && pending.is_none() && !stopping && !complete {
            if let Some((message, reply)) = queued.pop_front() {
                if !reply.is_closed() {
                    let id = next_id;
                    next_id += 1;
                    write(
                        &mut stdin,
                        json!({"id":id,"method":"turn/steer","params":{
                            "threadId":thread,"expectedTurnId":turn,"input":input(&message)
                        }}),
                    )
                    .await?;
                    pending = Some((id, reply));
                    deadline = tokio::time::Instant::now() + RESPONSE_LIMIT;
                }
                continue;
            }
        }
        tokio::select! {
            _ = tokio::time::sleep_until(deadline), if !ready || pending.is_some() || stopping => {
                if let Some((_, reply)) = pending.take() {
                    let _ = reply.send(Err("Codex did not acknowledge the correction. The job was stopped; review its changes before retrying.".into()));
                }
                if stopping {
                    // Asked to end its turn and did not within the grace. The
                    // process is killed by the caller; what it wrote stays.
                    outcome.stopped = true;
                    return Ok(outcome);
                }
                return Err(failed("Codex did not answer a control request within 30 seconds"));
            }
            Some(answer) = approvals.next(), if !approvals.is_empty() => {
                write(&mut stdin, answer).await?;
            }
            // A closed channel matches nothing, which disables this branch for
            // the rest of the select rather than spinning on `None`.
            Some(control) = controls.recv(), if !complete => {
                match control {
                    Control::Steer { message, reply } => {
                        if reply.is_closed() { continue; }
                        if message.trim().is_empty() {
                            let _ = reply.send(Err("Enter a correction to send".into()));
                            continue;
                        }
                        if stopping {
                            let _ = reply.send(Err("The job is stopping.".into()));
                            continue;
                        }
                        queued.push_back((message, reply));
                    }
                    Control::Stop => {
                        outcome.stopped = true;
                        if !ready {
                            // Nothing has started yet, so there is no turn to
                            // interrupt: ending here is ending before any work.
                            return Ok(outcome);
                        }
                        if !stopping {
                            let id = next_id;
                            next_id += 1;
                            write(&mut stdin, json!({"id":id,"method":"turn/interrupt","params":{
                                "threadId":thread,"turnId":turn
                            }})).await?;
                            stopping = true;
                            deadline = tokio::time::Instant::now() + super::STOP_GRACE;
                        }
                    }
                }
            }
            line = lines.next_line() => {
                let Some(line) = line.map_err(|_| failed("Could not read Codex output"))? else {
                    if stopping {
                        return Ok(outcome);
                    }
                    return Err(failed("Codex exited before finishing the job"));
                };
                let Ok(event) = serde_json::from_str::<Value>(&line) else { continue; };
                if let Some(method) = event["method"].as_str() {
                    let params = &event["params"];
                    if event.get("id").is_some() {
                        approvals.push(answer_request(event.clone(), gate, directory.into(), thread.clone(), turn.clone(), signals.clone()).boxed());
                        continue;
                    }
                    if params["threadId"].as_str() != Some(thread.as_str()) { continue; }
                    match method {
                        "turn/started" if turn.is_empty() => {
                            turn = params["turn"]["id"].as_str().unwrap_or_default().into();
                        }
                        "item/started" | "item/completed" if params["turnId"].as_str() == Some(turn.as_str()) => {
                            absorb(&mut outcome, &params["item"], method == "item/completed", &mut items, watching, directory);
                        }
                        "turn/completed" if params["turn"]["id"].as_str() == Some(turn.as_str()) => {
                            complete = true;
                            while let Ok(control) = controls.try_recv() {
                                if let Control::Steer { reply, .. } = control {
                                    queued.push_back((String::new(), reply));
                                }
                            }
                            match params["turn"]["status"].as_str() {
                                Some("completed") => {}
                                // Asked for, so not a failure: the turn ended where
                                // the operator stopped it and the thread is kept.
                                Some("interrupted") if stopping => outcome.stopped = true,
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
                if id >= 5 {
                    // An interrupt's acknowledgment. What matters is the
                    // `turn/completed` that follows it.
                    continue;
                }
                if let Some(error) = event.get("error") {
                    return Err(failed(error["message"].as_str().unwrap_or("Codex refused to start the job")));
                }
                let result = &event["result"];
                match id {
                    1 => {
                        write(&mut stdin, json!({"method":"initialized"})).await?;
                        write(&mut stdin, json!({"id":4,"method":"account/read","params":{"refreshToken":false}})).await?;
                        deadline = tokio::time::Instant::now() + RESPONSE_LIMIT;
                    }
                    4 => {
                        // Ask the process that will run the job. A separate CLI
                        // login check can disagree with a custom provider.
                        if result["requiresOpenaiAuth"] == true && result["account"].is_null() {
                            return Err(failed(format!(
                                "Codex is not signed in on this backend. Run `{}` on it, from Open terminal in this agent's Terminal panel, then retry the coding job. Guaca's own ChatGPT sign-in does not sign in Codex.",
                                super::sign_in(crate::domain::terminal::Harness::Codex)
                            )));
                        }
                        let mut policy = json!({
                            "cwd":directory,
                            "approvalPolicy": if gate == Gate::AskBeforePushing { "untrusted" } else { "never" },
                            "approvalsReviewer":"user", "sandbox":"danger-full-access",
                            "developerInstructions":super::APPENDED_PROMPT,
                        });
                        // The operator's choice for this agent, as the app-server's
                        // own parameter. Absent is Codex's configured model.
                        if let Some(model) = &job.tuning.model {
                            policy["model"] = json!(model);
                        }
                        // A follow-up carries the thread on, with everything it
                        // had already read; a new job starts one.
                        let request = if job.resume && !job.session.is_empty() {
                            let mut params = policy;
                            params["threadId"] = json!(job.session);
                            json!({"id":2,"method":"thread/resume","params":params})
                        } else {
                            let mut params = policy;
                            params["serviceName"] = json!("guaca");
                            json!({"id":2,"method":"thread/start","params":params})
                        };
                        write(&mut stdin, request).await?;
                        deadline = tokio::time::Instant::now() + RESPONSE_LIMIT;
                    }
                    2 => {
                        thread = result["thread"]["id"].as_str().filter(|id| !id.is_empty()).ok_or_else(|| failed("Codex returned no thread id"))?.into();
                        if gate == Gate::AskBeforePushing && (result["approvalPolicy"] != "untrusted" || result["approvalsReviewer"] != "user") {
                            return Err(failed("Codex did not enable Guaca's approval policy; the coding job was not started"));
                        }
                        outcome.session_id = thread.clone();
                        let _ = signals.send(Signal::Session(thread.clone())).await;
                        outcome.model = result["model"].as_str().unwrap_or_default().into();
                        let mut params = json!({"threadId":thread, "input":input(job.brief)});
                        if let Some(effort) = &job.tuning.effort {
                            params["effort"] = json!(effort);
                        }
                        write(&mut stdin, json!({"id":3,"method":"turn/start","params":params})).await?;
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
/// agent's gate, through the same signal and decision as Claude's hooks.
async fn answer_request(
    event: Value,
    gate: Gate,
    directory: String,
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
                let cwd = params["cwd"].as_str().unwrap_or(&directory);
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
    directory: &str,
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
        watching(Progress::Using { tool: tool.into(), detail: super::shown(directory, detail) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_list_is_the_pickers_with_each_models_own_efforts() {
        // Cut from 0.153.3's `model/list` answer.
        let answer = json!({"id":2,"result":{"nextCursor":null,"data":[
            {"id":"gpt-6-astra","displayName":"GPT-6-Astra","description":"Frontier","isDefault":true,"hidden":false,
             "supportedReasoningEfforts":[{"reasoningEffort":"low"},{"reasoningEffort":"ultra"}]},
            {"id":"gpt-5.5","displayName":"GPT-5.5","isDefault":false,"hidden":false,
             "supportedReasoningEfforts":[{"reasoningEffort":"low"},{"reasoningEffort":"xhigh"}]},
            {"id":"internal","displayName":"Internal","hidden":true}
        ]}});
        assert!(listed(&answer));
        let offers = offers(&answer);
        assert_eq!(offers.len(), 2, "hidden models are not offered");
        assert!(offers[0].default);
        assert_eq!(offers[0].efforts, ["low", "ultra"]);
        assert_eq!(offers[1].efforts, ["low", "xhigh"], "a model's own list, not the union");
    }
}
