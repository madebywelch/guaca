//! The official Codex CLI owns its authentication, model and repository tools.
//! Guaca starts one job and reads its JSONL events; it never spends a ChatGPT
//! credential through a different coding program.

use super::{Outcome, Progress};

pub(super) const BINARY: &str = "codex";
pub(super) const INSTALL: &str = "npm install -g @openai/codex";
pub(super) const INCOMPLETE: &str = "Codex exited before reporting a completed turn";

pub(super) fn argv(task: &str) -> Vec<String> {
    vec![
        "exec".into(),
        "--json".into(),
        "--color".into(),
        "never".into(),
        // Same operator-granted reach as the other coding harnesses. This is
        // not a sandbox, and a repository requiring the push gate cannot run
        // this adapter. The runtime enforces that before starting the job.
        "--dangerously-bypass-approvals-and-sandbox".into(),
        "--".into(),
        format!("{}\n\n{task}", super::APPENDED_PROMPT),
    ]
}

pub(super) fn absorb(
    outcome: &mut Outcome,
    event: &serde_json::Value,
    watching: &mut dyn FnMut(Progress),
) {
    match event["type"].as_str().unwrap_or_default() {
        "thread.started" => {
            outcome.session_id = event["thread_id"].as_str().unwrap_or_default().into();
        }
        "turn.started" => outcome.failed = Some(INCOMPLETE.into()),
        "turn.completed" => outcome.failed = None,
        "turn.failed" | "error" => {
            outcome.failed = Some(
                event["error"]["message"]
                    .as_str()
                    .or_else(|| event["message"].as_str())
                    .unwrap_or("Codex reported an error without a reason")
                    .into(),
            );
        }
        "item.completed" => {
            let item = &event["item"];
            match item["type"].as_str().unwrap_or_default() {
                "agent_message" => {
                    let text = item["text"].as_str().unwrap_or_default();
                    if !text.trim().is_empty() {
                        outcome.said = text.into();
                        watching(Progress::Said(text.into()));
                    }
                }
                // Completion is the common event for all these item types;
                // counting starts too would double-count ordinary commands.
                "command_execution" | "file_change" | "mcp_tool_call" | "web_search" => {
                    outcome.tool_calls += 1;
                    let (tool, detail) = match item["type"].as_str().unwrap_or_default() {
                        "command_execution" => {
                            ("shell", item["command"].as_str().unwrap_or_default())
                        }
                        "file_change" => ("edit", ""),
                        "mcp_tool_call" => (item["tool"].as_str().unwrap_or("MCP"), ""),
                        _ => ("search", ""),
                    };
                    watching(Progress::Using { tool: tool.into(), detail: detail.into() });
                }
                // Reasoning and arbitrary tool outputs never become records.
                _ => {}
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn errors_and_truncated_turns_do_not_claim_success() {
        let mut out = Outcome::default();
        absorb(&mut out, &json!({"type":"turn.started"}), &mut |_| {});
        absorb(
            &mut out,
            &json!({"type":"item.completed", "item":{"type":"agent_message","text":"Checking now"}}),
            &mut |_| {},
        );
        assert_eq!(out.failed.as_deref(), Some(INCOMPLETE));
        absorb(
            &mut out,
            &json!({"type":"turn.failed","error":{"message":"expired login"}}),
            &mut |_| {},
        );
        assert_eq!(out.failed.as_deref(), Some("expired login"));
        absorb(&mut out, &json!({"type":"turn.completed"}), &mut |_| {});
        assert!(out.failed.is_none());
        assert!(out.cost.is_none(), "token usage is not a dollar price");
    }
}
