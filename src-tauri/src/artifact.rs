//! Where a page an agent wrote is allowed to run.
//!
//! A chart spec is drawn by Guaca and needs nowhere to run. A page is
//! different: an agent asked for a diagram, a mock-up or a small thing to
//! click has written markup and script, and the only honest way to show it is
//! to run it. That cannot happen in the app's own document. The webview's
//! content policy is `script-src 'self'` and it must stay that way, and a
//! frame pointed at `srcdoc:`, `blob:` or `about:blank` inherits that policy
//! from whoever framed it, so the page would draw and its script would
//! silently never run. An empty rectangle that passes every test is the worst
//! failure this app can ship.
//!
//! So a page gets an origin of its own. This is a loopback HTTP server, bound
//! on a port the OS picks, that serves exactly one kind of thing: a document
//! the renderer registered a moment earlier, by the SHA-256 of its own bytes.
//! `http://127.0.0.1:{port}` is already in the app's `frame-src`, because the
//! computer viewer needed it first.
//!
//! It is deliberately not part of `proxy.rs`. That relay carries the tokens
//! that reach a running machine, and its request parser is where the sandbox
//! those tokens belong to is decided. Putting a second, unrelated route in
//! front of that is a branch nobody wants to reason about at three in the
//! morning. Two small servers with one job each is the cheaper arrangement.
//!
//! # What the page can and cannot do
//!
//! Everything served here carries [`ARTIFACT_CSP`], and it is the whole
//! argument for allowing this at all. A model's page is the least trustworthy
//! content in the app: it was written by something that may have read a hostile
//! web page earlier in the same turn. It may compute and it may draw, and it
//! may not reach anybody:
//!
//! - `default-src 'none'`: no fetch, no XHR, no websocket, no font, no
//!   stylesheet and no image from anywhere. An `<img src="https://…/?data=">`
//!   is the cheapest exfiltration there is and it is the first thing this
//!   closes.
//! - `script-src 'unsafe-inline'`: its own inline script runs, and nothing
//!   loaded from anywhere else does.
//! - `img-src data:` and `font-src data:`: it can draw a picture it generated
//!   itself, which is what a chart library written inline does.
//! - `form-action 'none'`, `frame-ancestors 'self'` and `sandbox`: it cannot
//!   post anywhere, cannot be framed by anything but this app, and gets an
//!   opaque origin with no same-origin access, so it cannot read the document
//!   that framed it.
//!
//! `allow-scripts` without `allow-same-origin` is what makes the last one true,
//! and the two must never be granted together: that combination lets the page
//! remove its own sandbox attribute and reload itself out of the box.
//!
//! # It may hand one value back, and Guaca is what carries it
//!
//! A page that can be worked and cannot answer is a dead end. The operator
//! picks one plan out of four, drags a range, ticks six of nine rows, and none
//! of it can reach the agent that drew the page: they are left retyping in the
//! composer what they just expressed by clicking. So [`BRIDGE`] defines
//! `guaca.answer(value)`, and it is not a hole in the paragraph above.
//!
//! It reaches no network. It posts to the window that framed it, which is the
//! one channel an opaque origin has and the one the height reporter has always
//! used, and what happens next is the app's decision rather than the page's:
//! the renderer draws the value in Guaca's own chrome, below the frame, and the
//! operator presses the button. The page fills a form in. A person sends it.
//!
//! That ordering is the safety argument and it is not ceremony. A transcript
//! re-frames a page whenever it draws one, so a page that could send by itself
//! would send again every time it was scrolled past, and every send is a turn
//! the operator did not ask for and does pay for. It is also the rule this app
//! applies everywhere a model's words would go out under the operator's name:
//! shown before they go, drawn as text, never as markup.
//!
//! JSON text rather than a structured-cloned value, because the string is what
//! is shown, what is capped and what is sent. A value that would not survive
//! `JSON.stringify` then fails inside the page, which is where the page can see
//! it happen, instead of arriving as something the app has to decide about.
//!
//! Nothing here is persisted. The document itself already lives in the message
//! that carried it, which is the record; this holds a copy only while a
//! transcript is drawing one, and a restart re-registers whatever is on screen.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// What a registered page is served under.
///
/// Every directive matters; the module comment says why each one is there.
/// `sandbox` is repeated in the header as well as on the frame because the
/// frame's attribute is set by the renderer and this is not: a page reached
/// directly, by an operator who copied the address, gets the same treatment.
pub const ARTIFACT_CSP: &str = "default-src 'none'; \
     script-src 'unsafe-inline'; \
     style-src 'unsafe-inline'; \
     img-src data:; \
     font-src data:; \
     form-action 'none'; \
     base-uri 'none'; \
     frame-ancestors 'self' tauri://localhost http://tauri.localhost https://tauri.localhost http://localhost:1420 http://127.0.0.1:1420; \
     sandbox allow-scripts";

/// How many documents are kept at once.
///
/// A transcript scrolls past a lot of pages and each one is held whole. This is
/// comfortably more than fit on a screen and far less than a long channel's
/// worth, and the oldest is dropped rather than the store being allowed to
/// grow: the renderer registers again when it draws again, so an eviction
/// costs one IPC call and nothing else.
const KEPT: usize = 24;

/// A document that is too big to be a figure is too big to be worth framing.
///
/// Also the ceiling on what one of these can cost in memory: `KEPT` times this.
const MOST_BYTES: usize = 512 * 1024;

/// Request head must arrive within this. A connection that opens and says
/// nothing is a probe or a mistake, and should not hold a task forever.
const HEAD_LIMIT: usize = 8 * 1024;

/// The pages the renderer currently has on screen.
#[derive(Clone, Default)]
pub struct Artifacts {
    inner: Arc<Mutex<Kept>>,
}

#[derive(Default)]
struct Kept {
    pages: HashMap<String, String>,
    /// Oldest first, so eviction is a pop from the front.
    order: Vec<String>,
}

/// Why a page was not taken.
#[derive(Debug, PartialEq)]
pub enum ArtifactError {
    Empty,
    TooBig { bytes: usize },
}

impl std::fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArtifactError::Empty => write!(f, "there is no document here to show"),
            ArtifactError::TooBig { bytes } => write!(
                f,
                "this page is {}KB, and {}KB is the most Guaca will frame. Attach it as a file \
                 instead.",
                bytes / 1024,
                MOST_BYTES / 1024
            ),
        }
    }
}

impl Artifacts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes a page and hands back the id it is served under.
    ///
    /// Addressed by the digest of its own bytes, so registering the same
    /// document twice is the same id and a transcript redrawn is not a leak.
    pub fn keep(&self, html: &str) -> Result<String, ArtifactError> {
        if html.trim().is_empty() {
            return Err(ArtifactError::Empty);
        }
        if html.len() > MOST_BYTES {
            return Err(ArtifactError::TooBig { bytes: html.len() });
        }

        let id = format!("{:x}", Sha256::digest(html.as_bytes()));
        let mut kept = self.inner.lock().expect("artifact store poisoned");

        // Moved to the back rather than left where it was: a page still being
        // looked at should not be evicted because it was registered first.
        kept.order.retain(|held| held != &id);
        kept.order.push(id.clone());
        kept.pages.insert(id.clone(), html.to_string());

        while kept.order.len() > KEPT {
            let oldest = kept.order.remove(0);
            kept.pages.remove(&oldest);
        }

        Ok(id)
    }

    fn get(&self, id: &str) -> Option<String> {
        self.inner.lock().expect("artifact store poisoned").pages.get(id).cloned()
    }
}

/// Starts the server and answers with the port it landed on.
///
/// Loopback, like the computer viewer, and for a plainer reason: nothing
/// outside this machine has any business reading what an agent drew.
pub async fn start(artifacts: Artifacts) -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();

    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                continue;
            };
            let artifacts = artifacts.clone();
            tokio::spawn(async move {
                if let Err(err) = serve(client, artifacts).await {
                    tracing::debug!(%err, "artifact connection ended");
                }
            });
        }
    });

    tracing::info!(port, "artifact viewer listening");
    Ok(port)
}

async fn serve(mut client: TcpStream, artifacts: Artifacts) -> std::io::Result<()> {
    let head = read_head(&mut client).await?;
    let Some(id) = requested(&head) else {
        return answer(&mut client, "400 Bad Request", "text/plain", "Not an artifact address.")
            .await;
    };
    // Said rather than dropped, whichever way it goes: a frame given nothing
    // draws a blank rectangle, which reads exactly like a page that rendered
    // nothing.
    let (status, content_type, body) = page_for(&artifacts, &id);
    let status = match status {
        200 => "200 OK",
        404 => "404 Not Found",
        _ => "400 Bad Request",
    };
    answer(&mut client, status, content_type, &body).await
}

/// The id a request line asks for, if it asks for one at all.
///
/// Only `GET /{hex}` counts. A path with anything else in it is not something
/// this serves, and the id is checked for shape here rather than trusted,
/// because it goes on to be a map key and a map key built from a request is
/// exactly the sort of thing that turns out to be a path later.
fn requested(head: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(head);
    let mut start = text.split("\r\n").next()?.split(' ');
    if start.next()? != "GET" {
        return None;
    }
    let path = start.next()?.split('?').next()?;
    let id = path.strip_prefix('/')?;
    let ok = id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit());
    ok.then(|| id.to_ascii_lowercase())
}

/// The document, plus the two things Guaca adds to it.
///
/// Prepended, not appended: a document with an unclosed tag swallows anything
/// after it, and a model's page is exactly where an unclosed tag lives. Ahead
/// of `<!doctype>` it is still parsed and run, because the parser treats a
/// stray script before the doctype as content and starts the document anyway.
/// It is also what makes `guaca.answer` defined before the page's own script
/// runs, rather than a function the page has to wait for.
fn wrap(page: &str) -> String {
    format!("{BRIDGE}{page}")
}

/// Everything the page can say to the window that framed it.
///
/// Two messages, on the one channel an opaque origin has, and `postMessage` to
/// `"*"` for both: the parent's origin is not something this document is
/// allowed to know, and it does not need to be, because the parent identifies
/// this frame by the window the message came from, which is the check that
/// actually holds.
///
/// **The height**, because a frame on another origin cannot be measured from
/// outside: a page that says nothing about its own size is drawn at whatever
/// was guessed, which for a one-line diagram is a tall gray box and for a long
/// one is a nested scrollbar. Reported on every change rather than once on
/// load, since a page that draws after a timer, or grows when something in it
/// is clicked, has a different answer a second later.
///
/// **The answer**, which is a value and never a send. `guaca.answer` posts and
/// nothing else happens: whether that value ever becomes a message is the
/// operator's decision, taken in Guaca's own chrome. See the note on this
/// module for why that ordering is the whole of what makes this safe. It is
/// serialized here rather than cloned, so a value that will not survive
/// `JSON.stringify` fails in the page, where the page is told about it by the
/// `false` coming back, instead of in the app.
const BRIDGE: &str = r#"<script>
(function () {
  var last = 0;
  function tell() {
    var height = Math.max(
      document.documentElement ? document.documentElement.scrollHeight : 0,
      document.body ? document.body.scrollHeight : 0
    );
    if (height && height !== last) {
      last = height;
      parent.postMessage({ guaca: "artifact-height", height: height }, "*");
    }
  }
  addEventListener("load", tell);
  addEventListener("resize", tell);
  if (window.ResizeObserver) {
    addEventListener("DOMContentLoaded", function () {
      new ResizeObserver(tell).observe(document.documentElement);
    });
  }
  window.guaca = {
    answer: function (value) {
      var said;
      try {
        said = JSON.stringify(value);
      } catch (err) {
        said = null;
      }
      if (typeof said !== "string") return false;
      parent.postMessage({ guaca: "artifact-answer", value: said }, "*");
      return true;
    }
  };
})();
</script>
"#;

async fn read_head(client: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > HEAD_LIMIT {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request head too long",
            ));
        }
        if client.read(&mut byte).await? == 0 {
            break;
        }
        head.push(byte[0]);
    }
    Ok(head)
}

/// What a request for `id` is answered with: status, content type, body.
///
/// The loopback server and the daemon's route both answer from this, so a
/// page reached either way is the same page with the same refusals.
pub fn page_for(artifacts: &Artifacts, id: &str) -> (u16, &'static str, String) {
    let shaped = id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !shaped {
        return (400, "text/plain", "Not an artifact address.".into());
    }
    match artifacts.get(&id.to_ascii_lowercase()) {
        Some(page) => (200, "text/html; charset=utf-8", wrap(&page)),
        None => (
            404,
            "text/plain",
            "This page is no longer held. Scroll back to it to draw it again.".into(),
        ),
    }
}

/// Every header a page is served under, whichever server serves it.
pub fn response_headers(content_type: &'static str) -> [(&'static str, &'static str); 5] {
    [
        ("content-type", content_type),
        ("content-security-policy", ARTIFACT_CSP),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cache-control", "no-store"),
    ]
}

async fn answer(
    client: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    // The policy rides on every answer, refusals included. A page served
    // without it is a page with the app's own policy inherited or none at all,
    // depending on how it was reached, and neither is the one that was argued
    // for above.
    let head = format!(
        "HTTP/1.1 {status}\r\n\
         content-type: {content_type}\r\n\
         content-security-policy: {ARTIFACT_CSP}\r\n\
         x-content-type-options: nosniff\r\n\
         referrer-policy: no-referrer\r\n\
         cache-control: no-store\r\n\
         content-length: {}\r\n\
         connection: close\r\n\r\n",
        body.len()
    );
    client.write_all(head.as_bytes()).await?;
    client.write_all(body.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "<!doctype html><p>hello</p>";

    #[test]
    fn addresses_a_page_by_its_own_bytes() {
        let artifacts = Artifacts::new();
        let once = artifacts.keep(PAGE).unwrap();
        let again = artifacts.keep(PAGE).unwrap();
        assert_eq!(once, again, "a transcript redrawn must not grow the store");
        assert_ne!(once, artifacts.keep("<!doctype html><p>other</p>").unwrap());
    }

    #[test]
    fn hands_back_what_it_was_given() {
        let artifacts = Artifacts::new();
        let id = artifacts.keep(PAGE).unwrap();
        assert_eq!(artifacts.get(&id).as_deref(), Some(PAGE));
        assert_eq!(artifacts.get("nope"), None);
    }

    #[test]
    fn drops_the_oldest_and_keeps_what_is_being_looked_at() {
        let artifacts = Artifacts::new();
        let first = artifacts.keep("<p>0</p>").unwrap();
        for at in 1..KEPT {
            artifacts.keep(&format!("<p>{at}</p>")).unwrap();
        }
        // Touched again, so it is no longer the oldest thing here.
        artifacts.keep("<p>0</p>").unwrap();
        let second = artifacts.keep("<p>new</p>").unwrap();

        assert!(artifacts.get(&first).is_some(), "a page still on screen was evicted");
        assert!(artifacts.get(&second).is_some());
        assert_eq!(artifacts.inner.lock().unwrap().pages.len(), KEPT);
    }

    #[test]
    fn refuses_a_document_too_big_to_be_a_figure() {
        let artifacts = Artifacts::new();
        let huge = "x".repeat(MOST_BYTES + 1);
        let Err(err) = artifacts.keep(&huge) else {
            panic!("expected a refusal");
        };
        // Every error the operator can hit says what to do about it.
        assert!(err.to_string().contains("Attach it as a file"), "{err}");
        assert_eq!(artifacts.keep("   "), Err(ArtifactError::Empty));
    }

    #[test]
    fn serves_only_a_digest_shaped_path() {
        let id = "a".repeat(64);
        assert_eq!(requested(format!("GET /{id} HTTP/1.1\r\n\r\n").as_bytes()), Some(id.clone()));
        assert_eq!(
            requested(format!("GET /{} HTTP/1.1\r\n\r\n", id.to_uppercase()).as_bytes()),
            Some(id.clone())
        );
        // The id becomes a map key, so its shape is checked rather than trusted.
        assert_eq!(requested(b"GET /../../etc/passwd HTTP/1.1\r\n\r\n"), None);
        assert_eq!(requested(b"GET / HTTP/1.1\r\n\r\n"), None);
        assert_eq!(requested(b"GET /short HTTP/1.1\r\n\r\n"), None);
        assert_eq!(requested(format!("POST /{id} HTTP/1.1\r\n\r\n").as_bytes()), None);
    }

    #[test]
    fn never_lets_a_page_reach_anything() {
        // The whole argument for running a model's page at all. A page written
        // by something that may have read a hostile web page an hour ago may
        // compute and may draw, and may not talk to anybody.
        assert!(ARTIFACT_CSP.contains("default-src 'none'"));
        assert!(ARTIFACT_CSP.contains("img-src data:"), "no remote image, which is the cheap leak");
        assert!(ARTIFACT_CSP.contains("form-action 'none'"));
        assert!(ARTIFACT_CSP.contains("sandbox allow-scripts"));
        // These two together let a page take its own sandbox off and reload
        // out of it, which is the one combination that must never appear.
        assert!(!ARTIFACT_CSP.contains("allow-same-origin"));
        // It has to be able to run, or none of the above bought anything.
        assert!(ARTIFACT_CSP.contains("script-src 'unsafe-inline'"));
    }

    /// One request, start to finish, over a real socket.
    ///
    /// The unit tests above check what this module believes; this checks what
    /// actually goes out on the wire, which is where a policy assembled into a
    /// header string can quietly stop being sent.
    async fn ask(port: u16, path: &str) -> String {
        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client
            .write_all(format!("GET {path} HTTP/1.1\r\nhost: localhost\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut said = Vec::new();
        client.read_to_end(&mut said).await.unwrap();
        String::from_utf8_lossy(&said).into_owned()
    }

    #[tokio::test]
    async fn serves_a_kept_page_under_the_policy() {
        let artifacts = Artifacts::new();
        let id = artifacts.keep(PAGE).unwrap();
        let port = start(artifacts).await.unwrap();

        let said = ask(port, &format!("/{id}")).await;
        assert!(said.starts_with("HTTP/1.1 200 OK"), "{said}");
        assert!(said.contains(&format!("content-security-policy: {ARTIFACT_CSP}")), "{said}");
        assert!(said.contains("<p>hello</p>"), "{said}");
        // The page has to be able to report its own height, or a frame on
        // another origin is drawn at whatever was guessed.
        assert!(said.contains("artifact-height"), "{said}");
        // And to hand a value back, or a page the operator can work is one
        // whose working reaches nobody.
        assert!(said.contains("artifact-answer"), "{said}");
    }

    #[tokio::test]
    async fn answers_an_id_it_is_not_holding_rather_than_dropping_the_connection() {
        let port = start(Artifacts::new()).await.unwrap();
        let said = ask(port, &format!("/{}", "b".repeat(64))).await;

        // A frame given nothing draws a blank rectangle, which reads exactly
        // like a page that rendered nothing.
        assert!(said.starts_with("HTTP/1.1 404"), "{said}");
        assert!(said.contains("Scroll back to it"), "{said}");
        // The refusal carries the policy too: it is reachable in a frame.
        assert!(said.contains("content-security-policy:"), "{said}");
    }

    #[test]
    fn puts_the_bridge_where_an_unclosed_tag_cannot_eat_it() {
        // A model's page is exactly where an unclosed tag lives, and anything
        // after one is swallowed by it. It is also what makes `guaca.answer`
        // defined before the page's own script asks for it.
        let wrapped = wrap("<!doctype html><p>hi");
        assert!(wrapped.starts_with("<script>"), "{wrapped}");
        assert!(wrapped.contains("artifact-height"));
        assert!(wrapped.contains("window.guaca"));
        assert!(wrapped.ends_with("<!doctype html><p>hi"));
    }

    #[test]
    fn the_answer_bridge_posts_a_value_and_never_a_message() {
        // The distinction this whole feature rests on. A page hands a value to
        // the window that framed it; whether that value ever becomes a message
        // is decided in Guaca's own chrome, by the operator, and there is
        // nothing in here that could take that step on its own.
        assert!(BRIDGE.contains(r#"parent.postMessage({ guaca: "artifact-answer""#), "{BRIDGE}");
        // Serialized in the page, so a value that cannot survive it fails
        // where the page can be told about it rather than in the app.
        assert!(BRIDGE.contains("JSON.stringify(value)"), "{BRIDGE}");
        assert!(BRIDGE.contains("return false"), "{BRIDGE}");
        // Nothing here may reach the network, which the policy already refuses
        // and which nothing in the bridge should be asking for either.
        assert!(!BRIDGE.contains("fetch("), "{BRIDGE}");
        assert!(!BRIDGE.contains("XMLHttpRequest"), "{BRIDGE}");
    }
}
