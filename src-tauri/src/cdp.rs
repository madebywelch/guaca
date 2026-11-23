use serde::Serialize;
use serde_json::{json, Value};

use crate::domain::signin::CookieMark;

const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum CdpError {
    #[error("could not reach the browser ({0})")]
    Transport(String),
    #[error("the browser has no page open")]
    NoPage,
    #[error("{0}")]
    Page(String),
    #[error(
        "the page moved on while that was in flight, so this action lost track of it. Read the \
         page again to get its current state and fresh element numbers, then carry on from there."
    )]
    TargetGone,
    #[error("the browser took too long to answer")]
    TimedOut,
    #[error("the browser replied in a form this build does not understand: {0}")]
    Protocol(String),
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Dialog {
    pub kind: String,
    pub message: String,
    pub accepted: bool,
}

const DIALOG_MESSAGE_CHARS: usize = 200;

pub struct Page {
    socket: Socket,
    next_id: u64,
    session: String,
    answered: Vec<Dialog>,
}

impl Page {
    pub async fn attach_to(ws_url: &str, target: Option<&str>) -> Result<Self, CdpError> {
        let mut page = Self::connect(ws_url).await?;

        let target = match target {
            Some(target) => target.to_string(),
            None => page.newest_page().await?,
        };

        let attached = page
            .browser_call("Target.attachToTarget", json!({ "targetId": target, "flatten": true }))
            .await?;
        page.session = attached["sessionId"]
            .as_str()
            .ok_or_else(|| CdpError::Protocol("attaching to the page returned no session".into()))?
            .to_string();

        page.page_call("Page.enable", json!({})).await?;

        Ok(page)
    }

    async fn connect(ws_url: &str) -> Result<Self, CdpError> {
        let (socket, _) =
            tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(ws_url))
                .await
                .map_err(|_| CdpError::TimedOut)?
                .map_err(|e| CdpError::Transport(e.to_string()))?;
        Ok(Self { socket, next_id: 0, session: String::new(), answered: Vec::new() })
    }

    async fn newest_page(&mut self) -> Result<String, CdpError> {
        let targets = self.browser_call("Target.getTargets", json!({})).await?;
        let target = targets["targetInfos"]
            .as_array()
            .into_iter()
            .flatten()
            .rfind(|info| info["type"] == "page")
            .and_then(|info| info["targetId"].as_str().map(str::to_string));

        match target {
            Some(target) => Ok(target),
            None => Ok(self
                .browser_call("Target.createTarget", json!({ "url": "about:blank" }))
                .await?["targetId"]
                .as_str()
                .ok_or(CdpError::NoPage)?
                .to_string()),
        }
    }

    pub async fn open_alone(ws_url: &str) -> Result<String, CdpError> {
        let mut browser = Self::connect(ws_url).await?;
        let context = browser.browser_call("Target.createBrowserContext", json!({})).await?;
        let context = context["browserContextId"]
            .as_str()
            .ok_or_else(|| CdpError::Protocol("the browser made no context".into()))?
            .to_string();
        let made = browser
            .browser_call(
                "Target.createTarget",
                json!({ "url": "about:blank", "browserContextId": context }),
            )
            .await?;
        made["targetId"].as_str().map(str::to_string).ok_or(CdpError::NoPage)
    }

    pub async fn close_alone(ws_url: &str, target: &str) -> Result<(), CdpError> {
        let mut browser = Self::connect(ws_url).await?;
        let targets = browser.browser_call("Target.getTargets", json!({})).await?;
        let context = targets["targetInfos"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|info| info["targetId"] == target)
            .and_then(|info| info["browserContextId"].as_str().map(str::to_string));
        let Some(context) = context else {
            return Ok(());
        };
        browser
            .browser_call("Target.disposeBrowserContext", json!({ "browserContextId": context }))
            .await?;
        Ok(())
    }

    pub fn answered(&self) -> &[Dialog] {
        &self.answered
    }

    async fn browser_call(&mut self, method: &str, params: Value) -> Result<Value, CdpError> {
        self.call(method, params, None).await
    }

    async fn page_call(&mut self, method: &str, params: Value) -> Result<Value, CdpError> {
        let session = self.session.clone();
        self.call(method, params, Some(&session)).await
    }

    async fn call(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value, CdpError> {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::Message;

        let want = self.send(method, params, session).await?;

        let deadline = tokio::time::Instant::now() + CALL_TIMEOUT;
        loop {
            let frame = tokio::time::timeout_at(deadline, self.socket.next())
                .await
                .map_err(|_| CdpError::TimedOut)?
                .ok_or_else(|| CdpError::Transport("the browser closed the connection".into()))?
                .map_err(|e| CdpError::Transport(e.to_string()))?;

            let Message::Text(text) = frame else {
                continue;
            };
            let Ok(message) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if message["method"] == "Page.javascriptDialogOpening" {
                self.answer_dialog(&message).await?;
                continue;
            }
            if message["id"].as_u64() != Some(want) {
                continue;
            }
            if let Some(error) = message["error"]["message"].as_str() {
                return Err(protocol_error(error));
            }
            return Ok(message["result"].clone());
        }
    }

    async fn send(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<u64, CdpError> {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message;

        self.next_id += 1;
        let id = self.next_id;

        let mut request = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            request["sessionId"] = json!(session);
        }

        self.socket
            .send(Message::text(request.to_string()))
            .await
            .map_err(|e| CdpError::Transport(e.to_string()))?;
        Ok(id)
    }

    async fn answer_dialog(&mut self, event: &Value) -> Result<(), CdpError> {
        let kind = event["params"]["type"].as_str().unwrap_or_default().to_string();
        let accepted = accept(&kind);
        let session = event["sessionId"].as_str().map(str::to_string);

        self.send("Page.handleJavaScriptDialog", json!({ "accept": accepted }), session.as_deref())
            .await?;

        self.answered.push(Dialog {
            kind,
            message: event["params"]["message"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(DIALOG_MESSAGE_CHARS)
                .collect(),
            accepted,
        });
        Ok(())
    }

    pub async fn evaluate(&mut self, expression: &str) -> Result<Value, CdpError> {
        let result = self
            .page_call(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                }),
            )
            .await?;

        if let Some(details) = result.get("exceptionDetails").filter(|d| !d.is_null()) {
            let described = details["exception"]["description"]
                .as_str()
                .or_else(|| details["exception"]["value"].as_str())
                .or_else(|| details["text"].as_str())
                .unwrap_or("the page threw an error");
            return Err(CdpError::Page(described.chars().take(400).collect()));
        }

        Ok(result["result"]["value"].clone())
    }

    pub async fn settle(&mut self, ms: u32) -> Result<(), CdpError> {
        self.evaluate(&format!("new Promise(r => setTimeout(() => r(1), {ms}))")).await?;
        Ok(())
    }

    pub async fn navigate(&mut self, url: &str) -> Result<(), CdpError> {
        let result = self.page_call("Page.navigate", json!({ "url": url })).await?;
        if let Some(why) = result["errorText"].as_str() {
            return Err(CdpError::Page(format!("that page would not load ({why})")));
        }
        Ok(())
    }

    pub async fn insert_text(&mut self, text: &str) -> Result<(), CdpError> {
        self.page_call("Input.insertText", json!({ "text": text })).await?;
        Ok(())
    }

    pub async fn press_enter(&mut self) -> Result<(), CdpError> {
        for kind in ["keyDown", "char", "keyUp"] {
            self.page_call(
                "Input.dispatchKeyEvent",
                json!({
                    "type": kind,
                    "key": "Enter",
                    "code": "Enter",
                    "text": "\r",
                    "windowsVirtualKeyCode": 13,
                    "nativeVirtualKeyCode": 13,
                }),
            )
            .await?;
        }
        Ok(())
    }

    pub async fn cookies(&mut self) -> Result<Vec<CookieMark>, CdpError> {
        let result = self.page_call("Storage.getCookies", json!({})).await?;
        Ok(result["cookies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|cookie| {
                Some(CookieMark {
                    domain: cookie["domain"].as_str()?.to_string(),
                    name: cookie["name"].as_str()?.to_string(),
                    http_only: cookie["httpOnly"].as_bool().unwrap_or(false),
                    session: cookie["session"].as_bool().unwrap_or(false),
                })
            })
            .collect())
    }

    pub async fn visited(&mut self) -> Result<Vec<String>, CdpError> {
        let result = self.page_call("Page.getNavigationHistory", json!({})).await?;
        let mut hosts: Vec<String> = result["entries"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| host_of(entry["url"].as_str()?))
            .collect();
        hosts.sort();
        hosts.dedup();
        Ok(hosts)
    }

    pub async fn url(&mut self) -> Result<String, CdpError> {
        Ok(self.evaluate("location.href").await?.as_str().unwrap_or_default().to_string())
    }
}

fn accept(kind: &str) -> bool {
    kind != "prompt"
}

fn protocol_error(message: &str) -> CdpError {
    const GONE: [&str; 6] = [
        "inspected target navigated or closed",
        "session with given id not found",
        "no target with given id found",
        "target closed",
        "cannot find context with specified id",
        "execution context was destroyed",
    ];

    let lowered = message.to_ascii_lowercase();
    if GONE.iter().any(|gone| lowered.contains(gone)) {
        return CdpError::TargetGone;
    }
    CdpError::Page(message.chars().take(300).collect())
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?;
    let host = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
    (!host.is_empty() && host.contains('.')).then(|| host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_is_read_from_a_url_and_nothing_is_invented() {
        assert_eq!(host_of("https://mail.google.com/u/0"), Some("mail.google.com".into()));
        assert_eq!(host_of("https://EXAMPLE.com:8443/x?y#z"), Some("example.com".into()));
        assert_eq!(host_of("https://mail.google.com@evil.com/"), Some("evil.com".into()));
        assert_eq!(host_of("about:blank"), None);
        assert_eq!(host_of("chrome://newtab"), None);
        assert_eq!(host_of(""), None);
    }

    #[test]
    fn a_page_that_went_away_is_told_apart_from_one_that_threw() {
        for message in [
            "Inspected target navigated or closed",
            "Session with given id not found.",
            "No target with given id found",
            "Target closed",
            "Cannot find context with specified id",
            "Execution context was destroyed.",
        ] {
            let err = protocol_error(message);
            assert!(matches!(err, CdpError::TargetGone), "{message} became {err:?}");
            assert!(err.to_string().contains("Read the page again"), "{err}");
        }
    }

    #[test]
    fn a_dialog_that_asks_a_question_is_answered_and_one_that_wants_a_value_is_not() {
        assert!(accept("beforeunload"), "the agent asked for the navigation");
        assert!(accept("confirm"), "the press that raised it was already gated");
        assert!(accept("alert"), "there is no other answer");
        assert!(!accept("prompt"));
    }

    #[test]
    fn what_the_page_itself_said_is_still_passed_through() {
        let err = protocol_error("Uncaught TypeError: x is not a function");
        assert!(matches!(err, CdpError::Page(_)), "{err:?}");
        assert_eq!(err.to_string(), "Uncaught TypeError: x is not a function");
    }
}
