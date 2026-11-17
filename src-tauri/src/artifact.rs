use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub const ARTIFACT_CSP: &str = "default-src 'none'; \
     script-src 'unsafe-inline'; \
     style-src 'unsafe-inline'; \
     img-src data:; \
     font-src data:; \
     form-action 'none'; \
     base-uri 'none'; \
     frame-ancestors 'self' tauri://localhost http://tauri.localhost https://tauri.localhost http://localhost:1420 http://127.0.0.1:1420; \
     sandbox allow-scripts";

const KEPT: usize = 24;

const MOST_BYTES: usize = crate::domain::artifact::MAX_PAGE;

const HEAD_LIMIT: usize = 8 * 1024;

#[derive(Clone, Default)]
pub struct Artifacts {
    inner: Arc<Mutex<Kept>>,
}

#[derive(Default)]
struct Kept {
    pages: HashMap<String, String>,
    order: Vec<String>,
}

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

    pub fn keep(&self, html: &str) -> Result<String, ArtifactError> {
        if html.trim().is_empty() {
            return Err(ArtifactError::Empty);
        }
        if html.len() > MOST_BYTES {
            return Err(ArtifactError::TooBig { bytes: html.len() });
        }

        let id = format!("{:x}", Sha256::digest(html.as_bytes()));
        let mut kept = self.inner.lock().expect("artifact store poisoned");

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
    let (status, content_type, body) = page_for(&artifacts, &id);
    let status = match status {
        200 => "200 OK",
        404 => "404 Not Found",
        _ => "400 Bad Request",
    };
    answer(&mut client, status, content_type, &body).await
}

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

fn wrap(page: &str) -> String {
    format!("{BRIDGE}{page}")
}

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
  function post(kind, value) {
    var said;
    try {
      said = JSON.stringify(value);
    } catch (err) {
      said = null;
    }
    if (typeof said !== "string") return false;
    parent.postMessage({ guaca: kind, value: said }, "*");
    return true;
  }
  var latest = null;
  var waiting = [];
  var watching = [];
  addEventListener("message", function (event) {
    if (event.source !== parent) return;
    var said = event.data;
    if (!said || said.guaca !== "artifact-data") return;
    latest = said.data;
    var ready = waiting;
    waiting = [];
    for (var i = 0; i < ready.length; i++) ready[i](latest);
    for (var j = 0; j < watching.length; j++) watching[j](latest);
  });
  window.guaca = {
    answer: function (value) {
      return post("artifact-answer", value);
    },
    send: function (value) {
      return post("artifact-send", value);
    },
    data: function () {
      if (latest !== null) return Promise.resolve(latest);
      return new Promise(function (resolve) {
        waiting.push(resolve);
      });
    },
    onData: function (watch) {
      watching.push(watch);
      if (latest !== null) watch(latest);
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
        assert_eq!(requested(b"GET /../../etc/passwd HTTP/1.1\r\n\r\n"), None);
        assert_eq!(requested(b"GET / HTTP/1.1\r\n\r\n"), None);
        assert_eq!(requested(b"GET /short HTTP/1.1\r\n\r\n"), None);
        assert_eq!(requested(format!("POST /{id} HTTP/1.1\r\n\r\n").as_bytes()), None);
    }

    #[test]
    fn never_lets_a_page_reach_anything() {
        assert!(ARTIFACT_CSP.contains("default-src 'none'"));
        assert!(ARTIFACT_CSP.contains("img-src data:"), "no remote image, which is the cheap leak");
        assert!(ARTIFACT_CSP.contains("form-action 'none'"));
        assert!(ARTIFACT_CSP.contains("sandbox allow-scripts"));
        assert!(!ARTIFACT_CSP.contains("allow-same-origin"));
        assert!(ARTIFACT_CSP.contains("script-src 'unsafe-inline'"));
    }

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
        assert!(said.contains("artifact-height"), "{said}");
        assert!(said.contains("artifact-answer"), "{said}");
    }

    #[tokio::test]
    async fn answers_an_id_it_is_not_holding_rather_than_dropping_the_connection() {
        let port = start(Artifacts::new()).await.unwrap();
        let said = ask(port, &format!("/{}", "b".repeat(64))).await;

        assert!(said.starts_with("HTTP/1.1 404"), "{said}");
        assert!(said.contains("Scroll back to it"), "{said}");
        assert!(said.contains("content-security-policy:"), "{said}");
    }

    #[test]
    fn puts_the_bridge_where_an_unclosed_tag_cannot_eat_it() {
        let wrapped = wrap("<!doctype html><p>hi");
        assert!(wrapped.starts_with("<script>"), "{wrapped}");
        assert!(wrapped.contains("artifact-height"));
        assert!(wrapped.contains("window.guaca"));
        assert!(wrapped.ends_with("<!doctype html><p>hi"));
    }

    #[test]
    fn data_reaches_a_page_only_from_the_window_that_framed_it() {
        assert!(BRIDGE.contains("if (event.source !== parent) return;"), "{BRIDGE}");
        assert!(BRIDGE.contains(r#"said.guaca !== "artifact-data""#), "{BRIDGE}");
        assert!(BRIDGE.contains(r#"return post("artifact-send", value);"#), "{BRIDGE}");
    }

    #[test]
    fn the_answer_bridge_posts_a_value_and_never_a_message() {
        assert!(BRIDGE.contains(r#"return post("artifact-answer", value);"#), "{BRIDGE}");
        assert!(BRIDGE.contains("parent.postMessage({ guaca: kind, value: said }"), "{BRIDGE}");
        assert!(BRIDGE.contains("JSON.stringify(value)"), "{BRIDGE}");
        assert!(BRIDGE.contains("return false"), "{BRIDGE}");
        assert!(!BRIDGE.contains("fetch("), "{BRIDGE}");
        assert!(!BRIDGE.contains("XMLHttpRequest"), "{BRIDGE}");
    }
}
