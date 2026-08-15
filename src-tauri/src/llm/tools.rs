//! The tools an agent can call.
//!
//! Two, deliberately. `directory` is A2A's Agent Card discovery reduced to what
//! a local app can use; `send_message` is the whole point of the product. Every
//! additional tool is surface a model can get wrong, so the bar for a third one
//! is high.
//!
//! Schemas are tight (`additionalProperties: false`, `minItems`, explicit
//! enums) because a precise interface is what makes correct usage the default.
//! Parsing is deliberately looser than the schema: models routinely send a bare
//! string where an array is specified, and refusing that produces a retry loop
//! rather than a working app.

use serde::{Deserialize, Serialize};

use crate::llm::openrouter::{ToolCall, ToolSpec};

pub const DIRECTORY: &str = "directory";
pub const SEND_MESSAGE: &str = "send_message";
pub const UPDATE_NOTES: &str = "update_notes";
pub const RUN_COMMAND: &str = "run_command";
pub const OPEN_ON_DESKTOP: &str = "open_on_desktop";

/// Tool definitions offered on every agent turn.
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: DIRECTORY.to_string(),
            description: "List the other agents in this workspace, with their skills and current \
                          status. Call this before send_message whenever you are not certain of \
                          an agent's exact name."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: UPDATE_NOTES.to_string(),
            // The description is the whole design. It has to make selective
            // writing and consolidation the obvious reading, because the model
            // has no other signal about what belongs in a durable file.
            description: "Replace your notes. Your notes are a short markdown file shown to you \
                          at the start of every turn, so anything kept there you will always \
                          know. Record only what will still matter in a week: who you are and \
                          how you work, the operator's standing preferences, decisions that hold \
                          across conversations, and durable facts. Do not record the \
                          conversation itself, task-by-task progress, or anything already in the \
                          messages above. This REPLACES the file entirely, so write out \
                          everything you want to keep and leave behind what no longer holds; if \
                          something you believed turned out to be wrong, correct it here rather \
                          than adding a contradiction. Space is limited, so choose."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The complete new contents of your notes, in markdown."
                    }
                },
                "required": ["content"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: RUN_COMMAND.to_string(),
            description: "Run a shell command on your own computer: a Linux machine with a \
                          terminal, a filesystem and internet access, kept between turns. Use it \
                          to look things up (`curl`), read and write files, install packages, \
                          and run code. This is how you reach anything you do not already know. \
                          The first call may take a few seconds while the machine starts."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "minLength": 1,
                        "description": "A bash command, e.g. `curl -s wttr.in/Charleston?format=3`."
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: OPEN_ON_DESKTOP.to_string(),
            description: "Open a program on your computer's screen, where the operator can watch \
                          it and take over. Your machine runs a full Linux desktop with \
                          google-chrome, firefox-esr, a file manager and an editor installed. \
                          Use this whenever you are asked to visit a site, look at a page, or do \
                          anything a person would do in a window: `run_command` fetches text, \
                          this shows the real thing on screen. The program keeps running after \
                          this returns."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "minLength": 1,
                        "description": "The program and its arguments, e.g. \
                                        `google-chrome https://cnn.com`."
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: SEND_MESSAGE.to_string(),
            description: "Send a message to one or more other agents. Delivery is asynchronous \
                          and non-blocking: this returns as soon as the messages are queued. \
                          Replies, if any, arrive later as new messages addressed to you. Do not \
                          wait for a reply and do not call this again to check for one."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "to": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "description": "Exact agent names, as returned by directory."
                    },
                    "text": {
                        "type": "string",
                        "minLength": 1,
                        "description": "The message body, written as if speaking directly to the \
                                        recipient. Do not address several agents in one body; \
                                        send the same text to each instead."
                    }
                },
                "required": ["to", "text"],
                "additionalProperties": false
            }),
        },
    ]
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolInvocation {
    Directory,
    SendMessage { to: Vec<String>, text: String },
    UpdateNotes { content: String },
    RunCommand { command: String },
    OpenOnDesktop { command: String },
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ToolParseError {
    #[error(
        "unknown tool {name:?}. Available tools: directory, send_message, update_notes, \
         run_command, open_on_desktop."
    )]
    UnknownTool { name: String },
    #[error("arguments for {name} were not valid JSON: {detail}")]
    BadJson { name: String, detail: String },
    #[error("send_message needs a non-empty `to` list of agent names")]
    MissingRecipients,
    #[error("send_message needs a non-empty `text`")]
    MissingText,
    #[error("update_notes needs a `content` string")]
    MissingContent,
    #[error("run_command needs a non-empty `command` string")]
    MissingCommand,
    #[error("open_on_desktop needs a non-empty `command` string")]
    MissingDesktopCommand,
}

impl ToolParseError {
    /// What gets handed back to the model. Says what was wrong and what a
    /// correct call looks like, so the next attempt can succeed.
    pub fn guidance(&self) -> String {
        match self {
            ToolParseError::UnknownTool { name } => {
                format!(
                    "Error: no tool named {name:?}. You can call `directory`, `send_message`, \
                     `update_notes`, or `run_command`."
                )
            }
            ToolParseError::BadJson { name, detail } => format!(
                "Error: the arguments to `{name}` were not valid JSON ({detail}). Send a single \
                 well-formed JSON object."
            ),
            ToolParseError::MissingDesktopCommand => {
                "Error: `command` must name a graphical program to start, for example \
                 {\"command\": \"google-chrome https://cnn.com\"}."
                    .to_string()
            }
            ToolParseError::MissingCommand => {
                "Error: `command` must be a non-empty string, for example \
                 {\"command\": \"curl -s wttr.in/Charleston?format=3\"}."
                    .to_string()
            }
            ToolParseError::MissingRecipients => {
                "Error: `to` must be a non-empty array of exact agent names. Call `directory` to \
                 see them."
                    .to_string()
            }
            ToolParseError::MissingText => {
                "Error: `text` must be a non-empty string containing the message body.".to_string()
            }
            ToolParseError::MissingContent => {
                "Error: `content` must be a string holding the complete new notes. To clear your \
                 notes, pass an empty string."
                    .to_string()
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct SendArgs {
    #[serde(default)]
    to: Option<serde_json::Value>,
    #[serde(default)]
    text: Option<String>,
    /// Accepted because models reach for it by analogy with other APIs.
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

pub fn parse(call: &ToolCall) -> Result<ToolInvocation, ToolParseError> {
    match call.name.as_str() {
        DIRECTORY => Ok(ToolInvocation::Directory),
        OPEN_ON_DESKTOP => {
            let value = call.parsed_arguments().map_err(|e| ToolParseError::BadJson {
                name: OPEN_ON_DESKTOP.to_string(),
                detail: e.to_string(),
            })?;
            match value.get("command").or_else(|| value.get("app")).or_else(|| value.get("url")) {
                Some(serde_json::Value::String(command)) if !command.trim().is_empty() => {
                    Ok(ToolInvocation::OpenOnDesktop { command: command.clone() })
                }
                _ => Err(ToolParseError::MissingDesktopCommand),
            }
        }
        RUN_COMMAND => {
            let value = call.parsed_arguments().map_err(|e| ToolParseError::BadJson {
                name: RUN_COMMAND.to_string(),
                detail: e.to_string(),
            })?;
            match value.get("command").or_else(|| value.get("cmd")) {
                Some(serde_json::Value::String(command)) if !command.trim().is_empty() => {
                    Ok(ToolInvocation::RunCommand { command: command.clone() })
                }
                _ => Err(ToolParseError::MissingCommand),
            }
        }
        UPDATE_NOTES => {
            let value = call.parsed_arguments().map_err(|e| ToolParseError::BadJson {
                name: UPDATE_NOTES.to_string(),
                detail: e.to_string(),
            })?;
            // An empty string is a legitimate instruction: clear the notes.
            match value.get("content").or_else(|| value.get("notes")) {
                Some(serde_json::Value::String(content)) => {
                    Ok(ToolInvocation::UpdateNotes { content: content.clone() })
                }
                _ => Err(ToolParseError::MissingContent),
            }
        }
        SEND_MESSAGE => {
            let value = call.parsed_arguments().map_err(|e| ToolParseError::BadJson {
                name: SEND_MESSAGE.to_string(),
                detail: e.to_string(),
            })?;
            let args: SendArgs = serde_json::from_value(value).map_err(|e| {
                ToolParseError::BadJson { name: SEND_MESSAGE.to_string(), detail: e.to_string() }
            })?;

            let mut to = normalize_recipients(args.to.as_ref());
            if to.is_empty() {
                // `agent: "Chef"` is a common near-miss worth accepting.
                if let Some(single) =
                    args.agent.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty())
                {
                    to.push(single.to_string());
                }
            }
            if to.is_empty() {
                return Err(ToolParseError::MissingRecipients);
            }

            let text = args
                .text
                .or(args.message)
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .ok_or(ToolParseError::MissingText)?;

            Ok(ToolInvocation::SendMessage { to, text })
        }
        other => Err(ToolParseError::UnknownTool { name: other.to_string() }),
    }
}

/// Coerces the several shapes models actually emit into a list of names.
///
/// Specified as an array of strings. Observed in the wild: a bare string, a
/// comma-separated string, an array containing objects with a `name` field.
/// Each is unambiguous, so rejecting them buys nothing but a retry.
fn normalize_recipients(value: Option<&serde_json::Value>) -> Vec<String> {
    let mut out = Vec::new();
    match value {
        Some(serde_json::Value::String(one)) => {
            for piece in one.split(',') {
                let trimmed = piece.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                }
            }
        }
        Some(serde_json::Value::Array(items)) => {
            for item in items {
                match item {
                    serde_json::Value::String(name) => {
                        let trimmed = name.trim();
                        if !trimmed.is_empty() {
                            out.push(trimmed.to_string());
                        }
                    }
                    serde_json::Value::Object(map) => {
                        if let Some(serde_json::Value::String(name)) =
                            map.get("name").or_else(|| map.get("agent"))
                        {
                            let trimmed = name.trim();
                            if !trimmed.is_empty() {
                                out.push(trimmed.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    // A model asked to message everyone sometimes lists a name twice. Sending
    // twice would waste a turn and trip the dedup guard for no reason.
    out.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    let mut seen = std::collections::HashSet::new();
    out.retain(|name| seen.insert(name.to_lowercase()));
    out
}

/// What `send_message` reports back per recipient.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum Delivery {
    Queued { to: String },
    Refused { to: String, reason: String },
}

/// Renders delivery results as the tool result string the model reads.
pub fn render_deliveries(results: &[Delivery]) -> String {
    let mut lines = Vec::new();
    let queued: Vec<&str> = results
        .iter()
        .filter_map(|d| match d {
            Delivery::Queued { to } => Some(to.as_str()),
            _ => None,
        })
        .collect();

    if !queued.is_empty() {
        lines.push(format!(
            "Queued for delivery to: {}. Replies will arrive later as new messages; do not wait.",
            queued.join(", ")
        ));
    }
    for result in results {
        if let Delivery::Refused { to, reason } = result {
            lines.push(format!("Not delivered to {to}: {reason}"));
        }
    }
    if lines.is_empty() {
        lines.push("No messages were sent.".to_string());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, arguments: &str) -> ToolCall {
        ToolCall { id: "call_1".into(), name: name.into(), arguments: arguments.into() }
    }

    #[test]
    fn a_command_is_parsed_from_either_spelling() {
        // Models reach for `cmd` about as often as `command`, and refusing one
        // of them wastes a whole turn on a rejection.
        for field in ["command", "cmd"] {
            let parsed = parse(&call(RUN_COMMAND, &format!("{{\"{field}\": \"echo hi\"}}")));
            assert_eq!(parsed, Ok(ToolInvocation::RunCommand { command: "echo hi".into() }));
        }
    }

    #[test]
    fn an_empty_command_is_refused_with_an_example() {
        let err = parse(&call(RUN_COMMAND, "{\"command\": \"   \"}")).unwrap_err();
        assert_eq!(err, ToolParseError::MissingCommand);
        assert!(err.guidance().contains("curl"), "the model needs to see a usable call");
    }

    #[test]
    fn a_desktop_program_is_parsed_from_any_of_the_obvious_spellings() {
        // Asked to visit a site, a model reaches for `url` as often as
        // `command`, and refusing one of them wastes a whole turn.
        for field in ["command", "app", "url"] {
            let parsed =
                parse(&call(OPEN_ON_DESKTOP, &format!("{{\"{field}\": \"google-chrome x\"}}")));
            assert_eq!(
                parsed,
                Ok(ToolInvocation::OpenOnDesktop { command: "google-chrome x".into() })
            );
        }
    }

    #[test]
    fn the_desktop_tool_names_a_browser_so_the_agent_knows_it_has_one() {
        // The failure this exists to stop: an agent with a working desktop
        // replying that it has no graphical browser.
        let spec = specs().into_iter().find(|s| s.name == OPEN_ON_DESKTOP).unwrap();
        assert!(spec.description.contains("google-chrome"), "{}", spec.description);
    }

    #[test]
    fn every_tool_is_offered_with_a_strict_schema() {
        let specs = specs();
        assert_eq!(
            specs.len(),
            5,
            "directory, run_command, open_on_desktop, send_message, update_notes"
        );
        for spec in &specs {
            assert_eq!(
                spec.parameters["additionalProperties"], false,
                "{} must reject stray fields",
                spec.name
            );
            assert!(
                spec.description.len() > 60,
                "{} needs a description a model can act on",
                spec.name
            );
        }
    }

    #[test]
    fn send_message_description_tells_the_model_not_to_block() {
        let spec = specs().into_iter().find(|s| s.name == SEND_MESSAGE).unwrap();
        let text = spec.description.to_lowercase();
        assert!(text.contains("non-blocking") || text.contains("asynchronous"));
        assert!(text.contains("do not wait"), "blocking on a reply is the failure mode to prevent");
    }

    #[test]
    fn directory_takes_no_arguments() {
        assert_eq!(parse(&call(DIRECTORY, "")).unwrap(), ToolInvocation::Directory);
        assert_eq!(parse(&call(DIRECTORY, "{}")).unwrap(), ToolInvocation::Directory);
    }

    #[test]
    fn send_message_parses_the_specified_shape() {
        let parsed =
            parse(&call(SEND_MESSAGE, r#"{"to":["Chef","Barista"],"text":"hello"}"#)).unwrap();
        assert_eq!(
            parsed,
            ToolInvocation::SendMessage {
                to: vec!["Chef".into(), "Barista".into()],
                text: "hello".into()
            }
        );
    }

    #[test]
    fn a_bare_string_recipient_is_accepted() {
        let parsed = parse(&call(SEND_MESSAGE, r#"{"to":"Chef","text":"hi"}"#)).unwrap();
        assert_eq!(
            parsed,
            ToolInvocation::SendMessage { to: vec!["Chef".into()], text: "hi".into() }
        );
    }

    #[test]
    fn a_comma_separated_recipient_string_is_split() {
        let parsed =
            parse(&call(SEND_MESSAGE, r#"{"to":"Chef, Barista ,Host","text":"hi"}"#)).unwrap();
        assert_eq!(
            parsed,
            ToolInvocation::SendMessage {
                to: vec!["Chef".into(), "Barista".into(), "Host".into()],
                text: "hi".into()
            }
        );
    }

    #[test]
    fn recipient_objects_are_unwrapped() {
        let parsed =
            parse(&call(SEND_MESSAGE, r#"{"to":[{"name":"Chef"},{"agent":"Host"}],"text":"hi"}"#))
                .unwrap();
        assert_eq!(
            parsed,
            ToolInvocation::SendMessage {
                to: vec!["Chef".into(), "Host".into()],
                text: "hi".into()
            }
        );
    }

    #[test]
    fn duplicate_recipients_are_collapsed_case_insensitively() {
        let parsed =
            parse(&call(SEND_MESSAGE, r#"{"to":["Chef","chef","CHEF","Host"],"text":"hi"}"#))
                .unwrap();
        match parsed {
            ToolInvocation::SendMessage { to, .. } => assert_eq!(to, vec!["Chef", "Host"]),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn the_message_alias_is_accepted_for_text() {
        let parsed = parse(&call(SEND_MESSAGE, r#"{"to":["Chef"],"message":"hi"}"#)).unwrap();
        assert_eq!(
            parsed,
            ToolInvocation::SendMessage { to: vec!["Chef".into()], text: "hi".into() }
        );
    }

    #[test]
    fn the_agent_alias_is_accepted_for_a_single_recipient() {
        let parsed = parse(&call(SEND_MESSAGE, r#"{"agent":"Chef","text":"hi"}"#)).unwrap();
        assert_eq!(
            parsed,
            ToolInvocation::SendMessage { to: vec!["Chef".into()], text: "hi".into() }
        );
    }

    #[test]
    fn text_takes_precedence_over_the_message_alias() {
        let parsed =
            parse(&call(SEND_MESSAGE, r#"{"to":["Chef"],"text":"real","message":"alias"}"#))
                .unwrap();
        match parsed {
            ToolInvocation::SendMessage { text, .. } => assert_eq!(text, "real"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn missing_recipients_are_rejected_with_guidance() {
        let err = parse(&call(SEND_MESSAGE, r#"{"text":"hi"}"#)).unwrap_err();
        assert_eq!(err, ToolParseError::MissingRecipients);
        assert!(err.guidance().contains("directory"), "tell the model how to recover");
    }

    #[test]
    fn empty_recipient_lists_are_rejected() {
        assert_eq!(
            parse(&call(SEND_MESSAGE, r#"{"to":[],"text":"hi"}"#)).unwrap_err(),
            ToolParseError::MissingRecipients
        );
        assert_eq!(
            parse(&call(SEND_MESSAGE, r#"{"to":["  ", ""],"text":"hi"}"#)).unwrap_err(),
            ToolParseError::MissingRecipients
        );
    }

    #[test]
    fn blank_text_is_rejected() {
        assert_eq!(
            parse(&call(SEND_MESSAGE, r#"{"to":["Chef"],"text":"   "}"#)).unwrap_err(),
            ToolParseError::MissingText
        );
    }

    #[test]
    fn malformed_json_is_reported_with_the_tool_name() {
        let err = parse(&call(SEND_MESSAGE, "{not json")).unwrap_err();
        assert!(matches!(err, ToolParseError::BadJson { ref name, .. } if name == SEND_MESSAGE));
        assert!(err.guidance().contains("well-formed JSON"));
    }

    #[test]
    fn update_notes_takes_the_complete_new_contents() {
        // Doubled hashes: a markdown heading inside the JSON would otherwise
        // close an `r#"..."#` literal early.
        let parsed = parse(&call(UPDATE_NOTES, r##"{"content":"# Style\nTerse."}"##)).unwrap();
        assert_eq!(parsed, ToolInvocation::UpdateNotes { content: "# Style\nTerse.".into() });
    }

    #[test]
    fn clearing_notes_is_allowed() {
        // An empty string is an instruction, not a mistake.
        assert_eq!(
            parse(&call(UPDATE_NOTES, r#"{"content":""}"#)).unwrap(),
            ToolInvocation::UpdateNotes { content: String::new() }
        );
    }

    #[test]
    fn update_notes_accepts_the_notes_alias() {
        assert_eq!(
            parse(&call(UPDATE_NOTES, r#"{"notes":"kept"}"#)).unwrap(),
            ToolInvocation::UpdateNotes { content: "kept".into() }
        );
    }

    #[test]
    fn update_notes_without_content_is_rejected_with_guidance() {
        let err = parse(&call(UPDATE_NOTES, "{}")).unwrap_err();
        assert_eq!(err, ToolParseError::MissingContent);
        assert!(err.guidance().contains("empty string"));
    }

    #[test]
    fn the_notes_tool_asks_for_durable_things_and_forbids_a_transcript_dump() {
        // The description is the only control over what an agent writes, so the
        // selective-write instruction has to survive edits.
        let spec = specs().into_iter().find(|s| s.name == UPDATE_NOTES).unwrap();
        let text = spec.description.to_lowercase();
        assert!(text.contains("still matter in a week"));
        assert!(text.contains("do not record the conversation"));
        assert!(text.contains("replaces the file"), "consolidation must be explicit");
        assert!(text.contains("space is limited"));
    }

    #[test]
    fn an_unknown_tool_lists_the_real_ones() {
        let err = parse(&call("delete_everything", "{}")).unwrap_err();
        assert!(matches!(err, ToolParseError::UnknownTool { .. }));
        assert!(err.guidance().contains("directory"));
        assert!(err.guidance().contains("send_message"));
        assert!(err.guidance().contains("update_notes"));
    }

    #[test]
    fn delivery_rendering_separates_success_from_refusal() {
        let rendered = render_deliveries(&[
            Delivery::Queued { to: "Chef".into() },
            Delivery::Queued { to: "Host".into() },
            Delivery::Refused { to: "Ghost".into(), reason: "no agent named Ghost".into() },
        ]);
        assert!(rendered.contains("Chef, Host"));
        assert!(rendered.contains("do not wait"), "reinforce non-blocking at the result too");
        assert!(rendered.contains("Not delivered to Ghost"));
    }

    #[test]
    fn delivery_rendering_handles_a_total_refusal() {
        let rendered = render_deliveries(&[Delivery::Refused {
            to: "Chef".into(),
            reason: "hop limit".into(),
        }]);
        assert!(!rendered.contains("Queued"));
        assert!(rendered.contains("hop limit"));
    }

    #[test]
    fn delivery_rendering_handles_an_empty_result() {
        assert_eq!(render_deliveries(&[]), "No messages were sent.");
    }
}
