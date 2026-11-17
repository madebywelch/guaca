use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::cdp::{CdpError, Dialog, Page};
use crate::domain::signin::BrowserState;

const API_BASE: &str = "https://api.onkernel.com";

fn api_base() -> String {
    match std::env::var("GUAC_KERNEL_API_BASE") {
        Ok(base) if !base.trim().is_empty() => base.trim().trim_end_matches('/').to_string(),
        _ => API_BASE.to_string(),
    }
}

const LIVE_VIEW_HOSTS: [&str; 2] = ["kernel.sh", "onkernel.com"];

const LIVE_VIEW_PORT: u16 = 8443;

const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

const MIN_TIMEOUT_SECONDS: u32 = 10;
const MAX_TIMEOUT_SECONDS: u32 = 259_200;

const SETTLE_NAVIGATE_MS: u32 = 2500;
const SETTLE_CLICK_MS: u32 = 1200;
const SETTLE_SCROLL_MS: u32 = 600;

const TAG: &str = "guac";

const AGENT_TAG: &str = "guac-agent";
const WORKSPACE_TAG: &str = "guac-workspace";

#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    #[error("no Kernel API key is set; add one in app settings to give agents a browser")]
    NoKey,
    #[error("this agent has not been given a browser; the operator gives one from its panel")]
    NotGiven,
    #[error("Kernel request failed: {0}")]
    Transport(String),
    #[error("Kernel rejected the request ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("Kernel replied in a form this build does not understand: {0}")]
    Protocol(String),
    #[error("Kernel replied in a form this build does not understand: {0}")]
    Input(String),
    #[error("Guaca could not use its own records: {0}")]
    Local(String),
    #[error("The browser provider did not confirm the action's outcome. It may have happened, so check before doing it again.")]
    OutcomeUnknown,
    #[error(transparent)]
    Cdp(#[from] CdpError),
}

fn uncertain_creation(err: KernelError) -> KernelError {
    match err {
        KernelError::Transport(_)
        | KernelError::Protocol(_)
        | KernelError::Api { status: 429 | 500..=599, .. } => KernelError::OutcomeUnknown,
        other => other,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Browser {
    pub session_id: String,
    pub state: String,
    #[serde(flatten)]
    pub view: LiveView,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "LiveViewFields", from = "LiveViewFields")]
pub enum LiveView {
    Framed(String),
    Refused(String),
    Absent,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveViewFields {
    live_view_url: Option<String>,
    unwatchable: Option<String>,
}

impl From<LiveView> for LiveViewFields {
    fn from(view: LiveView) -> Self {
        let (live_view_url, unwatchable) = match view {
            LiveView::Framed(url) => (Some(url), None),
            LiveView::Refused(origin) => (None, Some(origin)),
            LiveView::Absent => (None, None),
        };
        LiveViewFields { live_view_url, unwatchable }
    }
}

impl From<LiveViewFields> for LiveView {
    fn from(fields: LiveViewFields) -> Self {
        match (fields.live_view_url, fields.unwatchable) {
            (Some(_), Some(origin)) => {
                tracing::warn!(
                    origin = %origin,
                    "a browser was read with both a live view and an unwatchable origin; \
                     it is shown as unwatchable"
                );
                LiveView::Refused(origin)
            }
            (Some(url), None) => LiveView::Framed(url),
            (None, Some(origin)) => LiveView::Refused(origin),
            (None, None) => LiveView::Absent,
        }
    }
}

impl Browser {
    pub fn running(session: Session) -> Self {
        let view = match session.live_view_url {
            Some(url) if !framable(&url) => {
                let origin = origin_of(&url);
                tracing::warn!(
                    %origin,
                    session = %session.id,
                    "Kernel served a live view this window is not allowed to frame; \
                     the browser is running and only watching it is refused"
                );
                LiveView::Refused(origin)
            }
            Some(url) => LiveView::Framed(url),
            None => LiveView::Absent,
        };
        Browser { session_id: session.id, state: "running".to_string(), view }
    }
}

fn framable(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default().to_ascii_lowercase();
    let Some((host, port)) = authority.rsplit_once(':') else {
        return false;
    };
    if port.parse::<u16>() != Ok(LIVE_VIEW_PORT) {
        return false;
    }
    LIVE_VIEW_HOSTS.iter().any(|allowed| host.ends_with(&format!(".{allowed}")))
}

fn origin_of(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    if scheme.is_empty() {
        authority.to_string()
    } else {
        format!("{scheme}://{authority}")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub id: String,
    pub cdp_ws_url: String,
    pub live_view_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BrowserRow {
    session_id: String,
    #[serde(default)]
    tags: std::collections::HashMap<String, String>,
    #[serde(default)]
    cdp_ws_url: String,
    #[serde(default)]
    browser_live_view_url: Option<String>,
}

impl From<BrowserRow> for Session {
    fn from(row: BrowserRow) -> Self {
        Session {
            id: row.session_id,
            cdp_ws_url: row.cdp_ws_url,
            live_view_url: row.browser_live_view_url.filter(|url| !url.trim().is_empty()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProfileRow {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug)]
pub enum ProfileCreation {
    Created { id: String, name: String },
    Existing { name: String },
}

#[derive(Debug)]
pub enum BrowserCreation {
    Created(Session),
    Conflict { candidates: Vec<BrowserCandidate> },
}

#[derive(Debug)]
pub struct BrowserCandidate {
    pub id: String,
    pub workspace: Option<String>,
}

#[derive(Debug, Clone)]
pub struct KernelClient {
    http: reqwest::Client,
    api_key: crate::secrets::Secret,
    base: String,
}

impl KernelClient {
    pub fn new(api_key: &crate::secrets::Secret) -> Option<Self> {
        let api_key = api_key.expose().trim();
        if api_key.is_empty() {
            return None;
        }
        let http = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build().ok()?;
        Some(Self { http, api_key: api_key.into(), base: api_base() })
    }

    async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, KernelError> {
        let response = request
            .bearer_auth(self.api_key.expose())
            .send()
            .await
            .map_err(|e| KernelError::Transport(e.to_string()))?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            let message = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v["message"].as_str().map(str::to_string))
                .unwrap_or_else(|| body.chars().take(200).collect());
            return Err(KernelError::Api { status: status.as_u16(), message });
        }
        if body.trim().is_empty() {
            return serde_json::from_str("null")
                .map_err(|e| KernelError::Protocol(format!("empty reply: {e}")));
        }
        serde_json::from_str(&body)
            .map_err(|e| KernelError::Protocol(format!("could not read Kernel's reply: {e}")))
    }

    pub async fn ensure_profile(&self, agent: &str) -> Result<String, KernelError> {
        let name = profile_name(agent);
        match self
            .call::<ProfileRow>(
                self.http.post(format!("{}/profiles", self.base)).json(&json!({ "name": name })),
            )
            .await
        {
            Ok(row) => Ok(row.name.unwrap_or(name)),
            Err(KernelError::Api { status: 409, .. }) => Ok(name),
            Err(err) => Err(err),
        }
    }

    pub async fn create_profile(&self, agent: &str) -> Result<ProfileCreation, KernelError> {
        let name = profile_name(agent);
        match self
            .call::<ProfileRow>(
                self.http.post(format!("{}/profiles", self.base)).json(&json!({ "name": name })),
            )
            .await
        {
            Ok(row) => {
                let name = row.name.unwrap_or(name);
                Ok(ProfileCreation::Created {
                    id: row.id.filter(|id| !id.is_empty()).unwrap_or_else(|| name.clone()),
                    name,
                })
            }
            Err(KernelError::Api { status: 409, .. }) => Ok(ProfileCreation::Existing { name }),
            Err(err) => Err(uncertain_creation(err)),
        }
    }

    pub async fn profile_id(&self, name: &str) -> Result<Option<String>, KernelError> {
        match self.call::<ProfileRow>(self.http.get(format!("{}/profiles/{name}", self.base))).await
        {
            Ok(row) => Ok(row.id.filter(|id| !id.is_empty())),
            Err(KernelError::Api { status: 404, .. }) => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub async fn create_for_workspace(
        &self,
        agent: &str,
        profile: &str,
        idle_seconds: u32,
        stealth: bool,
        workspace: &str,
    ) -> Result<BrowserCreation, KernelError> {
        if workspace.trim().is_empty() {
            return Err(KernelError::Local("workspace identity is empty".into()));
        }
        let mut body = create_body(agent, profile, idle_seconds, stealth);
        body["tags"][WORKSPACE_TAG] = json!(workspace);
        match self
            .call::<BrowserRow>(self.http.post(format!("{}/browsers", self.base)).json(&body))
            .await
        {
            Ok(row) => Ok(BrowserCreation::Created(row.into())),
            Err(KernelError::Api { status: 409, .. }) => {
                Ok(BrowserCreation::Conflict { candidates: self.list_candidates(agent).await? })
            }
            Err(err) => Err(uncertain_creation(err)),
        }
    }

    async fn list_candidates(&self, agent: &str) -> Result<Vec<BrowserCandidate>, KernelError> {
        let mut result = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut offset = 0;
        loop {
            let rows: Vec<BrowserRow> = self
                .call(self.http.get(format!("{}/browsers", self.base)).query(&[
                    ("status", "active".to_string()),
                    ("tags[guac]", "true".to_string()),
                    ("tags[guac-agent]", agent.to_string()),
                    ("limit", "100".to_string()),
                    ("offset", offset.to_string()),
                ]))
                .await?;
            let count = rows.len();
            for row in rows {
                if !seen.insert(row.session_id.clone()) {
                    return Err(KernelError::Protocol(
                        "Kernel repeated a browser in its inventory".into(),
                    ));
                }
                if row.tags.get(TAG).map(String::as_str) == Some("true")
                    && row.tags.get(AGENT_TAG).map(String::as_str) == Some(agent)
                {
                    result.push(BrowserCandidate {
                        id: row.session_id,
                        workspace: row.tags.get(WORKSPACE_TAG).cloned(),
                    });
                }
            }
            if count < 100 {
                return Ok(result);
            }
            offset += count;
        }
    }

    pub async fn list_workspace(
        &self,
        workspace: &str,
        agent: Option<&str>,
    ) -> Result<Vec<String>, KernelError> {
        if workspace.trim().is_empty() {
            return Err(KernelError::Local("workspace identity is empty".into()));
        }
        let mut result = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut offset = 0;
        loop {
            let mut query = vec![
                ("status".to_string(), "active".to_string()),
                (format!("tags[{TAG}]"), "true".to_string()),
                (format!("tags[{WORKSPACE_TAG}]"), workspace.to_string()),
                ("limit".to_string(), "100".to_string()),
                ("offset".to_string(), offset.to_string()),
            ];
            if let Some(agent) = agent {
                query.push((format!("tags[{AGENT_TAG}]"), agent.to_string()));
            }
            let rows: Vec<BrowserRow> =
                self.call(self.http.get(format!("{}/browsers", self.base)).query(&query)).await?;
            let count = rows.len();
            for row in rows {
                if !seen.insert(row.session_id.clone()) {
                    return Err(KernelError::Protocol(
                        "Kernel repeated a browser in its inventory".into(),
                    ));
                }
                if row.tags.get(TAG).map(String::as_str) == Some("true")
                    && row.tags.get(WORKSPACE_TAG).map(String::as_str) == Some(workspace)
                    && agent.is_none_or(|agent| {
                        row.tags.get(AGENT_TAG).map(String::as_str) == Some(agent)
                    })
                {
                    result.push(row.session_id);
                }
            }
            if count < 100 {
                return Ok(result);
            }
            offset += count;
        }
    }

    pub async fn get_for_workspace(
        &self,
        id: &str,
        workspace: &str,
    ) -> Result<Option<Session>, KernelError> {
        if workspace.trim().is_empty() {
            return Err(KernelError::Local("workspace identity is empty".into()));
        }
        match self.call::<BrowserRow>(self.http.get(format!("{}/browsers/{id}", self.base))).await {
            Ok(row)
                if row.tags.get(TAG).map(String::as_str) == Some("true")
                    && row.tags.get(WORKSPACE_TAG).map(String::as_str) == Some(workspace) =>
            {
                Ok(Some(row.into()))
            }
            Ok(_) => Err(KernelError::Protocol(
                "browser ownership tag does not match this workspace".into(),
            )),
            Err(KernelError::Api { status: 404, .. }) => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub async fn create(
        &self,
        agent: &str,
        idle_seconds: u32,
        stealth: bool,
    ) -> Result<Session, KernelError> {
        let profile = self.ensure_profile(agent).await?;
        let made = self
            .call::<BrowserRow>(
                self.http.post(format!("{}/browsers", self.base)).json(&create_body(
                    agent,
                    &profile,
                    idle_seconds,
                    stealth,
                )),
            )
            .await;

        match made {
            Ok(row) => Ok(row.into()),
            Err(KernelError::Api { status: 409, message }) => match self.held_by(agent).await? {
                Some(live) => Ok(live),
                None => Err(KernelError::Api { status: 409, message }),
            },
            Err(err) => Err(err),
        }
    }

    async fn held_by(&self, agent: &str) -> Result<Option<Session>, KernelError> {
        let rows: Vec<BrowserRow> = self
            .call(self.http.get(format!("{}/browsers", self.base)).query(&[
                ("status", "active"),
                (&format!("tags[{TAG}]"), "true"),
                (&format!("tags[{AGENT_TAG}]"), agent),
                ("limit", "10"),
            ]))
            .await?;
        match rows.first() {
            Some(row) => self.get(&row.session_id).await,
            None => Ok(None),
        }
    }

    pub async fn get(&self, id: &str) -> Result<Option<Session>, KernelError> {
        match self.call::<BrowserRow>(self.http.get(format!("{}/browsers/{id}", self.base))).await {
            Ok(row) => Ok(Some(row.into())),
            Err(KernelError::Api { status: 404, .. }) => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub async fn delete(&self, id: &str) -> Result<(), KernelError> {
        match self.call::<Value>(self.http.delete(format!("{}/browsers/{id}", self.base))).await {
            Ok(_) => Ok(()),
            Err(KernelError::Api { status: 404, .. }) => Ok(()),
            Err(err) => Err(err),
        }
    }

    pub async fn delete_profile(&self, agent: &str) -> Result<(), KernelError> {
        self.delete_profile_named(&profile_name(agent)).await
    }

    pub async fn delete_profile_named(&self, name: &str) -> Result<(), KernelError> {
        match self.call::<Value>(self.http.delete(format!("{}/profiles/{name}", self.base))).await {
            Ok(_) => Ok(()),
            Err(KernelError::Api { status: 404, .. }) => Ok(()),
            Err(err) => Err(err),
        }
    }

    pub async fn list_ours(&self) -> Result<Vec<String>, KernelError> {
        let rows: Vec<BrowserRow> = self
            .call(self.http.get(format!("{}/browsers", self.base)).query(&[
                ("status", "active"),
                (&format!("tags[{TAG}]"), "true"),
                ("limit", "100"),
            ]))
            .await?;
        Ok(rows.into_iter().map(|row| row.session_id).collect())
    }

    pub async fn browse(
        &self,
        session: &Session,
        action: &str,
        args: &Value,
    ) -> Result<String, KernelError> {
        let mut page = Page::attach(&session.cdp_ws_url).await?;

        let settle_ms = match action {
            "open" => {
                let url = args["url"].as_str().unwrap_or_default().trim().to_string();
                if url.is_empty() {
                    return Err(KernelError::Input(
                        "open needs a `url`. Give the address you want, for example \
                         `https://example.com`."
                            .into(),
                    ));
                }
                let url = if url.starts_with("http://") || url.starts_with("https://") {
                    url
                } else {
                    format!("https://{url}")
                };
                page.navigate(&url).await?;
                SETTLE_NAVIGATE_MS
            }

            "read" => 0,

            "click" => {
                let target = element(args)?;
                require(&mut page, target).await?;
                page.evaluate(&format!(
                    "window.__guacEls[{target}].scrollIntoView({{block:'center'}})"
                ))
                .await?;
                page.evaluate(&format!("window.__guacEls[{target}].click()")).await?;
                SETTLE_CLICK_MS
            }

            "type" => {
                let target = element(args)?;
                let text = args["text"].as_str().unwrap_or_default();
                require(&mut page, target).await?;
                page.evaluate(&format!(
                    "(() => {{ const e = window.__guacEls[{target}];
                       e.scrollIntoView({{block:'center'}}); e.focus();
                       if (e.select) {{ e.select(); }}
                       else {{ const r = document.createRange(); r.selectNodeContents(e);
                               const s = getSelection(); s.removeAllRanges(); s.addRange(r); }}
                       return 1; }})()"
                ))
                .await?;
                page.insert_text(text).await?;
                if args["submit"].as_bool().unwrap_or(false) {
                    page.press_enter().await?;
                    SETTLE_NAVIGATE_MS
                } else {
                    SETTLE_SCROLL_MS
                }
            }

            "scroll" => {
                let amount = args["amount"].as_i64().unwrap_or(3).clamp(1, 20) * 400;
                let amount =
                    if args["direction"].as_str() == Some("up") { -amount } else { amount };
                page.evaluate(&format!("scrollBy(0, {amount})")).await?;
                SETTLE_SCROLL_MS
            }

            "back" => {
                page.evaluate("history.back()").await?;
                SETTLE_NAVIGATE_MS
            }

            other => {
                return Err(KernelError::Input(format!(
                    "`{other}` is not something a browser does. Use open, read, click, type, \
                     scroll or back."
                )))
            }
        };

        let (collected, answered) = match settle_and_collect(&mut page, settle_ms).await {
            Err(CdpError::TargetGone) => {
                let mut replacement = Page::attach(&session.cdp_ws_url).await?;
                let collected = settle_and_collect(&mut replacement, settle_ms).await?;
                let mut answered = page.answered().to_vec();
                answered.extend_from_slice(replacement.answered());
                (collected, answered)
            }
            other => (other?, page.answered().to_vec()),
        };
        let described = collected
            .as_str()
            .ok_or_else(|| KernelError::Protocol("the page did not describe itself".into()))?;
        Ok(note_dialogs(described, &answered))
    }

    pub async fn signed_in_state(&self, session: &Session) -> Result<BrowserState, KernelError> {
        let mut page = Page::attach(&session.cdp_ws_url).await?;
        Ok(BrowserState { cookies: page.cookies().await?, visited: page.visited().await? })
    }
}

async fn settle_and_collect(page: &mut Page, settle_ms: u32) -> Result<Value, CdpError> {
    if settle_ms > 0 {
        page.settle(settle_ms).await?;
    }
    page.evaluate(COLLECT).await
}

fn note_dialogs(described: &str, answered: &[Dialog]) -> String {
    if answered.is_empty() {
        return described.to_string();
    }
    let Ok(mut page) = serde_json::from_str::<Value>(described) else {
        return described.to_string();
    };
    page["dialogs"] = json!(answered);
    page.to_string()
}

fn element(args: &Value) -> Result<i64, KernelError> {
    args["id"].as_i64().or_else(|| args["id"].as_str()?.parse().ok()).ok_or_else(|| {
        KernelError::Input(
            "that needs an `id`: the number `read` gave the element you mean. Read the page again \
             if you do not have one."
                .into(),
        )
    })
}

async fn require(page: &mut Page, index: i64) -> Result<(), KernelError> {
    let held = page
        .evaluate(&format!(
            "Boolean(window.__guacEls && window.__guacEls[{index}] \
             && document.contains(window.__guacEls[{index}]))"
        ))
        .await?;
    if held.as_bool() == Some(true) {
        return Ok(());
    }
    Err(KernelError::Input(format!(
        "element {index} is not on the page any more, because the page changed since you read it. \
         Read it again: the numbers are handed out fresh each time."
    )))
}

fn profile_name(agent: &str) -> String {
    format!("guac-{agent}")
}

fn create_body(agent: &str, profile: &str, idle_seconds: u32, stealth: bool) -> Value {
    json!({
        "headless": false,
        "profile": { "name": profile, "save_changes": true },
        "timeout_seconds": idle_seconds.clamp(MIN_TIMEOUT_SECONDS, MAX_TIMEOUT_SECONDS),
        "stealth": stealth,
        "tags": { TAG: "true", AGENT_TAG: agent },
        "name": format!("guac-{agent}"),
    })
}

const COLLECT: &str = r#"
(() => {
  const sel = 'a,button,input,select,textarea,summary,[role=button],[role=link],'
            + '[role=tab],[role=checkbox],[role=menuitem],[contenteditable=true],[onclick]';
  const out = [];
  window.__guacEls = [];
  for (const el of document.querySelectorAll(sel)) {
    const r = el.getBoundingClientRect();
    if (r.width < 2 || r.height < 2) continue;
    if (r.bottom < 0 || r.top > innerHeight || r.right < 0 || r.left > innerWidth) continue;
    const style = getComputedStyle(el);
    if (style.visibility === 'hidden' || style.display === 'none' || style.opacity === '0') continue;

    const label = (el.innerText || el.value || el.placeholder || el.getAttribute('aria-label')
                   || el.getAttribute('title') || el.alt || '').trim().replace(/\s+/g, ' ');
    const id = window.__guacEls.length;
    window.__guacEls.push(el);
    out.push({
      id,
      tag: el.tagName.toLowerCase(),
      type: el.getAttribute('type') || '',
      text: label.slice(0, 80),
      x: Math.round(r.x + r.width / 2),
      y: Math.round(r.y + r.height / 2),
    });
    if (out.length >= 120) break;
  }
  const body = (document.body ? document.body.innerText : '').replace(/\n{3,}/g, '\n\n');
  return JSON.stringify({
    url: location.href,
    title: document.title,
    scroll: Math.round(scrollY),
    height: Math.round(document.body ? document.body.scrollHeight : 0),
    text: body.slice(0, 6000),
    elements: out,
  });
})()
"#;

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use futures_util::{SinkExt, StreamExt};
    use parking_lot::Mutex;
    use tokio_tungstenite::tungstenite::Message;

    use super::*;

    #[test]
    fn a_creation_failure_does_not_claim_the_resource_was_not_created() {
        for err in [
            KernelError::Transport("private-value".into()),
            KernelError::Protocol("private-value".into()),
            KernelError::Api { status: 503, message: "private-value".into() },
        ] {
            let error = uncertain_creation(err);
            assert!(matches!(error, KernelError::OutcomeUnknown));
            assert!(!error.to_string().contains("private-value"));
        }
        assert!(matches!(
            uncertain_creation(KernelError::Api { status: 403, message: String::new() }),
            KernelError::Api { status: 403, .. }
        ));
    }

    #[tokio::test]
    async fn browser_lookup_and_creation_report_the_same_server_failure_differently() {
        let app = axum::Router::new()
            .route(
                "/browsers/:id",
                axum::routing::get(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE }),
            )
            .route(
                "/browsers",
                axum::routing::post(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut client = KernelClient::new(&"test".into()).unwrap();
        client.base = format!("http://{address}");
        assert!(matches!(client.get("browser").await, Err(KernelError::Api { status: 503, .. })));
        assert!(matches!(
            client.create_for_workspace("agent", "profile", 60, false, "workspace").await,
            Err(KernelError::OutcomeUnknown)
        ));
    }

    #[tokio::test]
    async fn an_empty_workspace_identity_is_a_local_error() {
        let client = KernelClient::new(&"test".into()).unwrap();
        let error =
            client.create_for_workspace("agent", "profile", 60, false, "").await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "Guaca could not use its own records: workspace identity is empty"
        );
    }

    #[test]
    fn a_browser_provider_reply_error_keeps_its_provider_prefix() {
        assert_eq!(
            KernelError::Protocol("Kernel repeated a browser in its inventory".into()).to_string(),
            "Kernel replied in a form this build does not understand: Kernel repeated a browser in its inventory"
        );
    }

    fn live_view_origin(scheme: &str, host: &str) -> String {
        format!("{scheme}://*.{host}:{LIVE_VIEW_PORT}")
    }

    #[test]
    fn the_window_is_allowed_to_frame_and_reach_every_host_a_live_view_arrives_on() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        let csp = conf["app"]["security"]["csp"].as_str().unwrap_or_default();
        let directive = |name: &str| {
            csp.split(';')
                .find(|part| part.trim().starts_with(name))
                .unwrap_or_default()
                .to_string()
        };

        let frame_src = directive("frame-src");
        let connect_src = directive("connect-src");
        for host in LIVE_VIEW_HOSTS {
            let https = live_view_origin("https", host);
            assert!(frame_src.contains(&https), "got {frame_src:?}");
            assert!(connect_src.contains(&https), "got {connect_src:?}");
            assert!(
                connect_src.contains(&live_view_origin("wss", host)),
                "the pixels arrive over a WebSocket: {connect_src:?}"
            );
        }
    }

    #[test]
    fn a_live_view_the_window_cannot_frame_is_named_rather_than_drawn() {
        for host in LIVE_VIEW_HOSTS {
            assert!(
                framable(&format!("https://prod-jfk-1.{host}:8443/browser/live/tok")),
                "{host}"
            );
        }

        assert!(!framable("https://prod-jfk-1.example.com:8443/browser/live/tok"));
        assert!(
            !framable("https://prod-jfk-1.kernel.sh/browser/live/tok"),
            "the port is the origin"
        );
        assert!(!framable("http://prod-jfk-1.kernel.sh:8443/browser/live/tok"), "TLS is too");
        assert!(!framable("https://kernel.sh:8443/browser/live/tok"));
        assert!(!framable("https://kernel.sh.example.com:8443/live"), "a suffix, not a substring");

        let refused = Browser::running(Session {
            id: "s".into(),
            cdp_ws_url: "wss://x".into(),
            live_view_url: Some("https://prod-jfk-1.example.com:8443/browser/live/tok".into()),
        });
        assert_eq!(refused.view, LiveView::Refused("https://prod-jfk-1.example.com:8443".into()));
        assert_eq!(refused.state, "running", "the browser is fine; only watching it is refused");

        let shown = Browser::running(Session {
            id: "s".into(),
            cdp_ws_url: "wss://x".into(),
            live_view_url: Some("https://prod-jfk-1.kernel.sh:8443/browser/live/tok".into()),
        });
        assert_eq!(
            shown.view,
            LiveView::Framed("https://prod-jfk-1.kernel.sh:8443/browser/live/tok".into())
        );
    }

    #[derive(Clone, Default)]
    struct Logs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Logs {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl std::io::Write for Logs {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logs {
        type Writer = Logs;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn a_browser_read_as_both_framed_and_refused_is_read_as_unwatchable_with_one_warning() {
        let both = json!({
            "sessionId": "s",
            "state": "running",
            "liveViewUrl": "https://prod-jfk-1.kernel.sh:8443/browser/live/tok",
            "unwatchable": "https://prod-jfk-1.example.com:8443",
        });
        let logs = Logs::default();
        let read = tracing::subscriber::with_default(
            tracing_subscriber::fmt().with_writer(logs.clone()).with_ansi(false).finish(),
            || serde_json::from_value::<Browser>(both),
        )
        .expect("a contradictory reply still yields a browser");
        assert_eq!(read.view, LiveView::Refused("https://prod-jfk-1.example.com:8443".into()));
        let logged = logs.text();
        assert_eq!(logged.matches("WARN").count(), 1, "{logged}");
        assert!(logged.contains("both a live view and an unwatchable origin"), "{logged}");
        assert!(
            !logged.contains("/browser/live/tok"),
            "the live view address carries a token: {logged}"
        );

        for (url, unwatchable) in [
            (json!("https://prod-jfk-1.kernel.sh:8443/browser/live/tok"), Value::Null),
            (Value::Null, json!("https://prod-jfk-1.example.com:8443")),
            (Value::Null, Value::Null),
        ] {
            let wire = json!({
                "sessionId": "s",
                "state": "running",
                "liveViewUrl": url,
                "unwatchable": unwatchable,
            });
            let browser: Browser = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(serde_json::to_value(&browser).unwrap(), wire);
        }
    }

    #[test]
    fn an_origin_names_the_address_and_nothing_that_was_in_front_of_it() {
        assert_eq!(
            origin_of("https://user:secret@prod-jfk-1.kernel.sh:8443/browser/live/tok?jwt=abc"),
            "https://prod-jfk-1.kernel.sh:8443"
        );
        assert_eq!(origin_of("https://host:8443"), "https://host:8443");
    }

    #[test]
    fn a_live_view_is_never_routed_through_the_computer_viewer() {
        for host in LIVE_VIEW_HOSTS {
            let origin = live_view_origin("https", host);
            assert!(!origin.contains(crate::e2b::VIEWER_HOST), "{origin}");
        }
    }

    #[test]
    fn a_browser_is_created_watchable_and_on_the_agents_own_profile() {
        let body = create_body("agent-1", "guac-agent-1", 3600, false);

        assert_eq!(body["headless"], json!(false), "{body}");
        assert_eq!(body["profile"]["save_changes"], json!(true), "{body}");
        assert_eq!(body["profile"]["name"], json!("guac-agent-1"), "{body}");
        assert_eq!(body["timeout_seconds"], json!(3600), "{body}");
        assert_eq!(body["tags"]["guac"], json!("true"), "{body}");
        assert_eq!(body["tags"]["guac-agent"], json!("agent-1"), "{body}");
    }

    #[tokio::test]
    async fn workspace_creation_distinguishes_conflicts_and_filters_candidates() {
        use axum::{extract::State, http::StatusCode, routing::post, Json, Router};

        #[derive(Default)]
        struct Requests {
            profiles: AtomicUsize,
            browsers: AtomicUsize,
            bodies: Mutex<Vec<Value>>,
        }
        async fn profile(State(state): State<Arc<Requests>>) -> (StatusCode, Json<Value>) {
            match state.profiles.fetch_add(1, Ordering::SeqCst) {
                0 => (StatusCode::CREATED, Json(json!({"id":"profile-id", "name":"guac-agent-1"}))),
                1 => (StatusCode::CONFLICT, Json(json!({"message":"exists"}))),
                _ => (StatusCode::CREATED, Json(json!({"name":"guac-agent-1"}))),
            }
        }
        async fn create(
            State(state): State<Arc<Requests>>,
            Json(body): Json<Value>,
        ) -> (StatusCode, Json<Value>) {
            state.bodies.lock().push(body);
            if state.browsers.fetch_add(1, Ordering::SeqCst) == 0 {
                (
                    StatusCode::CREATED,
                    Json(json!({"session_id":"created", "cdp_ws_url":"ws://created"})),
                )
            } else {
                (StatusCode::CONFLICT, Json(json!({"message":"exists"})))
            }
        }
        async fn list() -> Json<Value> {
            Json(json!([
                {"session_id":"ours", "tags":{"guac":"true", "guac-agent":"agent-1", "guac-workspace":"workspace-1"}},
                {"session_id":"foreign", "tags":{"guac":"true", "guac-agent":"agent-1", "guac-workspace":"workspace-2"}},
                {"session_id":"legacy", "tags":{"guac":"true", "guac-agent":"agent-1"}}
            ]))
        }
        let state = Arc::new(Requests::default());
        let app = Router::new()
            .route("/profiles", post(profile))
            .route("/browsers", post(create).get(list))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = KernelClient { http: reqwest::Client::new(), api_key: "test".into(), base };
        assert!(
            matches!(client.create_profile("agent-1").await.unwrap(), ProfileCreation::Created { id, name } if id == "profile-id" && name == "guac-agent-1")
        );
        assert!(
            matches!(client.create_profile("agent-1").await.unwrap(), ProfileCreation::Existing { name } if name == "guac-agent-1")
        );
        assert!(
            matches!(client.create_profile("agent-1").await.unwrap(), ProfileCreation::Created { id, name } if id == name && name == "guac-agent-1")
        );
        assert!(
            matches!(client.create_for_workspace("agent-1", "guac-agent-1", 3600, false, "workspace-1").await.unwrap(), BrowserCreation::Created(session) if session.id == "created")
        );
        let BrowserCreation::Conflict { candidates } = client
            .create_for_workspace("agent-1", "guac-agent-1", 3600, false, "workspace-1")
            .await
            .unwrap()
        else {
            panic!("expected conflict")
        };
        assert_eq!(
            candidates.iter().map(|c| (c.id.as_str(), c.workspace.as_deref())).collect::<Vec<_>>(),
            [("ours", Some("workspace-1")), ("foreign", Some("workspace-2")), ("legacy", None)]
        );
        assert_eq!(client.list_workspace("workspace-1", Some("agent-1")).await.unwrap(), ["ours"]);
        assert_eq!(state.bodies.lock()[0]["tags"]["guac-workspace"], json!("workspace-1"));
        server.abort();
    }

    #[test]
    fn an_out_of_range_timeout_is_clamped_rather_than_rejected() {
        assert_eq!(create_body("a", "p", 0, false)["timeout_seconds"], json!(MIN_TIMEOUT_SECONDS));
        assert_eq!(
            create_body("a", "p", u32::MAX, false)["timeout_seconds"],
            json!(MAX_TIMEOUT_SECONDS)
        );
    }

    #[test]
    fn stealth_is_the_operators_choice_and_travels_as_asked() {
        assert_eq!(create_body("a", "p", 60, true)["stealth"], json!(true));
        assert_eq!(create_body("a", "p", 60, false)["stealth"], json!(false));
    }

    #[test]
    fn one_agent_always_finds_the_same_profile() {
        assert_eq!(profile_name("abc"), profile_name("abc"));
        assert_ne!(profile_name("abc"), profile_name("abd"));
        assert!(profile_name("abc").starts_with("guac-"));
    }

    #[test]
    fn an_element_number_is_read_from_either_shape_a_model_sends() {
        assert_eq!(element(&json!({ "id": 4 })).unwrap(), 4);
        assert_eq!(element(&json!({ "id": "4" })).unwrap(), 4);
        let refusal = element(&json!({})).unwrap_err().to_string();
        assert!(refusal.contains("read"), "{refusal}");
    }

    #[test]
    fn the_collector_hands_back_the_shape_the_transcript_renders() {
        for key in ["url", "title", "scroll", "height", "text", "elements"] {
            assert!(COLLECT.contains(&format!("{key}:")), "the page must report {key}");
        }
        assert!(COLLECT.contains("window.__guacEls"), "{COLLECT}");
    }

    #[test]
    fn a_key_that_is_blank_means_not_configured_rather_than_always_failing() {
        assert!(KernelClient::new(&"".into()).is_none());
        assert!(KernelClient::new(&"   ".into()).is_none());
        assert!(KernelClient::new(&"sk-live".into()).is_some());
    }

    #[test]
    fn a_missing_live_view_is_absent_rather_than_empty() {
        let row = BrowserRow {
            session_id: "s".into(),
            tags: Default::default(),
            cdp_ws_url: "wss://x".into(),
            browser_live_view_url: Some("  ".into()),
        };
        assert_eq!(Session::from(row).live_view_url, None);
    }

    const FAKE_PAGE: &str = r#"{"url":"https://example.com/after","title":"After","scroll":0,
        "height":2000,"text":"arrived","elements":[]}"#;

    struct FakeBrowser {
        url: String,
        evaluated: Arc<Mutex<Vec<String>>>,
        connections: Arc<AtomicUsize>,
        answers: Arc<Mutex<Vec<bool>>>,
    }

    impl FakeBrowser {
        async fn start_losing(lose_first: usize) -> Self {
            Self::start(lose_first, None).await
        }

        async fn start_asking(kind: &'static str) -> Self {
            Self::start(0, Some(kind)).await
        }

        async fn start(lose_first: usize, asks: Option<&'static str>) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a port");
            let address = listener.local_addr().expect("an address");
            let evaluated: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let connections = Arc::new(AtomicUsize::new(0));
            let answers: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::new()));

            let (recorded, counted, collected) =
                (evaluated.clone(), connections.clone(), answers.clone());
            tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else { return };
                    let lost = counted.fetch_add(1, Ordering::SeqCst) < lose_first;
                    let recorded = recorded.clone();
                    let collected = collected.clone();
                    tokio::spawn(async move {
                        let mut socket =
                            tokio_tungstenite::accept_async(stream).await.expect("a handshake");
                        let mut held: Option<Value> = None;
                        let mut enabled = false;
                        while let Some(Ok(frame)) = socket.next().await {
                            let Message::Text(text) = frame else { continue };
                            let call: Value = serde_json::from_str(&text).expect("a call");
                            let id = call["id"].clone();
                            let expression =
                                call["params"]["expression"].as_str().unwrap_or_default();
                            let reply = match call["method"].as_str().unwrap_or_default() {
                                "Target.getTargets" => json!({"id": id, "result":
                                    {"targetInfos": [{"type": "page", "targetId": "T"}]}}),
                                "Target.attachToTarget" => {
                                    json!({"id": id, "result": {"sessionId": "S"}})
                                }
                                "Page.enable" => {
                                    enabled = true;
                                    json!({"id": id, "result": {}})
                                }
                                "Page.navigate" => match asks.filter(|_| enabled) {
                                    Some(kind) => {
                                        let opening = json!({
                                            "method": "Page.javascriptDialogOpening",
                                            "sessionId": "S",
                                            "params": {
                                                "type": kind,
                                                "message": "Changes you made may not be saved.",
                                                "url": "https://example.com/before",
                                            },
                                        });
                                        socket
                                            .send(Message::text(opening.to_string()))
                                            .await
                                            .expect("the event");
                                        held = Some(id);
                                        continue;
                                    }
                                    None => json!({"id": id, "result": {}}),
                                },
                                "Page.handleJavaScriptDialog" => {
                                    collected
                                        .lock()
                                        .push(call["params"]["accept"].as_bool().expect("accept"));
                                    socket
                                        .send(Message::text(
                                            json!({"id": id, "result": {}}).to_string(),
                                        ))
                                        .await
                                        .expect("a reply");
                                    match held.take() {
                                        Some(waiting) => json!({"id": waiting, "result": {}}),
                                        None => continue,
                                    }
                                }
                                "Runtime.evaluate" => {
                                    recorded.lock().push(expression.to_string());
                                    let after = expression.contains("setTimeout")
                                        || expression.contains("location.href");
                                    if lost && after {
                                        json!({"id": id, "error":
                                            {"message": "Inspected target navigated or closed"}})
                                    } else if expression.contains("location.href") {
                                        json!({"id": id, "result": {"result": {"value": FAKE_PAGE}}})
                                    } else if expression.contains("Boolean(") {
                                        json!({"id": id, "result": {"result": {"value": true}}})
                                    } else {
                                        json!({"id": id, "result": {"result": {"value": 1}}})
                                    }
                                }
                                other => panic!("the fake browser was asked for {other}"),
                            };
                            socket.send(Message::text(reply.to_string())).await.expect("a reply");
                        }
                    });
                }
            });

            FakeBrowser { url: format!("ws://{address}"), evaluated, connections, answers }
        }

        fn session(&self) -> Session {
            Session { id: "s".into(), cdp_ws_url: self.url.clone(), live_view_url: None }
        }

        fn connections(&self) -> usize {
            self.connections.load(Ordering::SeqCst)
        }
    }

    #[tokio::test]
    async fn a_page_that_navigates_underneath_an_action_is_read_again_not_reported_as_broken() {
        let browser = FakeBrowser::start_losing(1).await;
        let client = KernelClient::new(&"sk-test".into()).expect("a client");

        let page = client
            .browse(&browser.session(), "open", &json!({ "url": "https://example.com" }))
            .await
            .expect("the page loaded, so the action succeeded");

        assert!(page.contains("https://example.com/after"), "{page}");
        assert_eq!(browser.connections(), 2);
    }

    #[tokio::test]
    async fn a_click_that_loses_the_page_is_never_sent_a_second_time() {
        let browser = FakeBrowser::start_losing(1).await;
        let client = KernelClient::new(&"sk-test".into()).expect("a client");

        let page = client
            .browse(&browser.session(), "click", &json!({ "id": 3 }))
            .await
            .expect("the click went through");

        assert!(page.contains("After"), "{page}");
        let clicks =
            browser.evaluated.lock().iter().filter(|seen| seen.contains(".click()")).count();
        assert_eq!(clicks, 1, "the click was sent again after the page moved");
    }

    #[tokio::test]
    async fn an_action_on_a_page_that_stays_put_opens_one_connection() {
        let browser = FakeBrowser::start_losing(0).await;
        let client = KernelClient::new(&"sk-test".into()).expect("a client");

        client.browse(&browser.session(), "read", &json!({})).await.expect("read the page");

        assert_eq!(browser.connections(), 1, "a working page must not pay for a reconnection");
    }

    #[tokio::test]
    async fn a_leave_site_box_is_answered_so_the_action_it_stopped_can_finish() {
        let browser = FakeBrowser::start_asking("beforeunload").await;
        let client = KernelClient::new(&"sk-test".into()).expect("a client");

        let page = client
            .browse(&browser.session(), "open", &json!({ "url": "https://example.com" }))
            .await
            .expect("the navigation was released, so the page arrived");

        assert_eq!(*browser.answers.lock(), vec![true], "the agent asked to leave");
        assert!(page.contains("https://example.com/after"), "{page}");
        assert!(page.contains("beforeunload"), "{page}");
        assert!(page.contains("Changes you made may not be saved."), "{page}");
    }

    #[tokio::test]
    async fn a_prompt_is_declined_rather_than_answered_with_nothing() {
        let browser = FakeBrowser::start_asking("prompt").await;
        let client = KernelClient::new(&"sk-test".into()).expect("a client");

        let page = client
            .browse(&browser.session(), "open", &json!({ "url": "https://example.com" }))
            .await
            .expect("the page still arrives");

        assert_eq!(*browser.answers.lock(), vec![false]);
        assert!(page.contains("prompt"), "{page}");
    }

    #[test]
    fn a_page_with_no_dialog_is_handed_back_exactly_as_the_browser_described_it() {
        assert_eq!(note_dialogs(FAKE_PAGE, &[]), FAKE_PAGE);
    }

    #[test]
    fn a_description_that_will_not_parse_keeps_the_page_and_loses_the_note() {
        let answered = [Dialog { kind: "alert".into(), message: "hi".into(), accepted: true }];
        assert_eq!(note_dialogs("<html>garbage", &answered), "<html>garbage");
    }

    #[tokio::test]
    async fn a_page_that_will_not_stay_still_says_what_to_do_about_it() {
        let browser = FakeBrowser::start_losing(usize::MAX).await;
        let client = KernelClient::new(&"sk-test".into()).expect("a client");

        let err = client
            .browse(&browser.session(), "open", &json!({ "url": "https://example.com" }))
            .await
            .expect_err("nothing could be read");

        assert!(err.to_string().contains("Read the page again"), "{err}");
        assert_eq!(browser.connections(), 2, "one retry, not a loop");
    }
}
