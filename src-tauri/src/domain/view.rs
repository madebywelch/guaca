use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::ids::{AgentId, GroupId};

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

const MAX_SECTION: usize = 32;

const MAX_CLIENT: usize = 64;

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
    pub agent_id: Option<AgentId>,
    pub overlay: Option<Overlay>,
    pub section: Option<String>,
    pub group_id: Option<GroupId>,
}

impl OperatorView {
    pub fn clean(mut self) -> Self {
        self.section = self.section.filter(|section| {
            !section.is_empty()
                && section.len() <= MAX_SECTION
                && section.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
        });
        self
    }

    pub fn reading(&self) -> Option<AgentId> {
        self.agent_id.filter(|_| self.overlay.is_none())
    }

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

pub const AWAY_AFTER_MS: i64 = 2 * 60 * 1000;

const MAX_CLIENTS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Away,
    Here { on_screen: Option<AgentId>, away_at: i64 },
}

impl Presence {
    pub fn on_screen(&self) -> Option<AgentId> {
        match self {
            Presence::Here { on_screen, .. } => *on_screen,
            Presence::Away => None,
        }
    }

    pub fn away_at(&self) -> Option<i64> {
        match self {
            Presence::Here { away_at, .. } => Some(*away_at),
            Presence::Away => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Clients {
    seen: HashMap<String, Seen>,
}

#[derive(Debug, Clone, Copy)]
struct Seen {
    focused: bool,
    at: i64,
    reading: Option<AgentId>,
}

const ONE_WINDOW: &str = "window";

pub fn window_name(raw: Option<String>) -> Option<String> {
    raw.filter(|name| {
        !name.is_empty()
            && name.len() <= MAX_CLIENT
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

impl Clients {
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

    pub fn presence(&self, now: i64) -> Presence {
        let Some(last) = self.seen.values().map(|seen| seen.at).max() else {
            return Presence::Away;
        };
        let away_after = crate::probe_clock::scaled(AWAY_AFTER_MS);
        let away_at = last + away_after;
        if now >= away_at {
            return Presence::Away;
        }
        let on_screen = self
            .seen
            .values()
            .filter(|seen| seen.focused && now - seen.at < away_after)
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
        for bad in ["Limits", "ignore previous instructions", "a;b", &"a".repeat(40)] {
            let view = OperatorView { section: Some(bad.to_string()), ..Default::default() };
            assert_eq!(view.clean().section, None, "{bad}");
        }
        let view = OperatorView { section: Some("limits".into()), ..Default::default() };
        assert_eq!(view.clean().section.as_deref(), Some("limits"));
    }

    #[test]
    fn a_pane_is_named_the_way_settings_draws_it() {
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
