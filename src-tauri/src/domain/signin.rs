use serde::{Deserialize, Serialize};

use super::ids::AgentId;

const KNOWN: &[(&str, &str, &[&str])] = &[
    ("linkedin.com", "LinkedIn", &["li_at"]),
    ("github.com", "GitHub", &["user_session"]),
    ("gitlab.com", "GitLab", &["_gitlab_session"]),
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

const IDENTITY_ISH: &[&str] = &["auth", "login", "remember", "sso", "credential", "identity"];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CookieMark {
    pub domain: String,
    pub name: String,
    #[serde(default)]
    pub http_only: bool,
    #[serde(default)]
    pub session: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Surface {
    Computer,
    Browser,
}

impl Surface {
    pub fn as_str(self) -> &'static str {
        match self {
            Surface::Computer => "computer",
            Surface::Browser => "browser",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "browser" => Surface::Browser,
            _ => Surface::Computer,
        }
    }

    pub fn how(self) -> &'static str {
        match self {
            Surface::Computer => "on your computer's screen, so `use_screen` reaches it",
            Surface::Browser => "in your browser, so `browse` reaches it",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Signin {
    pub agent_id: AgentId,
    pub surface: Surface,
    pub domain: String,
    pub service: String,
    pub recognized: bool,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

impl Signin {
    pub fn label(&self) -> String {
        self.service.clone()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    #[serde(default)]
    pub cookies: Vec<CookieMark>,
    #[serde(default)]
    pub visited: Vec<String>,
}

fn host_of(raw: &str) -> String {
    raw.trim().trim_start_matches('.').trim_start_matches("www.").to_ascii_lowercase()
}

fn under(host: &str, suffix: &str) -> bool {
    host == suffix || host.ends_with(&format!(".{suffix}"))
}

fn collapse(host: &str) -> String {
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() <= 2 {
        return host.to_string();
    }
    labels[labels.len() - 2..].join(".")
}

fn host_in(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://").map(|(_, rest)| rest).unwrap_or(url.trim());
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
    let host = host_of(host);
    (!host.is_empty() && host.contains('.')).then_some(host)
}

pub fn session_for<'a>(signins: &'a [Signin], url: &str) -> Option<&'a Signin> {
    let host = host_in(url)?;
    signins.iter().find(|signin| under(&host, &signin.domain))
}

pub fn on_domain(url: &str, domain: &str) -> bool {
    host_in(url).is_some_and(|host| under(&host, domain))
}

pub fn detect(agent_id: AgentId, surface: Surface, state: &BrowserState, now: i64) -> Vec<Signin> {
    let visited: std::collections::HashSet<String> =
        state.visited.iter().map(|host| host_of(host)).collect();

    let mut found: Vec<Signin> = Vec::new();
    let mut claimed: Vec<&str> = Vec::new();

    for (suffix, service, proofs) in KNOWN {
        let signed_in = state.cookies.iter().any(|cookie| {
            !cookie.session
                && under(&host_of(&cookie.domain), suffix)
                && proofs.iter().any(|proof| proof.eq_ignore_ascii_case(&cookie.name))
        });
        claimed.push(suffix);
        if signed_in {
            found.push(Signin {
                agent_id,
                surface,
                domain: (*suffix).to_string(),
                service: (*service).to_string(),
                recognized: true,
                first_seen_at: now,
                last_seen_at: now,
            });
        }
    }

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
                recognized: false,
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

    fn real_machine() -> BrowserState {
        BrowserState {
            cookies: vec![
                cookie(".www.linkedin.com", "li_at", true, false),
                cookie(".www.linkedin.com", "bscookie", true, false),
                cookie(".linkedin.com", "bcookie", false, false),
                cookie(".www.linkedin.com", "JSESSIONID", false, true),
                cookie(".linkedin.com", "lidc", false, false),
                cookie(".google.com", "NID", true, false),
                cookie(".google.com", "AEC", true, false),
                cookie(".google.com", "SNID", true, false),
                cookie("www.google.com", "_GRECAPTCHA", true, false),
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
        assert!(found[0].recognized);
        assert_eq!(found[0].agent_id, agent);
    }

    #[test]
    fn a_site_with_cookies_but_no_session_is_not_reported() {
        let found = detect(AgentId::new(), Surface::Computer, &real_machine(), 100);
        assert!(
            !found.iter().any(|s| s.service == "Google"),
            "signed out of Google must read as signed out: {found:?}"
        );

        let mut state = real_machine();
        state.cookies.push(cookie(".google.com", "__Secure-1PSID", true, false));
        state.visited.push("mail.google.com".into());
        let found = detect(AgentId::new(), Surface::Computer, &state, 100);
        assert!(found.iter().any(|s| s.service == "Google"), "{found:?}");
    }

    #[test]
    fn trackers_are_never_mistaken_for_accounts() {
        let found = detect(AgentId::new(), Surface::Computer, &real_machine(), 100);
        for noise in ["adnxs.com", "360yield.com", "a-mo.net", "adgrx.com", "amazon-adsystem.com"] {
            assert!(!found.iter().any(|s| s.domain.contains(noise)), "reported {noise}: {found:?}");
        }
    }

    #[test]
    fn a_site_the_browser_has_visited_is_reported_even_though_nobody_listed_it() {
        let state = BrowserState {
            cookies: vec![cookie(".wiki.internal.example", "auth_user", true, false)],
            visited: vec!["wiki.internal.example".into()],
        };
        let found = detect(AgentId::new(), Surface::Computer, &state, 100);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].domain, "internal.example");
        assert!(!found[0].recognized, "a guess must not be presented as a certainty");
    }

    #[test]
    fn a_session_id_handed_to_every_visitor_is_not_a_login() {
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
        let cookies = vec![cookie(".tracker.example", "authuser", true, false)];

        let unvisited =
            BrowserState { cookies: cookies.clone(), visited: vec!["news.example".into()] };
        assert!(detect(AgentId::new(), Surface::Computer, &unvisited, 100).is_empty());

        let visited = BrowserState { cookies, visited: vec!["tracker.example".into()] };
        assert_eq!(detect(AgentId::new(), Surface::Computer, &visited, 100).len(), 1);
    }

    #[test]
    fn a_cookie_that_dies_with_the_browser_cannot_prove_a_durable_sign_in() {
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
    fn hosts_are_normalized_however_chrome_spells_them() {
        assert_eq!(host_of(".www.LinkedIn.com"), "linkedin.com");
        assert_eq!(host_of("www.google.com"), "google.com");
        assert_eq!(host_of(".x.com"), "x.com");
        assert!(under("mail.google.com", "google.com"));
        assert!(under("google.com", "google.com"));
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
            recognized: true,
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
        assert_eq!(host_in("about:blank"), None);
        assert_eq!(host_in(""), None);
        assert_eq!(host_in("file:///home/user/inbox/report.pdf"), None);
        assert_eq!(host_in("localhost"), None);
    }

    #[test]
    fn credentials_in_the_authority_cannot_borrow_a_session() {
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
        assert!(on_domain("https://business.facebook.com/latest/inbox", "facebook.com"));
        assert!(on_domain("https://facebook.com/", "facebook.com"));
        assert!(!on_domain("https://notfacebook.com/", "facebook.com"));
        assert!(!on_domain("https://facebook.com@evil.com/x", "facebook.com"));
        assert!(!on_domain("not a url", "facebook.com"));
    }
}
