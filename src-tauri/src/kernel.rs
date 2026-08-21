//! Kernel browsers: one browser per agent.
//!
//! A computer and a browser are different things an agent can be given, and
//! this is the second one. A computer (`e2b.rs`) is a Linux machine with a
//! screen: an agent works it by looking at pixels and aiming a pointer, which
//! is how a person uses a computer and is as approximate as that sounds. A
//! browser is a hosted Chrome and nothing else, driven over the DevTools
//! protocol, so every link, button and field is named rather than guessed at.
//!
//! The two are not ranked. A page is better asked than looked at, so anything
//! on the web belongs on the browser. Everything that is not a web page has no
//! DOM to ask and belongs on the computer: a shell, a file, a PDF viewer, an
//! installer, a desktop application. An agent may hold both, and they
//! share nothing: separate cookie jars, separate sign-ins, separate sessions
//! the operator has to establish once each.
//!
//! ## Why a provider whose whole product is one browser
//!
//! Chrome's remote interface used to be opened on the E2B machine, and it did
//! not survive contact with a general-purpose desktop: the port belongs to a
//! profile and was lost whenever anything re-attached to that profile, and two
//! ways to use the web on one screen disagreed about which window was in front.
//! Here the browser *is* the product. There is one, it is the only thing in the
//! sandbox, the socket is handed out at creation, and there is no second route
//! to the same window for anything to fall out of step with.
//!
//! ## Two protocols, as with the other provider
//!
//! - The control plane at `api.onkernel.com` is plain REST with a bearer token:
//!   create, look up, delete, and profiles.
//! - Driving a browser is the DevTools protocol on the `cdp_ws_url` that
//!   creation returns. `cdp.rs` speaks it.
//!
//! ## Where a sign-in lives
//!
//! A browser session is disposable; a *profile* is not. Each agent gets one
//! named profile, every browser it is given is created against that profile
//! with `save_changes`, and the cookies are written back when the browser is
//! deleted or times out. That is the same property the E2B machine gets from
//! its disk surviving a sleep, reached a different way, and it is why an
//! operator signs an agent in once rather than once a session.
//!
//! One writer at a time, which the provider does not enforce: two browsers open
//! on one profile means the last one closed overwrites the other's cookies. So
//! an agent holds at most one browser, recorded on its row, and a second is
//! never created while the first is alive.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::cdp::{CdpError, Page};
use crate::domain::signin::BrowserState;

const API_BASE: &str = "https://api.onkernel.com";

/// Long enough for a browser to boot, short enough that a stuck control-plane
/// call does not hold an agent's turn open.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// What Kernel accepts as an inactivity timeout, in seconds.
///
/// The provider's own bounds. Sent outside them the call is rejected, and an
/// operator who typed a number into a settings box should not be able to make
/// every browser fail to start.
const MIN_TIMEOUT_SECONDS: u32 = 10;
const MAX_TIMEOUT_SECONDS: u32 = 259_200;

/// How long a settled page is given to react, in milliseconds.
///
/// Each of these was a page that had not finished changing when it was read.
/// A navigation is the longest because it is a network round trip; a click is
/// shorter because most of them only re-render.
const SETTLE_NAVIGATE_MS: u32 = 2500;
const SETTLE_CLICK_MS: u32 = 1200;
const SETTLE_SCROLL_MS: u32 = 600;

/// Marks every browser this app made, so the sweep can tell them from the ones
/// somebody else's project put on the same account.
const TAG: &str = "guac";

#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    #[error("no Kernel API key is set; add one in app settings to give agents a browser")]
    NoKey,
    #[error("Kernel request failed: {0}")]
    Transport(String),
    #[error("Kernel rejected the request ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("Kernel replied in a form this build does not understand: {0}")]
    Protocol(String),
    #[error(transparent)]
    Cdp(#[from] CdpError),
}

/// What the UI needs to show an agent's browser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Browser {
    pub session_id: String,
    /// `running` or `gone`. There is no third state worth naming: standby is
    /// invisible from outside and ends the moment anything drives the browser,
    /// so reporting it would be reporting something the operator cannot act on.
    pub state: String,
    /// Where the operator watches, and takes over. Absent once the browser has
    /// gone, because the URL dies with it.
    pub live_view_url: Option<String>,
}

/// A live browser, with everything needed to drive it.
///
/// Kept together because they are useless apart: a session id without its
/// socket names a browser nothing can talk to.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub id: String,
    pub cdp_ws_url: String,
    pub live_view_url: Option<String>,
}

/// Kernel's own reply shape, reduced to the fields this app uses.
#[derive(Debug, Deserialize)]
struct BrowserRow {
    session_id: String,
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
    name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct KernelClient {
    http: reqwest::Client,
    api_key: String,
}

impl KernelClient {
    /// `None` when no key is configured, so callers can tell "not set up" apart
    /// from "set up and failing".
    pub fn new(api_key: &str) -> Option<Self> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return None;
        }
        let http = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build().ok()?;
        Some(Self { http, api_key: api_key.to_string() })
    }

    async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, KernelError> {
        let response = request
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| KernelError::Transport(e.to_string()))?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            // Kernel reports a machine-readable code beside the sentence. The
            // sentence is what an operator reads, so it is what is kept.
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

    /// The name of the profile this agent's sign-ins live in, creating it if it
    /// is not there yet.
    ///
    /// Idempotent by way of the conflict: asking for a profile that exists is
    /// the normal case after the first time, and Kernel says 409 rather than
    /// handing the existing one back. That is the answer this wants, so it is
    /// read as success rather than as a failure to report.
    pub async fn ensure_profile(&self, agent: &str) -> Result<String, KernelError> {
        let name = profile_name(agent);
        match self
            .call::<ProfileRow>(
                self.http.post(format!("{API_BASE}/profiles")).json(&json!({ "name": name })),
            )
            .await
        {
            Ok(row) => Ok(row.name.unwrap_or(name)),
            Err(KernelError::Api { status: 409, .. }) => Ok(name),
            Err(err) => Err(err),
        }
    }

    /// Creates a browser for one agent, on that agent's own profile.
    ///
    /// Headful deliberately. A headless browser is a quarter of the memory and
    /// cheaper to run, and it has no live view: the operator could not watch,
    /// could not take over, and could not sign the agent in, which is the one
    /// thing only they can do. It is also the shape that sites blocking
    /// automation look for first.
    pub async fn create(
        &self,
        agent: &str,
        idle_seconds: u32,
        stealth: bool,
    ) -> Result<Session, KernelError> {
        let profile = self.ensure_profile(agent).await?;
        let row: BrowserRow = self
            .call(self.http.post(format!("{API_BASE}/browsers")).json(&create_body(
                agent,
                &profile,
                idle_seconds,
                stealth,
            )))
            .await?;
        Ok(row.into())
    }

    /// The browser with this id, or `None` if it has gone.
    ///
    /// A browser that timed out is not an error. It is the expected end of one:
    /// the profile holds the cookies, so the answer is to make another, and a
    /// caller that had to distinguish a 404 from a real failure would get it
    /// wrong somewhere.
    pub async fn get(&self, id: &str) -> Result<Option<Session>, KernelError> {
        match self.call::<BrowserRow>(self.http.get(format!("{API_BASE}/browsers/{id}"))).await {
            Ok(row) => Ok(Some(row.into())),
            Err(KernelError::Api { status: 404, .. }) => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Ends a browser and writes its cookies back to the agent's profile.
    ///
    /// Deleting is what saves. A browser left to time out saves too, eventually
    /// and on the provider's clock, so this is also how an operator's sign-in
    /// is made durable now rather than in an hour.
    pub async fn delete(&self, id: &str) -> Result<(), KernelError> {
        match self.call::<Value>(self.http.delete(format!("{API_BASE}/browsers/{id}"))).await {
            Ok(_) => Ok(()),
            // Already gone is the outcome that was asked for.
            Err(KernelError::Api { status: 404, .. }) => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// Forgets everything an agent's browsers were signed in to.
    ///
    /// Called when the agent is deleted, and it is the profile rather than the
    /// browser that matters here: the browser was disposable and the profile is
    /// where the cookies went. A name is free to reuse the moment an agent is
    /// deleted, and the profile is named from the agent, so leaving it behind
    /// would hand the next agent of that name somebody else's sessions.
    pub async fn delete_profile(&self, agent: &str) -> Result<(), KernelError> {
        let name = profile_name(agent);
        match self.call::<Value>(self.http.delete(format!("{API_BASE}/profiles/{name}"))).await {
            Ok(_) => Ok(()),
            Err(KernelError::Api { status: 404, .. }) => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// Every browser this app made, whoever it belongs to.
    ///
    /// Filtered by the tag rather than by name, because the sweep's job is to
    /// find the ones nobody holds a reference to any more, and a browser
    /// created just before a crash has a name nothing recorded.
    pub async fn list_ours(&self) -> Result<Vec<String>, KernelError> {
        let rows: Vec<BrowserRow> = self
            .call(self.http.get(format!("{API_BASE}/browsers")).query(&[
                ("status", "active"),
                (&format!("tags[{TAG}]"), "true"),
                ("limit", "100"),
            ]))
            .await?;
        Ok(rows.into_iter().map(|row| row.session_id).collect())
    }

    /// One browser action, answered as the JSON the transcript renders.
    ///
    /// The shape is the contract with `render_page`: a url, a title, where the
    /// page is scrolled to, its text, and the numbered controls. The numbering
    /// is stored on the page itself, so a click refers to the element the model
    /// was shown rather than to whatever now sits at some position.
    pub async fn browse(
        &self,
        session: &Session,
        action: &str,
        args: &Value,
    ) -> Result<String, KernelError> {
        let mut page = Page::attach(&session.cdp_ws_url).await?;

        match action {
            "open" => {
                let url = args["url"].as_str().unwrap_or_default().trim().to_string();
                if url.is_empty() {
                    return Err(KernelError::Protocol(
                        "open needs a `url`. Give the address you want, for example \
                         `https://example.com`."
                            .into(),
                    ));
                }
                // A model that writes `example.com` means the web, not a
                // relative path. Refusing would be pedantry it has to guess its
                // way out of.
                let url = if url.starts_with("http://") || url.starts_with("https://") {
                    url
                } else {
                    format!("https://{url}")
                };
                page.navigate(&url).await?;
                page.settle(SETTLE_NAVIGATE_MS).await?;
            }

            "read" => {}

            "click" => {
                let target = element(args)?;
                require(&mut page, target).await?;
                page.evaluate(&format!(
                    "window.__guacEls[{target}].scrollIntoView({{block:'center'}})"
                ))
                .await?;
                page.evaluate(&format!("window.__guacEls[{target}].click()")).await?;
                page.settle(SETTLE_CLICK_MS).await?;
            }

            "type" => {
                let target = element(args)?;
                let text = args["text"].as_str().unwrap_or_default();
                require(&mut page, target).await?;
                // Focused and emptied here, then filled by the browser's own
                // input path. Setting the property directly is what this used
                // to do, and it left every framework that keeps its own copy of
                // the value showing the old text after the next render.
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
                    page.settle(SETTLE_NAVIGATE_MS).await?;
                } else {
                    page.settle(SETTLE_SCROLL_MS).await?;
                }
            }

            "scroll" => {
                let amount = args["amount"].as_i64().unwrap_or(3).clamp(1, 20) * 400;
                let amount =
                    if args["direction"].as_str() == Some("up") { -amount } else { amount };
                page.evaluate(&format!("scrollBy(0, {amount})")).await?;
                page.settle(SETTLE_SCROLL_MS).await?;
            }

            "back" => {
                page.evaluate("history.back()").await?;
                page.settle(SETTLE_NAVIGATE_MS).await?;
            }

            other => {
                return Err(KernelError::Protocol(format!(
                    "`{other}` is not something a browser does. Use open, read, click, type, \
                     scroll or back."
                )))
            }
        }

        let collected = page.evaluate(COLLECT).await?;
        collected
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| KernelError::Protocol("the page did not describe itself".into()))
    }

    /// Asks a browser what it is signed in to.
    ///
    /// Cookie names and flags only, and the type they land in has no field a
    /// value could arrive in. The whole rule for turning those into an account
    /// is in `domain/signin.rs` and is shared with the other provider, because
    /// "a cookie's presence is not a login" is a fact about the web rather than
    /// about where the browser runs.
    pub async fn signed_in_state(&self, session: &Session) -> Result<BrowserState, KernelError> {
        let mut page = Page::attach(&session.cdp_ws_url).await?;
        Ok(BrowserState { cookies: page.cookies().await?, visited: page.visited().await? })
    }
}

/// The element a `click` or `type` names, refused clearly when it is missing.
fn element(args: &Value) -> Result<i64, KernelError> {
    args["id"].as_i64().or_else(|| args["id"].as_str()?.parse().ok()).ok_or_else(|| {
        KernelError::Protocol(
            "that needs an `id`: the number `read` gave the element you mean. Read the page again \
             if you do not have one."
                .into(),
        )
    })
}

/// Refuses to act on a number the page no longer holds.
///
/// A page that has re-rendered has forgotten its numbering, and acting on
/// whatever element now happens to hold that number is how an agent clicks the
/// wrong thing and reports success. Saying so plainly is the only safe answer,
/// and it has to say what to do next or it gets retried verbatim.
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
    Err(KernelError::Protocol(format!(
        "element {index} is not on the page any more, because the page changed since you read it. \
         Read it again: the numbers are handed out fresh each time."
    )))
}

/// The profile an agent's sign-ins live in.
///
/// Named from the agent rather than generated, so the same agent finds the same
/// profile after a restart with nothing recorded anywhere. Kernel's names allow
/// letters, digits, dots, dashes and underscores, which a uuid satisfies.
fn profile_name(agent: &str) -> String {
    format!("guac-{agent}")
}

/// The body that creates a browser an operator can watch.
///
/// Built here rather than inline so the shape can be asserted. The two fields
/// worth staring at are `headless`, which decides whether there is a live view
/// at all, and `save_changes`, which decides whether a sign-in outlives the
/// session it was performed in.
fn create_body(agent: &str, profile: &str, idle_seconds: u32, stealth: bool) -> Value {
    json!({
        // Watchable, and the shape that looks least like a robot.
        "headless": false,
        // Where the cookies come from, and where they go back to. Without
        // `save_changes` a browser reads the profile and never writes to it, so
        // every sign-in lasts exactly one session.
        "profile": { "name": profile, "save_changes": true },
        // Counted from the last time anything drove it. Standby comes first and
        // is free; this is how long after that the browser is deleted.
        "timeout_seconds": idle_seconds.clamp(MIN_TIMEOUT_SECONDS, MAX_TIMEOUT_SECONDS),
        "stealth": stealth,
        // The tag is what the sweep matches on; the name is what an operator
        // sees in Kernel's own dashboard when they go looking.
        "tags": { TAG: "true", "guac-agent": agent },
        "name": format!("guac-{agent}"),
    })
}

/// Everything a person could click, type into or choose from, numbered.
///
/// Kept as one expression evaluated in the page. The numbering is stored on the
/// page in `window.__guacEls`, which is what makes a click refer to the element
/// the model was shown: a position would refer to whatever has since moved
/// under it.
///
/// Bounded on purpose in three places. Off-screen and zero-sized elements are
/// not things a person can use and listing them buries the ones that are; the
/// element list stops at 120, because a page with more controls than that is
/// one where reading further does not help; and the text stops at 6000
/// characters, which is a page rather than a context window.
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
    use super::*;

    /// Where a live view is served from, and the port it is served on.
    ///
    /// Named here because the window's CSP has to allow exactly this, and the
    /// two silently disagreeing is a blocked iframe that looks identical to a
    /// browser that failed to start. The port is part of it: the origin is on
    /// 8443 rather than 443, and a CSP entry without it matches nothing.
    const LIVE_VIEW_ORIGIN: &str = "https://*.onkernel.com:8443";

    #[test]
    fn the_window_is_allowed_to_frame_and_reach_a_live_view() {
        // The computer's viewer learned this the hard way: the CSP was left
        // behind when the address changed, every check at the HTTP layer passed
        // because curl does not enforce CSP, and the pane stayed black.
        //
        // A live view needs two entries rather than one. The frame loads over
        // HTTPS and then opens a WebSocket of its own for the pixels, so a
        // `frame-src` without a matching `connect-src` is a frame that loads
        // and never paints.
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
        assert!(frame_src.contains(LIVE_VIEW_ORIGIN), "got {frame_src:?}");

        let connect_src = directive("connect-src");
        assert!(connect_src.contains(LIVE_VIEW_ORIGIN), "got {connect_src:?}");
        assert!(
            connect_src.contains("wss://*.onkernel.com:8443"),
            "the pixels arrive over a WebSocket: {connect_src:?}"
        );
    }

    #[test]
    fn a_live_view_is_never_routed_through_the_computer_viewer() {
        // The computer's viewer exists because a sandbox refuses traffic
        // without a header, and an iframe cannot set one. A live view URL
        // carries its own token in the path, so it needs no relay: pointing it
        // at the loopback proxy would send it to a host that only knows how to
        // reach E2B.
        assert!(LIVE_VIEW_ORIGIN.starts_with("https://"), "{LIVE_VIEW_ORIGIN}");
        assert!(!LIVE_VIEW_ORIGIN.contains(crate::e2b::VIEWER_HOST), "{LIVE_VIEW_ORIGIN}");
    }

    #[test]
    fn a_browser_is_created_watchable_and_on_the_agents_own_profile() {
        let body = create_body("agent-1", "guac-agent-1", 3600, false);

        // Headless has no live view, and the live view is the only way an
        // operator signs an agent in.
        assert_eq!(body["headless"], json!(false), "{body}");
        // Without this the profile is read and never written, so every sign-in
        // lasts one session and the operator does it again tomorrow.
        assert_eq!(body["profile"]["save_changes"], json!(true), "{body}");
        assert_eq!(body["profile"]["name"], json!("guac-agent-1"), "{body}");
        assert_eq!(body["timeout_seconds"], json!(3600), "{body}");
        // The sweep matches on this. Untagged browsers are somebody else's.
        assert_eq!(body["tags"]["guac"], json!("true"), "{body}");
        assert_eq!(body["tags"]["guac-agent"], json!("agent-1"), "{body}");
    }

    #[test]
    fn an_out_of_range_timeout_is_clamped_rather_than_rejected() {
        // An operator typing a number into a settings box must not be able to
        // make every browser fail to start.
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
        // Two browsers writing to one profile means the last one closed
        // overwrites the other, so the name has to be derived rather than
        // generated and recorded.
        assert_eq!(profile_name("abc"), profile_name("abc"));
        assert_ne!(profile_name("abc"), profile_name("abd"));
        assert!(profile_name("abc").starts_with("guac-"));
    }

    #[test]
    fn an_element_number_is_read_from_either_shape_a_model_sends() {
        // Models routinely send a string where an integer is specified, and
        // refusing that produces a retry loop rather than a working app.
        assert_eq!(element(&json!({ "id": 4 })).unwrap(), 4);
        assert_eq!(element(&json!({ "id": "4" })).unwrap(), 4);
        // And a missing one has to say what to do about it, or the model
        // rewords the same call and sends it again.
        let refusal = element(&json!({})).unwrap_err().to_string();
        assert!(refusal.contains("read"), "{refusal}");
    }

    #[test]
    fn the_collector_hands_back_the_shape_the_transcript_renders() {
        // `render_page` reads these keys. A collector that stopped emitting one
        // would leave a page that renders as a blank heading, which is not a
        // failure anything reports.
        for key in ["url", "title", "scroll", "height", "text", "elements"] {
            assert!(COLLECT.contains(&format!("{key}:")), "the page must report {key}");
        }
        // The numbering lives on the page, which is what makes a click refer to
        // the element the model was shown.
        assert!(COLLECT.contains("window.__guacEls"), "{COLLECT}");
    }

    #[test]
    fn a_key_that_is_blank_means_not_configured_rather_than_always_failing() {
        assert!(KernelClient::new("").is_none());
        assert!(KernelClient::new("   ").is_none());
        assert!(KernelClient::new("sk-live").is_some());
    }

    #[test]
    fn a_missing_live_view_is_absent_rather_than_empty() {
        // An empty string in an iframe's `src` loads the app itself into the
        // frame, which draws a copy of the window inside the pane.
        let row = BrowserRow {
            session_id: "s".into(),
            cdp_ws_url: "wss://x".into(),
            browser_live_view_url: Some("  ".into()),
        };
        assert_eq!(Session::from(row).live_view_url, None);
    }
}
