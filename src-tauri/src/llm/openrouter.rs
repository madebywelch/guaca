use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::config::{InferenceConfig, Provider};
use crate::llm::claude;
use crate::llm::codex;
use crate::llm::sse::SseDecoder;
use crate::subscription::Subscription;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum ChatMessage {
    System {
        content: SystemPrompt,
    },
    User {
        content: UserContent,
    },
    Assistant {
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<WireToolCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UserContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageSource },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageSource {
    pub url: String,
}

impl ChatMessage {
    pub fn user_seeing(text: impl Into<String>, image_data_url: impl Into<String>) -> Self {
        ChatMessage::User {
            content: UserContent::Parts(vec![
                ContentPart::Text { text: text.into() },
                ContentPart::ImageUrl { image_url: ImageSource { url: image_data_url.into() } },
            ]),
        }
    }

    pub fn system(content: impl Into<SystemPrompt>) -> Self {
        ChatMessage::System { content: content.into() }
    }

    pub fn user(content: impl Into<String>) -> Self {
        ChatMessage::User { content: UserContent::Text(content.into()) }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        ChatMessage::Assistant { content: Some(content.into()), tool_calls: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SystemPrompt {
    text: String,
    stable: usize,
    owner: Option<String>,
}

impl SystemPrompt {
    pub fn new(stable: String, volatile: &str, owner: Option<String>) -> Self {
        let at = stable.len();
        let mut text = stable;
        text.push_str(volatile);
        Self { text, stable: at, owner }
    }

    pub fn stable(&self) -> &str {
        &self.text[..self.stable]
    }

    pub fn volatile(&self) -> &str {
        &self.text[self.stable..]
    }

    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }

    pub fn extend_stable(&mut self, more: &str) {
        self.text.insert_str(self.stable, more);
        self.stable += more.len();
    }

    pub fn edit_volatile(&mut self, edit: impl FnOnce(&mut String)) {
        let mut volatile = self.text.split_off(self.stable);
        edit(&mut volatile);
        self.text.push_str(&volatile);
    }
}

impl std::fmt::Display for SystemPrompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::ops::Deref for SystemPrompt {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

impl From<String> for SystemPrompt {
    fn from(text: String) -> Self {
        Self { stable: text.len(), text, owner: None }
    }
}

impl From<&str> for SystemPrompt {
    fn from(text: &str) -> Self {
        text.to_string().into()
    }
}

impl Serialize for SystemPrompt {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for SystemPrompt {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(String::deserialize(deserializer)?.into())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: WireFunction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl Serialize for ToolSpec {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Function<'a> {
            name: &'a str,
            description: &'a str,
            parameters: &'a serde_json::Value,
        }
        #[derive(Serialize)]
        struct Wrapper<'a> {
            #[serde(rename = "type")]
            kind: &'static str,
            function: Function<'a>,
        }
        Wrapper {
            kind: "function",
            function: Function {
                name: &self.name,
                description: &self.description,
                parameters: &self.parameters,
            },
        }
        .serialize(serializer)
    }
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    pub temperature: Option<f32>,
}

#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    messages: WireMessages<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<&'a str>,
    #[serde(skip_serializing_if = "<[ToolSpec]>::is_empty")]
    tools: &'a [ToolSpec],
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

struct WireMessages<'a> {
    messages: &'a [ChatMessage],
    mark_stable: bool,
}

impl Serialize for WireMessages<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;

        #[derive(Serialize)]
        struct Part<'a> {
            #[serde(rename = "type")]
            kind: &'static str,
            text: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            cache_control: Option<serde_json::Value>,
        }
        #[derive(Serialize)]
        struct System<'a> {
            role: &'static str,
            content: Vec<Part<'a>>,
        }

        let mut seq = serializer.serialize_seq(Some(self.messages.len()))?;
        for message in self.messages {
            match message {
                ChatMessage::System { content }
                    if self.mark_stable && !content.stable().is_empty() =>
                {
                    let mut parts = vec![Part {
                        kind: "text",
                        text: content.stable(),
                        cache_control: Some(serde_json::json!({ "type": "ephemeral" })),
                    }];
                    if !content.volatile().is_empty() {
                        parts.push(Part {
                            kind: "text",
                            text: content.volatile(),
                            cache_control: None,
                        });
                    }
                    seq.serialize_element(&System { role: "system", content: parts })?;
                }
                other => seq.serialize_element(other)?,
            }
        }
        seq.end()
    }
}

fn wire<'a>(cfg: &InferenceConfig, request: &'a ChatRequest) -> WireRequest<'a> {
    let openrouter = reqwest::Url::parse(&cfg.base_url)
        .is_ok_and(|url| url.origin().ascii_serialization() == "https://openrouter.ai");
    let owner = request.messages.iter().find_map(|message| match message {
        ChatMessage::System { content } => content.owner(),
        _ => None,
    });
    WireRequest {
        model: &request.model,
        messages: WireMessages {
            messages: &request.messages,
            mark_stable: caches_only_when_told(&request.model),
        },
        session_id: owner.filter(|_| openrouter),
        tools: &request.tools,
        tool_choice: if request.tools.is_empty() { None } else { Some("auto") },
        stream: true,
        stream_options: Some(StreamOptions { include_usage: true }),
        temperature: request.temperature,
    }
}

fn caches_only_when_told(model: &str) -> bool {
    model.starts_with("anthropic/")
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token<'a> {
    Text(&'a str),
    Reasoning(&'a str),
}

pub const THOUGHT_BREAK: &str = "\n\n";

impl ToolCall {
    pub fn parsed_arguments(&self) -> Result<serde_json::Value, serde_json::Error> {
        let trimmed = self.arguments.trim();
        if trimmed.is_empty() {
            return Ok(serde_json::json!({}));
        }
        serde_json::from_str(trimmed)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Completion {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    #[serde(default, rename = "prompt_tokens_details", deserialize_with = "cached_of")]
    pub cached_tokens: u32,
    #[serde(default)]
    pub cost: Option<f64>,
}

fn cached_of<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    #[derive(Deserialize)]
    struct Details {
        #[serde(default)]
        cached_tokens: Option<u32>,
    }
    Ok(Option::<Details>::deserialize(deserializer)?.and_then(|d| d.cached_tokens).unwrap_or(0))
}

impl Completion {
    pub fn to_wire_tool_calls(&self) -> Vec<WireToolCall> {
        self.tool_calls
            .iter()
            .map(|tc| WireToolCall {
                id: tc.id.clone(),
                kind: "function".to_string(),
                function: WireFunction { name: tc.name.clone(), arguments: tc.arguments.clone() },
            })
            .collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("no API key is configured. Open Settings and paste an OpenRouter key.")]
    NotConfigured,
    #[error("the inference endpoint rejected the API key (HTTP {status}): {message}")]
    Auth { status: u16, message: String },
    #[error(
        "the ChatGPT subscription could not be used: {message}. Open Settings and sign in again."
    )]
    SubscriptionRejected { message: String },
    #[error("{program} is not installed, or is not on this app's PATH. Install it with `{install}`, or pick another provider in Settings.", install = crate::llm::claude::INSTALL)]
    ProgramMissing { program: &'static str },
    #[error("{program} could not answer: {message}")]
    ProgramFailed { program: &'static str, message: String },
    #[error(
        "{program} was refused by the model: {message}\n\nThat is the model's own safety check \
         rather than your sign-in or your plan, and it runs on the answer rather than on what \
         you asked for, so it can fire on an ordinary request. Reword the message, or give this \
         group another provider in its settings."
    )]
    ModelRefused { program: &'static str, message: String },
    #[error("rate limited by the inference endpoint: {message}")]
    RateLimited { message: String, retry_after_secs: Option<u64> },
    #[error("model {model:?} was rejected: {message}")]
    ModelRejected { model: String, message: String },
    #[error("inference endpoint returned HTTP {status}: {message}")]
    Upstream { status: u16, message: String },
    #[error("could not reach the inference endpoint at {url}: {source}")]
    Transport {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("inference request timed out after {secs}s")]
    Timeout { secs: u64 },
    #[error("could not decode the response stream: {0}")]
    Decode(String),
    #[error("the response stream ended mid-message")]
    Truncated,
}

impl LlmError {
    pub fn is_transient(&self) -> bool {
        match self {
            LlmError::RateLimited { .. } | LlmError::Timeout { .. } | LlmError::Truncated => true,
            LlmError::Upstream { status, .. } => *status >= 500 || *status == 200,
            LlmError::ProgramMissing { .. } | LlmError::ProgramFailed { .. } => false,
            LlmError::ModelRefused { .. } => true,
            LlmError::Transport { .. } => true,
            _ => false,
        }
    }

    pub fn headline(&self) -> String {
        match self {
            LlmError::NotConfigured => "no API key configured".into(),
            LlmError::Auth { .. } => "API key rejected".into(),
            LlmError::SubscriptionRejected { .. } => "sign in to ChatGPT again".into(),
            LlmError::ProgramMissing { program } => format!("{program} could not be found"),
            LlmError::ProgramFailed { program, .. } => format!("{program} could not answer"),
            LlmError::ModelRefused { program, .. } => format!("{program} was refused by the model"),
            LlmError::RateLimited { .. } => "rate limited".into(),
            LlmError::ModelRejected { model, .. } => format!("model {model} unavailable"),
            LlmError::Upstream { status, .. } => format!("upstream HTTP {status}"),
            LlmError::Transport { .. } => "could not reach endpoint".into(),
            LlmError::Timeout { secs } => format!("timed out after {secs}s"),
            LlmError::Decode(_) => "malformed response".into(),
            LlmError::Truncated => "stream ended early".into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    error: ApiErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    #[serde(default)]
    message: String,
    #[serde(default)]
    code: Option<serde_json::Value>,
}

fn extract_message(body: &str) -> String {
    match serde_json::from_str::<ApiErrorBody>(body) {
        Ok(parsed) if !parsed.error.message.is_empty() => parsed.error.message,
        Ok(parsed) => match parsed.error.code {
            Some(code) => format!("error code {code}"),
            None => body.chars().take(400).collect(),
        },
        Err(_) => {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                "empty error body".to_string()
            } else {
                trimmed.chars().take(400).collect()
            }
        }
    }
}

fn classify_status(status: u16, body: &str, model: &str, retry_after: Option<u64>) -> LlmError {
    let message = extract_message(body);
    match status {
        401 | 403 => LlmError::Auth { status, message },
        429 => LlmError::RateLimited { message, retry_after_secs: retry_after },
        400 | 404 if message.to_lowercase().contains("model") => {
            LlmError::ModelRejected { model: model.to_string(), message }
        }
        _ => LlmError::Upstream { status, message },
    }
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    error: Option<ApiErrorDetail>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<DeltaToolCall>,
}

impl Delta {
    fn thinking(&self) -> Option<&str> {
        [self.reasoning.as_deref(), self.reasoning_content.as_deref()]
            .into_iter()
            .flatten()
            .find(|fragment| !fragment.is_empty())
    }
}

#[derive(Debug, Deserialize)]
struct DeltaToolCall {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<DeltaFunction>,
}

#[derive(Debug, Deserialize)]
struct DeltaFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Default)]
struct ToolCallAccumulator {
    calls: BTreeMap<usize, ToolCall>,
}

impl ToolCallAccumulator {
    fn absorb(&mut self, delta: DeltaToolCall) {
        let entry = self.calls.entry(delta.index).or_insert_with(|| ToolCall {
            id: String::new(),
            name: String::new(),
            arguments: String::new(),
        });
        if let Some(id) = delta.id {
            if !id.is_empty() {
                entry.id = id;
            }
        }
        if let Some(function) = delta.function {
            if let Some(name) = function.name {
                if !name.is_empty() {
                    entry.name = name;
                }
            }
            if let Some(arguments) = function.arguments {
                entry.arguments.push_str(&arguments);
            }
        }
    }

    fn finish(self) -> Vec<ToolCall> {
        self.calls
            .into_values()
            .filter(|c| !c.name.is_empty())
            .map(|mut c| {
                if c.id.is_empty() {
                    c.id = format!("call_{}", c.name);
                }
                c
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    subscription: Option<Arc<Subscription>>,
}

impl LlmClient {
    pub fn new() -> Result<Self, LlmError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("guac/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|source| LlmError::Transport { url: "client".into(), source })?;
        Ok(Self { http, subscription: None })
    }

    pub fn with_subscription(mut self, subscription: Arc<Subscription>) -> Self {
        self.subscription = Some(subscription);
        self
    }

    pub async fn stream_chat<F>(
        &self,
        cfg: &InferenceConfig,
        request: &ChatRequest,
        mut on_token: F,
    ) -> Result<Completion, LlmError>
    where
        F: FnMut(Token<'_>),
    {
        if !cfg.is_ready() {
            return Err(LlmError::NotConfigured);
        }

        if cfg.provider == Provider::Chatgpt {
            let Some(subscription) = &self.subscription else {
                return Err(LlmError::SubscriptionRejected {
                    message: "this build has no sign-in store".to_string(),
                });
            };
            return codex::stream(&self.http, subscription, cfg, request, &mut on_token).await;
        }

        if cfg.provider == Provider::Claude {
            return claude::stream(cfg, request, &mut on_token).await;
        }

        let url = cfg.chat_completions_url();
        let timeout = Duration::from_secs(cfg.request_timeout_secs.clamp(5, 900));

        let body = wire(cfg, request);

        let response = self
            .http
            .post(&url)
            .bearer_auth(cfg.api_key.expose().trim())
            .header("HTTP-Referer", &cfg.referer)
            .header("X-Title", &cfg.title)
            .header("Accept", "text/event-stream")
            .timeout(timeout)
            .json(&body)
            .send()
            .await
            .map_err(|source| {
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
            return Err(classify_status(status.as_u16(), &text, &request.model, retry_after));
        }

        self.consume_stream(response, &url, timeout, &mut on_token).await
    }

    async fn consume_stream<F>(
        &self,
        response: reqwest::Response,
        url: &str,
        timeout: Duration,
        on_token: &mut F,
    ) -> Result<Completion, LlmError>
    where
        F: FnMut(Token<'_>),
    {
        let mut decoder = SseDecoder::new();
        let mut accumulator = ToolCallAccumulator::default();
        let mut content = String::new();
        let mut finish_reason = None;
        let mut saw_done = false;
        let mut usage = None;

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
                if payload == "[DONE]" {
                    saw_done = true;
                    continue;
                }
                if payload.is_empty() {
                    continue;
                }

                let parsed: StreamChunk = match serde_json::from_str(&payload) {
                    Ok(parsed) => parsed,
                    Err(err) => {
                        tracing::warn!(%err, payload = %truncate(&payload, 200), "skipping unparseable stream frame");
                        continue;
                    }
                };

                if let Some(error) = parsed.error {
                    return Err(LlmError::Upstream {
                        status: 200,
                        message: if error.message.is_empty() {
                            "the provider reported an error mid-stream".into()
                        } else {
                            error.message
                        },
                    });
                }

                if let Some(counted) = parsed.usage {
                    usage = Some(counted);
                }

                for choice in parsed.choices {
                    if let Some(fragment) = choice.delta.thinking() {
                        on_token(Token::Reasoning(fragment));
                    }
                    if let Some(fragment) = choice.delta.content {
                        if !fragment.is_empty() {
                            on_token(Token::Text(&fragment));
                            content.push_str(&fragment);
                        }
                    }
                    for call in choice.delta.tool_calls {
                        accumulator.absorb(call);
                    }
                    if let Some(reason) = choice.finish_reason {
                        finish_reason = Some(reason);
                    }
                }
            }
        }

        let tool_calls = accumulator.finish();

        if !saw_done && content.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
            return Err(LlmError::Truncated);
        }

        Ok(Completion { content, tool_calls, finish_reason, usage })
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    value.chars().take(max).collect::<String>() + "..."
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::Router;

    async fn stub(
        handler: impl Fn(serde_json::Value) -> axum::response::Response + Clone + Send + Sync + 'static,
    ) -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorder = seen.clone();

        let app = Router::new().route(
            "/v1/chat/completions",
            post(move |body: axum::extract::Json<serde_json::Value>| {
                let handler = handler.clone();
                let recorder = recorder.clone();
                async move {
                    recorder.lock().unwrap().push(body.0.clone());
                    handler(body.0)
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        (format!("http://{addr}/v1"), seen)
    }

    fn sse(frames: &[&str]) -> axum::response::Response {
        let body = frames.iter().map(|f| format!("data: {f}\n\n")).collect::<String>();
        ([("content-type", "text/event-stream")], body).into_response()
    }

    fn sse_owned(frames: &[String]) -> axum::response::Response {
        let refs: Vec<&str> = frames.iter().map(|s| s.as_str()).collect();
        sse(&refs)
    }

    fn cfg(base_url: String) -> InferenceConfig {
        InferenceConfig {
            base_url,
            api_key: "sk-test".into(),
            default_model: "test/model".into(),
            request_timeout_secs: 10,
            ..Default::default()
        }
    }

    fn request() -> ChatRequest {
        ChatRequest {
            model: "test/model".into(),
            messages: vec![ChatMessage::user("hello")],
            tools: Vec::new(),
            temperature: None,
        }
    }

    fn text_frame(content: &str) -> String {
        serde_json::json!({ "choices": [{ "delta": { "content": content } }] }).to_string()
    }

    fn thinking_frame(field: &str, content: &str) -> String {
        serde_json::json!({ "choices": [{ "delta": { field: content } }] }).to_string()
    }

    fn collect<'a>(tokens: &'a mut Vec<(&'static str, String)>) -> impl FnMut(Token<'_>) + 'a {
        |token| match token {
            Token::Text(text) => tokens.push(("text", text.to_string())),
            Token::Reasoning(text) => tokens.push(("reasoning", text.to_string())),
        }
    }

    #[tokio::test]
    async fn streams_text_and_reports_tokens_in_order() {
        let (base, _) = stub(|_| {
            sse(&[&text_frame("Hel"), &text_frame("lo, "), &text_frame("world"), "[DONE]"])
        })
        .await;

        let client = LlmClient::new().unwrap();
        let mut tokens = Vec::new();
        let completion =
            client.stream_chat(&cfg(base), &request(), collect(&mut tokens)).await.unwrap();

        assert_eq!(completion.content, "Hello, world");
        assert_eq!(
            tokens,
            vec![("text", "Hel".into()), ("text", "lo, ".into()), ("text", "world".into())],
            "tokens must surface incrementally"
        );
    }

    #[tokio::test]
    async fn reasoning_surfaces_apart_from_the_answer_and_never_joins_it() {
        let (base, _) = stub(|_| {
            sse(&[
                &thinking_frame("reasoning", "17 times 20 is 340"),
                &thinking_frame("reasoning", ", plus 51"),
                &text_frame("391"),
                "[DONE]",
            ])
        })
        .await;

        let client = LlmClient::new().unwrap();
        let mut tokens = Vec::new();
        let completion =
            client.stream_chat(&cfg(base), &request(), collect(&mut tokens)).await.unwrap();

        assert_eq!(
            tokens,
            vec![
                ("reasoning", "17 times 20 is 340".into()),
                ("reasoning", ", plus 51".into()),
                ("text", "391".into()),
            ]
        );
        assert_eq!(completion.content, "391", "reasoning is not part of what was said");
    }

    #[tokio::test]
    async fn reasoning_is_read_under_either_spelling() {
        for field in ["reasoning", "reasoning_content"] {
            let frame = thinking_frame(field, "weighing it up");
            let (base, _) = stub(move |_| sse(&[&frame, &text_frame("yes"), "[DONE]"])).await;

            let client = LlmClient::new().unwrap();
            let mut tokens = Vec::new();
            client.stream_chat(&cfg(base), &request(), collect(&mut tokens)).await.unwrap();
            assert_eq!(
                tokens.first(),
                Some(&("reasoning", "weighing it up".to_string())),
                "{field} was not read"
            );
        }
    }

    #[tokio::test]
    async fn a_frame_carrying_both_spellings_reports_the_thought_once() {
        let frame = serde_json::json!({"choices":[{"delta":{
            "reasoning": "checking the total",
            "reasoning_content": "checking the total",
        }}]})
        .to_string();
        let (base, _) = stub(move |_| sse(&[&frame, &text_frame("done"), "[DONE]"])).await;

        let client = LlmClient::new().unwrap();
        let mut tokens = Vec::new();
        client.stream_chat(&cfg(base), &request(), collect(&mut tokens)).await.unwrap();
        assert_eq!(
            tokens,
            vec![("reasoning", "checking the total".into()), ("text", "done".into())]
        );
    }

    #[tokio::test]
    async fn a_stream_that_only_ever_reasoned_is_still_truncated() {
        let (base, _) =
            stub(|_| sse(&[&thinking_frame("reasoning", "still working it out")])).await;

        let client = LlmClient::new().unwrap();
        let err = client.stream_chat(&cfg(base), &request(), |_| {}).await.unwrap_err();
        assert!(matches!(err, LlmError::Truncated), "got {err:?}");
    }

    #[tokio::test]
    async fn asks_for_token_counts_and_reads_them_from_the_last_frame() {
        let (base, seen) = stub(|_| {
            sse(&[
                &text_frame("done"),
                &serde_json::json!({
                    "choices": [],
                    "usage": {"prompt_tokens": 1204, "completion_tokens": 88, "total_tokens": 1292}
                })
                .to_string(),
                "[DONE]",
            ])
        })
        .await;

        let client = LlmClient::new().unwrap();
        let completion = client.stream_chat(&cfg(base), &request(), |_| {}).await.unwrap();

        let usage = completion.usage.expect("the provider counted, so this build reads it");
        assert_eq!(usage.prompt_tokens, 1204);
        assert_eq!(usage.completion_tokens, 88);

        let body = seen.lock().unwrap()[0].clone();
        assert_eq!(
            body["stream_options"]["include_usage"],
            serde_json::json!(true),
            "a streamed call reports nothing at all without this: {body}"
        );
    }

    fn turn(model: &str) -> ChatRequest {
        ChatRequest {
            model: model.into(),
            messages: vec![
                ChatMessage::system(SystemPrompt::new(
                    "You are Pip. The rules.".into(),
                    "\n## Your working notes\n- a minute ago, waiting on Robert\n",
                    Some("agent-7".into()),
                )),
                ChatMessage::user("[OPERATOR] hello"),
            ],
            tools: Vec::new(),
            temperature: None,
        }
    }

    #[tokio::test]
    async fn an_anthropic_model_is_told_where_the_reusable_part_of_the_prompt_ends() {
        let (base, seen) = stub(|_| sse(&[&text_frame("hi"), "[DONE]"])).await;
        let client = LlmClient::new().unwrap();
        client.stream_chat(&cfg(base), &turn("anthropic/claude-sonnet-5.5"), |_| {}).await.unwrap();

        let body = seen.lock().unwrap()[0].clone();
        let system = &body["messages"][0];
        assert_eq!(system["role"], "system", "{body}");
        assert_eq!(
            system["content"],
            serde_json::json!([
                {"type": "text", "text": "You are Pip. The rules.", "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": "\n## Your working notes\n- a minute ago, waiting on Robert\n"}
            ]),
            "the stable part carries the breakpoint and the part that moves does not: {body}"
        );
        assert_eq!(body["messages"][1]["content"], "[OPERATOR] hello", "{body}");
    }

    #[tokio::test]
    async fn any_other_model_gets_the_prompt_as_one_string_with_no_cache_marker() {
        let (base, seen) = stub(|_| sse(&[&text_frame("hi"), "[DONE]"])).await;
        let client = LlmClient::new().unwrap();
        client
            .stream_chat(&cfg(base), &turn("deepseek/deepseek-v4.1-flash"), |_| {})
            .await
            .unwrap();

        let body = seen.lock().unwrap()[0].clone();
        assert_eq!(
            body["messages"][0]["content"],
            "You are Pip. The rules.\n## Your working notes\n- a minute ago, waiting on Robert\n",
            "the same bytes as before the prompt had parts: {body}"
        );
        assert!(!body.to_string().contains("cache_control"), "{body}");
        assert!(body.get("session_id").is_none(), "this stub is not OpenRouter: {body}");
    }

    #[test]
    fn openrouter_is_given_the_agent_as_its_sticky_routing_key_and_nobody_else_is() {
        let request = turn("anthropic/claude-sonnet-5.5");
        let to = |base: &str| serde_json::to_value(wire(&cfg(base.into()), &request)).unwrap();
        assert_eq!(to("https://openrouter.ai/api/v1")["session_id"], "agent-7");
        for elsewhere in
            ["https://api.openai.com/v1", "http://localhost:1234/v1", "http://openrouter.ai/api/v1"]
        {
            assert!(to(elsewhere).get("session_id").is_none(), "{elsewhere}");
        }
        let mut anonymous = request.clone();
        anonymous.messages[0] = ChatMessage::system("Reply with the single word: ok");
        let body =
            serde_json::to_value(wire(&cfg("https://openrouter.ai/api/v1".into()), &anonymous))
                .unwrap();
        assert!(body.get("session_id").is_none(), "{body}");
    }

    #[tokio::test]
    async fn cached_prompt_tokens_are_read_as_a_share_of_the_prompt() {
        let (base, _) = stub(|_| {
            sse(&[
                &text_frame("done"),
                &serde_json::json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 10339, "completion_tokens": 60, "total_tokens": 10399,
                        "prompt_tokens_details": {"cached_tokens": 10318, "cache_write_tokens": 0}
                    }
                })
                .to_string(),
                "[DONE]",
            ])
        })
        .await;
        let client = LlmClient::new().unwrap();
        let usage =
            client.stream_chat(&cfg(base), &request(), |_| {}).await.unwrap().usage.unwrap();
        assert_eq!((usage.prompt_tokens, usage.cached_tokens), (10339, 10318));
        assert!(usage.cached_tokens <= usage.prompt_tokens);

        for details in [
            serde_json::json!(null),
            serde_json::json!({}),
            serde_json::json!({"cached_tokens": null}),
        ] {
            let usage: Usage = serde_json::from_value(serde_json::json!({
                "prompt_tokens": 5, "completion_tokens": 1, "prompt_tokens_details": details
            }))
            .unwrap();
            assert_eq!(usage.cached_tokens, 0);
        }
        let usage: Usage = serde_json::from_value(serde_json::json!({"prompt_tokens": 5})).unwrap();
        assert_eq!(usage.cached_tokens, 0);
    }

    #[test]
    fn the_stable_part_grows_in_front_of_what_moves() {
        let mut prompt = SystemPrompt::new("A.".into(), " moving", Some("me".into()));
        prompt.extend_stable(" B.");
        prompt.edit_volatile(|v| v.push_str(" more"));
        assert_eq!((prompt.stable(), prompt.volatile()), ("A. B.", " moving more"));
        assert_eq!(&*prompt, "A. B. moving more");
        assert_eq!(serde_json::to_value(&prompt).unwrap(), "A. B. moving more");
        let back: SystemPrompt = serde_json::from_value(serde_json::json!("whole")).unwrap();
        assert_eq!((back.stable(), back.volatile(), back.owner()), ("whole", "", None));
    }

    #[tokio::test]
    async fn a_provider_that_counts_nothing_reports_nothing() {
        let (base, _) = stub(|_| sse(&[&text_frame("done"), "[DONE]"])).await;
        let client = LlmClient::new().unwrap();
        let completion = client.stream_chat(&cfg(base), &request(), |_| {}).await.unwrap();
        assert!(completion.usage.is_none());
    }

    #[tokio::test]
    async fn assembles_a_tool_call_split_across_fragments() {
        let frames = vec![
            serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_1","type":"function","function":{"name":"send_message","arguments":""}}
            ]}}]})
            .to_string(),
            serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":"{\"to\":[\"Ch"}}
            ]}}]})
            .to_string(),
            serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":"ef\"],\"text\":\"hi\"}"}}
            ]}}]})
            .to_string(),
            serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}).to_string(),
            "[DONE]".to_string(),
        ];
        let (base, _) = stub(move |_| sse_owned(&frames)).await;

        let client = LlmClient::new().unwrap();
        let completion = client.stream_chat(&cfg(base), &request(), |_| {}).await.unwrap();

        assert_eq!(completion.tool_calls.len(), 1);
        let call = &completion.tool_calls[0];
        assert_eq!(call.id, "call_1");
        assert_eq!(call.name, "send_message");
        assert_eq!(
            call.parsed_arguments().unwrap(),
            serde_json::json!({"to": ["Chef"], "text": "hi"}),
            "arguments must reassemble into valid JSON"
        );
        assert_eq!(completion.finish_reason.as_deref(), Some("tool_calls"));
    }

    #[tokio::test]
    async fn assembles_parallel_tool_calls_in_index_order() {
        let frames = vec![
            serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":1,"id":"b","type":"function","function":{"name":"second","arguments":"{}"}}
            ]}}]})
            .to_string(),
            serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"a","type":"function","function":{"name":"first","arguments":"{}"}}
            ]}}]})
            .to_string(),
            "[DONE]".to_string(),
        ];
        let (base, _) = stub(move |_| sse_owned(&frames)).await;

        let client = LlmClient::new().unwrap();
        let completion = client.stream_chat(&cfg(base), &request(), |_| {}).await.unwrap();
        let names: Vec<&str> = completion.tool_calls.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["first", "second"], "index, not arrival, decides order");
    }

    #[tokio::test]
    async fn drops_nameless_tool_call_fragments() {
        let frames = vec![
            serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":"{}"}}
            ]}}]})
            .to_string(),
            "[DONE]".to_string(),
        ];
        let (base, _) = stub(move |_| sse_owned(&frames)).await;

        let client = LlmClient::new().unwrap();
        let completion = client.stream_chat(&cfg(base), &request(), |_| {}).await.unwrap();
        assert!(completion.tool_calls.is_empty(), "a nameless call must not be dispatched");
    }

    #[tokio::test]
    async fn tools_are_sent_in_openai_shape_with_auto_choice() {
        let (base, seen) = stub(|_| sse(&[&text_frame("ok"), "[DONE]"])).await;

        let mut req = request();
        req.tools = vec![ToolSpec {
            name: "directory".into(),
            description: "List agents".into(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }];

        let client = LlmClient::new().unwrap();
        client.stream_chat(&cfg(base), &req, |_| {}).await.unwrap();

        let body = seen.lock().unwrap()[0].clone();
        assert_eq!(body["stream"], true);
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "directory");
        assert!(body["tools"][0]["function"]["parameters"].is_object());
    }

    #[tokio::test]
    async fn no_tool_fields_are_sent_when_there_are_no_tools() {
        let (base, seen) = stub(|_| sse(&[&text_frame("ok"), "[DONE]"])).await;
        let client = LlmClient::new().unwrap();
        client.stream_chat(&cfg(base), &request(), |_| {}).await.unwrap();

        let body = seen.lock().unwrap()[0].clone();
        assert!(body.get("tools").is_none(), "an empty tools array confuses some providers");
        assert!(body.get("tool_choice").is_none());
    }

    #[tokio::test]
    async fn assistant_tool_call_history_round_trips_through_the_wire_format() {
        let (base, seen) = stub(|_| sse(&[&text_frame("ok"), "[DONE]"])).await;

        let mut req = request();
        req.messages = vec![
            ChatMessage::system("be useful"),
            ChatMessage::user("say hi to Chef"),
            ChatMessage::Assistant {
                content: None,
                tool_calls: vec![WireToolCall {
                    id: "call_1".into(),
                    kind: "function".into(),
                    function: WireFunction { name: "send_message".into(), arguments: "{}".into() },
                }],
            },
            ChatMessage::Tool { tool_call_id: "call_1".into(), content: "delivered".into() },
        ];

        let client = LlmClient::new().unwrap();
        client.stream_chat(&cfg(base), &req, |_| {}).await.unwrap();

        let body = seen.lock().unwrap()[0].clone();
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[2]["role"], "assistant");
        assert!(
            messages[2].get("content").is_none(),
            "a null content field is rejected by some providers"
        );
        assert_eq!(messages[2]["tool_calls"][0]["id"], "call_1");
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["tool_call_id"], "call_1");
    }

    #[tokio::test]
    async fn a_missing_key_fails_before_any_network_call() {
        let client = LlmClient::new().unwrap();
        let mut config = cfg("http://127.0.0.1:1/v1".into());
        config.api_key = "   ".into();
        let err = client.stream_chat(&config, &request(), |_| {}).await.unwrap_err();
        assert!(matches!(err, LlmError::NotConfigured));
        assert!(!err.is_transient(), "a missing key will not fix itself on retry");
    }

    #[tokio::test]
    async fn a_401_is_reported_as_an_auth_problem() {
        let (base, _) = stub(|_| {
            (
                axum::http::StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({"error": {"message": "No auth credentials found"}})),
            )
                .into_response()
        })
        .await;

        let client = LlmClient::new().unwrap();
        let err = client.stream_chat(&cfg(base), &request(), |_| {}).await.unwrap_err();
        match err {
            LlmError::Auth { status, ref message } => {
                assert_eq!(status, 401);
                assert!(message.contains("No auth credentials"));
            }
            other => panic!("expected Auth, got {other:?}"),
        }
        assert!(!err.is_transient());
    }

    #[tokio::test]
    async fn a_429_carries_the_retry_after_hint_and_is_transient() {
        let (base, _) = stub(|_| {
            (
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", "30")],
                axum::Json(serde_json::json!({"error": {"message": "slow down"}})),
            )
                .into_response()
        })
        .await;

        let client = LlmClient::new().unwrap();
        let err = client.stream_chat(&cfg(base), &request(), |_| {}).await.unwrap_err();
        match err {
            LlmError::RateLimited { retry_after_secs, .. } => {
                assert_eq!(retry_after_secs, Some(30));
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
        assert!(err.is_transient());
    }

    #[tokio::test]
    async fn an_unknown_model_is_named_rather_than_reported_as_a_generic_failure() {
        let (base, _) = stub(|_| {
            (
                axum::http::StatusCode::BAD_REQUEST,
                axum::Json(
                    serde_json::json!({"error": {"message": "model 'nope/nope' is not a valid model ID"}}),
                ),
            )
                .into_response()
        })
        .await;

        let client = LlmClient::new().unwrap();
        let mut req = request();
        req.model = "nope/nope".into();
        let err = client.stream_chat(&cfg(base), &req, |_| {}).await.unwrap_err();
        match err {
            LlmError::ModelRejected { ref model, .. } => assert_eq!(model, "nope/nope"),
            other => panic!("expected ModelRejected, got {other:?}"),
        }
        assert!(err.headline().contains("nope/nope"));
    }

    #[tokio::test]
    async fn a_500_is_transient_but_a_400_is_not() {
        let (base_500, _) = stub(|_| {
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "upstream exploded").into_response()
        })
        .await;
        let client = LlmClient::new().unwrap();
        let err = client.stream_chat(&cfg(base_500), &request(), |_| {}).await.unwrap_err();
        assert!(err.is_transient(), "a 5xx is worth retrying");

        let (base_400, _) = stub(|_| {
            (
                axum::http::StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({"error": {"message": "bad shape"}})),
            )
                .into_response()
        })
        .await;
        let err = client.stream_chat(&cfg(base_400), &request(), |_| {}).await.unwrap_err();
        assert!(!err.is_transient(), "a malformed request will fail identically on retry");
    }

    #[tokio::test]
    async fn an_error_arriving_mid_stream_is_surfaced() {
        let frames = vec![
            text_frame("partial answer"),
            serde_json::json!({"error": {"message": "provider dropped the request"}}).to_string(),
        ];
        let (base, _) = stub(move |_| sse_owned(&frames)).await;

        let client = LlmClient::new().unwrap();
        let err = client.stream_chat(&cfg(base), &request(), |_| {}).await.unwrap_err();
        match err {
            LlmError::Upstream { ref message, .. } => assert!(message.contains("provider dropped")),
            other => panic!("expected Upstream, got {other:?}"),
        }
        assert!(err.is_transient(), "a mid-stream provider failure must be retried");
    }

    #[tokio::test]
    async fn an_unparseable_frame_is_skipped_rather_than_failing_the_turn() {
        let (base, _) = stub(|_| {
            sse(&[&text_frame("good "), "{not json at all", &text_frame("news"), "[DONE]"])
        })
        .await;

        let client = LlmClient::new().unwrap();
        let completion = client.stream_chat(&cfg(base), &request(), |_| {}).await.unwrap();
        assert_eq!(completion.content, "good news");
    }

    #[tokio::test]
    async fn an_empty_stream_is_reported_as_truncated() {
        let (base, _) =
            stub(|_| ([("content-type", "text/event-stream")], "").into_response()).await;
        let client = LlmClient::new().unwrap();
        let err = client.stream_chat(&cfg(base), &request(), |_| {}).await.unwrap_err();
        assert!(matches!(err, LlmError::Truncated), "got {err:?}");
        assert!(err.is_transient());
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_is_a_transport_error_naming_the_url() {
        let client = LlmClient::new().unwrap();
        let err = client
            .stream_chat(&cfg("http://127.0.0.1:1/v1".into()), &request(), |_| {})
            .await
            .unwrap_err();
        match err {
            LlmError::Transport { ref url, .. } => assert!(url.contains("127.0.0.1:1")),
            other => panic!("expected Transport, got {other:?}"),
        }
        assert!(err.is_transient());
    }

    #[test]
    fn empty_tool_arguments_parse_as_an_empty_object() {
        let call = ToolCall { id: "x".into(), name: "directory".into(), arguments: "  ".into() };
        assert_eq!(call.parsed_arguments().unwrap(), serde_json::json!({}));
    }

    #[test]
    fn malformed_tool_arguments_report_an_error_rather_than_defaulting() {
        let call = ToolCall { id: "x".into(), name: "send".into(), arguments: "{oops".into() };
        assert!(call.parsed_arguments().is_err());
    }

    #[test]
    fn error_messages_are_extracted_from_several_body_shapes() {
        assert_eq!(extract_message(r#"{"error":{"message":"nope"}}"#), "nope");
        assert_eq!(extract_message(r#"{"error":{"code":429}}"#), "error code 429");
        assert_eq!(extract_message("plain text failure"), "plain text failure");
        assert_eq!(extract_message("   "), "empty error body");
    }

    #[test]
    fn error_message_extraction_is_bounded() {
        let huge = "x".repeat(10_000);
        assert!(extract_message(&huge).chars().count() <= 400);
    }
}
