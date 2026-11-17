use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::config::InferenceConfig;
use crate::llm::openrouter::{
    ChatMessage, ChatRequest, Completion, ContentPart, LlmError, Token, ToolCall, ToolSpec, Usage,
    UserContent, THOUGHT_BREAK,
};

pub const PROGRAM: &str = "claude";

pub const INSTALL: &str = "npm install -g @anthropic-ai/claude-code";

pub const MODEL_LABEL: &str = "claude";

const SAY: &str = "say";
const CALLS: &str = "calls";

const REFUSED: &str = "refusal";

pub fn schema(tools: &[ToolSpec]) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        SAY.to_string(),
        serde_json::json!({
            "type": "string",
            "description": "What you are saying this round. Leave it empty while you \
                            are still working, and in your last round when your instructions say \
                            to reply with nothing.",
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

pub fn conversation(messages: &[ChatMessage]) -> (String, Vec<serde_json::Value>) {
    let mut system = Vec::new();
    let mut blocks: Vec<serde_json::Value> = Vec::new();
    let mut names: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();

    let text = |blocks: &mut Vec<serde_json::Value>, body: String| {
        if !body.trim().is_empty() {
            blocks.push(serde_json::json!({ "type": "text", "text": body }));
        }
    };

    for message in messages {
        match message {
            ChatMessage::System { content } => system.push(content.to_string()),
            ChatMessage::User { content } => match content {
                UserContent::Text(body) => text(&mut blocks, body.clone()),
                UserContent::Parts(parts) => {
                    for part in parts {
                        match part {
                            ContentPart::Text { text: body } => text(&mut blocks, body.clone()),
                            ContentPart::ImageUrl { image_url } => {
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
    #[serde(default)]
    result: Option<String>,
}

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
    fn tally(&self) -> Usage {
        Usage {
            prompt_tokens: self
                .input_tokens
                .saturating_add(self.cache_creation_input_tokens)
                .saturating_add(self.cache_read_input_tokens),
            cached_tokens: self.cache_read_input_tokens,
            completion_tokens: self.output_tokens,
            cost: None,
        }
    }
}

#[derive(Debug, Default)]
struct Seam {
    message: u32,
    last: Option<(u32, u64)>,
}

fn fold<F>(
    line: &str,
    out: &mut Completion,
    seam: &mut Seam,
    on_token: &mut F,
) -> Result<bool, LlmError>
where
    F: FnMut(Token<'_>),
{
    let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
        return Ok(false);
    };

    match event.get("type").and_then(serde_json::Value::as_str) {
        Some("stream_event") => {
            let Some(inner) = event.get("event") else { return Ok(false) };
            if inner.get("type").and_then(serde_json::Value::as_str) == Some("message_start") {
                seam.message += 1;
            }
            if let Some(delta) = inner.get("delta") {
                let kind = delta.get("type").and_then(serde_json::Value::as_str);
                let text = match kind {
                    Some("thinking_delta") => delta.get("thinking"),
                    Some("text_delta") => delta.get("text"),
                    _ => None,
                };
                if let Some(text) =
                    text.and_then(serde_json::Value::as_str).filter(|text| !text.is_empty())
                {
                    let block = inner.get("index").and_then(serde_json::Value::as_u64);
                    let here = (seam.message, block.unwrap_or(0));
                    if seam.last.is_some_and(|last| last != here) {
                        on_token(Token::Reasoning(THOUGHT_BREAK));
                    }
                    seam.last = Some(here);
                    on_token(Token::Reasoning(text));
                }
            }
            Ok(false)
        }
        Some("result") => {
            let frame: ResultFrame = serde_json::from_value(event)
                .map_err(|err| LlmError::Decode(format!("{PROGRAM} result frame: {err}")))?;

            if frame.is_error || frame.structured_output.is_none() {
                return Err(failure(&frame));
            }

            let answer = frame.structured_output.unwrap_or_default();
            out.content = answer.say;
            out.tool_calls = answer
                .calls
                .into_iter()
                .enumerate()
                .map(|(index, call)| ToolCall {
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

fn failure(frame: &ResultFrame) -> LlmError {
    let said = frame
        .result
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("it reported no reason");

    if frame.stop_reason.as_deref() == Some(REFUSED) {
        return LlmError::ModelRefused { program: PROGRAM, message: said.to_string() };
    }

    if let Some(status) = frame.api_error_status.as_ref().and_then(|s| s.as_u64()) {
        return LlmError::Upstream { status: status as u16, message: said.to_string() };
    }

    match frame.subtype.as_str() {
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

fn scratch() -> Option<PathBuf> {
    let at = std::env::temp_dir().join("guaca-claude");
    std::fs::create_dir_all(&at).ok()?;
    Some(at)
}

pub async fn presence() -> Option<String> {
    let asked = tokio::process::Command::new(PROGRAM)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let answered = tokio::time::timeout(Duration::from_secs(10), asked).await.ok()?.ok()?;
    answered.status.success().then(|| String::from_utf8_lossy(&answered.stdout).trim().to_string())
}

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
    if let Some(at) = scratch() {
        command.current_dir(at);
    }

    let mut child = command
        .args(argv(&schema, &system))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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

    let mut stdin = child.stdin.take();
    let writer = tokio::spawn(async move {
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(payload.as_bytes()).await;
            let _ = stdin.write_all(b"\n").await;
            let _ = stdin.flush().await;
        }
        drop(stdin);
    });

    let stdout = child.stdout.take().ok_or(LlmError::ProgramFailed {
        program: PROGRAM,
        message: "the program produced no output stream".to_string(),
    })?;
    let stderr = child.stderr.take();

    let mut completion = Completion::default();
    let mut seam = Seam::default();
    let mut answered = false;

    let read = async {
        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = lines.next_line().await.map_err(|err| {
            LlmError::Decode(format!("could not read what {PROGRAM} wrote: {err}"))
        })? {
            if fold(&line, &mut completion, &mut seam, on_token)? {
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
        assert_eq!(variants[0]["properties"]["arguments"]["required"][0], "to");
    }

    #[test]
    fn a_turn_with_no_tools_is_offered_no_calls_field() {
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
        assert_eq!(blocks[0]["text"], "the build is green");
    }

    #[test]
    fn a_tool_result_is_written_under_the_name_of_the_tool_that_ran() {
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

        let mut seam = Seam::default();
        for line in [
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"two things"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"both are independent"}}}"#,
        ] {
            assert!(!fold(line, &mut out, &mut seam, &mut pen).unwrap());
        }

        assert_eq!(seen, ["two things", THOUGHT_BREAK, "both are independent"]);
        assert!(out.content.is_empty());
    }

    #[test]
    fn a_block_is_one_paragraph_however_many_pieces_it_arrives_in() {
        let mut seen = String::new();
        let mut out = Completion::default();
        let mut seam = Seam::default();
        let mut pen = |token: Token<'_>| {
            if let Token::Reasoning(text) = token {
                seen.push_str(text);
            }
        };

        for line in [
            r#"{"type":"stream_event","event":{"type":"message_start","message":{}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"reading "}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"the inbox."}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":""}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_start","message":{}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Now the reply."}}}"#,
        ] {
            assert!(!fold(line, &mut out, &mut seam, &mut pen).unwrap());
        }

        assert_eq!(seen, "reading the inbox.\n\nNow the reply.");
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

        assert!(fold(line, &mut out, &mut Seam::default(), &mut sink()).unwrap());
        assert_eq!(out.content, "told Pat");
        assert_eq!(out.finish_reason.as_deref(), Some("end_turn"));

        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].name, "send_message");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&out.tool_calls[0].arguments).unwrap(),
            serde_json::json!({ "to": "Pat", "body": "green" })
        );

        let usage = out.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 1110);
        assert_eq!(usage.completion_tokens, 5);
        assert_eq!(usage.cost, None);
    }

    #[test]
    fn a_cache_read_is_kept_apart_and_still_counted_in_the_prompt() {
        let usage: ProgramUsage = serde_json::from_value(serde_json::json!({
            "input_tokens": 12,
            "cache_creation_input_tokens": 300,
            "cache_read_input_tokens": 9000,
            "output_tokens": 40
        }))
        .unwrap();
        let tally = usage.tally();
        assert_eq!(
            tally.prompt_tokens, 9312,
            "everything the model read, as on the other two paths"
        );
        assert_eq!(tally.cached_tokens, 9000, "of which this much came out of the cache");
        assert!(tally.cached_tokens <= tally.prompt_tokens);
        assert_eq!(ProgramUsage::default().tally().cached_tokens, 0);
    }

    #[test]
    fn two_calls_in_one_round_do_not_share_an_id() {
        let mut out = Completion::default();
        let line = r#"{"type":"result","subtype":"success","is_error":false,
            "structured_output":{"say":"","calls":[
                {"name":"a","arguments":{}},{"name":"b","arguments":{}}]}}"#;
        assert!(fold(line, &mut out, &mut Seam::default(), &mut sink()).unwrap());
        assert_ne!(out.tool_calls[0].id, out.tool_calls[1].id);
    }

    #[test]
    fn a_line_that_is_not_json_does_not_fail_a_call_that_may_still_answer() {
        let mut out = Completion::default();
        assert!(!fold("Warning: something happened", &mut out, &mut Seam::default(), &mut sink())
            .unwrap());
    }

    #[test]
    fn a_failure_carries_the_programs_own_words() {
        let mut out = Completion::default();
        let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,
            "result":"Credit balance is too low"}"#;

        let err = fold(line, &mut out, &mut Seam::default(), &mut sink()).unwrap_err();
        assert!(matches!(err, LlmError::ProgramFailed { .. }));
        assert!(err.to_string().contains("Credit balance is too low"), "{err}");
        assert!(!err.is_transient(), "{err}");
    }

    #[test]
    fn a_model_refusal_is_not_a_spent_plan_and_is_worth_another_draw() {
        let mut out = Completion::default();
        let line = r#"{"type":"result","subtype":"success","is_error":true,
            "api_error_status":null,"stop_reason":"refusal",
            "result":"API Error: Opus 5's safeguards flagged this message. Details: `[reasoning_extraction]` Request ID: req_011"}"#;

        let err = fold(line, &mut out, &mut Seam::default(), &mut sink()).unwrap_err();
        assert!(matches!(err, LlmError::ModelRefused { .. }), "{err}");
        assert!(err.is_transient(), "{err}");
        assert!(err.to_string().contains("reasoning_extraction"), "{err}");
        assert!(err.to_string().contains("provider"), "{err}");
    }

    #[test]
    fn a_refusal_that_also_carries_a_status_is_still_a_refusal() {
        let mut out = Completion::default();
        let line = r#"{"type":"result","subtype":"success","is_error":true,
            "api_error_status":401,"stop_reason":"refusal","result":"API Error: flagged"}"#;

        let err = fold(line, &mut out, &mut Seam::default(), &mut sink()).unwrap_err();
        assert!(matches!(err, LlmError::ModelRefused { .. }), "{err}");
    }

    #[test]
    fn an_upstream_status_is_reported_as_one_so_a_bad_minute_is_retried() {
        let mut out = Completion::default();
        let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,
            "api_error_status":529,"result":"overloaded"}"#;

        let err = fold(line, &mut out, &mut Seam::default(), &mut sink()).unwrap_err();
        assert!(matches!(err, LlmError::Upstream { status: 529, .. }), "{err}");
        assert!(err.is_transient());
    }

    #[test]
    fn a_success_that_answered_in_the_wrong_shape_is_a_failure_and_says_so() {
        let mut out = Completion::default();
        let line = r#"{"type":"result","subtype":"success","is_error":false,"result":"hello"}"#;

        let err = fold(line, &mut out, &mut Seam::default(), &mut sink()).unwrap_err();
        assert!(err.to_string().contains("without answering in the requested shape"), "{err}");
        assert!(!err.is_transient());
    }

    #[test]
    fn the_argument_vector_switches_the_operators_own_setup_off() {
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

        assert!(!argv.iter().any(|a| a == "--model"), "{argv:?}");

        assert_eq!(pair("--input-format"), "stream-json");
        assert_eq!(pair("--output-format"), "stream-json");
        assert!(argv.iter().any(|a| a == "--verbose"));

        let schema: serde_json::Value = serde_json::from_str(&pair("--json-schema")).unwrap();
        assert_eq!(
            schema["properties"]["calls"]["items"]["oneOf"][0]["properties"]["name"]["const"],
            "send_message"
        );
    }
}
