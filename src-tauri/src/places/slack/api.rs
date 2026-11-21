use std::time::Duration;

use reqwest::header::{HeaderMap, CONTENT_TYPE, RETRY_AFTER};
use serde_json::Value;

use crate::places::{AdapterError, SendError};
use crate::secrets::Secret;

const TIMEOUT: Duration = Duration::from_secs(30);
const CODE_CHARS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code(String);

impl Code {
    pub fn parse(raw: &str) -> Self {
        let plain = !raw.is_empty()
            && raw.len() <= CODE_CHARS
            && raw.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        Self(if plain { raw.to_string() } else { "unknown_error".to_string() })
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Limited(Option<Duration>),
    Code(Code),
    Status(u16),
    Unreachable,
    Garbled,
}

impl Failure {
    pub fn is(&self, code: &str) -> bool {
        matches!(self, Failure::Code(found) if found.as_str() == code)
    }

    fn refused_token(&self) -> Option<&'static str> {
        let Failure::Code(code) = self else { return None };
        match code.as_str() {
            "invalid_auth" | "not_authed" => Some(
                "Slack refused the token. Generate a new one and paste it in Guaca's Slack settings.",
            ),
            "token_revoked" => Some(
                "Slack revoked the token. Reinstall the Guaca app to the workspace and paste its new token in Guaca's Slack settings.",
            ),
            "account_inactive" => Some(
                "Slack says the app's account is inactive. Reinstall the Guaca app to the workspace and paste its new token in Guaca's Slack settings.",
            ),
            _ => None,
        }
    }

    fn gone(&self) -> Option<&'static str> {
        let Failure::Code(code) = self else { return None };
        match code.as_str() {
            "channel_not_found" => Some(
                "Slack cannot find that conversation. Invite the Guaca app to it, or stop answering there.",
            ),
            "not_in_channel" => Some(
                "The Guaca app is not in that channel. Invite it with /invite @Guaca, or stop answering there.",
            ),
            "is_archived" => {
                Some("That channel is archived. Unarchive it, or stop answering there.")
            }
            _ => None,
        }
    }

    fn passing(&self) -> bool {
        match self {
            Failure::Limited(_) | Failure::Unreachable | Failure::Garbled => true,
            Failure::Status(status) => *status >= 500,
            Failure::Code(code) => matches!(
                code.as_str(),
                "internal_error" | "fatal_error" | "service_unavailable" | "request_timeout"
            ),
        }
    }

    pub fn reason(&self) -> String {
        if let Some(reason) = self.refused_token().or_else(|| self.gone()) {
            return reason.to_string();
        }
        match self {
            Failure::Limited(_) => {
                "Slack is limiting how fast Guaca may call it. Guaca will try again.".to_string()
            }
            Failure::Unreachable => "Slack could not be reached. Guaca will try again.".to_string(),
            Failure::Garbled => {
                "Slack answered with something Guaca could not read. Guaca will try again."
                    .to_string()
            }
            Failure::Status(status) if *status >= 500 => {
                format!("Slack answered with HTTP status {status}. Guaca will try again.")
            }
            Failure::Status(status) => format!("Slack answered with HTTP status {status}."),
            Failure::Code(code) if self.passing() => {
                format!("Slack could not do it just now ({}). Guaca will try again.", code.as_str())
            }
            Failure::Code(code) => format!("Slack refused the request ({}).", code.as_str()),
        }
    }

    pub fn into_send(self) -> SendError {
        match self {
            Failure::Limited(after) => SendError::Retry { after },
            _ if self.gone().is_some() => SendError::Gone { reason: self.reason() },
            _ if self.passing() => SendError::Retry { after: None },
            _ => SendError::Refused { reason: self.reason() },
        }
    }

    pub fn into_post(self) -> SendError {
        let ambiguous = match &self {
            Failure::Limited(_) => false,
            Failure::Status(status) => {
                !matches!(status, 400 | 401 | 403 | 404 | 405 | 413 | 415 | 422)
            }
            Failure::Code(code) => self.passing() || code.as_str() == "unknown_error",
            Failure::Unreachable | Failure::Garbled => true,
        };
        if ambiguous {
            SendError::Ambiguous { after: None }
        } else {
            self.into_send()
        }
    }

    pub fn into_adapter(self) -> AdapterError {
        AdapterError { retry: self.passing(), reason: self.reason() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Bot,
    App,
}

pub enum Body {
    Json(Value),
    Query(Vec<(&'static str, String)>),
    Empty,
}

pub struct Api {
    http: reqwest::Client,
    origin: String,
    bot: Secret,
    app: Secret,
}

impl Api {
    pub fn new(origin: String, bot: Secret, app: Secret) -> Self {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .build()
            .expect("Slack HTTP client setup failed");
        Self { http, origin, bot, app }
    }

    pub async fn call(&self, method: &str, token: Token, body: Body) -> Result<Value, Failure> {
        let secret = match token {
            Token::Bot => &self.bot,
            Token::App => &self.app,
        };
        let url = format!("{}/api/{method}", self.origin);
        let request = match body {
            Body::Json(value) => self
                .http
                .post(url)
                .header(CONTENT_TYPE, "application/json; charset=utf-8")
                .body(value.to_string()),
            Body::Query(pairs) => self.http.get(url).query(&pairs),
            Body::Empty => self.http.post(url),
        };
        let response =
            request.bearer_auth(secret.expose()).send().await.map_err(|_| Failure::Unreachable)?;
        let status = response.status();
        let after = retry_after(response.headers());
        if status.as_u16() == 429 {
            return Err(Failure::Limited(after));
        }
        if !status.is_success() {
            return Err(Failure::Status(status.as_u16()));
        }
        let value: Value = response.json().await.map_err(|_| Failure::Garbled)?;
        match value["ok"].as_bool() {
            Some(true) => return Ok(value),
            Some(false) => {}
            None => return Err(Failure::Garbled),
        }
        let raw = value["error"].as_str().unwrap_or_default();
        let code = if [&self.bot, &self.app]
            .iter()
            .any(|secret| !secret.is_empty() && raw.contains(secret.expose()))
        {
            Code::parse("")
        } else {
            Code::parse(raw)
        };
        match code.as_str() {
            "ratelimited" => Err(Failure::Limited(after)),
            _ => Err(Failure::Code(code)),
        }
    }
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers.get(RETRY_AFTER)?.to_str().ok()?.trim().parse::<u64>().ok().map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(raw: &str) -> Failure {
        Failure::Code(Code::parse(raw))
    }

    #[test]
    fn a_code_that_is_not_a_plain_slack_code_is_never_repeated() {
        assert_eq!(Code::parse("xoxb-test-not-a-token").as_str(), "unknown_error");
        assert_eq!(Code::parse("https://slack.com/api").as_str(), "unknown_error");
        assert_eq!(Code::parse("").as_str(), "unknown_error");
        assert_eq!(Code::parse(&"a".repeat(CODE_CHARS + 1)).as_str(), "unknown_error");
        assert_eq!(Code::parse("not_in_channel").as_str(), "not_in_channel");
    }

    #[test]
    fn auth_codes_are_refusals_that_say_what_to_do_and_stop_the_connection() {
        for raw in ["invalid_auth", "not_authed", "token_revoked", "account_inactive"] {
            let SendError::Refused { reason } = code(raw).into_send() else {
                panic!("{raw} must refuse");
            };
            assert!(reason.contains("paste"), "{raw}: {reason}");
            assert!(!code(raw).into_adapter().retry, "{raw} must not be retried");
        }
    }

    #[test]
    fn a_missing_channel_is_gone_and_a_limit_or_an_outage_is_retried() {
        for raw in ["channel_not_found", "not_in_channel", "is_archived"] {
            assert!(matches!(code(raw).into_send(), SendError::Gone { .. }), "{raw}");
        }
        assert_eq!(
            Failure::Limited(Some(Duration::from_secs(7))).into_send(),
            SendError::Retry { after: Some(Duration::from_secs(7)) }
        );
        assert_eq!(Failure::Status(503).into_send(), SendError::Retry { after: None });
        assert_eq!(Failure::Unreachable.into_send(), SendError::Retry { after: None });
        assert!(matches!(Failure::Status(404).into_send(), SendError::Refused { .. }));
        assert!(matches!(code("invalid_blocks").into_send(), SendError::Refused { .. }));
        assert!(Failure::Unreachable.into_adapter().retry);
    }

    #[test]
    fn no_reason_reads_like_a_shell_or_names_an_address() {
        let all = [
            code("invalid_auth"),
            code("token_revoked"),
            code("account_inactive"),
            code("channel_not_found"),
            code("not_in_channel"),
            code("is_archived"),
            code("internal_error"),
            code("invalid_blocks"),
            Failure::Limited(None),
            Failure::Status(302),
            Failure::Status(502),
            Failure::Unreachable,
            Failure::Garbled,
        ];
        for failure in all {
            let reason = failure.reason();
            assert!(!reason.contains("http://") && !reason.contains("https://"), "{reason}");
            assert!(!reason.contains('\u{2014}'), "{reason}");
            assert!(reason.ends_with('.'), "{reason}");
        }
    }
}
