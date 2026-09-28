//! What the operator is looking at, as the window they last used said.
//!
//! Reported by the page whenever it changes, and kept in memory only: it is a
//! fact about a moment rather than about the workspace, and a view that
//! outlived a restart would describe a window that has since closed. An agent
//! reads it through `settings`, which is where "what is this pane for" and
//! "change this" arrive, and a question about the screen is answered from what
//! the screen showed rather than guessed.
//!
//! Several windows can be open on one host. The last one to report is the one
//! described, because the page only reports while it has focus: whichever the
//! operator touched last is the one they are looking at.

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
            Overlay::CrewSettings => "a crew's settings",
            Overlay::Calendar => "the calendar",
            Overlay::Artifacts => "the artifacts, the pages its crews keep",
            Overlay::ForYou => "For You, the desk of what is waiting on them",
            Overlay::Search => "search",
            Overlay::Cafeteria => "the cafeteria of preset agents",
            Overlay::AgentEditor => "an agent's profile",
        }
    }
}

/// The longest pane name kept. Pane names are single words in the app.
const MAX_SECTION: usize = 32;

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
                    open = format!("the settings of the crew {crew}");
                }
            }
            match &self.section {
                Some(section) => parts.push(format!("with {open} open on {section}")),
                None => parts.push(format!("with {open} open")),
            }
        }
        format!("The operator is looking at {}.", parts.join(", "))
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
    fn it_reads_as_a_sentence_about_the_screen() {
        let view = OperatorView {
            agent_id: Some(AgentId::new()),
            overlay: Some(Overlay::CrewSettings),
            section: Some("limits".into()),
            group_id: Some(GroupId::new()),
        };
        assert_eq!(
            view.describe(Some("Pip"), Some("Ops")),
            "The operator is looking at the channel with Pip, with the settings of the crew Ops \
             open on limits."
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
}
