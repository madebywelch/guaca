use std::error::Error as StdError;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;

pub const PAGE_BYTES: usize = 2 * 1024 * 1024;
pub const PAGE_TIME: Duration = Duration::from_secs(20);
pub const MOST_REDIRECTS: usize = 5;
pub const MOST_LINKS: usize = 200;

const CONNECT_TIME: Duration = Duration::from_secs(10);
const AGENT: &str =
    concat!("Mozilla/5.0 (compatible; Guaca/", env!("CARGO_PKG_VERSION"), "; read_page)");
const ACCEPTS: &str = "text/html,application/xhtml+xml,text/plain;q=0.9,*/*;q=0.5";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub text: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub url: String,
    pub title: String,
    pub text: String,
    pub links: Vec<Link>,
    pub cut: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    #[error("`{0}` is not a web address: only http:// and https:// addresses can be read")]
    NotWeb(String),
    #[error(
        "{host} is on this host's own network (loopback, private, link-local or a cloud \
         metadata address), and pages there are never read"
    )]
    Private { host: String },
    #[error("{host} could not be reached: the name did not resolve or the site did not answer")]
    Unreachable { host: String },
    #[error("the page took longer than {secs} seconds and was given up on")]
    Slow { secs: u64 },
    #[error("the page redirected more than {MOST_REDIRECTS} times")]
    Redirects,
    #[error("the site answered with status {status}")]
    Status { status: u16 },
    #[error("the address holds {kind}, not a page of text")]
    NotText { kind: String },
}

pub fn forbidden(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => forbidden_v4(v4),
        IpAddr::V6(v6) => forbidden_v6(v6),
    }
}

fn forbidden_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        || a >= 240
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && (b == 18 || b == 19))
}

fn forbidden_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return forbidden_v4(v4);
    }
    let segments = ip.segments();
    let embedded = |high: u16, low: u16| Ipv4Addr::from((u32::from(high) << 16) | u32::from(low));
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] & 0xffc0) == 0xfec0
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || (segments[0] == 0x2001 && segments[1] == 0)
        || (segments[..6] == [0, 0, 0, 0, 0, 0] && forbidden_v4(embedded(segments[6], segments[7])))
        || (segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0]
            && forbidden_v4(embedded(segments[6], segments[7])))
        || (segments[0] == 0x2002 && forbidden_v4(embedded(segments[1], segments[2])))
}

pub fn vet(url: &reqwest::Url, allowed: fn(IpAddr) -> bool) -> Result<(), FetchError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(FetchError::NotWeb(url.to_string()));
    }
    let Some(host) = url.host_str().filter(|host| !host.is_empty()) else {
        return Err(FetchError::NotWeb(url.to_string()));
    };
    let Ok(ip) = host.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>() else {
        return Ok(());
    };
    if allowed(ip) {
        Ok(())
    } else {
        Err(FetchError::Private { host: url.host_str().unwrap_or_default().to_string() })
    }
}

fn permitted(ip: IpAddr) -> bool {
    !forbidden(ip)
}

#[derive(Debug, thiserror::Error)]
#[error("{0} resolves only to addresses on this host's own network")]
struct Inward(String);

struct Resolver {
    allowed: fn(IpAddr) -> bool,
}

impl reqwest::dns::Resolve for Resolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let allowed = self.allowed;
        let host = name.as_str().to_string();
        Box::pin(async move {
            let found: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|err| Box::new(err) as Box<dyn StdError + Send + Sync>)?
                .collect();
            let kept: Vec<SocketAddr> =
                found.into_iter().filter(|addr| allowed(addr.ip())).collect();
            if kept.is_empty() {
                return Err(Box::new(Inward(host)) as Box<dyn StdError + Send + Sync>);
            }
            Ok(Box::new(kept.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub bytes: usize,
    pub time: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { bytes: PAGE_BYTES, time: PAGE_TIME }
    }
}

#[derive(Debug, Clone)]
pub struct Fetcher {
    http: reqwest::Client,
    allowed: fn(IpAddr) -> bool,
    limits: Limits,
}

impl Fetcher {
    pub fn new() -> Result<Self, reqwest::Error> {
        Self::guarded(permitted, Limits::default())
    }

    fn guarded(allowed: fn(IpAddr) -> bool, limits: Limits) -> Result<Self, reqwest::Error> {
        let policy = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() > MOST_REDIRECTS {
                return attempt.error(FetchError::Redirects);
            }
            match vet(attempt.url(), allowed) {
                Ok(()) => attempt.follow(),
                Err(err) => attempt.error(err),
            }
        });
        let http = reqwest::Client::builder()
            .user_agent(AGENT)
            .no_proxy()
            .dns_resolver(Arc::new(Resolver { allowed }))
            .redirect(policy)
            .connect_timeout(CONNECT_TIME.min(limits.time))
            .timeout(limits.time)
            .build()?;
        Ok(Fetcher { http, allowed, limits })
    }

    pub async fn fetch(&self, address: &str) -> Result<Page, FetchError> {
        let asked = address.trim();
        let url = reqwest::Url::parse(asked).map_err(|_| FetchError::NotWeb(asked.to_string()))?;
        vet(&url, self.allowed)?;
        let host = url.host_str().unwrap_or_default().to_string();
        match tokio::time::timeout(self.limits.time, self.read(url, &host)).await {
            Ok(read) => read,
            Err(_) => Err(self.slow()),
        }
    }

    fn slow(&self) -> FetchError {
        FetchError::Slow { secs: self.limits.time.as_secs().max(1) }
    }

    async fn read(&self, url: reqwest::Url, host: &str) -> Result<Page, FetchError> {
        let response = self
            .http
            .get(url)
            .header(reqwest::header::ACCEPT, ACCEPTS)
            .send()
            .await
            .map_err(|err| self.failed(&err, host))?;

        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default();
            return Err(FetchError::NotWeb(location.to_string()));
        }
        if !status.is_success() {
            return Err(FetchError::Status { status: status.as_u16() });
        }
        let at = response.url().clone();
        let kind = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let markup = kind.is_empty() || kind.contains("html") || kind.contains("xml");
        if !markup && !kind.starts_with("text/") && !kind.contains("json") {
            let shown = kind.split(';').next().unwrap_or_default().trim().to_string();
            return Err(FetchError::NotText { kind: shown });
        }

        let mut body: Vec<u8> = Vec::new();
        let mut cut = false;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|err| self.failed(&err, host))?;
            let room = self.limits.bytes - body.len();
            if chunk.len() > room {
                body.extend_from_slice(&chunk[..room]);
                cut = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }

        let raw = String::from_utf8_lossy(&body);
        let (title, text, links) = if markup {
            extract(&raw, &at)
        } else {
            (String::new(), raw.trim().to_string(), Vec::new())
        };
        Ok(Page { url: at.to_string(), title, text, links, cut })
    }

    fn failed(&self, err: &reqwest::Error, host: &str) -> FetchError {
        let mut source: Option<&(dyn StdError + 'static)> = Some(err);
        while let Some(cause) = source {
            if let Some(fetch) = cause.downcast_ref::<FetchError>() {
                return fetch.clone();
            }
            if let Some(Inward(name)) = cause.downcast_ref::<Inward>() {
                return FetchError::Private { host: name.clone() };
            }
            source = cause.source();
        }
        if err.is_timeout() {
            return self.slow();
        }
        if err.is_redirect() {
            return FetchError::Redirects;
        }
        FetchError::Unreachable { host: host.to_string() }
    }
}

const SKIPPED: &[&str] =
    &["script", "style", "noscript", "template", "svg", "canvas", "iframe", "object", "select"];

const BLOCKS: &[&str] = &[
    "p",
    "div",
    "br",
    "tr",
    "table",
    "section",
    "article",
    "header",
    "footer",
    "nav",
    "main",
    "aside",
    "ul",
    "ol",
    "blockquote",
    "pre",
    "hr",
    "dl",
    "dd",
    "dt",
    "figure",
    "figcaption",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
];

struct Tag<'a> {
    name: String,
    closing: bool,
    attributes: &'a str,
}

fn tag_at(rest: &str) -> Option<(Tag<'_>, usize)> {
    let inner = rest.strip_prefix('<')?;
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(after) => (true, after),
        None => (false, inner),
    };
    let named = inner.find(|c: char| !c.is_ascii_alphanumeric() && c != '-').unwrap_or(inner.len());
    if named == 0 {
        return None;
    }
    let mut quote: Option<char> = None;
    let mut end = None;
    for (at, c) in inner.char_indices().skip(named) {
        match quote {
            Some(open) if c == open => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '>' => {
                end = Some(at);
                break;
            }
            None => {}
        }
    }
    let end = end?;
    let consumed = rest.len() - inner.len() + end + 1;
    Some((
        Tag { name: inner[..named].to_ascii_lowercase(), closing, attributes: &inner[named..end] },
        consumed,
    ))
}

fn attribute(attributes: &str, wanted: &str) -> Option<String> {
    let lower = attributes.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(wanted) {
        let at = from + found;
        from = at + wanted.len();
        let before = lower[..at].chars().next_back();
        if before.is_some_and(|c| !c.is_whitespace()) {
            continue;
        }
        let after = lower[from..].trim_start();
        let Some(value) = after.strip_prefix('=') else { continue };
        let start = attributes.len() - value.len();
        let value = attributes[start..].trim_start();
        let found = match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or_default(),
            _ => value.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or_default(),
        };
        return Some(decode(found));
    }
    None
}

fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let entity =
            rest[1..].find(';').filter(|end| *end <= 10).map(|end| (&rest[1..=end], end + 2));
        let decoded = entity.and_then(|(name, used)| named(name).map(|c| (c, used)));
        match decoded {
            Some((c, used)) => {
                out.push(c);
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn named(entity: &str) -> Option<char> {
    if let Some(number) = entity.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse().ok()?,
        };
        return char::from_u32(code).filter(|c| !c.is_control() || c.is_whitespace());
    }
    Some(match entity {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "mdash" => '\u{2014}',
        "ndash" => '\u{2013}',
        "hellip" => '\u{2026}',
        "lsquo" => '\u{2018}',
        "rsquo" => '\u{2019}',
        "ldquo" => '\u{201c}',
        "rdquo" => '\u{201d}',
        "middot" => '\u{b7}',
        "bull" => '\u{2022}',
        "copy" => '\u{a9}',
        "reg" => '\u{ae}',
        "trade" => '\u{2122}',
        "euro" => '\u{20ac}',
        "pound" => '\u{a3}',
        _ => return None,
    })
}

fn push_words(out: &mut String, words: &str) {
    let spaced = |out: &mut String| {
        if !out.is_empty() && !out.ends_with([' ', '\n']) {
            out.push(' ');
        }
    };
    if words.starts_with(char::is_whitespace) {
        spaced(out);
    }
    for (index, word) in words.split_whitespace().enumerate() {
        if index > 0 {
            spaced(out);
        }
        out.push_str(word);
    }
    if words.ends_with(char::is_whitespace) {
        spaced(out);
    }
}

fn break_line(out: &mut String) {
    while out.ends_with(' ') {
        out.pop();
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

fn break_paragraph(out: &mut String) {
    break_line(out);
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push('\n');
    }
}

pub fn plain(html: &str) -> String {
    match reqwest::Url::parse("https://plain.invalid/") {
        Ok(base) => extract(html, &base).1,
        Err(_) => html.to_string(),
    }
}

pub fn extract(html: &str, base: &reqwest::Url) -> (String, String, Vec<Link>) {
    let mut title = String::new();
    let mut text = String::new();
    let mut links: Vec<Link> = Vec::new();
    let mut skipping: Option<String> = None;
    let mut in_title = false;
    let mut link: Option<(String, String)> = None;
    let mut rest = html;

    while !rest.is_empty() {
        let Some(at) = rest.find('<') else {
            if skipping.is_none() && !in_title {
                let words = decode(rest);
                push_words(&mut text, &words);
                if let Some((_, said)) = link.as_mut() {
                    push_words(said, &words);
                }
            }
            break;
        };
        if at > 0 {
            let words = decode(&rest[..at]);
            if in_title {
                push_words(&mut title, &words);
            } else if skipping.is_none() {
                push_words(&mut text, &words);
                if let Some((_, said)) = link.as_mut() {
                    push_words(said, &words);
                }
            }
        }
        rest = &rest[at..];

        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |end| &after[end + 3..]);
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            rest = rest.find('>').map_or("", |end| &rest[end + 1..]);
            continue;
        }
        let Some((tag, used)) = tag_at(rest) else {
            if skipping.is_none() && !in_title {
                push_words(&mut text, "<");
            }
            rest = &rest[1..];
            continue;
        };
        rest = &rest[used..];

        if let Some(open) = &skipping {
            if tag.closing && tag.name == *open {
                skipping = None;
            }
            continue;
        }
        let name = tag.name.as_str();
        if SKIPPED.contains(&name) && !tag.closing && !tag.attributes.trim_end().ends_with('/') {
            skipping = Some(tag.name.clone());
            continue;
        }
        match name {
            "title" => in_title = !tag.closing && title.is_empty(),
            "a" if !tag.closing => {
                link = attribute(tag.attributes, "href").map(|href| (href, String::new()));
            }
            "a" => {
                if let Some((href, said)) = link.take() {
                    let joined = base.join(href.trim()).ok().filter(|url| {
                        matches!(url.scheme(), "http" | "https") && links.len() < MOST_LINKS
                    });
                    if let Some(mut url) = joined {
                        url.set_fragment(None);
                        let url = url.to_string();
                        if !links.iter().any(|seen| seen.url == url) {
                            links.push(Link { text: said.trim().to_string(), url });
                        }
                    }
                }
            }
            "li" if !tag.closing => {
                break_line(&mut text);
                text.push_str("- ");
            }
            "td" | "th" if !tag.closing => push_words(&mut text, " "),
            _ if name.len() == 2 && name.starts_with('h') && BLOCKS.contains(&name) => {
                break_paragraph(&mut text);
            }
            _ if BLOCKS.contains(&name) => break_line(&mut text),
            _ => {}
        }
    }

    let tidy = text
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
        .split("\n\n\n")
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut blank = false;
    let text = tidy
        .lines()
        .filter(|line| {
            let keep = !(line.is_empty() && blank);
            blank = line.is_empty();
            keep
        })
        .collect::<Vec<_>>()
        .join("\n");
    (title.trim().to_string(), text.trim().to_string(), links)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn base() -> reqwest::Url {
        reqwest::Url::parse("https://news.example/section/story.html").unwrap()
    }

    #[test]
    fn every_inward_address_is_forbidden() {
        let inward = [
            "127.0.0.1",
            "127.255.0.9",
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "169.254.0.1",
            "100.64.0.1",
            "100.100.100.200",
            "192.0.0.192",
            "198.18.0.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "192.0.2.1",
            "::",
            "::1",
            "fe80::1",
            "fc00::1",
            "fd00:ec2::254",
            "fec0::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:10.1.2.3",
            "::7f00:1",
            "64:ff9b::a9fe:a9fe",
            "2002:7f00:1::",
            "2002:a9fe:a9fe::1",
            "2001:db8::1",
            "2001::1",
        ];
        for address in inward {
            let ip: IpAddr = address.parse().unwrap();
            assert!(forbidden(ip), "{address} should be refused");
        }
    }

    #[test]
    fn the_open_internet_is_not_forbidden() {
        for address in
            ["1.1.1.1", "8.8.8.8", "93.184.216.34", "172.32.0.1", "2606:4700::1111", "2a00:1450::1"]
        {
            let ip: IpAddr = address.parse().unwrap();
            assert!(!forbidden(ip), "{address} should be readable");
        }
    }

    #[test]
    fn only_http_and_https_are_web_addresses() {
        for address in ["file:///etc/passwd", "ftp://files.example/a", "gopher://x.example/"] {
            let url = reqwest::Url::parse(address).unwrap();
            assert!(matches!(vet(&url, permitted), Err(FetchError::NotWeb(_))), "{address}");
        }
        let url = reqwest::Url::parse("https://example.com/a").unwrap();
        assert_eq!(vet(&url, permitted), Ok(()));
    }

    #[test]
    fn an_inward_address_is_refused_however_it_is_spelled() {
        for address in [
            "http://127.0.0.1/",
            "http://2130706433/",
            "http://0x7f.0.0.1/",
            "http://0177.0.0.1/",
            "http://[::1]:8080/",
            "http://[::ffff:7f00:1]/",
            "http://169.254.169.254/latest/meta-data/",
            "https://10.0.0.8/admin",
        ] {
            let url = reqwest::Url::parse(address).unwrap();
            assert!(matches!(vet(&url, permitted), Err(FetchError::Private { .. })), "{address}");
        }
    }

    #[tokio::test]
    async fn a_name_that_resolves_inward_is_refused_at_resolution() {
        use reqwest::dns::Resolve;
        let resolver = Resolver { allowed: permitted };
        let name: reqwest::dns::Name = "localhost".parse().unwrap();
        let refused = resolver.resolve(name).await.err().expect("localhost must not resolve");
        assert!(refused.downcast_ref::<Inward>().is_some(), "{refused}");
    }

    struct Site {
        address: SocketAddr,
        hits: Arc<AtomicUsize>,
    }

    async fn serve(answer: fn(&str, u16) -> Option<Vec<u8>>) -> Site {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = hits.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                counted.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut head = vec![0u8; 4096];
                    let read = socket.read(&mut head).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&head[..read]).to_string();
                    let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                    match answer(&path, address.port()) {
                        Some(reply) => {
                            let _ = socket.write_all(&reply).await;
                        }
                        None => std::future::pending::<()>().await,
                    }
                });
            }
        });
        Site { address, hits }
    }

    fn reply(status: &str, headers: &str, body: &str) -> Option<Vec<u8>> {
        Some(
            format!(
                "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .into_bytes(),
        )
    }

    fn loopback(ip: IpAddr) -> bool {
        ip == IpAddr::V4(Ipv4Addr::LOCALHOST) || permitted(ip)
    }

    fn reader(limits: Limits) -> Fetcher {
        Fetcher::guarded(loopback, limits).unwrap()
    }

    #[tokio::test]
    async fn the_real_guard_never_connects_to_this_host() {
        let site = serve(|_, _| reply("200 OK", "Content-Type: text/html\r\n", "secret")).await;
        let fetcher = Fetcher::new().unwrap();
        let port = site.address.port();
        for address in [format!("http://127.0.0.1:{port}/"), format!("http://localhost:{port}/")] {
            let refused = fetcher.fetch(&address).await;
            assert!(matches!(refused, Err(FetchError::Private { .. })), "{address}: {refused:?}");
        }
        assert_eq!(site.hits.load(Ordering::SeqCst), 0, "a refused address was still contacted");
    }

    #[tokio::test]
    async fn a_page_is_read_as_its_title_words_and_links() {
        let site = serve(|_, _| {
            reply(
                "200 OK",
                "Content-Type: text/html; charset=utf-8\r\n",
                "<html><head><title>Leasing &amp; buying</title><style>p{}</style></head>\
                 <body><script>steal()</script><h1>Compare</h1><p>Leasing costs \
                 <a href=\"/rates#top\">less up front</a>.</p><ul><li>One</li><li>Two</li></ul>\
                 </body></html>",
            )
        })
        .await;
        let page =
            reader(Limits::default()).fetch(&format!("http://{}/car", site.address)).await.unwrap();
        assert_eq!(page.title, "Leasing & buying");
        assert_eq!(page.text, "Compare\n\nLeasing costs less up front.\n- One\n- Two");
        assert_eq!(
            page.links,
            vec![Link {
                text: "less up front".into(),
                url: format!("http://{}/rates", site.address)
            }]
        );
        assert!(!page.cut);
    }

    #[tokio::test]
    async fn a_redirect_inward_is_refused_before_it_is_followed() {
        let site = serve(|path, _| match path {
            "/metadata" => {
                reply("302 Found", "Location: http://169.254.169.254/latest/meta-data/\r\n", "")
            }
            "/v6" => reply("301 Moved", "Location: http://[::1]/\r\n", ""),
            "/scheme" => reply("302 Found", "Location: file:///etc/passwd\r\n", ""),
            _ => reply("200 OK", "Content-Type: text/plain\r\n", "fine"),
        })
        .await;
        let fetcher = reader(Limits::default());
        for path in ["metadata", "v6"] {
            let refused = fetcher.fetch(&format!("http://{}/{path}", site.address)).await;
            assert!(matches!(refused, Err(FetchError::Private { .. })), "{path}: {refused:?}");
        }
        let refused = fetcher.fetch(&format!("http://{}/scheme", site.address)).await;
        assert!(matches!(refused, Err(FetchError::NotWeb(_))), "{refused:?}");
    }

    #[tokio::test]
    async fn a_redirect_loop_stops_at_the_limit() {
        let site = serve(|path, port| {
            let next: u32 = path.trim_start_matches('/').parse().unwrap_or(0) + 1;
            reply("302 Found", &format!("Location: http://127.0.0.1:{port}/{next}\r\n"), "")
        })
        .await;
        let refused = reader(Limits::default()).fetch(&format!("http://{}/0", site.address)).await;
        assert_eq!(refused, Err(FetchError::Redirects));
        assert_eq!(site.hits.load(Ordering::SeqCst), MOST_REDIRECTS + 1);
    }

    #[tokio::test]
    async fn a_page_past_the_size_cap_is_cut_rather_than_read_whole() {
        let site =
            serve(|_, _| reply("200 OK", "Content-Type: text/plain\r\n", &"word ".repeat(10_000)))
                .await;
        let limits = Limits { bytes: 1000, ..Limits::default() };
        let page = reader(limits).fetch(&format!("http://{}/", site.address)).await.unwrap();
        assert!(page.cut);
        assert!(page.text.len() <= 1000, "{} bytes kept", page.text.len());
    }

    #[tokio::test]
    async fn a_site_that_never_answers_is_given_up_on_at_the_time_cap() {
        let site = serve(|_, _| None).await;
        let limits = Limits { time: Duration::from_millis(300), ..Limits::default() };
        let began = std::time::Instant::now();
        let refused = reader(limits).fetch(&format!("http://{}/", site.address)).await;
        assert_eq!(refused, Err(FetchError::Slow { secs: 1 }));
        assert!(began.elapsed() < Duration::from_secs(5), "took {:?}", began.elapsed());
    }

    #[tokio::test]
    async fn a_file_that_is_not_text_is_refused_by_its_type() {
        let site = serve(|_, _| reply("200 OK", "Content-Type: application/pdf\r\n", "%PDF")).await;
        let refused = reader(Limits::default()).fetch(&format!("http://{}/", site.address)).await;
        assert_eq!(refused, Err(FetchError::NotText { kind: "application/pdf".into() }));
    }

    #[tokio::test]
    async fn a_failed_status_is_reported_with_it() {
        let site = serve(|_, _| reply("404 Not Found", "", "")).await;
        let refused = reader(Limits::default()).fetch(&format!("http://{}/", site.address)).await;
        assert_eq!(refused, Err(FetchError::Status { status: 404 }));
    }

    #[test]
    fn markup_is_reduced_to_words_and_entities_are_decoded() {
        let (title, text, _) = extract(
            "<!doctype html><title>A &lt;b&gt;</title><!-- hidden --><div>caf&#233; \
             &#x2014; 5&nbsp;km &unknown; &amp</div><noscript>nope</noscript><p>a<br>b</p>",
            &base(),
        );
        assert_eq!(title, "A <b>");
        assert_eq!(text, "caf\u{e9} \u{2014} 5 km &unknown; &amp\na\nb");
    }

    #[test]
    fn links_resolve_against_the_page_and_keep_only_the_web() {
        let (_, _, links) = extract(
            "<a href='../other.html'>Other</a><a href=\"mailto:a@b.c\">Mail</a>\
             <a href=\"javascript:go()\">Go</a><a HREF=https://x.example/y?q=1&amp;r=2>X</a>\
             <a href='../other.html#part'>Again</a><a data-href=\"/no\">No</a>",
            &base(),
        );
        assert_eq!(
            links,
            vec![
                Link { text: "Other".into(), url: "https://news.example/other.html".into() },
                Link { text: "X".into(), url: "https://x.example/y?q=1&r=2".into() },
            ]
        );
    }
}
