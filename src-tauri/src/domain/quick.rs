//! A quick action: one button on the status bar for something the operator
//! does often.
//!
//! Declarative on purpose. Hermes lets an agent write JavaScript that runs in
//! the window with the app's whole authority, and says in its own loader that
//! this is not a capability boundary. Here a button is data from a closed set
//! of things it can do, drawn by the app's own code, so an agent that adds one
//! has chosen a label and a destination and nothing else.
//!
//! One of those destinations is not harmless, and it decides the rest of the
//! design. A button that sends a message sends it as the operator: the text an
//! agent wrote arrives in another agent's channel as `[OPERATOR]`, on a click
//! the operator makes without reading it again. So an agent never puts a
//! button there on its own. Its request is a settings change, approved on the
//! desk with the whole text shown, and the one click after that is the
//! operator's own words because they read them once and said yes.

use serde::{Deserialize, Serialize};

use super::ids::{AgentId, GroupId};

/// How many fit on the bar. Past this it is a menu, and a menu is not quick.
pub const MAX_ACTIONS: usize = 8;

/// A button's label. Two or three words.
pub const MAX_LABEL: usize = 24;

/// What a button sends. A message, not a brief.
pub const MAX_TEXT: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickAction {
    pub id: String,
    pub label: String,
    pub does: Does,
    /// Who put it there: "the operator", or the agent that asked for it.
    pub added_by: String,
}

/// What pressing it does. A closed set: the app draws and runs each one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Does {
    /// Sends this text to this agent, as the operator would from its channel,
    /// and opens that channel to show the answer arriving.
    Message { agent_id: AgentId, text: String },
    /// Opens a place in the app.
    Open { place: Place },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Place {
    Channel {
        agent_id: AgentId,
    },
    Calendar,
    ForYou,
    Settings {
        #[serde(default)]
        section: Option<String>,
    },
    CrewSettings {
        group_id: GroupId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuickError {
    #[error("a quick action needs a label: two or three words for the button")]
    NoLabel,
    #[error("the label is {0} characters and a button holds {MAX_LABEL}. Say it in fewer words")]
    LongLabel(usize),
    #[error("a quick action that sends a message needs the message")]
    NoText,
    #[error(
        "the message is {0} characters and a button sends at most {MAX_TEXT}. Put the detail in a \
         skill and have the message point at it"
    )]
    LongText(usize),
    #[error(
        "the status bar already holds {MAX_ACTIONS} quick actions. Remove one before adding another"
    )]
    Full,
    #[error("no quick action has the id `{0}`. `read` lists them with their ids")]
    Unknown(String),
}

impl QuickAction {
    /// A button fit to put on the bar, with a fresh id.
    pub fn new(label: &str, does: Does, added_by: &str) -> Result<Self, QuickError> {
        let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
        if label.is_empty() {
            return Err(QuickError::NoLabel);
        }
        let length = label.chars().count();
        if length > MAX_LABEL {
            return Err(QuickError::LongLabel(length));
        }
        let does = match does {
            Does::Message { agent_id, text } => {
                let text = text.trim().to_string();
                if text.is_empty() {
                    return Err(QuickError::NoText);
                }
                let length = text.chars().count();
                if length > MAX_TEXT {
                    return Err(QuickError::LongText(length));
                }
                Does::Message { agent_id, text }
            }
            Does::Open { place: Place::Settings { section } } => {
                // A pane name, or none: it reaches the page as a route.
                let section = section.filter(|name| {
                    !name.is_empty()
                        && name.len() <= 32
                        && name.bytes().all(|b| b.is_ascii_lowercase())
                });
                Does::Open { place: Place::Settings { section } }
            }
            other => other,
        };
        let id = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
        Ok(Self { id, label, does, added_by: added_by.trim().to_string() })
    }

    /// What it does, in one line, with names the caller looked up. The words
    /// the operator approves are these, so the message is quoted whole.
    pub fn describe(&self, agent: impl Fn(AgentId) -> String) -> String {
        match &self.does {
            Does::Message { agent_id, text } => {
                format!(
                    "\u{201c}{}\u{201d} sends {}: \u{201c}{text}\u{201d}",
                    self.label,
                    agent(*agent_id)
                )
            }
            Does::Open { place } => {
                let place = match place {
                    Place::Channel { agent_id } => format!("the channel with {}", agent(*agent_id)),
                    Place::Calendar => "the calendar".to_string(),
                    Place::ForYou => "For You".to_string(),
                    Place::Settings { section: Some(section) } => format!("Settings on {section}"),
                    Place::Settings { section: None } => "Settings".to_string(),
                    Place::CrewSettings { .. } => "a crew's settings".to_string(),
                };
                format!("\u{201c}{}\u{201d} opens {place}", self.label)
            }
        }
    }
}

/// Adds one to the bar, refusing a full one.
pub fn add(bar: &mut Vec<QuickAction>, action: QuickAction) -> Result<(), QuickError> {
    if bar.len() >= MAX_ACTIONS {
        return Err(QuickError::Full);
    }
    bar.push(action);
    Ok(())
}

/// Takes one off the bar by id.
pub fn remove(bar: &mut Vec<QuickAction>, id: &str) -> Result<QuickAction, QuickError> {
    let at = bar.iter().position(|action| action.id == id).ok_or(QuickError::Unknown(id.into()))?;
    Ok(bar.remove(at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(text: &str) -> Does {
        Does::Message { agent_id: AgentId::new(), text: text.into() }
    }

    #[test]
    fn a_button_has_a_short_label_and_a_message_it_can_hold() {
        let made = QuickAction::new("  Morning   brief ", message(" Brief me. "), "Pip").unwrap();
        assert_eq!(made.label, "Morning brief");
        assert_eq!(made.id.len(), 8);
        assert!(matches!(made.does, Does::Message { ref text, .. } if text == "Brief me."));
        assert_eq!(QuickAction::new(" ", message("x"), "Pip"), Err(QuickError::NoLabel));
        assert!(matches!(
            QuickAction::new(&"w".repeat(MAX_LABEL + 1), message("x"), "Pip"),
            Err(QuickError::LongLabel(_))
        ));
        assert_eq!(QuickAction::new("Go", message("  "), "Pip"), Err(QuickError::NoText));
        assert!(matches!(
            QuickAction::new("Go", message(&"w".repeat(MAX_TEXT + 1)), "Pip"),
            Err(QuickError::LongText(_))
        ));
    }

    #[test]
    fn a_settings_route_is_one_word_or_nothing() {
        let place = Place::Settings { section: Some("limits; drop".into()) };
        let made = QuickAction::new("Limits", Does::Open { place }, "you").unwrap();
        assert_eq!(made.does, Does::Open { place: Place::Settings { section: None } });
    }

    #[test]
    fn the_bar_holds_a_few_and_says_so_when_full() {
        let mut bar = Vec::new();
        for n in 0..MAX_ACTIONS {
            add(&mut bar, QuickAction::new(&format!("B{n}"), message("x"), "you").unwrap())
                .unwrap();
        }
        let one_more = QuickAction::new("More", message("x"), "you").unwrap();
        assert_eq!(add(&mut bar, one_more), Err(QuickError::Full));
        let first = bar[0].id.clone();
        assert_eq!(remove(&mut bar, &first).unwrap().label, "B0");
        assert!(matches!(remove(&mut bar, &first), Err(QuickError::Unknown(_))));
    }

    #[test]
    fn what_the_operator_approves_quotes_the_whole_message() {
        let made = QuickAction::new("Brief", message("Give me the brief."), "Pip").unwrap();
        let said = made.describe(|_| "Scout".into());
        assert_eq!(said, "\u{201c}Brief\u{201d} sends Scout: \u{201c}Give me the brief.\u{201d}");
        // What crosses IPC is spelled the way the frontend spells it.
        let wire = serde_json::to_value(&made).unwrap();
        assert_eq!(wire["does"]["kind"], "message");
        assert!(wire["does"].get("agentId").is_some(), "{wire}");
        assert!(wire.get("addedBy").is_some(), "{wire}");
    }
}
