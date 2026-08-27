//! The Claude backend, which is where an Anthropic subscription can be spent.
//!
//! Guaca speaks one shape of model API everywhere else: a list of messages in,
//! a stream of deltas and a set of tool calls out. This backend is not an
//! endpoint at all. It runs the `claude` program once per model call, writes the
//! conversation to its stdin and reads its stdout, so this file is a
//! translation in the same sense [`super::codex`] is, against a process rather
//! than a wire.
//!
//! ## Why a program and not a token
//!
//! Anthropic restricts a consumer OAuth token to the program it was issued to,
//! server-side. Guaca holding one and dialling the Messages API is refused; the
//! `claude` program on the same machine and the same account runs the work off
//! the plan. So the way to spend a subscription is to *be* the program, which is
//! the same fact [`crate::domain::repository::Harness`] is built on, one level
//! up: there it decides who writes code, here it decides who answers a turn.
//! `docs/PROTOCOL.md` has the dates and the sources.
//!
//! ## Guaca keeps its own loop
//!
//! This is the decision the rest of the file follows from, and it is the
//! opposite of the one [`crate::coding`] makes.
//!
//! `claude` is an agent harness: given tools it can reach, it runs its own
//! rounds and hands back a conclusion. Letting it do that here would move
//! `max_tool_rounds`, `reserve_step` and every stop check inside a program this
//! app does not control, and the guard would be enforced by a process boundary
//! instead of by `runtime/guard.rs`. A coding job can afford that because it is
//! a different unit of work with its own budget. A turn cannot: it is the unit
//! the five limits are written in.
//!
//! So the program is given no tools at all (`--tools ""`, no MCP servers) and
//! asked for one structured answer per call, through `--json-schema`. What comes
//! back is what this app's own tool call already looks like: something to say,
//! and a list of calls to make. Guaca dispatches them, exactly as it does for
//! the other two providers, and the round loop in `runtime/mod.rs` never learns
//! there was a third.
//!
//! The schema is built from the turn's own [`ToolSpec`]s as a discriminated
//! union, so the program validates a call against the tool's real parameters
//! before this app ever sees it. A malformed tool call is not unlikely here, it
//! is unrepresentable.
//!
//! ## The program is run with the operator's settings switched off
//!
//! Every isolation flag in [`argv`] is load-bearing, and the measurement is in
//! its doc comment. `claude` started the ordinary way loads the operator's own
//! MCP servers, hooks and settings, and an agent in this app would inherit all
//! of it: their connected Gmail, their Grafana, their `SessionStart` hooks, and
//! a hundred thousand tokens of tool definitions on every call. What the
//! operator configures *for Claude* that this app does want is the part they
//! cannot pass another way: the sign-in and the model.
//!
//! ## The program is started in an empty directory this app owns
//!
//! A child process inherits its parent's working directory, and a
//! double-clicked app's is `/`: `launchd` starts one there, which is the same
//! finding [`crate::programs`] is built on, from the other end. `claude` takes
//! the directory it is started in as the project it is working on and asks the
//! operating system what it may reach inside it. Started in `/` that question
//! covers every protected folder on the machine, and macOS answers a child's
//! request in the name of the *responsible* process rather than the child, so
//! the operator is asked whether **Guaca** may read their photos, their music
//! library, their Desktop and their Downloads, once per model call, about
//! folders this app has never read and has no tool that could.
//!
//! Measured on 2026-08-27 against 2.1.247: one turn produced
//! `kTCCServiceMediaLibrary`, `kTCCServicePhotos`,
//! `kTCCServiceSystemPolicyDesktopFolder`,
//! `kTCCServiceSystemPolicyDownloadsFolder` and a refused
//! `kTCCServiceSystemPolicyAllFiles`, each logged by `tccd` as
//! `responsible=com.madebywelch.guac, accessing=com.anthropic.claude-code`.
//! [`scratch`] is what takes the disk back out of that question. It is not
//! tidiness: a permission dialog naming this app for something it does not do
//! is one an operator either denies, and then distrusts the app, or accepts,
//! and then has granted a real reach to a program that asked under a borrowed
//! name.
//!
//! ## What it does not report
//!
//! Money that moved. `total_cost_usd` on a plan is the equivalent API price
//! rather than a charge, which is the same claim [`crate::coding::Outcome`]
//! makes and no more. It is dropped rather than recorded, so the usage table
//! keeps meaning what it has always meant; the tokens are real and are counted.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::config::InferenceConfig;
use crate::llm::openrouter::{
    ChatMessage, ChatRequest, Completion, ContentPart, LlmError, Token, ToolCall, ToolSpec, Usage,
    UserContent,
};

/// The program, found on `PATH` rather than configured, for the reason
/// [`crate::coding`] finds its own there: a second place to say where it lives
/// is a second place for it to be wrong.
pub const PROGRAM: &str = "claude";

/// What to do about it not being there. Named once so the refusal and the
/// documentation cannot drift.
pub const INSTALL: &str = "npm install -g @anthropic-ai/claude-code";

/// What this provider reports as its model.
///
/// A label, not a model id. Which model runs is the program's own setting and
/// this app does not pass `--model`, so there is nothing truthful to put here
/// but the name of the thing that decided.
pub const MODEL_LABEL: &str = "claude";

/// The field the reply goes in.
const SAY: &str = "say";
/// The field the tool calls go in.
const CALLS: &str = "calls";

/// Builds the schema the program is held to.
///
/// One object with two fields, and `calls` is a discriminated union over the
/// tools this turn was actually offered: `name` is a `const`, and `arguments`
/// is that tool's own parameter schema. The program validates against it before
/// answering, so a call naming a tool that does not exist, or carrying the
/// wrong arguments for one that does, cannot reach [`Completion`].
///
/// A turn with no tools gets no `calls` field rather than an empty union.
/// `oneOf: []` matches nothing, so a model that decided to call something would
/// be held at a schema it cannot satisfy and the call would fail rather than
/// the tool being refused, which is the wrong error in the wrong place.
pub fn schema(tools: &[ToolSpec]) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        SAY.to_string(),
        serde_json::json!({
            "type": "string",
            "description": "What you are saying this round. Empty only when you are \
                            doing something and have nothing to say about it yet.",
        }),
    );

    let mut required = vec![SAY];

    if !tools.is_empty() {
        let variants: Vec<serde_json::Value> = tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name", "arguments"],
                    "properties": {
                        "name": { "const": tool.name, "description": tool.description },
                        "arguments": tool.parameters,
                    },
                })
            })
            .collect();

        properties.insert(
            CALLS.to_string(),
            serde_json::json!({
                "type": "array",
                "description": "The tools to run this round. Empty when there are none.",
                "items": { "oneOf": variants },
            }),
        );
        required.push(CALLS);
    }

    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    })
}

/// The argument vector, which is the part of this file a test can hold still.
///
/// Every flag below either isolates the run or is required by another flag, and
/// the isolation is not hygiene. Measured against this program at 2.1.247, one
/// trivial reply cost 104,371 input tokens and named 200-odd tools with these
/// flags off, and 783 tokens naming none with them on: the operator's own MCP
/// servers, settings and hooks are otherwise part of every turn every agent in
/// this app takes.
///
/// - **`--tools ""`** drops the built-ins. An agent here has this app's tools
///   and no `Bash`, `Edit` or `Read`.
/// - **`--strict-mcp-config`** with an empty **`--mcp-config`** drops the
///   operator's MCP servers. Without the strict flag the empty config is merged
///   with theirs rather than replacing it.
/// - **`--setting-sources ""`** drops their settings files, and with them their
///   hooks, which otherwise run on every call.
/// - **`--disable-slash-commands`** drops the skills, which are prompt weight
///   for a surface nothing here can reach.
/// - **`--no-session-persistence`** because this app holds the conversation.
///   A session on disk would be a second, diverging copy of it.
/// - **`--include-partial-messages`** is what makes the thinking and the
///   narration arrive while the call is running rather than at the end of it.
/// - **`--input-format stream-json`** keeps the conversation off the command
///   line, which is where a screenshot would not fit, and it is the only input
///   format that carries an image. It requires `--output-format stream-json`,
///   which requires `--verbose`.
///
/// No `--model`, for the reason [`MODEL_LABEL`] exists. No `--permission-mode`:
/// there is nothing to permit, because there are no tools.
pub fn argv(schema: &serde_json::Value, system_prompt: &str) -> Vec<String> {
    [
        "-p",
        "--system-prompt",
        system_prompt,
        "--json-schema",
        &schema.to_string(),
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--tools",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
        "--setting-sources",
        "",
        "--disable-slash-commands",
        "--no-session-persistence",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Splits this app's messages into the two things the program takes.
///
/// The system prompt goes on the command line and everything else becomes the
/// content of a single user message. That is not a simplification of the
/// conversation, it is where the conversation has to go: the program's streaming
/// input carries user messages, so there is nowhere to put an assistant turn or
/// a tool result as itself. They are written into the one message instead,
/// labeled, which is also what keeps this app the authority on the history.
/// Nothing is resumed and no session is kept, so the whole conversation is
/// rebuilt on every call, exactly as [`super::codex`] rebuilds its input.
///
/// A user message is passed through unlabeled. It is what `prompt.rs` wrote and
/// it already says what it is; a label here would be this file editing a prompt
/// that belongs to another file.
pub fn conversation(messages: &[ChatMessage]) -> (String, Vec<serde_json::Value>) {
    let mut system = Vec::new();
    let mut blocks: Vec<serde_json::Value> = Vec::new();
    // A tool result names the call it answers and not the tool it ran, so the
    // name has to be carried down from the assistant turn that asked. Without
    // it a round of results reads as a list of unattributed strings.
    let mut names: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();

    let text = |blocks: &mut Vec<serde_json::Value>, body: String| {
        if !body.trim().is_empty() {
            blocks.push(serde_json::json!({ "type": "text", "text": body }));
        }
    };

    for message in messages {
        match message {
            ChatMessage::System { content } => system.push(content.clone()),
            ChatMessage::User { content } => match content {
                UserContent::Text(body) => text(&mut blocks, body.clone()),
                UserContent::Parts(parts) => {
                    for part in parts {
                        match part {
                            ContentPart::Text { text: body } => text(&mut blocks, body.clone()),
                            ContentPart::ImageUrl { image_url } => {
                                // Only a `data:` URL, because that is the only
                                // kind this app makes: a screenshot never leaves
                                // the machine as a file. Anything else is
                                // dropped rather than fetched, which would be
                                // this transport reaching the network on a
                                // model's say-so.
                                if let Some(image) = image_block(&image_url.url) {
                                    blocks.push(image);
                                }
                            }
                        }
                    }
                }
            },
            ChatMessage::Assistant { content, tool_calls } => {
                if let Some(body) = content.as_ref().filter(|b| !b.trim().is_empty()) {
                    text(&mut blocks, format!("You said:\n{body}"));
                }
                for call in tool_calls {
                    names.insert(call.id.clone(), call.function.name.clone());
                    text(
                        &mut blocks,
                        format!("You called {}({})", call.function.name, call.function.arguments),
                    );
                }
            }
            ChatMessage::Tool { tool_call_id, content } => {
                let name = names.get(tool_call_id).map(String::as_str).unwrap_or("it");
                text(&mut blocks, format!("{name} answered:\n{content}"));
            }
        }
    }

    (system.join("\n\n"), blocks)
}

/// Turns a `data:` URL into the image block the program's input format takes.
///
/// `None` for anything that is not one, which the caller drops.
fn image_block(url: &str) -> Option<serde_json::Value> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let media_type = meta.strip_suffix(";base64")?;
    if media_type.is_empty() || data.is_empty() {
        return None;
    }
    Some(serde_json::json!({
        "type": "image",
        "source": { "type": "base64", "media_type": media_type, "data": data },
    }))
}

// ---- the stream ----------------------------------------------------------

/// The result frame, which is the last line the program writes.
///
/// Only the fields this app acts on. The frame carries a great deal more, and
/// deserializing all of it would be this file promising to keep up with a
/// program that adds a field a release.
/// Snake case on the wire, and deliberately not renamed. The frame mixes both
/// conventions (`modelUsage` sits beside `total_cost_usd`), so a blanket rule
/// here would be right about the fields it happened to be written against and
/// silently wrong about the next one: an absent field deserializes to its
/// default rather than failing, so the symptom is a reply that went missing.
#[derive(Debug, Default, Deserialize)]
struct ResultFrame {
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    subtype: String,
    #[serde(default)]
    structured_output: Option<StructuredOutput>,
    #[serde(default)]
    usage: Option<ProgramUsage>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    api_error_status: Option<serde_json::Value>,
    /// The program's own words when it failed, which is the only description of
    /// the failure that exists.
    #[serde(default)]
    result: Option<String>,
}

/// What the schema in [`schema`] produces.
#[derive(Debug, Default, Deserialize)]
struct StructuredOutput {
    #[serde(default)]
    say: String,
    #[serde(default)]
    calls: Vec<StructuredCall>,
}

#[derive(Debug, Deserialize)]
struct StructuredCall {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

/// The program counts cache tokens apart from fresh ones. All three were read.
#[derive(Debug, Default, Deserialize)]
struct ProgramUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

impl ProgramUsage {
    /// Cost is `None` on purpose, and it is not an omission.
    ///
    /// The program reports `total_cost_usd` on a plan as the equivalent API
    /// price rather than a charge. Recording it would put money in the usage
    /// table that nobody spent, and the table is what an operator reads to
    /// decide whether a crew is affordable. The same claim, and the same
    /// refusal to make a larger one, is in [`crate::coding::Outcome::cost`].
    fn tally(&self) -> Usage {
        Usage {
            prompt_tokens: self
                .input_tokens
                .saturating_add(self.cache_creation_input_tokens)
                .saturating_add(self.cache_read_input_tokens),
            completion_tokens: self.output_tokens,
            cost: None,
        }
    }
}

/// Folds one line of the program's stdout into the call's state.
///
/// Split out from the read loop so the whole translation is a pure function of
/// the lines, which is what the tests beside this file drive.
///
/// Both the thinking and the prose the model writes on its way to the answer go
/// to [`Token::Reasoning`], and neither is kept. The reply is the `say` field of
/// the structured answer, which arrives whole at the end of the call. That is
/// the one way this provider differs from the other two on screen: the thinking
/// line moves while the call runs, and the message lands complete rather than a
/// token at a time. Streaming it would mean decoding a JSON string that is still
/// arriving, and a half-decoded escape drawn into a channel is a worse failure
/// than a message that appears at once.
fn fold<F>(line: &str, out: &mut Completion, on_token: &mut F) -> Result<bool, LlmError>
where
    F: FnMut(Token<'_>),
{
    let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
        // The program writes diagnostics to stderr, but a line it cannot be
        // held to is not a reason to fail a call that may still succeed.
        return Ok(false);
    };

    match event.get("type").and_then(serde_json::Value::as_str) {
        Some("stream_event") => {
            let delta = event.get("event").and_then(|e| e.get("delta"));
            if let Some(delta) = delta {
                let kind = delta.get("type").and_then(serde_json::Value::as_str);
                let text = match kind {
                    Some("thinking_delta") => delta.get("thinking"),
                    Some("text_delta") => delta.get("text"),
                    _ => None,
                };
                if let Some(text) = text.and_then(serde_json::Value::as_str) {
                    on_token(Token::Reasoning(text));
                }
            }
            Ok(false)
        }
        Some("result") => {
            let frame: ResultFrame = serde_json::from_value(event)
                .map_err(|err| LlmError::Decode(format!("{PROGRAM} result frame: {err}")))?;

            if frame.is_error || frame.structured_output.is_none() {
                return Err(refusal(&frame));
            }

            let answer = frame.structured_output.unwrap_or_default();
            out.content = answer.say;
            out.tool_calls = answer
                .calls
                .into_iter()
                .enumerate()
                .map(|(index, call)| ToolCall {
                    // The program answers with a list rather than with calls
                    // that carry ids, so the id is made here. It has to be
                    // unique across the whole turn and not just this round:
                    // the history keeps every round's calls and pairs each
                    // result to one by id, so a counter that restarted would
                    // make round two's first result answer round one's.
                    id: format!("{}-{}", uuid::Uuid::new_v4(), index),
                    name: call.name,
                    arguments: call.arguments.to_string(),
                })
                .collect();
            out.finish_reason = frame.stop_reason;
            out.usage = frame.usage.as_ref().map(ProgramUsage::tally);
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Turns a failed result frame into the refusal an agent reads mid-turn.
///
/// The program's own words are carried through rather than replaced. With no
/// endpoint and no status code, they are the only description of what went
/// wrong that exists, and the commonest cause here is a sign-in or a spent plan
/// that only the operator can fix.
fn refusal(frame: &ResultFrame) -> LlmError {
    let said = frame
        .result
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("it reported no reason");

    if let Some(status) = frame.api_error_status.as_ref().and_then(|s| s.as_u64()) {
        return LlmError::Upstream { status: status as u16, message: said.to_string() };
    }

    match frame.subtype.as_str() {
        // The program hit its own turn ceiling. It cannot happen with no tools
        // to loop on, so it means the program did something this app did not
        // ask for, and reporting it as a bug in the request is wrong.
        "error_max_turns" => LlmError::ProgramFailed {
            program: PROGRAM,
            message: format!("{PROGRAM} stopped at its own turn limit: {said}"),
        },
        _ if frame.structured_output.is_none() && !frame.is_error => LlmError::ProgramFailed {
            program: PROGRAM,
            message: format!("{PROGRAM} finished without answering in the requested shape: {said}"),
        },
        _ => LlmError::ProgramFailed { program: PROGRAM, message: said.to_string() },
    }
}

/// The directory the program is started in, made if it is not there yet.
///
/// Empty, this app's, and about nothing. The module docs have the argument for
/// why it is not the one this process happens to be standing in.
///
/// `None` is the whole error handling and it means inherit, which is where this
/// started and is no worse than it was. Answering with a path that is not on
/// disk would be worse than either: [`std::process::Command::spawn`] reports a
/// missing working directory as `NotFound`, which the caller below maps to
/// [`LlmError::ProgramMissing`], and the operator is told to install a program
/// they already have.
fn scratch() -> Option<PathBuf> {
    let at = std::env::temp_dir().join("guaca-claude");
    std::fs::create_dir_all(&at).ok()?;
    Some(at)
}

/// Runs one model call and reads what the program says about it.
///
/// The one place the third provider parts company from the other two, reached
/// from `LlmClient::stream_chat` and from nowhere else.
pub async fn stream<F>(
    cfg: &InferenceConfig,
    request: &ChatRequest,
    on_token: &mut F,
) -> Result<Completion, LlmError>
where
    F: FnMut(Token<'_>),
{
    let schema = schema(&request.tools);
    let (system, blocks) = conversation(&request.messages);
    let timeout = Duration::from_secs(cfg.request_timeout_secs.clamp(5, 900));

    let mut command = tokio::process::Command::new(PROGRAM);
    // Set rather than inherited, and the module docs say what inheriting costs.
    if let Some(at) = scratch() {
        command.current_dir(at);
    }

    let mut child = command
        .args(argv(&schema, &system))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // A call this app has given up on must not leave a process holding the
        // operator's plan open. The same reason it is set in `coding/mod.rs`.
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => LlmError::ProgramMissing { program: PROGRAM },
            _ => LlmError::ProgramFailed { program: PROGRAM, message: err.to_string() },
        })?;

    let payload = serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": blocks },
    })
    .to_string();

    // Written from a task of its own rather than inline. A turn that has just
    // looked at a screen carries a picture of it, which is megabytes, and a
    // pipe holds a page: writing it here would block this task until the child
    // drained it, while the child blocked writing a stdout nobody was reading.
    let mut stdin = child.stdin.take();
    let writer = tokio::spawn(async move {
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(payload.as_bytes()).await;
            let _ = stdin.write_all(b"\n").await;
            let _ = stdin.flush().await;
        }
        // Dropped here, which is the end-of-input the program waits for.
        drop(stdin);
    });

    let stdout = child.stdout.take().ok_or(LlmError::ProgramFailed {
        program: PROGRAM,
        message: "the program produced no output stream".to_string(),
    })?;
    let stderr = child.stderr.take();

    let mut completion = Completion::default();
    let mut answered = false;

    let read = async {
        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = lines.next_line().await.map_err(|err| {
            LlmError::Decode(format!("could not read what {PROGRAM} wrote: {err}"))
        })? {
            if fold(&line, &mut completion, on_token)? {
                answered = true;
            }
        }
        Ok::<(), LlmError>(())
    };

    match tokio::time::timeout(timeout, read).await {
        Err(_) => {
            let _ = child.start_kill();
            return Err(LlmError::Timeout { secs: timeout.as_secs() });
        }
        Ok(result) => result?,
    }

    writer.abort();

    // A stream that ended without a result frame is the case the exit status
    // and stderr are the only account of: the program refused the command line,
    // died on a signal, or could not start at all. Reported as its own words
    // rather than as a decode failure, which would send the reader here instead
    // of to their sign-in.
    if !answered {
        let said = match stderr {
            Some(stderr) => {
                let mut buffer = String::new();
                let mut reader = BufReader::new(stderr);
                let _ = tokio::io::AsyncReadExt::read_to_string(&mut reader, &mut buffer).await;
                buffer.trim().to_string()
            }
            None => String::new(),
        };
        let status = child.wait().await.ok();
        let code = status
            .and_then(|s| s.code())
            .map(|c| format!("exit {c}"))
            .unwrap_or_else(|| "no exit status".to_string());
        let detail = if said.is_empty() { code } else { format!("{code}: {said}") };
        return Err(LlmError::ProgramFailed {
            program: PROGRAM,
            message: format!("{PROGRAM} ended without answering ({detail})"),
        });
    }

    let _ = child.wait().await;
    Ok(completion)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::openrouter::{WireFunction, WireToolCall};

    fn spec(name: &str) -> ToolSpec {
        ToolSpec {
            name: name.to_string(),
            description: format!("what {name} does"),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "to": { "type": "string" } },
                "required": ["to"],
            }),
        }
    }

    fn sink() -> impl FnMut(Token<'_>) {
        |_| {}
    }

    #[test]
    fn the_schema_is_a_union_over_the_tools_this_turn_was_offered() {
        let schema = schema(&[spec("send_message"), spec("note_progress")]);
        let variants = schema["properties"]["calls"]["items"]["oneOf"].as_array().unwrap();

        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0]["properties"]["name"]["const"], "send_message");
        assert_eq!(variants[1]["properties"]["name"]["const"], "note_progress");
        // The tool's own parameters, not a bare object: this is what makes the
        // program refuse a call with the wrong arguments before this app sees
        // it.
        assert_eq!(variants[0]["properties"]["arguments"]["required"][0], "to");
    }

    #[test]
    fn a_turn_with_no_tools_is_offered_no_calls_field() {
        // Not an empty union. `oneOf: []` matches nothing, so a model that
        // decided to call something would fail the schema instead of being
        // told it has no tools.
        let schema = schema(&[]);
        assert!(schema["properties"].get(CALLS).is_none());
        assert_eq!(schema["required"].as_array().unwrap(), &[serde_json::json!(SAY)]);
    }

    #[test]
    fn the_system_prompt_leaves_the_conversation_and_the_rest_becomes_one_message() {
        let (system, blocks) = conversation(&[
            ChatMessage::system("you are Dana"),
            ChatMessage::system("and you are brief"),
            ChatMessage::user("the build is green"),
        ]);

        assert_eq!(system, "you are Dana\n\nand you are brief");
        assert_eq!(blocks.len(), 1);
        // Verbatim. What `prompt.rs` wrote is not this file's to label.
        assert_eq!(blocks[0]["text"], "the build is green");
    }

    #[test]
    fn a_tool_result_is_written_under_the_name_of_the_tool_that_ran() {
        // The result carries the call's id and not its name, so the name has to
        // come down from the assistant turn. Without it a round of results is a
        // list of unattributed strings.
        let (_, blocks) = conversation(&[
            ChatMessage::user("who is around?"),
            ChatMessage::Assistant {
                content: Some("I will ask".into()),
                tool_calls: vec![WireToolCall {
                    id: "call-1".into(),
                    kind: "function".into(),
                    function: WireFunction {
                        name: "send_message".into(),
                        arguments: r#"{"to":"Pat"}"#.into(),
                    },
                }],
            },
            ChatMessage::Tool { tool_call_id: "call-1".into(), content: "delivered".into() },
        ]);

        let text: Vec<&str> = blocks.iter().map(|b| b["text"].as_str().unwrap()).collect();
        assert_eq!(text[1], "You said:\nI will ask");
        assert_eq!(text[2], r#"You called send_message({"to":"Pat"})"#);
        assert_eq!(text[3], "send_message answered:\ndelivered");
    }

    #[test]
    fn a_result_for_a_call_nobody_made_still_says_something() {
        // A failure path: history this file did not assemble, or a call whose
        // assistant turn was dropped. The result is worth more than nothing.
        let (_, blocks) = conversation(&[ChatMessage::Tool {
            tool_call_id: "orphan".into(),
            content: "delivered".into(),
        }]);
        assert_eq!(blocks[0]["text"], "it answered:\ndelivered");
    }

    #[test]
    fn a_screenshot_crosses_as_an_image_block() {
        let (_, blocks) = conversation(&[ChatMessage::user_seeing(
            "this is your screen",
            "data:image/png;base64,AAAA",
        )]);

        assert_eq!(blocks[1]["type"], "image");
        assert_eq!(blocks[1]["source"]["media_type"], "image/png");
        assert_eq!(blocks[1]["source"]["data"], "AAAA");
    }

    #[test]
    fn an_image_that_is_not_a_data_url_is_dropped_rather_than_fetched() {
        // Following it would be this transport reaching the network because a
        // model asked. Nothing in this app makes one, so anything else here is
        // either a bug or a suggestion.
        for url in ["https://example.com/a.png", "data:image/png,notbase64", "data:;base64,AA"] {
            assert!(image_block(url).is_none(), "{url} should not have crossed");
        }
    }

    #[test]
    fn an_empty_message_leaves_no_block_behind() {
        let (_, blocks) = conversation(&[ChatMessage::Assistant {
            content: Some("   ".into()),
            tool_calls: Vec::new(),
        }]);
        assert!(blocks.is_empty());
    }

    #[test]
    fn thinking_and_narration_are_shown_and_neither_becomes_the_reply() {
        let mut seen = Vec::new();
        let mut out = Completion::default();
        let mut pen = |token: Token<'_>| match token {
            Token::Reasoning(text) => seen.push(text.to_string()),
            Token::Text(text) => panic!("the reply must not stream: {text}"),
        };

        for line in [
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"two things"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"both are independent"}}}"#,
        ] {
            assert!(!fold(line, &mut out, &mut pen).unwrap());
        }

        assert_eq!(seen, ["two things", "both are independent"]);
        assert!(out.content.is_empty());
    }

    #[test]
    fn the_result_frame_is_the_reply_and_the_calls() {
        let mut out = Completion::default();
        let line = r#"{"type":"result","subtype":"success","is_error":false,
            "stop_reason":"end_turn",
            "structured_output":{"say":"told Pat","calls":[
                {"name":"send_message","arguments":{"to":"Pat","body":"green"}}]},
            "usage":{"input_tokens":10,"cache_creation_input_tokens":100,
                     "cache_read_input_tokens":1000,"output_tokens":5},
            "total_cost_usd":0.42}"#;

        assert!(fold(line, &mut out, &mut sink()).unwrap());
        assert_eq!(out.content, "told Pat");
        assert_eq!(out.finish_reason.as_deref(), Some("end_turn"));

        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].name, "send_message");
        // Re-serialized, because the runtime parses this string back.
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&out.tool_calls[0].arguments).unwrap(),
            serde_json::json!({ "to": "Pat", "body": "green" })
        );

        let usage = out.usage.unwrap();
        // Every token that was read, cached or not. All three were processed.
        assert_eq!(usage.prompt_tokens, 1110);
        assert_eq!(usage.completion_tokens, 5);
        // The price it quoted is not money that moved. `ProgramUsage::tally`.
        assert_eq!(usage.cost, None);
    }

    #[test]
    fn two_calls_in_one_round_do_not_share_an_id() {
        // The history pairs a result to a call by id, across the whole turn and
        // not just this round. Two the same is round two's result answering
        // round one's call.
        let mut out = Completion::default();
        let line = r#"{"type":"result","subtype":"success","is_error":false,
            "structured_output":{"say":"","calls":[
                {"name":"a","arguments":{}},{"name":"b","arguments":{}}]}}"#;
        assert!(fold(line, &mut out, &mut sink()).unwrap());
        assert_ne!(out.tool_calls[0].id, out.tool_calls[1].id);
    }

    #[test]
    fn a_line_that_is_not_json_does_not_fail_a_call_that_may_still_answer() {
        let mut out = Completion::default();
        assert!(!fold("Warning: something happened", &mut out, &mut sink()).unwrap());
    }

    #[test]
    fn a_refusal_carries_the_programs_own_words() {
        // With no endpoint and no status code they are the only account of the
        // failure there is, and the usual cause is a sign-in only the operator
        // can fix.
        let mut out = Completion::default();
        let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,
            "result":"Credit balance is too low"}"#;

        let err = fold(line, &mut out, &mut sink()).unwrap_err();
        assert!(matches!(err, LlmError::ProgramFailed { .. }));
        assert!(err.to_string().contains("Credit balance is too low"), "{err}");
    }

    #[test]
    fn an_upstream_status_is_reported_as_one_so_a_bad_minute_is_retried() {
        let mut out = Completion::default();
        let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,
            "api_error_status":529,"result":"overloaded"}"#;

        let err = fold(line, &mut out, &mut sink()).unwrap_err();
        assert!(matches!(err, LlmError::Upstream { status: 529, .. }), "{err}");
        // The whole reason to classify it rather than fold it into the words.
        assert!(err.is_transient());
    }

    #[test]
    fn a_success_that_answered_in_the_wrong_shape_is_a_failure_and_says_so() {
        // The case `docs/CODING.md` cost an afternoon over, in the other
        // harness: a failed turn reported inside a stream that exits zero. Read
        // by exit code alone it is a turn that had nothing to say.
        let mut out = Completion::default();
        let line = r#"{"type":"result","subtype":"success","is_error":false,"result":"hello"}"#;

        let err = fold(line, &mut out, &mut sink()).unwrap_err();
        assert!(err.to_string().contains("without answering in the requested shape"), "{err}");
        assert!(!err.is_transient());
    }

    #[test]
    fn the_argument_vector_switches_the_operators_own_setup_off() {
        // Measured, not assumed: with these off, one trivial reply cost 104,371
        // input tokens and named 200-odd of the operator's own tools. Each pair
        // is asserted adjacently because a flag whose value landed elsewhere is
        // a different command line.
        let argv = argv(&schema(&[spec("send_message")]), "you are Dana");
        let pair = |flag: &str| {
            let at = argv.iter().position(|a| a == flag).unwrap_or_else(|| panic!("no {flag}"));
            argv[at + 1].clone()
        };

        assert_eq!(pair("--system-prompt"), "you are Dana");
        assert_eq!(pair("--tools"), "");
        assert_eq!(pair("--setting-sources"), "");
        assert_eq!(pair("--mcp-config"), r#"{"mcpServers":{}}"#);
        assert!(argv.iter().any(|a| a == "--strict-mcp-config"));
        assert!(argv.iter().any(|a| a == "--disable-slash-commands"));
        assert!(argv.iter().any(|a| a == "--no-session-persistence"));

        // The model belongs to the program. `MODEL_LABEL` is the argument.
        assert!(!argv.iter().any(|a| a == "--model"), "{argv:?}");

        // Each of these is refused by the program without the one after it.
        assert_eq!(pair("--input-format"), "stream-json");
        assert_eq!(pair("--output-format"), "stream-json");
        assert!(argv.iter().any(|a| a == "--verbose"));

        // The schema is what holds the answer to a shape this app can dispatch.
        let schema: serde_json::Value = serde_json::from_str(&pair("--json-schema")).unwrap();
        assert_eq!(
            schema["properties"]["calls"]["items"]["oneOf"][0]["properties"]["name"]["const"],
            "send_message"
        );
    }
}
