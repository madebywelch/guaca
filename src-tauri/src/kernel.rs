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

/// The control plane this build talks to.
///
/// [`API_BASE`] everywhere except under `GUAC_KERNEL_API_BASE`, which is the
/// seam `tests/machines.rs` points at a scripted one. The same shape
/// `GUACA_ACCOUNT_ORIGIN` has and here for the same reason: the rule this
/// module keeps is that an agent holds one browser, and the only way to prove
/// it is against a provider that refuses the second.
fn api_base() -> String {
    match std::env::var("GUAC_KERNEL_API_BASE") {
        Ok(base) if !base.trim().is_empty() => base.trim().trim_end_matches('/').to_string(),
        _ => API_BASE.to_string(),
    }
}

/// The hosts a live view is served from, and the port it is served on.
///
/// Two hosts because the provider moved: a browser created today answers on
/// `kernel.sh`, and this app framed `onkernel.com` until it did. The old one
/// stays because which host an account is issued is Kernel's decision and not
/// one it announces. An entry for a host nobody is issued any more costs a
/// line in an allowlist; a missing one costs the whole pane.
///
/// The window's CSP has to allow exactly these, and `framable` is the runtime
/// half of the same rule. A test can only prove the config agrees with what
/// this build believes, and what changed here was what the provider sends.
const LIVE_VIEW_HOSTS: [&str; 2] = ["kernel.sh", "onkernel.com"];

/// Part of the origin rather than decoration: a live view answers on 8443
/// rather than 443, and a CSP entry without the port matches nothing.
const LIVE_VIEW_PORT: u16 = 8443;

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

/// Marks a browser with the agent it was made for, which is how one is found
/// again when nothing in this app is holding its id.
const AGENT_TAG: &str = "guac-agent";

#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    #[error("no Kernel API key is set; add one in app settings to give agents a browser")]
    NoKey,
    /// As `E2bError::NotGiven`, and for the same reason: a browser is handed
    /// out one agent at a time, and an agent that was not given one cannot make
    /// itself one.
    #[error("this agent has not been given a browser; the operator gives one from its panel")]
    NotGiven,
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
    /// gone, because the URL dies with it, and absent when the provider served
    /// one this window may not frame, which `unwatchable` then names.
    pub live_view_url: Option<String>,
    /// The origin of a live view this window is not allowed to frame.
    ///
    /// Set instead of `live_view_url` and never beside it: the pane has either
    /// a picture to draw or a sentence to say about why there is none.
    pub unwatchable: Option<String>,
}

impl Browser {
    /// A running browser, as its pane needs it.
    ///
    /// The live view is separated from its origin here because an iframe the
    /// CSP refuses is not an error anywhere. Nothing throws, nothing is logged,
    /// and the pane draws the surface behind the frame, which is a black
    /// rectangle full screen and a gray one in the panel: exactly what a
    /// browser that failed to start looks like. Kernel moving this host from
    /// `onkernel.com` to `kernel.sh` is what that cost, and the CSP test could
    /// not see it because both halves it compares are this build's own.
    pub fn running(session: Session) -> Self {
        let (live_view_url, unwatchable) = match session.live_view_url {
            Some(url) if !framable(&url) => {
                let origin = origin_of(&url);
                tracing::warn!(
                    %origin,
                    session = %session.id,
                    "Kernel served a live view this window is not allowed to frame; \
                     the browser is running and only watching it is refused"
                );
                (None, Some(origin))
            }
            watchable => (watchable, None),
        };
        Browser { session_id: session.id, state: "running".to_string(), live_view_url, unwatchable }
    }
}

/// Whether the window is allowed to frame this live view.
///
/// The same question the CSP answers, asked of the URL that actually arrived.
/// Scheme, host and port all count, and the host has to be *under* one of the
/// allowed names rather than equal to it, because a `*.` entry does not match
/// the bare domain either.
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

/// The `scheme://host:port` of a URL, which is the part a CSP entry is made of
/// and the part worth naming to whoever has to add one.
///
/// Anything before an `@` is dropped rather than shown. A live view URL carries
/// its token in the path and has never had credentials in it, but this string
/// reaches the pane, and a rule that only holds while a vendor keeps a habit is
/// not a rule.
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
    base: String,
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
        Some(Self { http, api_key: api_key.to_string(), base: api_base() })
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
                self.http.post(format!("{}/profiles", self.base)).json(&json!({ "name": name })),
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
            // The name is one per agent, so a conflict is this agent's own
            // browser, alive and unrecorded: a crash between creating one and
            // writing it down, or a row cleared while the browser was up. That
            // browser is what the caller asked for, and the alternative to
            // taking it is a `browse` tool that refuses every call until the
            // orphan times out.
            Err(KernelError::Api { status: 409, message }) => match self.held_by(agent).await? {
                Some(live) => Ok(live),
                None => Err(KernelError::Api { status: 409, message }),
            },
            Err(err) => Err(err),
        }
    }

    /// The browser this agent already has, whatever recorded it.
    ///
    /// Found by the tag rather than by the name, because the name is the thing
    /// the conflict was about and the tag is what says whose browser it is. The
    /// socket comes from asking for the session itself: a list row is not
    /// documented to carry one, and a `Session` without its socket names a
    /// browser nothing can talk to.
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

    /// The browser with this id, or `None` if it has gone.
    ///
    /// A browser that timed out is not an error. It is the expected end of one:
    /// the profile holds the cookies, so the answer is to make another, and a
    /// caller that had to distinguish a 404 from a real failure would get it
    /// wrong somewhere.
    pub async fn get(&self, id: &str) -> Result<Option<Session>, KernelError> {
        match self.call::<BrowserRow>(self.http.get(format!("{}/browsers/{id}", self.base))).await {
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
        match self.call::<Value>(self.http.delete(format!("{}/browsers/{id}", self.base))).await {
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
        match self.call::<Value>(self.http.delete(format!("{}/profiles/{name}", self.base))).await {
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
            .call(self.http.get(format!("{}/browsers", self.base)).query(&[
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

        // How long the page is given to finish what the action started. Each
        // arm does its action and says only that, because the wait and the
        // description that follows it are the same two steps for every action
        // and have to be able to happen on a page this socket has lost.
        let settle_ms = match action {
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
                SETTLE_NAVIGATE_MS
            }

            // Nothing happened, so there is nothing to wait for.
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
                return Err(KernelError::Protocol(format!(
                    "`{other}` is not something a browser does. Use open, read, click, type, \
                     scroll or back."
                )))
            }
        };

        // Attaching again is how the ordinary case survives: a navigation can
        // replace the target this socket attached to, and Chrome then fails the
        // wait and the description rather than the navigation that caused them.
        // An `open` that redirects is enough to do it, and the answer used to be
        // "Inspected target navigated or closed" on a page that had loaded
        // perfectly well. An agent that reads that concludes the browser cannot
        // see the web and goes to work the same page through screenshots.
        //
        // Read again, never act again. The action has already happened, and a
        // click sent a second time is the one mistake this must not make.
        let collected = match settle_and_collect(&mut page, settle_ms).await {
            Err(CdpError::TargetGone) => {
                let mut replacement = Page::attach(&session.cdp_ws_url).await?;
                settle_and_collect(&mut replacement, settle_ms).await?
            }
            other => other?,
        };
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

/// Waits for the page to finish, then has it describe itself.
///
/// One function because the two are never wanted apart, and because both have
/// to be repeatable against a page that has been replaced: a description taken
/// before the page settled is a description of the page before the action, and
/// a wait with no description afterward answers nothing.
async fn settle_and_collect(page: &mut Page, settle_ms: u32) -> Result<Value, CdpError> {
    if settle_ms > 0 {
        page.settle(settle_ms).await?;
    }
    page.evaluate(COLLECT).await
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
        "tags": { TAG: "true", AGENT_TAG: agent },
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use futures_util::{SinkExt, StreamExt};
    use parking_lot::Mutex;
    use tokio_tungstenite::tungstenite::Message;

    use super::*;

    /// One CSP entry, built from the same constants the runtime check uses.
    fn live_view_origin(scheme: &str, host: &str) -> String {
        format!("{scheme}://*.{host}:{LIVE_VIEW_PORT}")
    }

    #[test]
    fn the_window_is_allowed_to_frame_and_reach_every_host_a_live_view_arrives_on() {
        // The computer's viewer learned this the hard way: the CSP was left
        // behind when the address changed, every check at the HTTP layer passed
        // because curl does not enforce CSP, and the pane stayed black. Kernel
        // then moved its own live view from `onkernel.com` to `kernel.sh` and
        // it happened again, which is why `framable` exists beside this test.
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
        // What the CSP refuses draws as the surface behind the frame and not as
        // an error, so this is the only place the mismatch can be noticed. Both
        // hosts the provider has used are accepted, because an account is
        // issued one or the other and neither is this app's choice.
        for host in LIVE_VIEW_HOSTS {
            assert!(
                framable(&format!("https://prod-jfk-1.{host}:8443/browser/live/tok")),
                "{host}"
            );
        }

        // Each of these is a way the provider could move again, and each draws
        // the same black rectangle if it is trusted rather than checked.
        assert!(!framable("https://prod-jfk-1.example.com:8443/browser/live/tok"));
        assert!(
            !framable("https://prod-jfk-1.kernel.sh/browser/live/tok"),
            "the port is the origin"
        );
        assert!(!framable("http://prod-jfk-1.kernel.sh:8443/browser/live/tok"), "TLS is too");
        // `*.kernel.sh` does not match the bare domain, so neither does this.
        assert!(!framable("https://kernel.sh:8443/browser/live/tok"));
        assert!(!framable("https://kernel.sh.example.com:8443/live"), "a suffix, not a substring");

        let refused = Browser::running(Session {
            id: "s".into(),
            cdp_ws_url: "wss://x".into(),
            live_view_url: Some("https://prod-jfk-1.example.com:8443/browser/live/tok".into()),
        });
        // The pane is told the origin and never handed the URL: a frame pointed
        // at it is the blank rectangle this exists to replace.
        assert_eq!(refused.live_view_url, None);
        assert_eq!(refused.unwatchable.as_deref(), Some("https://prod-jfk-1.example.com:8443"));
        assert_eq!(refused.state, "running", "the browser is fine; only watching it is refused");

        let shown = Browser::running(Session {
            id: "s".into(),
            cdp_ws_url: "wss://x".into(),
            live_view_url: Some("https://prod-jfk-1.kernel.sh:8443/browser/live/tok".into()),
        });
        assert_eq!(
            shown.live_view_url.as_deref(),
            Some("https://prod-jfk-1.kernel.sh:8443/browser/live/tok")
        );
        assert_eq!(shown.unwatchable, None);
    }

    #[test]
    fn an_origin_names_the_address_and_nothing_that_was_in_front_of_it() {
        // This string reaches the pane. A URL that carried credentials would
        // put them there, and the provider not doing that today is a habit
        // rather than a guarantee.
        assert_eq!(
            origin_of("https://user:secret@prod-jfk-1.kernel.sh:8443/browser/live/tok?jwt=abc"),
            "https://prod-jfk-1.kernel.sh:8443"
        );
        assert_eq!(origin_of("https://host:8443"), "https://host:8443");
    }

    #[test]
    fn a_live_view_is_never_routed_through_the_computer_viewer() {
        // The computer's viewer exists because a sandbox refuses traffic
        // without a header, and an iframe cannot set one. A live view URL
        // carries its own token in the path, so it needs no relay: pointing it
        // at the loopback proxy would send it to a host that only knows how to
        // reach E2B.
        for host in LIVE_VIEW_HOSTS {
            let origin = live_view_origin("https", host);
            assert!(!origin.contains(crate::e2b::VIEWER_HOST), "{origin}");
        }
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

    /// The page the fake browser describes, in the shape `render_page` reads.
    const FAKE_PAGE: &str = r#"{"url":"https://example.com/after","title":"After","scroll":0,
        "height":2000,"text":"arrived","elements":[]}"#;

    /// A browser that speaks enough of the protocol for one action.
    ///
    /// A real socket rather than a stubbed client, because what is being tested
    /// is the protocol's own behavior: a session stops answering the moment the
    /// page it names is replaced, and nothing but a connection can express
    /// that. Every expression it is asked to evaluate is kept, which is how a
    /// test can say an action was not performed twice.
    struct FakeBrowser {
        url: String,
        evaluated: Arc<Mutex<Vec<String>>>,
        connections: Arc<AtomicUsize>,
    }

    impl FakeBrowser {
        /// `lose_first` connections answer the action and then report their
        /// target as gone, which is what Chrome does when a navigation replaces
        /// the target underneath an attached session.
        async fn start_losing(lose_first: usize) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a port");
            let address = listener.local_addr().expect("an address");
            let evaluated: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let connections = Arc::new(AtomicUsize::new(0));

            let (recorded, counted) = (evaluated.clone(), connections.clone());
            tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else { return };
                    let lost = counted.fetch_add(1, Ordering::SeqCst) < lose_first;
                    let recorded = recorded.clone();
                    tokio::spawn(async move {
                        let mut socket =
                            tokio_tungstenite::accept_async(stream).await.expect("a handshake");
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
                                "Page.navigate" => json!({"id": id, "result": {}}),
                                "Runtime.evaluate" => {
                                    recorded.lock().push(expression.to_string());
                                    // The settle and the description of the
                                    // page, which are what a navigation lands
                                    // on. Everything before them is the action
                                    // itself, and it goes through.
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

            FakeBrowser { url: format!("ws://{address}"), evaluated, connections }
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
        // The failure this exists for: an `open` that redirects loads a page
        // perfectly well, and every call after the navigation fails on a
        // session naming a target that is gone. The agent that met it concluded
        // its browser could not see the web and went to work the same page
        // through screenshots, which its model could not even accept.
        let browser = FakeBrowser::start_losing(1).await;
        let client = KernelClient::new("sk-test").expect("a client");

        let page = client
            .browse(&browser.session(), "open", &json!({ "url": "https://example.com" }))
            .await
            .expect("the page loaded, so the action succeeded");

        assert!(page.contains("https://example.com/after"), "{page}");
        // One that lost its target, one that read the page. Not a loop.
        assert_eq!(browser.connections(), 2);
    }

    #[tokio::test]
    async fn a_click_that_loses_the_page_is_never_sent_a_second_time() {
        // The whole reason the recovery re-reads instead of retrying. A button
        // that navigates is the commonest button there is, and pressing it
        // again because the answer went missing is the one mistake a browser
        // tool must not make.
        let browser = FakeBrowser::start_losing(1).await;
        let client = KernelClient::new("sk-test").expect("a client");

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
        let client = KernelClient::new("sk-test").expect("a client");

        client.browse(&browser.session(), "read", &json!({})).await.expect("read the page");

        assert_eq!(browser.connections(), 1, "a working page must not pay for a reconnection");
    }

    #[tokio::test]
    async fn a_page_that_will_not_stay_still_says_what_to_do_about_it() {
        let browser = FakeBrowser::start_losing(usize::MAX).await;
        let client = KernelClient::new("sk-test").expect("a client");

        let err = client
            .browse(&browser.session(), "open", &json!({ "url": "https://example.com" }))
            .await
            .expect_err("nothing could be read");

        // Chrome's own wording reaches nobody. What the agent gets is a way
        // forward, because a refusal that only says no gets retried verbatim.
        assert!(err.to_string().contains("Read the page again"), "{err}");
        assert_eq!(browser.connections(), 2, "one retry, not a loop");
    }
}
