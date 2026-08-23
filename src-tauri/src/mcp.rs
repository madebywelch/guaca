//! The client end of the Model Context Protocol, over streamable HTTP.
//!
//! Small on purpose. Guaca is not a general MCP host: it dials the handful of
//! servers it ships the addresses of, asks each what it can do, and calls those
//! tools on behalf of an agent. Three methods cover all of that — `initialize`,
//! `tools/list`, `tools/call` — and the transport is one POST each.
//!
//! ## Two content types, one request
//!
//! A streamable-HTTP server may answer a POST with `application/json` or with
//! `text/event-stream`, and it chooses per request. Both are in use across the
//! servers on the list: one answers every call as an event stream including
//! `initialize`, and Neon's answers in JSON. Neither is wrong and the spec
//! requires a client to accept both, which is why the reply is parsed by
//! sniffing the content type rather than by trusting the one a server used
//! last. This is not the streaming case `llm/sse.rs` handles: nothing here is
//! shown as it arrives, so the body is read whole and the one JSON-RPC object
//! in it is pulled out.
//!
//! ## The session header
//!
//! A server may hand back `Mcp-Session-Id` on `initialize` and then require it
//! on every later request. It may also not, and then requires that the header
//! is absent. Both are legal, so the id is whatever the server said and nothing
//! is invented.
//!
//! ## What a 401 means here
//!
//! It is the start of the sign-in, not a failure. An unauthenticated
//! `initialize` is how Guaca discovers whether a server needs a grant at all,
//! and `WWW-Authenticate` on the refusal is where the spec puts the address of
//! the metadata that says who can issue one. `oauth.rs` reads it.

use std::time::Duration;

use serde::Deserialize;

/// The revision Guaca speaks, sent on every request.
///
/// Servers use it to decide what to offer, and one that does not know this
/// revision says so rather than guessing.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Long enough for a server that is waking up, short enough that a turn does
/// not sit on a dead endpoint. Tool calls get their own, longer, budget.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("could not reach {endpoint}: {source}")]
    Transport {
        endpoint: String,
        #[source]
        source: reqwest::Error,
    },
    /// Split out because it is the one refusal with a way forward: sign in.
    #[error("{endpoint} wants you to sign in")]
    Unauthorized {
        endpoint: String,
        /// The challenge, verbatim. It carries the address of the metadata that
        /// says which authorization server can issue a grant for this resource.
        challenge: Option<String>,
    },
    #[error("{endpoint} returned HTTP {status}: {body}")]
    Status { endpoint: String, status: u16, body: String },
    #[error("{endpoint} answered with something that is not MCP: {detail}")]
    Malformed { endpoint: String, detail: String },
    /// The server understood the call and refused it. Handed to the agent as
    /// the tool's result rather than raised, so a turn can read the reason and
    /// try something else.
    #[error("{message}")]
    Rejected { message: String },
}

impl McpError {
    /// Whether signing in again is the thing that would fix this.
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, McpError::Unauthorized { .. })
    }
}

/// A tool as the server describes it.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Absent on a tool that takes no arguments, which is legal and means an
    /// empty object.
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Option<serde_json::Value>,
}

/// What `initialize` established, and what every later call has to carry.
#[derive(Debug, Clone)]
pub struct Session {
    pub endpoint: String,
    pub token: Option<String>,
    /// Whatever the server said, or nothing. Never invented: a server that
    /// issued no session id rejects a request that carries one.
    pub session_id: Option<String>,
    /// How the server names itself. The nearest thing to an account label an
    /// MCP server offers, and used as one only when it says something better
    /// than its own product name.
    pub server_name: String,
}

/// Opens a session, which is how Guaca finds out whether a grant is needed.
///
/// Called with no token first, on purpose. A server that authorizes everybody
/// answers, and the operator is never sent to a browser to authorize something
/// that was already open; one that does not answers 401 and says where its
/// authorization server is.
pub async fn open(endpoint: &str, token: Option<&str>) -> Result<Session, McpError> {
    let http = client()?;
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "Guaca", "version": env!("CARGO_PKG_VERSION") },
        },
    });

    let (value, session_id) = post(&http, endpoint, token, None, &body, CONNECT_TIMEOUT).await?;
    let server_name = value
        .get("serverInfo")
        .and_then(|info| info.get("name"))
        .and_then(|name| name.as_str())
        .unwrap_or_default()
        .to_string();

    let session = Session {
        endpoint: endpoint.to_string(),
        token: token.map(str::to_string),
        session_id,
        server_name,
    };

    // A notification, not a request: no id, and the server answers 202 with no
    // body. Skipping it leaves servers that gate on it refusing every later
    // call with "not initialized", which reads as an auth failure and is not.
    let ready = serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    let _ = notify(&http, &session, &ready).await;

    Ok(session)
}

/// Everything this server offers, once.
pub async fn list_tools(session: &Session) -> Result<Vec<ToolDescriptor>, McpError> {
    let http = client()?;
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
    let (value, _) = post(
        &http,
        &session.endpoint,
        session.token.as_deref(),
        session.session_id.as_deref(),
        &body,
        CONNECT_TIMEOUT,
    )
    .await?;

    #[derive(Deserialize)]
    struct Listed {
        #[serde(default)]
        tools: Vec<ToolDescriptor>,
    }

    serde_json::from_value::<Listed>(value).map(|listed| listed.tools).map_err(|err| {
        McpError::Malformed {
            endpoint: session.endpoint.clone(),
            detail: format!("its tool list did not parse: {err}"),
        }
    })
}

/// Runs one tool and renders its answer as the text an agent reads.
///
/// A refusal the server understood comes back as `Rejected` rather than as an
/// error on the transport, because those are different things to an agent: one
/// is "that call was wrong, here is why", the other is "the plugin is not
/// reachable". Only the first is worth rewording and trying again.
pub async fn call_tool(
    session: &Session,
    tool: &str,
    arguments: &serde_json::Value,
) -> Result<String, McpError> {
    let http = client()?;
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": { "name": tool, "arguments": arguments },
    });
    let (value, _) = post(
        &http,
        &session.endpoint,
        session.token.as_deref(),
        session.session_id.as_deref(),
        &body,
        CALL_TIMEOUT,
    )
    .await?;

    let rendered = render_content(&value);
    if value.get("isError").and_then(serde_json::Value::as_bool).unwrap_or(false) {
        return Err(McpError::Rejected {
            message: if rendered.is_empty() {
                format!("{tool} failed and said nothing about why")
            } else {
                rendered
            },
        });
    }

    Ok(if rendered.is_empty() { format!("{tool} returned nothing.") } else { rendered })
}

/// The parts of a result an agent can read, joined.
///
/// Non-text parts are named rather than dropped. A tool that answered with an
/// image and nothing else would otherwise look to the model like a tool that
/// answered with nothing, and the next thing it does is call it again.
fn render_content(result: &serde_json::Value) -> String {
    // Newer servers may answer with a structured result and no content block at
    // all. Falling through to the JSON is better than telling an agent the call
    // returned nothing when it returned everything.
    let Some(parts) = result.get("content").and_then(serde_json::Value::as_array) else {
        return match result.get("structuredContent") {
            Some(value) => value.to_string(),
            None => String::new(),
        };
    };

    let mut out = Vec::new();
    for part in parts {
        match part.get("type").and_then(serde_json::Value::as_str) {
            Some("text") => {
                if let Some(text) = part.get("text").and_then(serde_json::Value::as_str) {
                    out.push(text.to_string());
                }
            }
            Some(other) => out.push(format!("[{other}, which cannot be shown here]")),
            None => {}
        }
    }
    out.join("\n")
}

fn client() -> Result<reqwest::Client, McpError> {
    reqwest::Client::builder()
        .build()
        .map_err(|source| McpError::Transport { endpoint: "the http client".to_string(), source })
}

/// One JSON-RPC request, and the `result` out of the answer.
///
/// Returns the session id alongside, because `initialize` is the one call that
/// establishes it and the caller has nowhere else to read it from.
async fn post(
    http: &reqwest::Client,
    endpoint: &str,
    token: Option<&str>,
    session_id: Option<&str>,
    body: &serde_json::Value,
    timeout: Duration,
) -> Result<(serde_json::Value, Option<String>), McpError> {
    let mut request = http
        .post(endpoint)
        .timeout(timeout)
        .header("content-type", "application/json")
        // Both, always. Which one comes back is the server's choice and it may
        // make a different one per request.
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", PROTOCOL_VERSION);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if let Some(id) = session_id {
        request = request.header("mcp-session-id", id);
    }

    let response = request
        .json(body)
        .send()
        .await
        .map_err(|source| McpError::Transport { endpoint: endpoint.to_string(), source })?;

    let status = response.status();
    let issued = header(&response, "mcp-session-id");
    let challenge = header(&response, "www-authenticate");
    let content_type = header(&response, "content-type").unwrap_or_default();
    let text = response
        .text()
        .await
        .map_err(|source| McpError::Transport { endpoint: endpoint.to_string(), source })?;

    if status.as_u16() == 401 {
        return Err(McpError::Unauthorized { endpoint: endpoint.to_string(), challenge });
    }
    if !status.is_success() {
        return Err(McpError::Status {
            endpoint: endpoint.to_string(),
            status: status.as_u16(),
            body: text.chars().take(400).collect(),
        });
    }

    let envelope = decode(&content_type, &text).ok_or_else(|| McpError::Malformed {
        endpoint: endpoint.to_string(),
        detail: format!("no JSON-RPC message in a {content_type} body"),
    })?;

    if let Some(error) = envelope.get("error") {
        let message = error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("it refused and gave no reason");
        return Err(McpError::Rejected { message: message.to_string() });
    }

    let result = envelope.get("result").cloned().ok_or_else(|| McpError::Malformed {
        endpoint: endpoint.to_string(),
        detail: "a reply with neither a result nor an error".to_string(),
    })?;

    Ok((result, issued))
}

/// A notification: sent, and not waited on for anything but delivery.
async fn notify(
    http: &reqwest::Client,
    session: &Session,
    body: &serde_json::Value,
) -> Result<(), McpError> {
    let mut request = http
        .post(&session.endpoint)
        .timeout(CONNECT_TIMEOUT)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", PROTOCOL_VERSION);
    if let Some(token) = &session.token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if let Some(id) = &session.session_id {
        request = request.header("mcp-session-id", id.as_str());
    }
    request
        .json(body)
        .send()
        .await
        .map(|_| ())
        .map_err(|source| McpError::Transport { endpoint: session.endpoint.clone(), source })
}

fn header(response: &reqwest::Response, name: &str) -> Option<String> {
    response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
}

/// The JSON-RPC envelope out of a body that is either JSON or an event stream.
///
/// An event stream here carries one message and then ends, so the first `data:`
/// line that parses is the answer. Fields are joined with a newline as the SSE
/// grammar requires; a server that splits a large result across several `data:`
/// lines is legal and would otherwise decode as truncated JSON.
fn decode(content_type: &str, body: &str) -> Option<serde_json::Value> {
    if !content_type.contains("text/event-stream") {
        return serde_json::from_str(body).ok();
    }

    let mut data = String::new();
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            continue;
        }
        // A blank line ends the event. Whatever has been collected is the
        // message, if it is one.
        if line.trim().is_empty() && !data.is_empty() {
            if let Ok(value) = serde_json::from_str(&data) {
                return Some(value);
            }
            data.clear();
        }
    }
    serde_json::from_str(&data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_json_reply_decodes() {
        let value = decode("application/json", r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#);
        assert_eq!(value.unwrap()["result"]["ok"], serde_json::json!(true));
    }

    #[test]
    fn an_event_stream_reply_decodes() {
        // A real server answers every call this way, `initialize` included.
        // Parsing only JSON made a working server look like a broken one.
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":1}}\n\n";
        let value = decode("text/event-stream; charset=utf-8", body);
        assert_eq!(value.unwrap()["result"]["ok"], serde_json::json!(1));
    }

    #[test]
    fn an_event_split_across_data_lines_is_joined_before_it_is_parsed() {
        // The SSE grammar allows it and a large tool list is where it happens.
        // Taking the first line alone decodes as truncated JSON, which reads as
        // "this server is not MCP" rather than "this reply was long".
        let body = "data: {\"jsonrpc\":\"2.0\",\ndata: \"id\":2,\"result\":{\"tools\":[]}}\n\n";
        let value = decode("text/event-stream", body).expect("a joined event parses");
        assert_eq!(value["result"]["tools"], serde_json::json!([]));
    }

    #[test]
    fn a_stream_with_no_message_is_not_a_reply() {
        assert!(decode("text/event-stream", ": keep-alive\n\n").is_none());
        assert!(decode("application/json", "not json").is_none());
    }

    #[test]
    fn text_parts_are_joined_and_other_parts_are_named() {
        let result = serde_json::json!({
            "content": [
                { "type": "text", "text": "first" },
                { "type": "image", "data": "…" },
                { "type": "text", "text": "second" },
            ]
        });
        assert_eq!(render_content(&result), "first\n[image, which cannot be shown here]\nsecond");
    }

    #[test]
    fn a_structured_result_with_no_content_block_is_still_an_answer() {
        // Otherwise a tool that answered with everything is reported to the
        // agent as having answered with nothing, and it calls it again.
        let result = serde_json::json!({ "structuredContent": { "rows": 3 } });
        assert_eq!(render_content(&result), r#"{"rows":3}"#);
    }

    #[test]
    fn an_empty_result_renders_as_nothing_rather_than_as_a_shape() {
        assert_eq!(render_content(&serde_json::json!({ "content": [] })), "");
    }

    #[test]
    fn only_a_401_is_worth_signing_in_over() {
        let unauthorized =
            McpError::Unauthorized { endpoint: "https://example.test/mcp".into(), challenge: None };
        let refused = McpError::Rejected { message: "no such tool".into() };
        assert!(unauthorized.is_unauthorized());
        assert!(!refused.is_unauthorized());
    }
}
