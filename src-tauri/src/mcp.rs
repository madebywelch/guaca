use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Deserialize;

mod stdio;

pub const SUPPORTED: [&str; 5] =
    ["2026-07-28", "2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

pub const PROTOCOL_VERSION: &str = SUPPORTED[0];

pub const LEGACY_VERSION: &str = "2025-11-25";

const FIRST_MODERN: &str = "2026-07-28";

fn modern(version: &str) -> bool {
    version >= FIRST_MODERN
}

const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CLIENT: &str = "io.modelcontextprotocol/clientInfo";
const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_SERVER: &str = "io.modelcontextprotocol/serverInfo";

const UNSUPPORTED_VERSION: i64 = -32022;
const HEADER_MISMATCH: i64 = -32020;

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
    #[error("{endpoint} wants you to sign in")]
    Unauthorized { endpoint: String, challenge: Option<String> },
    #[error("{endpoint} returned HTTP {status}: {body}")]
    Status { endpoint: String, status: u16, body: String },
    #[error("{endpoint} answered with something that is not MCP: {detail}")]
    Malformed { endpoint: String, detail: String },
    #[error("{message}")]
    Rejected { message: String },
    #[error(
        "{endpoint} speaks MCP {}, and this build speaks {}. Nothing here will connect it; \
         update Guaca.",
        .supported.join(" or "),
        SUPPORTED.join(", ")
    )]
    NoSharedVersion { endpoint: String, supported: Vec<String> },
    #[error("{endpoint} refused the request as malformed ({detail}); this is a bug in Guaca")]
    HeaderMismatch { endpoint: String, detail: String },
    #[error("{endpoint} answered a GET with {content_type}, not an event stream")]
    NotAStream { endpoint: String, content_type: String },
    #[error("{endpoint} opened an event stream and did not answer {method} on it within {secs}s")]
    Silent { endpoint: String, method: String, secs: u64 },
    #[error("the connector run as `{}` failed: {detail}", .endpoint.trim_start_matches("stdio:"))]
    Program { endpoint: String, detail: String },
}

impl McpError {
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, McpError::Unauthorized { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Era {
    Modern { version: String },
    Legacy,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    Streamable,
    Sse,
    Stdio,
}

#[derive(Clone, Copy)]
pub struct Dial<'a> {
    pub endpoint: &'a str,
    pub token: Option<&'a str>,
    pub headers: &'a [(String, String)],
    pub legacy_transport: bool,
}

impl<'a> Dial<'a> {
    pub fn to(endpoint: &'a str) -> Dial<'a> {
        Dial { endpoint, token: None, headers: &[], legacy_transport: false }
    }

    pub fn with_token(self, token: Option<&'a str>) -> Dial<'a> {
        Dial { token, ..self }
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub endpoint: String,
    pub token: Option<String>,
    headers: Vec<(String, String)>,
    pub era: Era,
    pub transport: Transport,
    pub session_id: Option<String>,
    pub server_name: String,
    negotiated: Option<String>,
    process: Option<std::sync::Arc<stdio::Process>>,
}

impl Session {
    pub fn sse(&self) -> bool {
        self.transport == Transport::Sse
    }

    pub fn stdio(&self) -> bool {
        self.transport == Transport::Stdio
    }

    fn version(&self) -> &str {
        match &self.era {
            Era::Modern { version } => version,
            Era::Legacy => self.negotiated.as_deref().unwrap_or(LEGACY_VERSION),
        }
    }

    pub fn modern(&self) -> bool {
        matches!(self.era, Era::Modern { .. })
    }

    pub fn protocol(&self) -> &str {
        self.version()
    }
}

pub async fn open(dial: Dial<'_>) -> Result<Session, McpError> {
    if dial.endpoint.starts_with(crate::domain::plugin::STDIO) {
        return stdio::open(dial).await;
    }
    let http = client()?;

    if let Some(spoken) = remembered(dial.endpoint) {
        match establish(&http, dial, spoken, String::new()).await {
            Ok(session) => return Ok(session),
            Err(err) if err.is_unauthorized() || matches!(err, McpError::Transport { .. }) => {
                return Err(err)
            }
            Err(_) => forget(dial.endpoint),
        }
    }

    let probed = probe(&http, dial).await?;
    remember(dial.endpoint, &probed.spoken);
    establish(&http, dial, probed.spoken, probed.server_name).await
}

async fn establish(
    http: &reqwest::Client,
    dial: Dial<'_>,
    spoken: Spoken,
    server_name: String,
) -> Result<Session, McpError> {
    let Spoken { era, transport, negotiated } = spoken;
    let free = |era: Era, negotiated: Option<String>| Session {
        endpoint: dial.endpoint.to_string(),
        token: dial.token.map(str::to_string),
        headers: dial.headers.to_vec(),
        era,
        transport: transport.clone(),
        session_id: None,
        server_name: server_name.clone(),
        negotiated,
        process: None,
    };
    match (&era, &transport) {
        (Era::Modern { .. }, _) => Ok(free(era.clone(), None)),
        (Era::Legacy, Transport::Sse) => Ok(free(Era::Legacy, negotiated)),
        (Era::Legacy, Transport::Streamable) => initialize(http, dial).await,
        (Era::Legacy, Transport::Stdio) => stdio::open(dial).await,
    }
}

pub async fn describe(session: &Session) -> Result<String, McpError> {
    if !session.server_name.is_empty() {
        return Ok(session.server_name.clone());
    }
    if session.sse() {
        let http = client()?;
        return sse_probe(&http, dial_for(session)).await.map(|(name, _)| name);
    }
    if !session.modern() {
        return Ok(session.server_name.clone());
    }
    let result =
        request(session, "server/discover", None, serde_json::json!({}), CONNECT_TIMEOUT).await?;
    Ok(server_name(&result))
}

fn dial_for(session: &Session) -> Dial<'_> {
    Dial {
        endpoint: &session.endpoint,
        token: session.token.as_deref(),
        headers: &session.headers,
        legacy_transport: true,
    }
}

async fn request(
    session: &Session,
    method: &str,
    name: Option<&str>,
    params: serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value, McpError> {
    request_mirroring(session, method, name, params, Vec::new(), timeout).await
}

async fn request_mirroring(
    session: &Session,
    method: &str,
    name: Option<&str>,
    params: serde_json::Value,
    mirrored: Vec<(String, String)>,
    timeout: Duration,
) -> Result<serde_json::Value, McpError> {
    if let Some(process) = &session.process {
        let _ = (name, mirrored);
        return stdio::request(process, method, params, timeout).await;
    }
    let http = client()?;
    let result = if session.sse() {
        let answered =
            sse_exchange(&http, dial_for(session), Some((method, params)), timeout).await;
        answered.map(|answered| {
            settled(&session.endpoint, &answered.negotiated);
            answered.result
        })
    } else {
        let mut wire = Wire::of(session);
        wire.headers = mirrored;
        post(&http, wire, method, name, params, timeout).await.map(|(result, _)| result)
    };
    if matches!(
        &result,
        Err(McpError::Status { status: 400 | 404 | 405 | 406 | 415, .. }
            | McpError::NoSharedVersion { .. }
            | McpError::NotAStream { .. })
    ) {
        if let Ok(mut map) = eras().lock() {
            if map.get(&session.endpoint).is_some_and(|spoken| {
                spoken.era == session.era && spoken.transport == session.transport
            }) {
                map.remove(&session.endpoint);
            }
        }
    }
    result
}

pub async fn list_tools(session: &Session) -> Result<Vec<ToolDescriptor>, McpError> {
    #[derive(Deserialize)]
    struct Listed {
        #[serde(default)]
        tools: Vec<ToolDescriptor>,
        #[serde(rename = "nextCursor")]
        next_cursor: Option<String>,
    }

    let mut page = DiscoveryPage::First;
    let mut seen = std::collections::HashSet::new();
    let mut tools = Vec::new();
    while !matches!(page, DiscoveryPage::Complete) {
        let params = match &page {
            DiscoveryPage::First => serde_json::json!({}),
            DiscoveryPage::Next(cursor) => serde_json::json!({"cursor": cursor}),
            DiscoveryPage::Complete => unreachable!(),
        };
        let value = request(session, "tools/list", None, params, CONNECT_TIMEOUT).await?;
        let listed =
            serde_json::from_value::<Listed>(value).map_err(|err| McpError::Malformed {
                endpoint: session.endpoint.clone(),
                detail: format!("its tool list did not parse: {err}"),
            })?;
        tools.extend(listed.tools);
        page = page.advance(listed.next_cursor, &mut seen).map_err(|()| McpError::Malformed {
            endpoint: session.endpoint.clone(),
            detail: "its tool list has an invalid or repeated cursor".to_string(),
        })?;
    }

    if !session.modern() {
        return Ok(tools);
    }
    Ok(tools
        .into_iter()
        .filter(|tool| {
            let schema = tool.input_schema.as_ref();
            match schema.map(mirrored_params).unwrap_or_else(|| Ok(Vec::new())) {
                Ok(_) => true,
                Err(why) => {
                    tracing::warn!(
                        tool = tool.name,
                        endpoint = session.endpoint,
                        "dropping a tool whose x-mcp-header annotation cannot be honored: {why}"
                    );
                    false
                }
            }
        })
        .collect())
}

#[derive(Debug, PartialEq, Eq)]
enum DiscoveryPage {
    First,
    Next(String),
    Complete,
}

impl DiscoveryPage {
    fn advance(
        self,
        cursor: Option<String>,
        seen: &mut std::collections::HashSet<String>,
    ) -> Result<Self, ()> {
        if matches!(self, Self::Complete) {
            return Err(());
        }
        match cursor {
            None => Ok(Self::Complete),
            Some(cursor)
                if !cursor.is_empty() && seen.len() < 100 && seen.insert(cursor.clone()) =>
            {
                Ok(Self::Next(cursor))
            }
            Some(_) => Err(()),
        }
    }
}

pub async fn call_tool(
    session: &Session,
    tool: &str,
    arguments: &serde_json::Value,
    schema: Option<&serde_json::Value>,
) -> Result<Called, McpError> {
    let mirrored = match schema {
        Some(schema) if session.modern() => mirror(schema, arguments),
        _ => Vec::new(),
    };

    let value = request_mirroring(
        session,
        "tools/call",
        Some(tool),
        serde_json::json!({ "name": tool, "arguments": arguments }),
        mirrored,
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

    Ok(Called {
        text: if rendered.is_empty() { format!("{tool} returned nothing.") } else { rendered },
        structured: value.get("structuredContent").cloned(),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Called {
    pub text: String,
    pub structured: Option<serde_json::Value>,
}

struct Probed {
    spoken: Spoken,
    server_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Spoken {
    era: Era,
    transport: Transport,
    negotiated: Option<String>,
}

async fn probe(http: &reqwest::Client, dial: Dial<'_>) -> Result<Probed, McpError> {
    let endpoint = dial.endpoint;
    let wire = Wire {
        endpoint,
        token: dial.token,
        extra: dial.headers,
        version: PROTOCOL_VERSION.to_string(),
        modern: true,
        session_id: None,
        headers: Vec::new(),
    };
    match post(http, wire, "server/discover", None, serde_json::json!({}), CONNECT_TIMEOUT).await {
        Ok((result, _)) => Ok(Probed {
            spoken: Spoken {
                era: Era::Modern { version: PROTOCOL_VERSION.to_string() },
                transport: Transport::Streamable,
                negotiated: None,
            },
            server_name: server_name(&result),
        }),
        Err(McpError::NoSharedVersion { endpoint, supported }) => {
            let Some(version) = agreed(&supported) else {
                return Err(McpError::NoSharedVersion { endpoint, supported });
            };
            if !modern(&version) {
                return Ok(Probed { spoken: handshaking(), server_name: String::new() });
            }
            let wire = Wire {
                endpoint: &endpoint,
                token: dial.token,
                extra: dial.headers,
                version: version.clone(),
                modern: true,
                session_id: None,
                headers: Vec::new(),
            };
            let (result, _) =
                post(http, wire, "server/discover", None, serde_json::json!({}), CONNECT_TIMEOUT)
                    .await?;
            Ok(Probed {
                spoken: Spoken {
                    era: Era::Modern { version },
                    transport: Transport::Streamable,
                    negotiated: None,
                },
                server_name: server_name(&result),
            })
        }
        Err(err @ (McpError::Transport { .. } | McpError::Unauthorized { .. })) => Err(err),
        Err(err @ McpError::HeaderMismatch { .. }) => Err(err),
        Err(err @ (McpError::Status { .. } | McpError::Malformed { .. }))
            if dial.legacy_transport =>
        {
            match sse_probe(http, dial).await {
                Ok((server_name, negotiated)) => Ok(Probed {
                    spoken: Spoken {
                        era: Era::Legacy,
                        transport: Transport::Sse,
                        negotiated: Some(negotiated),
                    },
                    server_name,
                }),
                Err(
                    McpError::NotAStream { .. }
                    | McpError::Status { .. }
                    | McpError::Transport { .. },
                ) => Err(err),
                Err(refused) => Err(refused),
            }
        }
        Err(_) => Ok(Probed { spoken: handshaking(), server_name: String::new() }),
    }
}

fn handshaking() -> Spoken {
    Spoken { era: Era::Legacy, transport: Transport::Streamable, negotiated: None }
}

fn agreed(theirs: &[String]) -> Option<String> {
    SUPPORTED.iter().find(|ours| theirs.iter().any(|t| t == *ours)).map(|ours| ours.to_string())
}

async fn initialize(http: &reqwest::Client, dial: Dial<'_>) -> Result<Session, McpError> {
    let endpoint = dial.endpoint;
    let wire = Wire {
        endpoint,
        token: dial.token,
        extra: dial.headers,
        version: LEGACY_VERSION.to_string(),
        modern: false,
        session_id: None,
        headers: Vec::new(),
    };
    let params = serde_json::json!({
        "protocolVersion": LEGACY_VERSION,
        "capabilities": {},
        "clientInfo": client_info(),
    });
    let (value, session_id) = post(http, wire, "initialize", None, params, CONNECT_TIMEOUT).await?;

    let agreed = value
        .get("protocolVersion")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(LEGACY_VERSION)
        .to_string();
    if !SUPPORTED.contains(&agreed.as_str()) || modern(&agreed) {
        return Err(McpError::NoSharedVersion {
            endpoint: endpoint.to_string(),
            supported: vec![agreed],
        });
    }

    let session = Session {
        endpoint: endpoint.to_string(),
        token: dial.token.map(str::to_string),
        headers: dial.headers.to_vec(),
        era: Era::Legacy,
        transport: Transport::Streamable,
        session_id,
        server_name: value
            .get("serverInfo")
            .and_then(|info| info.get("name"))
            .and_then(|name| name.as_str())
            .unwrap_or_default()
            .to_string(),
        negotiated: Some(agreed),
        process: None,
    };

    let ready = serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    let _ = notify(http, &session, &ready).await;

    Ok(session)
}

fn eras() -> &'static Mutex<HashMap<String, Spoken>> {
    static ERAS: OnceLock<Mutex<HashMap<String, Spoken>>> = OnceLock::new();
    ERAS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn remembered(endpoint: &str) -> Option<Spoken> {
    eras().lock().ok()?.get(endpoint).cloned()
}

fn settled(endpoint: &str, negotiated: &str) {
    if let Ok(mut map) = eras().lock() {
        if let Some(spoken) = map.get_mut(endpoint) {
            spoken.negotiated = Some(negotiated.to_string());
        }
    }
}

fn remember(endpoint: &str, spoken: &Spoken) {
    if let Ok(mut map) = eras().lock() {
        map.insert(endpoint.to_string(), spoken.clone());
    }
}

pub fn forget(endpoint: &str) {
    if let Ok(mut map) = eras().lock() {
        map.remove(endpoint);
    }
}

fn server_name(discovered: &serde_json::Value) -> String {
    discovered
        .get("_meta")
        .and_then(|meta| meta.get(META_SERVER))
        .and_then(|info| info.get("name"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn client_info() -> serde_json::Value {
    serde_json::json!({ "name": "Guaca", "version": env!("CARGO_PKG_VERSION") })
}

struct Wire<'a> {
    endpoint: &'a str,
    token: Option<&'a str>,
    extra: &'a [(String, String)],
    version: String,
    modern: bool,
    session_id: Option<&'a str>,
    headers: Vec<(String, String)>,
}

impl<'a> Wire<'a> {
    fn of(session: &'a Session) -> Wire<'a> {
        Wire {
            endpoint: &session.endpoint,
            token: session.token.as_deref(),
            extra: &session.headers,
            version: session.version().to_string(),
            modern: session.modern(),
            session_id: session.session_id.as_deref(),
            headers: Vec::new(),
        }
    }
}

fn client() -> Result<reqwest::Client, McpError> {
    reqwest::Client::builder()
        .build()
        .map_err(|source| McpError::Transport { endpoint: "the http client".to_string(), source })
}

async fn post(
    http: &reqwest::Client,
    wire: Wire<'_>,
    method: &str,
    name: Option<&str>,
    params: serde_json::Value,
    timeout: Duration,
) -> Result<(serde_json::Value, Option<String>), McpError> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": if wire.modern { with_meta(params, &wire.version) } else { params },
    });

    let mut request = http
        .post(wire.endpoint)
        .timeout(timeout)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", &wire.version);
    for (name, value) in wire.extra {
        request = request.header(name.as_str(), value.as_str());
    }
    if let Some(token) = wire.token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if wire.modern {
        request = request.header("mcp-method", method);
        if let Some(name) = name {
            request = request.header("mcp-name", header_value(name));
        }
        for (header, value) in &wire.headers {
            request = request.header(format!("mcp-param-{header}"), value.as_str());
        }
    } else if let Some(id) = wire.session_id {
        request = request.header("mcp-session-id", id);
    }

    let response = request
        .json(&body)
        .send()
        .await
        .map_err(|source| McpError::Transport { endpoint: wire.endpoint.to_string(), source })?;

    let status = response.status();
    let issued = header(&response, "mcp-session-id");
    let challenge = header(&response, "www-authenticate");
    let content_type = header(&response, "content-type").unwrap_or_default();
    let text = response
        .text()
        .await
        .map_err(|source| McpError::Transport { endpoint: wire.endpoint.to_string(), source })?;

    if status.as_u16() == 401 {
        return Err(McpError::Unauthorized { endpoint: wire.endpoint.to_string(), challenge });
    }

    let envelope = decode(&content_type, &text);

    if let Some(error) = envelope.as_ref().and_then(|value| value.get("error")) {
        return Err(refusal(wire.endpoint, error));
    }

    if !status.is_success() {
        return Err(McpError::Status {
            endpoint: wire.endpoint.to_string(),
            status: status.as_u16(),
            body: text.chars().take(400).collect(),
        });
    }

    let envelope = envelope.ok_or_else(|| McpError::Malformed {
        endpoint: wire.endpoint.to_string(),
        detail: format!("no JSON-RPC message in a {content_type} body"),
    })?;

    let result = envelope.get("result").cloned().ok_or_else(|| McpError::Malformed {
        endpoint: wire.endpoint.to_string(),
        detail: "a reply with neither a result nor an error".to_string(),
    })?;

    Ok((result, issued))
}

fn refusal(endpoint: &str, error: &serde_json::Value) -> McpError {
    let code = error.get("code").and_then(serde_json::Value::as_i64).unwrap_or_default();
    let message = error
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("it refused and gave no reason");
    match code {
        UNSUPPORTED_VERSION => McpError::NoSharedVersion {
            endpoint: endpoint.to_string(),
            supported: error
                .get("data")
                .and_then(|data| data.get("supported"))
                .and_then(serde_json::Value::as_array)
                .map(|all| {
                    all.iter().filter_map(serde_json::Value::as_str).map(str::to_string).collect()
                })
                .unwrap_or_default(),
        },
        HEADER_MISMATCH => {
            McpError::HeaderMismatch { endpoint: endpoint.to_string(), detail: message.to_string() }
        }
        _ => McpError::Rejected { message: message.to_string() },
    }
}

fn result_of(endpoint: &str, envelope: &serde_json::Value) -> Result<serde_json::Value, McpError> {
    if let Some(error) = envelope.get("error") {
        return Err(refusal(endpoint, error));
    }
    envelope.get("result").cloned().ok_or_else(|| McpError::Malformed {
        endpoint: endpoint.to_string(),
        detail: "a reply with neither a result nor an error".to_string(),
    })
}

fn with_meta(params: serde_json::Value, version: &str) -> serde_json::Value {
    let mut params = match params {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    params.insert(
        "_meta".to_string(),
        serde_json::json!({
            META_VERSION: version,
            META_CLIENT: client_info(),
            META_CAPABILITIES: {},
        }),
    );
    serde_json::Value::Object(params)
}

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
        .header("mcp-protocol-version", session.version());
    for (name, value) in &session.headers {
        request = request.header(name.as_str(), value.as_str());
    }
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

async fn sse_exchange(
    http: &reqwest::Client,
    dial: Dial<'_>,
    call: Option<(&str, serde_json::Value)>,
    timeout: Duration,
) -> Result<Answered, McpError> {
    let endpoint = dial.endpoint;
    let method = call.as_ref().map(|(method, _)| *method).unwrap_or("initialize").to_string();

    let work = async {
        let mut request = http.get(endpoint).header("accept", "text/event-stream");
        for (name, value) in dial.headers {
            request = request.header(name.as_str(), value.as_str());
        }
        if let Some(token) = dial.token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = request
            .send()
            .await
            .map_err(|source| McpError::Transport { endpoint: endpoint.to_string(), source })?;

        let status = response.status();
        if status.as_u16() == 401 {
            return Err(McpError::Unauthorized {
                endpoint: endpoint.to_string(),
                challenge: header(&response, "www-authenticate"),
            });
        }
        if !status.is_success() {
            return Err(McpError::Status {
                endpoint: endpoint.to_string(),
                status: status.as_u16(),
                body: String::new(),
            });
        }
        let content_type = header(&response, "content-type").unwrap_or_default();
        if !content_type.contains("text/event-stream") {
            return Err(McpError::NotAStream { endpoint: endpoint.to_string(), content_type });
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        let named =
            read_until(endpoint, &mut stream, &mut buffer, |name, _| name == "endpoint").await?.1;
        let messages = same_origin(endpoint, &named)?;

        let hello = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": LEGACY_VERSION,
                "capabilities": {},
                "clientInfo": client_info(),
            },
        });
        post_message(http, dial, &messages, &hello).await?;
        let established = reply_to(endpoint, &mut stream, &mut buffer, 1).await?;

        let agreed = established
            .get("protocolVersion")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(LEGACY_VERSION)
            .to_string();
        if !SUPPORTED.contains(&agreed.as_str()) {
            return Err(McpError::NoSharedVersion {
                endpoint: endpoint.to_string(),
                supported: vec![agreed],
            });
        }
        let server_name = established
            .get("serverInfo")
            .and_then(|info| info.get("name"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();

        let ready = serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        let _ = post_message(http, dial, &messages, &ready).await;

        let Some((method, params)) = call else {
            return Ok(Answered {
                result: serde_json::Value::Null,
                server_name,
                negotiated: agreed,
            });
        };
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": method,
            "params": params,
        });
        let result = match post_message(http, dial, &messages, &body).await? {
            Some(inline) => result_of(endpoint, &inline)?,
            None => reply_to(endpoint, &mut stream, &mut buffer, 2).await?,
        };
        Ok(Answered { result, server_name, negotiated: agreed })
    };

    tokio::time::timeout(timeout, work).await.map_err(|_| McpError::Silent {
        endpoint: endpoint.to_string(),
        method,
        secs: timeout.as_secs(),
    })?
}

async fn sse_probe(http: &reqwest::Client, dial: Dial<'_>) -> Result<(String, String), McpError> {
    let answered = sse_exchange(http, dial, None, CONNECT_TIMEOUT).await?;
    Ok((answered.server_name, answered.negotiated))
}

struct Answered {
    result: serde_json::Value,
    server_name: String,
    negotiated: String,
}

async fn post_message(
    http: &reqwest::Client,
    dial: Dial<'_>,
    messages: &str,
    body: &serde_json::Value,
) -> Result<Option<serde_json::Value>, McpError> {
    let mut request = http
        .post(messages)
        .timeout(CONNECT_TIMEOUT)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    for (name, value) in dial.headers {
        request = request.header(name.as_str(), value.as_str());
    }
    if let Some(token) = dial.token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = request
        .json(body)
        .send()
        .await
        .map_err(|source| McpError::Transport { endpoint: messages.to_string(), source })?;

    let status = response.status();
    let challenge = header(&response, "www-authenticate");
    let content_type = header(&response, "content-type").unwrap_or_default();
    let text = response
        .text()
        .await
        .map_err(|source| McpError::Transport { endpoint: messages.to_string(), source })?;

    if status.as_u16() == 401 {
        return Err(McpError::Unauthorized { endpoint: messages.to_string(), challenge });
    }
    if !status.is_success() {
        return Err(McpError::Status {
            endpoint: messages.to_string(),
            status: status.as_u16(),
            body: text.chars().take(400).collect(),
        });
    }
    Ok(decode(&content_type, &text).filter(|value| value.get("jsonrpc").is_some()))
}

async fn reply_to<S, T>(
    endpoint: &str,
    stream: &mut S,
    buffer: &mut String,
    id: i64,
) -> Result<serde_json::Value, McpError>
where
    S: futures_util::Stream<Item = Result<T, reqwest::Error>> + Unpin,
    T: AsRef<[u8]>,
{
    let matching = |_: &str, data: &str| {
        serde_json::from_str::<serde_json::Value>(data)
            .is_ok_and(|value| value.get("id").and_then(serde_json::Value::as_i64) == Some(id))
    };
    let (_, data) = read_until(endpoint, stream, buffer, matching).await?;
    let envelope: serde_json::Value =
        serde_json::from_str(&data).map_err(|err| McpError::Malformed {
            endpoint: endpoint.to_string(),
            detail: format!("an event on its stream did not parse: {err}"),
        })?;
    result_of(endpoint, &envelope)
}

async fn read_until<S, T>(
    endpoint: &str,
    stream: &mut S,
    buffer: &mut String,
    want: impl Fn(&str, &str) -> bool,
) -> Result<(String, String), McpError>
where
    S: futures_util::Stream<Item = Result<T, reqwest::Error>> + Unpin,
    T: AsRef<[u8]>,
{
    use futures_util::StreamExt;
    loop {
        while let Some((name, data)) = take_event(buffer) {
            if want(&name, &data) {
                return Ok((name, data));
            }
        }
        let Some(chunk) = stream.next().await else {
            return Err(McpError::Malformed {
                endpoint: endpoint.to_string(),
                detail: "its event stream closed before it answered".to_string(),
            });
        };
        let chunk = chunk
            .map_err(|source| McpError::Transport { endpoint: endpoint.to_string(), source })?;
        buffer.push_str(&String::from_utf8_lossy(chunk.as_ref()));
    }
}

fn take_event(buffer: &mut String) -> Option<(String, String)> {
    let end = [buffer.find("\n\n").map(|at| (at, 2)), buffer.find("\r\n\r\n").map(|at| (at, 4))]
        .into_iter()
        .flatten()
        .min_by_key(|(at, _)| *at)?;
    let (at, width) = end;
    let block: String = buffer.drain(..at + width).collect();

    let mut name = String::new();
    let mut data = String::new();
    for raw in block.lines() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix("event:") {
            name = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if name.is_empty() {
        name = "message".to_string();
    }
    Some((name, data))
}

fn same_origin(endpoint: &str, named: &str) -> Result<String, McpError> {
    let named = named.trim();
    let refuse = |detail: String| McpError::Malformed { endpoint: endpoint.to_string(), detail };
    if named.is_empty() {
        return Err(refuse("its event stream named an empty message endpoint".to_string()));
    }
    let origin = origin_of(endpoint)
        .ok_or_else(|| refuse(format!("{endpoint} is not an address with an origin")))?;

    let resolved = if named.contains("://") {
        let theirs = origin_of(named)
            .ok_or_else(|| refuse(format!("its event stream named {named}, which is not a URL")))?;
        if theirs != origin {
            return Err(refuse(format!(
                "its event stream named {named} for messages, which is a different server; a \
                 group's credential is not sent to an address the operator did not name"
            )));
        }
        named.to_string()
    } else if let Some(path) = named.strip_prefix('/') {
        format!("{origin}/{path}")
    } else {
        let path = endpoint.strip_prefix(&origin).unwrap_or("");
        let base = path.rsplit_once('/').map(|(before, _)| before).unwrap_or("");
        format!("{origin}{base}/{named}")
    };
    Ok(resolved)
}

fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return None;
    }
    Some(format!("{}://{}", scheme.to_ascii_lowercase(), authority.to_ascii_lowercase()))
}

fn header(response: &reqwest::Response, name: &str) -> Option<String> {
    response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
}

const TCHAR: &str = "!#$%&'*+-.^_`|~";

fn mirrored_params(schema: &serde_json::Value) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut found: Vec<(String, Vec<String>)> = Vec::new();
    let mut path = Vec::new();
    reachable(schema, &mut path, &mut found)?;
    let annotated = annotations(schema);
    if annotated != found.len() {
        return Err(format!(
            "{} of its {annotated} x-mcp-header annotations are not reachable through `properties`",
            annotated - found.len()
        ));
    }
    Ok(found)
}

fn reachable(
    node: &serde_json::Value,
    path: &mut Vec<String>,
    out: &mut Vec<(String, Vec<String>)>,
) -> Result<(), String> {
    let Some(properties) = node.get("properties").and_then(serde_json::Value::as_object) else {
        return Ok(());
    };
    for (key, sub) in properties {
        if let Some(named) = sub.get("x-mcp-header") {
            let named =
                named.as_str().ok_or_else(|| format!("{key}'s x-mcp-header is not text"))?;
            if named.is_empty()
                || !named.chars().all(|c| c.is_ascii_alphanumeric() || TCHAR.contains(c))
            {
                return Err(format!("{key}'s x-mcp-header {named:?} is not a header name"));
            }
            if out.iter().any(|(taken, _)| taken.eq_ignore_ascii_case(named)) {
                return Err(format!("two parameters both mirror into {named:?}"));
            }
            match sub.get("type").and_then(serde_json::Value::as_str) {
                Some("string" | "integer" | "boolean") => {}
                other => {
                    return Err(format!(
                        "{key} mirrors into {named:?} and is {}, which cannot be a header",
                        other.unwrap_or("untyped")
                    ))
                }
            }
            path.push(key.clone());
            out.push((named.to_string(), path.clone()));
            path.pop();
        }
        path.push(key.clone());
        reachable(sub, path, out)?;
        path.pop();
    }
    Ok(())
}

fn annotations(node: &serde_json::Value) -> usize {
    match node {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(key, value)| usize::from(key == "x-mcp-header") + annotations(value))
            .sum(),
        serde_json::Value::Array(all) => all.iter().map(annotations).sum(),
        _ => 0,
    }
}

fn mirror(schema: &serde_json::Value, arguments: &serde_json::Value) -> Vec<(String, String)> {
    let Ok(wanted) = mirrored_params(schema) else { return Vec::new() };
    let mut out = Vec::new();
    for (name, path) in wanted {
        let mut at = arguments;
        for key in &path {
            let Some(next) = at.get(key) else {
                at = &serde_json::Value::Null;
                break;
            };
            at = next;
        }
        let rendered = match at {
            serde_json::Value::String(text) => text.clone(),
            serde_json::Value::Bool(yes) => yes.to_string(),
            serde_json::Value::Number(n) if n.is_i64() || n.is_u64() => n.to_string(),
            _ => continue,
        };
        out.push((name, header_value(&rendered)));
    }
    out
}

fn header_value(raw: &str) -> String {
    let plain = raw.chars().all(|c| c == '\t' || ('\u{20}'..='\u{7e}').contains(&c))
        && raw.trim() == raw
        && !(raw.starts_with("=?base64?") && raw.ends_with("?="));
    if plain {
        raw.to_string()
    } else {
        format!("=?base64?{}?=", crate::e2b::encode(raw.as_bytes()))
    }
}

fn render_content(result: &serde_json::Value) -> String {
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
    fn discovery_page_transitions_are_bounded() {
        let mut seen = std::collections::HashSet::new();
        let first = DiscoveryPage::First.advance(Some("a".into()), &mut seen).unwrap();
        assert_eq!(first, DiscoveryPage::Next("a".into()));
        let second = first.advance(Some("b".into()), &mut seen).unwrap();
        assert_eq!(second, DiscoveryPage::Next("b".into()));
        assert_eq!(second.advance(None, &mut seen), Ok(DiscoveryPage::Complete));
        assert_eq!(DiscoveryPage::First.advance(None, &mut seen), Ok(DiscoveryPage::Complete));
        assert!(DiscoveryPage::Complete.advance(None, &mut seen).is_err());
        assert!(DiscoveryPage::First.advance(Some("a".into()), &mut seen).is_err());
        assert!(DiscoveryPage::First.advance(Some(String::new()), &mut seen).is_err());
        seen.extend((0..98).map(|n| n.to_string()));
        assert!(DiscoveryPage::First.advance(Some("overflow".into()), &mut seen).is_err());
    }

    #[test]
    fn an_event_carries_its_name_as_well_as_its_data() {
        let mut buffer = String::from("event: endpoint\ndata: /messages?s=1\n\n");
        assert_eq!(take_event(&mut buffer), Some(("endpoint".into(), "/messages?s=1".into())));
        assert!(buffer.is_empty());
    }

    #[test]
    fn an_unnamed_event_is_a_message() {
        let mut buffer = String::from("data: {\"id\":1}\n\n");
        assert_eq!(take_event(&mut buffer), Some(("message".into(), "{\"id\":1}".into())));
    }

    #[test]
    fn an_event_split_across_chunks_waits_for_the_rest() {
        let mut buffer = String::from("event: message\ndata: {\"id\":2,\"resu");
        assert_eq!(take_event(&mut buffer), None);
        buffer.push_str("lt\":{}}\n\n");
        let (name, data) = take_event(&mut buffer).unwrap();
        assert_eq!(name, "message");
        assert_eq!(serde_json::from_str::<serde_json::Value>(&data).unwrap()["id"], 2);
    }

    #[test]
    fn crlf_framing_is_the_same_event() {
        let mut buffer = String::from("event: endpoint\r\ndata: /m\r\n\r\n");
        assert_eq!(take_event(&mut buffer), Some(("endpoint".into(), "/m".into())));
    }

    #[test]
    fn a_message_endpoint_is_resolved_against_the_stream_it_was_named_on() {
        let at = "https://box.example.com/sse";
        assert_eq!(
            same_origin(at, "/messages?s=1").unwrap(),
            "https://box.example.com/messages?s=1"
        );
        assert_eq!(
            same_origin(at, "messages?s=1").unwrap(),
            "https://box.example.com/messages?s=1"
        );
        assert_eq!(
            same_origin(at, "https://box.example.com/messages").unwrap(),
            "https://box.example.com/messages"
        );
    }

    #[test]
    fn a_message_endpoint_on_another_server_is_refused() {
        let refused = same_origin("https://box.example.com/sse", "https://evil.example/m");
        let message = refused.unwrap_err().to_string();
        assert!(message.contains("different server"), "{message}");
        assert!(message.contains("did not name"), "{message}");
    }

    #[test]
    fn every_revision_this_build_speaks_sorts_as_a_date() {
        let mut sorted = SUPPORTED;
        sorted.sort_unstable();
        sorted.reverse();
        assert_eq!(sorted, SUPPORTED, "SUPPORTED has to be newest first");
        assert!(modern(SUPPORTED[0]));
        assert!(!modern("2024-11-05"), "the transport this build falls back to is not modern");
    }

    #[test]
    fn a_json_reply_decodes() {
        let value = decode("application/json", r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#);
        assert_eq!(value.unwrap()["result"]["ok"], serde_json::json!(true));
    }

    #[test]
    fn an_event_stream_reply_decodes() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":1}}\n\n";
        let value = decode("text/event-stream; charset=utf-8", body);
        assert_eq!(value.unwrap()["result"]["ok"], serde_json::json!(1));
    }

    #[test]
    fn an_event_split_across_data_lines_is_joined_before_it_is_parsed() {
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

    #[test]
    fn the_newest_revision_is_the_one_a_modern_request_declares() {
        assert_eq!(PROTOCOL_VERSION, "2026-07-28");
        assert_eq!(
            agreed(&["2025-06-18".into(), "2025-11-25".into()]),
            Some("2025-11-25".to_string())
        );
        assert_eq!(agreed(&["2026-07-28".into()]), Some("2026-07-28".to_string()));
        assert_eq!(agreed(&["1999-01-01".into()]), None);
        assert_eq!(agreed(&[]), None);
    }

    #[test]
    fn a_revision_sorts_as_a_date_because_it_is_one() {
        assert!(modern(PROTOCOL_VERSION));
        assert!(modern("2030-01-01"));
        assert!(!modern(LEGACY_VERSION));
        assert!(!modern("2025-06-18"));
        assert!(!modern("2024-11-05"));
    }

    #[test]
    fn the_handshake_revision_is_one_this_build_speaks() {
        assert!(SUPPORTED.contains(&LEGACY_VERSION));
        assert_ne!(LEGACY_VERSION, PROTOCOL_VERSION);
    }

    #[test]
    fn a_modern_request_carries_its_version_where_the_server_compares_it() {
        let params = with_meta(serde_json::json!({ "name": "run_sql" }), "2026-07-28");
        assert_eq!(params["name"], serde_json::json!("run_sql"));
        assert_eq!(params["_meta"][META_VERSION], serde_json::json!("2026-07-28"));
        assert_eq!(params["_meta"][META_CLIENT]["name"], serde_json::json!("Guaca"));
        assert!(params["_meta"][META_CAPABILITIES].is_object());
    }

    #[test]
    fn a_modern_server_names_itself_under_the_meta_key_rather_than_beside_it() {
        let discovered = serde_json::json!({
            "supportedVersions": ["2026-07-28"],
            "_meta": { META_SERVER: { "name": "Scripted", "version": "1" } },
        });
        assert_eq!(server_name(&discovered), "Scripted");
        assert_eq!(server_name(&serde_json::json!({})), "");
    }

    #[test]
    fn a_header_carries_a_plain_value_and_hides_the_rest_behind_the_marker() {
        assert_eq!(header_value("us-west1"), "us-west1");
        assert_eq!(header_value(""), "");
        assert_eq!(header_value("Hello, 世界"), "=?base64?SGVsbG8sIOS4lueVjA==?=");
        assert_eq!(header_value(" padded "), "=?base64?IHBhZGRlZCA=?=");
        assert_eq!(header_value("line1\nline2"), "=?base64?bGluZTEKbGluZTI=?=");
        assert_eq!(header_value("=?base64?literal?="), "=?base64?PT9iYXNlNjQ/bGl0ZXJhbD89?=");
    }

    #[test]
    fn a_nested_parameter_is_mirrored_from_where_it_actually_lives() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "region": { "type": "string", "x-mcp-header": "Region" },
                "query": { "type": "string" },
                "opts": {
                    "type": "object",
                    "properties": { "dry": { "type": "boolean", "x-mcp-header": "Dry" } },
                },
            },
        });
        let mut wanted = mirrored_params(&schema).expect("a reachable annotation");
        wanted.sort();
        assert_eq!(
            wanted,
            vec![
                ("Dry".to_string(), vec!["opts".to_string(), "dry".to_string()]),
                ("Region".to_string(), vec!["region".to_string()]),
            ]
        );

        let sent = serde_json::json!({ "region": "us-west1", "opts": { "dry": true } });
        let mut sent = mirror(&schema, &sent);
        sent.sort();
        assert_eq!(
            sent,
            vec![
                ("Dry".to_string(), "true".to_string()),
                ("Region".to_string(), "us-west1".to_string()),
            ]
        );
    }

    #[test]
    fn a_parameter_this_call_did_not_send_is_left_out_rather_than_sent_empty() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "region": { "type": "string", "x-mcp-header": "Region" } },
        });
        assert!(mirror(&schema, &serde_json::json!({})).is_empty());
        assert!(mirror(&schema, &serde_json::json!({ "region": null })).is_empty());
    }

    #[test]
    fn an_annotation_that_is_not_reachable_through_properties_invalidates_the_tool() {
        for hidden in [
            serde_json::json!({
                "type": "object",
                "properties": {
                    "rows": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string", "x-mcp-header": "Id" },
                            },
                        },
                    },
                },
            }),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "who": { "oneOf": [{ "type": "string", "x-mcp-header": "Who" }] },
                },
            }),
        ] {
            assert!(mirrored_params(&hidden).is_err(), "{hidden}");
        }
    }

    #[test]
    fn a_header_name_a_server_could_not_send_is_refused_rather_than_sanitized() {
        for bad in ["", "with space", "with\r\n", "semi;colon"] {
            let schema = serde_json::json!({
                "type": "object",
                "properties": { "a": { "type": "string", "x-mcp-header": bad } },
            });
            assert!(mirrored_params(&schema).is_err(), "{bad:?}");
        }
        let float = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "number", "x-mcp-header": "A" } },
        });
        assert!(mirrored_params(&float).is_err());
        let clash = serde_json::json!({
            "type": "object",
            "properties": {
                "a": { "type": "string", "x-mcp-header": "Region" },
                "b": { "type": "string", "x-mcp-header": "region" },
            },
        });
        assert!(mirrored_params(&clash).is_err());
    }

    #[test]
    fn a_schema_with_no_annotations_mirrors_nothing_and_is_not_an_error() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "sql": { "type": "string" } },
        });
        assert_eq!(mirrored_params(&schema), Ok(Vec::new()));
        assert!(mirror(&schema, &serde_json::json!({ "sql": "select 1" })).is_empty());
        assert_eq!(mirrored_params(&serde_json::json!({})), Ok(Vec::new()));
    }

    #[test]
    fn a_server_that_shares_no_revision_says_so_in_terms_of_both_lists() {
        let refusal = McpError::NoSharedVersion {
            endpoint: "https://example.test/mcp".into(),
            supported: vec!["2030-01-01".into()],
        }
        .to_string();
        assert!(refusal.contains("2030-01-01"), "{refusal}");
        assert!(refusal.contains(PROTOCOL_VERSION), "{refusal}");
        assert!(refusal.contains("update Guaca"), "{refusal}");
    }
}
