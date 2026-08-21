//! The local viewer for an agent's computer.
//!
//! Sandboxes are created with `allow_public_traffic: false`, so every request
//! to their public URLs must carry an `e2b-traffic-access-token` header. That
//! is exactly what an embedded viewer cannot do: an iframe cannot set a header,
//! and neither can a browser WebSocket. Leaving the traffic open instead would
//! put an agent's desktop, logged-in sessions and all, behind nothing but an
//! unguessable id.
//!
//! So the webview points at `http://127.0.0.1:{port}/{sandbox}/{sandboxPort}/…`
//! and this relays it, attaching the token on the way out.
//!
//! The relay is byte-level rather than a request parser. Only the head is read
//! and rewritten; after that both directions are spliced until one side closes.
//! That is what lets one implementation carry the HTML, the assets and noVNC's
//! RFB socket, which is a WebSocket upgrade and cannot be handled by an ordinary
//! HTTP client.
//!
//! Paths keep the `/{sandbox}/{port}` prefix because noVNC references its own
//! files relatively: from `/{sandbox}/6080/vnc.html`, `app/ui.js` resolves back
//! through the same prefix without anything having to rewrite the HTML.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

use crate::db::Store;

/// Where a sandbox's public ports live.
const UPSTREAM_SUFFIX: &str = "e2b.app";

/// Head must arrive within this. A connection that opens and says nothing is
/// either a probe or a mistake, and it should not hold a task forever.
const HEAD_LIMIT: usize = 32 * 1024;

/// The page the app points its frame at.
///
/// Served from here rather than fetched from the machine, which is what lets a
/// stylesheet reach noVNC without this relay ever having to interpret a
/// response body. `e2b.rs` builds the address from this same constant, so the
/// two cannot drift apart.
pub const VIEWER_DOCUMENT: &str = "/viewer.html";

/// That page.
///
/// noVNC is framed from the same origin, which is what lets this reach into it
/// and turn off the one piece of its chrome an operator should never see: the
/// bar it slides across the top of the picture on every connect, announcing the
/// transport. A panel opening is not news, and "unencrypted" is not even true
/// of the hop that matters: this relay reaches the machine over TLS.
///
/// A warning or an error still shows. A desktop that dies quietly is worse than
/// one that says so.
///
/// The options stay in the address so the app keeps deciding them, which is why
/// the frame's `src` is set here rather than written into the markup.
const VIEWER_PAGE: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Computer</title>
<style>
  html, body { margin: 0; height: 100%; background: #000; overflow: hidden; }
  iframe { display: block; width: 100%; height: 100%; border: 0; }
</style>
</head>
<body>
<iframe id="desktop" title="Desktop"></iframe>
<script>
  var frame = document.getElementById("desktop");
  frame.addEventListener("load", function () {
    var doc = frame.contentDocument;
    if (!doc) return;
    var quiet = doc.createElement("style");
    quiet.textContent = "#noVNC_status.noVNC_status_normal{display:none!important}";
    doc.head.appendChild(quiet);
  });
  frame.src = "./vnc.html" + location.search;
</script>
</body>
</html>
"##;

/// Starts the viewer on a loopback port chosen by the OS.
///
/// Bound to 127.0.0.1 deliberately: this holds tokens that reach an agent's
/// machine, and nothing outside this computer has any business reaching it.
pub async fn start(store: Store) -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();

    let tls = Arc::new(connector());

    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                continue;
            };
            let store = store.clone();
            let tls = tls.clone();
            tokio::spawn(async move {
                if let Err(err) = serve(client, store, tls).await {
                    tracing::debug!(%err, "computer viewer connection ended");
                }
            });
        }
    });

    tracing::info!(port, "computer viewer listening");
    Ok(port)
}

fn connector() -> TlsConnector {
    let roots = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let config = ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
    TlsConnector::from(Arc::new(config))
}

async fn serve(
    mut client: TcpStream,
    store: Store,
    tls: Arc<TlsConnector>,
) -> Result<(), std::io::Error> {
    let head = read_head(&mut client).await?;
    let Some(request) = Request::parse(&head) else {
        return refuse(&mut client, "400 Bad Request", "Not a computer address.").await;
    };

    // The token is looked up here rather than travelling in the URL, so it
    // never reaches the webview and never appears in a page's address.
    let token = match store.sandbox_traffic_token(&request.sandbox) {
        Ok(Some(token)) if !token.is_empty() => token,
        _ => {
            return refuse(
                &mut client,
                "404 Not Found",
                "No computer is registered at that address.",
            )
            .await
        }
    };

    // This app's own page rather than anything on the machine, so it is answered
    // here. After the token lookup, not before: an address with no machine
    // behind it should say so rather than paint a frame that never connects.
    if request.wants_viewer() {
        return answer(&mut client, VIEWER_PAGE).await;
    }

    let host = format!("{}-{}.{UPSTREAM_SUFFIX}", request.port, request.sandbox);
    let upstream = TcpStream::connect((host.as_str(), 443)).await?;
    let name = ServerName::try_from(host.clone())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad sandbox host"))?;
    let mut upstream = tls.connect(name, upstream).await?;

    upstream.write_all(request.rewritten(&host, &token).as_bytes()).await?;
    upstream.write_all(&request.body).await?;
    upstream.flush().await?;

    // From here neither side is interpreted. An upgraded connection carries VNC
    // frames, and an ordinary one carries a response body; both are just bytes.
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await.map(|_| ())
}

async fn read_head(client: &mut TcpStream) -> Result<Vec<u8>, std::io::Error> {
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

async fn refuse(client: &mut TcpStream, status: &str, message: &str) -> Result<(), std::io::Error> {
    // Answered rather than dropped: an iframe given nothing shows a blank
    // rectangle, which reads the same as a computer that is simply asleep.
    let body = format!(
        "HTTP/1.1 {status}\r\ncontent-type: text/plain; charset=utf-8\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n{message}",
        message.len()
    );
    client.write_all(body.as_bytes()).await
}

async fn answer(client: &mut TcpStream, page: &str) -> Result<(), std::io::Error> {
    // Not cached: the page ships with the app, and a stale one from an older
    // build would frame the desktop with last version's rules.
    let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\n\
         content-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n",
        page.len()
    );
    client.write_all(head.as_bytes()).await?;
    client.write_all(page.as_bytes()).await
}

/// The part of a request this needs to understand: which sandbox, which port,
/// and the head to pass on with two lines changed.
#[derive(Debug, PartialEq)]
struct Request {
    sandbox: String,
    port: u16,
    /// Everything after `/{sandbox}/{port}`, including the query.
    path: String,
    method: String,
    /// Header lines, minus the ones this rewrites.
    headers: Vec<String>,
    /// A WebSocket upgrade, which must keep its connection open. Everything
    /// else is answered one request per connection.
    upgrade: bool,
    body: Vec<u8>,
}

impl Request {
    /// Whether this asks for the viewer's own page rather than for something on
    /// the machine.
    fn wants_viewer(&self) -> bool {
        let path = self.path.split('?').next().unwrap_or_default();
        path == VIEWER_DOCUMENT
    }

    fn parse(head: &[u8]) -> Option<Self> {
        let text = String::from_utf8_lossy(head);
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        let mut lines = head.split("\r\n");

        let mut start = lines.next()?.split(' ');
        let method = start.next()?.to_string();
        let target = start.next()?;

        let mut parts = target.trim_start_matches('/').splitn(3, '/');
        let sandbox = parts.next().filter(|s| !s.is_empty())?.to_string();
        let port: u16 = parts.next()?.parse().ok()?;
        let rest = parts.next().unwrap_or("");

        let mut upgrade = false;
        let mut headers = Vec::new();
        for line in lines {
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("upgrade:") {
                upgrade = true;
            }
            // Host is replaced and the token is added, so anything already
            // claiming to be either is dropped rather than forwarded. The
            // connection headers are decided below, not by the caller.
            let rewritten = lower.starts_with("host:")
                || lower.starts_with("e2b-traffic-access-token:")
                || lower.starts_with("connection:")
                || lower.starts_with("keep-alive:")
                || lower.starts_with("proxy-connection:");
            if !rewritten {
                headers.push(line.to_string());
            }
        }

        Some(Request {
            sandbox,
            port,
            path: format!("/{rest}"),
            method,
            headers,
            upgrade,
            body: body.as_bytes().to_vec(),
        })
    }

    fn rewritten(&self, host: &str, token: &str) -> String {
        let mut out = format!("{} {} HTTP/1.1\r\n", self.method, self.path);
        out.push_str(&format!("host: {host}\r\n"));
        out.push_str(&format!("e2b-traffic-access-token: {token}\r\n"));

        // One request per connection, because only the first one on a
        // connection is ever read and rewritten here: everything after the head
        // is spliced through untouched. A browser reuses a connection for every
        // asset on a page, so keep-alive sent the second request upstream with
        // the proxy's own path still on it and noVNC's scripts never loaded.
        // An upgrade is the exception; that connection has to stay open.
        if self.upgrade {
            out.push_str("connection: Upgrade\r\n");
        } else {
            out.push_str("connection: close\r\n");
        }
        for header in &self.headers {
            if header.is_empty() {
                continue;
            }
            out.push_str(header);
            out.push_str("\r\n");
        }
        out.push_str("\r\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(target: &str, extra: &str) -> Vec<u8> {
        format!("GET {target} HTTP/1.1\r\nhost: 127.0.0.1:9\r\n{extra}\r\n").into_bytes()
    }

    #[test]
    fn an_ordinary_request_is_answered_one_per_connection() {
        // Only the first request on a connection is read and rewritten; the
        // rest of the bytes are spliced through. A browser reuses a connection
        // for every asset, so keep-alive sent later requests upstream with the
        // proxy's own path still attached and they came back as errors, which
        // is a page whose scripts never load.
        let req =
            Request::parse(&head("/sbx/6080/app/ui.js", "connection: keep-alive\r\n")).unwrap();
        assert!(!req.upgrade);
        let out = req.rewritten("6080-sbx.e2b.app", "tok");
        assert!(out.contains("connection: close\r\n"), "{out}");
        assert!(!out.to_lowercase().contains("keep-alive"), "the caller's wish is not forwarded");
    }

    #[test]
    fn an_upgrade_keeps_its_connection_open() {
        let req = Request::parse(&head(
            "/sbx/6080/websockify",
            "connection: Upgrade\r\nupgrade: websocket\r\n",
        ))
        .unwrap();
        assert!(req.upgrade);
        let out = req.rewritten("6080-sbx.e2b.app", "tok");
        assert!(out.contains("connection: Upgrade\r\n"), "{out}");
        assert!(!out.contains("connection: close"), "closing it kills the desktop's socket");
    }

    #[test]
    fn a_request_names_the_sandbox_the_port_and_the_file() {
        let req = Request::parse(&head("/sbx123/6080/app/ui.js", "\r\n")).unwrap();
        assert_eq!(req.sandbox, "sbx123");
        assert_eq!(req.port, 6080);
        assert_eq!(req.path, "/app/ui.js", "the prefix is stripped, the rest is kept");
    }

    #[test]
    fn a_query_survives_the_rewrite() {
        // noVNC passes its options in the query, so losing it silently would
        // connect the viewer in the wrong mode.
        let req =
            Request::parse(&head("/sbx/6080/vnc.html?autoconnect=1&view_only=1", "\r\n")).unwrap();
        assert_eq!(req.path, "/vnc.html?autoconnect=1&view_only=1");
    }

    #[test]
    fn the_index_of_a_port_is_a_bare_slash() {
        let req = Request::parse(&head("/sbx/6080/", "\r\n")).unwrap();
        assert_eq!(req.path, "/");
    }

    #[test]
    fn the_rewritten_head_points_at_the_sandbox_and_carries_the_token() {
        let req = Request::parse(&head("/sbx/6080/vnc.html", "upgrade: websocket\r\n")).unwrap();
        let out = req.rewritten("6080-sbx.e2b.app", "tok123");
        assert!(out.starts_with("GET /vnc.html HTTP/1.1\r\n"));
        assert!(out.contains("host: 6080-sbx.e2b.app\r\n"));
        assert!(out.contains("e2b-traffic-access-token: tok123\r\n"));
        assert!(
            out.contains("upgrade: websocket"),
            "an upgrade has to survive or the desktop never connects"
        );
        assert!(!out.contains("127.0.0.1"), "the loopback host must not be forwarded");
    }

    #[test]
    fn a_token_supplied_by_the_page_is_dropped_rather_than_forwarded() {
        // The webview never holds the real token, so anything claiming to be
        // one came from inside the frame and is not trusted.
        let req =
            Request::parse(&head("/sbx/6080/x", "e2b-traffic-access-token: forged\r\n")).unwrap();
        let out = req.rewritten("6080-sbx.e2b.app", "real");
        assert!(!out.contains("forged"));
        assert!(out.contains("real"));
    }

    #[test]
    fn the_page_the_app_frames_is_this_app_s_own() {
        // Answered here rather than fetched from the machine, which is what lets
        // noVNC's own chrome be turned off without this relay ever having to
        // edit a response body.
        assert!(Request::parse(&head("/sbx/6080/viewer.html?autoconnect=1", "\r\n"))
            .unwrap()
            .wants_viewer());
        assert!(!Request::parse(&head("/sbx/6080/vnc.html", "\r\n")).unwrap().wants_viewer());
        assert!(
            !Request::parse(&head("/sbx/6080/app/viewer.html", "\r\n")).unwrap().wants_viewer(),
            "a similar address under the machine's own tree is still the machine's"
        );
    }

    #[test]
    fn the_viewer_page_passes_its_options_on_and_silences_only_the_chatter() {
        // The app decides autoconnect, scaling and reconnection in the address,
        // so the page has to hand the query on rather than hold a copy of it.
        assert!(VIEWER_PAGE.contains(r#""./vnc.html" + location.search"#));
        // And only noVNC's normal status goes. The same bar carries the only
        // notice an operator gets that a desktop stopped answering.
        assert!(VIEWER_PAGE.contains("#noVNC_status.noVNC_status_normal{display:none!important}"));
        assert!(
            !VIEWER_PAGE.contains("#noVNC_status{"),
            "hiding the bar outright would hide the failures it also carries"
        );
    }

    #[test]
    fn anything_that_is_not_a_computer_address_is_refused() {
        assert!(Request::parse(&head("/", "\r\n")).is_none());
        assert!(Request::parse(&head("/sbx", "\r\n")).is_none());
        assert!(Request::parse(&head("/sbx/not-a-port/x", "\r\n")).is_none());
    }
}
