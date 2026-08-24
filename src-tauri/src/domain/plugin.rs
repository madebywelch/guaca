//! Plugins: the services a crew can reach through their own MCP server.
//!
//! A plugin is not a credential. It is a remote server the operator signs in
//! to once, on behalf of a group, after which the agents that group chose are
//! offered that server's tools on every turn. Nothing is pasted, nothing is put
//! into a machine's environment, and the agent never holds a token: the call
//! goes out of Guaca over HTTP with the group's own grant on it.
//!
//! Signing in and being allowed to spend the sign-in are two decisions, and
//! [`PluginAccess`] is the second. Everyone in the crew is the default and the
//! usual answer; Stripe is the one that is not.
//!
//! That is the whole reason this exists beside [`super::connector`], which is
//! the other half and stays. A credential is for a service an agent reaches by
//! running a command on its machine, and there is no list of those: it is
//! whatever the operator holds a token for. A plugin is a service that
//! publishes tools, and the list is short on purpose.
//!
//! ## Why the catalog is short and lives in Rust
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
//! the call. The old catalog could live in the front end precisely because
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

use super::ids::{AgentId, GroupId, PluginId};

/// Which server a plugin is, and where the runtime dials it.
///
/// Six Guaca ships the address of, and whatever the operator added. The six are
/// a catalog rather than a limit: each one is on the list because somebody
/// checked that it publishes its own tools, acts on the operator's account and
/// lets an application register itself, and what that check buys is a tile with
/// a name, a sentence and a working sign-in behind one click.
///
/// [`PluginKind::Custom`] is the same mechanism with none of that checking
/// done. The operator supplies the two things the catalog was supplying — a
/// name and an address — and everything after that is identical: the same
/// probe, the same sign-in, the same tool list, the same per-agent and per-tool
/// answers, the same grant that never reaches a model. What is different is
/// that nobody vouched for the server, which is a sentence in the UI rather
/// than a different code path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PluginKind {
    Neon,
    Cloudflare,
    Linear,
    Stripe,
    Agentmail,
    /// The operator's own Guaca account, as a server. See [`PluginKind::account_backed`].
    Google,
    /// An MCP server the operator named and addressed themselves.
    ///
    /// The name is one word and it is the *only* name: it is what the panel
    /// shows, what the prompt says, and the prefix every one of its tools is
    /// called by. A display name beside it would be a second name that drifts
    /// from the one the agent types, and there is nowhere for the drift to
    /// show up except in a turn that cannot find a tool.
    ///
    /// The address is on the variant rather than looked up, because for this
    /// one there is nothing to look it up in: it is the row, and a row that has
    /// lost it is a row nothing can dial. See [`PluginKind::from_row`].
    Custom {
        slug: String,
        endpoint: String,
    },
}

/// Why a server an operator typed cannot be added.
///
/// Each says what to change rather than what was wrong. An operator adding a
/// server has a URL in their hand and no idea what shape Guaca wants it in.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CustomError {
    #[error(
        "a server needs a name. One word, letters and digits, which is what its tools will be \
         called by: `linear__create_issue` is what the crew sees."
    )]
    NoName,
    #[error(
        "{0} is not a name a tool can be called by. Use letters and digits, starting with a \
         letter: it becomes the first half of every tool name this server offers."
    )]
    BadName(String),
    #[error(
        "{0} is the name of a server Guaca already ships. Pick another name, or connect that one \
         from the list above."
    )]
    TakenName(String),
    #[error(
        "{0} is longer than {max} characters. Its name is the first half of every tool name this \
         server offers, and a provider refuses a function name past 64.",
        max = MAX_CUSTOM_NAME
    )]
    LongName(String),
    #[error("a server needs an address: the URL its MCP endpoint answers on, such as https://example.com/mcp.")]
    NoUrl,
    #[error(
        "{0} is not a URL Guaca can dial. It needs a scheme and a host, such as \
         https://example.com/mcp."
    )]
    BadUrl(String),
    #[error(
        "{0} is not https. A crew's sign-in and everything its agents send would cross the \
         network in the open. Use https, or a loopback address if the server is on this machine."
    )]
    Insecure(String),
    #[error(
        "{0} has a fragment on it. An MCP endpoint is the address a token is issued for, and a \
         fragment is never sent to a server: drop everything from the # onward."
    )]
    Fragment(String),
}

/// The longest name a custom server may have.
///
/// A provider refuses a function name past 64 characters, and a plugin's tool
/// is `name__tool`. Half is a name long enough for anything an operator would
/// type and short enough to leave a real tool name room on the other side.
pub const MAX_CUSTOM_NAME: usize = 32;

/// Headers the operator wrote, sent on every request to their own server.
///
/// The third thing a server somebody runs can want, after an address and a
/// token, and the one the catalog never needs: an `X-API-Key` because that is
/// where its framework looks, a pair of `Cf-Access-Client-*` because it sits
/// behind Cloudflare Access, a tenant id because one deployment serves several.
/// None of those is a sign-in and none of them is discoverable, so there is
/// nowhere for them to come from but the person who deployed the thing.
///
/// They are a property of *reaching* the server rather than of who the crew is,
/// which is why they are not a [`crate::plugins::Credential`] and compose with
/// every one of them. A server gated by Access and signed in to with OAuth is
/// the case that proves it: the headers get the request past the gate, and the
/// 401 behind the gate still starts the browser dance.
///
/// Every value is a secret and is stored, spent and hidden exactly as a pasted
/// key is. What crosses IPC on the way back out is [`Headers::names`] and never
/// a value, which is the same boundary a connector's secret has: the panel can
/// say what this server is being sent, and cannot say what it is worth.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers(Vec<(String, String)>);

/// One header, as it crosses IPC and as it is stored.
///
/// A named pair rather than a tuple because it is what an operator fills in and
/// what a column holds, and both of those are read by somebody: a two-element
/// array in a settings file is a row nobody can tell the halves of apart. What
/// the transport takes is the tuple, which is what `reqwest` wants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeaderPair {
    pub name: String,
    pub value: String,
}

/// Why a header an operator typed cannot be sent.
///
/// Each names the header and says what to change. An operator pasting a header
/// out of a vendor's `curl` example has no idea which of these Guaca reserves,
/// and a refusal that only says no gets retyped verbatim.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HeaderError {
    #[error("a header needs a name, such as `X-API-Key`. Remove the empty row or fill it in.")]
    NoName,
    #[error(
        "{0} is not a header name. Use letters, digits and `-`, such as `X-API-Key`: a header \
         name cannot contain spaces, colons or quotes. Paste only the part before the colon."
    )]
    BadName(String),
    #[error(
        "{0} needs a value. If the header is meant to be empty, remove it: a server that treats \
         an absent header and an empty one alike is rare enough not to guess at."
    )]
    NoValue(String),
    #[error(
        "{0}'s value has a line break or a control character in it. That is usually a key pasted \
         with the newline after it: paste the key alone."
    )]
    BadValue(String),
    #[error(
        "{0} is a header Guaca builds itself, and one written here would contradict the request \
         it is on. The server would refuse the call and say nothing you could act on."
    )]
    Reserved(String),
    #[error("{0} is given twice. A header has one value; remove the row you did not mean.")]
    Repeated(String),
    #[error(
        "that is more than {MAX_HEADERS} headers. A server needing more than that is one being \
         configured through Guaca rather than reached through it."
    )]
    TooMany,
    #[error(
        "{0}'s value is longer than {MAX_HEADER_VALUE} characters. That is longer than any \
         credential, and a proxy will refuse the request before the server sees it."
    )]
    LongValue(String),
}

/// How many headers one server may be sent.
///
/// Two is the real answer — Cloudflare Access wants a pair — and eight is room
/// for a deployment nobody anticipated. A list past that is a configuration
/// file, and this is not one.
pub const MAX_HEADERS: usize = 8;

/// The longest value a header may carry.
///
/// Long enough for a signed JWT, which is the largest credential anybody sends
/// this way, and short enough that the whole set stays inside the header
/// budget of every proxy between here and the server.
pub const MAX_HEADER_VALUE: usize = 4096;

/// The headers this client builds from the request it is on.
///
/// Written here rather than checked against what `mcp.rs` happens to send,
/// because the failure is silent: a modern server compares `mcp-method` and
/// `mcp-param-*` against the body and refuses a request where they disagree,
/// with an error the operator sees as "the server rejected the call". The
/// `mcp-` prefix covers those and everything the protocol adds later; the rest
/// are the ones an HTTP client owns.
const RESERVED: [&str; 5] = ["accept", "content-type", "content-length", "host", "connection"];

impl Headers {
    /// None, which is what a catalog server and most added ones send.
    pub fn none() -> Headers {
        Headers(Vec::new())
    }

    /// What the operator typed, if every row of it holds up.
    ///
    /// Names are lowercased, because HTTP field names are case-insensitive and
    /// two spellings of one header are the duplicate this refuses rather than
    /// two headers. What the panel shows is therefore the stored spelling and
    /// not the typed one, which is the same rule the server's name follows and
    /// for the same reason: the webview draws what came back.
    pub fn parse(rows: &[HeaderPair]) -> Result<Headers, HeaderError> {
        if rows.len() > MAX_HEADERS {
            return Err(HeaderError::TooMany);
        }
        let mut out: Vec<(String, String)> = Vec::new();
        for row in rows {
            let name = row.name.trim().to_ascii_lowercase();
            let value = row.value.trim().to_string();
            if name.is_empty() {
                return Err(HeaderError::NoName);
            }
            if !name.bytes().all(is_token) {
                return Err(HeaderError::BadName(row.name.trim().to_string()));
            }
            if name.starts_with("mcp-") || RESERVED.contains(&name.as_str()) {
                return Err(HeaderError::Reserved(name));
            }
            if value.is_empty() {
                return Err(HeaderError::NoValue(name));
            }
            if value.len() > MAX_HEADER_VALUE {
                return Err(HeaderError::LongValue(name));
            }
            // Visible ASCII and space only. A newline is the common one and it
            // is not cosmetic: a value carrying CRLF is header injection, and
            // the only way it gets into a box like this is a key copied with
            // the line ending attached.
            if !value.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
                return Err(HeaderError::BadValue(name));
            }
            if out.iter().any(|(held, _)| held == &name) {
                return Err(HeaderError::Repeated(name));
            }
            out.push((name, value));
        }
        Ok(Headers(out))
    }

    /// The stored form: JSON, in the one column that holds it.
    pub fn encode(&self) -> String {
        let rows: Vec<HeaderPair> = self
            .0
            .iter()
            .map(|(name, value)| HeaderPair { name: name.clone(), value: value.clone() })
            .collect();
        serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())
    }

    /// A stored row, back as headers.
    ///
    /// A column that will not parse is no headers rather than an error, for the
    /// reason an unreadable plugin row is skipped: it can only come from a
    /// newer build writing to the same file, and a crew losing a routing header
    /// is a plugin that stops working with a message the operator can act on,
    /// where a raised error is every agent in the crew losing its turn.
    pub fn decode(stored: &str) -> Headers {
        serde_json::from_str::<Vec<HeaderPair>>(stored)
            .ok()
            .and_then(|rows| Headers::parse(&rows).ok())
            .unwrap_or_default()
    }

    /// What the panel is allowed to know: which headers, never their values.
    pub fn names(&self) -> Vec<String> {
        self.0.iter().map(|(name, _)| name.clone()).collect()
    }

    /// Whether the operator supplied the credential themselves.
    ///
    /// `authorization` is allowed here on purpose — it is the only way to send
    /// `Basic`, or a scheme a vendor invented — and it is the one header that
    /// collides with a pasted key, which goes in the same slot. The caller that
    /// has both refuses rather than picking one: see `commands::add_plugin`.
    pub fn carries_authorization(&self) -> bool {
        self.0.iter().any(|(name, _)| name == "authorization")
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The pairs, for the one caller that puts them on a request.
    pub fn wire(&self) -> &[(String, String)] {
        &self.0
    }
}

/// The characters RFC 9110 allows in a field name.
fn is_token(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

impl PluginKind {
    /// The servers Guaca ships the address of. Not every kind: a custom one is
    /// not offered, it is added.
    pub const ALL: [PluginKind; 6] = [
        PluginKind::Neon,
        PluginKind::Cloudflare,
        PluginKind::Linear,
        PluginKind::Stripe,
        PluginKind::Agentmail,
        PluginKind::Google,
    ];

    /// One the operator typed, if both halves of it hold up.
    ///
    /// The name is normalized rather than demanded in a particular shape:
    /// "Home Assistant" becomes `home_assistant`, because an operator naming a
    /// server is thinking about the server and not about what a provider
    /// accepts in a function name. What comes back is shown to them
    /// immediately, so the name they end up with is one they saw before they
    /// pressed the button.
    ///
    /// The address is normalized the other way, to the canonical form RFC 8707
    /// asks for: the trailing slash goes, because that string is not only the
    /// URL a POST goes to but the resource identifier the sign-in is scoped to,
    /// and a server that publishes itself without one refuses a token issued
    /// for the other.
    pub fn custom(name: &str, url: &str) -> Result<PluginKind, CustomError> {
        let slug = normalize_name(name);
        if slug.is_empty() {
            return Err(CustomError::NoName);
        }
        if !slug.starts_with(|c: char| c.is_ascii_lowercase()) {
            return Err(CustomError::BadName(name.trim().to_string()));
        }
        if slug.len() > MAX_CUSTOM_NAME {
            return Err(CustomError::LongName(slug));
        }
        if PluginKind::from_slug(&slug).is_some() {
            return Err(CustomError::TakenName(slug));
        }
        Ok(PluginKind::Custom { slug, endpoint: canonical_url(url)? })
    }

    /// The stored form, and the prefix every one of its tools is offered under.
    ///
    /// Lowercase and one word, because it is half of a tool name and a model
    /// reads `neon__run_sql` before it reads anything else about the call.
    pub fn slug(&self) -> &str {
        match self {
            PluginKind::Neon => "neon",
            PluginKind::Cloudflare => "cloudflare",
            PluginKind::Linear => "linear",
            PluginKind::Stripe => "stripe",
            PluginKind::Agentmail => "agentmail",
            PluginKind::Google => "google",
            PluginKind::Custom { slug, .. } => slug,
        }
    }

    /// One of the six, by the name it is stored under. Never a custom one:
    /// those carry an address a slug alone cannot supply, and the caller that
    /// has one is reading a row. See [`PluginKind::from_row`].
    pub fn from_slug(slug: &str) -> Option<PluginKind> {
        PluginKind::ALL.into_iter().find(|kind| kind.slug() == slug)
    }

    /// A stored row's two columns, as the kind they describe.
    ///
    /// `None` is a row this build cannot dial, and there is exactly one way to
    /// be one: a slug that is not in the catalog and no address beside it,
    /// which is what a newer build's plugin looks like after a downgrade. A row
    /// that carries its own address needs no build knowledge at all, which is
    /// the whole point of a custom server and the reason this is not the same
    /// question as `from_slug`.
    ///
    /// The address is ignored for a catalog kind. Where its server lives is a
    /// decision this build makes and re-makes on every release: a vendor that
    /// moves is a new version of Guaca, not a stale string in an old row.
    pub fn from_row(slug: &str, endpoint: &str) -> Option<PluginKind> {
        if let Some(known) = PluginKind::from_slug(slug) {
            return Some(known);
        }
        if endpoint.is_empty() || normalize_name(slug) != slug {
            return None;
        }
        Some(PluginKind::Custom { slug: slug.to_string(), endpoint: endpoint.to_string() })
    }

    /// What a person and a model call it.
    ///
    /// For a custom server that is its name, which is also its slug: one string
    /// rather than two that can disagree. See [`PluginKind::Custom`].
    pub fn label(&self) -> &str {
        match self {
            PluginKind::Neon => "Neon",
            PluginKind::Cloudflare => "Cloudflare",
            PluginKind::Linear => "Linear",
            PluginKind::Stripe => "Stripe",
            PluginKind::Agentmail => "AgentMail",
            PluginKind::Google => "Google",
            PluginKind::Custom { slug, .. } => slug,
        }
    }

    /// Whether this is a server the operator added rather than one on the list.
    ///
    /// Read by the panel, which has to say that nobody vouched for it, and by
    /// nothing on the call path: a custom server is dialled, signed in to and
    /// narrowed by exactly the code the other six are.
    pub fn is_custom(&self) -> bool {
        matches!(self, PluginKind::Custom { .. })
    }

    /// The MCP server this plugin is.
    ///
    /// Written out in full rather than assembled from a host and a path,
    /// because this string is two things at once: the URL the runtime POSTs to
    /// and the RFC 8707 resource indicator the sign-in is scoped to. Stripe's
    /// has no path and the other four do, and each is the identifier that
    /// vendor publishes in its own protected-resource metadata. A tidier
    /// `format!("{host}/mcp")` would be a resource the server does not
    /// recognize, and the refusal arrives in the operator's browser.
    ///
    /// Cloudflare publishes two kinds of server, and this is the account-wide
    /// one. The fifteen `*.mcp.cloudflare.com` hosts are one product area each,
    /// so picking one is picking which fifteenth of the operator's account the
    /// crew gets, and picking several is a hundred tool definitions in front of
    /// a model that asked for "Cloudflare". `mcp.cloudflare.com` is the whole
    /// API behind `search` and `execute`: the model writes JavaScript against
    /// the OpenAPI document, Cloudflare runs it, and 2,500 endpoints cost about
    /// a thousand tokens of context instead of a million.
    pub fn endpoint(&self) -> &str {
        match self {
            PluginKind::Neon => "https://mcp.neon.tech/mcp",
            PluginKind::Cloudflare => "https://mcp.cloudflare.com/mcp",
            PluginKind::Linear => "https://mcp.linear.app/mcp",
            PluginKind::Stripe => "https://mcp.stripe.com",
            PluginKind::Agentmail => "https://mcp.agentmail.to/mcp",
            // The operator's own account. `account.rs` may be pointed elsewhere
            // for development, and `Runtime::plugin_endpoint` is what moves it;
            // this is the address a bundled build talks to and the one the tile
            // shows before anything is connected.
            PluginKind::Google => "https://guaca.bot/mcp",
            PluginKind::Custom { endpoint, .. } => endpoint,
        }
    }

    /// What the row stores in its `endpoint` column.
    ///
    /// Empty for a catalog kind, whose address belongs to the build rather than
    /// to the row: a vendor that moves ships as a new Guaca, and a stored copy
    /// would keep a crew dialling the old host until somebody reconnected it.
    /// A custom server has nowhere else to keep it.
    pub fn stored_endpoint(&self) -> &str {
        match self {
            PluginKind::Custom { endpoint, .. } => endpoint,
            _ => "",
        }
    }

    /// Whether this plugin's credential is the machine's Guaca account rather
    /// than a grant of its own.
    ///
    /// The other five are somebody else's servers, and a crew signs in to each
    /// one separately because there is nothing else it could do. Google is not
    /// a server: it is the operator's own account at `guaca.bot`, which already
    /// holds the grant and already refreshes it. Running a second OAuth dance
    /// for it would send an operator to a consent screen to authorize something
    /// they authorized when they signed in, and would leave a per-group grant
    /// beside a per-account one for the same access, expiring on its own clock.
    ///
    /// So the sign-in is the account's and the *decision to use it* stays the
    /// group's. Connecting it in a crew is what puts the tools in front of that
    /// crew's agents, and `PluginAccess` still decides which of them. Nothing
    /// else about a plugin changes: the call still goes out of Guaca, the token
    /// still never reaches a model, and the tool list is still read once.
    ///
    /// Never a custom server. An operator can point one at `guaca.bot` and it
    /// signs in to it the ordinary way, which is the honest outcome: the
    /// account's credential is the account's to lend, and lending it to an
    /// address somebody typed is not a decision a slug should be able to make.
    pub fn account_backed(&self) -> bool {
        matches!(self, PluginKind::Google)
    }

    /// One line on the tile, in terms of what the crew gets.
    pub fn blurb(&self) -> &str {
        match self {
            PluginKind::Neon => "Postgres databases: branch one, run SQL, migrate it.",
            PluginKind::Cloudflare => "The whole API: Workers, DNS, R2, D1, Zero Trust.",
            PluginKind::Linear => "Issues and projects: find them, file them, move them on.",
            PluginKind::Stripe => "The live account: payments, customers, invoices, refunds.",
            PluginKind::Agentmail => "Inboxes an agent owns: read a thread, send, reply, forward.",
            PluginKind::Google => {
                "Your Gmail, Calendar and Drive, through the Guaca account you signed in to."
            }
            PluginKind::Custom { .. } => {
                "A server you added. Whatever it publishes, this crew has."
            }
        }
    }

    /// Where the operator can read what they are about to authorize.
    pub fn docs(&self) -> &str {
        match self {
            PluginKind::Neon => "https://neon.com/docs/ai/neon-mcp-server",
            PluginKind::Cloudflare => {
                "https://developers.cloudflare.com/agents/model-context-protocol/cloudflare/servers-for-cloudflare/"
            }
            PluginKind::Linear => "https://linear.app/docs/mcp",
            PluginKind::Stripe => "https://docs.stripe.com/mcp",
            PluginKind::Agentmail => "https://www.agentmail.to/docs/integrations/mcp",
            PluginKind::Google => "https://guaca.bot/app",
            // Nobody wrote a page about this one. The protocol it has to speak
            // is the nearest thing to documentation Guaca can point at, and it
            // is what an operator debugging their own server wants.
            PluginKind::Custom { .. } => "https://modelcontextprotocol.io/specification",
        }
    }
}

/// A name as it will be stored, called and typed by a model.
///
/// Everything that is not a letter or a digit becomes one underscore, and runs
/// collapse. That is what makes "Home Assistant" and "home-assistant" the same
/// server rather than two, and it is also what guarantees no `__` survives,
/// which matters more than it looks: two underscores are what separate a plugin
/// from its tool, so a name containing them would split in the wrong place and
/// send a call to a plugin that does not exist.
fn normalize_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    out.trim_end_matches('_').to_string()
}

/// An address as RFC 8707 wants a resource identifier written.
///
/// Loopback is the one place plain HTTP is allowed, and it is not a loosening:
/// a server on this machine is the commonest thing an operator has written
/// themselves, and nothing on that connection leaves the machine. Everything
/// else carries a crew's grant, so it is https or it is refused here rather
/// than in a packet capture.
pub fn canonical_url(url: &str) -> Result<String, CustomError> {
    let url = url.trim();
    if url.is_empty() {
        return Err(CustomError::NoUrl);
    }
    if url.contains('#') {
        return Err(CustomError::Fragment(url.to_string()));
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err(CustomError::BadUrl(url.to_string()));
    };
    let scheme = scheme.to_ascii_lowercase();
    let (authority, path) = match rest.find(['/', '?']) {
        Some(at) => rest.split_at(at),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return Err(CustomError::BadUrl(url.to_string()));
    }
    // Credentials in the authority, which are refused rather than parsed
    // around. `http://localhost:80@evil.com/mcp` has a host of `evil.com` and a
    // *user* of `localhost:80`, so anything that reads the front of the
    // authority as the host takes it for a loopback address and lets a crew's
    // grant cross the open network to somebody else's server. An RFC 8707
    // resource identifier has no userinfo either, so there is nothing to keep.
    if authority.contains('@') {
        return Err(CustomError::BadUrl(url.to_string()));
    }
    let host = authority;
    // An IPv6 host is bracketed, so the port cannot be split off at the first
    // colon: `[::1]:8080` would come apart as `[` and the address would read as
    // some other host entirely — one that is not loopback, and would therefore
    // be refused for being plain HTTP while looking exactly like the case this
    // allows.
    let bare = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or_default().to_ascii_lowercase(),
        None => host.split(':').next().unwrap_or_default().to_ascii_lowercase(),
    };
    // Asked of the parsed address rather than matched against a list of three
    // spellings: the whole of 127.0.0.0/8 is loopback, and a prefix test would
    // take `127.evil.com` for one of them.
    let loopback =
        bare == "localhost" || bare.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
    match scheme.as_str() {
        "https" => {}
        "http" if loopback => {}
        "http" => return Err(CustomError::Insecure(url.to_string())),
        _ => return Err(CustomError::BadUrl(url.to_string())),
    }
    // Scheme and host lowercased, path left exactly as it was. The canonical
    // form of a resource identifier is lowercase in both of the first two and
    // case-sensitive in the third, and this string is compared against what the
    // server published: `HTTPS://Example.com/mcp` asks for a token scoped to a
    // resource the server does not have, and the refusal is `invalid_target` in
    // the operator's browser. A path folded with it would be a different
    // mistake in the other direction, since `/MCP` and `/mcp` are two
    // endpoints.
    let host = host.to_ascii_lowercase();
    Ok(format!("{scheme}://{host}{path}").trim_end_matches('/').to_string())
}

/// The slug, in both directions, and only the catalog on the way in.
///
/// Serializing is the whole story going out: the webview draws a row from the
/// name it was handed, and every command that acts on a connected plugin takes
/// its id. Coming in is narrower on purpose. The two commands that take a kind
/// — connecting one of the six, and pointing an account-backed one at another
/// identity — are both about servers this build ships the address of, and a
/// slug alone cannot rebuild a custom server anyway: its address is the half
/// that makes it dialable. Adding one is its own command, which takes both.
impl Serialize for PluginKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.slug())
    }
}

impl<'de> Deserialize<'de> for PluginKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let slug = String::deserialize(deserializer)?;
        PluginKind::from_slug(&slug).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "{slug:?} is not one of the servers Guaca ships the address of; a server you \
                 added is reached by its own id"
            ))
        })
    }
}

/// What the front end draws before anything is connected.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginOffer {
    pub kind: PluginKind,
    pub name: String,
    pub blurb: String,
    pub docs: String,
    /// The host the sign-in and every later call goes to, so an operator can
    /// see where their account is being handed to before they click.
    pub endpoint: String,
    /// Whether this one's credential is the operator's Guaca account, and
    /// therefore whether there is an identity to choose before connecting.
    ///
    /// Reported rather than inferred from the kind in the webview, for the
    /// reason the rest of this struct is: the runtime is what decides, and a
    /// second copy of that decision in the front end is a second place for it
    /// to be wrong.
    pub account_backed: bool,
}

pub fn catalog() -> Vec<PluginOffer> {
    PluginKind::ALL
        .into_iter()
        .map(|kind| PluginOffer {
            name: kind.label().to_string(),
            blurb: kind.blurb().to_string(),
            docs: kind.docs().to_string(),
            endpoint: kind.endpoint().to_string(),
            account_backed: kind.account_backed(),
            kind,
        })
        .collect()
}

/// What one dial of a server found out, without connecting anything.
///
/// The answer to "why does my server not work", which before this was a single
/// sentence out of whichever layer failed first. An operator debugging their own
/// deployment needs the things underneath separately: a `404` on the current
/// transport and a working event stream are the same failure to a sentence and
/// opposite instructions to a person.
///
/// It authorizes nothing and writes nothing. A server that wants a sign-in is
/// reported as wanting one rather than sent to a browser: a diagnostic that
/// opens a consent screen is a diagnostic nobody runs twice.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerReport {
    /// The address as Guaca canonicalized it, which is not always the address
    /// that was typed and is the one every later request will use.
    pub endpoint: String,
    /// Which transport it answered on, in the words the docs use for them.
    pub transport: String,
    /// The revision the two of them settled on. Empty when nothing was settled,
    /// which is a server that refused before the question was reached.
    pub protocol: String,
    /// Whether it wanted `initialize`. Not the same question as the transport:
    /// both eras are served over streamable HTTP and only one of them over the
    /// other transport, so an operator reading a bug report needs both.
    pub handshake: bool,
    pub signin: SigninNeed,
    /// How the server names itself, when it says. Often blank.
    pub server: String,
    /// Every tool it published, by name. Empty for a server that wants a
    /// sign-in, because nothing has been signed in to.
    pub tools: Vec<String>,
    /// How long the whole exchange took. The number an operator is actually
    /// after when a turn using this plugin feels slow.
    pub ms: u64,
}

/// What the server did about a credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SigninNeed {
    /// It asked for nothing and answered. A public server, or one the
    /// operator's own headers already authenticate.
    None,
    /// It refused without one. Not a failure and not a sign-in: connecting is
    /// what runs that, and this is what says it will be needed.
    Wanted,
    /// Something was presented and it was accepted.
    Accepted,
    /// Something was presented and it was refused. The one outcome that is
    /// neither reachable-and-fine nor unreachable, and the one an operator
    /// spends longest guessing at: the address is right and the key is not.
    Refused,
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

/// One of a connected plugin's tools, as the panel draws it.
///
/// The schema is not here and the description is, and that split is what the
/// row is for: an operator deciding whether a crew may call `delete_customer`
/// needs the sentence the vendor wrote about it, and has no use for the shape
/// of its arguments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginToolCard {
    /// The server's own name for it, unprefixed. Prefixed with the plugin's
    /// slug it becomes the name a model calls.
    pub name: String,
    pub description: String,
    /// Who may call this one, inside whoever the plugin itself reaches.
    ///
    /// The same two-state answer the plugin takes, one level down, and
    /// [`PluginAccess::Everyone`] until somebody says otherwise: what is
    /// written down is the narrowing, so a tool a vendor ships next month
    /// arrives on for whoever the plugin is on for.
    ///
    /// Both are needed. The plugin's answer is "who may spend this sign-in",
    /// which is about the account; this one is "who may do this particular
    /// thing with it", which is about the capability. An agent has to pass both.
    pub access: PluginAccess,
}

/// What one agent may call on one of its crew's plugins this turn, and what it
/// cannot — in the two shapes that have different ways forward.
///
/// Three lists, from one read, because all three are used on the same turn and
/// by different consumers. `offered` becomes the tool definitions the model is
/// given. `withheld` and `elsewhere` become the lines in the prompt that say a
/// capability exists and is not this agent's, and they are separate because
/// the answers are: nobody has `withheld`, so it is the operator's to switch
/// back on, and somebody has `elsewhere`, so it is a peer's to do. An agent
/// shown only `offered` reports "we cannot do refunds" when the true answer is
/// either "the operator switched refunds off" or "the agent next to you does
/// refunds".
#[derive(Debug, Clone, PartialEq)]
pub struct PluginToolset {
    pub kind: PluginKind,
    /// Callable, in the server's own order.
    pub offered: Vec<PluginTool>,
    /// Narrowed to nobody: off for everyone in the crew.
    ///
    /// Names only. A description and a schema for something that cannot be
    /// called is context paid for on every turn to describe an absence.
    pub withheld: Vec<String>,
    /// Narrowed to other agents. Names only, for the same reason.
    pub elsewhere: Vec<String>,
}

/// Who in a crew may call something: one plugin, or one of its tools.
///
/// A plugin is signed in once, for the group, and until this existed that
/// sign-in was the whole decision: every agent in the crew was offered every
/// tool it published. That reading only holds while a crew is uniform. A crew
/// is not: agents run on different models at different competencies, and the
/// one that files issues has no business holding the account that issues
/// refunds. So the sign-in stays the group's and who may spend it becomes a
/// second question, asked per plugin because that is the shape the answer has:
/// most plugins are for everybody and one or two are for one agent.
///
/// The same type answers the same question one level down, on
/// [`PluginToolCard`]. A plugin is not one capability either: two agents share
/// an inbox and want different halves of it, one reading and searching and the
/// other sending. A single answer per plugin cannot say that, and neither can
/// a single answer per tool for the whole crew — which is what this replaced.
/// The two compose rather than overlap: the plugin's answer is who may spend
/// the sign-in, the tool's is who may do that particular thing with it, and an
/// agent has to pass both.
///
/// Two states rather than a list with a sentinel, and the empty list is why.
/// `Everyone` is a decision about agents that do not exist yet — an agent hired
/// tomorrow gets it — which no list of today's ids can express. And a list that
/// meant "everyone" when it was empty would hand a plugin back to the whole
/// crew at the moment the operator unticked the last agent, which is the exact
/// opposite of what unticking means. On a tool the empty list has a name of its
/// own: it is the switched-off tool the old shape stored as a refusal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PluginAccess {
    /// Everyone in the group, including whoever joins it later.
    Everyone,
    /// These agents and nobody else. Legally empty: a plugin nobody may call is
    /// what an operator has on the way to naming the first one, and it is a
    /// state the UI says out loud rather than one this type forbids.
    Chosen { agents: Vec<AgentId> },
}

/// The stored form of [`PluginAccess::Everyone`], and the only value that
/// widens a plugin past its named agents.
///
/// Compared rather than parsed, in SQL and here, so that a value neither side
/// recognizes reads as a restriction rather than as an opening. A permission
/// that cannot be read must fail closed: the crew loses a plugin, which the
/// operator can see and fix, rather than gaining one nobody chose.
pub const ACCESS_EVERYONE: &str = "everyone";

/// The stored form of [`PluginAccess::Chosen`].
pub const ACCESS_CHOSEN: &str = "chosen";

impl PluginAccess {
    /// Whether this agent may be offered the plugin's tools, and may spend its
    /// grant. The same question in both places, asked of the same value.
    pub fn allows(&self, agent: AgentId) -> bool {
        match self {
            PluginAccess::Everyone => true,
            PluginAccess::Chosen { agents } => agents.contains(&agent),
        }
    }

    /// Whether this allows nobody at all.
    ///
    /// The one state that is true of the whole crew rather than of one agent,
    /// and the refusals turn on it: a tool nobody has is not a tool to hand to
    /// a peer. `Everyone` is never this, even in a group with no agents in it,
    /// because it is a standing answer rather than a set.
    pub fn allows_nobody(&self) -> bool {
        matches!(self, PluginAccess::Chosen { agents } if agents.is_empty())
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            PluginAccess::Everyone => ACCESS_EVERYONE,
            PluginAccess::Chosen { .. } => ACCESS_CHOSEN,
        }
    }

    /// Reads a row back. See [`ACCESS_EVERYONE`] for why anything else is a
    /// restriction rather than an error.
    pub fn from_row(access: &str, agents: Vec<AgentId>) -> Self {
        if access == ACCESS_EVERYONE {
            PluginAccess::Everyone
        } else {
            PluginAccess::Chosen { agents }
        }
    }
}

/// A plugin a group has connected.
///
/// Serializable in full: there is no grant on it. The tokens live in the store
/// and only ever leave it onto the wire to the server they came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    pub id: PluginId,
    /// Scoped to a group, like everything else an agent can see.
    pub group_id: GroupId,
    pub kind: PluginKind,
    /// What this server is called, and where it is.
    ///
    /// On the row rather than looked up from the kind, because for a server the
    /// operator added there is nothing to look it up in: the panel draws a
    /// connected plugin from what it read back, and one it cannot name is one
    /// with no way to disconnect it. For a catalog kind these are the same two
    /// strings the offer carries, which is what lets the panel draw both kinds
    /// of row with one piece of code.
    pub name: String,
    pub endpoint: String,
    /// Whether nobody vouched for this server.
    ///
    /// Reported rather than worked out in the webview by checking the slug
    /// against the catalog, for the reason the rest of this struct is: the
    /// runtime is what decides what a plugin is, and a second copy of that
    /// decision in the front end is a second place for it to be wrong.
    pub custom: bool,
    /// Who the grant turned out to be for, when the server said. Often blank:
    /// an MCP server is under no obligation to name the account it authorized,
    /// and inventing a label would be worse than an empty one.
    pub account: String,
    /// Every tool this server published, each with who may call it. The schemas
    /// are not here: they are bulk the webview has no use for, and the runtime
    /// reads them from the store on the turn that needs them.
    ///
    /// The whole list, including the narrowed and the switched-off. A panel that
    /// only drew the callable ones would be a panel with no way to switch
    /// anything back on, and no way to see what the crew is not being offered.
    pub tools: Vec<PluginToolCard>,
    /// Which of the crew may call them. The sign-in behind this row is the
    /// group's either way: this decides who is allowed to spend it, not who
    /// holds it.
    pub access: PluginAccess,
    /// Which authorized identity at the operator's Guaca account this crew uses.
    ///
    /// Only meaningful for an account-backed kind, and empty is the account's
    /// default. A person can authorize the same provider twice — a work Google
    /// and a personal one — and those are two grants with two ids; this is how
    /// one group says which of them it means while another says the other.
    pub connection: String,
    /// Which headers the operator gave this server, by name and never by value.
    ///
    /// Drawn so the panel can say what is being sent — an operator debugging
    /// their own server needs to know whether `x-api-key` is on the request —
    /// without the panel being a place a credential can be read back out of.
    /// Empty for every catalog server and for most added ones.
    pub headers: Vec<String>,
    /// False for a server that authorized nothing because it asked for nothing.
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
        let offered = catalog();
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

    fn header(name: &str, value: &str) -> HeaderPair {
        HeaderPair { name: name.into(), value: value.into() }
    }

    #[test]
    fn a_header_is_stored_lowercased_and_goes_out_that_way() {
        // Field names are case-insensitive on the wire, so the stored spelling
        // is the one the panel shows and the one duplicates are found by.
        let headers = Headers::parse(&[header("X-API-Key", " abc ")]).unwrap();
        assert_eq!(headers.wire(), [("x-api-key".to_string(), "abc".to_string())]);
        assert_eq!(headers.names(), ["x-api-key"]);
    }

    #[test]
    fn two_spellings_of_one_header_are_a_duplicate_rather_than_two_headers() {
        // Sent as written, one would silently win. Which one is `reqwest`'s
        // insertion order, which is not a decision anybody made.
        let both = Headers::parse(&[header("X-Api-Key", "a"), header("x-api-key", "b")]);
        assert_eq!(both, Err(HeaderError::Repeated("x-api-key".into())));
    }

    #[test]
    fn a_header_this_client_builds_itself_is_refused() {
        // The failure is silent otherwise: a modern server compares the
        // `mcp-*` headers against the body and refuses the request, with an
        // error the operator reads as the server rejecting their call.
        use HeaderError::*;
        assert!(matches!(Headers::parse(&[header("mcp-method", "x")]), Err(Reserved(_))));
        assert!(matches!(Headers::parse(&[header("Mcp-Param-Region", "x")]), Err(Reserved(_))));
        assert!(matches!(Headers::parse(&[header("content-type", "x")]), Err(Reserved(_))));
        assert!(matches!(Headers::parse(&[header("Accept", "x")]), Err(Reserved(_))));
    }

    #[test]
    fn authorization_is_allowed_because_it_is_the_only_way_to_send_another_scheme() {
        // `Basic`, or a scheme a vendor invented. The key box sends `Bearer`
        // and nothing else, so refusing this header would make those
        // unreachable. Who refuses the pair is `commands::presented`.
        let headers = Headers::parse(&[header("Authorization", "Basic dXNlcjpwYXNz")]).unwrap();
        assert!(headers.carries_authorization());
    }

    #[test]
    fn a_value_with_a_line_break_in_it_is_refused() {
        // Header injection, and the way it gets into a box like this is a key
        // copied with the newline after it.
        use HeaderError::*;
        assert!(matches!(Headers::parse(&[header("x-key", "a\r\nX-Admin: 1")]), Err(BadValue(_))));
        // Trailing whitespace alone is trimmed rather than refused: that is a
        // paste, not an attack, and refusing it teaches nothing.
        assert_eq!(
            Headers::parse(&[header("x-key", "abc\n")]).unwrap().wire(),
            [("x-key".to_string(), "abc".to_string())]
        );
    }

    #[test]
    fn a_name_that_is_not_a_field_name_says_which_part_to_paste() {
        use HeaderError::*;
        assert!(matches!(Headers::parse(&[header("X-API-Key:", "a")]), Err(BadName(_))));
        assert!(matches!(Headers::parse(&[header("X API Key", "a")]), Err(BadName(_))));
        assert_eq!(Headers::parse(&[header("  ", "a")]), Err(NoName));
        assert!(matches!(Headers::parse(&[header("x-key", " ")]), Err(NoValue(_))));
    }

    #[test]
    fn a_stored_column_that_will_not_parse_is_no_headers_rather_than_an_error() {
        // Same rule as an unreadable plugin row. It can only come from a newer
        // build writing to the same file, and a plugin that stops working with
        // a message beats every agent in the crew losing its turn.
        assert_eq!(Headers::decode("not json"), Headers::none());
        assert_eq!(Headers::decode(""), Headers::none());
        let round = Headers::parse(&[header("x-key", "abc")]).unwrap();
        assert_eq!(Headers::decode(&round.encode()), round);
    }

    #[test]
    fn a_header_a_server_could_never_receive_is_refused_before_it_is_stored() {
        use HeaderError::*;
        let many: Vec<HeaderPair> =
            (0..MAX_HEADERS + 1).map(|n| header(&format!("x-{n}"), "v")).collect();
        assert_eq!(Headers::parse(&many), Err(TooMany));
        let long = header("x-key", &"a".repeat(MAX_HEADER_VALUE + 1));
        assert!(matches!(Headers::parse(&[long]), Err(LongValue(_))));
    }

    #[test]
    fn a_custom_kind_serializes_as_its_name_and_does_not_come_back() {
        // Going out it is a name a row is drawn under. Coming in it is refused,
        // because the two commands that take a kind are both about servers this
        // build ships the address of, and a name on its own cannot rebuild a
        // custom server: the address is the half that makes it dialable.
        let mine = PluginKind::custom("Home Assistant", "https://ha.example.com/mcp").unwrap();
        assert_eq!(serde_json::to_string(&mine).unwrap(), "\"home_assistant\"");
        let back = serde_json::from_str::<PluginKind>("\"home_assistant\"");
        assert!(back.is_err(), "a custom name must not deserialize into a kind with no address");
    }

    #[test]
    fn a_name_becomes_the_word_its_tools_will_be_called_by() {
        // An operator naming a server is thinking about the server, not about
        // what a provider accepts in a function name. What comes back is shown
        // to them before they press the button, so the name they end up with is
        // one they saw.
        for (typed, want) in [
            ("Home Assistant", "home_assistant"),
            ("home-assistant", "home_assistant"),
            ("  Obsidian  ", "obsidian"),
            ("my.server v2", "my_server_v2"),
            // Two underscores are what separate a plugin from its tool, so a
            // name carrying a pair would split a call in the wrong place. Runs
            // collapse, which makes that impossible rather than checked for.
            ("a__b", "a_b"),
            ("weird!!!name!!!", "weird_name"),
        ] {
            let kind = PluginKind::custom(typed, "https://example.com/mcp").unwrap();
            assert_eq!(kind.slug(), want, "{typed}");
            assert_eq!(kind.label(), want, "a custom server has one name, not two");
        }
    }

    #[test]
    fn a_name_nothing_could_call_is_refused_with_the_fix_in_it() {
        use CustomError::*;
        let url = "https://example.com/mcp";
        assert_eq!(PluginKind::custom("", url), Err(NoName));
        assert_eq!(PluginKind::custom("   ", url), Err(NoName));
        assert_eq!(PluginKind::custom("!!!", url), Err(NoName));
        // A digit first is a function name some providers refuse and every
        // reader misreads. The message says to start with a letter.
        assert!(matches!(PluginKind::custom("2fast", url), Err(BadName(_))));
        // The catalog's own names, which would collide in `plugins_kind_unique`
        // and put two tool lists under one prefix.
        assert_eq!(PluginKind::custom("Neon", url), Err(TakenName("neon".into())));
        assert!(matches!(PluginKind::custom(&"x".repeat(40), url), Err(LongName(_))));
    }

    #[test]
    fn an_address_a_grant_would_cross_the_open_network_to_reach_is_refused() {
        use CustomError::*;
        assert!(matches!(PluginKind::custom("a", "http://example.com/mcp"), Err(Insecure(_))));
        assert!(matches!(PluginKind::custom("a", "ftp://example.com/mcp"), Err(BadUrl(_))));
        assert!(matches!(PluginKind::custom("a", "example.com/mcp"), Err(BadUrl(_))));
        assert!(matches!(PluginKind::custom("a", "https:///mcp"), Err(BadUrl(_))));
        assert_eq!(PluginKind::custom("a", ""), Err(NoUrl));
        // A fragment is never sent to a server, so a resource identifier that
        // carries one is a token issued for an address nothing will present.
        assert!(matches!(PluginKind::custom("a", "https://e.com/mcp#x"), Err(Fragment(_))));
    }

    #[test]
    fn a_server_on_this_machine_may_be_plain_http_and_nothing_else_may() {
        // The commonest server an operator wrote themselves, and the one case
        // where nothing on the connection leaves the machine.
        for local in [
            "http://localhost:8080/mcp",
            "http://127.0.0.1:3000/mcp",
            // The whole of 127.0.0.0/8 is loopback, not just the .1.
            "http://127.0.0.2/mcp",
            // Bracketed, so the port cannot be split off at the first colon.
            "http://[::1]/mcp",
            "http://[::1]:8080/mcp",
        ] {
            assert!(PluginKind::custom("local", local).is_ok(), "{local}");
        }
        // Not a host that merely reads like one.
        for pretender in [
            "http://localhost.evil.com/mcp",
            "http://127.evil.com/mcp",
            "http://127.0.0.1.evil.com/mcp",
            // And not one wearing a loopback address as a username. The host
            // here is `evil.com`, so accepting this would send a crew's grant
            // across the open network to somebody else's server while the
            // address read as local.
            "http://localhost:80@evil.com/mcp",
            "http://127.0.0.1@evil.com/mcp",
            "http://[::1]@evil.com/mcp",
            // Refused over https too: a resource identifier has no userinfo,
            // and one sent as the `resource` parameter is a token scoped to
            // something the server never published.
            "https://user:pass@example.com/mcp",
        ] {
            assert!(PluginKind::custom("a", pretender).is_err(), "{pretender}");
        }
    }

    #[test]
    fn an_address_is_canonical_because_it_is_also_the_resource_it_signs_in_for() {
        // The same string is the URL a POST goes to and the RFC 8707 resource
        // indicator the grant is scoped to. A server publishing itself without
        // the trailing slash refuses a token issued for the version with one.
        // Scheme and host folded down, path left alone: `/MCP` and `/mcp` are
        // two endpoints, and a server publishing one refuses a token issued for
        // the other exactly as it refuses one issued for `Example.com`.
        let kind = PluginKind::custom("a", "HTTPS://Example.com/MCP/").unwrap();
        assert_eq!(kind.endpoint(), "https://example.com/MCP");
        assert_eq!(kind.stored_endpoint(), kind.endpoint());
    }

    #[test]
    fn a_catalog_row_keeps_its_address_in_the_build_and_a_custom_one_in_the_row() {
        // A vendor that moves ships as a new Guaca. A stored copy of a catalog
        // endpoint would keep a crew dialling the old host until somebody
        // reconnected it, which is how migration 26 came to exist.
        assert_eq!(PluginKind::Neon.stored_endpoint(), "");
        assert_eq!(
            PluginKind::from_row("neon", "https://elsewhere.test/mcp"),
            Some(PluginKind::Neon)
        );

        // And a row that carries its own address needs no build knowledge.
        let mine = PluginKind::from_row("obsidian", "https://vault.test/mcp").unwrap();
        assert_eq!(mine, PluginKind::custom("obsidian", "https://vault.test/mcp").unwrap());
        assert!(mine.is_custom());
        assert!(!PluginKind::Neon.is_custom());
    }

    #[test]
    fn a_row_this_build_cannot_dial_is_not_a_plugin() {
        // A slug the catalog does not have and no address beside it, which is
        // what a newer build's plugin looks like after a downgrade. Skipping it
        // is what keeps one crew's unreadable row from failing every turn.
        assert_eq!(PluginKind::from_row("github", ""), None);
        // And a slug nothing could call, whatever address is beside it.
        assert_eq!(PluginKind::from_row("Bad Name", "https://x.test/mcp"), None);
        assert_eq!(PluginKind::from_row("a__b", "https://x.test/mcp"), None);
    }

    #[test]
    fn a_custom_server_is_never_paid_for_with_the_operator_s_account() {
        // An operator can point one at guaca.bot, and it signs in to it the
        // ordinary way. Lending the account's own credential to an address
        // somebody typed is not a decision a name should be able to make.
        let pretender = PluginKind::custom("google2", "https://guaca.bot/mcp").unwrap();
        assert!(!pretender.account_backed());
        assert!(PluginKind::Google.account_backed());
    }
}
