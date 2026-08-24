//! The client end of the Model Context Protocol, over streamable HTTP.
//!
//! Small on purpose. Guaca is not a general MCP host: it dials a server, asks
//! what it can do, and calls those tools on behalf of an agent. Three methods
//! cover all of that — `tools/list`, `tools/call`, and whichever handshake the
//! server's protocol era wants — and the transport is one POST each.
//!
//! ## Two eras, and which one a server is
//!
//! Revision `2026-07-28` deleted the handshake. Before it, a session was
//! established with `initialize`, the agreed version came back in the reply,
//! and the server could mint a session id every later request had to carry.
//! After it there is no session at all: every POST stands alone and declares
//! its own protocol version, in `_meta` in the body and in a header beside it.
//!
//! Both are in the field and will be for years, so this client speaks both. It
//! finds out which by trying the modern one first, exactly as the spec's
//! backward-compatibility section says to: `server/discover` is mandatory for a
//! modern server, so an answer to it — or a refusal in one of the shapes only a
//! modern server produces — identifies the era. Anything else is a server that
//! wants `initialize`.
//!
//! The answer is remembered per endpoint for the life of the process. That is
//! not the session cache this file argues against below: an era is a property
//! of the deployed server rather than of a grant, it cannot expire, and a
//! server upgraded underneath a running Guaca fails once and re-probes. Without
//! it every plugin call on a legacy server would pay for a probe it already
//! knows the answer to, which is one internet round trip in front of every tool
//! call in the crew.
//!
//! ## Two content types, one request
//!
//! A streamable-HTTP server may answer a POST with `application/json` or with
//! `text/event-stream`, and it chooses per request. Both are in use across the
//! servers on the list: one answers every call as an event stream including the
//! handshake, and Neon's answers in JSON. Neither is wrong and the spec requires
//! a client to accept both, which is why the reply is parsed by sniffing the
//! content type rather than by trusting the one a server used last. This is not
//! the streaming case `llm/sse.rs` handles: nothing here is shown as it arrives,
//! so the body is read whole and the one JSON-RPC object in it is pulled out.
//!
//! ## The session header
//!
//! Legacy only. A server may hand back `Mcp-Session-Id` on `initialize` and then
//! require it on every later request. It may also not, and then requires that
//! the header is absent. Both are legal, so the id is whatever the server said
//! and nothing is invented. A modern server is told to ignore the header and
//! mints none, so a modern session never carries one.
//!
//! ## What a 401 means here
//!
//! It is the start of the sign-in, not a failure, and it is answered before the
//! era is even known. An unauthenticated first request is how Guaca discovers
//! whether a server needs a grant at all, and `WWW-Authenticate` on the refusal
//! is where the spec puts the address of the metadata that says who can issue
//! one. `oauth.rs` reads it.
//!
//! ## Two transports, and who gets the older one
//!
//! Streamable HTTP is one POST per request and is what everything here assumes.
//! The transport it replaced — HTTP+SSE, revision `2024-11-05` — is a GET that
//! stays open, an `endpoint` event naming a second URL, and every request POSTed
//! to that URL with its reply arriving back down the stream. Deprecated since
//! `2025-03-26`, and still what a great many self-hosted servers speak, because
//! the framework somebody deployed two years ago has not been updated.
//!
//! Guaca falls back to it for a server the operator added and not for one on the
//! catalog, and the asymmetry is the catalog's whole argument rather than an
//! inconsistency. A vendor Guaca vouches for is a vendor Guaca can hold to the
//! current transport; a box in somebody's own network is not a vendor, and
//! refusing to speak to it is not a migration incentive, it is a plugin that
//! does not work. [`Dial::legacy_transport`] is the switch and `plugins.rs` is
//! the only thing that sets it.
//!
//! ## What is deliberately not here
//!
//! Resources, prompts, `subscriptions/listen` and the server-to-client input
//! requests a modern server can embed in a result: an agent here is offered
//! tools and nothing else, and a half-implemented capability is worse than an
//! absent one.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Deserialize;

/// The revisions Guaca speaks, newest first.
///
/// Order is the preference order: the first is what a modern request declares,
/// and a server that refuses it names what it has instead, out of which the
/// first of these that appears is taken.
///
/// The two oldest are here for the servers an operator runs rather than for the
/// catalog, which negotiates down to neither. `tools/list` and `tools/call` are
/// the only methods this client has ever sent and both are unchanged across all
/// five, so the cost of accepting an old revision is nothing and the cost of
/// refusing one is a working server Guaca will not talk to. A server that
/// answers `2024-11-05` is usually on the older transport as well: see
/// [`Transport`].
pub const SUPPORTED: [&str; 5] =
    ["2026-07-28", "2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// What a modern request declares, and the newest revision this build knows.
pub const PROTOCOL_VERSION: &str = SUPPORTED[0];

/// What `initialize` asks for on a server that turned out to be legacy.
///
/// The newest revision that still has a handshake. A legacy server negotiates
/// down from it and says in its reply what it settled on, and that answer is
/// what every later request carries.
pub const LEGACY_VERSION: &str = "2025-11-25";

/// The first revision with no handshake. At or after it a request stands alone.
///
/// A revision is a date, and a date in `YYYY-MM-DD` sorts as a string exactly
/// as it sorts as a date, so "is this one modern" is a comparison rather than a
/// second list to keep in step with the first.
const FIRST_MODERN: &str = "2026-07-28";

/// Whether a revision is one with no handshake.
///
/// Load-bearing in one place that is easy to miss: a modern-shaped refusal can
/// name a version that is *not* modern. A dual-era server asked for something
/// it lacks may answer `UnsupportedProtocolVersionError` listing `2025-11-25`,
/// and that is not an invitation to ask again in the modern shape — it is the
/// server saying to shake hands. Retrying modernly there is refused all over
/// again, and the plugin never connects.
fn modern(version: &str) -> bool {
    version >= FIRST_MODERN
}

/// The `_meta` keys a modern request carries, spelled as the spec spells them.
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CLIENT: &str = "io.modelcontextprotocol/clientInfo";
const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_SERVER: &str = "io.modelcontextprotocol/serverInfo";

/// The two JSON-RPC error codes the modern revision defines for itself.
///
/// Recognizing them is what tells a modern server apart from a legacy one, so
/// they are load-bearing rather than decoration: the spec's fallback rule is
/// that a `400` carrying one of these is a modern server to retry against, and
/// a `400` carrying anything else is a legacy server to hand `initialize` to.
const UNSUPPORTED_VERSION: i64 = -32022;
const HEADER_MISMATCH: i64 = -32020;

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
        /// says which authorization server can issue a grant for this resource,
        /// and the scopes this resource wants asked for.
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
    /// A modern server that shares no protocol revision with this build.
    ///
    /// Its own sentence because nothing but a new Guaca fixes it, and the
    /// operator has no way to tell that from the endpoint being wrong.
    #[error(
        "{endpoint} speaks MCP {}, and this build speaks {}. Nothing here will connect it; \
         update Guaca.",
        .supported.join(" or "),
        SUPPORTED.join(", ")
    )]
    NoSharedVersion { endpoint: String, supported: Vec<String> },
    /// Guaca sent a header that disagreed with its own request body. A bug
    /// here rather than anything the operator did, and it says so.
    #[error("{endpoint} refused the request as malformed ({detail}); this is a bug in Guaca")]
    HeaderMismatch { endpoint: String, detail: String },
    /// A GET that answered with something other than an event stream.
    ///
    /// Its own variant rather than a `Malformed` with a sentence in it, because
    /// the era probe has to tell "this address is not on the older transport"
    /// apart from "it is, and something on the stream was wrong": the first
    /// keeps the refusal the current transport already gave, and the second
    /// replaces it. A message match would put that decision in a string.
    #[error("{endpoint} answered a GET with {content_type}, not an event stream")]
    NotAStream { endpoint: String, content_type: String },
    /// An event stream that opened and then said nothing.
    ///
    /// Its own sentence rather than a timeout on the transport, because it is
    /// the one failure the older transport has that the current one does not:
    /// the socket is up, the request was accepted, and the answer was supposed
    /// to arrive on a stream that is still technically connected.
    #[error("{endpoint} opened an event stream and did not answer {method} on it within {secs}s")]
    Silent { endpoint: String, method: String, secs: u64 },
}

impl McpError {
    /// Whether signing in again is the thing that would fix this.
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, McpError::Unauthorized { .. })
    }
}

/// Which shape of the protocol a server turned out to speak.
///
/// Remembered per endpoint rather than per session: see the module docs for why
/// this is not the cached session the file argues against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Era {
    /// `2026-07-28` and later. No handshake and no session: every POST declares
    /// its own version, in `_meta` and in the header beside it. The version is
    /// carried because it is negotiated once, when the server is first probed,
    /// and a later request has nothing else to read it from.
    Modern { version: String },
    /// `2025-11-25` and earlier. `initialize` first, the agreed version in its
    /// reply, and a session id on every later request if the server minted one.
    Legacy,
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

/// Which of the two transports a server turned out to want.
///
/// Remembered per endpoint beside the era, and for the same reasons: it is a
/// property of the deployed server, it cannot expire, and re-probing it in
/// front of every tool call would be an internet round trip the answer to which
/// is already known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    /// One POST per request, the reply in its response. Everything current.
    Streamable,
    /// `2024-11-05`: a GET that stays open, an `endpoint` event naming a second
    /// URL, every request POSTed there and every reply arriving on the stream.
    ///
    /// Only ever reached for a server the operator added. See the module docs.
    Sse,
}

/// Everything needed to open a session, and nothing that comes back from one.
///
/// A struct rather than four arguments because two of them are `Option`-shaped
/// booleans that read identically at a call site, and the failure of getting
/// them the wrong way round is a crew's credential sent to a server that was
/// never asked whether it wanted one.
#[derive(Clone, Copy)]
pub struct Dial<'a> {
    pub endpoint: &'a str,
    /// The grant, a pasted key, or nothing. Sent as `Authorization: Bearer`.
    pub token: Option<&'a str>,
    /// What the operator gave this server beyond a credential: an API-key
    /// header, a pair of Access headers, a tenant id. Empty for the catalog.
    pub headers: &'a [(String, String)],
    /// Whether this server may be spoken to over the transport that streamable
    /// HTTP replaced.
    ///
    /// True only for a server the operator added. A vendor on the catalog is
    /// one Guaca vouched for, and vouching includes the transport; a box in
    /// somebody's own network is not a vendor, and refusing it is not a
    /// migration incentive. See the module docs.
    pub legacy_transport: bool,
}

impl<'a> Dial<'a> {
    /// An address and nothing else: no credential, no headers, current
    /// transport only. What a test and the era probe want.
    pub fn to(endpoint: &'a str) -> Dial<'a> {
        Dial { endpoint, token: None, headers: &[], legacy_transport: false }
    }

    pub fn with_token(self, token: Option<&'a str>) -> Dial<'a> {
        Dial { token, ..self }
    }
}

/// What a request needs to carry, and what the handshake established.
///
/// A modern session is established by nothing at all — it is the era, the
/// negotiated version and the credential, which is everything a standalone POST
/// needs. A legacy one is the same plus whatever `initialize` said.
#[derive(Debug, Clone)]
pub struct Session {
    pub endpoint: String,
    pub token: Option<String>,
    /// The operator's own headers, on every request this session makes.
    ///
    /// Owned rather than borrowed because a session outlives the call that
    /// opened it by exactly one await point per request, and a lifetime here
    /// would spread through every caller for a vector that is almost always
    /// empty and never longer than eight.
    headers: Vec<(String, String)>,
    pub era: Era,
    pub transport: Transport,
    /// Whatever the server said, or nothing. Never invented: a server that
    /// issued no session id rejects a request that carries one, and a modern
    /// server never issues one.
    pub session_id: Option<String>,
    /// How the server names itself, when the handshake was the thing that
    /// asked. Empty on a modern session, where nothing has asked yet:
    /// [`describe`] is what asks, and only the connect path needs it.
    pub server_name: String,
    /// What `initialize` settled on, for a legacy session.
    ///
    /// Not on [`Era`] and not merged into it, because the era is what is
    /// remembered per endpoint and this is not: a modern server negotiates once
    /// and every later POST declares the answer, while a legacy one negotiates
    /// inside every handshake and would have a remembered version overwritten
    /// on the next one anyway.
    negotiated: Option<String>,
}

impl Session {
    /// Whether this session talks over the transport streamable HTTP replaced.
    pub fn sse(&self) -> bool {
        self.transport == Transport::Sse
    }

    /// The revision every request on this session declares.
    fn version(&self) -> &str {
        match &self.era {
            Era::Modern { version } => version,
            // A legacy session carries the revision `initialize` settled on,
            // and there is exactly one place it could have come from.
            Era::Legacy => self.negotiated.as_deref().unwrap_or(LEGACY_VERSION),
        }
    }

    /// Whether this session is on the revision that deleted the handshake.
    pub fn modern(&self) -> bool {
        matches!(self.era, Era::Modern { .. })
    }

    /// The revision the two of them settled on, for whoever is reporting it.
    pub fn protocol(&self) -> &str {
        self.version()
    }
}

/// Opens a session, which is also how Guaca finds out whether a grant is needed.
///
/// Called with no token first, on purpose. A server that authorizes everybody
/// answers, and the operator is never sent to a browser to authorize something
/// that was already open; one that does not answers 401 and says where its
/// authorization server is. That happens before the era is known, because a 401
/// is the same answer in both.
pub async fn open(dial: Dial<'_>) -> Result<Session, McpError> {
    let http = client()?;

    // What this endpoint was last time. A remembered answer that turns out to
    // be wrong is a server upgraded underneath a running Guaca: the handshake
    // it wanted yesterday is a `400` today. One failure, then the truth.
    //
    // Only a protocol-level failure means that. A 401 is the sign-in and says
    // nothing about the era, and a transport failure says nothing about
    // anything, so both are handed back rather than spent on a second probe of
    // a server that is not answering.
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

/// A session for an era and a transport that have already been decided.
///
/// Free for a modern server, which is the whole of what that revision changed:
/// there is nothing to establish, because every POST carries what it needs.
/// Free for the older transport too, but for the opposite reason: its handshake
/// belongs to a stream that has not been opened yet, so every request on it
/// shakes hands inside itself and there is nothing a session could hold. Only a
/// legacy server on the current transport establishes anything here.
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
    };
    match (&era, &transport) {
        (Era::Modern { .. }, _) => Ok(free(era.clone(), None)),
        (Era::Legacy, Transport::Sse) => Ok(free(Era::Legacy, negotiated)),
        (Era::Legacy, Transport::Streamable) => initialize(http, dial).await,
    }
}

/// How the server names itself.
///
/// Free on a legacy session over the current transport, because the handshake
/// it already paid for said so. A round trip on the other two, for opposite
/// reasons: a modern session has no handshake and nothing has asked, and a
/// session on the older transport has a handshake that belonged to a stream
/// which has already been dropped. Only the connect path calls this. A tool
/// call must never pay for a label nobody reads.
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

/// This session, as the address and credential it was opened with.
///
/// The older transport takes a `Dial` rather than a session, because every
/// exchange on it establishes its own: the session on that side is the stream,
/// and the stream lasts one request.
fn dial_for(session: &Session) -> Dial<'_> {
    Dial {
        endpoint: &session.endpoint,
        token: session.token.as_deref(),
        headers: &session.headers,
        legacy_transport: true,
    }
}

/// One request, over whichever transport this session turned out to want.
///
/// The one place the two meet, and everything above it is written once. A
/// transport branch inside `list_tools` or `call_tool` would be the same
/// decision taken twice, and the second copy is the one that gets forgotten
/// when a third method is added.
async fn request(
    session: &Session,
    method: &str,
    name: Option<&str>,
    params: serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value, McpError> {
    request_mirroring(session, method, name, params, Vec::new(), timeout).await
}

/// The same, for the one call that has arguments a modern server wants mirrored.
async fn request_mirroring(
    session: &Session,
    method: &str,
    name: Option<&str>,
    params: serde_json::Value,
    mirrored: Vec<(String, String)>,
    timeout: Duration,
) -> Result<serde_json::Value, McpError> {
    let http = client()?;
    if session.sse() {
        // Mirroring is a modern-server rule and a modern server is never on
        // this transport, so there is nothing here to drop.
        let answered =
            sse_exchange(&http, dial_for(session), Some((method, params)), timeout).await?;
        // What this endpoint is remembered as, kept in step with what its last
        // handshake actually settled on. A session on this transport has
        // nowhere of its own to hold the answer, so a report built without this
        // would name the revision Guaca asked for rather than the one agreed.
        settled(&session.endpoint, &answered.negotiated);
        return Ok(answered.result);
    }
    let mut wire = Wire::of(session);
    wire.headers = mirrored;
    post(&http, wire, method, name, params, timeout).await.map(|(result, _)| result)
}

/// Everything this server offers, once.
///
/// A tool whose `x-mcp-header` annotations a modern server could not honor is
/// dropped rather than offered, because the spec makes the client responsible
/// for mirroring them and a call that cannot be built correctly is refused by
/// the server with an error no model can act on. Only on a modern session: on a
/// legacy one the annotation means nothing, and dropping the tool would take a
/// working capability away over a field nobody reads.
pub async fn list_tools(session: &Session) -> Result<Vec<ToolDescriptor>, McpError> {
    let value =
        request(session, "tools/list", None, serde_json::json!({}), CONNECT_TIMEOUT).await?;

    #[derive(Deserialize)]
    struct Listed {
        #[serde(default)]
        tools: Vec<ToolDescriptor>,
    }

    let listed = serde_json::from_value::<Listed>(value).map_err(|err| McpError::Malformed {
        endpoint: session.endpoint.clone(),
        detail: format!("its tool list did not parse: {err}"),
    })?;

    if !session.modern() {
        return Ok(listed.tools);
    }
    Ok(listed
        .tools
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

/// Runs one tool and renders its answer as the text an agent reads.
///
/// A refusal the server understood comes back as `Rejected` rather than as an
/// error on the transport, because those are different things to an agent: one
/// is "that call was wrong, here is why", the other is "the plugin is not
/// reachable". Only the first is worth rewording and trying again.
///
/// `schema` is the tool's own `inputSchema`, as the server published it and the
/// store kept it. It is passed rather than looked up because a modern server
/// may ask for some of a call's arguments to be mirrored into HTTP headers, and
/// the schema is where it says which: a call built without it is refused as a
/// header mismatch. `None` is a tool with no schema, which takes no arguments
/// and therefore mirrors nothing.
pub async fn call_tool(
    session: &Session,
    tool: &str,
    arguments: &serde_json::Value,
    schema: Option<&serde_json::Value>,
) -> Result<String, McpError> {
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

    Ok(if rendered.is_empty() { format!("{tool} returned nothing.") } else { rendered })
}

// ---- era ------------------------------------------------------------------

/// What a server turned out to be, and what it called itself while saying so.
struct Probed {
    spoken: Spoken,
    server_name: String,
}

/// The two things about a server that are fixed once it has been asked.
///
/// One value rather than two memos, because they are decided by one probe and
/// a build that remembered them separately could hold a transport for an
/// endpoint whose era it had just forgotten.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Spoken {
    era: Era,
    transport: Transport,
    /// What the last handshake on this endpoint settled on, for a server on the
    /// older transport. `None` everywhere else: a modern session declares its
    /// own revision and a legacy streamable one is told by `initialize` on
    /// every open, so only this transport has an answer with nowhere to live.
    negotiated: Option<String>,
}

/// Which era this endpoint speaks, by asking it something only a modern server
/// answers.
///
/// `server/discover` is mandatory for a modern server, so its answer is the
/// probe. The refusals are what carry the information, and each is a different
/// server:
///
/// - A result: modern, at the revision that was asked for.
/// - `UnsupportedProtocolVersionError`: modern, at a revision it names. The
///   spec says to retry with a mutually supported one rather than fall back,
///   because a modern error can only come from a modern server.
/// - `HeaderMismatch`: also modern, and a bug here. Raised rather than fallen
///   back from: falling back would hide it behind a legacy handshake that
///   happens to work.
/// - A transport failure or a 401: neither says anything about the era, so
///   both are the caller's to handle exactly as they were.
/// - Anything else — a `400`, a `404`, an unknown-method JSON-RPC error, a body
///   that is not MCP at all: a server that has never heard of `server/discover`,
///   which is a legacy server.
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
            // Named `NoSharedVersion` by `post` because it has no list of ours
            // to compare against; here there is one, and one of three things is
            // true. None overlaps, and the error stands as raised. One does and
            // it is modern, and the retry is the whole negotiation. Or the best
            // one shared is a revision with a handshake, which is a dual-era
            // server saying so in the only vocabulary a modern request gave it.
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
        // A refusal that is not MCP at all. On the current transport that is a
        // server which has never heard of `server/discover`, which is a legacy
        // server; but it is also exactly what the older transport looks like
        // from here, because a POST to its event stream lands on a URL that
        // only answers GET. So an operator's own server gets the older
        // transport tried before it is called legacy, and the original refusal
        // is what stands if that turns out to be wrong too.
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
                // Whose refusal the operator sees turns on how far the second
                // attempt got. A GET that was not answered with a stream, or
                // not answered at all, says nothing the first attempt did not;
                // anything past that came off the server's own event stream and
                // is the more specific of the two — a 401 that says to sign in,
                // a revision nothing shares, a session redirected onto another
                // host. Reporting the `405` there would send an operator to
                // look at a transport that was working.
                Err(
                    McpError::NotAStream { .. }
                    | McpError::Status { .. }
                    | McpError::Transport { .. },
                ) => Err(err),
                Err(refused) => Err(refused),
            }
        }
        // Everything else identifies a server that wants a handshake.
        Err(_) => Ok(Probed { spoken: handshaking(), server_name: String::new() }),
    }
}

/// A server that wants `initialize`, on the transport it was asked over.
fn handshaking() -> Spoken {
    Spoken { era: Era::Legacy, transport: Transport::Streamable, negotiated: None }
}

/// The newest revision this build and that server both have.
fn agreed(theirs: &[String]) -> Option<String> {
    SUPPORTED.iter().find(|ours| theirs.iter().any(|t| t == *ours)).map(|ours| ours.to_string())
}

/// The legacy handshake, and the revision it settled on.
///
/// The reply's `protocolVersion` is the agreement, not a formality: a server
/// that only knows an older revision answers with that one, and every later
/// request has to declare it. A revision this build has never heard of is
/// refused here rather than by sending it back and being refused there.
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
    // A revision this build does not have, or a modern one, which a handshake
    // cannot produce: this server has already been established as one that
    // wants `initialize`, so agreeing to a revision that has none would have
    // every later request declare a version its own shape contradicts.
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
    };

    // A notification, not a request: no id, and the server answers 202 with no
    // body. Skipping it leaves servers that gate on it refusing every later
    // call with "not initialized", which reads as an auth failure and is not.
    // Modern has no equivalent, because it has nothing to be initialized.
    let ready = serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    let _ = notify(http, &session, &ready).await;

    Ok(session)
}

/// What every endpoint turned out to speak, for the life of the process.
fn eras() -> &'static Mutex<HashMap<String, Spoken>> {
    static ERAS: OnceLock<Mutex<HashMap<String, Spoken>>> = OnceLock::new();
    ERAS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn remembered(endpoint: &str) -> Option<Spoken> {
    eras().lock().ok()?.get(endpoint).cloned()
}

/// Records what the last handshake on this endpoint settled on.
///
/// Only ever called for the older transport, and only for a report: nothing on
/// a request path reads it, because every exchange on that transport
/// re-negotiates inside itself.
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

/// Forgets what an endpoint was, so the next call probes again.
///
/// Called when a remembered era or transport turns out to be wrong, which is
/// what a server upgraded underneath a running Guaca looks like: the handshake
/// it wanted yesterday is a `400` today, and the event stream it served last
/// month is a `405`. One failure, then the truth.
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

// ---- transport ------------------------------------------------------------

/// Everything one POST needs that is not its method and its params.
struct Wire<'a> {
    endpoint: &'a str,
    token: Option<&'a str>,
    /// The operator's own headers. Applied before anything this client builds,
    /// so a name that would contradict the request cannot be written from here
    /// — `Headers::parse` refuses those, and this ordering is what makes that
    /// refusal the only thing standing between them.
    extra: &'a [(String, String)],
    version: String,
    modern: bool,
    session_id: Option<&'a str>,
    /// `Mcp-Param-*`, mirrored out of a modern call's own arguments.
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

/// One JSON-RPC request, and the `result` out of the answer.
///
/// Returns the session id alongside, because a legacy `initialize` is the one
/// call that establishes it and the caller has nowhere else to read it from.
///
/// The body is parsed whatever the status is, and that ordering is what makes
/// the era probe work: a modern server reports an unsupported version, an
/// unknown method and a bad header as `400` and `404` with a JSON-RPC error in
/// the body, and treating a non-2xx as opaque would throw away the one thing
/// that tells those apart from a legacy server refusing a method it has never
/// heard of.
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
        // Both, always. Which one comes back is the server's choice and it may
        // make a different one per request.
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", &wire.version);
    for (name, value) in wire.extra {
        request = request.header(name.as_str(), value.as_str());
    }
    if let Some(token) = wire.token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if wire.modern {
        // Mirrored from the body so an intermediary can route on them without
        // parsing it. The server compares them against the body and refuses the
        // request if they disagree, which is why they are built here from the
        // same values rather than passed in beside them.
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

/// A JSON-RPC `error` object, as the refusal it means here.
///
/// One function because both transports produce one and the three codes mean
/// the same thing on either: two of them are how a modern server is recognized,
/// and everything else is a server that understood the call and said no.
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

/// The `result` out of a decoded reply, or the refusal it carried instead.
fn result_of(endpoint: &str, envelope: &serde_json::Value) -> Result<serde_json::Value, McpError> {
    if let Some(error) = envelope.get("error") {
        return Err(refusal(endpoint, error));
    }
    envelope.get("result").cloned().ok_or_else(|| McpError::Malformed {
        endpoint: endpoint.to_string(),
        detail: "a reply with neither a result nor an error".to_string(),
    })
}

/// A modern request's `params`, with the metadata every one of them carries.
///
/// The version in here has to equal the one in the header beside it, or the
/// server refuses the request with a header mismatch. One value builds both.
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

// ---- the transport streamable HTTP replaced -------------------------------

/// One request over HTTP+SSE, handshake and all, on a stream of its own.
///
/// `call` is `None` for the probe, which only wants to know that the server is
/// there and what it calls itself. Whatever comes back, the stream is dropped
/// when this returns.
///
/// That is the same "a session per call" rule the module argues for on the
/// current transport, reached from the other side. There, keeping a session
/// would be a second thing that can go stale. Here there is nothing to keep:
/// `initialize` establishes a session that belongs to *this* event stream, and
/// a stream held open between turns is a socket per plugin per crew, kept alive
/// through sleep and reconnect, for a handshake that costs one round trip.
///
/// The whole thing is inside one timeout rather than one per request, because
/// the failure this transport actually has is a stream that opens and then says
/// nothing, and a per-request timeout on a socket that is technically still
/// connected never fires.
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
        // The one thing that says this is an event stream rather than a page
        // that happened to answer a GET. Without it, an endpoint behind a login
        // wall answers 200 with HTML and this would sit reading it for two
        // minutes before reporting a timeout.
        let content_type = header(&response, "content-type").unwrap_or_default();
        if !content_type.contains("text/event-stream") {
            return Err(McpError::NotAStream { endpoint: endpoint.to_string(), content_type });
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        // Where requests on this stream go. The server names it first and names
        // it once, and nothing can be sent until it has.
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

        // A notification, and the servers that gate on it refuse every later
        // call with "not initialized" if it is skipped. Nothing comes back.
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
        // A server that answers the POST with the reply rather than putting it
        // on the stream is out of spec and is in the field, so an answer here
        // is taken and the stream is not waited on for a second copy of it.
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

/// Opens a stream, shakes hands and asks for nothing else.
///
/// The era probe's second question: a server that answers this is one on the
/// older transport, and its own name comes back as the by-product the connect
/// path was going to ask for anyway.
async fn sse_probe(http: &reqwest::Client, dial: Dial<'_>) -> Result<(String, String), McpError> {
    let answered = sse_exchange(http, dial, None, CONNECT_TIMEOUT).await?;
    Ok((answered.server_name, answered.negotiated))
}

/// What one exchange on the older transport produced.
///
/// Three, because the handshake on this transport belongs to the stream rather
/// than to a session, so the two things a handshake establishes come back with
/// every request instead of being read off the session that has them.
struct Answered {
    result: serde_json::Value,
    server_name: String,
    negotiated: String,
}

/// POSTs one JSON-RPC message, and takes a reply if the server gave one.
///
/// The documented answer is `202` and an empty body: the reply comes down the
/// stream. Some servers answer with it directly, which is what the `Some` is
/// for, and there is no cost to accepting both.
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

/// Reads the stream until the reply to one request arrives.
///
/// Matched on the id rather than taken as the next message, because a server
/// may push a notification — a log line, a progress report — between the POST
/// and the answer, and reading one of those as the result is a tool call that
/// returns somebody else's message.
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

/// Reads events off an open stream until one the caller wants goes past.
///
/// A closed stream is the failure this returns rather than looping on: a server
/// that hangs up mid-session leaves an empty read forever, and an outer timeout
/// would report it two minutes later as silence, which reads as a slow server
/// rather than a dropped connection.
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

/// Pulls one complete event off the front of the buffer.
///
/// Its own function rather than `SseDecoder` because this transport needs the
/// event *name*: `endpoint` and `message` are two different things arriving on
/// one stream, and the decoder beside the model client throws the name away.
fn take_event(buffer: &mut String) -> Option<(String, String)> {
    // An event ends at a blank line, which is two newlines with nothing but an
    // optional carriage return between them.
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
    // The default event name, per the SSE specification.
    if name.is_empty() {
        name = "message".to_string();
    }
    Some((name, data))
}

/// The URL the server named for messages, if it is on the server's own origin.
///
/// Relative is the common form and absolute is legal, and an absolute one
/// pointing somewhere else is refused rather than followed. This is a redirect
/// invented by the far end after the connection was made, and following it
/// would put the crew's credential and every argument of every tool call on a
/// host the operator never named.
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
                 crew's credential is not sent to an address the operator did not name"
            )));
        }
        named.to_string()
    } else if let Some(path) = named.strip_prefix('/') {
        format!("{origin}/{path}")
    } else {
        // Relative to the directory the stream itself was opened in, which is
        // what a browser would do with it.
        let path = endpoint.strip_prefix(&origin).unwrap_or("");
        let base = path.rsplit_once('/').map(|(before, _)| before).unwrap_or("");
        format!("{origin}{base}/{named}")
    };
    Ok(resolved)
}

/// Scheme and authority, which is what "the same server" means here.
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

// ---- x-mcp-header ---------------------------------------------------------

/// The characters RFC 9110 allows unquoted in a header field name.
const TCHAR: &str = "!#$%&'*+-.^_`|~";

/// Which of a tool's arguments the server asked to see in headers, and where
/// each one lives in the call.
///
/// The path is a chain of `properties` keys and nothing else. The spec is
/// explicit that an annotation reachable only through `items`, `oneOf`,
/// `allOf`, `if`/`then`, or a `$ref` makes the whole tool definition invalid
/// rather than being ignored: the value there has no single place in a call, so
/// there is nothing a client could mirror. Counting every annotation in the
/// document and comparing it with what was reachable is how one hiding in those
/// is caught, and an unreachable one is an error rather than a warning because
/// the alternative is calls that the server refuses with a header mismatch.
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
            // `number` is excluded by the spec along with every non-primitive:
            // a float has no one decimal spelling, so a header and a body
            // carrying the same value could still fail to compare equal.
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

/// Every `x-mcp-header` in the document, wherever it is.
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

/// The headers one call carries, out of its own arguments.
///
/// A parameter with no value in this call is omitted rather than sent empty:
/// the server expects the header only when the body has the value, and one sent
/// anyway is a mismatch. A schema that could not be read at all mirrors nothing,
/// because `list_tools` already dropped the tools that would need it — this is
/// the same read a second time and it cannot disagree.
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

/// A value as a header may carry it.
///
/// Anything outside printable ASCII, anything padded, and anything that would
/// read as the encoding marker itself is base64 behind that marker. The last
/// case is the one that looks paranoid and is not: a value that legitimately
/// starts `=?base64?` and ends `?=` would otherwise be decoded by the server as
/// something it is not.
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

// ---- results --------------------------------------------------------------

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
    fn an_event_carries_its_name_as_well_as_its_data() {
        // The whole reason this is not `SseDecoder`: `endpoint` and `message`
        // arrive on one stream and mean entirely different things.
        let mut buffer = String::from("event: endpoint\ndata: /messages?s=1\n\n");
        assert_eq!(take_event(&mut buffer), Some(("endpoint".into(), "/messages?s=1".into())));
        assert!(buffer.is_empty());
    }

    #[test]
    fn an_unnamed_event_is_a_message() {
        // The SSE default, and servers rely on it.
        let mut buffer = String::from("data: {\"id\":1}\n\n");
        assert_eq!(take_event(&mut buffer), Some(("message".into(), "{\"id\":1}".into())));
    }

    #[test]
    fn an_event_split_across_chunks_waits_for_the_rest() {
        // A JSON payload cut mid-token is the routine case on a socket, and
        // parsing half of it is a tool call that comes back as a parse error.
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
        // A redirect invented by the far end after the connection was made.
        // Followed, it puts the crew's credential and every tool argument on a
        // host the operator never named.
        let refused = same_origin("https://box.example.com/sse", "https://evil.example/m");
        let message = refused.unwrap_err().to_string();
        assert!(message.contains("different server"), "{message}");
        assert!(message.contains("did not name"), "{message}");
    }

    #[test]
    fn every_revision_this_build_speaks_sorts_as_a_date() {
        // `modern` is a string comparison, which only works because a revision
        // is `YYYY-MM-DD`. A revision named any other way would silently sort
        // into the wrong era.
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
        // A real server answers every call this way, the handshake included.
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

    #[test]
    fn the_newest_revision_is_the_one_a_modern_request_declares() {
        // The order in `SUPPORTED` is the preference order, and `agreed` walks
        // it rather than the server's list: a server that offers three takes
        // the best of them rather than the first one it happened to name.
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
        // Which is what lets "is this one modern" be a comparison rather than a
        // second list beside `SUPPORTED` for somebody to forget to update.
        assert!(modern(PROTOCOL_VERSION));
        assert!(modern("2030-01-01"));
        assert!(!modern(LEGACY_VERSION));
        assert!(!modern("2025-06-18"));
        assert!(!modern("2024-11-05"));
    }

    #[test]
    fn the_handshake_revision_is_one_this_build_speaks() {
        // `initialize` asks for the newest revision that still has a handshake,
        // and every request after it declares whatever came back. A value here
        // that is not in `SUPPORTED` would have this client refuse a server
        // that agreed to exactly what it asked for.
        assert!(SUPPORTED.contains(&LEGACY_VERSION));
        assert_ne!(LEGACY_VERSION, PROTOCOL_VERSION);
    }

    #[test]
    fn a_modern_request_carries_its_version_where_the_server_compares_it() {
        // The header and the `_meta` field have to agree or the server refuses
        // the request. One value builds both, and this is what says so.
        let params = with_meta(serde_json::json!({ "name": "run_sql" }), "2026-07-28");
        assert_eq!(params["name"], serde_json::json!("run_sql"));
        assert_eq!(params["_meta"][META_VERSION], serde_json::json!("2026-07-28"));
        assert_eq!(params["_meta"][META_CLIENT]["name"], serde_json::json!("Guaca"));
        assert!(params["_meta"][META_CAPABILITIES].is_object());
    }

    #[test]
    fn a_modern_server_names_itself_under_the_meta_key_rather_than_beside_it() {
        // The handshake put `serverInfo` at the top of the result; the modern
        // revision moved it into `_meta` under a namespaced key. Reading the
        // old place would leave every modern plugin unlabelled.
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
        // Non-ASCII, padded, and the marker itself, which is the case that
        // looks paranoid: a value shaped like the encoding would be decoded by
        // the server into something it never was.
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
        // Compared as a set: these are headers, and the order they are added in
        // is the order a JSON object's keys happen to come out in.
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
        // The server expects the header exactly when the body has the value, so
        // an empty one is a mismatch and the whole call is refused.
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "region": { "type": "string", "x-mcp-header": "Region" } },
        });
        assert!(mirror(&schema, &serde_json::json!({})).is_empty());
        assert!(mirror(&schema, &serde_json::json!({ "region": null })).is_empty());
    }

    #[test]
    fn an_annotation_that_is_not_reachable_through_properties_invalidates_the_tool() {
        // The spec's rule, and the reason it is a rule: a value inside an array
        // or behind a `oneOf` has no single place in a call, so there is nothing
        // to mirror and a client that guessed would be refused on every call.
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
        // A float has no one decimal spelling, so the header and the body
        // carrying the same value could still fail to compare equal.
        let float = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "number", "x-mcp-header": "A" } },
        });
        assert!(mirrored_params(&float).is_err());
        // And two parameters cannot both claim one header name.
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
        // Which is every tool every server on the list publishes today. A
        // stricter reading here would drop all of them.
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
        // Nothing the operator can do fixes this one, and the sentence has to
        // say that rather than reading like a bad address.
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
