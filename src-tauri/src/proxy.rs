//! The local viewer for an agent's computer.
//!
//! The webview points at `http://127.0.0.1:{port}/{computerId}/{guestPort}/…`
//! and this relays it, asking a resolver where that port actually is and what
//! has to be added on the way out. An E2B sandbox is a TLS host that refuses
//! traffic without an `e2b-traffic-access-token` header; a local guest is plain
//! TCP to an address on this machine. The header is exactly what an embedded
//! viewer cannot supply for itself: an iframe cannot set one, and neither can a
//! browser WebSocket. Leaving the traffic open instead would put an agent's
//! desktop, logged-in sessions and all, behind nothing but an unguessable id.
//!
//! The relay is byte-level rather than a request parser. Only the head is read
//! and rewritten; after that both directions are spliced until one side closes.
//! That is what lets one implementation carry the HTML, the assets and noVNC's
//! RFB socket, which is a WebSocket upgrade and cannot be handled by an ordinary
//! HTTP client.
//!
//! Paths keep the `/{computer}/{port}` prefix because noVNC references its own
//! files relatively: from `/{computer}/6080/vnc.html`, `app/ui.js` resolves back
//! through the same prefix without anything having to rewrite the HTML.

use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

use crate::computer::provider::ViewerTarget;

/// Where an address in the URL leads. The proxy holds no state of its own: it
/// is handed a computer and a port and asks here, so nothing a provider needs
/// to reach a machine has to travel through the webview.
#[async_trait::async_trait]
pub trait ViewerResolver: Send + Sync + 'static {
    /// The upstream for `/{computer}/{port}/…`, or `None` if no computer is
    /// registered there.
    async fn viewer_target(&self, computer: &str, port: u16) -> Option<ViewerTarget>;
}

/// Head must arrive within this. A connection that opens and says nothing is
/// either a probe or a mistake, and it should not hold a task forever.
const HEAD_LIMIT: usize = 32 * 1024;

/// Starts the viewer on a loopback port chosen by the OS.
///
/// Bound to 127.0.0.1 deliberately: this carries the secrets that reach an
/// agent's machine, and nothing outside this computer has any business
/// reaching it.
pub async fn start(resolver: Arc<dyn ViewerResolver>) -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();

    let tls = Arc::new(connector());

    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                continue;
            };
            let resolver = resolver.clone();
            let tls = tls.clone();
            tokio::spawn(async move {
                if let Err(err) = serve(client, resolver, tls).await {
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
    resolver: Arc<dyn ViewerResolver>,
    tls: Arc<TlsConnector>,
) -> Result<(), std::io::Error> {
    let head = read_head(&mut client).await?;
    let Some(request) = Request::parse(&head) else {
        return refuse(&mut client, "400 Bad Request", "Not a computer address.").await;
    };

    // Asked here rather than carried in the URL, so whatever reaches the
    // machine never reaches the webview and never appears in a page's address.
    let Some(target) = resolver.viewer_target(&request.computer, request.port).await else {
        return refuse(&mut client, "404 Not Found", "No computer is registered at that address.")
            .await;
    };

    let mut upstream = TcpStream::connect((target.host.as_str(), target.port)).await?;
    let head = request.rewritten(&target);

    // The two upstreams are spliced by the same generic code rather than
    // through a boxed trait object: a relay is the one place where an extra
    // virtual call sits on every byte in both directions.
    if target.tls {
        let name = ServerName::try_from(target.host.clone()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad upstream host")
        })?;
        let mut upstream = tls.connect(name, upstream).await?;
        relay(&mut client, &mut upstream, &head, &request.body).await
    } else {
        relay(&mut client, &mut upstream, &head, &request.body).await
    }
}

/// Sends the rewritten head and then stops interpreting anything. An upgraded
/// connection carries VNC frames, and an ordinary one carries a response body;
/// both are just bytes.
async fn relay<U>(
    client: &mut TcpStream,
    upstream: &mut U,
    head: &str,
    body: &[u8],
) -> Result<(), std::io::Error>
where
    U: AsyncRead + AsyncWrite + Unpin,
{
    upstream.write_all(head.as_bytes()).await?;
    upstream.write_all(body).await?;
    upstream.flush().await?;
    tokio::io::copy_bidirectional(client, upstream).await.map(|_| ())
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

/// The part of a request this needs to understand: which computer, which port,
/// and the head to pass on with its first lines replaced.
#[derive(Debug, PartialEq)]
struct Request {
    computer: String,
    port: u16,
    /// Everything after `/{computer}/{port}`, including the query.
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
    fn parse(head: &[u8]) -> Option<Self> {
        let text = String::from_utf8_lossy(head);
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        let mut lines = head.split("\r\n");

        let mut start = lines.next()?.split(' ');
        let method = start.next()?.to_string();
        let target = start.next()?;

        let mut parts = target.trim_start_matches('/').splitn(3, '/');
        let computer = parts.next().filter(|s| !s.is_empty())?.to_string();
        let port: u16 = parts.next()?.parse().ok()?;
        let rest = parts.next().unwrap_or("");

        let mut upgrade = false;
        let mut headers = Vec::new();
        for line in lines {
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("upgrade:") {
                upgrade = true;
            }
            // Host is replaced, so anything already claiming to be one is
            // dropped rather than forwarded, and the connection headers are
            // decided below rather than by the caller. The traffic token is in
            // this fixed list too, because it is the one header known to be
            // sensitive before any target has been resolved; the rest of what a
            // target supplies is dropped in `rewritten`.
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
            computer,
            port,
            path: format!("/{rest}"),
            method,
            headers,
            upgrade,
            body: body.as_bytes().to_vec(),
        })
    }

    fn rewritten(&self, target: &ViewerTarget) -> String {
        let mut out = format!("{} {} HTTP/1.1\r\n", self.method, self.path);
        // A default port is left off the host line: an upstream that routes by
        // name sees the name it was issued under, not `name:443`.
        let default_port = target.port == if target.tls { 443 } else { 80 };
        if default_port {
            out.push_str(&format!("host: {}\r\n", target.host));
        } else {
            out.push_str(&format!("host: {}:{}\r\n", target.host, target.port));
        }
        for (name, value) in &target.headers {
            out.push_str(&format!("{name}: {value}\r\n"));
        }

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
            // Whatever the target supplies has already been written, and the
            // page cannot be trusted with a second copy: a header a provider
            // uses to admit traffic is one a frame would like to forge.
            let name = header.split_once(':').map_or(header.as_str(), |(name, _)| name).trim();
            if target.headers.iter().any(|(supplied, _)| supplied.eq_ignore_ascii_case(name)) {
                continue;
            }
            out.push_str(header);
            out.push_str("\r\n");
        }
        out.push_str("\r\n");
        out
    }
}

/// Until the computer manager owns every machine, the only viewer target this
/// app has is an E2B sandbox and the token sits on the agent that holds it.
#[async_trait::async_trait]
impl ViewerResolver for crate::db::Store {
    async fn viewer_target(&self, computer: &str, port: u16) -> Option<ViewerTarget> {
        let token =
            self.sandbox_traffic_token(computer).ok().flatten().filter(|t| !t.is_empty())?;
        Some(ViewerTarget {
            tls: true,
            host: format!("{port}-{computer}.e2b.app"),
            port: 443,
            headers: vec![("e2b-traffic-access-token".into(), token)],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(target: &str, extra: &str) -> Vec<u8> {
        format!("GET {target} HTTP/1.1\r\nhost: 127.0.0.1:9\r\n{extra}\r\n").into_bytes()
    }

    fn e2b(host: &str, token: &str) -> ViewerTarget {
        ViewerTarget {
            tls: true,
            host: host.into(),
            port: 443,
            headers: vec![("e2b-traffic-access-token".into(), token.into())],
        }
    }

    fn local(host: &str) -> ViewerTarget {
        ViewerTarget { tls: false, host: host.into(), port: 6080, headers: vec![] }
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
        let out = req.rewritten(&e2b("6080-sbx.e2b.app", "tok"));
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
        let out = req.rewritten(&e2b("6080-sbx.e2b.app", "tok"));
        assert!(out.contains("connection: Upgrade\r\n"), "{out}");
        assert!(!out.contains("connection: close"), "closing it kills the desktop's socket");
    }

    #[test]
    fn a_request_names_the_computer_the_port_and_the_file() {
        let req = Request::parse(&head("/sbx123/6080/app/ui.js", "\r\n")).unwrap();
        assert_eq!(req.computer, "sbx123");
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
    fn the_rewritten_head_points_at_the_target_and_carries_what_it_supplies() {
        let req = Request::parse(&head("/sbx/6080/vnc.html", "upgrade: websocket\r\n")).unwrap();
        let out = req.rewritten(&e2b("6080-sbx.e2b.app", "tok123"));
        assert!(out.starts_with("GET /vnc.html HTTP/1.1\r\n"));
        assert!(out.contains("host: 6080-sbx.e2b.app\r\n"), "no port suffix on the default one");
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
        let out = req.rewritten(&e2b("6080-sbx.e2b.app", "real"));
        assert!(!out.contains("forged"));
        assert!(out.contains("real"));
    }

    #[test]
    fn a_target_with_no_headers_adds_none_and_a_forged_one_is_still_dropped() {
        // A local guest wants nothing added; the page still cannot smuggle a
        // provider header upstream by naming one.
        let req =
            Request::parse(&head("/c1/6080/x", "e2b-traffic-access-token: forged\r\n")).unwrap();
        let out = req.rewritten(&local("192.168.64.3"));
        assert!(out.contains("host: 192.168.64.3:6080\r\n"), "{out}");
        assert!(!out.contains("forged"));
        assert!(!out.contains("e2b-traffic-access-token"));
    }

    #[test]
    fn a_header_the_target_supplies_replaces_one_the_page_sent() {
        let req = Request::parse(&head("/c1/6080/x", "x-guac-viewer: forged\r\n")).unwrap();
        let out = req.rewritten(&ViewerTarget {
            tls: false,
            host: "h".into(),
            port: 1,
            headers: vec![("x-guac-viewer".into(), "real".into())],
        });
        assert_eq!(out.matches("x-guac-viewer:").count(), 1, "{out}");
        assert!(out.contains("x-guac-viewer: real"));
    }

    #[test]
    fn anything_that_is_not_a_computer_address_is_refused() {
        assert!(Request::parse(&head("/", "\r\n")).is_none());
        assert!(Request::parse(&head("/sbx", "\r\n")).is_none());
        assert!(Request::parse(&head("/sbx/not-a-port/x", "\r\n")).is_none());
    }
}
