//! The DevTools protocol: asking a page instead of looking at it.
//!
//! Chrome already knows where every element is, what it says and what it does.
//! A screenshot is a guess at all three, so where a remote interface is
//! available this is the exact way to use a page, and the numbers a model is
//! given refer to elements rather than to positions.
//!
//! Nothing here knows about Kernel, and that is deliberate: the protocol is
//! Chrome's, and a client tied to one provider's control plane would have to be
//! written again for the next one. `kernel.rs` supplies a socket address and
//! turns what comes back into something a model reads.
//!
//! ## Two scopes on one socket
//!
//! The address a provider hands out is the *browser*, not a page. Browser-level
//! methods (`Target.*`, `Storage.getCookies`) are sent as they are; everything
//! that acts on a page has to carry the `sessionId` of an attached target.
//! Getting that wrong is not an error: `Runtime.evaluate` without a session
//! evaluates in the browser's own context, where there is no `document`, and
//! the reply is a perfectly well-formed "undefined".
//!
//! ## A connection per action
//!
//! Each action opens a socket, does one thing and closes it. It costs a
//! handshake, and it buys statelessness: an agent's turn can be minutes apart
//! from the next, the browser sleeps in between, and a socket held across that
//! is a socket that is dead in a way nothing notices until an action hangs on
//! it. What has to persist across actions is the element numbering, and that
//! lives on the page rather than here.

use serde_json::{json, Value};

use crate::domain::signin::CookieMark;

/// How long any one call may take.
///
/// A page that will not answer must fail rather than hold the agent's turn
/// open. Generous, because a navigation on a slow site legitimately takes this
/// long, and a timeout that fires early is indistinguishable to a model from a
/// site that is broken.
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// How long the handshake may take. Short, because there is nothing to wait
/// for: either the address is live or the browser has gone.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum CdpError {
    #[error("could not reach the browser ({0})")]
    Transport(String),
    #[error("the browser has no page open")]
    NoPage,
    /// What the page itself said. Passed through rather than summarized: a
    /// thrown exception's message is usually the most useful sentence available
    /// and the model is the one who has to act on it.
    #[error("{0}")]
    Page(String),
    /// The page this socket attached to is not there any more.
    ///
    /// Separate from `Page` because it is the one protocol error that is about
    /// this client rather than about the web: a navigation can replace a target,
    /// which invalidates the session and fails every later call on it. Chrome's
    /// own wording for that is "Inspected target navigated or closed", which
    /// reads to a model like the site broke. It reached one that way and went
    /// off to work the same page through screenshots instead.
    ///
    /// Says what to do, because a message that only says no gets retried
    /// verbatim or abandoned for another tool.
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

/// One attached page, for the length of one action.
pub struct Page {
    socket: Socket,
    next_id: u64,
    /// The attached target. Every method that touches the page carries it.
    session: String,
}

impl Page {
    /// Connects to a browser and attaches to the page an agent is working on.
    ///
    /// The last page target rather than the first, which is the tab most
    /// recently opened and so the one a click that opened a tab left in front.
    /// A browser with no page at all gets one: a fresh session has only the
    /// start page, and refusing here would mean an agent's first action always
    /// failing.
    pub async fn attach(ws_url: &str) -> Result<Self, CdpError> {
        let (socket, _) =
            tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(ws_url))
                .await
                .map_err(|_| CdpError::TimedOut)?
                .map_err(|e| CdpError::Transport(e.to_string()))?;

        let mut page = Self { socket, next_id: 0, session: String::new() };

        let targets = page.browser_call("Target.getTargets", json!({})).await?;
        let target = targets["targetInfos"]
            .as_array()
            .into_iter()
            .flatten()
            .rfind(|info| info["type"] == "page")
            .and_then(|info| info["targetId"].as_str().map(str::to_string));

        let target = match target {
            Some(target) => target,
            None => page
                .browser_call("Target.createTarget", json!({ "url": "about:blank" }))
                .await?["targetId"]
                .as_str()
                .ok_or(CdpError::NoPage)?
                .to_string(),
        };

        // `flatten` is what makes one socket enough. Without it the reply is a
        // session that can only be spoken to through `Target.sendMessageToTarget`,
        // which wraps every call in a string and every reply in an event.
        let attached = page
            .browser_call("Target.attachToTarget", json!({ "targetId": target, "flatten": true }))
            .await?;
        page.session = attached["sessionId"]
            .as_str()
            .ok_or_else(|| CdpError::Protocol("attaching to the page returned no session".into()))?
            .to_string();

        Ok(page)
    }

    /// A method against the browser itself.
    async fn browser_call(&mut self, method: &str, params: Value) -> Result<Value, CdpError> {
        self.call(method, params, None).await
    }

    /// A method against the attached page.
    async fn page_call(&mut self, method: &str, params: Value) -> Result<Value, CdpError> {
        let session = self.session.clone();
        self.call(method, params, Some(&session)).await
    }

    /// Sends one call and reads until its answer arrives.
    ///
    /// Everything else on the socket is skipped, and there is a lot of it: the
    /// protocol is bidirectional, so events for domains nobody enabled and
    /// replies for other sessions arrive in the same stream. Matching on the id
    /// is the only thing that identifies an answer.
    async fn call(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value, CdpError> {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        self.next_id += 1;
        let want = self.next_id;

        let mut request = json!({ "id": want, "method": method, "params": params });
        if let Some(session) = session {
            request["sessionId"] = json!(session);
        }

        self.socket
            .send(Message::text(request.to_string()))
            .await
            .map_err(|e| CdpError::Transport(e.to_string()))?;

        let deadline = tokio::time::Instant::now() + CALL_TIMEOUT;
        loop {
            let frame = tokio::time::timeout_at(deadline, self.socket.next())
                .await
                .map_err(|_| CdpError::TimedOut)?
                .ok_or_else(|| CdpError::Transport("the browser closed the connection".into()))?
                .map_err(|e| CdpError::Transport(e.to_string()))?;

            // Pings and binary frames are not answers. Tungstenite keeps the
            // connection alive on its own; nothing here has to.
            let Message::Text(text) = frame else {
                continue;
            };
            let Ok(message) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if message["id"].as_u64() != Some(want) {
                continue;
            }
            if let Some(error) = message["error"]["message"].as_str() {
                return Err(protocol_error(error));
            }
            return Ok(message["result"].clone());
        }
    }

    /// Runs an expression in the page and hands back its value.
    ///
    /// `awaitPromise` so an expression can wait for something, which is how
    /// every settle below is written. A thrown exception is an error rather
    /// than a null result: the two are indistinguishable in the reply
    /// otherwise, and a model told "null" for a page that threw goes looking
    /// for the missing element instead of reading the message.
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
            // The useful sentence is nested. The top level is always "Uncaught".
            let described = details["exception"]["description"]
                .as_str()
                .or_else(|| details["exception"]["value"].as_str())
                .or_else(|| details["text"].as_str())
                .unwrap_or("the page threw an error");
            return Err(CdpError::Page(described.chars().take(400).collect()));
        }

        Ok(result["result"]["value"].clone())
    }

    /// Waits, in the page, for a moment.
    ///
    /// In the page rather than here so it costs one round trip instead of two,
    /// and so a page that has already gone reports that rather than sleeping
    /// first.
    pub async fn settle(&mut self, ms: u32) -> Result<(), CdpError> {
        self.evaluate(&format!("new Promise(r => setTimeout(() => r(1), {ms}))")).await?;
        Ok(())
    }

    pub async fn navigate(&mut self, url: &str) -> Result<(), CdpError> {
        let result = self.page_call("Page.navigate", json!({ "url": url })).await?;
        // A navigation that Chrome refuses comes back with an error text and a
        // 200-shaped reply, so it has to be read out rather than assumed.
        if let Some(why) = result["errorText"].as_str() {
            return Err(CdpError::Page(format!("that page would not load ({why})")));
        }
        Ok(())
    }

    /// Types into whatever is focused, as input rather than as an assignment.
    ///
    /// `Input.insertText` rather than setting `.value`, which is what this used
    /// to do and what quietly fails on every framework that tracks its own
    /// state: the property changed, React's copy did not, and the next render
    /// put the old text back. This goes in as an input event, which is what a
    /// page listens for.
    pub async fn insert_text(&mut self, text: &str) -> Result<(), CdpError> {
        self.page_call("Input.insertText", json!({ "text": text })).await?;
        Ok(())
    }

    /// Presses Enter as a real key.
    ///
    /// Three events, because a page may listen for any of them, and plenty
    /// listen for the keystroke rather than for a form submission.
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

    /// Every cookie the browser holds, as names and flags only.
    ///
    /// The one place in this file that sees a session token, and it drops it
    /// here. `CookieMark` has no field a value could arrive in, so the boundary
    /// is enforced by the type rather than by remembering: everything past this
    /// line is safe to log, store and reason about.
    ///
    /// Sent with the page's session even though it is a browser-wide question,
    /// because that is the scope the method is reachable in on every build. It
    /// still answers for the whole browsing context rather than for one tab.
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

    /// The hosts this tab has actually been to.
    ///
    /// The weaker half of sign-in detection needs to know a site was visited
    /// rather than merely setting a cookie from inside somebody else's page,
    /// and on a machine that came from Chrome's own history file. A hosted
    /// browser has no such file to read, so this is the current tab's
    /// navigation list: less than a history, and honest about being less. What
    /// it costs is a few layer-two guesses, which is the layer that was already
    /// hedged.
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

    /// Where the page is now.
    ///
    /// Read back rather than assumed, because a click that navigates lands
    /// somewhere nobody named, and this is the value the guard on acting in the
    /// operator's name is decided from.
    pub async fn url(&mut self) -> Result<String, CdpError> {
        Ok(self.evaluate("location.href").await?.as_str().unwrap_or_default().to_string())
    }
}

/// What an error reply from the protocol means.
///
/// One distinction, and it is the difference between "the web did something"
/// and "we lost our handle on it". Chrome has several wordings for the second,
/// depending on whether the target, the session or the execution context is the
/// thing that went away, and they all mean the same thing to a caller: attach
/// again and ask again. Everything else is the page's own answer and is passed
/// through, which is what `Page` is for.
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

/// The host part of a URL, or nothing if it has none.
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
        // The trick that makes one host look like another. The host is what
        // follows the last `@`, never what precedes it.
        assert_eq!(host_of("https://mail.google.com@evil.com/"), Some("evil.com".into()));
        assert_eq!(host_of("about:blank"), None);
        assert_eq!(host_of("chrome://newtab"), None);
        assert_eq!(host_of(""), None);
    }

    #[test]
    fn a_page_that_went_away_is_told_apart_from_one_that_threw() {
        // Chrome's own wordings for the same fact, all of which used to reach a
        // model verbatim as though the site had failed.
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
            // The part that matters. A refusal that only says no gets retried
            // word for word, or the tool gets abandoned for a screenshot.
            assert!(err.to_string().contains("Read the page again"), "{err}");
        }
    }

    #[test]
    fn what_the_page_itself_said_is_still_passed_through() {
        let err = protocol_error("Uncaught TypeError: x is not a function");
        assert!(matches!(err, CdpError::Page(_)), "{err:?}");
        assert_eq!(err.to_string(), "Uncaught TypeError: x is not a function");
    }
}
