//! The Codex backend, which is where a ChatGPT subscription can be spent.
//!
//! Guaca speaks one shape of model API everywhere else: OpenAI-compatible
//! `/chat/completions`, a list of messages in and a stream of deltas out. The
//! subscription backend does not offer that endpoint. It speaks the Responses
//! API, which is a different request body, a different item model and a
//! different set of stream events for the same conversation. So this file is a
//! translation, and it is the only place in the app that knows there are two
//! protocols.
//!
//! Translating rather than teaching the runtime a second protocol is a
//! deliberate trade. It costs this file, which has to be right about both
//! shapes. It saves every other file: `runtime/mod.rs` assembles one kind of
//! request, `prompt.rs` writes one kind of history, tool results come back one
//! way, and none of them gain a branch for how the operator is paying.
//!
//! ## What the two protocols disagree about
//!
//! - **The system prompt is not a message.** It is `instructions`, a field
//!   beside the input, and the endpoint returns 400 without one.
//! - **A tool result is not a role.** Chat completions send `role: "tool"`
//!   carrying a `tool_call_id`. Responses send a `function_call_output` item
//!   carrying a `call_id`, as a sibling of the `function_call` item it answers.
//! - **A tool definition is flat.** `{type, name, description, parameters}`,
//!   not `{type, function: {...}}`. The nested form is accepted and then the
//!   model is never offered the tool, which presents as an agent that has
//!   forgotten how to do its job.
//! - **There is no temperature.** The endpoint rejects the parameter outright
//!   rather than ignoring it, so a request carrying one fails in full.
//! - **Nothing says `[DONE]`.** The stream ends on `response.completed`, and
//!   the usage arrives inside it rather than in a frame of its own.
//!
//! ## What a subscription does not report
//!
//! A price. Every other endpoint Guaca talks to either charges per token and
//! says what it charged, or is local and charges nothing. A subscription has
//! already been paid for, so the tokens are counted and the cost is `None`,
//! which is the same thing a local model reports and reads correctly everywhere
//! downstream.
//!
//! ## Reasoning is asked for and not kept
//!
//! Summaries are requested, streamed to the operator as they arrive, and
//! dropped. The encrypted reasoning blobs that would let a later round resume
//! the model's own working are deliberately *not* requested: keeping them would
//! mean persisting reasoning and sending it back, which the rest of this app
//! promises never to do. The cost is that a multi-round turn re-reasons from its
//! tool results instead of continuing. The alternative is breaking the one
//! promise `Token::Reasoning` exists to keep.

use std::collections::BTreeMap;
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::config::InferenceConfig;
use crate::llm::openrouter::{
    ChatMessage, ChatRequest, Completion, ContentPart, LlmError, Token, ToolCall, ToolSpec, Usage,
    UserContent,
};
use crate::llm::sse::SseDecoder;
use crate::subscription::{SigninError, Subscription};

/// The backend a subscription's calls go to.
///
/// Not the public API base. `api.openai.com` bills a key; this bills a plan,
/// and the two are different products at different hosts that happen to share a
/// request shape.
pub const DEFAULT_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";

/// What the backend is told is calling it.
///
/// Sent because the backend expects a Codex-family client on this endpoint and
/// answers differently without it. It is not a claim to be the CLI: Guaca's own
/// name is in the `User-Agent` beside it, which is the header that identifies
/// the software.
const ORIGINATOR: &str = "codex_cli_rs";

/// Models a subscription can actually run, most capable first.
///
/// Here so Settings can offer them and so a wrong one can be named in an error.
/// Not a restriction: the model is still a text field, and the backend is the
/// authority on what it will accept. It refuses an unknown model by name, which
/// is a better error than anything this list could produce.
pub const MODELS: &[&str] =
    &["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.5", "gpt-5.4", "gpt-5.4-mini"];

/// The model a fresh sign-in starts on.
///
/// The middle of the current family rather than the top of it: a subscription
/// has a quota measured in hours, and a crew of agents talking to each other
/// will find the ceiling faster than one person typing.
pub const DEFAULT_MODEL: &str = "gpt-5.6-luna";

// ---- request -------------------------------------------------------------

#[derive(Debug, Serialize)]
struct Request<'a> {
    model: &'a str,
    /// Required. The endpoint answers 400 without it, and "instructions" is not
    /// a word that appears in the error.
    instructions: String,
    input: Vec<Item>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Function<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    /// One at a time. Guaca's guard counts tool rounds per turn and its fanout
    /// limit is enforced inside a single `send_message`, so a model that fired
    /// three tools at once would be spending against limits that were written
    /// for a sequence.
    parallel_tool_calls: bool,
    /// Nothing is kept server-side. A conversation lives in Guaca's SQLite and
    /// is replayed in full on every call, so a stored copy is a second history
    /// that can disagree with the one on screen.
    store: bool,
    stream: bool,
    reasoning: Reasoning,
}

#[derive(Debug, Serialize)]
struct Reasoning {
    /// A readable summary of the model's working, for the Thinking line. The
    /// encrypted full reasoning is not requested: see the note at the top.
    summary: &'static str,
}

/// A tool, in the flat shape this endpoint wants.
#[derive(Debug, Serialize)]
struct Function<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
    /// Off deliberately. Strict mode requires every property to be required and
    /// `additionalProperties: false` throughout, and Guaca's tool schemas have
    /// optional arguments. On, the endpoint rejects the schema; the tool is then
    /// missing rather than lenient.
    strict: bool,
}

/// One entry in the input list.
///
/// Untagged in the sense that each variant writes its own `type`, because the
/// three are not variants of one shape: a message has a role and content parts,
/// a call has a name and an argument string, and an output has neither.
#[derive(Debug, Serialize, PartialEq)]
#[serde(untagged)]
enum Item {
    Message {
        #[serde(rename = "type")]
        kind: &'static str,
        role: &'static str,
        content: Vec<Part>,
    },
    FunctionCall {
        #[serde(rename = "type")]
        kind: &'static str,
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionOutput {
        #[serde(rename = "type")]
        kind: &'static str,
        call_id: String,
        output: String,
    },
}

impl Item {
    fn message(role: &'static str, content: Vec<Part>) -> Self {
        Item::Message { kind: "message", role, content }
    }
}

/// A piece of one message.
///
/// The spellings differ from the chat-completions ones by more than a prefix:
/// text a model wrote back is `output_text` and text going to it is
/// `input_text`, and sending the wrong one in an assistant turn is a 400 about
/// the content type rather than about the role.
#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Part {
    InputText {
        text: String,
    },
    OutputText {
        text: String,
    },
    /// A `data:` URL, exactly as a screenshot arrives. Flat here, unlike chat
    /// completions where the same field is an object with a `url` inside it.
    InputImage {
        image_url: String,
    },
}

/// Turns a Guaca request into a Responses one.
///
/// Split out from the call so it can be tested directly. Most of the ways this
/// can be wrong produce a 400 from somebody else's server, and reading the
/// assembled body is far cheaper than reading that.
fn build<'a>(request: &'a ChatRequest, model: &'a str) -> Request<'a> {
    let mut instructions: Vec<&str> = Vec::new();
    let mut input: Vec<Item> = Vec::new();

    for message in &request.messages {
        match message {
            // Hoisted out of the conversation and joined, in order. Guaca sends
            // one system message today, but a second one appended later would
            // otherwise be silently dropped rather than appended here.
            ChatMessage::System { content } => instructions.push(content),

            ChatMessage::User { content } => {
                let parts = match content {
                    UserContent::Text(text) => vec![Part::InputText { text: text.clone() }],
                    UserContent::Parts(parts) => parts
                        .iter()
                        .map(|part| match part {
                            ContentPart::Text { text } => Part::InputText { text: text.clone() },
                            ContentPart::ImageUrl { image_url } => {
                                Part::InputImage { image_url: image_url.url.clone() }
                            }
                        })
                        .collect(),
                };
                input.push(Item::message("user", parts));
            }

            ChatMessage::Assistant { content, tool_calls } => {
                // An assistant turn that said nothing and only called a tool is
                // normal, and an empty content list is not a valid message.
                if let Some(text) = content.as_deref().filter(|t| !t.is_empty()) {
                    input.push(Item::message(
                        "assistant",
                        vec![Part::OutputText { text: text.to_string() }],
                    ));
                }
                for call in tool_calls {
                    input.push(Item::FunctionCall {
                        kind: "function_call",
                        call_id: call.id.clone(),
                        name: call.function.name.clone(),
                        arguments: call.function.arguments.clone(),
                    });
                }
            }

            ChatMessage::Tool { tool_call_id, content } => input.push(Item::FunctionOutput {
                kind: "function_call_output",
                call_id: tool_call_id.clone(),
                output: content.clone(),
            }),
        }
    }

    Request {
        model,
        instructions: instructions.join("\n\n"),
        input,
        tools: request
            .tools
            .iter()
            .map(|tool: &ToolSpec| Function {
                kind: "function",
                name: &tool.name,
                description: &tool.description,
                parameters: &tool.parameters,
                strict: false,
            })
            .collect(),
        tool_choice: if request.tools.is_empty() { None } else { Some("auto") },
        parallel_tool_calls: false,
        store: false,
        stream: true,
        reasoning: Reasoning { summary: "auto" },
    }
}

// ---- stream --------------------------------------------------------------

/// One frame of the response stream.
///
/// Only the events Guaca acts on are named; everything else is `Other` and
/// ignored. The endpoint adds events over time, and a decoder that errors on an
/// unfamiliar one breaks on a Tuesday for reasons nobody changed.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Event {
    #[serde(rename = "response.output_text.delta")]
    TextDelta { delta: String },
    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningDelta { delta: String },
    /// The completed form of one output item. Authoritative for a tool call:
    /// it carries the id, the name and the whole argument string, so the
    /// argument deltas do not have to be reassembled.
    #[serde(rename = "response.output_item.done")]
    ItemDone { item: OutputItem },
    #[serde(rename = "response.completed")]
    Completed { response: Envelope },
    #[serde(rename = "response.failed")]
    Failed { response: Envelope },
    #[serde(rename = "response.incomplete")]
    Incomplete { response: Envelope },
    /// A top-level error, which can arrive after a 200 has been sent.
    #[serde(rename = "error")]
    Error {
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        code: Option<String>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum OutputItem {
    #[serde(rename = "function_call")]
    FunctionCall {
        #[serde(default)]
        call_id: String,
        #[serde(default)]
        id: String,
        name: String,
        #[serde(default)]
        arguments: String,
    },
    #[serde(other)]
    Other,
}

/// The response object, as carried by whichever event settles the turn.
///
/// The `status` field it also carries is deliberately not read: which of
/// `completed`, `failed` and `incomplete` happened is already the event's own
/// type, and consulting both invites the two disagreeing.
#[derive(Debug, Default, Deserialize)]
struct Envelope {
    #[serde(default)]
    usage: Option<ResponseUsage>,
    #[serde(default)]
    error: Option<ErrorDetail>,
    #[serde(default)]
    incomplete_details: Option<IncompleteDetails>,
}

#[derive(Debug, Deserialize)]
struct ErrorDetail {
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IncompleteDetails {
    #[serde(default)]
    reason: Option<String>,
}

/// Token counts, in this protocol's spelling.
#[derive(Debug, Default, Deserialize)]
struct ResponseUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

impl From<ResponseUsage> for Usage {
    fn from(counted: ResponseUsage) -> Self {
        Usage {
            prompt_tokens: counted.input_tokens,
            completion_tokens: counted.output_tokens,
            // A subscription has already been paid for. Reporting zero would
            // read as a free call in the usage view; `None` reads as "not
            // priced", which is the truth and what a local model reports too.
            cost: None,
        }
    }
}

/// Streams one turn against the subscription backend.
///
/// Mirrors `LlmClient::stream_chat`, including the contract on `on_token`: it
/// runs on this task and must not block.
pub async fn stream<F>(
    http: &reqwest::Client,
    subscription: &Subscription,
    cfg: &InferenceConfig,
    request: &ChatRequest,
    on_token: &mut F,
) -> Result<Completion, LlmError>
where
    F: FnMut(Token<'_>),
{
    let access = subscription.access().await.map_err(auth_error)?;

    // From the credential rather than from the constant: the account and the
    // backend that will accept it are one fact, and it is what lets the
    // end-to-end suite drive this transport against a stub.
    let url = format!("{}/responses", subscription.backend().trim_end_matches('/'));
    let timeout = Duration::from_secs(cfg.request_timeout_secs.clamp(5, 900));
    let body = build(request, &request.model);

    let mut send = http
        .post(&url)
        .bearer_auth(&access.token)
        .header("originator", ORIGINATOR)
        .header("Accept", "text/event-stream")
        .timeout(timeout)
        .json(&body);

    // Which workspace the call is billed to. Omitted rather than sent empty for
    // a personal account that has never had one: the backend rejects a blank
    // value and accepts an absent one.
    if !access.account_id.is_empty() {
        send = send.header("ChatGPT-Account-Id", &access.account_id);
    }

    let response = send.send().await.map_err(|source| {
        if source.is_timeout() {
            LlmError::Timeout { secs: timeout.as_secs() }
        } else {
            LlmError::Transport { url: url.clone(), source }
        }
    })?;

    let status = response.status();
    if !status.is_success() {
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let text = response.text().await.unwrap_or_default();
        return Err(classify(status.as_u16(), &text, &request.model, retry_after));
    }

    consume(response, &url, timeout, on_token).await
}

async fn consume<F>(
    response: reqwest::Response,
    url: &str,
    timeout: Duration,
    on_token: &mut F,
) -> Result<Completion, LlmError>
where
    F: FnMut(Token<'_>),
{
    let mut decoder = SseDecoder::new();
    let mut content = String::new();
    // Keyed by the id the endpoint gives each call, so a turn that calls two
    // tools keeps them in the order they were emitted rather than in hash order.
    let mut calls: BTreeMap<usize, ToolCall> = BTreeMap::new();
    let mut usage = None;
    let mut settled = false;

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|source| {
            if source.is_timeout() {
                LlmError::Timeout { secs: timeout.as_secs() }
            } else {
                LlmError::Transport { url: url.to_string(), source }
            }
        })?;

        let text = String::from_utf8_lossy(&bytes);
        for payload in decoder.push(&text) {
            if payload.is_empty() || payload == "[DONE]" {
                continue;
            }

            let event: Event = match serde_json::from_str(&payload) {
                Ok(event) => event,
                // One unreadable frame must not discard a response that is
                // otherwise fine, exactly as on the other transport.
                Err(err) => {
                    tracing::warn!(
                        %err,
                        payload = %payload.chars().take(200).collect::<String>(),
                        "skipping an unreadable codex stream frame"
                    );
                    continue;
                }
            };

            match event {
                Event::TextDelta { delta } => {
                    if !delta.is_empty() {
                        on_token(Token::Text(&delta));
                        content.push_str(&delta);
                    }
                }
                // Shown as it happens and accumulated nowhere, which is the
                // same rule the other transport follows.
                Event::ReasoningDelta { delta } => {
                    if !delta.is_empty() {
                        on_token(Token::Reasoning(&delta));
                    }
                }
                Event::ItemDone { item } => {
                    if let OutputItem::FunctionCall { call_id, id, name, arguments } = item {
                        let next = calls.len();
                        // `call_id` is what a result has to be filed under.
                        // Falling back to the item id keeps a call usable if
                        // only one of the two is sent, because a tool call
                        // dropped here is a turn that silently did nothing.
                        let id = if call_id.is_empty() { id } else { call_id };
                        calls.insert(
                            next,
                            ToolCall {
                                id: if id.is_empty() { format!("call_{name}") } else { id },
                                name,
                                arguments,
                            },
                        );
                    }
                }
                Event::Completed { response } => {
                    if let Some(counted) = response.usage {
                        usage = Some(counted.into());
                    }
                    settled = true;
                }
                Event::Failed { response } => {
                    return Err(LlmError::Upstream {
                        status: 200,
                        message: response
                            .error
                            .and_then(|e| e.message)
                            .unwrap_or_else(|| "the model call failed mid-stream".to_string()),
                    });
                }
                // Not a failure of the request: the model ran into a ceiling
                // partway. Whatever it produced first is kept, because a turn
                // that reached its limit having said something useful is worth
                // more than an error.
                Event::Incomplete { response } => {
                    let reason = response
                        .incomplete_details
                        .and_then(|d| d.reason)
                        .unwrap_or_else(|| "unknown".to_string());
                    if let Some(counted) = response.usage {
                        usage = Some(counted.into());
                    }
                    tracing::warn!(%reason, "a codex turn ended incomplete");
                    settled = true;
                }
                Event::Error { message, code } => {
                    return Err(LlmError::Upstream {
                        status: 200,
                        message: message.or(code).unwrap_or_else(|| {
                            "the endpoint reported an error mid-stream".to_string()
                        }),
                    });
                }
                Event::Other => {}
            }
        }
    }

    let tool_calls: Vec<ToolCall> = calls.into_values().collect();

    // Nothing said, nothing called, and no completion event: the stream was cut
    // off. Reporting that beats handing back an empty turn, which the runtime
    // would file as an agent choosing to say nothing.
    if !settled && content.is_empty() && tool_calls.is_empty() {
        return Err(LlmError::Truncated);
    }

    Ok(Completion {
        content,
        // Nothing downstream reads this beyond "the turn produced something",
        // so it is reported in the vocabulary the rest of the app already uses
        // rather than in this protocol's.
        finish_reason: Some(if tool_calls.is_empty() { "stop" } else { "tool_calls" }.to_string()),
        tool_calls,
        usage,
    })
}

/// Turns a sign-in problem into a model-call problem.
///
/// The distinction matters to the caller: `stream_with_retries` retries what is
/// transient and gives up immediately on what is not, and a sign-in that has
/// expired answers the same way every time.
fn auth_error(err: SigninError) -> LlmError {
    match err {
        SigninError::Transport { url, source } => LlmError::Transport { url, source },
        other => LlmError::SubscriptionRejected { message: other.to_string() },
    }
}

/// Maps a backend status onto the error the operator or the agent will read.
///
/// Separate from the other transport's classifier because the same status means
/// something different here. A 401 on a pasted key means the key is wrong; a 401
/// here means a sign-in has to be repeated, and telling the operator to check
/// their API key sends them to a field they have not used.
fn classify(status: u16, body: &str, model: &str, retry_after: Option<u64>) -> LlmError {
    let message = detail(body);
    match status {
        401 | 403 => LlmError::SubscriptionRejected { message },
        429 => LlmError::RateLimited { message, retry_after_secs: retry_after },
        // The backend refuses an unsupported model by name, and that sentence
        // is the most useful thing an operator can be shown, so it is passed
        // through rather than replaced.
        400 | 404 if message.to_lowercase().contains("model") => {
            LlmError::ModelRejected { model: model.to_string(), message }
        }
        _ => LlmError::Upstream { status, message },
    }
}

/// Pulls the readable part out of an error body.
///
/// This endpoint reports a rejected request as a bare `{"detail": "..."}` and a
/// rejected parameter as the usual `{"error": {"message": ...}}`. Reading only
/// one of the two leaves the most common failure, a model the plan cannot use,
/// showing as a wall of JSON.
fn detail(body: &str) -> String {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let found = parsed.as_ref().and_then(|v| {
        v.get("detail")
            .and_then(|d| d.as_str())
            .or_else(|| v.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()))
            .or_else(|| v.get("message").and_then(|m| m.as_str()))
    });
    match found {
        Some(message) if !message.trim().is_empty() => message.to_string(),
        _ => {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                "empty error body".to_string()
            } else {
                trimmed.chars().take(400).collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::llm::openrouter::{ImageSource, WireFunction, WireToolCall};

    fn spec() -> ToolSpec {
        ToolSpec {
            name: "send_message".into(),
            description: "Send a message.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "to": { "type": "string" } },
                "required": ["to"],
            }),
        }
    }

    fn wire(body: &Request<'_>) -> serde_json::Value {
        serde_json::to_value(body).unwrap()
    }

    #[test]
    fn a_system_message_becomes_instructions_rather_than_input() {
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::system("be terse"), ChatMessage::user("hello")],
            tools: Vec::new(),
            temperature: Some(0.7),
        };
        let json = wire(&build(&request, "m"));

        assert_eq!(json["instructions"], "be terse");
        assert_eq!(json["input"].as_array().unwrap().len(), 1, "the system turn is not an input");
        assert_eq!(json["input"][0]["role"], "user");
        assert_eq!(json["input"][0]["content"][0]["type"], "input_text");
    }

    #[test]
    fn several_system_messages_are_joined_in_order() {
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![
                ChatMessage::system("first"),
                ChatMessage::system("second"),
                ChatMessage::user("go"),
            ],
            tools: Vec::new(),
            temperature: None,
        };
        assert_eq!(wire(&build(&request, "m"))["instructions"], "first\n\nsecond");
    }

    #[test]
    fn a_temperature_is_never_sent() {
        // The endpoint rejects the parameter rather than ignoring it, so one
        // leaking through fails every turn of every agent.
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::system("s"), ChatMessage::user("u")],
            tools: Vec::new(),
            temperature: Some(0.0),
        };
        let json = wire(&build(&request, "m"));
        assert!(json.get("temperature").is_none(), "{json}");
    }

    #[test]
    fn nothing_is_stored_server_side_and_the_stream_is_asked_for() {
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::user("u")],
            tools: Vec::new(),
            temperature: None,
        };
        let json = wire(&build(&request, "m"));
        assert_eq!(json["store"], false);
        assert_eq!(json["stream"], true);
        assert_eq!(json["parallel_tool_calls"], false);
        assert_eq!(json["reasoning"]["summary"], "auto");
        // The encrypted reasoning would have to be sent back to be worth
        // asking for, and this app does not keep reasoning.
        assert!(json.get("include").is_none(), "{json}");
    }

    #[test]
    fn a_tool_is_flat_rather_than_nested_under_function() {
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::user("u")],
            tools: vec![spec()],
            temperature: None,
        };
        let json = wire(&build(&request, "m"));

        assert_eq!(json["tools"][0]["type"], "function");
        assert_eq!(json["tools"][0]["name"], "send_message", "{json}");
        assert!(
            json["tools"][0].get("function").is_none(),
            "the nested shape is accepted and then the tool is never offered"
        );
        assert_eq!(json["tools"][0]["strict"], false);
        assert_eq!(json["tool_choice"], "auto");
    }

    #[test]
    fn no_tools_means_no_tool_choice() {
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::user("u")],
            tools: Vec::new(),
            temperature: None,
        };
        let json = wire(&build(&request, "m"));
        assert!(json.get("tools").is_none());
        assert!(json.get("tool_choice").is_none());
    }

    #[test]
    fn a_tool_result_is_filed_under_the_call_it_answers() {
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![
                ChatMessage::user("what is the weather"),
                ChatMessage::Assistant {
                    content: None,
                    tool_calls: vec![WireToolCall {
                        id: "call_1".into(),
                        kind: "function".into(),
                        function: WireFunction {
                            name: "get_weather".into(),
                            arguments: r#"{"city":"Paris"}"#.into(),
                        },
                    }],
                },
                ChatMessage::Tool { tool_call_id: "call_1".into(), content: "18C".into() },
            ],
            tools: Vec::new(),
            temperature: None,
        };
        let json = wire(&build(&request, "m"));
        let input = json["input"].as_array().unwrap();

        assert_eq!(input.len(), 3);
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["call_id"], "call_1");
        assert_eq!(input[1]["name"], "get_weather");
        assert_eq!(input[1]["arguments"], r#"{"city":"Paris"}"#);

        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["call_id"], "call_1", "a result filed elsewhere answers nothing");
        assert_eq!(input[2]["output"], "18C");
        assert!(input[2].get("role").is_none(), "a tool result is not a role here");
    }

    #[test]
    fn an_assistant_turn_that_only_called_a_tool_sends_no_empty_message() {
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::Assistant {
                content: Some(String::new()),
                tool_calls: vec![WireToolCall {
                    id: "call_1".into(),
                    kind: "function".into(),
                    function: WireFunction { name: "t".into(), arguments: "{}".into() },
                }],
            }],
            tools: Vec::new(),
            temperature: None,
        };
        let input = wire(&build(&request, "m"));
        let input = input["input"].as_array().unwrap();
        assert_eq!(input.len(), 1, "an empty content list is not a valid message");
        assert_eq!(input[0]["type"], "function_call");
    }

    #[test]
    fn what_an_assistant_said_is_output_text_not_input_text() {
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::assistant("I looked it up")],
            tools: Vec::new(),
            temperature: None,
        };
        let json = wire(&build(&request, "m"));
        assert_eq!(json["input"][0]["role"], "assistant");
        assert_eq!(json["input"][0]["content"][0]["type"], "output_text");
    }

    #[test]
    fn a_screenshot_becomes_an_input_image_with_a_flat_url() {
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::user_seeing(
                "what is on screen",
                "data:image/png;base64,AA",
            )],
            tools: Vec::new(),
            temperature: None,
        };
        let json = wire(&build(&request, "m"));
        let content = &json["input"][0]["content"];

        assert_eq!(content[0]["type"], "input_text", "the text goes first");
        assert_eq!(content[1]["type"], "input_image");
        // Flat, unlike chat completions where this is `{ url: ... }`.
        assert_eq!(content[1]["image_url"], "data:image/png;base64,AA");
    }

    #[test]
    fn an_image_part_is_recognized_however_it_was_built() {
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::User {
                content: UserContent::Parts(vec![ContentPart::ImageUrl {
                    image_url: ImageSource { url: "data:image/jpeg;base64,BB".into() },
                }]),
            }],
            tools: Vec::new(),
            temperature: None,
        };
        let json = wire(&build(&request, "m"));
        assert_eq!(json["input"][0]["content"][0]["image_url"], "data:image/jpeg;base64,BB");
    }

    // ---- stream decoding -------------------------------------------------

    /// Feeds frames through the real decoder, as one body split at `at` bytes.
    ///
    /// Split deliberately: the bugs worth catching in a stream decoder live at
    /// chunk boundaries, and a test that hands it one whole string never sees
    /// them.
    async fn decode(frames: &[&str], at: usize) -> (Result<Completion, LlmError>, Vec<String>) {
        let body: String = frames.iter().map(|f| format!("{f}\n\n")).collect();
        let (head, tail) = body.split_at(at.min(body.len()));

        let app = axum::Router::new().route(
            "/responses",
            axum::routing::post({
                let head = head.to_string();
                let tail = tail.to_string();
                move || {
                    let head = head.clone();
                    let tail = tail.clone();
                    async move {
                        let chunks = futures_util::stream::iter(vec![
                            Ok::<_, std::io::Error>(head),
                            Ok(tail),
                        ]);
                        axum::response::Response::builder()
                            .header("content-type", "text/event-stream")
                            .body(axum::body::Body::from_stream(chunks))
                            .unwrap()
                    }
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let response =
            reqwest::Client::new().post(format!("http://{addr}/responses")).send().await.unwrap();

        let mut seen = Vec::new();
        let result = consume(response, "test", Duration::from_secs(5), &mut |token| match token {
            Token::Text(t) => seen.push(format!("text:{t}")),
            Token::Reasoning(t) => seen.push(format!("think:{t}")),
        })
        .await;
        (result, seen)
    }

    const TEXT_TURN: &[&str] = &[
        r#"data: {"type":"response.created","response":{}}"#,
        r#"data: {"type":"response.output_text.delta","delta":"Hello"}"#,
        r#"data: {"type":"response.output_text.delta","delta":" there"}"#,
        r#"data: {"type":"response.output_text.done","text":"Hello there"}"#,
        r#"data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":12,"output_tokens":3}}}"#,
    ];

    #[tokio::test]
    async fn text_deltas_are_streamed_and_accumulated() {
        let (result, seen) = decode(TEXT_TURN, usize::MAX).await;
        let completion = result.unwrap();

        assert_eq!(completion.content, "Hello there");
        assert_eq!(seen, vec!["text:Hello", "text: there"]);
        assert_eq!(completion.finish_reason.as_deref(), Some("stop"));
    }

    #[tokio::test]
    async fn a_split_frame_is_still_decoded() {
        // Every byte boundary, so a frame cut inside its JSON, inside its
        // `data:` prefix and between its newlines are all covered.
        for at in 0..240 {
            let (result, _) = decode(TEXT_TURN, at).await;
            assert_eq!(result.unwrap().content, "Hello there", "split at {at}");
        }
    }

    #[tokio::test]
    async fn usage_is_read_out_of_the_completion_event_and_carries_no_price() {
        let (result, _) = decode(TEXT_TURN, usize::MAX).await;
        let usage = result.unwrap().usage.expect("the counts are on the last frame");

        assert_eq!(usage.prompt_tokens, 12);
        assert_eq!(usage.completion_tokens, 3);
        // Not zero: a subscription call is not a free call, it is an unpriced
        // one, and zero would read as free in the usage view.
        assert_eq!(usage.cost, None);
    }

    #[tokio::test]
    async fn a_tool_call_is_taken_whole_from_the_finished_item() {
        let (result, _) = decode(
            &[
                r#"data: {"type":"response.output_item.added","item":{"type":"function_call","call_id":"call_9","name":"send_message","arguments":""}}"#,
                r#"data: {"type":"response.function_call_arguments.delta","delta":"{\"to\":"}"#,
                r#"data: {"type":"response.function_call_arguments.delta","delta":"\"bob\"}"}"#,
                r#"data: {"type":"response.output_item.done","item":{"type":"function_call","call_id":"call_9","name":"send_message","arguments":"{\"to\":\"bob\"}"}}"#,
                r#"data: {"type":"response.completed","response":{"status":"completed"}}"#,
            ],
            usize::MAX,
        )
        .await;
        let completion = result.unwrap();

        assert_eq!(completion.tool_calls.len(), 1, "the deltas must not double the call");
        let call = &completion.tool_calls[0];
        assert_eq!(call.id, "call_9");
        assert_eq!(call.name, "send_message");
        assert_eq!(call.arguments, r#"{"to":"bob"}"#);
        assert_eq!(completion.finish_reason.as_deref(), Some("tool_calls"));
    }

    #[tokio::test]
    async fn two_tool_calls_keep_the_order_they_arrived_in() {
        let (result, _) = decode(
            &[
                r#"data: {"type":"response.output_item.done","item":{"type":"function_call","call_id":"a","name":"first","arguments":"{}"}}"#,
                r#"data: {"type":"response.output_item.done","item":{"type":"function_call","call_id":"b","name":"second","arguments":"{}"}}"#,
                r#"data: {"type":"response.completed","response":{}}"#,
            ],
            usize::MAX,
        )
        .await;
        let names: Vec<_> = result.unwrap().tool_calls.into_iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["first", "second"]);
    }

    #[tokio::test]
    async fn a_finished_message_item_is_not_mistaken_for_a_tool_call() {
        let (result, _) = decode(
            &[
                r#"data: {"type":"response.output_text.delta","delta":"hi"}"#,
                r#"data: {"type":"response.output_item.done","item":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}}"#,
                r#"data: {"type":"response.completed","response":{}}"#,
            ],
            usize::MAX,
        )
        .await;
        let completion = result.unwrap();
        assert_eq!(completion.content, "hi");
        assert!(completion.tool_calls.is_empty());
    }

    #[tokio::test]
    async fn reasoning_is_streamed_and_never_accumulated() {
        let (result, seen) = decode(
            &[
                r#"data: {"type":"response.reasoning_summary_text.delta","delta":"weighing it"}"#,
                r#"data: {"type":"response.output_text.delta","delta":"answer"}"#,
                r#"data: {"type":"response.completed","response":{}}"#,
            ],
            usize::MAX,
        )
        .await;
        let completion = result.unwrap();

        assert_eq!(seen, vec!["think:weighing it", "text:answer"]);
        assert_eq!(completion.content, "answer", "reasoning must not reach the transcript");
    }

    #[tokio::test]
    async fn an_unfamiliar_event_is_ignored_rather_than_failing_the_turn() {
        let (result, _) = decode(
            &[
                r#"data: {"type":"response.something.invented.later","payload":{"x":1}}"#,
                r#"data: {"type":"response.output_text.delta","delta":"fine"}"#,
                r#"data: {"type":"response.completed","response":{}}"#,
            ],
            usize::MAX,
        )
        .await;
        assert_eq!(result.unwrap().content, "fine");
    }

    #[tokio::test]
    async fn an_unreadable_frame_is_skipped_rather_than_discarding_the_turn() {
        let (result, _) = decode(
            &[
                r#"data: {"type":"response.output_text.delta","delta":"kept"}"#,
                r#"data: {not json at all"#,
                r#"data: {"type":"response.completed","response":{}}"#,
            ],
            usize::MAX,
        )
        .await;
        assert_eq!(result.unwrap().content, "kept");
    }

    #[tokio::test]
    async fn a_failure_after_the_headers_is_reported_with_its_reason() {
        let (result, _) = decode(
            &[
                r#"data: {"type":"response.output_text.delta","delta":"partial"}"#,
                r#"data: {"type":"response.failed","response":{"status":"failed","error":{"message":"the model gave up"}}}"#,
            ],
            usize::MAX,
        )
        .await;
        match result {
            Err(LlmError::Upstream { message, .. }) => assert_eq!(message, "the model gave up"),
            other => panic!("expected an upstream error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_mid_stream_error_event_is_reported() {
        let (result, _) =
            decode(&[r#"data: {"type":"error","message":"quota exhausted"}"#], usize::MAX).await;
        match result {
            Err(LlmError::Upstream { message, .. }) => assert_eq!(message, "quota exhausted"),
            other => panic!("expected an upstream error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_incomplete_turn_keeps_what_it_managed_to_say() {
        let (result, _) = decode(
            &[
                r#"data: {"type":"response.output_text.delta","delta":"as far as I got"}"#,
                r#"data: {"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":5,"output_tokens":9}}}"#,
            ],
            usize::MAX,
        )
        .await;
        let completion = result.unwrap();
        assert_eq!(completion.content, "as far as I got");
        assert_eq!(completion.usage.unwrap().completion_tokens, 9);
    }

    #[tokio::test]
    async fn a_stream_that_ends_having_said_nothing_is_reported_as_truncated() {
        let (result, _) =
            decode(&[r#"data: {"type":"response.created","response":{}}"#], usize::MAX).await;
        assert!(matches!(result, Err(LlmError::Truncated)), "got {result:?}");
    }

    #[tokio::test]
    async fn a_turn_cut_off_after_saying_something_keeps_it() {
        // No completion event, but there is text: the operator should see what
        // arrived rather than an error about a stream they cannot inspect.
        let (result, _) = decode(
            &[r#"data: {"type":"response.output_text.delta","delta":"half a"}"#],
            usize::MAX,
        )
        .await;
        assert_eq!(result.unwrap().content, "half a");
    }

    // ---- error classification --------------------------------------------

    #[test]
    fn a_rejected_sign_in_is_not_reported_as_a_bad_api_key() {
        let err = classify(401, r#"{"detail":"invalid token"}"#, "m", None);
        match err {
            LlmError::SubscriptionRejected { message } => assert_eq!(message, "invalid token"),
            other => panic!("a 401 here means sign in again, got {other:?}"),
        }
        // And it is not worth retrying: it answers the same way every time.
        assert!(!LlmError::SubscriptionRejected { message: "x".into() }.is_transient());
    }

    #[test]
    fn a_model_the_plan_cannot_use_says_which_model() {
        let err = classify(
            400,
            r#"{"detail":"The 'gpt-5.6-codex' model is not supported when using Codex with a ChatGPT account."}"#,
            "gpt-5.6-codex",
            None,
        );
        match err {
            LlmError::ModelRejected { model, message } => {
                assert_eq!(model, "gpt-5.6-codex");
                assert!(message.contains("not supported"), "{message}");
            }
            other => panic!("expected a model rejection, got {other:?}"),
        }
    }

    #[test]
    fn a_rate_limit_keeps_the_retry_after() {
        match classify(429, "{}", "m", Some(30)) {
            LlmError::RateLimited { retry_after_secs, .. } => {
                assert_eq!(retry_after_secs, Some(30))
            }
            other => panic!("expected a rate limit, got {other:?}"),
        }
    }

    #[test]
    fn both_error_body_shapes_are_read() {
        // What this endpoint sends for a refused request.
        assert_eq!(
            detail(r#"{"detail":"Unsupported parameter: temperature"}"#),
            "Unsupported parameter: temperature"
        );
        // And what it sends for a refused parameter.
        assert_eq!(
            detail(r#"{"error":{"message":"bad image","type":"invalid_request_error"}}"#),
            "bad image"
        );
        // Anything else is shown as it arrived, truncated.
        assert_eq!(detail("<html>gateway</html>"), "<html>gateway</html>");
        assert_eq!(detail("   "), "empty error body");
    }

    #[test]
    fn a_sign_in_that_cannot_be_reached_stays_transient() {
        // A network problem reaching the sign-in service is worth retrying; a
        // refused sign-in is not, and collapsing the two makes a turn retry
        // three times against a credential that will never work.
        let refused = auth_error(SigninError::TimedOut);
        assert!(!refused.is_transient());
    }
}
