//! What an agent is already signed in to.
//!
//! Not something an operator types. Whatever is holding the cookies knows, so
//! Guaca asks it rather than asking the person, and an agent that got signed in
//! ten seconds ago advertises it on the roster without anybody recording
//! anything.
//!
//! Two things can be holding them, and the rule below is shared by both because
//! it is a fact about the web rather than about where a browser runs. A computer
//! keeps its cookies in a file, read off the disk by `sessions.py`; a hosted
//! browser is asked over the DevTools protocol by `cdp.rs`. Both arrive here as
//! a `BrowserState`, and both are recorded against the `Surface` they came from,
//! because a session on one is not reachable from the other.
//!
//! The hard part is not reading cookies, it is deciding what a cookie means.
//! Two failure modes, both observed on a real machine while this was written:
//!
//! - **Ad-tech noise.** A profile that has browsed for an hour holds a thousand
//!   cookies across three hundred domains, and the obvious heuristic, a durable
//!   `httpOnly` cookie, is true of `adnxs.com`, `360yield.com` and forty other
//!   trackers nobody has an account with.
//! - **Present but signed out.** `google.com` sets `NID` and `AEC` on a browser
//!   that has never seen a Google account. Reporting a domain because cookies
//!   exist for it would have told an agent it could read Gmail when it could
//!   not, which is worse than saying nothing: it declines *after* wasting a
//!   turn, and the operator sees a broken account rather than an absent one.
//!
//! So detection is deliberately conservative, in two layers:
//!
//! 1. A table of services whose session cookie is known by name. `li_at` means
//!    LinkedIn, `user_session` means GitHub. A domain in this table is judged
//!    only by its signature: no signature, no claim, and no falling through to
//!    the guesswork below. That is what keeps `google.com` honest.
//! 2. For everything else, a domain is reported only if the browser has
//!    actually *been* there, which is what separates a site from a tracker: an
//!    ad network sets cookies from inside someone else's page and never appears
//!    in history.
//!
//! Cookie values are never read, anywhere. The name and the flags are the whole
//! signal, and a session token is exactly the thing this file must not handle.

use serde::{Deserialize, Serialize};

use super::ids::AgentId;

/// A service whose session cookie is recognisable by name.
///
/// Only cookies that mean "somebody is logged in" belong here. A preference or
/// consent cookie set for anonymous visitors is what produced the false
/// positive this table exists to prevent.
const KNOWN: &[(&str, &str, &[&str])] = &[
    // (host suffix, what to call it, any one of these proves a session)
    ("linkedin.com", "LinkedIn", &["li_at"]),
    ("github.com", "GitHub", &["user_session"]),
    ("gitlab.com", "GitLab", &["_gitlab_session"]),
    // `NID` and `AEC` are set for signed-out visitors, so neither counts.
    ("google.com", "Google", &["SID", "__Secure-1PSID", "__Secure-3PSID"]),
    ("youtube.com", "YouTube", &["SID", "__Secure-1PSID"]),
    ("x.com", "X", &["auth_token"]),
    ("twitter.com", "X", &["auth_token"]),
    ("facebook.com", "Facebook", &["c_user"]),
    ("instagram.com", "Instagram", &["sessionid"]),
    ("reddit.com", "Reddit", &["reddit_session"]),
    ("slack.com", "Slack", &["d"]),
    ("notion.so", "Notion", &["token_v2"]),
    ("discord.com", "Discord", &["__Secure-token"]),
    ("spotify.com", "Spotify", &["sp_dc"]),
    ("amazon.com", "Amazon", &["at-main", "sess-at-main"]),
    ("dropbox.com", "Dropbox", &["jar"]),
    ("figma.com", "Figma", &["__Host-figma.authn"]),
    ("atlassian.net", "Atlassian", &["cloud.session.token"]),
];

/// Substrings that suggest a site has established *who* you are.
///
/// Deliberately excludes "session", "sessid" and "token", which look like the
/// obvious candidates and are the reason this list is short. A session id is
/// handed to every anonymous visitor: a real capture reported `PHPSESSID` on a
/// listings site and `SIFT_SESSION_ID` on another, both durable, both httpOnly,
/// and neither meaning anybody had logged in. What survives here are words that
/// only appear once an identity exists.
///
/// Even then, only consulted for a domain the browser has actually visited.
const IDENTITY_ISH: &[&str] = &["auth", "login", "remember", "sso", "credential", "identity"];

/// One cookie, reduced to the part that is safe to reason about.
///
/// There is no value field, and that is the point: this type is what the
/// sandbox hands back, so a session token has nowhere to travel to even by
/// accident.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CookieMark {
    pub domain: String,
    pub name: String,
    #[serde(default)]
    pub http_only: bool,
    /// True for a cookie that dies with the browser. A session cookie cannot
    /// prove a durable sign-in, because it is gone the moment the machine
    /// sleeps and the browser restarts.
    #[serde(default)]
    pub session: bool,
}

/// Which of an agent's two places holds a session.
///
/// An agent can be given a computer and a browser, they are different machines
/// on different providers, and their cookie jars are unrelated. Recording which
/// one a session is in is not bookkeeping: an agent signed in to Gmail on its
/// computer's screen and told only "you can reach Gmail" calls `browse`, is
/// shown a login page, and reports the account as broken. The operator has the
/// same problem in reverse, because signing an agent in is something only they
/// can do and they have to know which window to do it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Surface {
    /// The Linux machine with a screen. Reached by looking and pointing.
    Computer,
    /// The hosted browser. Reached by asking the page.
    Browser,
}

impl Surface {
    pub fn as_str(self) -> &'static str {
        match self {
            Surface::Computer => "computer",
            Surface::Browser => "browser",
        }
    }

    /// Anything unrecognised reads as the computer, which is where every
    /// session recorded before there was a second surface came from.
    pub fn parse(raw: &str) -> Self {
        match raw {
            "browser" => Surface::Browser,
            _ => Surface::Computer,
        }
    }

    /// How an agent is told to reach it, in the words of the tool that does.
    pub fn how(self) -> &'static str {
        match self {
            Surface::Computer => "on your computer's screen, so `use_screen` reaches it",
            Surface::Browser => "in your browser, so `browse` reaches it",
        }
    }
}

/// A site an agent is signed in to, and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Signin {
    pub agent_id: AgentId,
    /// Which of the agent's two places this session is in. A session on one is
    /// not reachable from the other.
    pub surface: Surface,
    /// The host, normalised: `linkedin.com`.
    pub domain: String,
    /// What to call it. A recognised service gets its real name; anything else
    /// is called by its domain rather than given an invented one.
    pub service: String,
    /// Whether this came from a known signature or from the weaker
    /// visited-plus-session-cookie rule. Shown to the operator so a guess is
    /// never presented as a certainty.
    pub recognised: bool,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

impl Signin {
    /// How this reads to the agent that holds it, and on a peer's roster.
    pub fn label(&self) -> String {
        self.service.clone()
    }
}

/// What the machine reported.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    #[serde(default)]
    pub cookies: Vec<CookieMark>,
    /// Hosts the browser has actually navigated to.
    #[serde(default)]
    pub visited: Vec<String>,
}

/// Strips the leading dot Chrome puts on a domain cookie, and any `www.`.
fn host_of(raw: &str) -> String {
    raw.trim().trim_start_matches('.').trim_start_matches("www.").to_ascii_lowercase()
}

/// Whether `host` is at or under `suffix`.
fn under(host: &str, suffix: &str) -> bool {
    host == suffix || host.ends_with(&format!(".{suffix}"))
}

/// The last two labels, which is close enough to a registrable domain for a
/// name shown to a person and costs no public-suffix list.
fn collapse(host: &str) -> String {
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() <= 2 {
        return host.to_string();
    }
    labels[labels.len() - 2..].join(".")
}

/// The host part of a URL, normalised the same way a cookie's domain is.
///
/// Hand-rolled rather than pulled from a URL crate because the only thing
/// wanted here is the authority, and anything that fails to parse must come
/// back as nothing rather than as a host that happens to match.
fn host_in(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://").map(|(_, rest)| rest).unwrap_or(url.trim());
    let authority = rest.split(['/', '?', '#']).next()?;
    // Credentials in the authority are the trick that makes `evil.com` look
    // like `mail.google.com@evil.com`, so the host is what follows the last
    // `@`, never what precedes it.
    let host = authority.rsplit('@').next()?;
    let host = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
    let host = host_of(host);
    (!host.is_empty() && host.contains('.')).then_some(host)
}

/// The session this URL is being read as, if the agent holds one for it.
///
/// The question a guard asks before letting a page it has just read drive an
/// action: acting on a site nobody is logged in to spends the agent's own time,
/// while acting on one it holds a session for spends the operator's name.
pub fn session_for<'a>(signins: &'a [Signin], url: &str) -> Option<&'a Signin> {
    let host = host_in(url)?;
    signins.iter().find(|signin| under(&host, &signin.domain))
}

/// Whether a URL is on `domain`, decided exactly as a session is.
///
/// The same question as `session_for` asked about one domain instead of a list,
/// and here rather than at the call site because the parsing above is the part
/// that has to see through `evil.com` wearing another site's name. A second
/// copy of it somewhere else is a second copy that can rot.
pub fn on_domain(url: &str, domain: &str) -> bool {
    host_in(url).is_some_and(|host| under(&host, domain))
}

/// Reads a browser's cookie jar and says what it is signed in to.
///
/// `now` is passed rather than read so the result is a pure function of its
/// input and the tests do not depend on a clock.
pub fn detect(agent_id: AgentId, surface: Surface, state: &BrowserState, now: i64) -> Vec<Signin> {
    let visited: std::collections::HashSet<String> =
        state.visited.iter().map(|host| host_of(host)).collect();

    let mut found: Vec<Signin> = Vec::new();
    let mut claimed: Vec<&str> = Vec::new();

    // Layer one: services recognised by signature.
    for (suffix, service, proofs) in KNOWN {
        let signed_in = state.cookies.iter().any(|cookie| {
            !cookie.session
                && under(&host_of(&cookie.domain), suffix)
                && proofs.iter().any(|proof| proof.eq_ignore_ascii_case(&cookie.name))
        });
        // Claimed either way. A known service that fails its own signature is
        // signed out, and must not be picked up by the looser rule below on the
        // strength of a consent cookie.
        claimed.push(suffix);
        if signed_in {
            found.push(Signin {
                agent_id,
                surface,
                domain: (*suffix).to_string(),
                service: (*service).to_string(),
                recognised: true,
                first_seen_at: now,
                last_seen_at: now,
            });
        }
    }

    // Layer two: somewhere the browser has actually been, holding something
    // that looks like a session. Visiting is what separates a site you use from
    // a tracker embedded in someone else's page.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for cookie in &state.cookies {
        let host = host_of(&cookie.domain);
        if claimed.iter().any(|suffix| under(&host, suffix)) {
            continue;
        }
        if cookie.session || !cookie.http_only {
            continue;
        }
        let name = cookie.name.to_ascii_lowercase();
        if !IDENTITY_ISH.iter().any(|marker| name.contains(marker)) {
            continue;
        }
        // The discriminator. Ad networks set cookies from inside pages you did
        // visit, and never appear in history themselves.
        if !visited.iter().any(|been| been == &host || under(been, &host)) {
            continue;
        }

        let domain = collapse(&host);
        if seen.insert(domain.clone()) {
            found.push(Signin {
                agent_id,
                surface,
                domain: domain.clone(),
                service: domain,
                recognised: false,
                first_seen_at: now,
                last_seen_at: now,
            });
        }
    }

    found.sort_by_key(|found| found.service.to_lowercase());
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(domain: &str, name: &str, http_only: bool, session: bool) -> CookieMark {
        CookieMark { domain: domain.into(), name: name.into(), http_only, session }
    }

    /// The shape of a real machine, taken from a live sandbox while this was
    /// written: signed in to LinkedIn, not signed in to Google, and carrying
    /// several hundred trackers. Cookie names are the ones actually observed,
    /// because they are the whole point; the sites somebody had been reading
    /// are not, so those are stand-ins.
    fn real_machine() -> BrowserState {
        BrowserState {
            cookies: vec![
                // LinkedIn, signed in.
                cookie(".www.linkedin.com", "li_at", true, false),
                cookie(".www.linkedin.com", "bscookie", true, false),
                cookie(".linkedin.com", "bcookie", false, false),
                cookie(".www.linkedin.com", "JSESSIONID", false, true),
                cookie(".linkedin.com", "lidc", false, false),
                // Google, visited but never signed in. These are what an
                // anonymous visitor gets.
                cookie(".google.com", "NID", true, false),
                cookie(".google.com", "AEC", true, false),
                cookie(".google.com", "SNID", true, false),
                cookie("www.google.com", "_GRECAPTCHA", true, false),
                // Trackers, none of which anybody has an account with.
                cookie(".adnxs.com", "uuid2", true, false),
                cookie(".adnxs.com", "uids", true, false),
                cookie(".360yield.com", "tuuid", true, false),
                cookie(".a-mo.net", "_sv3_0", true, false),
                cookie(".adgrx.com", "ADGRX_UID", true, false),
                cookie(".amazon-adsystem.com", "ad-id", true, false),
            ],
            visited: vec![
                "www.linkedin.com".into(),
                "news.example".into(),
                "search.example".into(),
                "listings.example".into(),
            ],
        }
    }

    #[test]
    fn a_real_machine_reports_the_one_account_it_actually_has() {
        let agent = AgentId::new();
        let found = detect(agent, Surface::Computer, &real_machine(), 100);

        assert_eq!(found.len(), 1, "expected LinkedIn and nothing else, got {found:?}");
        assert_eq!(found[0].service, "LinkedIn");
        assert_eq!(found[0].domain, "linkedin.com");
        assert!(found[0].recognised);
        assert_eq!(found[0].agent_id, agent);
    }

    #[test]
    fn a_site_with_cookies_but_no_session_is_not_reported() {
        // The false positive that matters most. `google.com` sets NID and AEC
        // on a browser that has never seen an account, and they are httpOnly
        // and durable, so anything short of a real signature check claims the
        // agent can read Gmail. It then wastes a turn finding out it cannot,
        // and the operator sees a broken account rather than an absent one.
        let found = detect(AgentId::new(), Surface::Computer, &real_machine(), 100);
        assert!(
            !found.iter().any(|s| s.service == "Google"),
            "signed out of Google must read as signed out: {found:?}"
        );

        // And the same jar plus a real session cookie does report it.
        let mut state = real_machine();
        state.cookies.push(cookie(".google.com", "__Secure-1PSID", true, false));
        state.visited.push("mail.google.com".into());
        let found = detect(AgentId::new(), Surface::Computer, &state, 100);
        assert!(found.iter().any(|s| s.service == "Google"), "{found:?}");
    }

    #[test]
    fn trackers_are_never_mistaken_for_accounts() {
        // Every one of these is httpOnly and durable, which is why the obvious
        // heuristic is useless: a browser that has read the news for an hour
        // holds hundreds of them.
        let found = detect(AgentId::new(), Surface::Computer, &real_machine(), 100);
        for noise in ["adnxs.com", "360yield.com", "a-mo.net", "adgrx.com", "amazon-adsystem.com"] {
            assert!(!found.iter().any(|s| s.domain.contains(noise)), "reported {noise}: {found:?}");
        }
    }

    #[test]
    fn a_site_the_browser_has_visited_is_reported_even_though_nobody_listed_it() {
        // The whole point of the second layer: an operator should not have to
        // teach Guaca about their own intranet or a service nobody has heard of.
        let state = BrowserState {
            cookies: vec![cookie(".wiki.internal.example", "auth_user", true, false)],
            visited: vec!["wiki.internal.example".into()],
        };
        let found = detect(AgentId::new(), Surface::Computer, &state, 100);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].domain, "internal.example");
        assert!(!found[0].recognised, "a guess must not be presented as a certainty");
    }

    #[test]
    fn a_session_id_handed_to_every_visitor_is_not_a_login() {
        // Both cookie names are real, from a capture of a machine that had
        // browsed for an hour: durable, httpOnly, on sites the browser had
        // genuinely visited, and neither meant anybody had logged in.
        // `PHPSESSID` exists for anonymous visitors by definition, and
        // `SIFT_SESSION_ID` belongs to a fraud-detection vendor. This is why the
        // second layer looks for words that imply an identity rather than words
        // that imply a session.
        let state = BrowserState {
            cookies: vec![
                cookie("listings.example", "PHPSESSID", true, false),
                cookie("www.events.example", "SIFT_SESSION_ID", true, false),
            ],
            visited: vec!["listings.example".into(), "www.events.example".into()],
        };
        assert!(
            detect(AgentId::new(), Surface::Computer, &state, 100).is_empty(),
            "an anonymous session id must not read as an account"
        );
    }

    #[test]
    fn a_tracker_with_a_session_shaped_name_still_needs_to_have_been_visited() {
        // This is the whole discriminator, so it is worth its own test: the
        // same cookie is reported or ignored purely on whether the browser has
        // ever been to that host.
        let cookies = vec![cookie(".tracker.example", "authuser", true, false)];

        let unvisited =
            BrowserState { cookies: cookies.clone(), visited: vec!["news.example".into()] };
        assert!(detect(AgentId::new(), Surface::Computer, &unvisited, 100).is_empty());

        let visited = BrowserState { cookies, visited: vec!["tracker.example".into()] };
        assert_eq!(detect(AgentId::new(), Surface::Computer, &visited, 100).len(), 1);
    }

    #[test]
    fn a_cookie_that_dies_with_the_browser_cannot_prove_a_durable_sign_in() {
        // Machines sleep and Chrome restarts, so a session cookie is gone by
        // the time an agent would act on it.
        let state = BrowserState {
            cookies: vec![cookie(".www.linkedin.com", "li_at", true, true)],
            visited: vec!["www.linkedin.com".into()],
        };
        assert!(
            detect(AgentId::new(), Surface::Computer, &state, 100).is_empty(),
            "a session cookie proves nothing"
        );
    }

    #[test]
    fn a_known_service_that_is_signed_out_never_falls_through_to_guesswork() {
        // Without this, a signed-out Google with any session-shaped cookie
        // would come back as a bare `google.com` entry, which is the same lie
        // in a worse costume.
        let state = BrowserState {
            cookies: vec![
                cookie(".google.com", "NID", true, false),
                cookie(".google.com", "some_auth_thing", true, false),
            ],
            visited: vec!["www.google.com".into()],
        };
        assert!(
            detect(AgentId::new(), Surface::Computer, &state, 100).is_empty(),
            "a known service is judged once"
        );
    }

    #[test]
    fn hosts_are_normalised_however_chrome_spells_them() {
        assert_eq!(host_of(".www.LinkedIn.com"), "linkedin.com");
        assert_eq!(host_of("www.google.com"), "google.com");
        assert_eq!(host_of(".x.com"), "x.com");
        assert!(under("mail.google.com", "google.com"));
        assert!(under("google.com", "google.com"));
        // The check has to be on a label boundary, or `notgoogle.com` matches.
        assert!(!under("notgoogle.com", "google.com"));
        assert_eq!(collapse("wiki.internal.example"), "internal.example");
        assert_eq!(collapse("example.com"), "example.com");
    }

    #[test]
    fn one_service_is_reported_once_however_many_cookies_prove_it() {
        let state = BrowserState {
            cookies: vec![
                cookie(".www.linkedin.com", "li_at", true, false),
                cookie(".linkedin.com", "li_at", true, false),
            ],
            visited: vec!["www.linkedin.com".into()],
        };
        assert_eq!(detect(AgentId::new(), Surface::Computer, &state, 100).len(), 1);
    }

    #[test]
    fn nothing_in_the_jar_means_nothing_claimed() {
        assert!(detect(AgentId::new(), Surface::Computer, &BrowserState::default(), 100).is_empty());
    }

    fn signin_for(domain: &str) -> Signin {
        Signin {
            surface: Surface::Computer,
            agent_id: AgentId::new(),
            domain: domain.into(),
            service: domain.into(),
            recognised: true,
            first_seen_at: 0,
            last_seen_at: 0,
        }
    }

    #[test]
    fn a_host_is_read_out_of_a_url_the_way_a_cookie_domain_is() {
        assert_eq!(
            host_in("https://www.Mail.Google.com/u/0?x=1").as_deref(),
            Some("mail.google.com")
        );
        assert_eq!(host_in("http://github.com:8080/a/b").as_deref(), Some("github.com"));
        assert_eq!(host_in("linkedin.com/feed").as_deref(), Some("linkedin.com"));
    }

    #[test]
    fn anything_without_a_host_is_nothing_rather_than_a_guess() {
        // A guard that reads an unparseable URL as "no session" is the safe
        // way round only because the caller treats None as "not signed in and
        // therefore not the operator's name". Anything shaped like a host has
        // to be rejected rather than half-matched.
        assert_eq!(host_in("about:blank"), None);
        assert_eq!(host_in(""), None);
        assert_eq!(host_in("file:///home/user/inbox/report.pdf"), None);
        assert_eq!(host_in("localhost"), None);
    }

    #[test]
    fn credentials_in_the_authority_cannot_borrow_a_session() {
        // The oldest phishing URL there is. `mail.google.com` before an `@` is
        // a username, and the host is what follows it. Reading the string from
        // the left would hand an attacker's page the operator's Gmail session.
        let held = [signin_for("google.com")];
        assert_eq!(host_in("https://mail.google.com@evil.com/x").as_deref(), Some("evil.com"));
        assert!(
            session_for(&held, "https://mail.google.com@evil.com/x").is_none(),
            "a session must not be matched against a username"
        );
    }

    #[test]
    fn a_session_covers_its_subdomains_and_nothing_that_merely_ends_with_it() {
        let held = [signin_for("google.com")];
        assert!(session_for(&held, "https://mail.google.com/inbox").is_some());
        assert!(session_for(&held, "https://google.com/").is_some());
        assert!(
            session_for(&held, "https://notgoogle.com/").is_none(),
            "a suffix match without the dot would cover every lookalike domain"
        );
        assert!(session_for(&held, "https://example.org/").is_none());
    }

    #[test]
    fn asking_about_one_domain_sees_through_the_same_two_tricks() {
        // `on_domain` is what a turn's consent grant is checked against, so a
        // version that answered these differently from `session_for` would let
        // a page the operator never saw inherit the yes they gave for another.
        assert!(on_domain("https://business.facebook.com/latest/inbox", "facebook.com"));
        assert!(on_domain("https://facebook.com/", "facebook.com"));
        assert!(!on_domain("https://notfacebook.com/", "facebook.com"));
        assert!(!on_domain("https://facebook.com@evil.com/x", "facebook.com"));
        assert!(!on_domain("not a url", "facebook.com"));
    }
}
