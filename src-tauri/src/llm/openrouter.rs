//! OpenAI-compatible chat client, pointed at OpenRouter by default.
//!
//! The base URL is configuration, not a constant, so the same code talks to
//! OpenRouter, a local LM Studio, or a stub server in tests. Tests exercise the
//! real transport against a local stub rather than mocking the client, because
//! the bugs worth catching here (chunk boundaries, tool-call assembly, error
//! classification) all live in the wire handling.

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

// ---- request types -------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum ChatMessage {
    System {
        content: String,
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

/// What a user turn carries. A plain string for ordinary text, or a list of
/// parts when one of them is an image.
///
/// Untagged so both forms serialize the way OpenAI-compatible endpoints expect:
/// a bare string, or the array of `{type: ...}` objects. Sending the array form
/// for every message would work too, but it is noisier on the wire and some
/// endpoints handle the string form better.
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
    /// A `data:` URL. Screenshots never leave the machine as files, so there is
    /// nothing to host and nothing to clean up.
    pub url: String,
}

impl ChatMessage {
    /// A user turn carrying a picture, which is how an agent sees its screen.
    ///
    /// The text goes first: a model shown an image with no framing tends to
    /// describe it, and what is wanted is for it to act on it.
    pub fn user_seeing(text: impl Into<String>, image_data_url: impl Into<String>) -> Self {
        ChatMessage::User {
            content: UserContent::Parts(vec![
                ContentPart::Text { text: text.into() },
                ContentPart::ImageUrl { image_url: ImageSource { url: image_data_url.into() } },
            ]),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        ChatMessage::System { content: content.into() }
    }

    pub fn user(content: impl Into<String>) -> Self {
        ChatMessage::User { content: UserContent::Text(content.into()) }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        ChatMessage::Assistant { content: Some(content.into()), tool_calls: Vec::new() }
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
    /// A JSON string, not an object. The API defines it that way, and models
    /// emit it incrementally, so it stays a string until it is parsed once.
    pub arguments: String,
}

/// A tool offered to the model.
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
    messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "<[ToolSpec]>::is_empty")]
    tools: &'a [ToolSpec],
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    stream: bool,
    /// Asks for the token counts in the stream's last frame.
    ///
    /// Without it a streamed completion reports nothing at all about what it
    /// cost, and an operator watching a crew work has no way to tell a agent
    /// thinking hard from one stuck in a loop. Part of the OpenAI wire format,
    /// so a local server that does not implement it ignores it.
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

// ---- response types ------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// One fragment of a streamed response, as it arrives.
///
/// Reasoning is carried apart from the answer rather than concatenated into it.
/// It is shown while a turn is running and then dropped: never persisted, never
/// hashed, never sent back to a model. With a single `&str` callback the two
/// were the same thing by the time anyone could tell them apart, and the only
/// way back would be parsing prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token<'a> {
    /// Part of what the model is saying.
    Text(&'a str),
    /// Part of how it got there, when the provider publishes any.
    Reasoning(&'a str),
}

impl ToolCall {
    /// Parses the argument string, tolerating an empty one.
    ///
    /// Models sometimes emit no arguments at all for a zero-parameter tool;
    /// treating that as `{}` avoids a spurious failure.
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
    /// None when the provider did not report any. Never guessed: a made-up
    /// token count is worse than no token count.
    pub usage: Option<Usage>,
}

/// What one model call cost, as the provider counted it.
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    /// Dollars, when the provider prices the call. OpenRouter does; an
    /// OpenAI-compatible server run locally has nothing to charge and says
    /// nothing, which is why this is optional rather than zero.
    #[serde(default)]
    pub cost: Option<f64>,
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

// ---- errors --------------------------------------------------------------

/// Classified so the caller can tell an operator mistake from an upstream
/// outage from a bug in Guac. Collapsing these into one string is what makes
/// "it didn't work" take an hour to diagnose.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("no API key is configured. Open Settings and paste an OpenRouter key.")]
    NotConfigured,
    #[error("the inference endpoint rejected the API key (HTTP {status}): {message}")]
    Auth { status: u16, message: String },
    /// A subscription that is missing, expired, or refused.
    ///
    /// Separate from `Auth` because the way out is different and an agent
    /// reading this mid-turn has to be told which one it is: nobody fixes a
    /// rejected sign-in by checking their API key.
    #[error(
        "the ChatGPT subscription could not be used: {message}. Open Settings and sign in again."
    )]
    SubscriptionRejected { message: String },
    /// The program a provider runs could not be found.
    ///
    /// Separate from `NotConfigured` because nothing in Settings fixes it and
    /// an agent reading this mid-turn must not be sent to look for a key that
    /// this provider never wanted.
    ///
    /// It says *or is not on this app's PATH* because both are real and they are
    /// told apart by looking in a terminal rather than by anything this app can
    /// see. `programs.rs` is what makes the second one rare; a bare "not
    /// installed" said to an operator who has the program is a refusal whose
    /// only next move is to install it again.
    #[error("{program} is not installed, or is not on this app's PATH. Install it with `{install}`, or pick another provider in Settings.", install = crate::llm::claude::INSTALL)]
    ProgramMissing { program: &'static str },
    /// The program ran and would not answer.
    ///
    /// Its own words, because with no endpoint and no status code they are the
    /// only account of the failure there is, and the commonest cause is a
    /// sign-in or a spent plan that only the operator can put right.
    #[error("{program} could not answer: {message}")]
    ProgramFailed { program: &'static str, message: String },
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
    /// Whether retrying the identical request could plausibly succeed.
    pub fn is_transient(&self) -> bool {
        match self {
            LlmError::RateLimited { .. } | LlmError::Timeout { .. } | LlmError::Truncated => true,
            LlmError::Upstream { status, .. } => *status >= 500,
            LlmError::ProgramMissing { .. } | LlmError::ProgramFailed { .. } => false,
            LlmError::Transport { .. } => true,
            _ => false,
        }
    }

    /// Short form for the transcript chip.
    pub fn headline(&self) -> String {
        match self {
            LlmError::NotConfigured => "no API key configured".into(),
            LlmError::Auth { .. } => "API key rejected".into(),
            LlmError::SubscriptionRejected { .. } => "sign in to ChatGPT again".into(),
            LlmError::ProgramMissing { program } => format!("{program} could not be found"),
            LlmError::ProgramFailed { program, .. } => format!("{program} could not answer"),
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

/// Pulls a human-readable message out of an error body, falling back to the
/// raw text when the shape is unfamiliar.
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
        // OpenRouter reports an unknown or unavailable model as a 400 or 404.
        // Surfacing that as a generic upstream error sends the operator hunting
        // for a network problem that is not there.
        400 | 404 if message.to_lowercase().contains("model") => {
            LlmError::ModelRejected { model: model.to_string(), message }
        }
        _ => LlmError::Upstream { status, message },
    }
}

// ---- streaming wire shapes ----------------------------------------------

#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    /// Sent once, in a frame that carries no choices.
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
    /// The model's own working, when it publishes any.
    ///
    /// Two spellings because two conventions exist: OpenRouter says
    /// `reasoning`, and the OpenAI-compatible servers that followed DeepSeek
    /// say `reasoning_content`. Reading only one of them shows a thinking model
    /// thinking on one endpoint and working in silence on the next. A provider
    /// that sends both sends the same words twice, which is why they are
    /// coalesced rather than concatenated.
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<DeltaToolCall>,
}

impl Delta {
    /// Whichever spelling of the reasoning this frame used, if either.
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

/// Reassembles tool calls that arrive as fragments.
///
/// The id and name land on the first fragment, arguments accumulate across
/// many. Keyed by index because that is the only field guaranteed on every
/// fragment. `BTreeMap` so the final order matches the model's own ordering
/// rather than hash order.
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
            // A fragment with no name is not a callable request. Dropping it is
            // better than dispatching to an empty tool name.
            .filter(|c| !c.name.is_empty())
            .map(|mut c| {
                if c.id.is_empty() {
                    // Some providers omit ids entirely. The id only has to be
                    // unique within one exchange for the tool result to match up.
                    c.id = format!("call_{}", c.name);
                }
                c
            })
            .collect()
    }
}

// ---- client --------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    /// The ChatGPT sign-in, when this build has one attached.
    ///
    /// Optional rather than required so every test that only exercises the
    /// compatible transport constructs a client the same way it always did. A
    /// subscription call without one is a configuration error with a clear
    /// message, not a panic.
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

    /// Gives this client the sign-in that subscription calls are made with.
    pub fn with_subscription(mut self, subscription: Arc<Subscription>) -> Self {
        self.subscription = Some(subscription);
        self
    }

    /// Streams a completion, calling `on_token` for each fragment.
    ///
    /// `on_token` runs on the caller's task and must not block; it exists so
    /// the UI can render a turn as it happens rather than after it ends.
    ///
    /// The one place the two providers part company. Everything above this call
    /// assembles a single kind of request and reads a single kind of completion,
    /// which is why a subscription needed no change to the runtime, the prompt
    /// or the guard.
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

        let body = WireRequest {
            model: &request.model,
            messages: &request.messages,
            tools: &request.tools,
            tool_choice: if request.tools.is_empty() { None } else { Some("auto") },
            stream: true,
            stream_options: Some(StreamOptions { include_usage: true }),
            temperature: request.temperature,
        };

        let response = self
            .http
            .post(&url)
            .bearer_auth(cfg.api_key.trim())
            // OpenRouter uses these for attribution and leaderboard ranking.
            // Other OpenAI-compatible servers ignore them.
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
                    // A single unparseable frame should not discard a response
                    // that is otherwise fine. Log and keep going.
                    Err(err) => {
                        tracing::warn!(%err, payload = %truncate(&payload, 200), "skipping unparseable stream frame");
                        continue;
                    }
                };

                // Errors can arrive mid-stream after a 200 has already been sent.
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
                    // Ahead of the text, and accumulated nowhere: a model
                    // reasons its way to what it then says, and nothing
                    // downstream of this call is allowed to keep the reasoning.
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

        // A stream that produced nothing and never terminated cleanly was cut
        // off. Reporting that beats handing back an empty message.
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

    /// Spins a stub server and returns its base URL plus the bodies it saw.
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

    /// Same as [`sse`], for frames the test owns rather than builds inline.
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

    /// A reasoning frame, spelled the way the endpoint under test spells it.
    fn thinking_frame(field: &str, content: &str) -> String {
        serde_json::json!({ "choices": [{ "delta": { field: content } }] }).to_string()
    }

    /// Everything a stream reported, tagged, in arrival order.
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
        // A model that publishes its working writes it before the answer, in
        // frames of the same shape. Concatenating the two would put the
        // thinking in the transcript, in the prompt of every later turn, and in
        // the fingerprint the loop guard compares.
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
        // OpenRouter says `reasoning`; the servers that followed DeepSeek say
        // `reasoning_content`. An operator pointing Guaca at a local endpoint
        // should not lose the line under the composer.
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
        // A provider that fills both fields is saying the same thing twice.
        // Passing both through doubles every word on screen.
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
        // Thinking is not an answer. A stream cut off after its reasoning has
        // produced nothing to say, and reporting it as an empty success is how
        // an agent goes silent with no explanation.
        let (base, _) =
            stub(|_| sse(&[&thinking_frame("reasoning", "still working it out")])).await;

        let client = LlmClient::new().unwrap();
        let err = client.stream_chat(&cfg(base), &request(), |_| {}).await.unwrap_err();
        assert!(matches!(err, LlmError::Truncated), "got {err:?}");
    }

    #[tokio::test]
    async fn asks_for_token_counts_and_reads_them_from_the_last_frame() {
        // The realistic shape: usage arrives alone, after the content, in a
        // frame that carries no choices at all.
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

        // Nothing is counted unless it is asked for.
        let body = seen.lock().unwrap()[0].clone();
        assert_eq!(
            body["stream_options"]["include_usage"],
            serde_json::json!(true),
            "a streamed call reports nothing at all without this: {body}"
        );
    }

    #[tokio::test]
    async fn a_provider_that_counts_nothing_reports_nothing() {
        // Rather than zero, which would read on screen exactly like a real
        // count of zero and quietly understate what a crew was spending.
        let (base, _) = stub(|_| sse(&[&text_frame("done"), "[DONE]"])).await;
        let client = LlmClient::new().unwrap();
        let completion = client.stream_chat(&cfg(base), &request(), |_| {}).await.unwrap();
        assert!(completion.usage.is_none());
    }

    #[tokio::test]
    async fn assembles_a_tool_call_split_across_fragments() {
        // The realistic shape: id and name once, arguments in pieces.
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
        // A 200 followed by an error frame is a real OpenRouter behavior when
        // an upstream provider fails after the connection is established.
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
        // Port 1 is reserved and refuses immediately.
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
