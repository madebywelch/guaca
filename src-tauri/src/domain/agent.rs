use crate::config::Provider;
use crate::domain::effort::ReasoningEffort;
use serde::{Deserialize, Serialize};

use super::ids::{AgentId, GroupId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Active,
    Paused,
    Terminated,
}

impl Lifecycle {
    pub fn is_discoverable(self) -> bool {
        matches!(self, Lifecycle::Active | Lifecycle::Paused)
    }

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Consent {
    #[default]
    Open,
    AskBeforeActing,
}

impl Consent {
    pub fn as_str(self) -> &'static str {
        match self {
            Consent::Open => "open",
            Consent::AskBeforeActing => "askBeforeActing",
        }
    }

    pub fn parse(raw: &str) -> Consent {
        match raw {
            "askBeforeActing" => Consent::AskBeforeActing,
            _ => Consent::Open,
        }
    }

    pub fn asks(self) -> bool {
        self == Consent::AskBeforeActing
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub id: AgentId,
    pub group_id: GroupId,
    pub name: String,
    pub avatar: String,
    pub color: String,
    pub model: String,
    #[serde(default)]
    pub subscription_model: String,
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
    pub system_prompt: String,
    pub skills: Vec<String>,
    pub sandbox_id: Option<String>,
    #[serde(skip_serializing)]
    pub sandbox_envd_token: Option<crate::secrets::Secret>,
    #[serde(skip_serializing)]
    pub sandbox_traffic_token: Option<crate::secrets::Secret>,
    pub browser_id: Option<String>,
    pub has_computer: bool,
    pub has_browser: bool,
    pub browser_consent: Consent,
    pub runs_errands: bool,
    pub lifecycle: Lifecycle,
    pub pinned: bool,
    pub rail_order: i32,
    pub version: u32,
    pub created_at: i64,
    pub updated_at: i64,
    pub discarded_at: Option<i64>,
}

pub const COMPOST_DAYS: i64 = 30;

pub const COMPOST_MS: i64 = COMPOST_DAYS * 24 * 60 * 60 * 1000;

impl std::fmt::Debug for AgentCard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentCard")
            .field("id", &self.id)
            .field("group_id", &self.group_id)
            .field("name", &self.name)
            .field("avatar", &self.avatar)
            .field("color", &self.color)
            .field("model", &self.model)
            .field("subscription_model", &self.subscription_model)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("system_prompt", &self.system_prompt)
            .field("skills", &self.skills)
            .field("sandbox_id", &self.sandbox_id)
            .field("browser_id", &self.browser_id)
            .field("has_computer", &self.has_computer)
            .field("has_browser", &self.has_browser)
            .field("browser_consent", &self.browser_consent)
            .field("runs_errands", &self.runs_errands)
            .field("lifecycle", &self.lifecycle)
            .field("pinned", &self.pinned)
            .field("rail_order", &self.rail_order)
            .field("version", &self.version)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("discarded_at", &self.discarded_at)
            .finish_non_exhaustive()
    }
}

impl AgentCard {
    pub fn discarded(&self) -> bool {
        self.discarded_at.is_some()
    }

    pub fn own_model(&self, provider: Provider) -> Option<&str> {
        let named = match provider {
            Provider::Compatible => &self.model,
            Provider::Chatgpt => &self.subscription_model,
            Provider::Claude => return None,
        };
        Some(named.trim()).filter(|model| !model.is_empty())
    }

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

pub fn split_legacy_model(model: &str, crew: Option<Provider>) -> (String, String) {
    let model = model.trim().to_string();
    if model.is_empty() || model.contains('/') {
        return (model, String::new());
    }
    match crew {
        Some(Provider::Compatible) => (model, String::new()),
        Some(Provider::Chatgpt) => (String::new(), model),
        Some(Provider::Claude) | None => (model.clone(), model),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub id: AgentId,
    pub name: String,
    pub skills: Vec<String>,
    #[serde(default)]
    pub reaches: Vec<String>,
    pub lifecycle: Lifecycle,
    pub version: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDraft {
    #[serde(default)]
    pub group_id: Option<GroupId>,
    pub name: String,
    pub avatar: String,
    pub color: String,
    pub model: String,
    #[serde(default)]
    pub subscription_model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
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

        let model = self.model.trim();

        let color = normalize_color(&self.color)
            .ok_or_else(|| DraftError::BadColor { got: self.color.clone() })?;

        Ok(CleanDraft {
            group_id: self.group_id,
            name: name.to_string(),
            avatar: avatar.to_string(),
            color,
            model: model.to_string(),
            subscription_model: self.subscription_model.as_deref().map(|m| m.trim().to_string()),
            reasoning_effort: self.reasoning_effort,
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
    pub subscription_model: Option<String>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub system_prompt: String,
    pub skills: Vec<String>,
}

const CHARACTERS: [&str; 21] = [
    "orb", "egg", "pebble", "drop", "bean", "lobe", "puck", "cell", "knot", "moon", "wave", "mote",
    "bead", "gourd", "slab", "pip", "husk", "loop", "crumb", "tide", "cinder",
];
const ACCENTS: [&str; 12] = [
    "#c28c31", "#bf5f3c", "#a8453a", "#b3805c", "#8b8f45", "#5e8158", "#4d8b83", "#5a7d99",
    "#4f5f96", "#7c5a8c", "#b26f86", "#6c6f70",
];

pub fn suggest_look(name: &str, taken: &[AgentCard]) -> (String, String) {
    let seed = name
        .trim()
        .to_lowercase()
        .bytes()
        .fold(0u32, |hash, byte| hash.wrapping_mul(31).wrapping_add(u32::from(byte)));

    let unused = |options: &[&str], used: &[String]| -> String {
        let start = (seed as usize) % options.len();
        (0..options.len())
            .map(|offset| options[(start + offset) % options.len()])
            .find(|option| !used.iter().any(|t| t.eq_ignore_ascii_case(option)))
            .unwrap_or(options[start])
            .to_string()
    };

    let avatars: Vec<String> = taken.iter().map(|card| card.avatar.clone()).collect();
    let colors: Vec<String> = taken.iter().map(|card| card.color.clone()).collect();
    (unused(&CHARACTERS, &avatars), unused(&ACCENTS, &colors))
}

pub fn copy_name(original: &str, taken: &[String]) -> String {
    let base = original.trim();
    let root = base
        .rsplit_once(" copy")
        .filter(|(_, tail)| tail.trim().is_empty() || tail.trim().parse::<u32>().is_ok())
        .map(|(head, _)| head)
        .unwrap_or(base);

    let is_free = |candidate: &str| !taken.iter().any(|t| t.trim().eq_ignore_ascii_case(candidate));

    let first = format!("{root} copy");
    if is_free(&first) {
        return truncate_name(&first);
    }
    (2..=taken.len() as u32 + 2)
        .map(|n| format!("{root} copy {n}"))
        .find(|candidate| is_free(candidate))
        .map(|name| truncate_name(&name))
        .unwrap_or_else(|| truncate_name(&first))
}

pub fn hire_names(wanted: &[String], taken: &[String]) -> Vec<String> {
    let mut pool = taken.to_vec();
    let mut out = Vec::with_capacity(wanted.len());
    for want in wanted {
        let clash = pool.iter().any(|held| held.trim().eq_ignore_ascii_case(want.trim()));
        let name = if clash { copy_name(want, &pool) } else { want.trim().to_string() };
        pool.push(name.clone());
        out.push(name);
    }
    out
}

fn truncate_name(name: &str) -> String {
    if name.chars().count() <= MAX_NAME_LEN {
        return name.to_string();
    }
    name.chars().take(MAX_NAME_LEN).collect()
}

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

    #[test]
    fn a_card_holds_its_sandbox_tokens_as_secrets_and_never_serializes_them() {
        let card = AgentCard {
            sandbox_id: Some("sbx".into()),
            sandbox_envd_token: Some("envd-hunter2".into()),
            sandbox_traffic_token: Some("traffic-hunter2".into()),
            ..card()
        };
        let held: [Option<&crate::secrets::Secret>; 2] =
            [card.sandbox_envd_token.as_ref(), card.sandbox_traffic_token.as_ref()];
        assert!(held.iter().all(Option::is_some));
        let json = serde_json::to_string(&card).unwrap();
        assert!(json.contains("sbx") && !json.contains("hunter2"), "{json}");
    }

    fn draft() -> AgentDraft {
        AgentDraft {
            group_id: None,
            name: "  Manager  ".into(),
            avatar: "orb".into(),
            color: "#7FB069".into(),
            model: "anthropic/claude-sonnet-4.5".into(),
            subscription_model: None,
            reasoning_effort: None,
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
        d.name = "\u{1f951}".repeat(40);
        assert!(d.validate().is_ok());
        d.name = "\u{1f951}".repeat(49);
        assert_eq!(d.validate(), Err(DraftError::NameTooLong { max: MAX_NAME_LEN }));
    }

    #[test]
    fn a_blank_model_is_allowed_and_means_inherit() {
        let mut d = draft();
        d.model = "  ".into();
        assert_eq!(d.validate().unwrap().model, "");
    }

    #[test]
    fn a_model_is_sent_only_to_the_provider_it_was_chosen_for() {
        let engineer = AgentCard {
            model: "xiaomi/mimo-v2.6-pro".into(),
            subscription_model: "gpt-6-astra".into(),
            ..card()
        };
        assert_eq!(engineer.own_model(Provider::Chatgpt), Some("gpt-6-astra"));
        assert_eq!(engineer.own_model(Provider::Compatible), Some("xiaomi/mimo-v2.6-pro"));
        assert_eq!(engineer.own_model(Provider::Claude), None, "the program chooses");

        let unset = AgentCard { subscription_model: "  ".into(), ..engineer };
        assert_eq!(unset.own_model(Provider::Chatgpt), None);
    }

    #[test]
    fn an_absent_subscription_model_is_left_alone_and_a_blank_one_clears() {
        let mut d = draft();
        assert_eq!(d.validate().unwrap().subscription_model, None);
        d.subscription_model = Some("  ".into());
        assert_eq!(d.validate().unwrap().subscription_model.as_deref(), Some(""));
        d.subscription_model = Some(" gpt-6-astra ".into());
        assert_eq!(d.validate().unwrap().subscription_model.as_deref(), Some("gpt-6-astra"));
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

    fn card() -> AgentCard {
        AgentCard {
            id: AgentId::new(),
            group_id: GroupId::new(),
            name: "Manager".into(),
            avatar: "orb".into(),
            color: "#7fb069".into(),
            model: "m".into(),
            subscription_model: String::new(),
            reasoning_effort: None,
            system_prompt: "SECRET INSTRUCTIONS".into(),
            skills: vec!["delegation".into()],
            sandbox_id: None,
            sandbox_envd_token: None,
            sandbox_traffic_token: None,
            browser_id: None,
            has_computer: false,
            has_browser: false,
            browser_consent: Consent::default(),
            runs_errands: false,
            lifecycle: Lifecycle::Active,
            pinned: false,
            rail_order: 0,
            version: 1,
            created_at: 0,
            updated_at: 0,
            discarded_at: None,
        }
    }

    #[test]
    fn a_card_debugs_without_its_sandbox_tokens() {
        let card = AgentCard {
            sandbox_id: Some("sbx".into()),
            sandbox_envd_token: Some("envd-hunter2".into()),
            sandbox_traffic_token: Some("traffic-hunter2".into()),
            ..card()
        };
        let printed = format!("{card:?}");
        assert!(printed.contains("sbx"), "{printed}");
        assert!(!printed.contains("hunter2"), "a token reached Debug: {printed}");
    }

    #[test]
    fn directory_entry_never_leaks_the_system_prompt() {
        let json = serde_json::to_string(&card().directory_entry(vec!["Gmail as robert@x".into()]))
            .unwrap();
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
    fn a_copy_takes_a_name_nobody_in_the_group_is_using() {
        assert_eq!(copy_name("Manager", &taken_names(&["Manager"])), "Manager copy");
        assert_eq!(
            copy_name("Manager", &taken_names(&["Manager", "Manager copy"])),
            "Manager copy 2"
        );
        assert_eq!(
            copy_name("Manager", &taken_names(&["Manager", "Manager copy", "Manager copy 2"])),
            "Manager copy 3"
        );
        assert_eq!(copy_name("Manager", &taken_names(&["manager copy"])), "Manager copy 2");
    }

    #[test]
    fn copying_a_copy_does_not_stack_the_word() {
        assert_eq!(
            copy_name("Manager copy", &taken_names(&["Manager", "Manager copy"])),
            "Manager copy 2"
        );
        assert_eq!(
            copy_name("Manager copy 2", &taken_names(&["Manager copy", "Manager copy 2"])),
            "Manager copy 3"
        );
        assert_eq!(copy_name("Manager copywriter", &[]), "Manager copywriter copy");
    }

    #[test]
    fn a_copy_of_a_maximum_length_name_is_still_a_legal_name() {
        let long = "\u{1f951}".repeat(MAX_NAME_LEN);
        let copy = copy_name(&long, &[]);
        assert_eq!(copy.chars().count(), MAX_NAME_LEN);
        let draft = AgentDraft { name: copy, ..draft() };
        assert!(draft.validate().is_ok());
    }

    #[test]
    fn a_hire_keeps_the_name_on_the_card_when_nobody_holds_it() {
        let wanted = taken_names(&["Manager", "Researcher"]);
        assert_eq!(hire_names(&wanted, &[]), wanted);
    }

    #[test]
    fn hiring_the_same_preset_twice_in_one_go_does_not_ask_for_one_name_twice() {
        let wanted = taken_names(&["Researcher", "Researcher"]);
        assert_eq!(hire_names(&wanted, &[]), taken_names(&["Researcher", "Researcher copy"]));
    }

    #[test]
    fn a_hire_steps_around_whoever_is_already_in_the_group() {
        assert_eq!(
            hire_names(&taken_names(&["Manager"]), &taken_names(&["Manager"])),
            taken_names(&["Manager copy"])
        );
        assert_eq!(
            hire_names(&taken_names(&["Manager"]), &taken_names(&["manager"])),
            taken_names(&["Manager copy"])
        );
    }

    #[test]
    fn a_batch_of_hires_is_a_batch_of_legal_drafts() {
        let roster = taken_names(&["Manager", "Researcher", "Critic"]);
        let names = hire_names(&roster, &roster);
        assert_eq!(names.len(), roster.len());
        for name in &names {
            let draft = AgentDraft { name: name.clone(), ..draft() };
            assert!(draft.validate().is_ok(), "{name:?} is not a name an operator could save");
        }
        let unique: std::collections::HashSet<String> =
            names.iter().map(|n| n.to_lowercase()).collect();
        assert_eq!(unique.len(), names.len(), "two hires were given the same name: {names:?}");
    }

    #[test]
    fn hiring_nobody_creates_nobody() {
        assert!(hire_names(&[], &taken_names(&["Manager"])).is_empty());
    }

    fn taken_names(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn lifecycle_string_form_round_trips() {
        for state in [Lifecycle::Active, Lifecycle::Paused, Lifecycle::Terminated] {
            assert_eq!(Lifecycle::parse(state.as_str()), Some(state));
        }
        assert_eq!(Lifecycle::parse("nonsense"), None);
    }
}
