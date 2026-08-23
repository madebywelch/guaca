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

use crate::db::store::{Store, StoreError};
use crate::domain::ids::{GroupId, PluginId};
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
    open: impl FnOnce(&str) -> Result<(), String>,
) -> Result<Plugin, PluginError> {
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

    let tools: Vec<PluginTool> = mcp::list_tools(&session)
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
        .collect();

    // Only when it says something other than its own product name. "Neon MCP
    // Server" as an account label is worse than a blank one: it looks like an
    // answer to "whose account is this" and is not.
    let account = session.server_name.clone();
    let account = if account.eq_ignore_ascii_case(kind.label()) { String::new() } else { account };

    Ok(store.save_plugin(group, kind, &account, &tools, grant.as_ref())?)
}

/// Runs one of a plugin's tools on behalf of an agent.
///
/// Renews the grant first when it is close to expiring, and once more if the
/// server rejects it anyway: a token can be revoked at the vendor between one
/// turn and the next, and a clock that is a little wrong makes "close to
/// expiring" a guess rather than a fact.
pub async fn call(
    store: &Store,
    group: GroupId,
    kind: PluginKind,
    // See `connect`: production passes `kind.endpoint()`.
    endpoint: &str,
    tool: &str,
    arguments: &serde_json::Value,
) -> Result<String, PluginError> {
    let Some((id, grant)) = store.plugin_grant(group, kind)? else {
        return Err(PluginError::NotConnected { label: kind.label() });
    };

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
    fn an_expired_sign_in_says_that_connecting_again_is_the_fix() {
        let refusal = PluginError::SigninExpired { label: "Cloudflare" }.to_string();
        assert!(refusal.contains("Cloudflare"));
        assert!(refusal.contains("connect it again"));
    }
}
