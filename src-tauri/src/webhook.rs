//! Where an event arrives to fire a routine.
//!
//! A routine standing on `event:stripe/invoice.payment_failed` holds no slot
//! and nothing sweeps it. What fires it is a POST to this receiver at
//! `/events/stripe/invoice.payment_failed`, which hands the two identifiers
//! and the body to `Runtime::deliver_event` and answers with what became of
//! them. That is the whole of the event source: Guaca polls no service and
//! signs in to none for this, and whatever posts here is the operator's to
//! wire. A shell script on a git hook, a tunnel pointed at a vendor's webhook,
//! a `curl` in a cron line are all the same thing from this side.
//!
//! # Loopback, and a secret in a header
//!
//! Bound on `127.0.0.1` like the two viewers, and unlike them it *does*
//! something when reached: a routine fired is a turn spent and an agent acting.
//! Loopback alone does not close that. Any page open in the operator's browser
//! may POST to a loopback port cross-origin, and a body-only secret would ride
//! along in such a request unread. So the secret is a bearer token in the
//! `Authorization` header, and that placement is the mechanism rather than a
//! convention: a browser will not attach that header cross-origin without a
//! preflight, the preflight is answered here with no CORS headers at all, and
//! the browser never sends the POST. Anything on the machine that can read the
//! secret out of the config file can already read the API key beside it.
//!
//! # The address survives a restart
//!
//! The port and the secret are written into the config the first time the
//! receiver comes up, and read back on every launch after. The operator wires
//! something to an address exactly once, and a receiver that took a fresh port
//! from the OS each morning would break that wiring silently each morning. If
//! the recorded port is taken, the receiver falls back to whatever is free,
//! says so in the log, and writes the new one down; the routine's panel shows
//! whichever address is current.
//!
//! # What the body is
//!
//! Data. It is handed to the routine's part untouched and the plain-text
//! projection fences it under a line that says so, because the instruction
//! carries the operator's authority and the body carries none: it is whatever
//! the service put on the wire. Capped at [`MOST_BODY_BYTES`], which is more
//! than a vendor's event and less than what a turn should spend reading.

use std::io;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::config::WebhookConfig;
use crate::domain::routine::EventTrigger;
use crate::runtime::{EventDelivery, Runtime};

/// The one route this answers. The rest of the path is the event.
pub const ROUTE: &str = "/events/";

/// The most a body may be. Everything under it reaches the agent whole.
pub const MOST_BODY_BYTES: usize = 64 * 1024;

/// A request head longer than this is not one somebody wrote by hand.
const HEAD_LIMIT: usize = 8 * 1024;

/// How long one connection may take from open to answered.
///
/// A client that opens and stalls would otherwise hold a task for good; a
/// client that is merely slow has far longer than it needs.
const PATIENCE: Duration = Duration::from_secs(10);

/// Gives the config a secret if it has none. Answers whether anything changed,
/// so the caller can write the file once rather than on every launch.
///
/// The port is deliberately not decided here: what port the receiver can bind
/// is only known once it has tried, and `start` is what tries.
pub fn prepare(config: &mut WebhookConfig) -> bool {
    if !config.secret.trim().is_empty() {
        return false;
    }
    config.secret = crate::oauth::secret();
    true
}

/// Starts the receiver and answers with the port it is listening on.
///
/// `wanted` is the port recorded from last time, or zero for the first time.
/// A recorded port that cannot be bound is not fatal: the receiver takes a
/// free one and the caller writes it down, because a receiver that is not up
/// is a routine that never fires with nothing on screen to say so.
pub async fn start(runtime: Runtime, wanted: u16, secret: String) -> io::Result<u16> {
    let listener = match TcpListener::bind(("127.0.0.1", wanted)).await {
        Ok(listener) => listener,
        Err(err) if wanted != 0 => {
            tracing::warn!(
                port = wanted,
                %err,
                "the event receiver's recorded port is taken; taking another, which changes \
                 the address anything posting events was given"
            );
            TcpListener::bind(("127.0.0.1", 0)).await?
        }
        Err(err) => return Err(err),
    };
    let port = listener.local_addr()?.port();

    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                continue;
            };
            let runtime = runtime.clone();
            let secret = secret.clone();
            tokio::spawn(async move {
                match tokio::time::timeout(PATIENCE, serve(client, runtime, &secret)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => tracing::debug!(%err, "event connection ended"),
                    Err(_) => tracing::debug!("event connection took too long and was dropped"),
                }
            });
        }
    });

    tracing::info!(port, "event receiver listening");
    Ok(port)
}

/// Why a request was not an event.
///
/// Every one is answered in a sentence that says what to send instead, because
/// the reader is somebody with a terminal open wondering why nothing happened.
#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    NotPost,
    NotAnEventAddress,
    NotAnEvent(String),
    Unauthorized,
    TooBig(usize),
    Malformed(&'static str),
}

impl Refusal {
    fn status(&self) -> &'static str {
        match self {
            Refusal::NotPost => "405 Method Not Allowed",
            Refusal::NotAnEventAddress => "404 Not Found",
            Refusal::NotAnEvent(_) | Refusal::Malformed(_) => "400 Bad Request",
            Refusal::Unauthorized => "401 Unauthorized",
            Refusal::TooBig(_) => "413 Content Too Large",
        }
    }

    fn explain(&self) -> String {
        match self {
            Refusal::NotPost => "An event is a POST. Nothing else is answered here.".to_string(),
            Refusal::NotAnEventAddress => format!(
                "Not an event address. Post to {ROUTE}<service>/<topic>, the two names the \
                 routine in Guaca was given."
            ),
            Refusal::NotAnEvent(rest) => format!(
                "{rest:?} is not an event. It is a service, a slash, then that service's own \
                 name for what happened, with no spaces in either: stripe/invoice.paid."
            ),
            Refusal::Unauthorized => "Send Guaca's webhook secret as `Authorization: Bearer \
                                      <secret>`. It is shown on any routine that waits on an \
                                      event."
                .to_string(),
            Refusal::TooBig(bytes) => format!(
                "The body is {}KB and {}KB is the most a routine is handed. Post a reference \
                 the agent can follow rather than the document.",
                bytes / 1024,
                MOST_BODY_BYTES / 1024
            ),
            Refusal::Malformed(what) => format!("Could not read the request: {what}."),
        }
    }
}

/// A request head, read and not yet judged.
///
/// Read whole before anything is refused, because the body length is needed
/// on the way to a refusal too: a client that has not finished sending its
/// body when the answer arrives sees the connection reset rather than the
/// sentence that says what it did wrong.
#[derive(Debug, PartialEq, Eq)]
struct Head {
    method: String,
    /// The path without its query. A query is not part of an event.
    path: String,
    /// The token in `Authorization: Bearer …`, when there was one.
    bearer: Option<String>,
    /// Bytes of body that follow. Zero is a body-less event.
    body_len: usize,
    chunked: bool,
    /// Whether the client is waiting to be told to send the body.
    expects_continue: bool,
}

/// Reads the request line and the four headers that matter.
///
/// Everything else in the head is ignored: this is not a web server and does
/// not want to become one.
fn read_lines(head: &[u8]) -> Result<Head, Refusal> {
    let text = String::from_utf8_lossy(head);
    let mut lines = text.split("\r\n");
    let request = lines.next().ok_or(Refusal::Malformed("no request line"))?;
    let mut words = request.split(' ');
    let method = words.next().ok_or(Refusal::Malformed("no method"))?.to_string();
    let target = words.next().ok_or(Refusal::Malformed("no path"))?;
    let path = target.split('?').next().unwrap_or(target).to_string();

    let mut bearer = None;
    let mut body_len = 0usize;
    let mut chunked = false;
    let mut expects_continue = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "authorization" => {
                bearer = value
                    .split_once(' ')
                    .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
                    .map(|(_, token)| token.trim().to_string());
            }
            "content-length" => {
                body_len = value
                    .parse()
                    .map_err(|_| Refusal::Malformed("content-length is not a number"))?;
            }
            "transfer-encoding" => chunked = true,
            "expect" => expects_continue = value.eq_ignore_ascii_case("100-continue"),
            _ => {}
        }
    }
    Ok(Head { method, path, bearer, body_len, chunked, expects_continue })
}

/// Whether the head is an event, and which.
///
/// The token is checked before the address is read past its route, so a
/// request with the wrong secret learns nothing about which events exist.
fn accept(head: &Head, secret: &str) -> Result<EventTrigger, Refusal> {
    if head.method != "POST" {
        // Answered before the path so a stray GET at the right address says
        // what the address is for rather than that it does not exist.
        return Err(if head.path.starts_with(ROUTE) {
            Refusal::NotPost
        } else {
            Refusal::NotAnEventAddress
        });
    }
    let rest = head.path.strip_prefix(ROUTE).ok_or(Refusal::NotAnEventAddress)?;
    if !head.bearer.as_deref().is_some_and(|token| same_secret(token, secret)) {
        return Err(Refusal::Unauthorized);
    }
    if head.chunked {
        // Chunked bodies are a second parser for one client that cannot say
        // its length up front, and every client that posts an event can.
        return Err(Refusal::Malformed("send the body with a content-length rather than chunked"));
    }
    if head.body_len > MOST_BODY_BYTES {
        return Err(Refusal::TooBig(head.body_len));
    }
    EventTrigger::parse(rest).ok_or_else(|| Refusal::NotAnEvent(rest.to_string()))
}

/// Whether two secrets match, taking the same time whether or not they do.
///
/// A loopback receiver is not where a timing attack is mounted, and this
/// costs four lines to not have to argue that.
fn same_secret(given: &str, secret: &str) -> bool {
    if given.len() != secret.len() {
        return false;
    }
    given.bytes().zip(secret.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

async fn serve(mut client: TcpStream, runtime: Runtime, secret: &str) -> io::Result<()> {
    let raw = read_head(&mut client).await?;
    let head = match read_lines(&raw) {
        Ok(head) => head,
        Err(refusal) => return refuse(&mut client, &refusal).await,
    };

    let event = match accept(&head, secret) {
        Ok(event) => event,
        Err(refusal) => {
            // The body is drained before the refusal is written, when there is
            // one on its way and it is not absurd. Answered mid-send, the
            // client reads a reset instead of the sentence; a client waiting
            // on `100-continue` has sent nothing and will not now.
            if !head.expects_continue && head.body_len <= MOST_BODY_BYTES {
                let mut unread = vec![0u8; head.body_len];
                client.read_exact(&mut unread).await?;
            }
            return refuse(&mut client, &refusal).await;
        }
    };

    if head.expects_continue {
        // curl sends this for anything over a kilobyte and waits a second
        // before giving up on being told to go ahead.
        client.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await?;
    }
    let mut body = vec![0u8; head.body_len];
    client.read_exact(&mut body).await?;
    let payload = String::from_utf8_lossy(&body);
    let payload = (!payload.trim().is_empty()).then(|| payload.into_owned());

    match runtime.deliver_event(&event, payload) {
        Ok(delivery) => answer(&mut client, &delivery, &event).await,
        Err(err) => {
            tracing::error!(%err, "an event could not be delivered");
            respond(
                &mut client,
                "500 Internal Server Error",
                &serde_json::json!({ "error": format!("Guaca could not read its routines: {err}") }),
            )
            .await
        }
    }
}

/// What the poster is told.
///
/// A 404 for an event nobody stands on, and it is the one answer here that
/// matters most: a 200 would tell a script its wiring works while the routine
/// it was meant for sits under a different spelling.
async fn answer(
    client: &mut TcpStream,
    delivery: &EventDelivery,
    event: &EventTrigger,
) -> io::Result<()> {
    if delivery.listening == 0 {
        return respond(
            client,
            "404 Not Found",
            &serde_json::json!({
                "error": format!(
                    "No active routine waits on {}/{}. Add one in Guaca, under the agent that \
                     should handle it, and check the spelling of both names.",
                    event.service, event.topic
                ),
                "listening": 0,
            }),
        )
        .await;
    }
    respond(client, "200 OK", &serde_json::to_value(delivery).unwrap_or_default()).await
}

async fn refuse(client: &mut TcpStream, refusal: &Refusal) -> io::Result<()> {
    respond(client, refusal.status(), &serde_json::json!({ "error": refusal.explain() })).await
}

async fn respond(client: &mut TcpStream, status: &str, body: &serde_json::Value) -> io::Result<()> {
    let body = body.to_string();
    // No CORS header, ever. Its absence is what keeps a browser from posting
    // here; see the module comment.
    let head = format!(
        "HTTP/1.1 {status}\r\n\
         content-type: application/json\r\n\
         cache-control: no-store\r\n\
         content-length: {}\r\n\
         connection: close\r\n\r\n",
        body.len()
    );
    client.write_all(head.as_bytes()).await?;
    client.write_all(body.as_bytes()).await
}

async fn read_head(client: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > HEAD_LIMIT {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "request head too long"));
        }
        if client.read(&mut byte).await? == 0 {
            break;
        }
        head.push(byte[0]);
    }
    Ok(head)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "s3cr3t-s3cr3t-s3cr3t-s3cr3t-s3cr3t-s3cr3t-";

    fn head_bytes(method: &str, path: &str, headers: &[(&str, &str)]) -> Vec<u8> {
        let mut out = format!("{method} {path} HTTP/1.1\r\nhost: 127.0.0.1\r\n");
        for (name, value) in headers {
            out.push_str(&format!("{name}: {value}\r\n"));
        }
        out.push_str("\r\n");
        out.into_bytes()
    }

    /// Both steps, as `serve` takes them.
    fn parse(raw: &[u8]) -> Result<(Head, EventTrigger), Refusal> {
        let head = read_lines(raw)?;
        let event = accept(&head, SECRET)?;
        Ok((head, event))
    }

    fn bearer() -> (&'static str, String) {
        ("Authorization", format!("Bearer {SECRET}"))
    }

    #[test]
    fn a_post_with_the_secret_is_the_event_it_names() {
        let (name, value) = bearer();
        let (head, event) = parse(&head_bytes(
            "POST",
            "/events/Stripe/invoice.paid",
            &[(name, &value), ("Content-Length", "12")],
        ))
        .unwrap();
        // The service is lowered on the way in, exactly as the stored trigger
        // was, or a routine on `stripe` would never match a post to `Stripe`.
        assert_eq!(event.service, "stripe");
        assert_eq!(event.topic, "invoice.paid");
        assert_eq!(head.body_len, 12);
        assert!(!head.expects_continue);
    }

    #[test]
    fn the_secret_is_checked_before_the_event_is_read() {
        // A wrong token learns nothing about which addresses exist.
        let bad = ("Authorization", "Bearer nope");
        let refused = parse(&head_bytes("POST", "/events/not an event", &[bad]));
        assert_eq!(refused.unwrap_err(), Refusal::Unauthorized);

        let missing = parse(&head_bytes("POST", "/events/stripe/invoice.paid", &[]));
        assert_eq!(missing.unwrap_err(), Refusal::Unauthorized);

        // Right length, wrong bytes: the comparison is not a prefix check.
        let near = format!("Bearer {}", SECRET.replace('3', "4"));
        let refused =
            parse(&head_bytes("POST", "/events/stripe/invoice.paid", &[("Authorization", &near)]));
        assert_eq!(refused.unwrap_err(), Refusal::Unauthorized);
    }

    #[test]
    fn a_refusal_says_what_to_send_instead() {
        let (name, value) = bearer();
        let cases = [
            (head_bytes("GET", "/events/stripe/invoice.paid", &[]), Refusal::NotPost),
            (head_bytes("GET", "/", &[]), Refusal::NotAnEventAddress),
            (head_bytes("POST", "/hooks/stripe", &[(name, &value)]), Refusal::NotAnEventAddress),
            (
                head_bytes("POST", "/events/stripe", &[(name, &value)]),
                Refusal::NotAnEvent("stripe".into()),
            ),
            (
                head_bytes(
                    "POST",
                    "/events/stripe/invoice.paid",
                    &[(name, &value), ("Content-Length", "99999999")],
                ),
                Refusal::TooBig(99_999_999),
            ),
            (
                head_bytes(
                    "POST",
                    "/events/stripe/invoice.paid",
                    &[(name, &value), ("Transfer-Encoding", "chunked")],
                ),
                Refusal::Malformed("send the body with a content-length rather than chunked"),
            ),
        ];
        for (request, expected) in cases {
            let got = parse(&request).unwrap_err();
            assert_eq!(got, expected);
            // Every refusal is read by somebody at a terminal, and has to hand
            // them the next thing to try.
            let said = got.explain();
            assert!(
                said.contains("Post to")
                    || said.contains("is a POST")
                    || said.contains("Send Guaca's")
                    || said.contains("stripe/invoice.paid")
                    || said.contains("Post a reference")
                    || said.contains("content-length"),
                "{said}"
            );
        }
    }

    #[test]
    fn a_client_waiting_to_be_told_to_continue_is_noticed() {
        let (name, value) = bearer();
        let (head, event) = parse(&head_bytes(
            "POST",
            "/events/github/pull_request.opened?delivery=1",
            &[(name, &value), ("Expect", "100-continue"), ("content-length", "2048")],
        ))
        .unwrap();
        assert!(head.expects_continue);
        // A query string is not part of the event.
        assert_eq!(event.topic, "pull_request.opened");
    }

    #[test]
    fn a_refused_request_still_says_how_much_body_is_on_its_way() {
        // What `serve` drains before writing the refusal, so the client reads
        // the sentence rather than a reset.
        let head = read_lines(&head_bytes(
            "POST",
            "/events/stripe/invoice.paid",
            &[("Content-Length", "40")],
        ))
        .unwrap();
        assert_eq!(accept(&head, SECRET).unwrap_err(), Refusal::Unauthorized);
        assert_eq!(head.body_len, 40);
    }

    #[test]
    fn preparing_writes_a_secret_once() {
        let mut config = WebhookConfig::default();
        assert!(prepare(&mut config), "an empty config needs a secret");
        let first = config.secret.clone();
        assert!(first.len() >= 40, "256 bits of base64url: {first:?}");
        assert!(!prepare(&mut config), "a second launch keeps the secret it wrote");
        assert_eq!(config.secret, first);
    }
}
