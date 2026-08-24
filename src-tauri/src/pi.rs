//! Running a coding harness against a linked repository.
//!
//! Guaca does not write code. It starts something that does, in a directory the
//! operator linked, and reads what comes back. The harness is `pi`, which the
//! operator installs and signs in to themselves.
//!
//! ## Why a harness and not tools
//!
//! A turn and a coding task are different units of work, and that is the whole
//! argument. A guaca turn is one model call plus `max_tool_rounds` rounds,
//! twenty-four by default, inside a conversation bounded at sixty model calls.
//! A real change to a repository is a few hundred tool calls, its own context
//! window and its own compaction. Reaching that with `read` and `edit` tools in
//! this runtime means raising both limits to coding scale, and both are per
//! group, so the guard that keeps a crew of eight from talking forever comes
//! off for every agent in every crew.
//!
//! So the harness keeps its own loop, its own context and its own budget, and
//! Guaca spends one tool round starting it.
//!
//! ## Why the credentials are not ours
//!
//! `pi` reads its own auth from `~/.pi/agent/auth.json` or the environment. It
//! is already signed in or it is not, and Guaca passing a key would put the
//! operator's Guaca key on a second bill under a second provider for work they
//! are already paying for. The consequence is stated rather than hidden: a
//! job's spend does not appear in this app's usage table, because this app did
//! not spend it. What the job reports back is what `pi` says it cost.
//!
//! ## What is not here
//!
//! Any confinement. The process runs as the operator, in their repository, with
//! their credentials and their network, and it may commit, push and open pull
//! requests. That was asked for explicitly. `pi` has no permission system of
//! its own and says so in its own documentation, so the boundary is the
//! directory the operator chose and the fact that git can undo what happens
//! inside it. Nothing in this file should ever be described as a sandbox.

use tokio::io::{AsyncBufReadExt, BufReader};

/// The binary, by name. Found on `PATH` rather than configured: an operator who
/// has `pi` has it on their path, and a second place to say where it lives is a
/// second place for that to be wrong.
const BINARY: &str = "pi";

/// How long a job may run before it is killed.
///
/// Generous, because the unit of work is a change to a repository rather than a
/// reply. What it must not be is unbounded: a harness waiting on a prompt
/// nobody will answer holds a process and a tokio task for the life of the app.
const CEILING: std::time::Duration = std::time::Duration::from_secs(45 * 60);

#[derive(Debug, thiserror::Error)]
pub enum PiError {
    #[error(
        "the `pi` coding harness is not installed, or is not on this app's PATH. Install it with \
         `npm install -g --ignore-scripts @earendil-works/pi-coding-agent`, then try again"
    )]
    NotInstalled,
    #[error("the coding harness could not be started: {0}")]
    Start(String),
    #[error(
        "the coding harness exited without answering ({0}). Its own output is above; run `pi` in \
         this repository yourself to see what it says"
    )]
    NoAnswer(String),
    #[error("the job ran for {0} minutes without finishing and was stopped")]
    TooLong(u64),
}

/// What one job did, as the agent that started it is told.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// The harness's own last word. What it says it did.
    pub said: String,
    /// How many tools it ran. The one number that says whether it worked in the
    /// repository or just talked about it.
    pub tool_calls: u32,
    /// What the harness reports the job cost, on the operator's own `pi`
    /// credentials. `None` when the provider does not price calls, which is not
    /// the same as free and must not be added up as zero.
    pub cost: Option<f64>,
    /// The model the harness chose. Not Guaca's to pick: `pi` resolves it from
    /// its own settings and its own sign-ins.
    pub model: String,
    /// What the harness said went wrong, when it ended a turn on an error.
    ///
    /// `pi` reports a failed turn *inside* its stream and still exits zero: the
    /// final assistant message carries `stopReason: "error"` and an
    /// `errorMessage`, with empty content. Read by exit code and text alone,
    /// that is indistinguishable from a job with nothing to do.
    ///
    /// It cost an afternoon to find. An expired Codex token turned every coding
    /// job in a live workspace into a silent no-op, the agents reported that
    /// nothing needed doing, and `pi auth check` called the provider ready
    /// throughout.
    pub failed: Option<String>,
}

/// One line of progress, for whoever is watching the job run.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// A tool the harness started, by name.
    Using(String),
    /// Something the harness said on its way through.
    Said(String),
}

/// Whether the harness is installed at all.
///
/// Asked before a job is started rather than discovered by failing to spawn
/// one, so the refusal an agent reads names the install command instead of an
/// operating system error.
pub async fn installed() -> bool {
    tokio::process::Command::new(BINARY)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|code| code.success())
        .unwrap_or(false)
}

/// Runs one task to completion in one repository.
///
/// `--mode json` rather than `--mode rpc`. RPC is the richer protocol and buys
/// steering and a clean abort mid-run; a job that is started, watched and
/// killed needs neither yet, and JSON mode is one prompt, one stream and an
/// exit. When steering arrives, this is the function that changes.
///
/// `--no-session` is deliberately *not* passed. A session on disk is what lets
/// the operator open the same work in their own terminal with `pi -c`, which is
/// the difference between a harness the app runs and a black box.
pub async fn run(
    repository: &str,
    task: &str,
    mut watching: impl FnMut(Progress),
) -> Result<Outcome, PiError> {
    let mut child = tokio::process::Command::new(BINARY)
        .current_dir(repository)
        .args(["--mode", "json", "-p", task])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // Killed with this handle rather than left behind. A job whose task is
        // dropped mid-await otherwise leaves a coding agent running in somebody
        // else's repository with nothing holding a reference to it.
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => PiError::NotInstalled,
            _ => PiError::Start(err.to_string()),
        })?;

    let stdout = child.stdout.take().ok_or_else(|| PiError::Start("no output".into()))?;
    let stderr = child.stderr.take();
    let mut lines = BufReader::new(stdout).lines();
    let mut outcome = Outcome::default();

    let reading = async {
        // Split on `\n` and nothing else: pi's own protocol note, and the
        // reason is that `U+2028` and `U+2029` are legal inside JSON strings.
        // Rust's `lines` is compliant where several line readers are not.
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            match event["type"].as_str().unwrap_or_default() {
                "tool_execution_start" => {
                    outcome.tool_calls += 1;
                    if let Some(name) = event["toolName"].as_str() {
                        watching(Progress::Using(name.to_string()));
                    }
                }
                // The authoritative message, as opposed to the deltas: pi's
                // own documentation says `message_end` is the final one, and
                // reassembling the text from `text_delta` would be a second
                // copy of the same string that could disagree with it.
                "message_end" if event["message"]["role"] == "assistant" => {
                    let said = text_of(&event["message"]);
                    if !said.trim().is_empty() {
                        watching(Progress::Said(said.clone()));
                        // Kept rather than appended. Every assistant message is
                        // a round of one turn, and the last one is the answer;
                        // joined, a job that narrated its work would report the
                        // narration as its result.
                        outcome.said = said;
                    }
                    if let Some(model) = event["message"]["model"].as_str() {
                        outcome.model = model.to_string();
                    }
                }
                "message_update" => {
                    // Cumulative rather than additive: pi reports the running
                    // total on every update, so adding them up multiplies the
                    // bill by the number of updates.
                    if let Some(total) = event["usage"]["cost"]["total"].as_f64() {
                        if total > 0.0 {
                            outcome.cost = Some(total);
                        }
                    }
                }
                _ => {}
            }
        }
    };

    // Stopping is the caller aborting the task this runs in, which drops the
    // child, which `kill_on_drop` turns into a killed process. A cancellation
    // token beside that would be a second way to stop one thing, and the two
    // would have to agree about which had happened.
    tokio::select! {
        _ = reading => {}
        _ = tokio::time::sleep(CEILING) => {
            let _ = child.kill().await;
            return Err(PiError::TooLong(CEILING.as_secs() / 60));
        }
    }

    let status = child.wait().await.map_err(|err| PiError::Start(err.to_string()))?;
    if !status.success() && outcome.said.trim().is_empty() {
        // Only when there is nothing to report. A harness that answered and
        // then exited non-zero has still done the work, and throwing its answer
        // away over the exit code is how an agent reports a finished change as
        // a failure.
        let mut why = format!("exit {}", status.code().unwrap_or(-1));
        if let Some(stderr) = stderr {
            let mut text = String::new();
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                text.push_str(&line);
                text.push('\n');
                if text.len() > 2_000 {
                    break;
                }
            }
            if !text.trim().is_empty() {
                why = format!("{why}: {}", text.trim());
            }
        }
        return Err(PiError::NoAnswer(why));
    }

    Ok(outcome)
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

    fn drive(lines: &[&str]) -> Outcome {
        // The parser, without a process. What this covers is the shape of pi's
        // stream, which is the thing that goes stale when pi ships.
        let mut outcome = Outcome::default();
        for line in lines {
            let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            match event["type"].as_str().unwrap_or_default() {
                "tool_execution_start" => outcome.tool_calls += 1,
                "message_end" if event["message"]["role"] == "assistant" => {
                    let said = text_of(&event["message"]);
                    if !said.trim().is_empty() {
                        outcome.said = said;
                    }
                    if let Some(model) = event["message"]["model"].as_str() {
                        outcome.model = model.to_string();
                    }
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
                    if let Some(total) = event["usage"]["cost"]["total"].as_f64() {
                        if total > 0.0 {
                            outcome.cost = Some(total);
                        }
                    }
                }
                _ => {}
            }
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
    fn every_refusal_says_what_to_do_about_it() {
        // Read by a model mid-turn and by a person under pressure. A refusal
        // that only says no gets reworded and retried.
        let missing = PiError::NotInstalled.to_string();
        assert!(missing.contains("npm install"), "{missing}");
        assert!(PiError::TooLong(45).to_string().contains("45 minutes"));
    }
}
