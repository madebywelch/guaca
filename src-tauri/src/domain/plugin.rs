//! Plugins: the services a crew can reach through their own MCP server.
//!
//! A plugin is not a credential. It is a remote server the operator signs in
//! to once, on behalf of a group, after which that server's tools are offered
//! to every agent in the crew on every turn. Nothing is pasted, nothing is put
//! into a machine's environment, and the agent never holds a token: the call
//! goes out of Guaca over HTTP with the group's own grant on it.
//!
//! That is the whole reason this exists beside [`super::connector`], which is
//! the other half and stays. A credential is for a service an agent reaches by
//! running a command on its machine, and there is no list of those: it is
//! whatever the operator holds a token for. A plugin is a service that
//! publishes tools, and the list is short on purpose.
//!
//! ## Why the catalogue is short and lives in Rust
//!
//! The old list was twelve brands and a text box, which asked the operator to
//! know four things about a token they had: the variable it belongs in, the
//! account it acts as, a note for the agent, and whether the service was worth
//! wiring up at all. A handful of servers that answer all four themselves is a
//! better offer than twelve that answer none.
//!
//! What is on it is decided mechanically rather than editorially: a server has
//! to publish its own tools, act on the operator's account rather than describe
//! how to, and let an application register itself. `docs/PLUGINS.md` argues each
//! of the three, and the third is what `scripts/plugins.sh` checks against the
//! live vendors.
//!
//! It is in Rust rather than in the webview because the runtime is what makes
//! the call. The old catalogue could live in the front end precisely because
//! the backend had no opinion: it stored whatever service name it was handed.
//! An endpoint the runtime dials is not that, and a second copy of it in the
//! webview would be a second place for it to be wrong.
//!
//! ## What is stored, and what never crosses IPC
//!
//! The grant. An access token, its refresh token, and the client registration
//! that produced them are columns on the row and are never returned by a
//! command, never rendered into a prompt and never sent to a model. This is the
//! same boundary `connector_env` draws around a pasted secret, and the reason
//! is the same: the operator's Neon account has more behind it than Guaca.

use serde::{Deserialize, Serialize};

use super::ids::{GroupId, PluginId};

/// One of the servers Guaca knows how to sign in to.
///
/// A closed set, and it has to stay closed: the endpoint is what the runtime
/// dials, so "any MCP server" would mean an operator typing a URL that Guaca
/// then sends the crew's tokens to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginKind {
    Neon,
    Cloudflare,
    Linear,
    Stripe,
    Agentmail,
}

impl PluginKind {
    pub const ALL: [PluginKind; 5] = [
        PluginKind::Neon,
        PluginKind::Cloudflare,
        PluginKind::Linear,
        PluginKind::Stripe,
        PluginKind::Agentmail,
    ];

    /// The stored form, and the prefix every one of its tools is offered under.
    ///
    /// Lowercase and one word, because it is half of a tool name and a model
    /// reads `neon__run_sql` before it reads anything else about the call.
    pub const fn slug(self) -> &'static str {
        match self {
            PluginKind::Neon => "neon",
            PluginKind::Cloudflare => "cloudflare",
            PluginKind::Linear => "linear",
            PluginKind::Stripe => "stripe",
            PluginKind::Agentmail => "agentmail",
        }
    }

    pub fn from_slug(slug: &str) -> Option<PluginKind> {
        PluginKind::ALL.into_iter().find(|kind| kind.slug() == slug)
    }

    pub const fn label(self) -> &'static str {
        match self {
            PluginKind::Neon => "Neon",
            PluginKind::Cloudflare => "Cloudflare",
            PluginKind::Linear => "Linear",
            PluginKind::Stripe => "Stripe",
            PluginKind::Agentmail => "AgentMail",
        }
    }

    /// The MCP server this plugin is.
    ///
    /// Written out in full rather than assembled from a host and a path,
    /// because this string is two things at once: the URL the runtime POSTs to
    /// and the RFC 8707 resource indicator the sign-in is scoped to. Stripe's
    /// has no path and the other four do, and each is the identifier that
    /// vendor publishes in its own protected-resource metadata. A tidier
    /// `format!("{host}/mcp")` would be a resource the server does not
    /// recognise, and the refusal arrives in the operator's browser.
    ///
    /// Cloudflare publishes two kinds of server, and this is the account-wide
    /// one. The fifteen `*.mcp.cloudflare.com` hosts are one product area each,
    /// so picking one is picking which fifteenth of the operator's account the
    /// crew gets, and picking several is a hundred tool definitions in front of
    /// a model that asked for "Cloudflare". `mcp.cloudflare.com` is the whole
    /// API behind `search` and `execute`: the model writes JavaScript against
    /// the OpenAPI document, Cloudflare runs it, and 2,500 endpoints cost about
    /// a thousand tokens of context instead of a million.
    pub const fn endpoint(self) -> &'static str {
        match self {
            PluginKind::Neon => "https://mcp.neon.tech/mcp",
            PluginKind::Cloudflare => "https://mcp.cloudflare.com/mcp",
            PluginKind::Linear => "https://mcp.linear.app/mcp",
            PluginKind::Stripe => "https://mcp.stripe.com",
            PluginKind::Agentmail => "https://mcp.agentmail.to/mcp",
        }
    }

    /// One line on the tile, in terms of what the crew gets.
    pub const fn blurb(self) -> &'static str {
        match self {
            PluginKind::Neon => "Postgres databases: branch one, run SQL, migrate it.",
            PluginKind::Cloudflare => "The whole API: Workers, DNS, R2, D1, Zero Trust.",
            PluginKind::Linear => "Issues and projects: find them, file them, move them on.",
            PluginKind::Stripe => "The live account: payments, customers, invoices, refunds.",
            PluginKind::Agentmail => "Inboxes an agent owns: read a thread, send, reply, forward.",
        }
    }

    /// Where the operator can read what they are about to authorise.
    pub const fn docs(self) -> &'static str {
        match self {
            PluginKind::Neon => "https://neon.com/docs/ai/neon-mcp-server",
            PluginKind::Cloudflare => {
                "https://developers.cloudflare.com/agents/model-context-protocol/cloudflare/servers-for-cloudflare/"
            }
            PluginKind::Linear => "https://linear.app/docs/mcp",
            PluginKind::Stripe => "https://docs.stripe.com/mcp",
            PluginKind::Agentmail => "https://www.agentmail.to/docs/integrations/mcp",
        }
    }
}

/// What the front end draws before anything is connected.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginOffer {
    pub kind: PluginKind,
    pub name: &'static str,
    pub blurb: &'static str,
    pub docs: &'static str,
    /// The host the sign-in and every later call goes to, so an operator can
    /// see where their account is being handed to before they click.
    pub endpoint: &'static str,
}

pub fn catalogue() -> Vec<PluginOffer> {
    PluginKind::ALL
        .into_iter()
        .map(|kind| PluginOffer {
            kind,
            name: kind.label(),
            blurb: kind.blurb(),
            docs: kind.docs(),
            endpoint: kind.endpoint(),
        })
        .collect()
}

/// One tool a connected server offers.
///
/// Read once, when the plugin is connected, and kept. A `tools/list` on every
/// turn would be a network round trip in front of every model call, paid by
/// every agent in the crew, to re-learn something that changes when a vendor
/// ships rather than when an agent thinks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginTool {
    /// The server's own name for it, unprefixed.
    pub name: String,
    pub description: String,
    /// JSON Schema, straight from the server and passed to the model as-is.
    pub input_schema: serde_json::Value,
}

/// A plugin a group has connected.
///
/// Serialisable in full: there is no grant on it. The tokens live in the store
/// and only ever leave it onto the wire to the server they came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    pub id: PluginId,
    /// Scoped to a group, like everything else an agent can see.
    pub group_id: GroupId,
    pub kind: PluginKind,
    /// Who the grant turned out to be for, when the server said. Often blank:
    /// an MCP server is under no obligation to name the account it authorised,
    /// and inventing a label would be worse than an empty one.
    pub account: String,
    /// What this crew can call, by unprefixed name. The schemas are not here:
    /// they are bulk the webview has no use for, and the runtime reads them
    /// from the store on the turn that needs them.
    pub tools: Vec<String>,
    /// False for a server that authorised nothing because it asked for nothing.
    /// Every server on the list today asks, so this is true in practice; it is
    /// read off whether a grant was actually issued rather than off the fact
    /// that connecting succeeded, because a server that stops asking must not
    /// make the row claim a sign-in that never happened.
    pub signed_in: bool,
    pub connected_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_is_offered_exactly_once() {
        let offered = catalogue();
        assert_eq!(offered.len(), PluginKind::ALL.len());
        for kind in PluginKind::ALL {
            assert_eq!(offered.iter().filter(|o| o.kind == kind).count(), 1);
        }
    }

    #[test]
    fn a_slug_is_one_lowercase_word_because_it_is_half_of_a_tool_name() {
        // A model is offered `neon__run_sql`. Anything in here that a provider
        // would reject in a function name breaks every tool the plugin has, and
        // it breaks it at the moment the crew tries to use it rather than here.
        for kind in PluginKind::ALL {
            let slug = kind.slug();
            assert!(!slug.is_empty());
            assert!(
                slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                "{slug} has to survive being part of a tool name"
            );
            assert_eq!(PluginKind::from_slug(slug), Some(kind));
        }
    }

    #[test]
    fn a_slug_that_is_not_offered_is_not_a_plugin() {
        assert_eq!(PluginKind::from_slug("github"), None);
        assert_eq!(PluginKind::from_slug(""), None);
        // Clerk was withdrawn: its server hands out SDK snippets and touches no
        // account, so it was documentation behind a sign-in prompt. Migration 25
        // deletes the rows, and this is what keeps one that survives a downgrade
        // from resolving to a plugin again.
        assert_eq!(PluginKind::from_slug("clerk"), None);
    }

    #[test]
    fn every_endpoint_is_https_and_named_in_full() {
        // The endpoint is what the runtime dials with the crew's grant on it. A
        // scheme-relative or plain-http entry here is a token sent in the open.
        for kind in PluginKind::ALL {
            assert!(kind.endpoint().starts_with("https://"), "{}", kind.label());
            assert!(kind.docs().starts_with("https://"), "{}", kind.label());
        }
    }

    #[test]
    fn cloudflare_is_the_account_wide_server_and_not_one_product_area() {
        // A `*.mcp.cloudflare.com` host is one product area of fifteen, so an
        // operator who connected "Cloudflare" would get a crew that can make a
        // Worker and cannot see a DNS record, with nothing on the tile saying
        // so. The apex host is the whole API behind `search` and `execute`.
        let endpoint = PluginKind::Cloudflare.endpoint();
        assert_eq!(endpoint, "https://mcp.cloudflare.com/mcp");
        assert!(
            !endpoint.contains(".mcp.cloudflare.com"),
            "a subdomain here is one product area, not the account: {endpoint}"
        );
    }

    #[test]
    fn a_kind_round_trips_through_json_as_the_slug() {
        // The webview asks to connect by kind, and the tile it clicked was
        // drawn from the same value. A rename on one side has to fail here.
        for kind in PluginKind::ALL {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.slug()));
            assert_eq!(serde_json::from_str::<PluginKind>(&json).unwrap(), kind);
        }
    }
}
