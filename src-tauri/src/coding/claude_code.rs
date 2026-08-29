//! Claude Code, as this app starts it and reads it.
//!
//! The same two functions [`super::pi`] has, against a different program with a
//! different stream. What makes it worth a second parser rather than a flag on
//! the first is in [`crate::domain::repository::Harness`]: a Claude
//! subscription is spent by this program and by nothing else holding its
//! credential.

use super::{first_line, Outcome, Progress, Wiring};

pub(super) const BINARY: &str = "claude";

pub(super) const INSTALL: &str = "npm install -g @anthropic-ai/claude-code";

/// `--output-format stream-json`, which is the only mode that says what the run
/// is *doing* rather than what it concluded, and which the CLI refuses without
/// `--verbose`.
///
/// `--permission-mode bypassPermissions` because there is nobody to ask. The job
/// is started by an agent, runs unattended for many minutes, and reads its
/// stdin from `/dev/null`: a prompt on this path is not a safety control, it is
/// a process that hangs until the ceiling kills it and reports nothing. What
/// makes that acceptable is stated in [`super`] and is not this flag: the
/// operator chose the directory, git is the undo, and nothing here is a
/// sandbox.
///
/// No `--model`. Which model runs is Claude Code's own setting, and a second
/// place to say it is a second place for it to be wrong. No `--continue`
/// either: each job is its own session.
///
/// # What the bridge adds, and what it deliberately does not
///
/// With a [`Wiring`], three more flags, and the two that are absent from it
/// matter as much as the three that are there.
///
/// - **`--session-id`** names the session rather than reading it back. One
///   value is then the job's address on the bridge, the key of its mailbox, and
///   what an operator hands to `claude --resume` to open this job in their own
///   terminal. That last one is the whole reason it is chosen: `claude -c`
///   resumes whatever ran last in the directory, which after two jobs is the
///   wrong one.
/// - **`--settings`** carries this job's hooks, which are what make it
///   reachable while it runs. Additive: it loads *alongside* the operator's own
///   settings files rather than replacing them.
/// - **`--mcp-config`** adds Guaca's own two-tool server, and is passed
///   **without `--strict-mcp-config`** for the same reason.
///
/// That is the opposite choice from [`crate::llm::claude`], which switches
/// every one of those off, and the two are right for opposite reasons. A turn
/// there is answered by a program that should have this app's tools and nothing
/// else. A job here is a coding agent working in the operator's own repository,
/// where their rules file, their project settings and their MCP servers are the
/// thing that makes it useful. `docs/CODING.md` has the measurement of what
/// that inheritance costs and the one hazard in it.
///
/// No `--setting-sources` either, for the same reason: naming sources here
/// would be this app deciding which of the operator's own files count in their
/// own repository.
pub(super) fn argv(task: &str, wiring: Option<&Wiring>) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        task,
        "--output-format",
        "stream-json",
        "--verbose",
        "--permission-mode",
        "bypassPermissions",
        "--append-system-prompt",
        super::APPENDED_PROMPT,
    ]
    .iter()
    .map(|arg| arg.to_string())
    .collect();

    if let Some(wiring) = wiring {
        args.push("--session-id".to_string());
        args.push(wiring.session_id.clone());
        args.push("--settings".to_string());
        args.push(wiring.settings.to_string_lossy().into_owned());
        args.push("--mcp-config".to_string());
        args.push(wiring.mcp_config.clone());
    }
    args
}

/// Folds one event from Claude Code's stream into the outcome.
///
/// Its own function so the tests drive the real thing, for the reason written
/// out in [`super::pi::absorb`]: a test holding its own copy of the match is a
/// test that asserts the copy.
pub(super) fn absorb(
    outcome: &mut Outcome,
    event: &serde_json::Value,
    watching: &mut dyn FnMut(Progress),
) {
    match event["type"].as_str().unwrap_or_default() {
        // The first event of a run, and the only one that names the model
        // before anything has been spent. A job that dies on its first call
        // still reports what it was going to run as.
        "system" if event["subtype"] == "init" => {
            if let Some(model) = event["model"].as_str() {
                outcome.model = model.to_string();
            }
        }
        "assistant" => {
            for part in event["message"]["content"].as_array().into_iter().flatten() {
                match part["type"].as_str().unwrap_or_default() {
                    "tool_use" => {
                        outcome.tool_calls += 1;
                        if let Some(name) = part["name"].as_str() {
                            watching(Progress::Using {
                                tool: name.to_string(),
                                detail: detail_of(name, &part["input"]),
                            });
                        }
                    }
                    "text" => {
                        let said = part["text"].as_str().unwrap_or_default().to_string();
                        if !said.trim().is_empty() {
                            watching(Progress::Said(said.clone()));
                            // Last one wins, as in pi. This is the fallback: the
                            // `result` event below is the authoritative answer,
                            // and this is what a run that was killed at the
                            // ceiling has to report instead of nothing.
                            outcome.said = said;
                        }
                    }
                    // `thinking` is deliberately not one of these. A turn's
                    // thinking is shown and never kept, and this stream reaches
                    // a channel that is a record.
                    _ => {}
                }
            }
            if let Some(model) = event["message"]["model"].as_str() {
                outcome.model = model.to_string();
            }
        }
        // The last event of a run, and the whole of the answer. `result` carries
        // the final assistant text on a success and the reason on a failure, so
        // which of the two it is decides where it goes: an errored run carries
        // no answer, exactly as pi's does.
        "result" => {
            let text = event["result"].as_str().unwrap_or_default().trim().to_string();
            if event["is_error"] == serde_json::Value::Bool(true) {
                // The subtype is the machine-readable half (`error_max_turns`,
                // `error_during_execution`) and is worth carrying: it is the
                // difference between a job that ran out of room and a job whose
                // credential is spent, and the two have different fixes.
                let subtype = event["subtype"].as_str().unwrap_or("error");
                outcome.failed = Some(match text.is_empty() {
                    true => format!("the harness did not say why ({subtype})"),
                    false => format!("{text} ({subtype})"),
                });
                outcome.said = String::new();
            } else {
                if !text.is_empty() {
                    outcome.said = text;
                }
                // Cleared rather than left. A run that failed a turn and then
                // finished has done the work, which is the rule pi's parser has
                // for the same reason.
                outcome.failed = None;
            }
            // What Claude Code says the job cost, which on a subscription is the
            // equivalent API price rather than money that moved. Reported as it
            // stands, because pi prices a subscription-funded job the same way
            // and neither number claims to be more than what the harness said.
            // Zero is absent, not free: `Outcome::cost` is the argument.
            if let Some(total) = event["total_cost_usd"].as_f64() {
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
/// Claude Code's built-ins, which are capitalized and are not pi's: `Bash`
/// against `bash`, `file_path` against `path`. One table per harness rather
/// than a merged one, because a merged table is a place for one program's
/// field name to be read out of the other's arguments.
///
/// Anything else falls back to nothing rather than guessing a field, which
/// covers every MCP tool the operator has connected: a wrong guess here prints
/// somebody's data into a channel.
fn detail_of(tool: &str, input: &serde_json::Value) -> String {
    let pick = match tool {
        "Bash" | "BashOutput" => "command",
        "Read" | "Write" | "Edit" | "NotebookEdit" => "file_path",
        "Grep" | "Glob" => "pattern",
        "WebFetch" => "url",
        "WebSearch" => "query",
        "Task" => "description",
        _ => return String::new(),
    };
    first_line(input[pick].as_str().unwrap_or_default()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives the real parser, not a copy of it.
    fn drive(lines: &[&str]) -> (Outcome, Vec<Progress>) {
        let mut outcome = Outcome::default();
        let mut seen = Vec::new();
        for line in lines {
            let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            absorb(&mut outcome, &event, &mut |p| seen.push(p));
        }
        (outcome, seen)
    }

    /// A whole successful run, captured from the real CLI: init, a thinking
    /// block, a tool call, the answer, the result.
    const A_RUN: &[&str] = &[
        r#"{"type":"system","subtype":"init","model":"claude-opus-5","permissionMode":"bypassPermissions","cwd":"/repo"}"#,
        r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-5","content":[{"type":"thinking","thinking":"the operator must never read this"}]}}"#,
        r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-5","content":[{"type":"text","text":"Looking at the tests first."},{"type":"tool_use","name":"Bash","input":{"command":"npm test"}}]}}"#,
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"1 failing"}]}}"#,
        r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-5","content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/repo/src/api.ts","old_string":"a","new_string":"b"}}]}}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"Fixed and pushed.","total_cost_usd":0.04,"num_turns":3}"#,
    ];

    #[test]
    fn a_finished_run_reports_the_result_the_tools_and_the_model() {
        let (outcome, _) = drive(A_RUN);
        assert_eq!(outcome.said, "Fixed and pushed.");
        assert_eq!(outcome.tool_calls, 2);
        assert_eq!(outcome.model, "claude-opus-5");
        assert_eq!(outcome.cost, Some(0.04));
        assert_eq!(outcome.failed, None);
    }

    #[test]
    fn the_watcher_gets_the_tools_and_the_narration_and_nothing_else() {
        let (_, seen) = drive(A_RUN);
        assert_eq!(
            seen,
            vec![
                Progress::Said("Looking at the tests first.".into()),
                Progress::Using { tool: "Bash".into(), detail: "npm test".into() },
                Progress::Using { tool: "Edit".into(), detail: "/repo/src/api.ts".into() },
            ]
        );
    }

    #[test]
    fn thinking_never_reaches_the_watcher() {
        // A turn's thinking is shown and never kept, and this stream reaches a
        // panel in a channel. The run above has a thinking block in it, and the
        // assertion is that nothing carrying its text came out.
        let (outcome, seen) = drive(A_RUN);
        assert!(!outcome.said.contains("never read this"));
        for line in &seen {
            let drawn = match line {
                Progress::Said(text) => text.clone(),
                Progress::Using { tool, detail } => format!("{tool} {detail}"),
            };
            assert!(!drawn.contains("never read this"), "{drawn}");
        }
    }

    #[test]
    fn the_result_beats_the_narration_it_followed() {
        // Every text block is a round of one turn. Whichever is last would be
        // the narration before the final tool call if `result` were ignored.
        let (outcome, _) = drive(&[
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Let me check the build."}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"npm run build"}}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"Build is green."}"#,
        ]);
        assert_eq!(outcome.said, "Build is green.");
    }

    #[test]
    fn a_run_killed_before_its_result_still_reports_what_it_last_said() {
        // The ceiling, or a killed process. Without the fallback the agent that
        // asked is told a job that ran for forty-five minutes said nothing.
        let (outcome, _) = drive(&[
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Halfway through the migration."}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/repo/m.sql"}}]}}"#,
        ]);
        assert_eq!(outcome.said, "Halfway through the migration.");
        assert_eq!(outcome.tool_calls, 1);
    }

    #[test]
    fn a_failed_run_is_a_failure_and_not_an_empty_job() {
        // The same afternoon pi's parser cost. Claude Code reports a failure in
        // its own stream, and a run that is refused before its first tool call
        // is otherwise indistinguishable from a job with nothing to do.
        let (outcome, _) = drive(&[
            r#"{"type":"system","subtype":"init","model":"claude-opus-5"}"#,
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"You're out of extra usage. Add more at claude.ai/settings/usage and keep going."}"#,
        ]);
        let why = outcome.failed.expect("a refused run has to say so");
        assert!(why.contains("out of extra usage"), "{why}");
        // The subtype is the difference between running out of room and running
        // out of quota, and the two have different fixes.
        assert!(why.contains("error_during_execution"), "{why}");
        assert!(outcome.said.is_empty(), "an errored run carries no answer");
        assert_eq!(outcome.model, "claude-opus-5", "it still says what it would have run as");
    }

    #[test]
    fn a_failure_that_says_nothing_still_says_something() {
        let (outcome, _) =
            drive(&[r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#]);
        assert_eq!(
            outcome.failed.as_deref(),
            Some("the harness did not say why (error_max_turns)")
        );
    }

    #[test]
    fn a_priceless_run_reports_nothing_rather_than_zero() {
        // Absent is not free. A zero here is added into a total the operator
        // reads as what the work cost.
        let (outcome, _) = drive(&[
            r#"{"type":"result","subtype":"success","is_error":false,"result":"Done.","total_cost_usd":0}"#,
        ]);
        assert_eq!(outcome.cost, None);
    }

    #[test]
    fn a_tool_result_is_not_mistaken_for_the_answer() {
        // Tool results come back as `user` messages in this stream, and the one
        // in the run above is a failing test suite.
        let (outcome, _) = drive(&[
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"1 failing"}]}}"#,
        ]);
        assert_eq!(outcome.said, "");
        assert_eq!(outcome.tool_calls, 0);
    }

    #[test]
    fn a_line_that_is_not_json_does_not_end_the_stream() {
        let (outcome, _) = drive(&[
            "npm warn something",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"a"}}]}}"#,
        ]);
        assert_eq!(outcome.tool_calls, 1);
    }

    #[test]
    fn the_brief_is_an_argument_and_the_stream_is_asked_for() {
        let args = argv("fix the flaky test", None);
        assert!(args.contains(&"fix the flaky test".to_string()));
        assert!(args.contains(&"stream-json".to_string()));
        // The CLI refuses stream-json without it, and the refusal is a job that
        // never starts.
        assert!(args.contains(&"--verbose".to_string()));
        // Nobody is there to answer a prompt: stdin is /dev/null and the asking
        // mode is a process that hangs until the ceiling kills it. The gate is
        // a hook, which overrides this per command rather than replacing it.
        assert!(args.contains(&"bypassPermissions".to_string()));
        assert!(args.contains(&super::super::APPENDED_PROMPT.to_string()));
        // Which model runs is Claude Code's own setting.
        assert!(!args.contains(&"--model".to_string()));
        assert!(!args.contains(&"--continue".to_string()));
    }

    #[test]
    fn a_job_without_a_bridge_is_the_job_this_app_always_ran() {
        // `pi`, an older Claude Code, or a bridge that could not start. Every
        // one of them has to be the vector that worked before any of this.
        let bare = argv("do the thing", None);
        for flag in ["--session-id", "--settings", "--mcp-config"] {
            assert!(!bare.contains(&flag.to_string()), "{flag}");
        }
    }

    #[test]
    fn a_bridged_job_is_named_and_hooked_and_keeps_the_operators_own_setup() {
        let wiring = Wiring {
            session_id: "f96abc0a-5404-4e51-b465-b96677118cf9".into(),
            settings: std::path::PathBuf::from("/tmp/guaca-job-x/settings.json"),
            mcp_config: r#"{"mcpServers":{"guaca":{}}}"#.into(),
        };
        let args = argv("do the thing", Some(&wiring));

        let after =
            |flag: &str| args.iter().position(|arg| arg == flag).map(|at| args[at + 1].clone());
        assert_eq!(after("--session-id").as_deref(), Some(wiring.session_id.as_str()));
        assert_eq!(after("--settings").as_deref(), Some("/tmp/guaca-job-x/settings.json"));
        assert_eq!(after("--mcp-config").as_deref(), Some(wiring.mcp_config.as_str()));

        // The two absences are the point. A coding job wants the operator's own
        // rules file, project settings and MCP servers: this adds to them and
        // must never replace them, which is the opposite of what `llm/claude.rs`
        // does and right for the opposite reason.
        assert!(!args.contains(&"--strict-mcp-config".to_string()));
        assert!(!args.contains(&"--setting-sources".to_string()));
    }

    #[test]
    fn the_tool_table_is_this_harnesss_own_and_not_the_other_ones() {
        // Claude Code's built-ins are capitalized and carry `file_path`; pi's
        // are lowercase and carry `path`. Read out of the wrong table, every
        // line in the panel is blank.
        assert_eq!(detail_of("Bash", &serde_json::json!({"command": "npm test"})), "npm test");
        assert_eq!(detail_of("Read", &serde_json::json!({"file_path": "src/a.ts"})), "src/a.ts");
        assert_eq!(detail_of("read", &serde_json::json!({"path": "src/a.ts"})), "");
        // Every MCP tool the operator has connected lands here, and a guessed
        // field would print its arguments into a channel.
        assert_eq!(detail_of("mcp__linear__create_issue", &serde_json::json!({"title": "x"})), "");
    }
}
