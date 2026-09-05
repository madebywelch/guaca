//! Connecting a plugin, and spending the grant it produced.
//!
//! Three files meet here and each one refuses to know about the others:
//! [`crate::mcp`] speaks the protocol and has never heard of a group,
//! [`crate::oauth`] runs the sign-in and has never heard of MCP, and the store
//! holds the grant and has never heard of either. This is the only place that
//! knows a plugin is all three.
//!
//! ## A session per call, on purpose
//!
//! Every tool call opens a fresh MCP session: `initialize`, then the call. The
//! obvious optimization is to keep the session and reuse it, and it is not
//! taken. A cached session is a second thing that can be stale — the server can
//! expire it, the token under it can be refreshed, and the crew can disconnect
//! the plugin — and every one of those failures surfaces as a tool call that
//! fails for no reason a model can act on. The extra round trip is tens of
//! milliseconds against a call that is already going over the internet to run
//! somebody's SQL. Correctness first; this is the place to come back to if a
//! measurement ever says the handshake is what a turn is waiting on.
//!
//! ## What the agent is never told
//!
//! The token, and the headers beside it. A plugin tool call is made by Guaca,
//! not by the machine: the agent names a tool and arguments and reads a result,
//! and neither the grant nor anything [`crate::domain::plugin::Headers`] holds
//! appears in the prompt, the transcript, an event, or the sandbox's
//! environment. This is the same boundary a pasted credential has, and it is
//! stronger, because with a plugin there is no variable for the agent to echo.

use crate::account::AccountError;
use crate::db::store::{PluginReach, Store, StoreError};
use crate::domain::ids::{AgentId, GroupId, PluginId};
use crate::domain::now_ms;
use crate::domain::plugin::{Headers, Plugin, PluginKind, PluginTool, ServerReport, SigninNeed};
use crate::mcp::{self, McpError};
use crate::oauth::{self, Grant, OauthError};

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Signin(#[from] OauthError),
    #[error(transparent)]
    Server(#[from] McpError),
    #[error(
        "{label} is not connected for this group. Ask the operator to connect it in the group's \
         Plugins settings; nothing you can do from here will connect it."
    )]
    NotConnected { label: String },
    #[error(
        "{label} comes from your Guaca account, and this machine is not signed in to one. Ask the \
         operator to sign in under Settings, Account, and to authorize {label} at guaca.bot; \
         nothing you can do from here will do it."
    )]
    NoAccount { label: String },
    #[error(
        "{label} comes from your Guaca account, which is signed in and could not produce a \
         credential just now: {detail}. This is the service refusing, not a sign-in to redo, so \
         do not tell the operator to sign in again. Carry on with work that does not need \
         {label} and try it again later in this turn or in a later one; if it is still refusing \
         then, tell the operator exactly what this said."
    )]
    AccountRefused { label: String, detail: String },
    #[error(
        "{label} is connected for this group, but not for you: the operator chose which agents \
         may use it. Ask a peer who has it to do that part, or ask the operator to add you in \
         the group's Plugins settings. Nothing you can do from here will add you."
    )]
    NotChosen { label: String },
    #[error(
        "{label}'s `{tool}` is switched off for this group: the operator decides which of a \
         plugin's tools the crew may call, and this one is off for everybody. Do not ask a peer, \
         because no peer has it. Use another of {label}'s tools if one will do, or tell the \
         operator which tool you need and that it is switched off in the group's Plugins \
         settings."
    )]
    ToolDenied { label: String, tool: String },
    #[error(
        "{label} is yours, but its `{tool}` is not: the operator chose which agents may call that \
         one. A peer has it — your roster says who — so hand that part over. Use another of \
         {label}'s tools if one will do, or ask the operator to add you to `{tool}` in the \
         group's Plugins settings. Nothing you can do from here will add you."
    )]
    ToolNotChosen { label: String, tool: String },
    #[error(
        "{label}'s sign-in is no longer accepted, and renewing it did not work. Ask the operator \
         to connect it again in the group's Plugins settings."
    )]
    SigninExpired { label: String },
}

/// What a connection is paid for with.
///
/// Three, because there are three honest answers to "who is this crew when it
/// calls this server", and they cannot be inferred from each other. The catalog
/// vendors discover theirs; the operator's own account lends its own; and a
/// server somebody wrote themselves very often wants a key that was minted by
/// hand and has no authorization server behind it at all.
///
/// [`Headers`] is not a fourth and is passed beside this rather than into it.
/// It answers a different question — how a request *reaches* the server, not
/// who is asking — so it composes with all three instead of replacing one. The
/// server that proves the difference is one behind a gate that also signs in:
/// the headers get past the gate, and the 401 from behind it is still
/// `Discover`'s to answer.
#[derive(Debug, Clone, Copy)]
pub enum Credential<'a> {
    /// Whatever the server asks for: nothing, or the browser dance.
    ///
    /// The only right answer for a server that publishes protected-resource
    /// metadata, and the default for one nobody knows anything about.
    Discover,
    /// A key the operator pasted, sent as a bearer token.
    ///
    /// Stored exactly where a grant's access token is stored and spent exactly
    /// the same way, which is what keeps it out of prompts, transcripts, events
    /// and the webview: there is no second path for a second kind of secret.
    /// What it does not have is a refresh token, so a server that stops
    /// accepting it says so once and the operator pastes another.
    Key(&'a str),
    /// The machine's Guaca account. See [`crate::domain::plugin::PluginKind::account_backed`].
    Account(AccountUse<'a>),
}

/// Signs in to a plugin's server and records what it can do.
///
/// The first request goes out with no token deliberately, and it is doing two
/// jobs. It is the only honest way to find out whether this server wants one —
/// sending an operator to a browser to authorize a server that authorizes
/// everybody is a consent prompt for nothing — and its refusal is where the
/// address of the sign-in comes from: the `WWW-Authenticate` challenge names the
/// server's own protected-resource metadata, which is the answer the vendor just
/// gave rather than a well-known path Guaca guessed at.
///
/// A pasted key skips that. A server that takes one has nothing to discover:
/// there is no authorization server, the 401 names no metadata, and asking
/// first would be a round trip whose only outcome is a refusal Guaca already
/// knows how to answer.
// Eight, and every one is a different caller's decision: the store, the crew,
// the kind, where it is dialed, what it is dialed with, what else goes on the
// wire, where the browser lands, and how the browser is opened. A struct for
// them would be a struct built at one call site and read at one.
#[allow(clippy::too_many_arguments)]
pub async fn connect(
    store: &Store,
    group: GroupId,
    kind: &PluginKind,
    // Passed rather than read off the kind, for the reason `Subscription` takes
    // an issuer: a test drives this whole flow against a scripted server and
    // there is no other seam wide enough to put one behind. Production passes
    // `Runtime::plugin_endpoint`, which is `kind.endpoint()` unless a test
    // moved it, and the account's own address for an account-backed kind.
    endpoint: &str,
    credential: Credential<'_>,
    // What the operator gave this server beyond a credential. Empty for the
    // catalog, which needs none, and for most added servers. Passed rather than
    // read back off the row because this is the call that writes the row: a
    // reconnection is where they change.
    headers: &Headers,
    // Where the browser comes back to. The host's decision, passed for the
    // reason `open` is: this file knows the dance and not the machine.
    landing: &oauth::Landing,
    open: impl FnOnce(&str) -> Result<(), String>,
) -> Result<Plugin, PluginError> {
    // An account-backed plugin never runs the browser dance. Its server is the
    // operator's own account, which is already signed in, and the row stores no
    // grant: the token rotates on the account's clock, so a copy here would be
    // a second thing to keep fresh and a second thing to be stale.
    if kind.account_backed() {
        let Credential::Account(used) = credential else {
            return Err(PluginError::NoAccount { label: kind.label().to_string() });
        };
        let (_session, tools) =
            ask(dial(kind.is_custom(), endpoint, headers).with_token(Some(used.token))).await?;
        // No account label. An MCP server's own name is not an account, and for
        // this one it is "Guaca Connectors", which on a card reading "Signed in
        // as ..." looks exactly like an answer to whose mailbox this is and is
        // not. Which identity this row uses is `connection`, and the operator's
        // own name for it lives at the account, where it can change without
        // this row going stale.
        return Ok(store.save_plugin(group, kind, "", &tools, None, used.connection, headers)?);
    }

    let dialed = dial(kind.is_custom(), endpoint, headers);
    let (session, tools, grant) = match credential {
        // A pasted key is a grant with nothing to renew it. Everything after
        // this line — where it is stored, how it is spent, what happens when the
        // server stops accepting it — is the OAuth path's, unchanged.
        Credential::Key(key) => {
            let (session, tools) = ask(dialed.with_token(Some(key))).await?;
            (session, tools, Some(Grant::key(key)))
        }
        // Headers the operator wrote are not a second credential path and do
        // not skip this. A server they authenticate answers the first request,
        // and nothing is discovered because nothing was refused; one that gates
        // on them *and* signs in — an MCP server behind Cloudflare Access is the
        // shape — answers 401 from behind the gate, and the browser dance runs
        // with the headers on every request it makes from here on.
        _ => match ask(dialed).await {
            Ok((session, tools)) => (session, tools, None),
            Err(err) if err.is_unauthorized() => {
                let challenge = match &err {
                    McpError::Unauthorized { challenge, .. } => challenge.clone(),
                    _ => None,
                };
                // The operator's own headers go with it. A gate in front of
                // a self-hosted server refuses the metadata document a
                // sign-in has to read first, so without them this composition
                // fails at discovery with a 403 nobody can act on. `Gate` is
                // what keeps them off the authorization server's own host.
                let grant = oauth::authorize(
                    endpoint,
                    challenge.as_deref(),
                    &oauth::Gate::on(endpoint, headers.wire()),
                    landing,
                    open,
                    now_ms,
                )
                .await?;
                let (session, tools) = ask(dialed.with_token(Some(&grant.access_token))).await?;
                (session, tools, Some(grant))
            }
            Err(err) => return Err(err.into()),
        },
    };

    let account_label = account_label(&session, kind).await;

    // Only an account-backed kind carries one; the others sign in per group and
    // their grant already names the identity it was issued to.
    Ok(store.save_plugin(group, kind, &account_label, &tools, grant.as_ref(), "", headers)?)
}

/// How this server is reached, before anything is presented to it.
///
/// The one place the transport question is answered, and it is answered off
/// the kind: a server the operator added may be spoken to over the transport
/// streamable HTTP replaced, and one on the catalog may not. `mcp.rs` has the
/// argument; this is where it is applied, because the kind is a plugin concept
/// and that file has never heard of one.
fn dial<'a>(custom: bool, endpoint: &'a str, headers: &'a Headers) -> mcp::Dial<'a> {
    mcp::Dial { endpoint, token: None, headers: headers.wire(), legacy_transport: custom }
}

/// Dials a server and says what it found, without connecting or authorizing it.
///
/// The whole of "test this". It runs the same probe, the same handshake and the
/// same `tools/list` a connection runs, over the same two transports, with
/// whatever credential and headers it was given — and then throws the answer
/// away instead of writing a row. Anything less than the real path is a test
/// that passes for a server the crew cannot use.
///
/// A 401 is a finding rather than a failure, and it is the one place this
/// deliberately stops short of `connect`: the browser dance is not run. An
/// operator testing an address is asking whether it is the right address, and
/// answering that by sending them to a consent screen is a question they did
/// not ask.
pub async fn inspect(
    custom: bool,
    endpoint: &str,
    token: Option<&str>,
    headers: &Headers,
) -> Result<ServerReport, PluginError> {
    let started = std::time::Instant::now();
    let dialed = dial(custom, endpoint, headers).with_token(token);
    let report =
        |session: &mcp::Session, server: String, tools: Vec<PluginTool>, signin| ServerReport {
            endpoint: endpoint.to_string(),
            transport: if session.sse() {
                "HTTP+SSE (2024-11-05)".to_string()
            } else {
                "streamable HTTP".to_string()
            },
            protocol: session.protocol().to_string(),
            handshake: !session.modern(),
            signin,
            server,
            tools: tools.into_iter().map(|tool| tool.name).collect(),
            ms: started.elapsed().as_millis() as u64,
        };

    match ask(dialed).await {
        Ok((session, tools)) => {
            let server = mcp::describe(&session).await.unwrap_or_default();
            let signin = if token.is_some() { SigninNeed::Accepted } else { SigninNeed::None };
            Ok(report(&session, server, tools, signin))
        }
        // Reachable, and it wants something. Which of the two it is turns on
        // what was presented, and the difference is the whole value of asking:
        // with nothing presented this is a server that signs in, and with a key
        // presented it is a key the server does not accept. Those have nothing
        // in common except the status code.
        Err(err) if err.is_unauthorized() => Ok(ServerReport {
            endpoint: endpoint.to_string(),
            // Unknown, and said as nothing rather than guessed at: the refusal
            // arrived before either question was settled.
            transport: String::new(),
            protocol: String::new(),
            handshake: false,
            signin: if token.is_some() { SigninNeed::Refused } else { SigninNeed::Wanted },
            server: String::new(),
            tools: Vec::new(),
            ms: started.elapsed().as_millis() as u64,
        }),
        Err(err) => Err(err.into()),
    }
}

/// The same question, asked of a plugin the crew has already connected.
///
/// Not a narrower `connect`: connecting replaces the row, re-reads the tool
/// list and can open a browser, which is a lot to do to somebody who asked
/// whether their server was up. This spends the grant that is already there and
/// writes nothing but a renewal.
///
/// The renewal is not a side effect to avoid. A stale token is what the next
/// real call would refresh, so a check that skipped it would report "the
/// sign-in was refused" about a plugin that works.
pub async fn check(
    store: &Store,
    id: PluginId,
    dialed: crate::db::store::Dialed,
    endpoint: &str,
    account: Held<'_>,
) -> Result<ServerReport, PluginError> {
    let crate::db::store::Dialed { kind, grant, headers, .. } = dialed;
    if kind.account_backed() {
        let used = account.spend(&kind)?;
        return inspect(kind.is_custom(), endpoint, Some(used.token), &headers).await;
    }
    let grant = match grant {
        Some(held) if held.stale(now_ms()) => {
            Some(renew(store, id, &held, &kind, endpoint, &headers).await?)
        }
        other => other,
    };
    inspect(kind.is_custom(), endpoint, grant.as_ref().map(|g| g.access_token.as_str()), &headers)
        .await
}

/// Opens a session and asks for the tool list, as one question.
///
/// One function because the answer this path wants is "does this server want a
/// grant", and which of the two requests reveals that depends on the era. A
/// legacy server has a handshake to refuse; a modern one has nothing to
/// establish, so the first request that carries any weight is `tools/list`, and
/// its 401 is the same news arriving one call later.
///
/// Split apart, the modern case is a bug that only shows up the second time:
/// the first connect probes and gets the 401, and every connect after it finds
/// the era remembered, makes no request at all, and reports a raw 401 out of
/// the tool list instead of starting the sign-in.
async fn ask(dial: mcp::Dial<'_>) -> Result<(mcp::Session, Vec<PluginTool>), McpError> {
    let session = mcp::open(dial).await?;
    let tools = read_tools(&session).await?;
    Ok((session, tools))
}

async fn read_tools(session: &mcp::Session) -> Result<Vec<PluginTool>, McpError> {
    Ok(mcp::list_tools(session)
        .await?
        .into_iter()
        .map(|tool| PluginTool {
            name: tool.name,
            description: tool.description,
            // A tool that declares no schema takes no arguments. Passing null
            // to a provider is a malformed function definition and takes every
            // other tool on the turn down with it.
            input_schema: tool
                .input_schema
                .unwrap_or_else(|| serde_json::json!({ "type": "object", "properties": {} })),
        })
        .collect())
}

/// Whose account the grant turned out to be for, when the server said.
///
/// Only when it says something other than its own product name. "Neon MCP
/// Server" as an account label is worse than a blank one: it looks like an
/// answer to "whose account is this" and is not.
///
/// A failure to ask is a blank label rather than a failed connection. This is
/// the one thing on the connect path that nothing depends on, and losing the
/// whole sign-in over a name is the wrong trade.
async fn account_label(session: &mcp::Session, kind: &PluginKind) -> String {
    let name = mcp::describe(session).await.unwrap_or_default();
    if name.eq_ignore_ascii_case(kind.label()) {
        String::new()
    } else {
        name
    }
}

/// The machine's Guaca account, as a plugin credential.
///
/// The token and the identity travel together and mean nothing apart: the token
/// says which account, and the connection says which of the identities that
/// account has authorized. A caller holding one and guessing the other is the
/// bug this type exists to prevent, which is a crew reaching the wrong mailbox.
#[derive(Debug, Clone, Copy)]
pub struct AccountUse<'a> {
    pub token: &'a str,
    /// Which authorized identity, or empty for the account's default. Empty is
    /// what a plugin connected before connections existed keeps sending, and it
    /// is right for an account with one identity, which is most of them.
    pub connection: &'a str,
}

/// What the machine's account had for this call.
///
/// Three states rather than an `Option`, because two of them send an operator
/// to different places and only one of them is a sign-in. An `Option` folded
/// them together, and what came out the other end was an agent telling its
/// operator to sign in to an account that was signed in, refreshing normally,
/// and serving every call ten seconds later. The refusal is carried rather
/// than re-derived here: `account.rs` is where the status the service answered
/// with is, and by the time a plugin sees this that answer is gone.
#[derive(Debug, Clone)]
pub enum Held<'a> {
    /// Nothing is signed in on this machine.
    Absent,
    /// A token, read fresh for this call.
    Token(AccountUse<'a>),
    /// Signed in, and the account could not produce a token, in its own words.
    Refused(String),
}

impl<'a> Held<'a> {
    /// What one read of the account came to.
    ///
    /// The only place an [`AccountError`] is sorted into these three, so the
    /// turn, the connect and the check cannot classify the same failure
    /// differently. Takes the read by reference because the token is borrowed
    /// for the length of the call rather than copied into it.
    pub fn read(read: &'a Result<String, AccountError>, connection: &'a str) -> Self {
        match read {
            Ok(token) => Held::Token(AccountUse { token, connection }),
            Err(AccountError::NotSignedIn) => Held::Absent,
            Err(err) => Held::Refused(err.to_string()),
        }
    }

    /// The credential, or the refusal that says which of the two happened.
    ///
    /// One function because the connect path, the check path and the call path
    /// all have to answer this, and the answer an agent reads and the answer an
    /// operator reads must not be able to disagree.
    pub fn spend(self, kind: &PluginKind) -> Result<AccountUse<'a>, PluginError> {
        match self {
            Held::Token(used) => Ok(used),
            Held::Absent => Err(PluginError::NoAccount { label: kind.label().to_string() }),
            Held::Refused(detail) => {
                Err(PluginError::AccountRefused { label: kind.label().to_string(), detail })
            }
        }
    }
}

impl AccountUse<'_> {
    /// Where this connection's server is, given the account's own origin.
    ///
    /// Unnamed stays on the bare path rather than inventing one, because that
    /// is the address an already-connected plugin is dialling and an upgrade
    /// must not quietly repoint a working crew.
    pub fn endpoint(origin: &str, connection: &str) -> String {
        let origin = origin.trim_end_matches('/');
        if connection.is_empty() {
            format!("{origin}/mcp")
        } else {
            format!("{origin}/mcp/{connection}")
        }
    }
}

/// Which plugin a call is for, and on whose behalf.
///
/// Grouped rather than passed loose because the five travel together and mean
/// nothing apart: the reach check needs the first three, the transport needs
/// the last two, and a caller that had four of them would be about to guess the
/// fifth.
pub struct Target<'a> {
    pub group: GroupId,
    pub agent: AgentId,
    pub kind: &'a PluginKind,
    /// See `connect`: production passes `Runtime::plugin_endpoint`.
    pub endpoint: &'a str,
    /// See `connect`: the machine's account, for an account-backed kind.
    pub account: Held<'a>,
}

/// Runs one of a plugin's tools on behalf of an agent.
///
/// Asked of the agent and of the tool, and that is the check rather than a
/// second one: a model can name a tool it was never offered, so filtering the
/// definitions decides what an agent is *told* it has and this decides what it
/// *gets*. Both halves of the rule are read again here, in `plugin_reach`.
///
/// Renews the grant first when it is close to expiring, and once more if the
/// server rejects it anyway: a token can be revoked at the vendor between one
/// turn and the next, and a clock that is a little wrong makes "close to
/// expiring" a guess rather than a fact.
///
/// The tool's own schema comes back from the same read as the grant, and it is
/// not decoration: a modern server may ask for some of a call's arguments to be
/// mirrored into HTTP headers, and it says which in the schema. A call built
/// without it is refused by the server as a header mismatch.
pub async fn call(
    store: &Store,
    target: Target<'_>,
    tool: &str,
    arguments: &serde_json::Value,
) -> Result<String, PluginError> {
    let Target { group, agent, kind, endpoint, account } = target;
    let label = || kind.label().to_string();
    let (id, grant, headers, schema) = match store.plugin_reach(group, agent, kind, tool)? {
        PluginReach::Granted(reached) => {
            (reached.id, reached.grant, reached.headers, reached.schema)
        }
        PluginReach::NotConnected => return Err(PluginError::NotConnected { label: label() }),
        PluginReach::NotChosen => return Err(PluginError::NotChosen { label: label() }),
        PluginReach::ToolDenied => {
            return Err(PluginError::ToolDenied { label: label(), tool: tool.to_string() })
        }
        PluginReach::ToolNotChosen => {
            return Err(PluginError::ToolNotChosen { label: label(), tool: tool.to_string() })
        }
    };

    // The reach check above is the same one every plugin gets — the crew, the
    // agent and the tool — and running it before this branch is what makes both
    // of those decisions mean something for an account-backed plugin too. What
    // differs is only the credential: this one is the account's, freshly read,
    // so there is nothing stored to renew and nothing to retry a 401 with. An
    // account that has been signed out reads as no token at all, which says so
    // rather than failing on the wire.
    if kind.account_backed() {
        let used = account.spend(kind)?;
        let session =
            mcp::open(dial(kind.is_custom(), endpoint, &headers).with_token(Some(used.token)))
                .await?;
        return Ok(mcp::call_tool(&session, tool, arguments, schema.as_ref()).await?);
    }

    let grant = match grant {
        Some(held) if held.stale(now_ms()) => {
            Some(renew(store, id, &held, kind, endpoint, &headers).await?)
        }
        other => other,
    };

    let dialed = dial(kind.is_custom(), endpoint, &headers);
    let attempt = run(dialed, grant.as_ref(), tool, arguments, schema.as_ref()).await;
    match attempt {
        Err(McpError::Unauthorized { .. }) => {
            // One retry, and only for a grant there is something to renew. A
            // public server answering 401 is a server that changed its mind
            // about being public, which no refresh fixes — and so is a key the
            // operator pasted, which has no refresh token behind it either.
            let Some(held) = grant else {
                return Err(PluginError::SigninExpired { label: label() });
            };
            let renewed = renew(store, id, &held, kind, endpoint, &headers).await?;
            run(dialed, Some(&renewed), tool, arguments, schema.as_ref()).await.map_err(Into::into)
        }
        other => other.map_err(Into::into),
    }
}

async fn run(
    dial: mcp::Dial<'_>,
    grant: Option<&Grant>,
    tool: &str,
    arguments: &serde_json::Value,
    schema: Option<&serde_json::Value>,
) -> Result<String, McpError> {
    let session = mcp::open(dial.with_token(grant.map(|g| g.access_token.as_str()))).await?;
    mcp::call_tool(&session, tool, arguments, schema).await
}

/// Renews a grant and writes it back, so the next turn does not renew it again.
///
/// Takes the operator's headers for the same reason the sign-in does: when the
/// token endpoint is on their own server — a self-hosted MCP server that is its
/// own issuer, behind a gate — a refresh without them is a `403` a day after
/// everything worked. `Gate` is what keeps them off a vendor's token endpoint,
/// which is a different origin and none of its business.
async fn renew(
    store: &Store,
    id: PluginId,
    grant: &Grant,
    kind: &PluginKind,
    endpoint: &str,
    headers: &Headers,
) -> Result<Grant, PluginError> {
    let renewed = oauth::refresh(grant, &oauth::Gate::on(endpoint, headers.wire()), now_ms)
        .await
        .map_err(|_| PluginError::SigninExpired { label: kind.label().to_string() })?;
    store.refresh_plugin_grant(id, &renewed)?;
    Ok(renewed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unnamed_connection_stays_on_the_bare_path() {
        // The address an already-connected plugin is dialling. An upgrade that
        // invented an id here would silently repoint a working crew at a
        // different mailbox, which is the one failure this column exists to
        // prevent rather than cause.
        assert_eq!(AccountUse::endpoint("https://guaca.bot", ""), "https://guaca.bot/mcp");
        assert_eq!(AccountUse::endpoint("https://guaca.bot/", ""), "https://guaca.bot/mcp");
    }

    #[test]
    fn a_named_connection_is_part_of_the_address() {
        // Which identity a crew uses is in the path, because that is how the
        // server scopes a session and there is nowhere else in MCP to put it.
        assert_eq!(
            AccountUse::endpoint("https://guaca.bot", "acct_1"),
            "https://guaca.bot/mcp/acct_1"
        );
        // Development points the account somewhere else, and the plugin has to
        // follow it or a crew signs in to one origin and calls another.
        assert_eq!(
            AccountUse::endpoint("http://localhost:8787", "acct_1"),
            "http://localhost:8787/mcp/acct_1"
        );
    }

    #[test]
    fn a_plugin_that_is_not_connected_says_who_can_connect_it() {
        // An agent reads this mid-turn. A refusal that only says no gets
        // reworded and retried; this one has to close that door, because
        // nothing the agent can do will open it.
        let refusal = PluginError::NotConnected { label: "Neon".into() }.to_string();
        assert!(refusal.contains("Neon"));
        assert!(refusal.contains("operator"));
        assert!(refusal.contains("nothing you can do"));
    }

    #[test]
    fn a_plugin_this_agent_was_not_chosen_for_does_not_read_as_a_missing_sign_in() {
        // Two refusals, two different answers. An agent told "not connected"
        // about a plugin the crew is using would send the operator to a panel
        // that already says Disconnect, and would never think to ask the peer
        // who can actually do it.
        let refusal = PluginError::NotChosen { label: "Stripe".into() }.to_string();
        assert!(refusal.contains("Stripe"));
        assert!(refusal.contains("not for you"), "{refusal}");
        assert!(refusal.contains("peer"), "the way forward is delegation: {refusal}");
        assert!(!refusal.contains("is not connected"), "{refusal}");
    }

    #[test]
    fn a_switched_off_tool_does_not_send_the_agent_round_the_crew() {
        // The difference from `NotChosen`, and it is the whole reason this is a
        // third sentence rather than a reuse of that one. Narrowing a plugin
        // leaves a peer who can; switching a tool off leaves nobody, so an
        // agent told to ask around spends a turn proving it.
        let refusal =
            PluginError::ToolDenied { label: "Stripe".into(), tool: "create_refund".into() }
                .to_string();
        assert!(refusal.contains("create_refund"), "{refusal}");
        assert!(refusal.contains("off for everybody"), "{refusal}");
        assert!(refusal.contains("Do not ask a peer"), "{refusal}");
        assert!(
            refusal.contains("operator"),
            "the way forward is the one person who can: {refusal}"
        );
    }

    #[test]
    fn an_expired_sign_in_says_that_connecting_again_is_the_fix() {
        let refusal = PluginError::SigninExpired { label: "Cloudflare".into() }.to_string();
        assert!(refusal.contains("Cloudflare"));
        assert!(refusal.contains("connect it again"));
    }

    #[test]
    fn a_read_of_the_account_sorts_into_the_three_states_one_way() {
        // The classification every caller shares. Two of these used to be one
        // `None`, and the sentence it produced sent an operator to sign in to
        // an account that was signed in.
        let nothing: Result<String, AccountError> = Err(AccountError::NotSignedIn);
        assert!(matches!(Held::read(&nothing, ""), Held::Absent));

        let refused: Result<String, AccountError> = Err(AccountError::Upstream {
            origin: "https://guaca.bot".into(),
            status: 400,
            message: "invalid_grant".into(),
        });
        assert!(matches!(Held::read(&refused, ""), Held::Refused(_)));

        let held: Result<String, AccountError> = Ok("token".into());
        match Held::read(&held, "acct_1") {
            Held::Token(used) => {
                assert_eq!(used.token, "token");
                // The identity travels with the token or a crew reaches the
                // wrong mailbox.
                assert_eq!(used.connection, "acct_1");
            }
            other => panic!("a token is a token: {other:?}"),
        }
    }

    #[test]
    fn the_two_ways_to_have_no_credential_do_not_share_a_sentence() {
        let absent = Held::Absent.spend(&PluginKind::Google).unwrap_err().to_string();
        let refused = Held::Refused("https://guaca.bot answered HTTP 500: nope".into())
            .spend(&PluginKind::Google)
            .unwrap_err()
            .to_string();

        // One is a thing to do and the other is a thing to wait out, so the
        // only wrong answer is the pair reading alike.
        assert!(absent.contains("not signed in to one"), "{absent}");
        assert!(absent.contains("Settings"), "{absent}");
        assert!(refused.contains("HTTP 500"), "what the service said: {refused}");
        assert!(refused.contains("try it again later"), "the way forward: {refused}");
        assert!(!refused.contains("Settings"), "nothing in Settings fixes this: {refused}");
        assert!(!refused.contains("not signed in to one"), "{refused}");
    }
}
