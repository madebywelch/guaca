use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

use crate::db::Store;

const UPSTREAM_SUFFIX: &str = "e2b.app";

const HEAD_LIMIT: usize = 32 * 1024;

pub const VIEWER_DOCUMENT: &str = "/viewer.html";

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
    let body = format!(
        "HTTP/1.1 {status}\r\ncontent-type: text/plain; charset=utf-8\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n{message}",
        message.len()
    );
    client.write_all(body.as_bytes()).await
}

async fn answer(client: &mut TcpStream, page: &str) -> Result<(), std::io::Error> {
    let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\n\
         content-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n",
        page.len()
    );
    client.write_all(head.as_bytes()).await?;
    client.write_all(page.as_bytes()).await
}

#[derive(Debug, PartialEq)]
struct Request {
    sandbox: String,
    port: u16,
    path: String,
    method: String,
    headers: Vec<String>,
    upgrade: bool,
    body: Vec<u8>,
}

impl Request {
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
        let req =
            Request::parse(&head("/sbx/6080/x", "e2b-traffic-access-token: forged\r\n")).unwrap();
        let out = req.rewritten("6080-sbx.e2b.app", "real");
        assert!(!out.contains("forged"));
        assert!(out.contains("real"));
    }

    #[test]
    fn the_page_the_app_frames_is_this_app_s_own() {
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
        assert!(VIEWER_PAGE.contains(r#""./vnc.html" + location.search"#));
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
