//! A decision waits for a person without holding a turn. An answer informs;
//! it never authorizes an external action. Completion requires a separate receipt.
use chrono::{DateTime, Duration, NaiveTime};
use serde::{Deserialize, Serialize};

use super::ids::{AgentId, DecisionId, GroupId, RunId};
use super::{instant, local};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionRequest {
    pub question: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub recommendation: String,
    #[serde(default)]
    pub options: Vec<String>,
    /// A source reference, drawn as text. Never treated as an executable URL.
    #[serde(default)]
    pub source: String,
}

impl DecisionRequest {
    pub fn validate(mut self) -> Result<Self, String> {
        fn text(value: &mut String, max: usize, required: bool) -> Result<(), String> {
            *value = value.trim().to_string();
            if (required && value.is_empty()) || value.chars().count() > max {
                return Err(format!(
                    "Use {} to {max} characters per field.",
                    usize::from(required)
                ));
            }
            Ok(())
        }
        text(&mut self.question, 300, true)?;
        text(&mut self.context, 1200, false)?;
        text(&mut self.recommendation, 600, false)?;
        text(&mut self.source, 600, false)?;
        if self.options.len() > 6 {
            return Err("Offer at most six choices.".into());
        }
        for option in &mut self.options {
            text(option, 120, true)?;
        }
        let unique: std::collections::HashSet<_> = self.options.iter().collect();
        if unique.len() != self.options.len() {
            return Err("Choices must be distinct.".into());
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DecisionStatus {
    Pending,
    Answered,
    Completed,
    Withdrawn,
}
impl DecisionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Answered => "answered",
            Self::Completed => "completed",
            Self::Withdrawn => "withdrawn",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkDecision {
    pub id: DecisionId,
    pub agent_id: AgentId,
    pub group_id: GroupId,
    pub topic: String,
    pub request: DecisionRequest,
    pub status: DecisionStatus,
    pub answer: Option<String>,
    pub outcome: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub due_at: Option<i64>,
    pub remind_at: i64,
    pub snoozed_until: Option<i64>,
    pub delivery_run: Option<RunId>,
    pub interrupted: bool,
}

/// Twice daily, in the host's local zone. The host is also where routine dates
/// are interpreted. Deadlines get one earlier reminder, then join the digest.
pub fn next_reminder(now: i64, due: Option<i64>) -> i64 {
    let regular = next_briefing(now);
    due.and_then(|at| at.checked_sub(60 * 60 * 1000))
        .filter(|at| *at > now)
        .map_or(regular, |at| regular.min(at))
}

pub fn next_briefing(now: i64) -> i64 {
    if let Some(local) = local(now) {
        for day in 0..=1 {
            if let Some(date) = local.date_naive().checked_add_signed(Duration::days(day)) {
                for hour in [9, 16] {
                    if let Some(at) =
                        instant(date.and_time(NaiveTime::from_hms_opt(hour, 0, 0).unwrap()))
                    {
                        if at > now {
                            return at;
                        }
                    }
                }
            }
        }
    }
    now.saturating_add(12 * 60 * 60 * 1000)
}

pub fn deadline(value: Option<&str>) -> Result<Option<i64>, String> {
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(value).map(|date| date.timestamp_millis()).map_err(|_| {
                "Use an RFC 3339 deadline with a timezone, or omit it when none was given.".into()
            })
        })
        .transpose()
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DecisionAction {
    Request {
        topic: String,
        #[serde(flatten)]
        request: DecisionRequest,
        due_at: Option<String>,
    },
    List,
    Complete {
        id: DecisionId,
        outcome: String,
    },
    Withdraw {
        id: DecisionId,
        outcome: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_requests_parse_with_their_flat_fields_and_deadline() {
        let action: DecisionAction=serde_json::from_str(r#"{"action":"request","topic":"mail/thread","question":"10 or 11?","options":["10","11"],"due_at":"2026-09-06T15:00:00-04:00"}"#).unwrap();
        let DecisionAction::Request { request, due_at, .. } = action else {
            panic!("wrong action")
        };
        assert_eq!(request.options, vec!["10", "11"]);
        assert!(deadline(due_at.as_deref()).unwrap().is_some());
    }
    #[test]
    fn ambiguous_deadlines_are_refused_and_briefings_advance() {
        assert!(deadline(Some("tomorrow")).is_err());
        let now = 1_700_000_000_000;
        let next = next_briefing(now);
        assert!(next > now);
        assert!(next <= now + 24 * 60 * 60 * 1000);
        assert!(next_briefing(next) > next);
        assert!(next_reminder(now, Some(now + 90 * 60 * 1000)) <= now + 30 * 60 * 1000);
    }
}
