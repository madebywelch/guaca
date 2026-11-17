use serde::{Deserialize, Serialize};

use super::ids::{AgentId, GroupId, PluginId};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PluginKind {
    Neon,
    Cloudflare,
    Linear,
    Stripe,
    Agentmail,
    Supermemory,
    Firecrawl,
    Google,
    Custom { slug: String, endpoint: String },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CustomError {
    #[error(
        "a server needs a name. One word, letters and digits, which is what its tools will be \
         called by."
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
        "a server run on the host needs its command: the program and its arguments, such as \
         `npx -y @modelcontextprotocol/server-github`."
    )]
    NoCommand,
    #[error(
        "`{0}` has a quote that is never closed. Close it, or drop it: the command is split into \
         words the way a shell would, and no shell ever sees it."
    )]
    BadCommand(String),
    #[error(
        "{0} is not a URL Guaca can dial. It needs a scheme and a host, such as \
         https://example.com/mcp."
    )]
    BadUrl(String),
    #[error(
        "{0} is not https. A group's sign-in and everything its agents send would cross the \
         network in the open. Use https, or a loopback address if the server is on this machine."
    )]
    Insecure(String),
    #[error(
        "{0} has a fragment on it. An MCP endpoint is the address a token is issued for, and a \
         fragment is never sent to a server: drop everything from the # onward."
    )]
    Fragment(String),
}

pub const MAX_CUSTOM_NAME: usize = 32;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers {
    pairs: Vec<(String, String)>,
    env: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeaderPair {
    pub name: String,
    pub value: String,
}

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
        "{0} is not an environment variable name. Use letters, digits and underscores, starting \
         with a letter or an underscore, such as GITHUB_TOKEN."
    )]
    BadEnvName(String),
    #[error(
        "that is more than {MAX_ENV} variables. A program needing more than that is one being \
         configured through Guaca rather than run by it: point it at a config file instead."
    )]
    TooManyEnv,
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

pub const MAX_HEADERS: usize = 8;

pub const MAX_HEADER_VALUE: usize = 4096;

pub const MAX_ENV: usize = 16;

pub const STDIO: &str = "stdio:";

pub fn canonical_address(typed: &str) -> Result<String, CustomError> {
    let Some(command) = typed.trim().strip_prefix(STDIO) else {
        return canonical_url(typed);
    };
    let endpoint = format!("{STDIO}{}", command.trim());
    if command.trim().is_empty() {
        return Err(CustomError::NoCommand);
    }
    if command_line(&endpoint).is_none() {
        return Err(CustomError::BadCommand(command.trim().to_string()));
    }
    Ok(endpoint)
}

pub fn command_line(endpoint: &str) -> Option<Vec<String>> {
    let rest = endpoint.strip_prefix(STDIO)?;
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(open), c) if c == open => quote = None,
            (Some('"'), '\\') | (None, '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
                started = true;
            }
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (None, c) => {
                word.push(c);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if started {
        words.push(word);
    }
    (!words.is_empty()).then_some(words)
}

const RESERVED: [&str; 5] = ["accept", "content-type", "content-length", "host", "connection"];

impl Headers {
    pub fn none() -> Headers {
        Headers::default()
    }

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
            if !value.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
                return Err(HeaderError::BadValue(name));
            }
            if out.iter().any(|(held, _)| held == &name) {
                return Err(HeaderError::Repeated(name));
            }
            out.push((name, value));
        }
        Ok(Headers { pairs: out, env: false })
    }

    pub fn parse_env(rows: &[HeaderPair]) -> Result<Headers, HeaderError> {
        if rows.len() > MAX_ENV {
            return Err(HeaderError::TooManyEnv);
        }
        let mut out: Vec<(String, String)> = Vec::new();
        for row in rows {
            let name = row.name.trim().to_string();
            let value = row.value.trim().to_string();
            if name.is_empty() {
                return Err(HeaderError::NoName);
            }
            let valid = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
            if !valid {
                return Err(HeaderError::BadEnvName(name));
            }
            if value.is_empty() {
                return Err(HeaderError::NoValue(name));
            }
            if value.len() > MAX_HEADER_VALUE {
                return Err(HeaderError::LongValue(name));
            }
            if value.chars().any(char::is_control) {
                return Err(HeaderError::BadValue(name));
            }
            if out.iter().any(|(held, _)| held == &name) {
                return Err(HeaderError::Repeated(name));
            }
            out.push((name, value));
        }
        Ok(Headers { pairs: out, env: true })
    }

    pub fn is_env(&self) -> bool {
        self.env
    }

    pub fn encode(&self) -> String {
        let rows: Vec<HeaderPair> = self
            .pairs
            .iter()
            .map(|(name, value)| HeaderPair { name: name.clone(), value: value.clone() })
            .collect();
        let value =
            if self.env { serde_json::json!({ "env": rows }) } else { serde_json::json!(rows) };
        serde_json::to_string(&value).unwrap_or_else(|_| "[]".to_string())
    }

    pub fn decode(stored: &str) -> Headers {
        #[derive(Deserialize)]
        struct Env {
            env: Vec<HeaderPair>,
        }
        if let Ok(Env { env }) = serde_json::from_str::<Env>(stored) {
            return Headers::parse_env(&env).unwrap_or_default();
        }
        serde_json::from_str::<Vec<HeaderPair>>(stored)
            .ok()
            .and_then(|rows| Headers::parse(&rows).ok())
            .unwrap_or_default()
    }

    pub fn names(&self) -> Vec<String> {
        self.pairs.iter().map(|(name, _)| name.clone()).collect()
    }

    pub fn carries_authorization(&self) -> bool {
        !self.env && self.pairs.iter().any(|(name, _)| name == "authorization")
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    pub fn wire(&self) -> &[(String, String)] {
        &self.pairs
    }
}

fn is_token(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

impl PluginKind {
    pub const ALL: [PluginKind; 8] = [
        PluginKind::Neon,
        PluginKind::Cloudflare,
        PluginKind::Linear,
        PluginKind::Stripe,
        PluginKind::Agentmail,
        PluginKind::Supermemory,
        PluginKind::Firecrawl,
        PluginKind::Google,
    ];

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
        Ok(PluginKind::Custom { slug, endpoint: canonical_address(url)? })
    }

    pub fn is_stdio(&self) -> bool {
        self.endpoint().starts_with(STDIO)
    }

    pub fn slug(&self) -> &str {
        match self {
            PluginKind::Neon => "neon",
            PluginKind::Cloudflare => "cloudflare",
            PluginKind::Linear => "linear",
            PluginKind::Stripe => "stripe",
            PluginKind::Agentmail => "agentmail",
            PluginKind::Supermemory => "supermemory",
            PluginKind::Firecrawl => "firecrawl",
            PluginKind::Google => "google",
            PluginKind::Custom { slug, .. } => slug,
        }
    }

    pub fn from_slug(slug: &str) -> Option<PluginKind> {
        PluginKind::ALL.into_iter().find(|kind| kind.slug() == slug)
    }

    pub fn from_row(slug: &str, endpoint: &str) -> Option<PluginKind> {
        if let Some(known) = PluginKind::from_slug(slug) {
            return Some(known);
        }
        if endpoint.is_empty() || normalize_name(slug) != slug {
            return None;
        }
        Some(PluginKind::Custom { slug: slug.to_string(), endpoint: endpoint.to_string() })
    }

    pub fn label(&self) -> &str {
        match self {
            PluginKind::Neon => "Neon",
            PluginKind::Cloudflare => "Cloudflare",
            PluginKind::Linear => "Linear",
            PluginKind::Stripe => "Stripe",
            PluginKind::Agentmail => "AgentMail",
            PluginKind::Supermemory => "Supermemory",
            PluginKind::Firecrawl => "Firecrawl",
            PluginKind::Google => "Google",
            PluginKind::Custom { slug, .. } => slug,
        }
    }

    pub fn is_custom(&self) -> bool {
        matches!(self, PluginKind::Custom { .. })
    }

    pub fn endpoint(&self) -> &str {
        match self {
            PluginKind::Neon => "https://mcp.neon.tech/mcp",
            PluginKind::Cloudflare => "https://mcp.cloudflare.com/mcp",
            PluginKind::Linear => "https://mcp.linear.app/mcp",
            PluginKind::Stripe => "https://mcp.stripe.com",
            PluginKind::Agentmail => "https://mcp.agentmail.to/mcp",
            PluginKind::Supermemory => "https://mcp.supermemory.ai/mcp",
            PluginKind::Firecrawl => "https://mcp.firecrawl.dev/v2/mcp-oauth",
            PluginKind::Google => "https://guaca.bot/mcp",
            PluginKind::Custom { endpoint, .. } => endpoint,
        }
    }

    pub fn stored_endpoint(&self) -> &str {
        match self {
            PluginKind::Custom { endpoint, .. } => endpoint,
            _ => "",
        }
    }

    pub fn account_backed(&self) -> bool {
        matches!(self, PluginKind::Google)
    }

    pub fn blurb(&self) -> &str {
        match self {
            PluginKind::Neon => "Postgres databases: branch one, run SQL, migrate it.",
            PluginKind::Cloudflare => "The whole API: Workers, DNS, R2, D1, Zero Trust.",
            PluginKind::Linear => "Issues and projects: find them, file them, move them on.",
            PluginKind::Stripe => "The live account: payments, customers, invoices, refunds.",
            PluginKind::Agentmail => "Inboxes an agent owns: read a thread, send, reply, forward.",
            PluginKind::Supermemory => "Memory your other assistants share: search, save, forget.",
            PluginKind::Firecrawl => "The live web on your credits: search, scrape, crawl, watch.",
            PluginKind::Google => {
                "Your Gmail, Calendar and Drive, through the Guaca account you signed in to."
            }
            PluginKind::Custom { .. } => {
                "A server you added. Whatever it publishes, this group has."
            }
        }
    }

    pub fn docs(&self) -> &str {
        match self {
            PluginKind::Neon => "https://neon.com/docs/ai/neon-mcp-server",
            PluginKind::Cloudflare => {
                "https://developers.cloudflare.com/agents/model-context-protocol/cloudflare/servers-for-cloudflare/"
            }
            PluginKind::Linear => "https://linear.app/docs/mcp",
            PluginKind::Stripe => "https://docs.stripe.com/mcp",
            PluginKind::Agentmail => "https://www.agentmail.to/docs/integrations/mcp",
            PluginKind::Supermemory => "https://supermemory.ai/docs/supermemory-mcp/mcp",
            PluginKind::Firecrawl => "https://docs.firecrawl.dev/mcp-server",
            PluginKind::Google => "https://guaca.bot/app",
            PluginKind::Custom { .. } => "https://modelcontextprotocol.io/specification",
        }
    }
}

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
    if authority.contains('@') {
        return Err(CustomError::BadUrl(url.to_string()));
    }
    let host = authority;
    let bare = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or_default().to_ascii_lowercase(),
        None => host.split(':').next().unwrap_or_default().to_ascii_lowercase(),
    };
    let loopback =
        bare == "localhost" || bare.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
    match scheme.as_str() {
        "https" => {}
        "http" if loopback => {}
        "http" => return Err(CustomError::Insecure(url.to_string())),
        _ => return Err(CustomError::BadUrl(url.to_string())),
    }
    let host = host.to_ascii_lowercase();
    Ok(format!("{scheme}://{host}{path}").trim_end_matches('/').to_string())
}

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginOffer {
    pub kind: PluginKind,
    pub name: String,
    pub blurb: String,
    pub docs: String,
    pub endpoint: String,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerReport {
    pub endpoint: String,
    pub transport: String,
    pub protocol: String,
    pub handshake: bool,
    pub signin: SigninNeed,
    pub server: String,
    pub tools: Vec<String>,
    pub ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SigninNeed {
    None,
    Wanted,
    Accepted,
    Refused,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginToolCard {
    pub name: String,
    pub description: String,
    pub access: PluginAccess,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginToolset {
    pub kind: PluginKind,
    pub offered: Vec<PluginTool>,
    pub withheld: Vec<String>,
    pub elsewhere: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PluginAccess {
    Everyone,
    Chosen { agents: Vec<AgentId> },
}

pub const ACCESS_EVERYONE: &str = "everyone";

pub const ACCESS_CHOSEN: &str = "chosen";

impl PluginAccess {
    pub fn allows(&self, agent: AgentId) -> bool {
        match self {
            PluginAccess::Everyone => true,
            PluginAccess::Chosen { agents } => agents.contains(&agent),
        }
    }

    pub fn allows_nobody(&self) -> bool {
        matches!(self, PluginAccess::Chosen { agents } if agents.is_empty())
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            PluginAccess::Everyone => ACCESS_EVERYONE,
            PluginAccess::Chosen { .. } => ACCESS_CHOSEN,
        }
    }

    pub fn from_row(access: &str, agents: Vec<AgentId>) -> Self {
        if access == ACCESS_EVERYONE {
            PluginAccess::Everyone
        } else {
            PluginAccess::Chosen { agents }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    pub id: PluginId,
    pub group_id: GroupId,
    pub kind: PluginKind,
    pub name: String,
    pub endpoint: String,
    pub custom: bool,
    pub account: String,
    pub tools: Vec<PluginToolCard>,
    pub access: PluginAccess,
    pub connection: String,
    pub headers: Vec<String>,
    pub signed_in: bool,
    pub connected_at: i64,
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_command_is_split_into_words_the_way_a_shell_would_and_never_run_by_one() {
        let split = |text: &str| command_line(&format!("{STDIO}{text}"));
        assert_eq!(
            split("npx -y @modelcontextprotocol/server-github").unwrap(),
            ["npx", "-y", "@modelcontextprotocol/server-github"]
        );
        assert_eq!(
            split(r#"uvx "my server" --flag='a b'"#).unwrap(),
            ["uvx", "my server", "--flag=a b"]
        );
        assert_eq!(split(r"run a\ b").unwrap(), ["run", "a b"]);
        assert_eq!(split("echo hi; rm -rf /").unwrap(), ["echo", "hi;", "rm", "-rf", "/"]);
        assert_eq!(split("   "), None);
        assert_eq!(split("unclosed 'quote"), None);
        assert_eq!(command_line("https://example.com/mcp"), None);
    }

    #[test]
    fn a_server_run_on_the_host_is_named_by_its_command() {
        let kind = PluginKind::custom("github", "  stdio:  npx -y server-github ").unwrap();
        assert_eq!(kind.endpoint(), "stdio:npx -y server-github");
        assert!(kind.is_stdio());
        assert!(!PluginKind::custom("web", "https://example.com/mcp").unwrap().is_stdio());
        assert_eq!(PluginKind::custom("x", "stdio:"), Err(CustomError::NoCommand));
        assert!(matches!(PluginKind::custom("x", "stdio:a 'b"), Err(CustomError::BadCommand(_))));
    }

    #[test]
    fn a_programs_environment_keeps_its_case_and_survives_the_column() {
        let rows = [
            HeaderPair { name: "GITHUB_TOKEN".into(), value: "ghp_x".into() },
            HeaderPair { name: "_private".into(), value: "1".into() },
        ];
        let env = Headers::parse_env(&rows).unwrap();
        assert!(env.is_env());
        assert_eq!(env.names(), ["GITHUB_TOKEN", "_private"]);
        let back = Headers::decode(&env.encode());
        assert_eq!(back, env, "an environment read back is still one, with its case");
        let header = [HeaderPair { name: "X-API-Key".into(), value: "k".into() }];
        let headers = Headers::decode(&Headers::parse(&header).unwrap().encode());
        assert_eq!(headers.names(), ["x-api-key"]);
        assert!(!headers.is_env());
        let auth = [HeaderPair { name: "authorization".into(), value: "x".into() }];
        assert!(!Headers::parse_env(&auth).unwrap().carries_authorization());
        for bad in ["1TOKEN", "MY-TOKEN", "A B"] {
            let row = [HeaderPair { name: bad.into(), value: "v".into() }];
            assert!(matches!(Headers::parse_env(&row), Err(HeaderError::BadEnvName(_))), "{bad}");
        }
        let newline = [HeaderPair { name: "T".into(), value: "a\nb".into() }];
        assert!(matches!(Headers::parse_env(&newline), Err(HeaderError::BadValue(_))));
    }
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
        assert_eq!(PluginKind::from_slug("clerk"), None);
    }

    #[test]
    fn every_endpoint_is_https_and_named_in_full() {
        for kind in PluginKind::ALL {
            assert!(kind.endpoint().starts_with("https://"), "{}", kind.label());
            assert!(kind.docs().starts_with("https://"), "{}", kind.label());
        }
    }

    #[test]
    fn cloudflare_is_the_account_wide_server_and_not_one_product_area() {
        let endpoint = PluginKind::Cloudflare.endpoint();
        assert_eq!(endpoint, "https://mcp.cloudflare.com/mcp");
        assert!(
            !endpoint.contains(".mcp.cloudflare.com"),
            "a subdomain here is one product area, not the account: {endpoint}"
        );
    }

    #[test]
    fn firecrawl_is_the_address_that_signs_in_and_not_the_keyless_one() {
        assert_eq!(PluginKind::Firecrawl.endpoint(), "https://mcp.firecrawl.dev/v2/mcp-oauth");
    }

    #[test]
    fn a_kind_round_trips_through_json_as_the_slug() {
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
        let headers = Headers::parse(&[header("X-API-Key", " abc ")]).unwrap();
        assert_eq!(headers.wire(), [("x-api-key".to_string(), "abc".to_string())]);
        assert_eq!(headers.names(), ["x-api-key"]);
    }

    #[test]
    fn two_spellings_of_one_header_are_a_duplicate_rather_than_two_headers() {
        let both = Headers::parse(&[header("X-Api-Key", "a"), header("x-api-key", "b")]);
        assert_eq!(both, Err(HeaderError::Repeated("x-api-key".into())));
    }

    #[test]
    fn a_header_this_client_builds_itself_is_refused() {
        use HeaderError::*;
        assert!(matches!(Headers::parse(&[header("mcp-method", "x")]), Err(Reserved(_))));
        assert!(matches!(Headers::parse(&[header("Mcp-Param-Region", "x")]), Err(Reserved(_))));
        assert!(matches!(Headers::parse(&[header("content-type", "x")]), Err(Reserved(_))));
        assert!(matches!(Headers::parse(&[header("Accept", "x")]), Err(Reserved(_))));
    }

    #[test]
    fn authorization_is_allowed_because_it_is_the_only_way_to_send_another_scheme() {
        let headers = Headers::parse(&[header("Authorization", "Basic dXNlcjpwYXNz")]).unwrap();
        assert!(headers.carries_authorization());
    }

    #[test]
    fn a_value_with_a_line_break_in_it_is_refused() {
        use HeaderError::*;
        assert!(matches!(Headers::parse(&[header("x-key", "a\r\nX-Admin: 1")]), Err(BadValue(_))));
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
        let mine = PluginKind::custom("Home Assistant", "https://ha.example.com/mcp").unwrap();
        assert_eq!(serde_json::to_string(&mine).unwrap(), "\"home_assistant\"");
        let back = serde_json::from_str::<PluginKind>("\"home_assistant\"");
        assert!(back.is_err(), "a custom name must not deserialize into a kind with no address");
    }

    #[test]
    fn a_name_becomes_the_word_its_tools_will_be_called_by() {
        for (typed, want) in [
            ("Home Assistant", "home_assistant"),
            ("home-assistant", "home_assistant"),
            ("  Obsidian  ", "obsidian"),
            ("my.server v2", "my_server_v2"),
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
        assert!(matches!(PluginKind::custom("2fast", url), Err(BadName(_))));
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
        assert!(matches!(PluginKind::custom("a", "https://e.com/mcp#x"), Err(Fragment(_))));
    }

    #[test]
    fn a_server_on_this_machine_may_be_plain_http_and_nothing_else_may() {
        for local in [
            "http://localhost:8080/mcp",
            "http://127.0.0.1:3000/mcp",
            "http://127.0.0.2/mcp",
            "http://[::1]/mcp",
            "http://[::1]:8080/mcp",
        ] {
            assert!(PluginKind::custom("local", local).is_ok(), "{local}");
        }
        for pretender in [
            "http://localhost.evil.com/mcp",
            "http://127.evil.com/mcp",
            "http://127.0.0.1.evil.com/mcp",
            "http://localhost:80@evil.com/mcp",
            "http://127.0.0.1@evil.com/mcp",
            "http://[::1]@evil.com/mcp",
            "https://user:pass@example.com/mcp",
        ] {
            assert!(PluginKind::custom("a", pretender).is_err(), "{pretender}");
        }
    }

    #[test]
    fn an_address_is_canonical_because_it_is_also_the_resource_it_signs_in_for() {
        let kind = PluginKind::custom("a", "HTTPS://Example.com/MCP/").unwrap();
        assert_eq!(kind.endpoint(), "https://example.com/MCP");
        assert_eq!(kind.stored_endpoint(), kind.endpoint());
    }

    #[test]
    fn a_catalog_row_keeps_its_address_in_the_build_and_a_custom_one_in_the_row() {
        assert_eq!(PluginKind::Neon.stored_endpoint(), "");
        assert_eq!(
            PluginKind::from_row("neon", "https://elsewhere.test/mcp"),
            Some(PluginKind::Neon)
        );

        let mine = PluginKind::from_row("obsidian", "https://vault.test/mcp").unwrap();
        assert_eq!(mine, PluginKind::custom("obsidian", "https://vault.test/mcp").unwrap());
        assert!(mine.is_custom());
        assert!(!PluginKind::Neon.is_custom());
    }

    #[test]
    fn a_row_this_build_cannot_dial_is_not_a_plugin() {
        assert_eq!(PluginKind::from_row("github", ""), None);
        assert_eq!(PluginKind::from_row("Bad Name", "https://x.test/mcp"), None);
        assert_eq!(PluginKind::from_row("a__b", "https://x.test/mcp"), None);
    }

    #[test]
    fn a_custom_server_is_never_paid_for_with_the_operator_s_account() {
        let pretender = PluginKind::custom("google2", "https://guaca.bot/mcp").unwrap();
        assert!(!pretender.account_backed());
        assert!(PluginKind::Google.account_backed());
    }
}
