//! Agent identity and capability description.
//!
//! `AgentCard` is lifted from A2A's Agent Card: a self-describing document that
//! says who an agent is and what it can do, so peers can discover it without
//! out-of-band configuration. Guac keeps the useful half (name, skills,
//! version, lifecycle) and drops the parts that only pay off across a trust
//! boundary that a local single-process app does not have: no `.well-known`
//! hosting, no DIDs, no signatures, no registry.

use serde::{Deserialize, Serialize};

use super::ids::{AgentId, GroupId, RepositoryId};

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
    /// The hosted browser this agent uses, once it has been given one. A
    /// different thing from the computer above and on a different provider: the
    /// computer is a machine with a screen, this is a Chrome with a DOM. An
    /// agent may hold both, one, or neither.
    ///
    /// Only the session. The socket that drives it and the URL the operator
    /// watches are read from the provider when needed, because both change when
    /// a browser is replaced and a stale copy of either is a pane pointed at
    /// something that has gone.
    pub browser_id: Option<String>,
    /// Whether the operator has given this agent a computer at all.
    ///
    /// Not the same question as `sandbox_id`, and the difference is the whole
    /// point of the field. A machine is rented and reclaimed on the provider's
    /// clock; being allowed one is a decision, and it has to outlive every
    /// machine made under it. An agent with this false is offered no tool that
    /// reaches a machine and cannot make one, whatever it decides it needs.
    pub has_computer: bool,
    /// The same decision about the browser, and separately, because they are
    /// two places: a crew where one agent reads the web and nobody else leaves
    /// the workspace is the ordinary shape, not a special case.
    pub has_browser: bool,
    /// The repository this agent works in, if the operator gave it one.
    ///
    /// At most one, always. Two agents on one codebase coordinate in the crew
    /// they share; one agent on two codebases is a change whose shape nobody
    /// can see. It is also what makes the rail a tree rather than a
    /// many-to-many drawn twice.
    ///
    /// Always in this agent's own group. A repository belongs to a crew, so one
    /// from another crew is as unreachable as that crew's credentials, and the
    /// store refuses it rather than storing a row every read would filter out.
    pub repository_id: Option<RepositoryId>,
    pub lifecycle: Lifecycle,
    /// Kept at the top of the rail. Where a row is drawn and nothing else: a
    /// pinned agent is addressed, paid for and messaged exactly as before, so
    /// no peer is ever told about this.
    pub pinned: bool,
    /// Where the operator put this row. Lower is higher up its section.
    ///
    /// The arrangement, not the drawn order: a working agent is lifted to the
    /// top of its section by the rail itself and drops back here when it stops.
    /// Where a row is drawn and nothing else, exactly like `pinned`, so it does
    /// not bump the version and no peer is told. Ties are legal and are broken
    /// by `created_at`, which is what an upgrade leaves behind.
    pub rail_order: i32,
    /// Bumped on every update. A2A's Update phase exists so peers can detect
    /// that a card changed under them; the version is what makes that possible.
    pub version: u32,
    pub created_at: i64,
    pub updated_at: i64,
    /// When this agent was thrown out, while it can still be pulled back.
    ///
    /// Set only on a `Terminated` row, and only for as long as the wait lasts:
    /// `None` is both an agent nobody has deleted and one whose thirty days
    /// are up and whose machines, memory and schedule are already gone. The
    /// lifecycle is what tells those two apart, which is why this is not a
    /// state of its own. See [`COMPOST_DAYS`].
    pub discarded_at: Option<i64>,
}

/// How long a deleted agent waits before it is gone for good.
///
/// Long enough that an operator who deleted the wrong row and did not notice
/// until the next time they went looking still has it, and short enough that
/// the compost is not a second roster nobody empties. The number is drawn in
/// the panel, and `Compost.test.tsx` reads it out of this file rather than
/// restating it, because a warning is read as a fact about what will happen.
pub const COMPOST_DAYS: i64 = 30;

/// The same wait, in the milliseconds every timestamp in this app is stamped
/// with.
pub const COMPOST_MS: i64 = COMPOST_DAYS * 24 * 60 * 60 * 1000;

impl AgentCard {
    /// Whether this agent is in the compost: deleted, and still able to come
    /// back. Its machines, memory, schedule and sign-ins are all still there.
    pub fn discarded(&self) -> bool {
        self.discarded_at.is_some()
    }

    /// The directory entry a peer sees. Deliberately excludes `system_prompt`:
    /// one agent should not be able to read another's instructions just by
    /// listing the directory.
    ///
    /// `reaches` is passed in rather than read from the card because accounts
    /// live in their own table, and because deciding what a peer may know about
    /// them is a judgment the caller has to make deliberately.
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
    /// What this peer can reach and the reader cannot: accounts signed in on its
    /// machine, as `Gmail as robert@…`, and any of the crew's plugins the
    /// reader was not chosen for, as `the Stripe plugin`.
    ///
    /// A skill is a claim its agent wrote about itself; this is a fact the
    /// operator established. It is here so an agent asked for something it has
    /// no account for can name the peer that does, instead of reporting that
    /// the crew cannot do it. Only what the reader lacks: listing something it
    /// holds itself reads as a reason to delegate work it can already do.
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

/// The character keys and accents the UI offers when a person picks a look.
///
/// Here so an agent created by another agent gets one too. This is a courtesy
/// and not a contract with the frontend catalog: an unrecognized key still
/// draws, from a hash of the key itself, so the two drifting apart costs a
/// character somebody did not choose rather than a blank avatar.
const CHARACTERS: [&str; 16] = [
    "avocado", "lime", "tomato", "onion", "garlic", "chilli", "cilantro", "salt", "corn", "pepper",
    "radish", "carrot", "mushroom", "chip", "pit", "mill",
];
const ACCENTS: [&str; 12] = [
    "#c7d96b", "#6faa5c", "#b0784a", "#7fd1a3", "#e2674a", "#d9534f", "#e8b84b", "#6aa9d9",
    "#9b8ad4", "#d97ea8", "#c2926b", "#8aa0a6",
];

/// Picks a look for an agent nobody chose one for.
///
/// Deterministic from the name and then nudged past whatever the group is
/// already using, so ten agents made in one turn are ten different faces rather
/// than ten of the same one.
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

/// What to call a copy of an agent, given who is already in the group.
///
/// Names are unique per group and the database enforces it, so a duplicate
/// that guessed wrong would surface to the operator as a constraint violation
/// on a button whose whole job is to succeed. Copying a copy gives
/// `Manager copy 2` rather than `Manager copy copy`: the second is what the
/// rule says and not what anybody means.
pub fn copy_name(original: &str, taken: &[String]) -> String {
    let base = original.trim();
    // Strip a trailing `copy` or `copy N` so the chain stays flat.
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
    // Bounded by the roster: one of the first `taken.len() + 2` has to be free.
    (2..=taken.len() as u32 + 2)
        .map(|n| format!("{root} copy {n}"))
        .find(|candidate| is_free(candidate))
        .map(|name| truncate_name(&name))
        .unwrap_or_else(|| truncate_name(&first))
}

/// Names for a set of agents hired in one go, given who already holds a name in
/// the group they are joining.
///
/// Resolved in sequence rather than independently, and that is the whole point:
/// two agents hired from the same preset, or two presets that happen to share a
/// name, are both free against the roster as it stands and would both be told
/// they can have it. The second write then fails a unique index on a button
/// whose job is to succeed. Each name handed out joins the pool the next one is
/// checked against.
///
/// A name nobody is using is returned untouched, so an ordinary hire produces
/// the name on the card rather than a decorated one. Clashes fall through to
/// [`copy_name`], because one naming rule the operator can predict beats two.
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

/// Keeps a generated name inside the limit `validate` enforces, by characters
/// rather than bytes: a crew of emoji-named agents is legal.
fn truncate_name(name: &str) -> String {
    if name.chars().count() <= MAX_NAME_LEN {
        return name.to_string();
    }
    name.chars().take(MAX_NAME_LEN).collect()
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
            browser_id: None,
            has_computer: false,
            has_browser: false,
            repository_id: None,
            lifecycle: Lifecycle::Active,
            pinned: false,
            rail_order: 0,
            version: 1,
            created_at: 0,
            updated_at: 0,
            discarded_at: None,
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
        // Names are unique case-insensitively, so a clash in another case is
        // still a clash the database would refuse.
        assert_eq!(copy_name("Manager", &taken_names(&["manager copy"])), "Manager copy 2");
    }

    #[test]
    fn copying_a_copy_does_not_stack_the_word() {
        // `Manager copy copy` is what the rule says and not what anyone means.
        assert_eq!(
            copy_name("Manager copy", &taken_names(&["Manager", "Manager copy"])),
            "Manager copy 2"
        );
        assert_eq!(
            copy_name("Manager copy 2", &taken_names(&["Manager copy", "Manager copy 2"])),
            "Manager copy 3"
        );
        // A name that merely ends in a word starting with "copy" is not one.
        assert_eq!(copy_name("Manager copywriter", &[]), "Manager copywriter copy");
    }

    #[test]
    fn a_copy_of_a_maximum_length_name_is_still_a_legal_name() {
        // The suffix is what pushes it over, and a draft the operator cannot
        // fix is worse than a name that lost its last few characters.
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
        // Both are free against the roster as it stands, so resolving them
        // independently hands out "Researcher" twice and the second write dies
        // on the unique index.
        let wanted = taken_names(&["Researcher", "Researcher"]);
        assert_eq!(hire_names(&wanted, &[]), taken_names(&["Researcher", "Researcher copy"]));
    }

    #[test]
    fn a_hire_steps_around_whoever_is_already_in_the_group() {
        assert_eq!(
            hire_names(&taken_names(&["Manager"]), &taken_names(&["Manager"])),
            taken_names(&["Manager copy"])
        );
        // Case is not a difference the database recognizes, so it is not one
        // here either.
        assert_eq!(
            hire_names(&taken_names(&["Manager"]), &taken_names(&["manager"])),
            taken_names(&["Manager copy"])
        );
    }

    #[test]
    fn a_batch_of_hires_is_a_batch_of_legal_drafts() {
        // The names this hands out go straight into `validate`, so anything it
        // can produce has to survive it. A crew hired into a group that already
        // holds every one of them is the case that decorates every name.
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
