//! `pi`, as this app starts it and reads it.
//!
//! Two functions: the argument vector, and the fold from one of its events into
//! an [`Outcome`]. The process itself is [`super::run`]'s.

use super::{first_line, Outcome, Progress};

pub(super) const BINARY: &str = "pi";

pub(super) const INSTALL: &str = "npm install -g --ignore-scripts @earendil-works/pi-coding-agent";

/// `--mode json` rather than `--mode rpc`. RPC is the richer protocol and buys
/// steering and a clean abort mid-run; a job that is started, watched and killed
/// needs neither yet, and JSON mode is one prompt, one stream and an exit. When
/// steering arrives, this is the function that changes.
///
/// `--no-session` is deliberately *not* passed. A session on disk is what lets
/// the operator open the same work in their own terminal with `pi -c`.
///
/// No `--provider` and no `--model`. Which sign-in pays and which model runs are
/// pi's own settings, and a second place to say either is a second place for it
/// to be wrong. Choosing between *programs* is what the repository's harness
/// column is for, and it is a different question: see
/// [`crate::domain::repository::Harness`].
pub(super) fn argv(task: &str) -> Vec<String> {
    ["--mode", "json", "--append-system-prompt", super::APPENDED_PROMPT, "-p", task]
        .iter()
        .map(|arg| arg.to_string())
        .collect()
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
    fn the_brief_is_the_last_argument_and_the_prompt_is_appended() {
        let args = argv("fix the flaky test");
        assert_eq!(args.last().unwrap(), "fix the flaky test");
        assert!(args.contains(&"--mode".to_string()) && args.contains(&"json".to_string()));
        assert!(args.contains(&super::super::APPENDED_PROMPT.to_string()));
        // A session on disk is what lets the operator pick the work up in their
        // own terminal, which is the difference between this and a black box.
        assert!(!args.contains(&"--no-session".to_string()));
        // Which sign-in pays is pi's own setting. Choosing between programs is
        // the repository's harness, and it is not spelled as a provider flag.
        assert!(!args.contains(&"--provider".to_string()));
    }

    #[test]
    fn a_tool_this_build_does_not_know_prints_nothing_rather_than_a_guess() {
        // A wrong guess here puts somebody's file contents into a channel.
        assert_eq!(detail_of("mcp__whatever", &serde_json::json!({"secret": "hunter2"})), "");
        assert_eq!(detail_of("bash", &serde_json::json!({"command": "npm test"})), "npm test");
        assert_eq!(detail_of("read", &serde_json::json!({"path": "src/api.go"})), "src/api.go");
    }
}
