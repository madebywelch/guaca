use std::net::SocketAddr;
use std::time::Duration;

use serde_json::{json, Value};

use crate::cdp::{CdpError, Dialog, Page};
use crate::db::store::Store;
use crate::domain::ids::AgentId;
use crate::domain::signin::BrowserState;
use crate::kernel::{KernelClient, KernelError};

const SETTLE_NAVIGATE_MS: u32 = 2500;
const SETTLE_CLICK_MS: u32 = 1200;
const SETTLE_SCROLL_MS: u32 = 600;

const LOCAL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub id: String,
    pub cdp_ws_url: String,
    pub live_view_url: Option<String>,
    pub target: Option<String>,
}

#[derive(Debug)]
pub struct Ensured {
    pub session: Session,
    pub fresh: bool,
    pub replaced: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    #[error("{}", crate::domain::deployment::Absent::LocalBrowser.sentence())]
    Unavailable,
    #[error("this agent has not been given a browser; the operator gives one from its panel")]
    NotGiven,
    #[error(
        "The browser on this machine is being set up, so nothing was opened. The first time \
         takes a minute or two. Carry on with what you can do without the web, and try again \
         shortly."
    )]
    SettingUp,
    #[error("{0}")]
    Input(String),
    #[error("the browser on this machine did not answer as expected: {0}")]
    Local(String),
    #[error(transparent)]
    Kernel(#[from] KernelError),
    #[error(transparent)]
    Cdp(#[from] CdpError),
}

impl BrowserError {
    pub fn for_agent(self) -> String {
        match self {
            BrowserError::Kernel(
                KernelError::Transport(_) | KernelError::Api { status: 429 | 500..=599, .. },
            ) => "The browser provider is temporarily unavailable. Try again later.".to_string(),
            BrowserError::Kernel(KernelError::Api { status: 401 | 403, .. }) => {
                "The browser provider refused access. Ask the operator to check its credentials and access.".to_string()
            }
            BrowserError::Kernel(KernelError::Api { .. } | KernelError::Protocol(_))
            | BrowserError::Cdp(_)
            | BrowserError::Local(_) => {
                "The browser provider could not confirm the resource's state. Ask the operator to check it.".to_string()
            }
            other => other.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalState {
    Absent,
    Kernel,
    Idle,
    SettingUp,
    Ready,
}

pub async fn local_state(
    kernel_key: &crate::secrets::Secret,
    local: Option<&Local>,
    wanted: bool,
) -> LocalState {
    let Some(local) = local else {
        return LocalState::Absent;
    };
    if KernelClient::new(kernel_key).is_some() {
        return LocalState::Kernel;
    }
    if local.ready().await {
        LocalState::Ready
    } else if wanted {
        LocalState::SettingUp
    } else {
        LocalState::Idle
    }
}

#[derive(Debug, Clone)]
pub enum Backend {
    Kernel(KernelClient),
    Local(Local),
}

impl Backend {
    pub fn choose(kernel_key: &crate::secrets::Secret, local: Option<&Local>) -> Option<Self> {
        KernelClient::new(kernel_key)
            .map(Backend::Kernel)
            .or_else(|| local.cloned().map(Backend::Local))
    }

    pub async fn ensure(
        &self,
        store: &Store,
        agent: AgentId,
        idle_seconds: u32,
        stealth: bool,
    ) -> Result<Ensured, BrowserError> {
        match self {
            Backend::Kernel(client) => {
                Ok(client.ensure_for(store, agent, idle_seconds, stealth).await?)
            }
            Backend::Local(local) => local.ensure(store, agent).await,
        }
    }

    pub async fn get(&self, id: &str) -> Result<Option<Session>, BrowserError> {
        match self {
            Backend::Kernel(client) => Ok(client.get(id).await?),
            Backend::Local(local) => local.get(id).await,
        }
    }

    pub async fn close(&self, id: &str) -> Result<(), BrowserError> {
        match self {
            Backend::Kernel(client) => Ok(client.delete(id).await?),
            Backend::Local(local) => local.close(id).await,
        }
    }

    pub async fn purge(&self, store: &Store, agent: AgentId, browser: Option<&str>) {
        match self {
            Backend::Kernel(client) => client.purge_for(store, agent, browser).await,
            Backend::Local(local) => {
                if let Some(browser) = browser {
                    if let Err(err) = local.close(browser).await {
                        tracing::warn!(%err, %browser, "could not close the agent's browser");
                    }
                }
            }
        }
    }

    pub async fn sweep(&self, store: &Store) -> Result<usize, BrowserError> {
        match self {
            Backend::Kernel(client) => Ok(client.sweep(store).await?),
            Backend::Local(_) => Ok(0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Local {
    address: String,
}

impl Local {
    pub fn new(address: &str) -> Option<Self> {
        let address = address.trim();
        (!address.is_empty()).then(|| Self { address: address.to_string() })
    }

    async fn endpoint(&self) -> Result<(String, String), BrowserError> {
        let found =
            tokio::time::timeout(LOCAL_TIMEOUT, tokio::net::lookup_host(self.address.as_str()))
                .await
                .ok()
                .and_then(Result::ok)
                .map(|found| found.collect::<Vec<SocketAddr>>())
                .unwrap_or_default();
        let address = found
            .iter()
            .find(|address| address.is_ipv4())
            .or_else(|| found.first())
            .ok_or(BrowserError::SettingUp)?;
        let base = format!("http://{address}");
        let http = reqwest::Client::builder()
            .timeout(LOCAL_TIMEOUT)
            .build()
            .map_err(|err| BrowserError::Local(err.to_string()))?;
        let version: Value = match http.get(format!("{base}/json/version")).send().await {
            Ok(reply) if reply.status().is_success() => reply.json().await.map_err(|_| {
                BrowserError::Local("it described itself in a form this build does not read".into())
            })?,
            _ => return Err(BrowserError::SettingUp),
        };
        let socket = version["webSocketDebuggerUrl"]
            .as_str()
            .filter(|socket| socket.starts_with(&format!("ws://{address}/")))
            .ok_or_else(|| {
                BrowserError::Local(
                    "it handed back a socket at an address it was not asked on".into(),
                )
            })?;
        Ok((base, socket.to_string()))
    }

    async fn pages(&self, base: &str) -> Result<Vec<String>, BrowserError> {
        let http = reqwest::Client::builder()
            .timeout(LOCAL_TIMEOUT)
            .build()
            .map_err(|err| BrowserError::Local(err.to_string()))?;
        let listed: Vec<Value> = http
            .get(format!("{base}/json/list"))
            .send()
            .await
            .map_err(|_| BrowserError::SettingUp)?
            .json()
            .await
            .map_err(|_| {
                BrowserError::Local("it listed its pages in a form this build does not read".into())
            })?;
        Ok(listed
            .iter()
            .filter(|target| target["type"] == "page")
            .filter_map(|target| target["id"].as_str().map(str::to_string))
            .collect())
    }

    fn session(id: &str, socket: &str) -> Session {
        Session {
            id: id.to_string(),
            cdp_ws_url: socket.to_string(),
            live_view_url: None,
            target: Some(id.to_string()),
        }
    }

    async fn ready(&self) -> bool {
        self.endpoint().await.is_ok()
    }

    async fn ensure(&self, store: &Store, agent: AgentId) -> Result<Ensured, BrowserError> {
        let held = store
            .get_agent(agent)
            .map_err(|err| {
                BrowserError::Local(format!("could not read the agent's browser: {err}"))
            })?
            .ok_or_else(|| BrowserError::Local("the agent is no longer available".into()))?
            .browser_id;
        let (base, socket) = self.endpoint().await?;
        if let Some(id) = &held {
            if self.pages(&base).await?.contains(id) {
                return Ok(Ensured {
                    session: Self::session(id, &socket),
                    fresh: false,
                    replaced: false,
                });
            }
        }
        let target = Page::open_alone(&socket).await?;
        if let Err(err) = store.set_agent_browser(agent, Some(&target)) {
            if let Err(cleanup) = Page::close_alone(&socket, &target).await {
                tracing::warn!(%cleanup, %target, "a browser that could not be recorded was left open");
            }
            return Err(BrowserError::Local(format!("the browser could not be recorded ({err})")));
        }
        Ok(Ensured {
            session: Self::session(&target, &socket),
            fresh: true,
            replaced: held.is_some(),
        })
    }

    async fn get(&self, id: &str) -> Result<Option<Session>, BrowserError> {
        let (base, socket) = self.endpoint().await?;
        Ok(self
            .pages(&base)
            .await?
            .iter()
            .any(|page| page == id)
            .then(|| Self::session(id, &socket)))
    }

    async fn close(&self, id: &str) -> Result<(), BrowserError> {
        match self.endpoint().await {
            Ok((_, socket)) => Ok(Page::close_alone(&socket, id).await?),
            Err(BrowserError::SettingUp) => Ok(()),
            Err(err) => Err(err),
        }
    }
}

pub async fn browse(session: &Session, action: &str, args: &Value) -> Result<String, BrowserError> {
    let mut page = Page::attach_to(&session.cdp_ws_url, session.target.as_deref()).await?;

    let settle_ms = match action {
        "open" => {
            let url = args["url"].as_str().unwrap_or_default().trim().to_string();
            if url.is_empty() {
                return Err(BrowserError::Input(
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
            let amount = if args["direction"].as_str() == Some("up") { -amount } else { amount };
            page.evaluate(&format!("scrollBy(0, {amount})")).await?;
            SETTLE_SCROLL_MS
        }

        "back" => {
            page.evaluate("history.back()").await?;
            SETTLE_NAVIGATE_MS
        }

        other => {
            return Err(BrowserError::Input(format!(
                "`{other}` is not something a browser does. Use open, read, click, type, \
                 scroll or back."
            )))
        }
    };

    let (collected, answered) = match settle_and_collect(&mut page, settle_ms).await {
        Err(CdpError::TargetGone) => {
            let mut replacement =
                Page::attach_to(&session.cdp_ws_url, session.target.as_deref()).await?;
            let collected = settle_and_collect(&mut replacement, settle_ms).await?;
            let mut answered = page.answered().to_vec();
            answered.extend_from_slice(replacement.answered());
            (collected, answered)
        }
        other => (other?, page.answered().to_vec()),
    };
    let described = collected
        .as_str()
        .ok_or_else(|| CdpError::Protocol("the page did not describe itself".into()))?;
    Ok(note_dialogs(described, &answered))
}

pub async fn signed_in_state(session: &Session) -> Result<BrowserState, BrowserError> {
    let mut page = Page::attach_to(&session.cdp_ws_url, session.target.as_deref()).await?;
    Ok(BrowserState { cookies: page.cookies().await?, visited: page.visited().await? })
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

fn element(args: &Value) -> Result<i64, BrowserError> {
    args["id"].as_i64().or_else(|| args["id"].as_str()?.parse().ok()).ok_or_else(|| {
        BrowserError::Input(
            "that needs an `id`: the number `read` gave the element you mean. Read the page again \
             if you do not have one."
                .into(),
        )
    })
}

async fn require(page: &mut Page, index: i64) -> Result<(), BrowserError> {
    let held = page
        .evaluate(&format!(
            "Boolean(window.__guacEls && window.__guacEls[{index}] \
             && document.contains(window.__guacEls[{index}]))"
        ))
        .await?;
    if held.as_bool() == Some(true) {
        return Ok(());
    }
    Err(BrowserError::Input(format!(
        "element {index} is not on the page any more, because the page changed since you read it. \
         Read it again: the numbers are handed out fresh each time."
    )))
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
            Session {
                id: "s".into(),
                cdp_ws_url: self.url.clone(),
                live_view_url: None,
                target: None,
            }
        }

        fn connections(&self) -> usize {
            self.connections.load(Ordering::SeqCst)
        }
    }

    #[tokio::test]
    async fn a_page_that_navigates_underneath_an_action_is_read_again_not_reported_as_broken() {
        let browser = FakeBrowser::start_losing(1).await;

        let page = browse(&browser.session(), "open", &json!({ "url": "https://example.com" }))
            .await
            .expect("the page loaded, so the action succeeded");

        assert!(page.contains("https://example.com/after"), "{page}");
        assert_eq!(browser.connections(), 2);
    }

    #[tokio::test]
    async fn a_click_that_loses_the_page_is_never_sent_a_second_time() {
        let browser = FakeBrowser::start_losing(1).await;

        let page = browse(&browser.session(), "click", &json!({ "id": 3 }))
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

        browse(&browser.session(), "read", &json!({})).await.expect("read the page");

        assert_eq!(browser.connections(), 1, "a working page must not pay for a reconnection");
    }

    #[tokio::test]
    async fn a_leave_site_box_is_answered_so_the_action_it_stopped_can_finish() {
        let browser = FakeBrowser::start_asking("beforeunload").await;

        let page = browse(&browser.session(), "open", &json!({ "url": "https://example.com" }))
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

        let page = browse(&browser.session(), "open", &json!({ "url": "https://example.com" }))
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

        let err = browse(&browser.session(), "open", &json!({ "url": "https://example.com" }))
            .await
            .expect_err("nothing could be read");

        assert!(err.to_string().contains("Read the page again"), "{err}");
        assert_eq!(browser.connections(), 2, "one retry, not a loop");
    }

    #[derive(Default)]
    struct Sidecar {
        pages: Mutex<Vec<(String, String)>>,
        contexts: AtomicUsize,
        targets: AtomicUsize,
        evaluated: Mutex<Vec<(String, String)>>,
    }

    struct FakeSidecar {
        address: SocketAddr,
        state: Arc<Sidecar>,
        socket_host: Arc<Mutex<Option<String>>>,
    }

    impl FakeSidecar {
        async fn start() -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a port");
            let address = listener.local_addr().expect("an address");
            let state = Arc::new(Sidecar::default());
            let socket_host = Arc::new(Mutex::new(None::<String>));
            let (served, hosted) = (state.clone(), socket_host.clone());
            tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else { return };
                    let (state, hosted) = (served.clone(), hosted.clone());
                    tokio::spawn(
                        async move { serve_sidecar(stream, address, state, hosted).await },
                    );
                }
            });
            FakeSidecar { address, state, socket_host }
        }

        fn local(&self) -> Local {
            Local::new(&self.address.to_string()).expect("an address")
        }
    }

    async fn serve_sidecar(
        mut stream: tokio::net::TcpStream,
        address: SocketAddr,
        state: Arc<Sidecar>,
        hosted: Arc<Mutex<Option<String>>>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut head = vec![0u8; 2048];
        let peeked = stream.peek(&mut head).await.expect("a request");
        let request = String::from_utf8_lossy(&head[..peeked]).to_string();
        if request.to_ascii_lowercase().contains("upgrade: websocket") {
            let mut socket = tokio_tungstenite::accept_async(stream).await.expect("a handshake");
            while let Some(Ok(frame)) = socket.next().await {
                let Message::Text(text) = frame else { continue };
                let call: Value = serde_json::from_str(&text).expect("a call");
                let id = call["id"].clone();
                let session = call["sessionId"].as_str().unwrap_or_default().to_string();
                let result = match call["method"].as_str().unwrap_or_default() {
                    "Target.createBrowserContext" => {
                        let n = state.contexts.fetch_add(1, Ordering::SeqCst);
                        json!({ "browserContextId": format!("context-{n}") })
                    }
                    "Target.createTarget" => {
                        let context = call["params"]["browserContextId"]
                            .as_str()
                            .expect("a page made alone, in a context of its own")
                            .to_string();
                        let target =
                            format!("page-{}", state.targets.fetch_add(1, Ordering::SeqCst));
                        state.pages.lock().push((target.clone(), context));
                        json!({ "targetId": target })
                    }
                    "Target.getTargets" => json!({ "targetInfos": state.pages.lock().iter()
                        .map(|(target, context)| json!({
                            "targetId": target, "type": "page", "browserContextId": context,
                        }))
                        .collect::<Vec<_>>() }),
                    "Target.disposeBrowserContext" => {
                        let context =
                            call["params"]["browserContextId"].as_str().unwrap_or_default();
                        state.pages.lock().retain(|(_, held)| held != context);
                        json!({})
                    }
                    "Target.attachToTarget" => {
                        json!({ "sessionId": call["params"]["targetId"] })
                    }
                    "Page.enable" => json!({}),
                    "Runtime.evaluate" => {
                        let expression = call["params"]["expression"].as_str().unwrap_or_default();
                        state.evaluated.lock().push((session.clone(), expression.to_string()));
                        if expression.contains("location.href") {
                            json!({ "result": { "value": FAKE_PAGE } })
                        } else {
                            json!({ "result": { "value": 1 } })
                        }
                    }
                    other => panic!("the fake sidecar was asked for {other}"),
                };
                let reply = json!({ "id": id, "result": result });
                socket.send(Message::text(reply.to_string())).await.expect("a reply");
            }
            return;
        }

        let mut read = vec![0u8; 2048];
        let _ = stream.read(&mut read).await;
        let path = request.split_whitespace().nth(1).unwrap_or_default();
        let host = hosted.lock().clone().unwrap_or_else(|| address.to_string());
        let body = match path {
            "/json/version" => {
                json!({ "webSocketDebuggerUrl": format!("ws://{host}/devtools/browser/fake") })
            }
            "/json/list" => json!(state
                .pages
                .lock()
                .iter()
                .map(|(target, _)| json!({ "id": target, "type": "page" }))
                .collect::<Vec<_>>()),
            _ => json!({}),
        }
        .to_string();
        let reply = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(reply.as_bytes()).await;
    }

    fn workspace() -> (tempfile::TempDir, Store, AgentId) {
        let dir = tempfile::tempdir().expect("a directory");
        let store = Store::open(&dir.path().join("guac.db")).expect("a store");
        let agent = store
            .create_agent(&crate::domain::agent::CleanDraft {
                group_id: None,
                name: "Reader".into(),
                avatar: "avocado".into(),
                color: "#7fb069".into(),
                model: String::new(),
                subscription_model: None,
                reasoning_effort: None,
                system_prompt: String::new(),
                skills: vec![],
            })
            .expect("an agent");
        (dir, store, agent.id)
    }

    fn nothing_listens() -> Local {
        let parked = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let address = parked.local_addr().expect("an address");
        drop(parked);
        Local::new(&address.to_string()).expect("an address")
    }

    #[tokio::test]
    async fn a_browse_while_the_sidecar_is_absent_says_it_is_being_set_up_and_opens_nothing() {
        let (_dir, store, agent) = workspace();
        let err = Backend::Local(nothing_listens())
            .ensure(&store, agent, 60, false)
            .await
            .expect_err("there is no browser to open");
        assert!(matches!(err, BrowserError::SettingUp), "{err:?}");
        let said = err.for_agent();
        assert!(said.contains("being set up"), "{said}");
        assert!(
            said.contains("try again shortly"),
            "a refusal that only says no gets retried: {said}"
        );
        assert_eq!(store.get_agent(agent).unwrap().unwrap().browser_id, None);
    }

    #[tokio::test]
    async fn a_browse_while_the_sidecar_is_still_downloading_is_being_set_up_too() {
        let (_dir, store, agent) = workspace();
        let downloading = Local::new("guaca-browser-not-started.invalid:9223").expect("an address");
        let err = Backend::Local(downloading)
            .ensure(&store, agent, 60, false)
            .await
            .expect_err("a name Docker has not given out yet resolves to nothing");
        assert!(matches!(err, BrowserError::SettingUp), "{err:?}");
    }

    #[tokio::test]
    async fn the_host_says_setting_up_only_once_somebody_wants_a_browser() {
        let none = crate::secrets::Secret::from("");
        let absent = nothing_listens();
        assert_eq!(local_state(&none, None, true).await, LocalState::Absent);
        assert_eq!(local_state(&none, Some(&absent), false).await, LocalState::Idle);
        assert_eq!(local_state(&none, Some(&absent), true).await, LocalState::SettingUp);
        let sidecar = FakeSidecar::start().await;
        assert_eq!(local_state(&none, Some(&sidecar.local()), true).await, LocalState::Ready);
        assert_eq!(
            local_state(&"sk-kernel".into(), Some(&sidecar.local()), true).await,
            LocalState::Kernel
        );
    }

    #[test]
    fn a_kernel_key_wins_over_the_browser_on_this_machine() {
        let local = Local::new("guaca-browser:9223");
        assert!(matches!(
            Backend::choose(&"sk-kernel".into(), local.as_ref()),
            Some(Backend::Kernel(_))
        ));
        assert!(matches!(Backend::choose(&"  ".into(), local.as_ref()), Some(Backend::Local(_))));
        assert!(Backend::choose(&"".into(), None).is_none());
        assert!(Local::new("  ").is_none(), "a blank address is no sidecar");
    }

    #[tokio::test]
    async fn each_agent_gets_a_page_of_its_own_and_keeps_it() {
        let sidecar = FakeSidecar::start().await;
        let (_dir, store, agent) = workspace();
        let browsers = Backend::Local(sidecar.local());

        let first = browsers.ensure(&store, agent, 60, false).await.expect("a browser");
        assert!(first.fresh && !first.replaced, "{first:?}");
        assert_eq!(first.session.live_view_url, None, "there is no live view in this slice");
        assert_eq!(first.session.target.as_deref(), Some(first.session.id.as_str()));
        assert_eq!(
            store.get_agent(agent).unwrap().unwrap().browser_id.as_deref(),
            Some(first.session.id.as_str())
        );

        let again = browsers.ensure(&store, agent, 60, false).await.expect("the same browser");
        assert!(!again.fresh, "{again:?}");
        assert_eq!(again.session, first.session);
        assert_eq!(sidecar.state.contexts.load(Ordering::SeqCst), 1);

        let page = browse(&again.session, "read", &json!({})).await.expect("the page");
        assert!(page.contains("https://example.com/after"), "{page}");
        assert!(
            sidecar.state.evaluated.lock().iter().all(|(session, _)| *session == first.session.id),
            "every action ran on the agent's own page"
        );
    }

    #[tokio::test]
    async fn a_page_the_sidecar_lost_is_replaced_and_closing_one_disposes_its_context() {
        let sidecar = FakeSidecar::start().await;
        let (_dir, store, agent) = workspace();
        let browsers = Backend::Local(sidecar.local());
        let first = browsers.ensure(&store, agent, 60, false).await.expect("a browser");

        browsers.close(&first.session.id).await.expect("closed");
        assert!(sidecar.state.pages.lock().is_empty());
        assert_eq!(browsers.get(&first.session.id).await.expect("asked"), None);

        let second = browsers.ensure(&store, agent, 60, false).await.expect("another browser");
        assert!(second.fresh && second.replaced, "{second:?}");
        assert_ne!(second.session.id, first.session.id);
    }

    #[tokio::test]
    async fn a_socket_handed_back_at_another_address_is_never_followed() {
        let sidecar = FakeSidecar::start().await;
        *sidecar.socket_host.lock() = Some("127.0.0.1:9222".into());
        let (_dir, store, agent) = workspace();
        let err = Backend::Local(sidecar.local())
            .ensure(&store, agent, 60, false)
            .await
            .expect_err("the relay pointed somewhere else");
        assert!(matches!(err, BrowserError::Local(_)), "{err:?}");
        assert_eq!(sidecar.state.contexts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn closing_a_browser_on_a_sidecar_that_is_not_there_is_not_an_error() {
        Backend::Local(nothing_listens()).close("page-0").await.expect("nothing to close");
    }
}
