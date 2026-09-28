//! Where a `pi` job is paid for with Guaca's own key, without being given it.
//!
//! `pi` takes a key from its own auth file or from the environment, and either
//! is somewhere its own `bash` tool can print: a brief, a README or a page the
//! job reads can ask it to, and the key the operator pasted into Guaca's
//! settings would leave the machine in a job's transcript. So the key stays in
//! this process. A job is given a loopback address and a token of its own, and
//! the token is good for one thing, on one port, for the life of one job: a
//! chat completion, sent on to the endpoint in Guaca's settings with the real
//! key put on it here.
//!
//! What `pi` is told is a provider override from an extension, which its own
//! documentation offers for exactly this ("route requests through corporate
//! proxies or API gateways"). Against OpenRouter the override replaces only the
//! address and the key of pi's own `openrouter` provider, so its catalog still
//! knows each model's context, its thinking and its price, and the cost a job
//! reports is pi's own arithmetic, as it is on any other account. Anywhere else
//! there is no catalog to keep, and the job gets a provider of one model.
//!
//! Every call is metered on its way back: the `usage` a completion reports is
//! recorded against the job, in the same table Guaca's own turns are, because
//! the key is Guaca's and the operator is owed the same account of what it
//! spent. Metering reads the stream and never changes it.
//!
//! Loopback, like the other three origins this app serves, and for the same
//! reason: nothing on another machine has any business here. The token is
//! still checked, because anything on this machine could otherwise spend the
//! key, and a token that outlived its job would be a second copy of the key
//! with a longer life than anybody chose.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// How long a request head may be. pi's is a few hundred bytes.
const HEAD_LIMIT: usize = 16 * 1024;

/// How long a request body may be. A long session with pictures in it runs to
/// megabytes, and a refusal here is a job that stops mid-work.
const BODY_LIMIT: usize = 64 * 1024 * 1024;

/// The one path a job may ask for, under the `/v1` its base URL ends in.
const COMPLETIONS: &str = "/v1/chat/completions";

/// Where lent requests go, and what they are sent with. Read from the
/// settings when a job starts, so a key changed between jobs is the next
/// job's key.
#[derive(Clone)]
pub struct Upstream {
    /// An OpenAI-compatible base, `https://openrouter.ai/api/v1` by default.
    pub base_url: String,
    pub api_key: String,
    /// OpenRouter attributes requests by these two, as it does Guaca's own.
    pub referer: String,
    pub title: String,
}

impl Upstream {
    /// Whether this is OpenRouter, whose models pi's catalog already knows.
    pub fn is_openrouter(&self) -> bool {
        reqwest::Url::parse(&self.base_url)
            .ok()
            .and_then(|url| url.host_str().map(|host| host == "openrouter.ai"))
            .unwrap_or(false)
    }
}

/// What one relayed call reported it cost, as the endpoint said it.
#[derive(Debug, Clone, PartialEq)]
pub struct Spent {
    pub model: String,
    pub prompt: u32,
    pub completion: u32,
    /// OpenRouter prices each call; an endpoint that does not leaves this
    /// absent, which is not the same as free.
    pub cost: Option<f64>,
}

/// Where a lease's spend is recorded.
pub type Meter = Arc<dyn Fn(Spent) + Send + Sync>;

/// The relay. One per runtime, listening from the first job that needs it.
#[derive(Clone)]
pub struct Relay {
    inner: Arc<Inner>,
}

struct Inner {
    port: tokio::sync::OnceCell<u16>,
    leases: Mutex<HashMap<String, (Upstream, Option<Meter>)>>,
    client: reqwest::Client,
}

impl Default for Relay {
    fn default() -> Self {
        Self::new()
    }
}

impl Relay {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self {
            inner: Arc::new(Inner {
                port: tokio::sync::OnceCell::new(),
                leases: Mutex::new(HashMap::new()),
                client,
            }),
        }
    }

    /// A token for one job, good until the lease is dropped. Each call it
    /// makes is reported to `meter`.
    pub async fn lend(&self, upstream: Upstream, meter: Option<Meter>) -> std::io::Result<Lease> {
        let port = *self.inner.port.get_or_try_init(|| listen(self.clone())).await?;
        let token = uuid::Uuid::new_v4().simple().to_string();
        let openrouter = upstream.is_openrouter();
        self.inner.leases.lock().insert(token.clone(), (upstream, meter));
        tracing::info!(port, openrouter, "lent Guaca's key to a coding job");
        Ok(Lease { relay: self.clone(), token, port, openrouter })
    }

    fn upstream(&self, token: &str) -> Option<(Upstream, Option<Meter>)> {
        self.inner.leases.lock().get(token).cloned()
    }

    /// How many jobs hold a token now. What a test reads to know a lease ended.
    pub fn outstanding(&self) -> usize {
        self.inner.leases.lock().len()
    }
}

/// One job's use of the key. Dropped with the job, which is what ends it.
pub struct Lease {
    relay: Relay,
    token: String,
    port: u16,
    openrouter: bool,
}

impl Lease {
    /// What the job is told its provider lives at.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    /// What the job presents as its key. Worthless off this machine, and
    /// worthless here once the job has ended.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Whether the job keeps pi's own `openrouter` provider, only readdressed.
    pub fn openrouter(&self) -> bool {
        self.openrouter
    }

    /// One that holds nothing, for a test of what a job is told.
    #[cfg(test)]
    pub(crate) fn fixed(port: u16, token: &str, openrouter: bool) -> Lease {
        Lease { relay: Relay::new(), token: token.into(), port, openrouter }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.relay.inner.leases.lock().remove(&self.token);
    }
}

async fn listen(relay: Relay) -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                continue;
            };
            let relay = relay.clone();
            tokio::spawn(async move {
                if let Err(err) = serve(client, relay).await {
                    tracing::debug!(%err, "key relay connection ended");
                }
            });
        }
    });
    tracing::info!(port, "key relay listening");
    Ok(port)
}

/// What a request asked for, once its head has been read.
#[derive(Debug, PartialEq)]
struct Asked {
    method: String,
    path: String,
    token: Option<String>,
    length: Option<usize>,
    chunked: bool,
}

fn asked(head: &[u8]) -> Option<Asked> {
    let text = String::from_utf8_lossy(head);
    let mut lines = text.split("\r\n");
    let mut start = lines.next()?.split(' ');
    let method = start.next()?.to_string();
    let path = start.next()?.split('?').next()?.to_string();
    let (mut token, mut length, mut chunked) = (None, None, false);
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "authorization" => {
                token = value
                    .strip_prefix("Bearer ")
                    .or_else(|| value.strip_prefix("bearer "))
                    .map(|token| token.trim().to_string());
            }
            "content-length" => length = value.parse().ok(),
            "transfer-encoding" => chunked = value.to_ascii_lowercase().contains("chunked"),
            _ => {}
        }
    }
    Some(Asked { method, path, token, length, chunked })
}

async fn serve(mut client: TcpStream, relay: Relay) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > HEAD_LIMIT || client.read(&mut byte).await? == 0 {
            return refuse(&mut client, "400 Bad Request", "the request head did not end").await;
        }
        head.push(byte[0]);
    }
    let Some(asked) = asked(&head) else {
        return refuse(&mut client, "400 Bad Request", "that is not an HTTP request").await;
    };
    if asked.method != "POST" || asked.path != COMPLETIONS {
        return refuse(
            &mut client,
            "404 Not Found",
            "Guaca lends its key for chat completions and nothing else",
        )
        .await;
    }
    // Checked before the body is read, so a request without a live job's
    // token costs nothing but its head.
    let Some((upstream, meter)) = asked.token.as_deref().and_then(|token| relay.upstream(token))
    else {
        return refuse(
            &mut client,
            "401 Unauthorized",
            "this token is not a running coding job's. It ends with the job that was given it",
        )
        .await;
    };
    if asked.chunked {
        return refuse(&mut client, "411 Length Required", "send the body with a content-length")
            .await;
    }
    let length = asked.length.unwrap_or(0);
    if length > BODY_LIMIT {
        return refuse(&mut client, "413 Payload Too Large", "the request is larger than 64 MB")
            .await;
    }
    let mut body = vec![0u8; length];
    client.read_exact(&mut body).await?;
    let asked_for = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|request| request["model"].as_str().map(str::to_string))
        .unwrap_or_default();

    let url = format!("{}/chat/completions", upstream.base_url.trim_end_matches('/'));
    let sent = relay
        .inner
        .client
        .post(&url)
        .bearer_auth(upstream.api_key.trim())
        .header("content-type", "application/json")
        .header("HTTP-Referer", &upstream.referer)
        .header("X-Title", &upstream.title)
        .body(body)
        .send()
        .await;
    let mut answer = match sent {
        Ok(answer) => answer,
        Err(err) => {
            tracing::warn!(%err, "the endpoint in Guaca's settings did not answer a coding job");
            let why = format!("Guaca could not reach {}: {err}", upstream.base_url);
            return refuse(&mut client, "502 Bad Gateway", &why).await;
        }
    };

    let status = answer.status();
    let kind = answer
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    // Streamed as it arrives and delimited by the connection closing, which
    // needs no length and no chunk framing of its own: the answer is an event
    // stream a model is still writing.
    let head = format!(
        "HTTP/1.1 {} {}\r\ncontent-type: {kind}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    );
    client.write_all(head.as_bytes()).await?;
    let mut relayed = 0usize;
    let mut metering = Metering::default();
    loop {
        match answer.chunk().await {
            Ok(Some(chunk)) => {
                relayed += chunk.len();
                metering.feed(&chunk);
                client.write_all(&chunk).await?;
            }
            Ok(None) => break,
            Err(err) => {
                tracing::warn!(%err, "a relayed answer ended early");
                break;
            }
        }
    }
    tracing::debug!(status = status.as_u16(), relayed, "relayed a coding job's model call");
    if let (Some(meter), Some(spent)) = (meter, metering.finish(&asked_for)) {
        meter(spent);
    }
    client.shutdown().await
}

/// What a relayed answer said it cost, read as it passes.
///
/// A stream reports usage in its last event and a plain answer in its body,
/// so both are read: lines as they complete, and the whole answer at the end
/// if no line carried it. Bounded, because this is accounting and a long
/// answer must not be held twice in memory for it.
#[derive(Default)]
struct Metering {
    line: Vec<u8>,
    whole: Vec<u8>,
    found: Option<(Option<String>, serde_json::Value)>,
}

/// How much of a plain answer is kept to read its usage from.
const METERED: usize = 1024 * 1024;

impl Metering {
    fn feed(&mut self, chunk: &[u8]) {
        if self.whole.len() < METERED {
            self.whole.extend_from_slice(&chunk[..chunk.len().min(METERED - self.whole.len())]);
        }
        for &byte in chunk {
            if byte != b'\n' {
                if self.line.len() < METERED {
                    self.line.push(byte);
                }
                continue;
            }
            let line = std::mem::take(&mut self.line);
            let text = String::from_utf8_lossy(&line);
            if let Some(event) = text.trim().strip_prefix("data:") {
                self.read(event.trim());
            }
        }
    }

    fn read(&mut self, json: &str) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else { return };
        if value["usage"].is_object() {
            let model = value["model"].as_str().map(str::to_string);
            self.found = Some((model, value["usage"].clone()));
        }
    }

    fn finish(mut self, asked_for: &str) -> Option<Spent> {
        if self.found.is_none() {
            let whole = String::from_utf8_lossy(&std::mem::take(&mut self.whole)).into_owned();
            self.read(&whole);
        }
        let (model, usage) = self.found?;
        let count = |field: &str| usage[field].as_u64().unwrap_or(0).min(u32::MAX as u64) as u32;
        let spent = Spent {
            model: model.filter(|model| !model.is_empty()).unwrap_or_else(|| asked_for.to_string()),
            prompt: count("prompt_tokens"),
            completion: count("completion_tokens"),
            cost: usage["cost"].as_f64(),
        };
        (spent.prompt > 0 || spent.completion > 0).then_some(spent)
    }
}

/// An answer in the shape an OpenAI client reads an error from, so pi says
/// what went wrong in its own words rather than failing to parse it.
async fn refuse(client: &mut TcpStream, status: &str, why: &str) -> std::io::Result<()> {
    let body = serde_json::json!({"error": {"message": why}}).to_string();
    let head = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    client.write_all(head.as_bytes()).await?;
    client.write_all(body.as_bytes()).await?;
    client.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_head_is_read_for_what_the_relay_decides_on() {
        let head = b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer abc\r\nContent-Length: 12\r\n\r\n";
        assert_eq!(
            asked(head),
            Some(Asked {
                method: "POST".into(),
                path: COMPLETIONS.into(),
                token: Some("abc".into()),
                length: Some(12),
                chunked: false,
            })
        );
        let chunked = b"POST /v1/chat/completions HTTP/1.1\r\ntransfer-encoding: chunked\r\n\r\n";
        assert!(asked(chunked).unwrap().chunked);
    }

    #[test]
    fn a_streamed_answer_is_metered_from_its_last_usage_event_split_anywhere() {
        let stream = "data: {\"model\":\"qwen/qwen3-coder\",\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n\
                      data: {\"model\":\"qwen/qwen3-coder\",\"choices\":[],\"usage\":{\"prompt_tokens\":120,\"completion_tokens\":7,\"cost\":0.0003}}\n\n\
                      data: [DONE]\n\n";
        for cut in [1, 7, 40, stream.len()] {
            let mut metering = Metering::default();
            for piece in stream.as_bytes().chunks(cut) {
                metering.feed(piece);
            }
            assert_eq!(
                metering.finish("asked"),
                Some(Spent {
                    model: "qwen/qwen3-coder".into(),
                    prompt: 120,
                    completion: 7,
                    cost: Some(0.0003)
                }),
                "cut every {cut} bytes"
            );
        }
    }

    #[test]
    fn a_plain_answer_is_metered_from_its_body_and_an_unpriced_one_has_no_cost() {
        let mut metering = Metering::default();
        metering.feed(br#"{"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":2}}"#);
        assert_eq!(
            metering.finish("local/model"),
            Some(Spent { model: "local/model".into(), prompt: 5, completion: 2, cost: None })
        );
        // An answer with no usage in it is not a call that cost nothing.
        let mut silent = Metering::default();
        silent.feed(b"data: {\"choices\":[]}\n\ndata: [DONE]\n\n");
        assert_eq!(silent.finish("m"), None);
    }

    #[test]
    fn openrouter_is_known_by_its_host_and_nothing_else() {
        let upstream = |base: &str| Upstream {
            base_url: base.into(),
            api_key: String::new(),
            referer: String::new(),
            title: String::new(),
        };
        assert!(upstream("https://openrouter.ai/api/v1").is_openrouter());
        assert!(!upstream("https://openrouter.ai.example.com/v1").is_openrouter());
        assert!(!upstream("http://127.0.0.1:1234/v1").is_openrouter());
    }
}
