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
//! obvious optimisation is to keep the session and reuse it, and it is not
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
//! The token. A plugin tool call is made by Guaca, not by the machine: the
//! agent names a tool and arguments and reads a result, and the grant does not
//! appear in the prompt, the transcript, an event, or the sandbox's
//! environment. This is the same boundary a pasted credential has, and it is
//! stronger, because with a plugin there is no variable for the agent to echo.

use crate::db::store::{PluginReach, Store, StoreError};
use crate::domain::ids::{AgentId, GroupId, PluginId};
use crate::domain::now_ms;
use crate::domain::plugin::{Plugin, PluginKind, PluginTool};
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
    NotConnected { label: &'static str },
    #[error(
        "{label} comes from your Guaca account, and this machine is not signed in to one. Ask the \
         operator to sign in under Settings, Account, and to authorize {label} at guaca.bot; \
         nothing you can do from here will do it."
    )]
    NoAccount { label: &'static str },
    #[error(
        "{label} is connected for this group, but not for you: the operator chose which agents \
         may use it. Ask a peer who has it to do that part, or ask the operator to add you in \
         the group's Plugins settings. Nothing you can do from here will add you."
    )]
    NotChosen { label: &'static str },
    #[error(
        "{label}'s `{tool}` is switched off for this group: the operator decides which of a \
         plugin's tools the crew may call, and this one is off for everybody. Do not ask a peer, \
         because no peer has it. Use another of {label}'s tools if one will do, or tell the \
         operator which tool you need and that it is switched off in the group's Plugins \
         settings."
    )]
    ToolDenied { label: &'static str, tool: String },
    #[error(
        "{label}'s sign-in is no longer accepted, and renewing it did not work. Ask the operator \
         to connect it again in the group's Plugins settings."
    )]
    SigninExpired { label: &'static str },
}

/// Signs in to a plugin's server and records what it can do.
///
/// The first `initialize` goes out with no token deliberately, and it is doing
/// two jobs. It is the only honest way to find out whether this server wants
/// one — sending an operator to a browser to authorise a server that authorises
/// everybody is a consent prompt for nothing — and its refusal is where the
/// address of the sign-in comes from: the `WWW-Authenticate` challenge names
/// the server's own protected-resource metadata, which is the answer the vendor
/// just gave rather than a well-known path Guaca guessed at.
pub async fn connect(
    store: &Store,
    group: GroupId,
    kind: PluginKind,
    // Passed rather than read off the kind, for the reason `Subscription`
    // takes an issuer: a test drives this whole flow against a scripted server
    // and there is no other seam wide enough to put one behind. Production
    // passes `kind.endpoint()` and nothing else ever should.
    endpoint: &str,
    // The machine's Guaca account token, for a plugin whose credential is that
    // account rather than a grant of its own. `None` for every other kind, and
    // for an account-backed one it is the caller saying the operator is signed
    // in: see [`PluginKind::account_backed`].
    account: Option<&str>,
    open: impl FnOnce(&str) -> Result<(), String>,
) -> Result<Plugin, PluginError> {
    // An account-backed plugin never runs the browser dance. Its server is the
    // operator's own account, which is already signed in, and the row stores no
    // grant: the token rotates on the account's clock, so a copy here would be
    // a second thing to keep fresh and a second thing to be stale.
    if kind.account_backed() {
        let token = account.ok_or(PluginError::NoAccount { label: kind.label() })?;
        let session = mcp::open(endpoint, Some(token)).await?;
        let tools = read_tools(&session).await?;
        let account_label = account_label(&session, kind);
        return Ok(store.save_plugin(group, kind, &account_label, &tools, None)?);
    }

    let (session, grant) = match mcp::open(endpoint, None).await {
        Ok(session) => (session, None),
        Err(err) if err.is_unauthorized() => {
            let challenge = match &err {
                McpError::Unauthorized { challenge, .. } => challenge.clone(),
                _ => None,
            };
            let grant = oauth::authorize(endpoint, challenge.as_deref(), open, now_ms).await?;
            let session = mcp::open(endpoint, Some(&grant.access_token)).await?;
            (session, Some(grant))
        }
        Err(err) => return Err(err.into()),
    };

    let tools = read_tools(&session).await?;
    let account_label = account_label(&session, kind);

    Ok(store.save_plugin(group, kind, &account_label, &tools, grant.as_ref())?)
}

async fn read_tools(session: &mcp::Session) -> Result<Vec<PluginTool>, PluginError> {
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
fn account_label(session: &mcp::Session, kind: PluginKind) -> String {
    let name = session.server_name.clone();
    if name.eq_ignore_ascii_case(kind.label()) {
        String::new()
    } else {
        name
    }
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
/// Which plugin a call is for, and on whose behalf.
///
/// Grouped rather than passed loose because the five travel together and mean
/// nothing apart: the reach check needs the first three, the transport needs
/// the last two, and a caller that had four of them would be about to guess the
/// fifth.
pub struct Target<'a> {
    pub group: GroupId,
    pub agent: AgentId,
    pub kind: PluginKind,
    /// See `connect`: production passes `kind.endpoint()`.
    pub endpoint: &'a str,
    /// See `connect`: the machine's account token, for an account-backed kind.
    pub account: Option<&'a str>,
}

pub async fn call(
    store: &Store,
    target: Target<'_>,
    tool: &str,
    arguments: &serde_json::Value,
) -> Result<String, PluginError> {
    let Target { group, agent, kind, endpoint, account } = target;
    let (id, grant) = match store.plugin_reach(group, agent, kind, tool)? {
        PluginReach::Granted { id, grant } => (id, grant),
        PluginReach::NotConnected => return Err(PluginError::NotConnected { label: kind.label() }),
        PluginReach::NotChosen => return Err(PluginError::NotChosen { label: kind.label() }),
        PluginReach::ToolDenied => {
            return Err(PluginError::ToolDenied { label: kind.label(), tool: tool.to_string() })
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
        let token = account.ok_or(PluginError::NoAccount { label: kind.label() })?;
        let session = mcp::open(endpoint, Some(token)).await?;
        return Ok(mcp::call_tool(&session, tool, arguments).await?);
    }

    let grant = match grant {
        Some(held) if held.stale(now_ms()) => Some(renew(store, id, &held, kind).await?),
        other => other,
    };

    let attempt = run(endpoint, grant.as_ref(), tool, arguments).await;
    match attempt {
        Err(McpError::Unauthorized { .. }) => {
            // One retry, and only for a grant there is something to renew. A
            // public server answering 401 is a server that changed its mind
            // about being public, which no refresh fixes.
            let Some(held) = grant else {
                return Err(PluginError::SigninExpired { label: kind.label() });
            };
            let renewed = renew(store, id, &held, kind).await?;
            run(endpoint, Some(&renewed), tool, arguments).await.map_err(Into::into)
        }
        other => other.map_err(Into::into),
    }
}

async fn run(
    endpoint: &str,
    grant: Option<&Grant>,
    tool: &str,
    arguments: &serde_json::Value,
) -> Result<String, McpError> {
    let session = mcp::open(endpoint, grant.map(|g| g.access_token.as_str())).await?;
    mcp::call_tool(&session, tool, arguments).await
}

/// Renews a grant and writes it back, so the next turn does not renew it again.
async fn renew(
    store: &Store,
    id: PluginId,
    grant: &Grant,
    kind: PluginKind,
) -> Result<Grant, PluginError> {
    let renewed = oauth::refresh(grant, now_ms)
        .await
        .map_err(|_| PluginError::SigninExpired { label: kind.label() })?;
    store.refresh_plugin_grant(id, &renewed)?;
    Ok(renewed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugin_that_is_not_connected_says_who_can_connect_it() {
        // An agent reads this mid-turn. A refusal that only says no gets
        // reworded and retried; this one has to close that door, because
        // nothing the agent can do will open it.
        let refusal = PluginError::NotConnected { label: "Neon" }.to_string();
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
        let refusal = PluginError::NotChosen { label: "Stripe" }.to_string();
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
            PluginError::ToolDenied { label: "Stripe", tool: "create_refund".into() }.to_string();
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
        let refusal = PluginError::SigninExpired { label: "Cloudflare" }.to_string();
        assert!(refusal.contains("Cloudflare"));
        assert!(refusal.contains("connect it again"));
    }
}
