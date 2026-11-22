use std::collections::BTreeMap;
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::config::InferenceConfig;
use crate::llm::openrouter::{
    ChatMessage, ChatRequest, Completion, ContentPart, LlmError, Token, ToolCall, ToolSpec, Usage,
    UserContent, THOUGHT_BREAK,
};
use crate::llm::sse::SseDecoder;
use crate::subscription::{SigninError, Subscription};

pub const DEFAULT_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";

const ORIGINATOR: &str = "codex_cli_rs";

const CATALOG_CLIENT_VERSION: &str = "999.0.0";

pub const DEFAULT_MODEL: &str = "gpt-5.6-luna";

pub async fn models(subscription: &Subscription) -> Result<Vec<SubscriptionModel>, LlmError> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut access = subscription.access().await.map_err(auth_error)?;
        let url = format!("{}/models", subscription.backend().trim_end_matches('/'));
        let http = reqwest::Client::builder()
            .user_agent(concat!("guac/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|source| LlmError::Transport { url: url.clone(), source })?;
        let mut renewed = false;
        loop {
            let mut request = http
                .get(&url)
                .query(&[("client_version", CATALOG_CLIENT_VERSION)])
                .bearer_auth(access.token.expose())
                .header("originator", ORIGINATOR);
            if !access.account_id.is_empty() {
                request = request.header("ChatGPT-Account-Id", &access.account_id);
            }
            let response = request
                .send()
                .await
                .map_err(|source| LlmError::Transport { url: url.clone(), source })?;
            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED && !renewed {
                renewed = true;
                access = subscription.renew(access.token.expose()).await.map_err(auth_error)?;
                continue;
            }
            if !status.is_success() {
                return Err(LlmError::Upstream {
                    status: status.as_u16(),
                    message: "Could not load ChatGPT models. Reopen the provider panel to retry."
                        .into(),
                });
            }
            let catalog = response
                .json::<ModelCatalog>()
                .await
                .map_err(|err| LlmError::Decode(format!("ChatGPT model catalog: {err}")))?;
            return catalog.visible();
        }
    })
    .await
    .map_err(|_| LlmError::Timeout { secs: 10 })?
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionModel {
    pub slug: String,
    pub default_reasoning_effort: Option<crate::domain::effort::ReasoningEffort>,
    pub reasoning_efforts: Vec<EffortPreset>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffortPreset {
    pub effort: crate::domain::effort::ReasoningEffort,
    pub description: String,
}

#[derive(Deserialize)]
struct ModelCatalog {
    models: Vec<CatalogModel>,
}

#[derive(Deserialize)]
struct CatalogModel {
    slug: String,
    visibility: String,
    #[serde(default)]
    priority: i64,
    #[serde(default)]
    default_reasoning_level: Option<crate::domain::effort::ReasoningEffort>,
    #[serde(default)]
    supported_reasoning_levels: Vec<EffortPreset>,
}

impl ModelCatalog {
    fn visible(mut self) -> Result<Vec<SubscriptionModel>, LlmError> {
        self.models.sort_by_key(|model| model.priority);
        let mut models: Vec<SubscriptionModel> = Vec::new();
        for model in self.models {
            if model.visibility == "list"
                && !model.slug.trim().is_empty()
                && !models.iter().any(|entry| entry.slug == model.slug)
            {
                models.push(SubscriptionModel {
                    slug: model.slug,
                    default_reasoning_effort: model.default_reasoning_level,
                    reasoning_efforts: model.supported_reasoning_levels,
                });
            }
        }
        if models.is_empty() {
            return Err(LlmError::Decode(
                "ChatGPT returned no selectable models. Reopen the provider panel to retry.".into(),
            ));
        }
        Ok(models)
    }
}

#[derive(Debug, Serialize)]
struct Request<'a> {
    model: &'a str,
    instructions: String,
    input: Vec<Item>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Tool<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    parallel_tool_calls: bool,
    store: bool,
    stream: bool,
    reasoning: Reasoning,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct Reasoning {
    #[serde(skip_serializing_if = "Option::is_none")]
    effort: Option<crate::domain::effort::ReasoningEffort>,
    summary: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Tool<'a> {
    Function(Function<'a>),
    Hosted {
        #[serde(rename = "type")]
        kind: &'static str,
    },
}

#[derive(Debug, Serialize)]
struct Function<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
    strict: bool,
}

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

#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Part {
    InputText { text: String },
    OutputText { text: String },
    InputImage { image_url: String },
}

fn build<'a>(
    request: &'a ChatRequest,
    model: &'a str,
    effort: crate::domain::effort::ReasoningEffort,
) -> Request<'a> {
    let mut instructions: Vec<&str> = Vec::new();
    let mut input: Vec<Item> = Vec::new();
    let mut owner = None;

    for message in &request.messages {
        match message {
            ChatMessage::System { content } => {
                instructions.push(content);
                owner = owner.or(content.owner());
            }

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
            .map(|tool: &ToolSpec| {
                Tool::Function(Function {
                    kind: "function",
                    name: &tool.name,
                    description: &tool.description,
                    parameters: &tool.parameters,
                    strict: false,
                })
            })
            .collect(),
        tool_choice: if request.tools.is_empty() { None } else { Some("auto") },
        parallel_tool_calls: false,
        store: false,
        stream: true,
        reasoning: Reasoning { summary: "auto", effort: effort.wire() },
        prompt_cache_key: owner,
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Event {
    #[serde(rename = "response.output_text.delta")]
    TextDelta { delta: String },
    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningDelta {
        delta: String,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        summary_index: u32,
    },
    #[serde(rename = "response.output_item.done")]
    ItemDone { item: OutputItem },
    #[serde(rename = "response.completed")]
    Completed { response: Envelope },
    #[serde(rename = "response.failed")]
    Failed { response: Envelope },
    #[serde(rename = "response.incomplete")]
    Incomplete { response: Envelope },
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

#[derive(Debug, Default, Deserialize)]
struct ResponseUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    input_tokens_details: Option<InputDetails>,
}

#[derive(Debug, Default, Deserialize)]
struct InputDetails {
    #[serde(default)]
    cached_tokens: u32,
}

impl From<ResponseUsage> for Usage {
    fn from(counted: ResponseUsage) -> Self {
        Usage {
            prompt_tokens: counted.input_tokens,
            completion_tokens: counted.output_tokens,
            cached_tokens: counted.input_tokens_details.map_or(0, |d| d.cached_tokens),
            cost: None,
        }
    }
}

pub(super) async fn stream<F>(
    http: &reqwest::Client,
    subscription: &Subscription,
    cfg: &InferenceConfig,
    request: &ChatRequest,
    on_token: &mut F,
) -> Result<Completion, LlmError>
where
    F: FnMut(Token<'_>),
{
    let body = build(request, &request.model, cfg.reasoning_effort);
    send(http, subscription, cfg, &body, on_token).await
}

fn searching<'a>(
    request: &'a ChatRequest,
    effort: crate::domain::effort::ReasoningEffort,
) -> Request<'a> {
    let mut body = build(request, &request.model, effort);
    body.tools.push(Tool::Hosted { kind: "web_search" });
    body.tool_choice = Some("auto");
    body
}

pub(super) async fn search(
    http: &reqwest::Client,
    subscription: &Subscription,
    cfg: &InferenceConfig,
    model: &str,
    query: &str,
    count: usize,
) -> Result<crate::llm::search::Searched, LlmError> {
    let request = ChatRequest {
        model: model.to_string(),
        messages: vec![
            ChatMessage::system(crate::llm::search::SEARCHER),
            ChatMessage::user(crate::llm::search::ask(query, count)),
        ],
        tools: Vec::new(),
        temperature: None,
    };
    let body = searching(&request, cfg.reasoning_effort);
    let completion = send(http, subscription, cfg, &body, &mut |_| {}).await?;
    Ok(crate::llm::search::Searched {
        hits: crate::llm::search::kept(crate::llm::search::listed(&completion.content), count),
        usage: completion.usage,
    })
}

async fn send<F>(
    http: &reqwest::Client,
    subscription: &Subscription,
    cfg: &InferenceConfig,
    body: &Request<'_>,
    on_token: &mut F,
) -> Result<Completion, LlmError>
where
    F: FnMut(Token<'_>),
{
    let mut access = subscription.access().await.map_err(auth_error)?;

    let url = format!("{}/responses", subscription.backend().trim_end_matches('/'));
    let timeout = Duration::from_secs(cfg.request_timeout_secs.clamp(5, 900));

    let mut renewed = false;
    loop {
        let mut send = http
            .post(&url)
            .bearer_auth(access.token.expose())
            .header("originator", ORIGINATOR)
            .header("Accept", "text/event-stream")
            .timeout(timeout)
            .json(body);

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
        if status.is_success() {
            return consume(response, &url, timeout, on_token).await;
        }

        if status == reqwest::StatusCode::UNAUTHORIZED && !renewed {
            renewed = true;
            access = subscription.renew(access.token.expose()).await.map_err(auth_error)?;
            continue;
        }

        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let text = response.text().await.unwrap_or_default();
        return Err(classify(status.as_u16(), &text, body.model, retry_after));
    }
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
    let mut calls: BTreeMap<usize, ToolCall> = BTreeMap::new();
    let mut usage = None;
    let mut settled = false;
    let mut part: Option<(String, u32)> = None;

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
                Event::ReasoningDelta { delta, item_id, summary_index } => {
                    if !delta.is_empty() {
                        let this = (item_id, summary_index);
                        if part.as_ref().is_some_and(|last| *last != this) {
                            on_token(Token::Reasoning(THOUGHT_BREAK));
                        }
                        part = Some(this);
                        on_token(Token::Reasoning(&delta));
                    }
                }
                Event::ItemDone { item } => {
                    if let OutputItem::FunctionCall { call_id, id, name, arguments } = item {
                        let next = calls.len();
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

    if !settled && content.is_empty() && tool_calls.is_empty() {
        return Err(LlmError::Truncated);
    }

    Ok(Completion {
        content,
        finish_reason: Some(if tool_calls.is_empty() { "stop" } else { "tool_calls" }.to_string()),
        tool_calls,
        usage,
    })
}

fn auth_error(err: SigninError) -> LlmError {
    match err {
        SigninError::Transport { url, source } => LlmError::Transport { url, source },
        other => LlmError::SubscriptionRejected { message: other.to_string() },
    }
}

fn classify(status: u16, body: &str, model: &str, retry_after: Option<u64>) -> LlmError {
    let message = detail(body);
    match status {
        401 | 403 => LlmError::SubscriptionRejected { message },
        429 => LlmError::RateLimited { message, retry_after_secs: retry_after },
        400 | 404 if message.to_lowercase().contains("model") => {
            LlmError::ModelRejected { model: model.to_string(), message }
        }
        _ => LlmError::Upstream { status, message },
    }
}

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
    fn a_search_asks_for_the_hosted_web_search_tool_and_stores_nothing() {
        let request = ChatRequest {
            model: "gpt-5".into(),
            messages: vec![ChatMessage::system("search"), ChatMessage::user("lease or buy")],
            tools: Vec::new(),
            temperature: None,
        };
        let body = wire(&searching(&request, Default::default()));
        assert_eq!(body["tools"], serde_json::json!([{ "type": "web_search" }]));
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["store"], false);
    }

    #[test]
    fn a_system_message_becomes_instructions_rather_than_input() {
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::system("be terse"), ChatMessage::user("hello")],
            tools: Vec::new(),
            temperature: Some(0.7),
        };
        let json = wire(&build(&request, "m", Default::default()));

        assert_eq!(json["instructions"], "be terse");
        assert_eq!(json["input"].as_array().unwrap().len(), 1, "the system turn is not an input");
        assert_eq!(json["input"][0]["role"], "user");
        assert_eq!(json["input"][0]["content"][0]["type"], "input_text");
    }

    #[test]
    fn a_turn_names_its_agent_as_the_cache_key_and_a_one_off_call_names_nobody() {
        use crate::llm::openrouter::SystemPrompt;
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![
                ChatMessage::system(SystemPrompt::new(
                    "You are Pip.".into(),
                    " It is Tuesday.",
                    Some("agent-7".into()),
                )),
                ChatMessage::user("hello"),
            ],
            tools: Vec::new(),
            temperature: None,
        };
        let json = wire(&build(&request, "m", Default::default()));
        assert_eq!(json["prompt_cache_key"], "agent-7", "{json}");
        assert_eq!(
            json["instructions"], "You are Pip. It is Tuesday.",
            "one string, stable part first"
        );

        let probe = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::system("Reply with the single word: ok")],
            tools: Vec::new(),
            temperature: None,
        };
        let json = wire(&build(&probe, "m", Default::default()));
        assert!(json.get("prompt_cache_key").is_none(), "{json}");
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
        assert_eq!(
            wire(&build(&request, "m", Default::default()))["instructions"],
            "first\n\nsecond"
        );
    }

    #[test]
    fn a_temperature_is_never_sent() {
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::system("s"), ChatMessage::user("u")],
            tools: Vec::new(),
            temperature: Some(0.0),
        };
        let json = wire(&build(&request, "m", Default::default()));
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
        let json = wire(&build(&request, "m", Default::default()));
        assert_eq!(json["store"], false);
        assert_eq!(json["stream"], true);
        assert_eq!(json["parallel_tool_calls"], false);
        assert_eq!(json["reasoning"]["summary"], "auto");
        assert!(json["reasoning"].get("effort").is_none());
        for effort in ["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"] {
            let chosen = serde_json::from_value(serde_json::json!(effort)).unwrap();
            let explicit = wire(&build(&request, "m", chosen));
            assert_eq!(explicit["reasoning"]["effort"], effort);
            assert_eq!(explicit["reasoning"]["summary"], "auto");
        }
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
        let json = wire(&build(&request, "m", Default::default()));

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
        let json = wire(&build(&request, "m", Default::default()));
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
        let json = wire(&build(&request, "m", Default::default()));
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
        let input = wire(&build(&request, "m", Default::default()));
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
        let json = wire(&build(&request, "m", Default::default()));
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
        let json = wire(&build(&request, "m", Default::default()));
        let content = &json["input"][0]["content"];

        assert_eq!(content[0]["type"], "input_text", "the text goes first");
        assert_eq!(content[1]["type"], "input_image");
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
        let json = wire(&build(&request, "m", Default::default()));
        assert_eq!(json["input"][0]["content"][0]["image_url"], "data:image/jpeg;base64,BB");
    }

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
        assert_eq!(usage.cost, None);
        assert_eq!(usage.cached_tokens, 0, "the frame says nothing about a cache");
    }

    #[tokio::test]
    async fn cached_input_is_read_as_a_share_of_the_input() {
        let (result, _) = decode(
            &[
                r#"data: {"type":"response.output_text.delta","delta":"Hi"}"#,
                r#"data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":9000,"input_tokens_details":{"cached_tokens":8704},"output_tokens":3}}}"#,
            ],
            usize::MAX,
        )
        .await;
        let usage = result.unwrap().usage.unwrap();
        assert_eq!((usage.prompt_tokens, usage.cached_tokens), (9000, 8704));
        assert!(usage.cached_tokens <= usage.prompt_tokens);
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
    async fn each_summary_part_starts_on_a_line_of_its_own() {
        let (result, seen) = decode(
            &[
                r#"data: {"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":0,"delta":"**Updating skill "}"#,
                r#"data: {"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":0,"delta":"memory**"}"#,
                r#"data: {"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":1,"delta":"**Checking suppression and site**"}"#,
                r#"data: {"type":"response.reasoning_summary_text.delta","item_id":"rs_2","summary_index":0,"delta":"**Sending outreach**"}"#,
                r#"data: {"type":"response.completed","response":{}}"#,
            ],
            usize::MAX,
        )
        .await;
        result.unwrap();

        let thought: String =
            seen.iter().filter_map(|token| token.strip_prefix("think:")).collect();
        assert_eq!(
            thought,
            "**Updating skill memory**\n\n**Checking suppression and site**\n\n**Sending outreach**"
        );
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
        let (result, _) = decode(
            &[r#"data: {"type":"response.output_text.delta","delta":"half a"}"#],
            usize::MAX,
        )
        .await;
        assert_eq!(result.unwrap().content, "half a");
    }

    #[test]
    fn a_rejected_sign_in_is_not_reported_as_a_bad_api_key() {
        let err = classify(401, r#"{"detail":"invalid token"}"#, "m", None);
        match err {
            LlmError::SubscriptionRejected { message } => assert_eq!(message, "invalid token"),
            other => panic!("a 401 here means sign in again, got {other:?}"),
        }
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
        assert_eq!(
            detail(r#"{"detail":"Unsupported parameter: temperature"}"#),
            "Unsupported parameter: temperature"
        );
        assert_eq!(
            detail(r#"{"error":{"message":"bad image","type":"invalid_request_error"}}"#),
            "bad image"
        );
        assert_eq!(detail("<html>gateway</html>"), "<html>gateway</html>");
        assert_eq!(detail("   "), "empty error body");
    }

    #[test]
    fn a_sign_in_that_cannot_be_reached_stays_transient() {
        let refused = auth_error(SigninError::TimedOut);
        assert!(!refused.is_transient());
    }
}
