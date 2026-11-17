use crate::account::AccountError;
use crate::db::store::{PluginReach, Store, StoreError};
use crate::domain::ids::{AgentId, GroupId, PluginId};
use crate::domain::now_ms;
use crate::domain::plugin::{Headers, Plugin, PluginKind, PluginTool, ServerReport, SigninNeed};
use crate::mcp::{self, McpError};
use crate::oauth::{self, Grant, OauthError};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Signin(#[from] OauthError),
    #[error(transparent)]
    Server(#[from] McpError),
    #[error("{says}")]
    Unreachable { says: String, detail: String },
    #[error(
        "{label} is not connected for this group. Ask the operator to connect it in the group's \
         Connectors settings; nothing you can do from here will connect it."
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
    #[error("{label}'s account sign-in is no longer accepted. Ask the operator to sign in again in Settings, Account.")]
    AccountExpired { label: String },
    #[error(
        "{label} is connected for this group, but not for you: the operator chose which agents \
         may use it. Ask a peer who has it to do that part, or ask the operator to add you in \
         the group's Connectors settings. Nothing you can do from here will add you."
    )]
    NotChosen { label: String },
    #[error(
        "{label}'s `{tool}` is switched off for this group: the operator decides which of a \
         connector's tools the group may call, and this one is off for everybody. Do not ask a peer, \
         because no peer has it. Use another of {label}'s tools if one will do, or tell the \
         operator which tool you need and that it is switched off in the group's Connectors \
         settings."
    )]
    ToolDenied { label: String, tool: String },
    #[error(
        "{label} is yours, but its `{tool}` is not: the operator chose which agents may call that \
         one. A peer has it — your roster says who — so hand that part over. Use another of \
         {label}'s tools if one will do, or ask the operator to add you to `{tool}` in the \
         group's Connectors settings. Nothing you can do from here will add you."
    )]
    ToolNotChosen { label: String, tool: String },
    #[error(
        "{label}'s sign-in is no longer accepted, and renewing it did not work. Ask the operator \
         to connect it again in the group's Connectors settings."
    )]
    SigninExpired { label: String },
    #[error("{label} could not confirm whether its sign-in was renewed. Ask the operator to check the connection before trying again.")]
    RenewalUnknown { label: String },
    #[error("{label} could not be reached before the tool was called. Try again later.")]
    Unavailable { label: String },
    #[error("{label} did not confirm the tool's outcome. The action may have happened, so check its result before doing it again.")]
    OutcomeUnknown { label: String },
}

impl PluginError {
    pub fn for_agent(&self, label: &str) -> String {
        match self {
            Self::AccountRefused { .. } => format!(
                "{label} could not confirm its account sign-in. Ask the operator to check it before trying again."
            ),
            Self::Server(McpError::Unauthorized { .. }) => format!(
                "{label}'s sign-in is no longer accepted. Ask the operator to reconnect it."
            ),
            Self::Server(McpError::Status { status: 403, .. }) => format!(
                "{label} refused access. Ask the operator to check the provider's access settings."
            ),
            Self::Server(_) | Self::Signin(_) => format!(
                "{label} did not confirm the tool's outcome. The action may have happened, so check its result before doing it again."
            ),
            _ => self.to_string(),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Credential<'a> {
    Discover,
    Key(&'a str),
    Account(AccountUse<'a>),
}

#[allow(clippy::too_many_arguments)]
pub async fn connect(
    store: &Store,
    group: GroupId,
    kind: &PluginKind,
    endpoint: &str,
    credential: Credential<'_>,
    headers: &Headers,
    landing: &oauth::Landing,
    open: impl FnOnce(&str) -> Result<(), String>,
) -> Result<Plugin, PluginError> {
    if kind.account_backed() {
        let Credential::Account(used) = credential else {
            return Err(PluginError::NoAccount { label: kind.label().to_string() });
        };
        let (_session, tools) =
            ask(dial(kind.is_custom(), endpoint, headers).with_token(Some(used.token))).await?;
        return Ok(store.save_plugin(group, kind, "", &tools, None, used.connection, headers)?);
    }

    let dialed = dial(kind.is_custom(), endpoint, headers);
    let (session, tools, grant) = match credential {
        Credential::Key(key) => {
            let (session, tools) = ask(dialed.with_token(Some(key))).await?;
            (session, tools, Some(Grant::key(key)))
        }
        _ => match ask(dialed).await {
            Ok((session, tools)) => (session, tools, None),
            Err(err) if err.is_unauthorized() => {
                let challenge = match &err {
                    McpError::Unauthorized { challenge, .. } => challenge.clone(),
                    _ => None,
                };
                let grant = oauth::authorize(
                    endpoint,
                    challenge.as_deref(),
                    &oauth::Gate::on(endpoint, headers.wire()),
                    landing,
                    open,
                    now_ms,
                )
                .await?;
                let (session, tools) =
                    ask(dialed.with_token(Some(grant.access_token.expose()))).await?;
                (session, tools, Some(grant))
            }
            Err(err) => return Err(err.into()),
        },
    };

    let account_label = account_label(&session, kind).await;

    Ok(store.save_plugin(group, kind, &account_label, &tools, grant.as_ref(), "", headers)?)
}

pub fn for_operator(err: McpError) -> PluginError {
    match err {
        McpError::Transport { endpoint, source } => {
            let says = format!(
                "Could not reach {endpoint}. {}. Check the address and its path, and that the \
                 server is running and reachable from here.",
                unreachable_cause(&source)
            );
            PluginError::Unreachable { says, detail: chain(&source) }
        }
        other => PluginError::Server(other),
    }
}

pub fn said_to_operator(err: PluginError) -> PluginError {
    match err {
        PluginError::Server(mcp) => for_operator(mcp),
        other => other,
    }
}

fn unreachable_cause(err: &reqwest::Error) -> &'static str {
    if err.is_timeout() {
        "Nothing answered in time"
    } else if err.is_connect() {
        "No connection was made: the name may not resolve, the port may be closed, or the server \
         may have refused the encrypted connection"
    } else if err.is_body() || err.is_decode() {
        "It answered, and the answer could not be read"
    } else {
        "The request failed before an answer came back"
    }
}

fn chain(err: &dyn std::error::Error) -> String {
    let mut said = err.to_string();
    let mut at = err.source();
    while let Some(next) = at {
        said.push_str(": ");
        said.push_str(&next.to_string());
        at = next.source();
    }
    said
}

fn dial<'a>(custom: bool, endpoint: &'a str, headers: &'a Headers) -> mcp::Dial<'a> {
    mcp::Dial { endpoint, token: None, headers: headers.wire(), legacy_transport: custom }
}

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
            transport: if session.stdio() {
                "stdio, a program on this host".to_string()
            } else if session.sse() {
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
        Err(err) if err.is_unauthorized() => Ok(ServerReport {
            endpoint: endpoint.to_string(),
            transport: String::new(),
            protocol: String::new(),
            handshake: false,
            signin: if token.is_some() { SigninNeed::Refused } else { SigninNeed::Wanted },
            server: String::new(),
            tools: Vec::new(),
            ms: started.elapsed().as_millis() as u64,
        }),
        Err(err) => Err(for_operator(err)),
    }
}

pub async fn check(
    store: &Store,
    id: PluginId,
    dialed: crate::db::store::Dialed,
    endpoint: &str,
    account: Held<'_>,
) -> Result<ServerReport, PluginError> {
    let crate::db::store::Dialed { kind, grant, generation, headers, .. } = dialed;
    if kind.account_backed() {
        let used = account.spend(&kind)?;
        return inspect(kind.is_custom(), endpoint, Some(used.token), &headers).await;
    }
    let grant = match grant {
        Some(held) if held.stale(now_ms()) => {
            Some(renew(store, id, generation, &kind, endpoint, &headers).await?.0)
        }
        other => other,
    };
    inspect(kind.is_custom(), endpoint, grant.as_ref().map(|g| g.access_token.expose()), &headers)
        .await
}

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
            input_schema: tool
                .input_schema
                .unwrap_or_else(|| serde_json::json!({ "type": "object", "properties": {} })),
        })
        .collect())
}

async fn account_label(session: &mcp::Session, kind: &PluginKind) -> String {
    let name = mcp::describe(session).await.unwrap_or_default();
    if name.eq_ignore_ascii_case(kind.label()) {
        String::new()
    } else {
        name
    }
}

#[derive(Clone, Copy)]
pub struct AccountUse<'a> {
    pub token: &'a str,
    pub connection: &'a str,
}

#[derive(Clone)]
pub enum Held<'a> {
    Absent,
    Token(AccountUse<'a>),
    Invalid,
    Refused(String),
}

impl std::fmt::Debug for AccountUse<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountUse").field("connection", &self.connection).finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Held<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Held::Absent => f.write_str("Absent"),
            Held::Token(used) => f.debug_tuple("Token").field(used).finish(),
            Held::Invalid => f.write_str("Invalid"),
            Held::Refused(said) => f.debug_tuple("Refused").field(said).finish(),
        }
    }
}

impl<'a> Held<'a> {
    pub fn read(read: &'a Result<String, AccountError>, connection: &'a str) -> Self {
        match read {
            Ok(token) => Held::Token(AccountUse { token, connection }),
            Err(AccountError::NotSignedIn) => Held::Absent,
            Err(AccountError::Upstream { status: 400, message, .. })
                if serde_json::from_str::<serde_json::Value>(message)
                    .ok()
                    .and_then(|value| value.get("error")?.as_str().map(str::to_owned))
                    .as_deref()
                    == Some("invalid_grant") =>
            {
                Held::Invalid
            }
            Err(err) => Held::Refused(err.to_string()),
        }
    }

    pub fn spend(self, kind: &PluginKind) -> Result<AccountUse<'a>, PluginError> {
        match self {
            Held::Token(used) => Ok(used),
            Held::Absent => Err(PluginError::NoAccount { label: kind.label().to_string() }),
            Held::Invalid => Err(PluginError::AccountExpired { label: kind.label().to_string() }),
            Held::Refused(detail) => {
                Err(PluginError::AccountRefused { label: kind.label().to_string(), detail })
            }
        }
    }
}

impl AccountUse<'_> {
    pub fn endpoint(origin: &str, connection: &str) -> String {
        let origin = origin.trim_end_matches('/');
        if connection.is_empty() {
            format!("{origin}/mcp")
        } else {
            format!("{origin}/mcp/{connection}")
        }
    }
}

pub struct Target<'a> {
    pub group: GroupId,
    pub agent: AgentId,
    pub kind: &'a PluginKind,
    pub endpoint: &'a str,
    pub account: Held<'a>,
}

pub async fn call(
    store: &Store,
    target: Target<'_>,
    tool: &str,
    arguments: &serde_json::Value,
) -> Result<mcp::Called, PluginError> {
    let Target { group, agent, kind, endpoint, account } = target;
    let label = || kind.label().to_string();
    let (id, grant, mut generation, headers, schema) =
        match store.plugin_reach(group, agent, kind, tool)? {
            PluginReach::Granted(reached) => {
                (reached.id, reached.grant, reached.generation, reached.headers, reached.schema)
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

    if kind.account_backed() {
        let used = account.spend(kind)?;
        return run(
            dial(kind.is_custom(), endpoint, &headers),
            Some(used.token),
            tool,
            arguments,
            schema.as_ref(),
            &label(),
        )
        .await;
    }

    let grant = match grant {
        Some(held) if held.stale(now_ms()) => {
            let (renewed, next) = renew(store, id, generation, kind, endpoint, &headers).await?;
            generation = next;
            Some(renewed)
        }
        other => other,
    };

    let dialed = dial(kind.is_custom(), endpoint, &headers);
    let attempt = run(
        dialed,
        grant.as_ref().map(|g| g.access_token.expose()),
        tool,
        arguments,
        schema.as_ref(),
        &label(),
    )
    .await;
    match attempt {
        Err(PluginError::Server(McpError::Unauthorized { .. })) => {
            let Some(_held) = grant else {
                return Err(PluginError::SigninExpired { label: label() });
            };
            let (renewed, _) = renew(store, id, generation, kind, endpoint, &headers).await?;
            match run(
                dialed,
                Some(renewed.access_token.expose()),
                tool,
                arguments,
                schema.as_ref(),
                &label(),
            )
            .await
            {
                Err(PluginError::Server(McpError::Unauthorized { .. })) => {
                    Err(PluginError::SigninExpired { label: label() })
                }
                other => other,
            }
        }
        other => other,
    }
}

async fn run(
    dial: mcp::Dial<'_>,
    token: Option<&str>,
    tool: &str,
    arguments: &serde_json::Value,
    schema: Option<&serde_json::Value>,
    label: &str,
) -> Result<mcp::Called, PluginError> {
    let session = mcp::open(dial.with_token(token)).await.map_err(|err| match err {
        McpError::Transport { .. }
        | McpError::Silent { .. }
        | McpError::Status { status: 429 | 500..=599, .. } => {
            PluginError::Unavailable { label: label.to_string() }
        }
        other => PluginError::Server(other),
    })?;
    mcp::call_tool(&session, tool, arguments, schema).await.map_err(|err| match err {
        McpError::Transport { .. }
        | McpError::Silent { .. }
        | McpError::Status { status: 429 | 500..=599, .. }
        | McpError::Malformed { .. } => PluginError::OutcomeUnknown { label: label.to_string() },
        other => PluginError::Server(other),
    })
}

async fn renew(
    store: &Store,
    id: PluginId,
    generation: i64,
    kind: &PluginKind,
    endpoint: &str,
    headers: &Headers,
) -> Result<(Grant, i64), PluginError> {
    static LOCKS: OnceLock<Mutex<HashMap<PluginId, Weak<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    let lock = {
        let mut locks = LOCKS.get_or_init(Default::default).lock().unwrap();
        locks.retain(|_, lock| lock.strong_count() > 0);
        match locks.get(&id).and_then(Weak::upgrade) {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(id, Arc::downgrade(&lock));
                lock
            }
        }
    };
    let _guard = lock.lock().await;
    let expired = || PluginError::SigninExpired { label: kind.label().to_string() };
    let current = store.plugin_dial(id)?.ok_or_else(expired)?;
    if current.kind != *kind || current.headers != *headers {
        return Err(expired());
    }
    let grant = current.grant.ok_or_else(expired)?;
    if current.generation != generation {
        return Ok((grant, current.generation));
    }
    let renewed = oauth::refresh(&grant, &oauth::Gate::on(endpoint, headers.wire()), now_ms)
        .await
        .map_err(|error| match error {
            OauthError::NoRefreshToken => expired(),
            OauthError::Status { status: 400, ref body, .. }
                if serde_json::from_str::<serde_json::Value>(body)
                    .ok()
                    .and_then(|value| value.get("error")?.as_str().map(str::to_owned))
                    .as_deref()
                    == Some("invalid_grant") =>
            {
                expired()
            }
            _ => PluginError::RenewalUnknown { label: kind.label().to_string() },
        })?;
    if !store.refresh_plugin_grant(id, generation, &renewed)? {
        return Err(expired());
    }
    Ok((renewed, generation + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_errors_never_include_provider_response_bodies() {
        let failure = PluginError::Server(McpError::Status {
            endpoint: "https://example.invalid/private-value".into(),
            status: 500,
            body: "private-value".into(),
        });
        let said = failure.for_agent("Neon");
        assert!(!said.contains("private-value"));
        assert!(said.contains("may have happened"));
    }

    #[test]
    fn an_account_invalid_grant_requires_a_new_signin() {
        let error: Result<String, AccountError> = Err(AccountError::Upstream {
            origin: "https://example.invalid".into(),
            status: 400,
            message: r#"{"error":"invalid_grant","detail":"private-value"}"#.into(),
        });
        let said = Held::read(&error, "").spend(&PluginKind::Google).unwrap_err().to_string();
        assert!(said.contains("sign in again"), "{said}");
        assert!(!said.contains("private-value"), "{said}");
        assert!(!said.contains("try again later"), "{said}");
    }

    #[test]
    fn an_unnamed_connection_stays_on_the_bare_path() {
        assert_eq!(AccountUse::endpoint("https://guaca.bot", ""), "https://guaca.bot/mcp");
        assert_eq!(AccountUse::endpoint("https://guaca.bot/", ""), "https://guaca.bot/mcp");
    }

    #[test]
    fn a_named_connection_is_part_of_the_address() {
        assert_eq!(
            AccountUse::endpoint("https://guaca.bot", "acct_1"),
            "https://guaca.bot/mcp/acct_1"
        );
        assert_eq!(
            AccountUse::endpoint("http://localhost:8787", "acct_1"),
            "http://localhost:8787/mcp/acct_1"
        );
    }

    async fn refused_dial() -> PluginError {
        let failed = reqwest::Client::new()
            .get("http://127.0.0.1:9/mcp")
            .send()
            .await
            .expect_err("a closed loopback port cannot answer");
        for_operator(McpError::Transport {
            endpoint: "http://127.0.0.1:9/mcp".into(),
            source: failed,
        })
    }

    #[tokio::test]
    async fn a_server_that_does_not_answer_is_said_in_words_a_person_can_act_on() {
        let refused = refused_dial().await;
        let says = refused.to_string();
        assert!(!says.contains("error sending request for url"), "{says}");
        assert!(says.contains("http://127.0.0.1:9/mcp"), "{says}");
        assert!(says.contains("Check the address"), "{says}");
        assert!(says.contains(". No connection was made"), "{says}");
        assert!(says.starts_with("Could not reach"), "{says}");
        assert!(says.ends_with('.'), "{says}");

        match refused {
            PluginError::Unreachable { detail, .. } => {
                assert!(detail.contains("error sending request for url"), "{detail}");
                assert!(detail.contains("Connection refused"), "{detail}");
            }
            other => panic!("a transport failure should be said to the operator: {other:?}"),
        }
    }

    #[test]
    fn a_plugin_that_is_not_connected_says_who_can_connect_it() {
        let refusal = PluginError::NotConnected { label: "Neon".into() }.to_string();
        assert!(refusal.contains("Neon"));
        assert!(refusal.contains("operator"));
        assert!(refusal.contains("nothing you can do"));
    }

    #[test]
    fn a_plugin_this_agent_was_not_chosen_for_does_not_read_as_a_missing_sign_in() {
        let refusal = PluginError::NotChosen { label: "Stripe".into() }.to_string();
        assert!(refusal.contains("Stripe"));
        assert!(refusal.contains("not for you"), "{refusal}");
        assert!(refusal.contains("peer"), "the way forward is delegation: {refusal}");
        assert!(!refusal.contains("is not connected"), "{refusal}");
    }

    #[test]
    fn a_switched_off_tool_does_not_send_the_agent_round_the_crew() {
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
                assert_eq!(used.connection, "acct_1");
            }
            other => panic!("a token is a token: {other:?}"),
        }
    }

    #[test]
    fn a_held_token_debugs_without_the_token() {
        let read = Ok("acct-hunter2".to_string());
        let printed = format!("{:?}", Held::read(&read, "acct_1"));
        assert!(printed.contains("acct_1"), "{printed}");
        assert!(!printed.contains("hunter2"), "a token reached Debug: {printed}");
    }

    #[test]
    fn the_two_ways_to_have_no_credential_do_not_share_a_sentence() {
        let absent = Held::Absent.spend(&PluginKind::Google).unwrap_err().to_string();
        let refused = Held::Refused("https://guaca.bot answered HTTP 500: nope".into())
            .spend(&PluginKind::Google)
            .unwrap_err()
            .to_string();

        assert!(absent.contains("not signed in to one"), "{absent}");
        assert!(absent.contains("Settings"), "{absent}");
        assert!(refused.contains("HTTP 500"), "what the service said: {refused}");
        assert!(refused.contains("try it again later"), "the way forward: {refused}");
        assert!(!refused.contains("Settings"), "nothing in Settings fixes this: {refused}");
        assert!(!refused.contains("not signed in to one"), "{refused}");
    }
}
