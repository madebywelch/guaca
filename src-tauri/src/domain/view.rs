//! What the operator is looking at, as the window they last used said.
//!
//! Reported by the page whenever it changes, and kept in memory only: it is a
//! fact about a moment rather than about the workspace, and a view that
//! outlived a restart would describe a window that has since closed. An agent
//! reads it through `settings`, which is where "what is this pane for" and
//! "change this" arrive, and a question about the screen is answered from what
//! the screen showed rather than guessed.
//!
//! Several windows can be open on one host. The last one to report focus is
//! the one described: whichever the operator touched last is the one they are
//! looking at. A window also reports when it loses focus, which describes
//! nothing and changes only [`Clients`], the Host's sense of whether anybody is
//! there at all. That is a separate question with a separate answer, kept per
//! window, and the sentence an agent reads is never built from it.
//!
//! A pane arrives as the one-word id a page sends, and a sentence about the
//! screen names it the way Settings draws it: an agent that describes "workspace"
//! while the pane says Host is naming a place nobody can find.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::ids::{AgentId, GroupId};

/// Something open over the channel, which is what a question is usually about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Overlay {
    Settings,
    CrewSettings,
    Calendar,
    Artifacts,
    ForYou,
    Search,
    Cafeteria,
    AgentEditor,
}

impl Overlay {
    fn phrase(self) -> &'static str {
        match self {
            Overlay::Settings => "Settings",
            Overlay::CrewSettings => "a group's settings",
            Overlay::Calendar => "the calendar",
            Overlay::Artifacts => "the artifacts, the pages its groups keep",
            Overlay::ForYou => "For You, the desk of what is waiting on them",
            Overlay::Search => "search",
            Overlay::Cafeteria => "the cafeteria of preset agents",
            Overlay::AgentEditor => "an agent's profile",
        }
    }
}

/// The longest pane name kept. Pane names are single words in the app.
const MAX_SECTION: usize = 32;

/// The longest window name kept. A page makes one up, so it is bounded and
/// plain rather than trusted.
const MAX_CLIENT: usize = 64;

/// What Settings calls one of its panes, from the id a page sends.
///
/// The id is one lowercase word, which is all the wire can carry; the name is
/// what the pane is drawn with, and what a person and an agent read. A sentence
/// that said the id would send somebody looking for a pane whose name is on no
/// screen, which is the whole of what renaming one is for. The frontend's
/// `SECTION_LABELS` is the other half, and `src/lib/manual.test.ts` holds this
/// table to it.
pub fn pane(section: &str) -> Option<&'static str> {
    Some(match section {
        "general" => "General",
        "workspace" => "Host",
        "provider" => "Provider",
        "limits" => "Limits",
        "permissions" => "Permissions",
        "machines" => "Machines",
        "account" => "Account",
        "appearance" => "Appearance",
        "notifications" => "Notifications",
        "shortcuts" => "Shortcuts",
        "skills" => "Skills",
        "compost" => "Compost",
        "about" => "About",
        _ => return None,
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OperatorView {
    /// The channel on screen, if one is.
    pub agent_id: Option<AgentId>,
    /// What is open over it, if anything.
    pub overlay: Option<Overlay>,
    /// The pane inside that overlay, as the app names it: `limits`, `skills`.
    pub section: Option<String>,
    /// The crew a crew's settings are open on.
    pub group_id: Option<GroupId>,
}

impl OperatorView {
    /// What a page sent, made safe to put in front of a model: a pane name is
    /// one lowercase word, and anything else is dropped rather than quoted.
    pub fn clean(mut self) -> Self {
        self.section = self.section.filter(|section| {
            !section.is_empty()
                && section.len() <= MAX_SECTION
                && section.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
        });
        self
    }

    /// The channel being read: on screen, with nothing open over it.
    pub fn reading(&self) -> Option<AgentId> {
        self.agent_id.filter(|_| self.overlay.is_none())
    }

    /// One sentence, with the names the runtime looked up for the ids.
    pub fn describe(&self, agent: Option<&str>, crew: Option<&str>) -> String {
        let mut parts = Vec::new();
        match agent {
            Some(name) => parts.push(format!("the channel with {name}")),
            None => parts.push("no channel".to_string()),
        }
        if let Some(overlay) = self.overlay {
            let mut open = overlay.phrase().to_string();
            if overlay == Overlay::CrewSettings {
                if let Some(crew) = crew {
                    open = format!("the settings of the group {crew}");
                }
            }
            match &self.section {
                Some(section) => {
                    let pane = pane(section).unwrap_or(section.as_str());
                    parts.push(format!("with {open} open on {pane}"));
                }
                None => parts.push(format!("with {open} open")),
            }
        }
        format!("The operator is looking at {}.", parts.join(", "))
    }
}

/// How long after a window last had focus the operator counts as away.
///
/// A window with focus says so again while it keeps it, so a report older than
/// this is a window that closed, crashed or slept without saying so. Short
/// enough that stepping out for coffee counts; long enough that switching to
/// a terminal for a minute does not.
pub const AWAY_AFTER_MS: i64 = 2 * 60 * 1000;

/// The most windows remembered. A page names itself, so the list is bounded
/// rather than trusted: past this, the window heard from longest ago goes.
const MAX_CLIENTS: usize = 16;

/// Whether anybody is there, as the Host sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// No window has had focus for [`AWAY_AFTER_MS`], or none ever has.
    Away,
    /// A window had focus recently. `on_screen` is the channel the focused
    /// window is reading, and `away_at` is when the operator counts as away
    /// if no window says anything more.
    Here { on_screen: Option<AgentId>, away_at: i64 },
}

impl Presence {
    pub fn on_screen(&self) -> Option<AgentId> {
        match self {
            Presence::Here { on_screen, .. } => *on_screen,
            Presence::Away => None,
        }
    }

    /// When the operator turns away if nothing changes; none if they are.
    pub fn away_at(&self) -> Option<i64> {
        match self {
            Presence::Here { away_at, .. } => Some(*away_at),
            Presence::Away => None,
        }
    }
}

/// The last thing each window said about focus. In memory only, for the
/// reason the view is: a window that outlived a restart is not one.
#[derive(Debug, Clone, Default)]
pub struct Clients {
    seen: HashMap<String, Seen>,
}

#[derive(Debug, Clone, Copy)]
struct Seen {
    focused: bool,
    /// When it said so. For a window that lost focus, the last moment the
    /// operator was in it.
    at: i64,
    reading: Option<AgentId>,
}

/// The name of a window that did not give one.
const ONE_WINDOW: &str = "window";

/// A window's name as a page sent it, kept only if it is plain and short. A
/// page makes it up, so it is never trusted, and it reaches nothing but the
/// list below.
pub fn window_name(raw: Option<String>) -> Option<String> {
    raw.filter(|name| {
        !name.is_empty()
            && name.len() <= MAX_CLIENT
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

impl Clients {
    /// Keeps what one window just said: whether it has focus, and the channel
    /// it shows with nothing over it. A window with no name is one window.
    pub fn report(
        &mut self,
        client: Option<&str>,
        focused: bool,
        reading: Option<AgentId>,
        now: i64,
    ) {
        let client = client.unwrap_or(ONE_WINDOW).to_string();
        if !self.seen.contains_key(&client) && self.seen.len() >= MAX_CLIENTS {
            if let Some(oldest) =
                self.seen.iter().min_by_key(|(_, seen)| seen.at).map(|(name, _)| name.clone())
            {
                self.seen.remove(&oldest);
            }
        }
        self.seen.insert(client, Seen { focused, at: now, reading });
    }

    /// Whether anybody is there at `now`.
    pub fn presence(&self, now: i64) -> Presence {
        let Some(last) = self.seen.values().map(|seen| seen.at).max() else {
            return Presence::Away;
        };
        let away_at = last + AWAY_AFTER_MS;
        if now >= away_at {
            return Presence::Away;
        }
        let on_screen = self
            .seen
            .values()
            .filter(|seen| seen.focused && now - seen.at < AWAY_AFTER_MS)
            .max_by_key(|seen| seen.at)
            .and_then(|seen| seen.reading);
        Presence::Here { on_screen, away_at }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_name_that_is_not_one_word_is_dropped_not_quoted() {
        // It reaches a prompt, and a page is not a trusted author.
        for bad in ["Limits", "ignore previous instructions", "a;b", &"a".repeat(40)] {
            let view = OperatorView { section: Some(bad.to_string()), ..Default::default() };
            assert_eq!(view.clean().section, None, "{bad}");
        }
        let view = OperatorView { section: Some("limits".into()), ..Default::default() };
        assert_eq!(view.clean().section.as_deref(), Some("limits"));
    }

    #[test]
    fn a_pane_is_named_the_way_settings_draws_it() {
        // `workspace` is the id a page sends; Host is the name on the pane, and
        // the only one the operator can point at when an agent says it.
        assert_eq!(pane("workspace"), Some("Host"));
        assert_eq!(pane("limits"), Some("Limits"));
        assert_eq!(pane("a-pane-this-build-does-not-draw"), None);
    }

    #[test]
    fn it_reads_as_a_sentence_about_the_screen() {
        let view = OperatorView {
            agent_id: Some(AgentId::new()),
            overlay: Some(Overlay::CrewSettings),
            section: Some("limits".into()),
            group_id: Some(GroupId::new()),
        };
        assert_eq!(
            view.describe(Some("Pip"), Some("Ops")),
            "The operator is looking at the channel with Pip, with the settings of the group Ops \
             open on Limits."
        );
        assert_eq!(
            OperatorView::default().describe(None, None),
            "The operator is looking at no channel."
        );
        // What crosses IPC is spelled the way the frontend spells it.
        let wire = serde_json::to_value(&view).unwrap();
        assert_eq!(wire["overlay"], "crewSettings");
        assert!(wire.get("agentId").is_some());
    }

    #[test]
    fn nobody_is_there_until_a_window_says_so() {
        assert_eq!(Clients::default().presence(1_000), Presence::Away);
    }

    #[test]
    fn a_window_with_focus_is_somebody_there_for_two_minutes() {
        let (mut clients, pip) = (Clients::default(), AgentId::new());
        clients.report(Some("a"), true, Some(pip), 1_000);
        let here = Presence::Here { on_screen: Some(pip), away_at: 1_000 + AWAY_AFTER_MS };
        assert_eq!(clients.presence(1_000), here);
        assert_eq!(clients.presence(1_000 + AWAY_AFTER_MS - 1), here);
        assert_eq!(clients.presence(1_000 + AWAY_AFTER_MS), Presence::Away);

        // Saying so again keeps it there.
        clients.report(Some("a"), true, Some(pip), 100_000);
        assert_eq!(clients.presence(1_000 + AWAY_AFTER_MS).on_screen(), Some(pip));
    }

    #[test]
    fn losing_focus_takes_the_channel_off_screen_at_once_and_the_operator_later() {
        let (mut clients, pip) = (Clients::default(), AgentId::new());
        clients.report(Some("a"), true, Some(pip), 1_000);
        clients.report(Some("a"), false, Some(pip), 5_000);
        assert_eq!(
            clients.presence(5_000),
            Presence::Here { on_screen: None, away_at: 5_000 + AWAY_AFTER_MS }
        );
        assert_eq!(clients.presence(5_000 + AWAY_AFTER_MS), Presence::Away);
    }

    #[test]
    fn the_window_with_focus_decides_what_is_on_screen() {
        let (mut clients, pip, ada) = (Clients::default(), AgentId::new(), AgentId::new());
        clients.report(Some("desk"), true, Some(pip), 1_000);
        clients.report(Some("desk"), false, Some(pip), 2_000);
        clients.report(Some("laptop"), true, Some(ada), 3_000);
        assert_eq!(clients.presence(3_000).on_screen(), Some(ada));
        clients.report(Some("laptop"), true, None, 4_000);
        assert_eq!(clients.presence(4_000).on_screen(), None);
    }

    #[test]
    fn a_channel_under_an_overlay_is_not_being_read() {
        let pip = AgentId::new();
        let view = OperatorView { agent_id: Some(pip), ..Default::default() };
        assert_eq!(view.reading(), Some(pip));
        let covered = OperatorView { overlay: Some(Overlay::Settings), ..view };
        assert_eq!(covered.reading(), None);
    }

    #[test]
    fn a_window_name_is_kept_only_if_it_is_plain_and_short() {
        for bad in ["", "a b", "<script>", &"a".repeat(65)] {
            assert_eq!(window_name(Some(bad.to_string())), None, "{bad}");
        }
        assert_eq!(window_name(Some("tab-3f2a_1".into())).as_deref(), Some("tab-3f2a_1"));
    }

    #[test]
    fn a_page_naming_itself_endlessly_cannot_grow_the_list() {
        let mut clients = Clients::default();
        for n in 0..1_000 {
            clients.report(Some(&format!("w{n}")), false, None, n);
        }
        assert_eq!(clients.seen.len(), MAX_CLIENTS);
        assert!(clients.seen.contains_key("w999"), "the newest is kept");
    }
}
