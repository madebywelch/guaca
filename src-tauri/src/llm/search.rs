use std::time::Duration;

use serde::Deserialize;

use crate::config::{InferenceConfig, Provider};
use crate::llm::openrouter::{LlmError, Usage};
use crate::secrets::Secret;

pub const DEFAULT_HITS: usize = 5;
pub const MOST_HITS: usize = 10;
pub const SNIPPET_CHARS: usize = 320;
pub const SEARCH_TIME: Duration = Duration::from_secs(20);

pub const BRAVE: &str = "https://api.search.brave.com/res/v1/web/search";
pub const TAVILY: &str = "https://api.tavily.com/search";
const TAVILY_PREFIX: &str = "tvly-";

pub const NO_BACKEND: &str = "This endpoint cannot search the web for agents. Add a Brave Search \
                              or Tavily key here and it can. Without one, agents can still read a \
                              page at an address they are given.";

pub const SEARCHER: &str = "You run web searches for another agent and report what the search \
                            returned. You never answer from memory and never invent an address.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Searched {
    pub hits: Vec<Hit>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Chatgpt,
    Claude,
    OpenRouter,
    Brave,
    Tavily,
}

impl Backend {
    pub fn for_turn(cfg: &InferenceConfig, key: &Secret) -> Option<Backend> {
        match cfg.provider {
            Provider::Chatgpt => Some(Backend::Chatgpt),
            Provider::Claude => Some(Backend::Claude),
            Provider::Compatible => Backend::keyed(key).or_else(|| {
                crate::llm::openrouter::is_openrouter(&cfg.base_url).then_some(Backend::OpenRouter)
            }),
        }
    }

    fn keyed(key: &Secret) -> Option<Backend> {
        let key = key.expose().trim();
        if key.is_empty() {
            None
        } else if key.starts_with(TAVILY_PREFIX) {
            Some(Backend::Tavily)
        } else {
            Some(Backend::Brave)
        }
    }

    pub fn spends_a_model_call(self) -> bool {
        matches!(self, Backend::Chatgpt | Backend::Claude | Backend::OpenRouter)
    }

    pub fn label(self) -> &'static str {
        match self {
            Backend::Chatgpt => "your ChatGPT plan",
            Backend::Claude => "the claude program",
            Backend::OpenRouter => "OpenRouter's web search",
            Backend::Brave => "Brave Search",
            Backend::Tavily => "Tavily",
        }
    }

    pub fn disclosed(self) -> String {
        match self {
            Backend::Chatgpt => "Agents search through your ChatGPT plan, so a query goes to \
                                 OpenAI and nowhere else."
                .into(),
            Backend::Claude => "Agents search through the claude program, so a query goes to \
                                Anthropic and nowhere else."
                .into(),
            Backend::OpenRouter => "Agents search through OpenRouter's web search, which bills \
                                    your OpenRouter account for each search. A Brave Search or \
                                    Tavily key here is used instead."
                .into(),
            Backend::Brave | Backend::Tavily => format!(
                "Agents search with {}, using the key here, so a query goes to {} as well as \
                 to your provider.",
                self.label(),
                self.label()
            ),
        }
    }
}

pub fn ask(query: &str, count: usize) -> String {
    format!(
        "Search the web for: {query}\n\n\
         Answer with up to {count} of the most useful results the search returned, one per line \
         and nothing else, each exactly like this:\n\
         - [Title of the page](https://the.page/address) One or two sentences of what the page \
         says that bears on the search.\n\
         List only pages the search actually returned, with their real addresses. Do not answer \
         the question yourself."
    )
}

fn web_address(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    matches!(parsed.scheme(), "http" | "https").then(|| parsed.to_string())
}

fn clipped(text: &str) -> String {
    let words = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if words.chars().count() <= SNIPPET_CHARS {
        return words;
    }
    let mut kept: String = words.chars().take(SNIPPET_CHARS).collect();
    kept.push('\u{2026}');
    kept
}

pub fn hit(title: &str, url: &str, snippet: &str) -> Option<Hit> {
    let url = web_address(url)?;
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(Hit {
        title: if title.is_empty() { url.clone() } else { title },
        url,
        snippet: clipped(snippet),
    })
}

pub fn kept(hits: impl IntoIterator<Item = Hit>, count: usize) -> Vec<Hit> {
    let mut out: Vec<Hit> = Vec::new();
    for found in hits {
        if out.len() >= count {
            break;
        }
        match out.iter_mut().find(|seen| seen.url == found.url) {
            Some(seen) if seen.snippet.is_empty() => seen.snippet = found.snippet,
            Some(_) => {}
            None => out.push(found),
        }
    }
    out
}

fn listed_line(line: &str) -> Option<Hit> {
    let open = line.find('[')?;
    let rest = &line[open + 1..];
    let close = rest.find("](")?;
    let title = &rest[..close];
    let after = &rest[close + 2..];
    let mut depth = 0usize;
    let mut end = None;
    for (at, c) in after.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => {
                end = Some(at);
                break;
            }
            ')' => depth -= 1,
            _ => {}
        }
    }
    let end = end?;
    let snippet = after[end + 1..].trim_start_matches([' ', ':', '-', '\u{2014}', '\u{2013}']);
    hit(title, &after[..end], snippet)
}

pub fn listed(text: &str) -> Vec<Hit> {
    text.lines().filter_map(listed_line).collect()
}

fn refused(status: u16, body: &str, retry_after: Option<u64>) -> LlmError {
    let message = body.trim().chars().take(400).collect::<String>();
    match status {
        401 | 403 => LlmError::Auth { status, message },
        429 => LlmError::RateLimited { message, retry_after_secs: retry_after },
        _ => LlmError::Upstream { status, message },
    }
}

fn unsent(url: &str, timeout: Duration, source: reqwest::Error) -> LlmError {
    if source.is_timeout() {
        LlmError::Timeout { secs: timeout.as_secs() }
    } else {
        LlmError::Transport { url: url.to_string(), source }
    }
}

async fn answered(
    sent: Result<reqwest::Response, reqwest::Error>,
    url: &str,
    timeout: Duration,
) -> Result<String, LlmError> {
    let response = sent.map_err(|source| unsent(url, timeout, source))?;
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let body = response.text().await.map_err(|source| unsent(url, timeout, source))?;
    if !status.is_success() {
        return Err(refused(status.as_u16(), &body, retry_after));
    }
    Ok(body)
}

#[derive(Debug, Default, Deserialize)]
struct BraveAnswer {
    #[serde(default)]
    web: BraveWeb,
}

#[derive(Debug, Default, Deserialize)]
struct BraveWeb {
    #[serde(default)]
    results: Vec<Found>,
}

#[derive(Debug, Default, Deserialize)]
struct Found {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default, alias = "content")]
    description: String,
}

#[derive(Debug, Default, Deserialize)]
struct TavilyAnswer {
    #[serde(default)]
    results: Vec<Found>,
}

fn found_hits(results: Vec<Found>, count: usize) -> Vec<Hit> {
    kept(
        results.into_iter().filter_map(|found| {
            hit(
                &crate::web::plain(&found.title),
                &found.url,
                &crate::web::plain(&found.description),
            )
        }),
        count,
    )
}

pub async fn brave(
    http: &reqwest::Client,
    endpoint: &str,
    key: &Secret,
    query: &str,
    count: usize,
) -> Result<Searched, LlmError> {
    let count_text = count.to_string();
    let sent = http
        .get(endpoint)
        .query(&[("q", query), ("count", count_text.as_str())])
        .header("Accept", "application/json")
        .header("X-Subscription-Token", key.expose().trim())
        .timeout(SEARCH_TIME)
        .send()
        .await;
    let body = answered(sent, endpoint, SEARCH_TIME).await?;
    let answer: BraveAnswer = serde_json::from_str(&body)
        .map_err(|err| LlmError::Decode(format!("Brave Search answer: {err}")))?;
    Ok(Searched { hits: found_hits(answer.web.results, count), usage: None })
}

pub async fn tavily(
    http: &reqwest::Client,
    endpoint: &str,
    key: &Secret,
    query: &str,
    count: usize,
) -> Result<Searched, LlmError> {
    let sent = http
        .post(endpoint)
        .bearer_auth(key.expose().trim())
        .json(&serde_json::json!({ "query": query, "max_results": count }))
        .timeout(SEARCH_TIME)
        .send()
        .await;
    let body = answered(sent, endpoint, SEARCH_TIME).await?;
    let answer: TavilyAnswer = serde_json::from_str(&body)
        .map_err(|err| LlmError::Decode(format!("Tavily answer: {err}")))?;
    Ok(Searched { hits: found_hits(answer.results, count), usage: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn inference(provider: Provider, base_url: &str) -> InferenceConfig {
        InferenceConfig { provider, base_url: base_url.into(), ..InferenceConfig::default() }
    }

    #[test]
    fn each_provider_searches_through_the_party_already_chosen() {
        let none = Secret::default();
        assert_eq!(
            Backend::for_turn(&inference(Provider::Chatgpt, ""), &none),
            Some(Backend::Chatgpt)
        );
        assert_eq!(
            Backend::for_turn(&inference(Provider::Claude, ""), &none),
            Some(Backend::Claude)
        );
        assert_eq!(
            Backend::for_turn(
                &inference(Provider::Compatible, "https://openrouter.ai/api/v1"),
                &none
            ),
            Some(Backend::OpenRouter)
        );
    }

    #[test]
    fn a_local_endpoint_searches_only_with_a_key_the_operator_set() {
        let local = inference(Provider::Compatible, "http://127.0.0.1:1234/v1");
        assert_eq!(Backend::for_turn(&local, &Secret::default()), None);
        assert_eq!(Backend::for_turn(&local, &"  ".into()), None);
        assert_eq!(Backend::for_turn(&local, &"BSAkey".into()), Some(Backend::Brave));
        assert_eq!(Backend::for_turn(&local, &"tvly-abc".into()), Some(Backend::Tavily));
        let lookalike = inference(Provider::Compatible, "https://openrouter.ai.evil.example/v1");
        assert_eq!(Backend::for_turn(&lookalike, &Secret::default()), None);
    }

    #[test]
    fn a_key_set_for_search_does_not_move_a_subscription_off_its_own_search() {
        let key: Secret = "BSAkey".into();
        assert_eq!(
            Backend::for_turn(&inference(Provider::Chatgpt, ""), &key),
            Some(Backend::Chatgpt)
        );
        assert_eq!(
            Backend::for_turn(
                &inference(Provider::Compatible, "https://openrouter.ai/api/v1"),
                &key
            ),
            Some(Backend::Brave)
        );
    }

    #[test]
    fn only_a_model_backed_search_spends_a_model_call() {
        assert!(Backend::Chatgpt.spends_a_model_call());
        assert!(Backend::OpenRouter.spends_a_model_call());
        assert!(!Backend::Brave.spends_a_model_call());
        assert!(!Backend::Tavily.spends_a_model_call());
    }

    #[test]
    fn a_listed_answer_is_read_back_into_results() {
        let said = "Here you go:\n\
                    - [Car leasing (explained)](https://en.wikipedia.org/wiki/Car_leasing_(finance)) \u{2014} How a lease works.\n\
                    1. [Buy or lease?](https://cars.example/buy-or-lease): Costs compared.\n\
                    - [Bad](javascript:alert(1)) nope\n\
                    - [No address] nothing\n";
        assert_eq!(
            listed(said),
            vec![
                Hit {
                    title: "Car leasing (explained)".into(),
                    url: "https://en.wikipedia.org/wiki/Car_leasing_(finance)".into(),
                    snippet: "How a lease works.".into(),
                },
                Hit {
                    title: "Buy or lease?".into(),
                    url: "https://cars.example/buy-or-lease".into(),
                    snippet: "Costs compared.".into(),
                },
            ]
        );
    }

    #[test]
    fn results_are_capped_deduplicated_and_clipped() {
        let long = "word ".repeat(200);
        let hits = kept(
            [
                hit("A", "https://a.example/", "").unwrap(),
                hit("A again", "https://a.example/", "filled in").unwrap(),
                hit("", "https://b.example/", &long).unwrap(),
                hit("C", "https://c.example/", "c").unwrap(),
            ],
            2,
        );
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].snippet, "filled in");
        assert_eq!(hits[1].title, "https://b.example/");
        assert!(hits[1].snippet.chars().count() <= SNIPPET_CHARS + 1);
    }

    async fn one_answer(body: &'static str) -> (String, Arc<Mutex<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(String::new()));
        let kept = seen.clone();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut head = vec![0u8; 8192];
            let read = socket.read(&mut head).await.unwrap();
            *kept.lock().unwrap() = String::from_utf8_lossy(&head[..read]).to_string();
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
        });
        (format!("http://{address}/search"), seen)
    }

    #[tokio::test]
    async fn brave_is_asked_with_the_key_in_a_header_and_never_in_the_address() {
        let (endpoint, seen) = one_answer(
            r#"{"web":{"results":[{"title":"Lease <strong>or</strong> buy","url":"https://cars.example/a","description":"It <strong>depends</strong> &amp; more."}]}}"#,
        )
        .await;
        let http = reqwest::Client::new();
        let searched =
            brave(&http, &endpoint, &"BSA-secret-key".into(), "lease or buy", 3).await.unwrap();
        assert_eq!(
            searched.hits,
            vec![Hit {
                title: "Lease or buy".into(),
                url: "https://cars.example/a".into(),
                snippet: "It depends & more.".into(),
            }]
        );
        let request = seen.lock().unwrap().clone();
        let first = request.lines().next().unwrap_or_default();
        assert!(first.contains("q=lease+or+buy") && first.contains("count=3"), "{first}");
        assert!(!first.contains("BSA-secret-key"), "{first}");
        assert!(request.to_lowercase().contains("x-subscription-token: bsa-secret-key"));
    }

    #[tokio::test]
    async fn tavily_is_asked_with_a_bearer_key_and_its_results_are_read() {
        let (endpoint, seen) = one_answer(
            r#"{"results":[{"title":"Leasing","url":"https://cars.example/b","content":"A lease costs less up front."}]}"#,
        )
        .await;
        let http = reqwest::Client::new();
        let searched = tavily(&http, &endpoint, &"tvly-secret".into(), "lease", 5).await.unwrap();
        assert_eq!(searched.hits[0].snippet, "A lease costs less up front.");
        let request = seen.lock().unwrap().clone();
        assert!(request.contains("Bearer tvly-secret"), "{request}");
        assert!(request.contains("\"max_results\":5"), "{request}");
    }

    #[tokio::test]
    async fn a_rejected_key_is_reported_as_one() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut head = vec![0u8; 4096];
            let _ = socket.read(&mut head).await;
            let _ = socket
                .write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
        });
        let http = reqwest::Client::new();
        let refused =
            brave(&http, &format!("http://{address}/"), &"BSA".into(), "q", 5).await.unwrap_err();
        assert!(matches!(refused, LlmError::Auth { status: 401, .. }), "{refused:?}");
    }
}
