use base64::Engine;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::domain::outbox::{Gathered, Kind};
use crate::domain::outcome::Status as Ended;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Format {
    #[default]
    Ntfy,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Content {
    #[default]
    Outcome,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said {
    pub title: String,
    pub body: String,
}

pub fn say(kind: Kind, item: &Gathered, count: usize, content: Content) -> Said {
    let said = |title: &str, body: &str| Said { title: title.into(), body: body.into() };
    match (kind, content) {
        (Kind::Reminder, _) => said("Needs you", &item.body),
        (Kind::NeedsYou, Content::Outcome) => {
            said("Needs you", &format!("{}. {}", item.title, item.body))
        }
        (Kind::NeedsYou, Content::Generic) => {
            said("Needs you", "An agent is waiting on you. Open Guaca to see what.")
        }
        (Kind::Failure, Content::Outcome) => said(&item.title, or_title(&item.body, &item.title)),
        (Kind::Failure, Content::Generic) => {
            said("Something could not finish", "Open Guaca to see what happened.")
        }
        (Kind::Outcome, Content::Outcome) => said(&item.title, or_title(&item.body, &item.title)),
        (Kind::Outcome, Content::Generic) if count > 1 => {
            said(&format!("{count} finished"), "Open For you in Guaca to see what.")
        }
        (Kind::Outcome, Content::Generic) => {
            said("Something finished", "Open For you in Guaca to see what.")
        }
    }
}

fn or_title<'a>(body: &'a str, title: &'a str) -> &'a str {
    if body.trim().is_empty() {
        title
    } else {
        body
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Push {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    pub agent: Option<String>,
    pub group: Option<String>,
    pub status: Option<Ended>,
    pub created_at: i64,
}

pub const DELIVERY_HEADER: &str = "X-Guaca-Delivery";

#[derive(Clone, PartialEq)]
pub struct Request {
    pub headers: Vec<(&'static str, String)>,
    pub body: String,
}

pub fn priority(kind: Kind) -> u8 {
    match kind {
        Kind::NeedsYou | Kind::Reminder | Kind::Failure => 4,
        Kind::Outcome => 3,
    }
}

pub fn request(format: Format, push: &Push, token: Option<&str>, link: Option<&str>) -> Request {
    let mut headers = vec![(DELIVERY_HEADER, push.id.clone())];
    if let Some(token) = token.map(str::trim).filter(|token| !token.is_empty()) {
        headers.push(("Authorization", authorization(token)));
    }
    let link = link.map(str::trim).filter(|link| !link.is_empty());
    match format {
        Format::Ntfy => {
            headers.push(("Content-Type", "text/plain; charset=utf-8".into()));
            headers.push(("Title", header_text(&push.title)));
            headers.push(("Priority", priority(push.kind).to_string()));
            headers.push(("Tags", "guaca".into()));
            if let Some(link) = link {
                headers.push(("Click", header_text(link)));
            }
            Request { headers, body: push.body.clone() }
        }
        Format::Json => {
            headers.push(("Content-Type", "application/json".into()));
            let body = serde_json::json!({
                "id": push.id,
                "kind": push.kind,
                "title": push.title,
                "body": push.body,
                "group": push.group,
                "agent": push.agent,
                "status": push.status,
                "createdAt": push.created_at,
                "link": link,
            });
            Request { headers, body: body.to_string() }
        }
    }
}

fn authorization(token: &str) -> String {
    let scheme = token.split_once(' ').map(|(scheme, _)| scheme.to_ascii_lowercase());
    match scheme.as_deref() {
        Some("bearer" | "basic") => token.to_string(),
        _ => format!("Bearer {token}"),
    }
}

fn header_text(value: &str) -> String {
    let flat: String = value.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = flat.trim();
    if flat.is_ascii() {
        return flat.to_string();
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(flat.as_bytes());
    format!("=?UTF-8?B?{encoded}?=")
}

pub const GIVE_UP_MS: i64 = 24 * 60 * 60 * 1000;

const BACKOFF_MS: [i64; 4] = [30_000, 2 * 60_000, 10 * 60_000, 60 * 60_000];

pub const TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Accepted,
    Refused { status: u16, retry_after_ms: Option<i64> },
    Unreachable(Unreachable),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreachable {
    Timeout,
    Connect,
    Invalid,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    Sent,
    Again { at: i64, error: String },
    Failed { error: String },
}

pub fn next(answer: Answer, attempts: u32, created_at: i64, now: i64) -> Next {
    let (error, wait) = match answer {
        Answer::Accepted => return Next::Sent,
        Answer::Refused { status, .. }
            if (300..500).contains(&status) && status != 408 && status != 429 =>
        {
            return Next::Failed { error: refused(status) };
        }
        Answer::Refused { status: 429, retry_after_ms: Some(after) } => {
            (refused(429), after.clamp(1_000, GIVE_UP_MS))
        }
        Answer::Refused { status, .. } => (refused(status), backoff(attempts)),
        Answer::Unreachable(Unreachable::Invalid) => {
            return Next::Failed { error: unreachable(Unreachable::Invalid) };
        }
        Answer::Unreachable(why) => (unreachable(why), backoff(attempts)),
    };
    let at = now + wait;
    if at > created_at + GIVE_UP_MS {
        Next::Failed { error }
    } else {
        Next::Again { at, error }
    }
}

fn backoff(attempts: u32) -> i64 {
    let index = (attempts.max(1) as usize - 1).min(BACKOFF_MS.len() - 1);
    BACKOFF_MS[index]
}

pub fn refused(status: u16) -> String {
    let reason = StatusCode::from_u16(status)
        .ok()
        .and_then(|code| code.canonical_reason())
        .unwrap_or("Refused");
    let check = match status {
        401 | 403 => " Check the token.",
        404 | 300..=399 => " Check the address.",
        _ => "",
    };
    format!("{status} {reason}.{check}")
}

pub fn unreachable(why: Unreachable) -> String {
    match why {
        Unreachable::Timeout => format!("No answer within {TIMEOUT_SECS} seconds."),
        Unreachable::Connect => "Could not connect. Check the address.".into(),
        Unreachable::Invalid => {
            "The address or the token has characters a request cannot carry.".into()
        }
        Unreachable::Other => "The request did not complete.".into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub last_sent_at: Option<i64>,
    pub last_attempt_at: Option<i64>,
    pub last_error: Option<String>,
    pub retry_at: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::AgentId;
    use crate::domain::outbox::Target;

    const NOW: i64 = 1_800_000_000_000;

    fn item(title: &str, body: &str) -> Gathered {
        Gathered {
            title: title.into(),
            body: body.into(),
            target: Target::Channel { agent_id: AgentId::new() },
        }
    }

    fn push(kind: Kind) -> Push {
        Push {
            id: "d-1".into(),
            kind,
            title: "Digest".into(),
            body: "Sent the digest to 12 people.".into(),
            agent: Some("Digest".into()),
            group: Some("Mornings".into()),
            status: Some(Ended::Done),
            created_at: NOW,
        }
    }

    fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
        request.headers.iter().find(|(key, _)| *key == name).map(|(_, value)| value.as_str())
    }

    #[test]
    fn a_finished_run_says_its_own_line_and_generic_says_only_that_it_finished() {
        let digest = item("Digest", "Sent the digest to 12 people.");
        assert_eq!(
            say(Kind::Outcome, &digest, 1, Content::Outcome),
            Said { title: "Digest".into(), body: "Sent the digest to 12 people.".into() }
        );
        let generic = say(Kind::Outcome, &digest, 1, Content::Generic);
        assert_eq!(generic.title, "Something finished");
        assert!(!generic.body.contains("digest") && !generic.title.contains("Digest"));

        let four = item("4 finished", "R1: Sent.\nR2: Drafted.");
        assert_eq!(say(Kind::Outcome, &four, 4, Content::Outcome).body, "R1: Sent.\nR2: Drafted.");
        let generic = say(Kind::Outcome, &four, 4, Content::Generic);
        assert_eq!(generic.title, "4 finished");
        assert!(!generic.body.contains("R1"), "{generic:?}");

        let blank = item("Digest", "  ");
        assert_eq!(say(Kind::Outcome, &blank, 1, Content::Outcome).body, "Digest");
    }

    #[test]
    fn what_needs_the_operator_is_titled_so_and_generic_names_nobody() {
        let asked = item("Pip has a question", "Its turn is parked until you answer.");
        let said = say(Kind::NeedsYou, &asked, 1, Content::Outcome);
        assert_eq!(said.title, "Needs you");
        assert_eq!(said.body, "Pip has a question. Its turn is parked until you answer.");
        let generic = say(Kind::NeedsYou, &asked, 1, Content::Generic);
        assert!(!generic.body.contains("Pip"), "{generic:?}");

        let reminder = item("For you", "2 decisions need a review.");
        for content in [Content::Outcome, Content::Generic] {
            let said = say(Kind::Reminder, &reminder, 1, content);
            assert_eq!(
                (said.title.as_str(), said.body.as_str()),
                ("Needs you", "2 decisions need a review.")
            );
        }

        let failed = item("Digest could not finish", "The model call failed.");
        assert_eq!(
            say(Kind::Failure, &failed, 1, Content::Outcome).title,
            "Digest could not finish"
        );
        let generic = say(Kind::Failure, &failed, 1, Content::Generic);
        assert!(!generic.title.contains("Digest") && !generic.body.contains("model"));
    }

    #[test]
    fn an_ntfy_push_is_the_line_with_the_rest_in_headers() {
        let sent = request(Format::Ntfy, &push(Kind::Outcome), Some("tk_secret"), None);
        assert_eq!(sent.body, "Sent the digest to 12 people.");
        assert_eq!(header(&sent, "Title"), Some("Digest"));
        assert_eq!(header(&sent, "Priority"), Some("3"));
        assert_eq!(header(&sent, "Tags"), Some("guaca"));
        assert_eq!(header(&sent, "Authorization"), Some("Bearer tk_secret"));
        assert_eq!(header(&sent, DELIVERY_HEADER), Some("d-1"));
        assert_eq!(header(&sent, "Click"), None, "no link base, no click");

        let sent = request(Format::Ntfy, &push(Kind::NeedsYou), None, Some("https://box.example/"));
        assert_eq!(header(&sent, "Priority"), Some("4"));
        assert_eq!(header(&sent, "Click"), Some("https://box.example/"));
        assert_eq!(header(&sent, "Authorization"), None, "no token, no header");
        assert_eq!(priority(Kind::Failure), 4);
        assert_eq!(priority(Kind::Reminder), 4);
    }

    #[test]
    fn a_json_push_is_one_object_with_every_field() {
        let sent = request(
            Format::Json,
            &push(Kind::Outcome),
            Some("tk_secret"),
            Some("https://box.example"),
        );
        let body: serde_json::Value = serde_json::from_str(&sent.body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "id": "d-1",
                "kind": "outcome",
                "title": "Digest",
                "body": "Sent the digest to 12 people.",
                "group": "Mornings",
                "agent": "Digest",
                "status": "done",
                "createdAt": NOW,
                "link": "https://box.example",
            })
        );
        assert_eq!(header(&sent, "Content-Type"), Some("application/json"));
        assert_eq!(header(&sent, "Authorization"), Some("Bearer tk_secret"));
        assert_eq!(header(&sent, DELIVERY_HEADER), Some("d-1"));
        assert_eq!(header(&sent, "Title"), None, "the title is in the body");
        assert!(!sent.body.contains("tk_secret"));
    }

    #[test]
    fn a_pasted_scheme_is_kept_and_a_header_never_breaks_a_line() {
        let with = |token: &str| {
            let sent = request(Format::Ntfy, &push(Kind::Outcome), Some(token), None);
            header(&sent, "Authorization").map(str::to_string)
        };
        assert_eq!(with("Bearer tk_a").as_deref(), Some("Bearer tk_a"));
        assert_eq!(with("Basic dXNlcjpwYXNz").as_deref(), Some("Basic dXNlcjpwYXNz"));
        assert_eq!(with("  ").as_deref(), None);

        let mut named = push(Kind::Outcome);
        named.title = "Bob\r\nX-Evil: 1".into();
        let sent = request(Format::Ntfy, &named, None, None);
        assert!(!header(&sent, "Title").unwrap().contains('\n'));

        named.title = "Café".into();
        let sent = request(Format::Ntfy, &named, None, None);
        assert_eq!(header(&sent, "Title"), Some("=?UTF-8?B?Q2Fmw6k=?="));
    }

    #[test]
    fn retries_back_off_and_stop_after_a_day() {
        let failing = Answer::Refused { status: 500, retry_after_ms: None };
        let waits: Vec<i64> = (1..=5)
            .map(|attempt| match next(failing, attempt, NOW, NOW) {
                Next::Again { at, .. } => at - NOW,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(waits, [30_000, 120_000, 600_000, 3_600_000, 3_600_000]);

        let late = NOW + GIVE_UP_MS - 1_000;
        assert_eq!(
            next(failing, 20, NOW, late),
            Next::Failed { error: "500 Internal Server Error.".into() },
            "a retry past a day is not scheduled"
        );
        assert_eq!(next(Answer::Accepted, 3, NOW, late), Next::Sent);

        let lost = Answer::Unreachable(Unreachable::Timeout);
        assert!(matches!(next(lost, 1, NOW, NOW), Next::Again { at, .. } if at == NOW + 30_000));
        assert!(matches!(
            next(Answer::Refused { status: 408, retry_after_ms: None }, 1, NOW, NOW),
            Next::Again { .. }
        ));
    }

    #[test]
    fn a_refusal_that_will_not_change_fails_at_once_and_says_what_to_check() {
        let refused = |status| next(Answer::Refused { status, retry_after_ms: None }, 1, NOW, NOW);
        assert_eq!(
            refused(401),
            Next::Failed { error: "401 Unauthorized. Check the token.".into() }
        );
        assert_eq!(refused(403), Next::Failed { error: "403 Forbidden. Check the token.".into() });
        assert_eq!(
            refused(404),
            Next::Failed { error: "404 Not Found. Check the address.".into() }
        );
        assert_eq!(
            refused(301),
            Next::Failed { error: "301 Moved Permanently. Check the address.".into() },
            "a redirect is not followed, so the token goes nowhere it was not sent"
        );
        assert!(matches!(refused(413), Next::Failed { .. }));
        assert!(matches!(
            next(Answer::Unreachable(Unreachable::Invalid), 1, NOW, NOW),
            Next::Failed { .. }
        ));
    }

    #[test]
    fn too_many_requests_waits_as_long_as_it_is_told() {
        let told = Answer::Refused { status: 429, retry_after_ms: Some(90_000) };
        assert_eq!(
            next(told, 1, NOW, NOW),
            Next::Again { at: NOW + 90_000, error: "429 Too Many Requests.".into() }
        );
        let untold = Answer::Refused { status: 429, retry_after_ms: None };
        assert!(matches!(next(untold, 2, NOW, NOW), Next::Again { at, .. } if at == NOW + 120_000));
    }

    #[test]
    fn nothing_a_person_reads_has_an_em_dash() {
        let mut said: Vec<String> = [401, 403, 404, 301, 429, 500, 503].map(refused).to_vec();
        for why in
            [Unreachable::Timeout, Unreachable::Connect, Unreachable::Invalid, Unreachable::Other]
        {
            said.push(unreachable(why));
        }
        let anything = item("A", "B");
        for kind in [Kind::Outcome, Kind::NeedsYou, Kind::Reminder, Kind::Failure] {
            for content in [Content::Outcome, Content::Generic] {
                let Said { title, body } = say(kind, &anything, 2, content);
                said.extend([title, body]);
            }
        }
        for line in said {
            assert!(!line.contains('\u{2014}'), "{line}");
        }
    }

    #[test]
    fn formats_and_contents_are_spelled_for_the_wire() {
        assert_eq!(serde_json::to_value(Format::Ntfy).unwrap(), "ntfy");
        assert_eq!(serde_json::to_value(Format::Json).unwrap(), "json");
        assert_eq!(serde_json::to_value(Content::Generic).unwrap(), "generic");
        assert_eq!(Format::default(), Format::Ntfy);
        assert_eq!(Content::default(), Content::Outcome);
    }
}
