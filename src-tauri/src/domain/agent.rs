//! Agent identity and capability description.
//!
//! `AgentCard` is lifted from A2A's Agent Card: a self-describing document that
//! says who an agent is and what it can do, so peers can discover it without
//! out-of-band configuration. Guac keeps the useful half (name, skills,
//! version, lifecycle) and drops the parts that only pay off across a trust
//! boundary that a local single-process app does not have: no `.well-known`
//! hosting, no DIDs, no signatures, no registry.

use serde::{Deserialize, Serialize};

use super::ids::{AgentId, GroupId};

/// Where an agent is in its lifecycle.
///
/// The survey models agents as Creation -> Operation -> Update -> Termination.
/// Creation and Update are transitions rather than resting states, so only the
/// three states an agent can actually sit in are represented here. Collapsing
/// the other two removes states that could never be observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    /// Accepting and processing envelopes.
    Active,
    /// Reachable in the directory, but queues rather than processes.
    /// Useful for pinning one agent still while you watch the others.
    Paused,
    /// Drained and closed. Retained for transcript history, invisible to peers.
    Terminated,
}

impl Lifecycle {
    /// Whether peers should see this agent when they call `directory`.
    pub fn is_discoverable(self) -> bool {
        matches!(self, Lifecycle::Active | Lifecycle::Paused)
    }

    /// Whether the actor should pull from its inbox right now.
    pub fn accepts_work(self) -> bool {
        matches!(self, Lifecycle::Active)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Lifecycle::Active => "active",
            Lifecycle::Paused => "paused",
            Lifecycle::Terminated => "terminated",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Lifecycle::Active),
            "paused" => Some(Lifecycle::Paused),
            "terminated" => Some(Lifecycle::Terminated),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub id: AgentId,
    /// The isolation boundary this agent sits in. Peers outside it are not
    /// listed by `directory` and cannot be addressed; see `domain::group`.
    pub group_id: GroupId,
    pub name: String,
    /// Key into the frontend avatar catalog, not a drawing. Storing the key
    /// means a character can be redrawn entirely without touching stored data.
    pub avatar: String,
    /// Accent color as `#rrggbb`. Validated on write, never on read.
    pub color: String,
    /// OpenRouter model slug. Per-agent so a cheap agent and an expensive one
    /// can share a room.
    pub model: String,
    pub system_prompt: String,
    /// Free-text capability lines. This is what peers actually read when they
    /// decide who to talk to, so it is the highest-leverage field on the card.
    pub skills: Vec<String>,
    /// The sandbox this agent uses as its computer, once it has been given one,
    /// and the tokens that reach it. Never set by an operator edit.
    pub sandbox_id: Option<String>,
    /// Never sent to the webview in a form it could use directly; the viewer
    /// goes through the local proxy, which holds these.
    #[serde(skip_serializing)]
    pub sandbox_envd_token: Option<String>,
    #[serde(skip_serializing)]
    pub sandbox_traffic_token: Option<String>,
    pub lifecycle: Lifecycle,
    /// Bumped on every update. A2A's Update phase exists so peers can detect
    /// that a card changed under them; the version is what makes that possible.
    pub version: u32,
    pub created_at: i64,
    pub updated_at: i64,
}

impl AgentCard {
    /// The directory entry a peer sees. Deliberately excludes `system_prompt`:
    /// one agent should not be able to read another's instructions just by
    /// listing the directory.
    ///
    /// `reaches` is passed in rather than read from the card because accounts
    /// live in their own table, and because deciding what a peer may know about
    /// them is a judgement the caller has to make deliberately.
    pub fn directory_entry(&self, reaches: Vec<String>) -> DirectoryEntry {
        DirectoryEntry {
            id: self.id,
            name: self.name.clone(),
            skills: self.skills.clone(),
            reaches,
            lifecycle: self.lifecycle,
            version: self.version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub id: AgentId,
    pub name: String,
    pub skills: Vec<String>,
    /// Accounts signed in on this agent's machine, as `Gmail as robert@…`.
    ///
    /// A skill is a claim its agent wrote about itself; this is a fact the
    /// operator established. It is here so an agent asked for something it has
    /// no account for can name the peer that does, instead of reporting that
    /// the crew cannot do it.
    #[serde(default)]
    pub reaches: Vec<String>,
    pub lifecycle: Lifecycle,
    pub version: u32,
}

/// Fields an operator can set. Separate from `AgentCard` so that `id`,
/// `version`, and timestamps cannot be forged across the IPC boundary.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDraft {
    /// Absent means "leave it where it is" on update, and "the default group"
    /// on create. The UI omits it entirely until a second group exists.
    #[serde(default)]
    pub group_id: Option<GroupId>,
    pub name: String,
    pub avatar: String,
    pub color: String,
    pub model: String,
    pub system_prompt: String,
    #[serde(default)]
    pub skills: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DraftError {
    #[error("name must not be blank")]
    BlankName,
    #[error("name must be {max} characters or fewer")]
    NameTooLong { max: usize },
    #[error("color must be a #rrggbb hex string, got {got:?}")]
    BadColor { got: String },
    #[error("avatar must not be blank")]
    BlankAvatar,
    #[error("an agent named {name:?} already exists")]
    DuplicateName { name: String },
}

pub const MAX_NAME_LEN: usize = 48;

impl AgentDraft {
    /// Normalizes and validates operator input.
    ///
    /// Returns the cleaned values rather than mutating in place so that a
    /// rejected draft leaves no half-applied state behind.
    pub fn validate(&self) -> Result<CleanDraft, DraftError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(DraftError::BlankName);
        }
        if name.chars().count() > MAX_NAME_LEN {
            return Err(DraftError::NameTooLong { max: MAX_NAME_LEN });
        }

        let avatar = self.avatar.trim();
        if avatar.is_empty() {
            return Err(DraftError::BlankAvatar);
        }

        // Blank is legal and means "inherit": the group's model, or the app
        // default if the group does not name one. Requiring a model here was
        // what made a group-level model impossible to express.
        let model = self.model.trim();

        let color = normalize_color(&self.color)
            .ok_or_else(|| DraftError::BadColor { got: self.color.clone() })?;

        Ok(CleanDraft {
            group_id: self.group_id,
            name: name.to_string(),
            avatar: avatar.to_string(),
            color,
            model: model.to_string(),
            system_prompt: self.system_prompt.trim().to_string(),
            skills: self
                .skills
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CleanDraft {
    pub group_id: Option<GroupId>,
    pub name: String,
    pub avatar: String,
    pub color: String,
    pub model: String,
    pub system_prompt: String,
    pub skills: Vec<String>,
}

/// Accepts `#rgb` and `#rrggbb`, with or without the leading `#`, and returns
/// a canonical lowercase `#rrggbb`.
fn normalize_color(input: &str) -> Option<String> {
    let hex = input.trim().trim_start_matches('#');
    let expanded = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => hex.to_string(),
        _ => return None,
    };
    if !expanded.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("#{}", expanded.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> AgentDraft {
        AgentDraft {
            group_id: None,
            name: "  Manager  ".into(),
            avatar: "avocado".into(),
            color: "#7FB069".into(),
            model: "anthropic/claude-sonnet-4.5".into(),
            system_prompt: "  You coordinate.  ".into(),
            skills: vec!["  delegation  ".into(), "   ".into()],
        }
    }

    #[test]
    fn validate_trims_and_canonicalizes() {
        let clean = draft().validate().unwrap();
        assert_eq!(clean.name, "Manager");
        assert_eq!(clean.color, "#7fb069");
        assert_eq!(clean.system_prompt, "You coordinate.");
        assert_eq!(clean.skills, vec!["delegation".to_string()], "blank skills are dropped");
    }

    #[test]
    fn blank_name_is_rejected() {
        let mut d = draft();
        d.name = "   ".into();
        assert_eq!(d.validate(), Err(DraftError::BlankName));
    }

    #[test]
    fn overlong_name_is_rejected_by_character_count_not_bytes() {
        let mut d = draft();
        // 40 emoji is 40 characters but well over 48 bytes. Counting bytes here
        // would reject a legal name.
        d.name = "\u{1f951}".repeat(40);
        assert!(d.validate().is_ok());
        d.name = "\u{1f951}".repeat(49);
        assert_eq!(d.validate(), Err(DraftError::NameTooLong { max: MAX_NAME_LEN }));
    }

    #[test]
    fn a_blank_model_is_allowed_and_means_inherit() {
        // The runtime resolves agent over group over app default, so a blank
        // model here is how an agent says "whatever my group is using".
        let mut d = draft();
        d.model = "  ".into();
        assert_eq!(d.validate().unwrap().model, "");
    }

    #[test]
    fn color_accepts_short_and_long_forms() {
        assert_eq!(normalize_color("#7FB069").as_deref(), Some("#7fb069"));
        assert_eq!(normalize_color("7fb069").as_deref(), Some("#7fb069"));
        assert_eq!(normalize_color("#abc").as_deref(), Some("#aabbcc"));
        assert_eq!(normalize_color("abc").as_deref(), Some("#aabbcc"));
    }

    #[test]
    fn color_rejects_garbage() {
        assert_eq!(normalize_color("#gggggg"), None);
        assert_eq!(normalize_color("#12345"), None);
        assert_eq!(normalize_color(""), None);
        assert_eq!(normalize_color("rgb(1,2,3)"), None);
    }

    #[test]
    fn directory_entry_never_leaks_the_system_prompt() {
        let card = AgentCard {
            id: AgentId::new(),
            group_id: GroupId::new(),
            name: "Manager".into(),
            avatar: "avocado".into(),
            color: "#7fb069".into(),
            model: "m".into(),
            system_prompt: "SECRET INSTRUCTIONS".into(),
            skills: vec!["delegation".into()],
            sandbox_id: None,
            sandbox_envd_token: None,
            sandbox_traffic_token: None,
            lifecycle: Lifecycle::Active,
            version: 1,
            created_at: 0,
            updated_at: 0,
        };
        let json =
            serde_json::to_string(&card.directory_entry(vec!["Gmail as robert@x".into()])).unwrap();
        assert!(!json.contains("SECRET"), "directory entry leaked the prompt: {json}");
        assert!(json.contains("Gmail as robert@x"), "a peer has to be able to see who to ask");
    }

    #[test]
    fn terminated_agents_are_not_discoverable_and_take_no_work() {
        assert!(!Lifecycle::Terminated.is_discoverable());
        assert!(!Lifecycle::Terminated.accepts_work());
        assert!(Lifecycle::Paused.is_discoverable(), "paused agents stay addressable");
        assert!(!Lifecycle::Paused.accepts_work(), "paused agents queue instead of running");
        assert!(Lifecycle::Active.is_discoverable());
        assert!(Lifecycle::Active.accepts_work());
    }

    #[test]
    fn lifecycle_string_form_round_trips() {
        for state in [Lifecycle::Active, Lifecycle::Paused, Lifecycle::Terminated] {
            assert_eq!(Lifecycle::parse(state.as_str()), Some(state));
        }
        assert_eq!(Lifecycle::parse("nonsense"), None);
    }
}
