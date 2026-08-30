//! What the menu bar says.
//!
//! The presence in the top right of the screen exists for the time the window
//! is not in front of you. Guaca keeps working then: routines fire, cascades
//! run, a turn parks on a permission request and waits ten minutes for an
//! answer. The strip is the one place that can say so without being opened.
//!
//! Three channels, and they are not interchangeable, which is the whole design:
//!
//!   the glyph     state, without being looked at. Outline, filled, or red.
//!   the title     the count of turns blocked on the operator, and nothing
//!                 else. Menu bar width is shared with every other app, so a
//!                 number that is always there is noise; one that appears only
//!                 when an agent is parked is information.
//!   the tooltip   one line, on hover. The glance that costs no click.
//!   the menu      the whole picture, and the answers.
//!
//! The picture is drawn by crew wherever a crew is worth naming, because two
//! crews can hold two agents with the same name and the same face: a row that
//! says only that Scout is thinking is a row an operator with two Scouts cannot
//! act on. Naming stops the moment there is one crew to name, which is the rule
//! the window's own crews' column is drawn by. `src/lib/presence.ts`.
//!
//! Nothing here knows Tauri exists. This file decides what the strip says and
//! `tray.rs` draws it, which is what makes every judgment below arguable in a
//! test rather than by opening the app and squinting at the corner.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::domain::approval::{Approval, Decision, ProtectedAction};
use crate::domain::escalation::Escalation;
use crate::domain::ids::{AgentId, ApprovalId, GroupId};
use crate::domain::signin::Surface;
use crate::domain::usage::Tokens;
use crate::domain::worknote;
use crate::runtime::events::{Activity, UiEvent};

/// The most requests listed before the rest become a count. Answering the
/// fifth parked turn from a menu is not the workflow this is for.
const MAX_WAITING: usize = 5;

/// The most working agents listed, for the same reason.
const MAX_WORKING: usize = 6;

/// The most escalations listed. Bounded by the crew rather than by the app --
/// an agent holds one -- so this is only ever reached by a workspace where a
/// lot has gone wrong at once, which is the workspace least helped by a menu
/// forty rows long.
const MAX_STUCK: usize = 5;

/// The most fields of a request shown under it, and how much of each.
///
/// A request's detail is what the model asked for, and one of the fields on a
/// `createAgent` request is an entire system prompt. It is shown because a
/// decision made without it is a decision made blind, and it is cut because a
/// menu item is one line.
const MAX_DETAIL: usize = 4;
const DETAIL_CHARS: usize = 90;

/// Which glyph the strip draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    /// Nothing is running. An outline.
    Idle,
    /// Something is. The same shape, filled.
    Working,
    /// A turn is parked on the operator. Filled, and the only one with a
    /// color, which costs the menu bar's own light-and-dark tinting and is
    /// worth it exactly once.
    Attention,
}

/// Everything about the strip itself, as opposed to the menu under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    pub glyph: Glyph,
    /// Text beside the glyph, or nothing at all.
    pub title: Option<String>,
    pub tooltip: String,
}

/// One row of the menu.
///
/// A row rather than a menu item: this describes what is said and what
/// answering it means, and nothing about how a platform draws a menu.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    /// Not clickable. A heading, a total, or a count of what did not fit.
    Note(String),
    Separator,
    /// A turn parked on the operator, and the answers they can give it.
    Waiting {
        id: ApprovalId,
        /// The agent that asked, so the row can open its channel.
        agent: AgentId,
        /// Guaca's own wording. Never the model's: see `domain::approval`.
        label: String,
        /// The request's fields, one line each, already cut to length.
        detail: Vec<String>,
        /// Whether "always allow" is one of the answers. False for anything
        /// done in the operator's name, exactly as in the transcript: a
        /// standing yes to "act outside the workspace" would cover every
        /// future send rather than this one.
        always: bool,
    },
    /// An agent and what it is doing. Opens its channel.
    Agent {
        id: AgentId,
        label: String,
    },
    /// A crew, and how much of it is working. Opens the window inside it.
    ///
    /// A heading that is also a destination, which the two above are not: the
    /// rows under it name agents, and an operator who wanted the crew rather
    /// than one of its agents would otherwise have to pick somebody to get
    /// there and then let go of them.
    Crew {
        id: GroupId,
        label: String,
    },
    /// Bring the window back.
    Open,
    /// End every conversation in flight. Absent when there is nothing to end.
    StopAll(String),
    Quit,
}

impl Row {
    /// The text this row shows, when it has text that can change.
    ///
    /// `Open` and `Quit` are named by constants and a separator says nothing,
    /// so none of the three can ever need editing in place.
    fn label(&self) -> Option<&str> {
        match self {
            Row::Note(text) | Row::StopAll(text) => Some(text),
            Row::Waiting { label, .. } | Row::Agent { label, .. } | Row::Crew { label, .. } => {
                Some(label)
            }
            Row::Separator | Row::Open | Row::Quit => None,
        }
    }

    /// What this row is, with everything that can be edited in place left out.
    ///
    /// Two menus with the same shapes in the same order are the same menu
    /// saying different numbers, which is a text edit. Anything else is a
    /// rebuild. A request's detail is part of its shape because it never
    /// changes for a given request, so including it costs nothing and keeps
    /// the submenu under it from having to be diffed.
    fn shape(&self) -> String {
        match self {
            Row::Note(_) => "note".to_string(),
            Row::Separator => "separator".to_string(),
            Row::Waiting { id, detail, always, .. } => {
                format!("waiting:{id}:{always}:{}", detail.join("\u{1}"))
            }
            Row::Agent { id, .. } => format!("agent:{id}"),
            // The count on it moves as agents start and stop, so the label is
            // out of the shape and the row is edited rather than replaced.
            Row::Crew { id, .. } => format!("crew:{id}"),
            Row::Open => "open".to_string(),
            Row::StopAll(_) => "stop".to_string(),
            Row::Quit => "quit".to_string(),
        }
    }
}

/// What clicking a row means.
///
/// The stored form is a menu item id, which is a string on every platform, so
/// this is a wire format between the two halves of this feature and is tested
/// as one. An id that does not parse is a menu that was built by an older
/// version of this file and is ignored rather than guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Open,
    StopAll,
    /// Show the window with one agent's channel open.
    Reveal(AgentId),
    /// Show the window inside one crew, with no channel chosen.
    Enter(GroupId),
    Decide(ApprovalId, Decision),
}

impl Command {
    pub fn id(self) -> String {
        match self {
            Command::Open => "guac.open".to_string(),
            Command::StopAll => "guac.stop".to_string(),
            Command::Reveal(agent) => format!("guac.reveal.{agent}"),
            Command::Enter(crew) => format!("guac.crew.{crew}"),
            Command::Decide(approval, decision) => {
                format!("guac.decide.{}.{approval}", decision_token(decision))
            }
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "guac.open" => return Some(Command::Open),
            "guac.stop" => return Some(Command::StopAll),
            _ => {}
        }

        if let Some(agent) = id.strip_prefix("guac.reveal.") {
            return agent.parse().ok().map(Command::Reveal);
        }

        if let Some(crew) = id.strip_prefix("guac.crew.") {
            return crew.parse().ok().map(Command::Enter);
        }

        let rest = id.strip_prefix("guac.decide.")?;
        let (token, approval) = rest.split_once('.')?;
        let decision = match token {
            "allow" => Decision::Allow,
            "always" => Decision::AlwaysAllow,
            "deny" => Decision::Deny,
            _ => return None,
        };
        approval.parse().ok().map(|id| Command::Decide(id, decision))
    }
}

/// Deliberately not `Decision`'s serialized spelling. That one crosses IPC and
/// is read by the webview; this one is a menu item id, and a shared token would
/// make a rename in either place a bug in the other.
fn decision_token(decision: Decision) -> &'static str {
    match decision {
        Decision::Allow => "allow",
        Decision::AlwaysAllow => "always",
        Decision::Deny => "deny",
    }
}

/// One agent, as much of one as the strip needs.
///
/// Its crew is a field rather than a second map keyed by the same id: the two
/// are read from one row of the roster, and two maps that could disagree is an
/// agent drawn under another crew's heading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub name: String,
    pub crew: GroupId,
}

/// A crew, as much of one as the strip needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Crew {
    pub id: GroupId,
    pub name: String,
}

/// An agent that is working, and where.
struct Busy {
    id: AgentId,
    /// Its crew, when the strip is naming crews at all.
    crew: Option<Crew>,
    /// The agent and what it is doing, and never its crew: in the menu that is
    /// the heading over it.
    label: String,
}

/// Everything the strip needs to know, read fresh rather than accumulated.
///
/// All of it but `session` is a read of something that already holds the truth:
/// the roster, the activity map, the pending requests, the usage table. Only
/// the session total has nowhere to be read from, because "since this window
/// opened" is not a question SQLite is asked anywhere else.
///
/// Read fresh on purpose. A presence assembled from events drifts the moment
/// one is missed, and the thing that would drift is the number the operator is
/// using to decide whether to go and look.
///
/// Serializable because it has a second source. Read off the local runtime
/// when the window shows this machine's workspace, and handed over by the
/// window when it shows a box's: the strip follows the window, and the window
/// is the one thing that already holds a remote workspace's state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub roster: HashMap<AgentId, Member>,
    /// Every crew, in the order the crews' column draws them.
    ///
    /// That order rather than by how busy each one is, for the same reason the
    /// working list is alphabetical inside a rank: a menu whose sections change
    /// places between two glances has to be read from the top every time, and
    /// this is the order the operator already learned in the window.
    pub crews: Vec<Crew>,
    pub activity: HashMap<AgentId, Activity>,
    /// Which of the working agents are on a machine, and which machine.
    ///
    /// Beside the activity map rather than in it, because it is a different
    /// read with a different life: the activity map says a turn is running,
    /// and this says one call inside it is on a rented desktop or a hosted
    /// browser, which is the moment worth opening the window for.
    pub on_machine: HashMap<AgentId, Surface>,
    /// Pending requests, oldest first.
    pub waiting: Vec<Approval>,
    /// Open escalations, oldest first. Beside the requests rather than in with
    /// them because the two are answered differently and only one of them can
    /// be answered from here at all.
    pub stuck: Vec<Escalation>,
    /// Spent since the window opened.
    pub session: Tokens,
    /// Spent ever, across every crew.
    pub all_time: Tokens,
    /// Conversations in flight.
    pub running: usize,
}

impl Presence {
    fn name_of(&self, id: AgentId) -> &str {
        self.roster.get(&id).map(|member| member.name.as_str()).unwrap_or("A deleted agent")
    }

    /// A crew's name, when naming it says anything.
    ///
    /// Nothing at all while the workspace has one crew. That is the rule the
    /// window's crews' column is drawn by rather than a shortcut: a name that is
    /// the only name distinguishes nobody, and every row carrying it has spent
    /// menu width saying where the only place is. Nothing either for a crew that
    /// is not on the list, which is one that has been disbanded out from under a
    /// turn still finishing.
    fn crew_named(&self, group: GroupId) -> Option<&Crew> {
        if self.crews.len() < 2 {
            return None;
        }
        self.crews.iter().find(|crew| crew.id == group)
    }

    /// The crew an agent is in, by the same rule.
    fn crew_of(&self, agent: AgentId) -> Option<&Crew> {
        self.crew_named(self.roster.get(&agent)?.crew)
    }

    /// Where the crews' column would draw a crew, and past the end for one it
    /// would not draw at all.
    fn crew_rank(&self, crew: Option<&Crew>) -> usize {
        let Some(crew) = crew else { return usize::MAX };
        self.crews.iter().position(|one| one.id == crew.id).unwrap_or(usize::MAX)
    }

    /// Agents mid-inference or with work queued, by crew, the busiest first.
    ///
    /// Crew first so the menu can put a heading over each run of them, and the
    /// crews in the column's own order. Thinking before queued because one is
    /// spending money right now and the other is about to, and alphabetical
    /// within each so the list does not reshuffle itself between two glances.
    ///
    /// A workspace with one crew names none, so every row ranks the same and
    /// the sort collapses to the two it always was.
    fn busy(&self) -> Vec<Busy> {
        let mut rows: Vec<(usize, u8, &str, Busy)> = self
            .activity
            .iter()
            .filter_map(|(id, activity)| {
                let (rank, what) = match activity {
                    // The machine over the model: an agent driving its
                    // computer is thinking too, and "thinking" is the less
                    // useful of the two things to be told about it.
                    Activity::Thinking => (
                        0,
                        match self.on_machine.get(id) {
                            Some(Surface::Computer) => "on its computer".to_string(),
                            Some(Surface::Browser) => "in its browser".to_string(),
                            None => "thinking".to_string(),
                        },
                    ),
                    Activity::Queued { depth } => (
                        1,
                        format!(
                            "{depth} {} waiting",
                            if *depth == 1 { "message" } else { "messages" }
                        ),
                    ),
                    // A parked turn is in the section above, under the request
                    // it is parked on. Listing it here as well would offer the
                    // operator a row that goes somewhere instead of the row
                    // that answers it.
                    Activity::AwaitingApproval | Activity::Idle | Activity::Paused => return None,
                };
                let name = self.name_of(*id);
                let crew = self.crew_of(*id);
                Some((
                    self.crew_rank(crew),
                    rank,
                    name,
                    Busy { id: *id, crew: crew.cloned(), label: format!("{name} · {what}") },
                ))
            })
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(b.2)));
        rows.into_iter().map(|(_, _, _, one)| one).collect()
    }

    fn paused(&self) -> usize {
        self.activity.values().filter(|a| matches!(a, Activity::Paused)).count()
    }

    /// The glyph, the title and the tooltip.
    pub fn look(&self) -> Look {
        // One number, because the operator is answering one question with it:
        // is anything over there mine. A parked turn and an agent that has
        // stopped are different work and the same answer to that question, and
        // two numbers in the menu bar is the state nobody can read at a glance.
        let waiting = self.waiting.len() + self.stuck.len();
        let busy = self.busy();

        let glyph = if waiting > 0 {
            Glyph::Attention
        } else if !busy.is_empty() || self.running > 0 {
            Glyph::Working
        } else {
            Glyph::Idle
        };

        // The count and nothing else. A word beside it would be read once and
        // then be permanent furniture; the number changes, which is what makes
        // it worth the space.
        let title = (waiting > 0).then(|| waiting.to_string());

        let state = if waiting == 1 {
            // Named either way, because one is the case where a name fits and
            // it is the whole difference between "something needs you" and
            // knowing whether to go and look now. Its crew too, for the same
            // reason and only when there is more than one: the operator is
            // deciding whether to go and look, and where is half of that.
            let (who, crew) = match self.waiting.first() {
                Some(approval) => {
                    (self.name_of(approval.agent_id), self.crew_named(approval.group_id))
                }
                None => {
                    let stuck = &self.stuck[0];
                    (self.name_of(stuck.agent_id), self.crew_named(stuck.group_id))
                }
            };
            match crew {
                Some(crew) => format!("{who} in {} is waiting on you", crew.name),
                None => format!("{who} is waiting on you"),
            }
        } else if waiting > 1 {
            // Not where. Several parked turns are several crews as often as
            // not, and a tooltip is one line: the count is what decides whether
            // to open the window, and the menu under it says where.
            format!("{waiting} agents are waiting on you")
        } else if busy.is_empty() {
            "nothing running".to_string()
        } else {
            let count = if busy.len() == 1 {
                "1 agent working".to_string()
            } else {
                format!("{} agents working", busy.len())
            };
            // Where, when where is a thing this workspace has. One crew working
            // is named, because that is the answer; several are counted,
            // because the names would not fit and the menu has them.
            match crews_working(&busy).as_slice() {
                [] => count,
                [only] => format!("{count} in {}", only.name),
                several => format!("{count} in {} crews", several.len()),
            }
        };

        // Named, because a tooltip in the menu bar is one of a dozen and the
        // glyph is the only other thing saying which app this is.
        let mut tooltip = format!("Guaca · {state}");
        if let Some(spent) = spent_phrase(&self.session) {
            tooltip.push_str(" · ");
            tooltip.push_str(&spent);
            tooltip.push_str(" this session");
        }
        Look { glyph, title, tooltip }
    }

    /// The menu, top to bottom.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();

        if !self.waiting.is_empty() {
            rows.push(Row::Note("Waiting on you".to_string()));
            for approval in self.waiting.iter().take(MAX_WAITING) {
                // A question is counted here and cannot be answered here. Its
                // answer is a word the operator picks or writes, and a menu
                // item is a thing you click: the shapes do not meet, and a menu
                // that offered Allow and Deny for "which vendor" would be
                // asking a question it could not take the answer to.
                //
                // So it is a row that opens the channel, which is where it can
                // be answered. Left out of the menu entirely it would still be
                // in the title's count, and the operator would open the window
                // looking for a request the menu had not mentioned.
                // The crew off the request rather than off the agent that
                // asked. They are the same crew, and this one is the crew the
                // run happened in whatever has since been done to the roster.
                let crew = self.crew_named(approval.group_id);

                let Some(action) = approval.request.action() else {
                    rows.push(Row::Agent {
                        id: approval.agent_id,
                        label: format!(
                            "{} · in Guaca",
                            in_crew(one_line(&approval.summary, 110), crew)
                        ),
                    });
                    continue;
                };

                rows.push(Row::Waiting {
                    id: approval.id,
                    agent: approval.agent_id,
                    label: in_crew(one_line(&approval.summary, 120), crew),
                    detail: approval
                        .detail
                        .iter()
                        .take(MAX_DETAIL)
                        .map(|field| {
                            // Labeled, always, and the label is Guaca's word
                            // rather than the model's. A bare value crafted to
                            // read like an answer would sit in a menu of
                            // answers with nothing to distinguish it.
                            format!("{}: {}", field.label, one_line(&field.value, DETAIL_CHARS))
                        })
                        .collect(),
                    // Mirrors the card in the transcript, and for the same
                    // reason: `ActOnBehalf` has no standing yes.
                    always: action == ProtectedAction::CreateAgent,
                });
            }
            // Said rather than dropped. A menu that quietly stops at five
            // reads as five being all there is.
            if let Some(more) = overflow(self.waiting.len(), MAX_WAITING) {
                rows.push(Row::Note(format!("{more} more waiting, in Guaca")));
            }
            rows.push(Row::Separator);
        }

        // Its own section rather than more rows under "Waiting on you", which
        // they would be dishonest as: nothing here is parked, none of it
        // expires, and every one of them has been true for longer than a menu
        // usually reports on. The age is on the row for that reason -- an
        // escalation is a duration rather than a piece of news, and the number
        // in the title says how many and never how long.
        //
        // Each row opens its channel and none of them clears from here. Clearing
        // is one click and would fit a menu item, which is exactly the problem:
        // the click that takes it off the desk is not the click that deals with
        // it, and the two must not be the same size. `docs/ATTENTION.md`.
        if !self.stuck.is_empty() {
            rows.push(Row::Note("Stuck on you".to_string()));
            let now = crate::domain::now_ms();
            for one in self.stuck.iter().take(MAX_STUCK) {
                rows.push(Row::Agent {
                    id: one.agent_id,
                    label: format!(
                        "{} · {} · {}",
                        in_crew(
                            self.name_of(one.agent_id).to_string(),
                            self.crew_named(one.group_id)
                        ),
                        one_line(&one.summary, 90),
                        worknote::how_long_ago(one.raised_at, now)
                    ),
                });
            }
            if let Some(more) = overflow(self.stuck.len(), MAX_STUCK) {
                rows.push(Row::Note(format!("{more} more stuck, in Guaca")));
            }
            rows.push(Row::Separator);
        }

        let busy = self.busy();
        if !busy.is_empty() {
            rows.push(Row::Note("Working".to_string()));
            // A crew's heading goes in with the first of its rows rather than
            // ahead of the run, so a crew whose agents all fell past the cap is
            // never a heading with nothing under it. The count on it is the
            // crew's own, which is what the row is about; what the menu had
            // room for is the note at the end.
            let mut heading: Option<GroupId> = None;
            for one in busy.iter().take(MAX_WORKING) {
                let crew = one.crew.as_ref();
                if crew.map(|crew| crew.id) != heading {
                    heading = crew.map(|crew| crew.id);
                    if let Some(crew) = crew {
                        let working =
                            busy.iter().filter(|other| other.crew.as_ref() == Some(crew)).count();
                        rows.push(Row::Crew {
                            id: crew.id,
                            label: format!("{} · {working} working", crew.name),
                        });
                    }
                }
                rows.push(Row::Agent { id: one.id, label: one.label.clone() });
            }
            if let Some(more) = overflow(busy.len(), MAX_WORKING) {
                rows.push(Row::Note(format!("{more} more working")));
            }
            rows.push(Row::Separator);
        }

        if self.waiting.is_empty() && self.stuck.is_empty() && busy.is_empty() {
            rows.push(Row::Note("Nothing running".to_string()));
            rows.push(Row::Separator);
        }

        // A paused agent is a state the operator chose and then stopped seeing.
        // It accumulates messages while it is paused, so a count is worth a
        // line; which ones is a question for the rail.
        let paused = self.paused();
        if paused > 0 {
            rows.push(Row::Note(format!(
                "{paused} {} paused",
                if paused == 1 { "agent" } else { "agents" }
            )));
            rows.push(Row::Separator);
        }

        rows.push(Row::Note(format!("This session · {}", total_phrase(&self.session))));
        rows.push(Row::Note(format!("All time · {}", total_phrase(&self.all_time))));
        rows.push(Row::Separator);

        rows.push(Row::Open);
        if self.running > 0 {
            rows.push(Row::StopAll(format!(
                "Stop {}",
                if self.running == 1 {
                    "the conversation running".to_string()
                } else {
                    format!("all {} conversations running", self.running)
                }
            )));
        }
        rows.push(Row::Separator);
        rows.push(Row::Quit);

        rows
    }
}

/// A label with the crew it happened in on the end, or the label alone.
///
/// The crew last because the row is read left to right and the first thing the
/// operator is looking for is which of their agents it is. `None` is a
/// workspace with one crew, where the answer is the same for every row.
fn in_crew(label: String, crew: Option<&Crew>) -> String {
    match crew {
        Some(crew) => format!("{label} · {}", crew.name),
        None => label,
    }
}

/// The crews the working agents are spread over, in the order they appear.
///
/// Deduplicated by walking rather than by a set, which [`Presence::busy`] has
/// already earned: it comes back sorted by crew, so every crew's agents are one
/// run.
fn crews_working(busy: &[Busy]) -> Vec<&Crew> {
    let mut crews: Vec<&Crew> = Vec::new();
    for one in busy {
        let Some(crew) = one.crew.as_ref() else { continue };
        if crews.last().map(|last: &&Crew| last.id) != Some(crew.id) {
            crews.push(crew);
        }
    }
    crews
}

/// How many did not fit, or nothing when they all did.
fn overflow(total: usize, shown: usize) -> Option<usize> {
    total.checked_sub(shown).filter(|more| *more > 0)
}

/// What has to change about a drawn menu for it to say this.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    Nothing,
    /// These rows say something new and the menu keeps its shape, so the items
    /// are edited where they are. That is not an optimization: replacing the
    /// menu closes one the operator is reading, and the numbers in here move
    /// every few seconds while a crew works.
    Text(Vec<(usize, String)>),
    /// A row arrived, left, or became a different kind of row.
    Rebuild,
}

pub fn plan(before: &[Row], after: &[Row]) -> Update {
    if before.len() != after.len() {
        return Update::Rebuild;
    }
    let mut edits = Vec::new();
    for (index, (was, now)) in before.iter().zip(after).enumerate() {
        if was.shape() != now.shape() {
            return Update::Rebuild;
        }
        if was.label() != now.label() {
            // A row with a shape has a label or does not; the shapes matched,
            // so a label here means both have one.
            if let Some(label) = now.label() {
                edits.push((index, label.to_string()));
            }
        }
    }
    if edits.is_empty() {
        Update::Nothing
    } else {
        Update::Text(edits)
    }
}

/// True when this event can change anything the strip shows.
///
/// The gate on the whole mechanism. A stream delta arrives once per token, and
/// a menu bar that rebuilt for each would spend the run on the main thread
/// drawing a menu nobody has open.
pub fn touches(event: &UiEvent) -> bool {
    matches!(
        event,
        UiEvent::AgentsChanged
            | UiEvent::ActivityChanged { .. }
            // A machine tool starting or finishing is what moves a working
            // row between "thinking" and "on its computer". Once per call
            // rather than once per token, so this stays far from the delta.
            | UiEvent::ToolStarted { .. }
            | UiEvent::ToolFinished { .. }
            | UiEvent::TokensUsed { .. }
            | UiEvent::RunSettled { .. }
            | UiEvent::ApprovalRequested { .. }
            | UiEvent::ApprovalSettled { .. }
            | UiEvent::EscalationRaised { .. }
            | UiEvent::EscalationCleared { .. }
    )
}

/// Adds one model call to a running total.
pub fn add_call(total: &mut Tokens, prompt: u32, completion: u32, cost: Option<f64>) {
    total.prompt += u64::from(prompt);
    total.completion += u64::from(completion);
    total.calls += 1;
    if let Some(cost) = cost {
        total.cost = Some(total.cost.unwrap_or(0.0) + cost);
    }
}

/// Adds up what several crews have spent.
///
/// `cost` stays `None` until something priced arrives, because a provider that
/// prices nothing is not a provider that charges zero and a workspace with one
/// local crew and one hosted one must report the hosted one's bill rather than
/// the average of a number and a silence.
pub fn sum(totals: impl IntoIterator<Item = Tokens>) -> Tokens {
    let mut out = Tokens::default();
    for one in totals {
        out.prompt += one.prompt;
        out.completion += one.completion;
        out.calls += one.calls;
        if let Some(cost) = one.cost {
            out.cost = Some(out.cost.unwrap_or(0.0) + cost);
        }
    }
    out
}

/// Both numbers, for a menu row that has the width for them.
fn total_phrase(total: &Tokens) -> String {
    if total.calls == 0 {
        return "nothing yet".to_string();
    }
    let count = compact(total.total());
    match priced(total.cost) {
        Some(cost) => format!("{count} tokens · {}", money(cost)),
        None => format!("{count} tokens"),
    }
}

/// The one number, for a tooltip that does not have room for two.
///
/// The count is the fallback rather than the other way around, because it is
/// the figure that always moves: every call adds to it whatever the provider
/// charged.
fn spent_phrase(total: &Tokens) -> Option<String> {
    if total.calls == 0 {
        return None;
    }
    Some(match priced(total.cost) {
        Some(cost) => money(cost),
        None => format!("{} tokens", compact(total.total())),
    })
}

/// The smallest price [`money`] can draw. Below it every digit is a zero.
const MIN_PRICE: f64 = 0.0001;

/// The price, when there is one worth the width it takes.
///
/// Three things report no charge and only one of them is `None`. A local server
/// and a subscription plan price nothing, so their cost is absent. A free model
/// prices every call at a real zero, and free inference over an afternoon stays
/// zero, which would draw `$0.0000` in the menu bar: seven characters of a strip
/// shared with every other app, saying nothing. A paid call small enough to
/// round away says the same nothing at more precision, which is why the floor is
/// what [`money`] can render rather than zero itself.
///
/// The same rule as `priced` in `components/TokenMeter.tsx`, and it has to stay
/// that way: the strip and the group meters are two readings of one number, and
/// an operator who saw them disagree would have no way to tell which was lying.
fn priced(cost: Option<f64>) -> Option<f64> {
    cost.filter(|cost| *cost >= MIN_PRICE)
}

/// A price, at the precision the number deserves. The same rule as `money` in
/// `components/TokenMeter.tsx`, so the two surfaces cannot disagree about what a
/// run cost.
pub fn money(dollars: f64) -> String {
    if dollars >= 100.0 {
        format!("${}", dollars.round() as i64)
    } else if dollars >= 1.0 {
        format!("${dollars:.2}")
    } else if dollars >= 0.01 {
        format!("${dollars:.3}")
    } else {
        format!("${dollars:.4}")
    }
}

/// 1.2k, 3.4M. Exact below a thousand, as in the window.
pub fn compact(tokens: u64) -> String {
    if tokens < 1_000 {
        return tokens.to_string();
    }
    if tokens < 1_000_000 {
        let thousands = tokens as f64 / 1_000.0;
        return if thousands < 10.0 {
            format!("{thousands:.1}k")
        } else {
            format!("{}k", thousands.round() as i64)
        };
    }
    let millions = tokens as f64 / 1_000_000.0;
    if millions < 10.0 {
        format!("{millions:.1}M")
    } else {
        format!("{}M", millions.round() as i64)
    }
}

/// One line, at most `max` characters.
///
/// A menu item is one line whatever it is given, so a value with a newline in
/// it draws as far as the newline and silently loses the rest. Model text
/// reaches this, so the whitespace is collapsed rather than trusted.
fn one_line(text: &str, max: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let kept: String = flat.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

/// Doubles an ampersand on the way into a menu item.
///
/// Every platform's menu treats `&` in an item's text as a mnemonic marker and
/// eats it: an agent called `R&D` draws as `RD`, and a document named `A & B`
/// loses the middle of its name. `&&` is the escape, and it is applied here
/// rather than where the row was composed so that the rows a test reads are the
/// words a person would.
pub fn escape_mnemonic(text: &str) -> String {
    text.replace('&', "&&")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::approval::{ApprovalState, DetailField, Request};

    /// The window hands a presence over in the same shape this file reads
    /// locally. A field that serializes under one name and deserializes under
    /// another is a strip that draws a box's crew as nobody.
    #[test]
    fn a_presence_handed_over_by_the_window_reads_back_whole() {
        let agent = AgentId::new();
        let crew = GroupId::new();
        let mut roster = HashMap::new();
        roster.insert(agent, Member { name: "Chef".into(), crew });
        let presence = Presence {
            roster,
            crews: vec![Crew { id: crew, name: "Kitchen".into() }],
            activity: HashMap::from([(agent, Activity::Queued { depth: 2 })]),
            waiting: Vec::new(),
            stuck: Vec::new(),
            session: Tokens { prompt: 3, completion: 2, cost: None, calls: 1 },
            all_time: Tokens { prompt: 30, completion: 20, cost: Some(0.5), calls: 9 },
            running: 1,
        };
        let json = serde_json::to_value(&presence).unwrap();
        // The frontend spells it the way every other type crossing IPC does.
        assert!(json.get("allTime").is_some(), "{json}");
        assert_eq!(json["activity"][agent.to_string()]["state"], "queued");
        let back: Presence = serde_json::from_value(json).unwrap();
        assert_eq!(back.roster[&agent].name, "Chef");
        assert_eq!(back.crews[0].name, "Kitchen");
        assert_eq!(back.activity[&agent], Activity::Queued { depth: 2 });
        assert_eq!(back.all_time.cost, Some(0.5));
        assert_eq!(back.running, 1);
    }
    use crate::domain::envelope::{Envelope, Intent, Part, Participant, Trust};
    use crate::domain::ids::{MessageId, RunId};

    fn approval(agent: AgentId, action: ProtectedAction, summary: &str) -> Approval {
        request(agent, Request::Permission { action }, summary)
    }

    fn request(agent: AgentId, request: Request, summary: &str) -> Approval {
        Approval {
            id: ApprovalId::new(),
            agent_id: agent,
            group_id: GroupId::new(),
            run_id: RunId::new(),
            request,
            summary: summary.to_string(),
            detail: vec![DetailField::new("Name", "Scribe")],
            state: ApprovalState::Pending,
            answer: None,
            created_at: 0,
            decided_at: None,
        }
    }

    /// A workspace with one crew and one named agent, doing nothing.
    ///
    /// One crew is what an install that has never made another one has, and it
    /// is the case where the strip names no crew anywhere: everything below
    /// that says nothing about crews is asserting that.
    fn quiet() -> (Presence, AgentId) {
        let mut presence = Presence { crews: vec![crew("Everyone")], ..Default::default() };
        let scout = hire(&mut presence, "Scout");
        presence.activity.insert(scout, Activity::Idle);
        (presence, scout)
    }

    fn crew(name: &str) -> Crew {
        Crew { id: GroupId::new(), name: name.to_string() }
    }

    /// Puts a named agent in the first crew, which is where a workspace that
    /// has made only one keeps everybody.
    fn hire(presence: &mut Presence, name: &str) -> AgentId {
        let crew = presence.crews.first().map(|crew| crew.id).unwrap_or_default();
        hire_into(presence, name, crew)
    }

    fn hire_into(presence: &mut Presence, name: &str, crew: GroupId) -> AgentId {
        let id = AgentId::new();
        presence.roster.insert(id, Member { name: name.to_string(), crew });
        id
    }

    /// Everything a section says, in order, up to the separator that ends it.
    fn section(rows: &[Row], heading: &str) -> Vec<String> {
        let Some(start) =
            rows.iter().position(|row| matches!(row, Row::Note(text) if text == heading))
        else {
            panic!("no {heading:?} section: {rows:?}");
        };
        rows[start + 1..]
            .iter()
            .take_while(|row| !matches!(row, Row::Separator))
            .filter_map(|row| row.label().map(str::to_string))
            .collect()
    }

    fn notes(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .filter_map(|row| match row {
                Row::Note(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// One open escalation from `agent`, raised `days` ago.
    fn escalation(agent: AgentId, summary: &str, days: i64) -> Escalation {
        let now = crate::domain::now_ms();
        Escalation {
            id: crate::domain::ids::EscalationId::new(),
            agent_id: agent,
            group_id: GroupId::new(),
            run_id: RunId::new(),
            summary: summary.to_string(),
            raised_at: now - days * 24 * 3_600_000,
            said_at: now,
            times: 1,
            cleared_at: None,
        }
    }

    #[test]
    fn an_agent_that_has_stopped_turns_the_glyph_without_anything_being_parked() {
        // The state this whole mechanism exists for. Nothing is parked, so the
        // activity map says idle and the approvals table is empty: read from
        // either of those alone, a workspace where a crew gave up on Friday
        // draws exactly like one where everything is fine.
        let (mut presence, scout) = quiet();
        presence.stuck.push(escalation(scout, "the deploy needs a key only you have", 2));

        let look = presence.look();
        assert_eq!(look.glyph, Glyph::Attention);
        assert_eq!(look.title.as_deref(), Some("1"));
        assert_eq!(look.tooltip, "Guaca · Scout is waiting on you");
    }

    #[test]
    fn the_title_is_one_number_over_both_kinds() {
        // The operator is answering one question with it: is anything over
        // there mine. Two numbers in the menu bar is a state nobody can read at
        // a glance, and a title that counted only the parked turns would say
        // "1" about a workspace holding three things.
        let (mut presence, scout) = quiet();
        presence.waiting.push(approval(scout, ProtectedAction::CreateAgent, "wants an agent"));
        presence.stuck.push(escalation(scout, "the tooling is down", 2));

        assert_eq!(presence.look().title.as_deref(), Some("2"));
        assert_eq!(presence.look().tooltip, "Guaca · 2 agents are waiting on you");
    }

    #[test]
    fn a_stuck_agent_is_its_own_section_with_the_age_on_the_row() {
        // An escalation is a duration rather than a piece of news, and the
        // title says how many and never how long. This row is the only place
        // the operator can read it without opening the window.
        let (mut presence, scout) = quiet();
        presence.stuck.push(escalation(scout, "the deploy needs a key only you have", 2));

        let rows = presence.rows();
        assert!(notes(&rows).contains(&"Stuck on you".to_string()));
        assert!(
            rows.contains(&Row::Agent {
                id: scout,
                label: "Scout · the deploy needs a key only you have · 2d ago".to_string(),
            }),
            "{rows:?}"
        );
    }

    #[test]
    fn a_stuck_agent_is_not_also_listed_as_nothing_running() {
        // "Nothing running" beside a crew that has stopped dead is the strip
        // saying the one thing that is not true.
        let (mut presence, scout) = quiet();
        presence.stuck.push(escalation(scout, "no key", 1));

        assert!(!notes(&presence.rows()).contains(&"Nothing running".to_string()));
    }

    #[test]
    fn nothing_clears_an_escalation_from_the_menu() {
        // Clearing is one click and would fit a menu item, which is exactly the
        // problem: the click that takes it off the desk is not the click that
        // deals with it, and the two must not be the same size. The row opens
        // the channel instead.
        let (mut presence, scout) = quiet();
        presence.stuck.push(escalation(scout, "no key", 1));

        for row in presence.rows() {
            assert!(
                !matches!(row, Row::Waiting { .. }),
                "an escalation has no verdict to take from a menu"
            );
        }
    }

    #[test]
    fn an_idle_workspace_says_so_and_offers_nothing_to_stop() {
        let (presence, _) = quiet();

        let look = presence.look();
        assert_eq!(look.glyph, Glyph::Idle);
        assert_eq!(look.title, None, "an idle strip takes no width in the menu bar");
        assert_eq!(look.tooltip, "Guaca · nothing running");

        let rows = presence.rows();
        assert!(notes(&rows).contains(&"Nothing running".to_string()));
        assert!(
            !rows.iter().any(|row| matches!(row, Row::StopAll(_))),
            "a stop for nothing is a button that reports nothing happened"
        );
        assert!(rows.contains(&Row::Open));
        assert!(rows.contains(&Row::Quit));
    }

    #[test]
    fn a_working_crew_fills_the_glyph_and_lists_who() {
        let (mut presence, scout) = quiet();
        let analyst = hire(&mut presence, "Analyst");
        presence.activity.insert(scout, Activity::Thinking);
        presence.activity.insert(analyst, Activity::Queued { depth: 3 });
        presence.running = 1;

        let look = presence.look();
        assert_eq!(look.glyph, Glyph::Working);
        assert_eq!(look.title, None, "working is not something to be pulled out of flow for");
        assert_eq!(look.tooltip, "Guaca · 2 agents working");

        let rows = presence.rows();
        let labels: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                Row::Agent { label, .. } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        // Mid-inference first: that one is spending right now.
        assert_eq!(labels, vec!["Scout · thinking", "Analyst · 3 messages waiting"]);
        assert!(rows.iter().any(|row| matches!(row, Row::StopAll(_))));
    }

    #[test]
    fn an_agent_on_a_machine_is_said_to_be_there_rather_than_thinking() {
        // The activity map cannot tell a model thinking from a model driving
        // a rented desktop, and the second is the one worth opening the
        // window for: there is a screen to watch, and a sign-in may be
        // happening in the operator's name.
        let (mut presence, scout) = quiet();
        let analyst = hire(&mut presence, "Analyst");
        let clerk = hire(&mut presence, "Clerk");
        presence.activity.insert(scout, Activity::Thinking);
        presence.activity.insert(analyst, Activity::Thinking);
        presence.activity.insert(clerk, Activity::Thinking);
        presence.on_machine.insert(scout, Surface::Computer);
        presence.on_machine.insert(analyst, Surface::Browser);
        presence.running = 1;

        let labels: Vec<String> = presence
            .rows()
            .iter()
            .filter_map(|row| match row {
                Row::Agent { label, .. } => Some(label.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            labels,
            vec!["Analyst · in its browser", "Clerk · thinking", "Scout · on its computer"]
        );

        // The row moves back the moment the call ends, and that is an edit to
        // the open menu rather than a menu replaced under the operator.
        let before = presence.rows();
        presence.on_machine.clear();
        let Update::Text(edits) = plan(&before, &presence.rows()) else {
            panic!("a label that moved replaced the menu");
        };
        assert!(edits.iter().any(|(_, text)| text == "Scout · thinking"), "{edits:?}");
    }

    #[test]
    fn one_queued_message_is_not_pluralised() {
        let (mut presence, scout) = quiet();
        presence.activity.insert(scout, Activity::Queued { depth: 1 });

        let rows = presence.rows();
        assert!(rows
            .contains(&Row::Agent { id: scout, label: "Scout · 1 message waiting".to_string() }));
    }

    #[test]
    fn a_parked_turn_turns_the_glyph_red_and_puts_a_count_in_the_menu_bar() {
        let (mut presence, scout) = quiet();
        presence.activity.insert(scout, Activity::AwaitingApproval);
        presence.waiting.push(approval(
            scout,
            ProtectedAction::CreateAgent,
            "Scout wants to create an agent called Scribe",
        ));

        let look = presence.look();
        assert_eq!(look.glyph, Glyph::Attention);
        assert_eq!(look.title.as_deref(), Some("1"));
        assert_eq!(look.tooltip, "Guaca · Scout is waiting on you");

        let rows = presence.rows();
        assert!(notes(&rows).contains(&"Waiting on you".to_string()));
        let Some(Row::Waiting { label, detail, always, agent, .. }) =
            rows.iter().find(|row| matches!(row, Row::Waiting { .. }))
        else {
            panic!("the request is not in the menu: {rows:?}");
        };
        assert_eq!(label, "Scout wants to create an agent called Scribe");
        assert_eq!(detail, &vec!["Name: Scribe".to_string()]);
        assert!(*always, "creating an agent is narrow enough to be worth not asking twice");
        assert_eq!(*agent, scout, "the row has to be able to open the channel that asked");
    }

    #[test]
    fn a_parked_turn_is_not_also_listed_as_working() {
        // It is in the section above, under the request that unblocks it.
        // Listing it twice offers a row that goes somewhere instead of the row
        // that answers.
        let (mut presence, scout) = quiet();
        presence.activity.insert(scout, Activity::AwaitingApproval);
        presence.waiting.push(approval(scout, ProtectedAction::ActOnBehalf, "Scout wants to act"));

        let rows = presence.rows();
        assert!(!rows.iter().any(|row| matches!(row, Row::Agent { .. })));
        assert!(!notes(&rows).contains(&"Working".to_string()));
    }

    #[test]
    fn acting_in_the_operators_name_is_never_offered_an_always() {
        // The same refusal as the card in the transcript. "Always" is scoped to
        // an agent and an action, and this action is "act outside the
        // workspace", so a standing yes would cover every future send.
        let (mut presence, scout) = quiet();
        presence.waiting.push(approval(
            scout,
            ProtectedAction::ActOnBehalf,
            "Scout wants to act on GitHub in your name",
        ));

        let Some(Row::Waiting { always, .. }) =
            presence.rows().into_iter().find(|row| matches!(row, Row::Waiting { .. }))
        else {
            panic!("no request in the menu");
        };
        assert!(!always);
    }

    #[test]
    fn a_request_that_needs_answering_outranks_a_crew_that_is_working() {
        let (mut presence, scout) = quiet();
        presence.activity.insert(scout, Activity::Thinking);
        presence.waiting.push(approval(scout, ProtectedAction::CreateAgent, "Scout wants to"));

        assert_eq!(presence.look().glyph, Glyph::Attention);
    }

    #[test]
    fn a_long_list_says_how_many_did_not_fit() {
        let mut presence = Presence::default();
        for index in 0..MAX_WAITING + 3 {
            let agent = hire(&mut presence, &format!("Agent {index}"));
            presence.waiting.push(approval(agent, ProtectedAction::CreateAgent, "wants to"));
        }

        let rows = presence.rows();
        assert_eq!(
            rows.iter().filter(|row| matches!(row, Row::Waiting { .. })).count(),
            MAX_WAITING
        );
        assert!(
            notes(&rows).contains(&"3 more waiting, in Guaca".to_string()),
            "a menu that stops at five without saying so reads as five being all there is"
        );
    }

    #[test]
    fn a_long_working_list_says_the_same() {
        let mut presence = Presence::default();
        for index in 0..MAX_WORKING + 2 {
            let agent = hire(&mut presence, &format!("Agent {index:02}"));
            presence.activity.insert(agent, Activity::Thinking);
        }

        let rows = presence.rows();
        assert_eq!(rows.iter().filter(|row| matches!(row, Row::Agent { .. })).count(), MAX_WORKING);
        assert!(notes(&rows).contains(&"2 more working".to_string()));
    }

    #[test]
    fn paused_agents_are_counted_rather_than_listed() {
        let (mut presence, scout) = quiet();
        let other = hire(&mut presence, "Analyst");
        presence.activity.insert(scout, Activity::Paused);
        presence.activity.insert(other, Activity::Paused);

        assert!(notes(&presence.rows()).contains(&"2 agents paused".to_string()));
        assert_eq!(presence.look().glyph, Glyph::Idle, "paused is not running");
    }

    // ---- which crew ------------------------------------------------------

    /// Two crews, in the order the column would draw them.
    fn two_crews() -> (Presence, Crew, Crew) {
        let research = crew("Research");
        let ops = crew("Ops");
        let presence =
            Presence { crews: vec![research.clone(), ops.clone()], ..Default::default() };
        (presence, research, ops)
    }

    #[test]
    fn the_working_list_is_arranged_by_crew_when_there_is_more_than_one() {
        // The whole point of naming a crew at all. Two crews can hold two
        // agents with the same name and the same face, so "Scout · thinking"
        // on its own is a row the operator cannot act on: it does not say
        // which Scout, and clicking it is the only way to find out.
        let (mut presence, research, ops) = two_crews();
        let scout = hire_into(&mut presence, "Scout", research.id);
        let analyst = hire_into(&mut presence, "Analyst", research.id);
        let deploy = hire_into(&mut presence, "Deploy", ops.id);
        presence.activity.insert(scout, Activity::Thinking);
        presence.activity.insert(analyst, Activity::Queued { depth: 2 });
        presence.activity.insert(deploy, Activity::Thinking);

        let rows = presence.rows();
        assert_eq!(
            section(&rows, "Working"),
            [
                "Research · 2 working",
                "Scout · thinking",
                "Analyst · 2 messages waiting",
                "Ops · 1 working",
                "Deploy · thinking",
            ],
            "{rows:?}"
        );
        // The heading is a destination, not a label: an operator who wanted the
        // crew rather than one of its agents has one click for it.
        assert!(rows.iter().any(|row| matches!(row, Row::Crew { id, .. } if *id == ops.id)));
    }

    #[test]
    fn the_crews_stay_in_the_columns_own_order_as_the_work_moves() {
        // Not busiest first. A menu whose sections change places between two
        // glances has to be read from the top every time, and this is the order
        // the operator already learned in the window.
        let (mut presence, research, ops) = two_crews();
        let scout = hire_into(&mut presence, "Scout", research.id);
        let deploy = hire_into(&mut presence, "Deploy", ops.id);
        presence.activity.insert(scout, Activity::Queued { depth: 1 });
        presence.activity.insert(deploy, Activity::Thinking);

        assert_eq!(
            section(&presence.rows(), "Working"),
            [
                "Research · 1 working",
                "Scout · 1 message waiting",
                "Ops · 1 working",
                "Deploy · thinking",
            ]
        );
    }

    #[test]
    fn one_crew_is_named_nowhere_at_all() {
        // A name that is the only name distinguishes nobody, and every row
        // carrying it has spent menu width saying where the only place is. The
        // same rule the window draws the crews' column by.
        let (mut presence, scout) = quiet();
        presence.activity.insert(scout, Activity::Thinking);
        presence.stuck.push(escalation(scout, "no key", 1));

        let rows = presence.rows();
        assert!(!rows.iter().any(|row| matches!(row, Row::Crew { .. })), "{rows:?}");
        assert_eq!(section(&rows, "Working"), ["Scout · thinking"]);
        assert_eq!(presence.look().tooltip, "Guaca · Scout is waiting on you");
    }

    #[test]
    fn a_crew_whose_agents_all_fell_past_the_cap_draws_no_heading() {
        // A heading with nothing under it is the menu claiming a crew is
        // working and then listing nobody from it.
        let (mut presence, research, ops) = two_crews();
        for index in 0..MAX_WORKING {
            let agent = hire_into(&mut presence, &format!("Agent {index:02}"), research.id);
            presence.activity.insert(agent, Activity::Thinking);
        }
        let deploy = hire_into(&mut presence, "Deploy", ops.id);
        presence.activity.insert(deploy, Activity::Thinking);

        let rows = presence.rows();
        assert!(
            !rows.iter().any(|row| matches!(row, Row::Crew { id, .. } if *id == ops.id)),
            "{rows:?}"
        );
        assert!(notes(&rows).contains(&"1 more working".to_string()), "{rows:?}");
    }

    #[test]
    fn a_crews_count_is_its_own_rather_than_what_the_menu_had_room_for() {
        // The row is a statement about the crew. What did not fit is the note
        // at the end of the section, and the two add up.
        let (mut presence, research, _) = two_crews();
        for index in 0..MAX_WORKING + 2 {
            let agent = hire_into(&mut presence, &format!("Agent {index:02}"), research.id);
            presence.activity.insert(agent, Activity::Thinking);
        }

        let rows = presence.rows();
        assert!(
            rows.contains(&Row::Crew {
                id: research.id,
                label: format!("Research · {} working", MAX_WORKING + 2),
            }),
            "{rows:?}"
        );
        assert!(notes(&rows).contains(&"2 more working".to_string()));
    }

    #[test]
    fn a_parked_turn_says_which_crew_it_parked_in() {
        let (mut presence, _, ops) = two_crews();
        let scout = hire_into(&mut presence, "Scout", ops.id);
        let mut ask = approval(
            scout,
            ProtectedAction::CreateAgent,
            "Scout wants to create an agent called Scribe",
        );
        // Off the request rather than off the roster: this is the crew the run
        // happened in whatever has since been done to the agent.
        ask.group_id = ops.id;
        presence.activity.insert(scout, Activity::AwaitingApproval);
        presence.waiting.push(ask);

        assert_eq!(presence.look().tooltip, "Guaca · Scout in Ops is waiting on you");
        assert_eq!(
            section(&presence.rows(), "Waiting on you"),
            ["Scout wants to create an agent called Scribe · Ops"]
        );
    }

    #[test]
    fn a_stuck_agent_says_which_crew_it_is_stuck_in() {
        let (mut presence, research, _) = two_crews();
        let scout = hire_into(&mut presence, "Scout", research.id);
        let mut raised = escalation(scout, "the deploy needs a key only you have", 2);
        raised.group_id = research.id;
        presence.stuck.push(raised);

        assert_eq!(
            section(&presence.rows(), "Stuck on you"),
            ["Scout · Research · the deploy needs a key only you have · 2d ago"]
        );
    }

    #[test]
    fn the_tooltip_names_one_working_crew_and_counts_several() {
        // One line, and where is half of what the operator is deciding with it.
        // Named while a name is the answer; counted once the names would not
        // fit, because the menu under it has them.
        let (mut presence, research, ops) = two_crews();
        let scout = hire_into(&mut presence, "Scout", research.id);
        let analyst = hire_into(&mut presence, "Analyst", research.id);
        presence.activity.insert(scout, Activity::Thinking);
        presence.activity.insert(analyst, Activity::Thinking);
        assert_eq!(presence.look().tooltip, "Guaca · 2 agents working in Research");

        let deploy = hire_into(&mut presence, "Deploy", ops.id);
        presence.activity.insert(deploy, Activity::Thinking);
        assert_eq!(presence.look().tooltip, "Guaca · 3 agents working in 2 crews");
    }

    #[test]
    fn spend_is_shown_at_the_precision_the_number_deserves() {
        let (mut presence, _) = quiet();
        add_call(&mut presence.session, 1_200, 340, Some(0.0042));
        presence.all_time =
            Tokens { prompt: 8_000_000, completion: 4_400_000, cost: Some(24.1), calls: 900 };

        let notes = notes(&presence.rows());
        assert!(notes.contains(&"This session · 1.5k tokens · $0.0042".to_string()), "{notes:?}");
        assert!(notes.contains(&"All time · 12M tokens · $24.10".to_string()), "{notes:?}");
        assert_eq!(presence.look().tooltip, "Guaca · nothing running · $0.0042 this session");
    }

    #[test]
    fn an_unpriced_provider_shows_a_count_and_never_a_zero() {
        // A local server prices nothing, which is not the same as charging
        // nothing, and `$0.00` beside a working crew is a lie either way.
        let (mut presence, _) = quiet();
        add_call(&mut presence.session, 900, 100, None);

        let notes = notes(&presence.rows());
        assert!(notes.contains(&"This session · 1.0k tokens".to_string()), "{notes:?}");
        assert!(notes.contains(&"All time · nothing yet".to_string()), "{notes:?}");
        assert_eq!(presence.look().tooltip, "Guaca · nothing running · 1.0k tokens this session");
    }

    #[test]
    fn a_free_model_draws_no_price_rather_than_four_zeroes() {
        // A free model prices every call at a real zero, so the cost is
        // `Some(0.0)` and not `None`, and free inference over an afternoon stays
        // there. `$0.0000` in the menu bar is seven characters of a strip shared
        // with every other app, saying nothing. The same floor as the group
        // meters: see `priced` in `components/TokenMeter.tsx`.
        let (mut presence, _) = quiet();
        add_call(&mut presence.session, 900, 100, Some(0.0));

        let notes = notes(&presence.rows());
        assert!(notes.contains(&"This session · 1.0k tokens".to_string()), "{notes:?}");
        assert_eq!(presence.look().tooltip, "Guaca · nothing running · 1.0k tokens this session");

        // And a paid call too small for `money` to render is the same nothing at
        // more precision.
        let mut rounds_away = Tokens::default();
        add_call(&mut rounds_away, 10, 1, Some(0.000_02));
        assert_eq!(spent_phrase(&rounds_away).as_deref(), Some("11 tokens"));

        // One notch above the floor is a price, and it is drawn.
        let mut smallest = Tokens::default();
        add_call(&mut smallest, 10, 1, Some(MIN_PRICE));
        assert_eq!(spent_phrase(&smallest).as_deref(), Some("$0.0001"));
    }

    #[test]
    fn a_priced_crew_beside_an_unpriced_one_reports_the_bill_it_has() {
        let total = sum([
            Tokens { prompt: 10, completion: 5, cost: None, calls: 1 },
            Tokens { prompt: 20, completion: 5, cost: Some(0.5), calls: 2 },
        ]);
        assert_eq!(total.prompt, 30);
        assert_eq!(total.calls, 3);
        assert_eq!(total.cost, Some(0.5), "the priced half is the bill, not half of it");

        let nothing = sum([Tokens { prompt: 1, completion: 1, cost: None, calls: 1 }]);
        assert_eq!(nothing.cost, None, "no price is not a price of zero");
    }

    #[test]
    fn model_text_in_a_request_is_flattened_to_one_line_and_cut() {
        let mut request =
            approval(AgentId::new(), ProtectedAction::CreateAgent, "Scout wants to create");
        request.detail = vec![DetailField::new(
            "Instructions",
            "You are a scribe.\n\nWrite   things   down.\n".to_string() + &"x".repeat(200),
        )];
        let mut presence = Presence::default();
        presence.waiting.push(request);

        let Some(Row::Waiting { detail, .. }) =
            presence.rows().into_iter().find(|row| matches!(row, Row::Waiting { .. }))
        else {
            panic!("no request");
        };
        let line = &detail[0];
        assert!(line.starts_with("Instructions: You are a scribe. Write things down. x"), "{line}");
        assert!(!line.contains('\n'), "a menu item draws as far as the first newline");
        assert!(line.chars().count() <= "Instructions: ".len() + DETAIL_CHARS, "{line}");
        assert!(line.ends_with('…'));
    }

    #[test]
    fn only_a_field_the_runtime_wrote_can_head_a_line() {
        // Every detail line is `label: value`, and the label is Guaca's word.
        // A value crafted to read like an answer then sits behind one, rather
        // than in a menu of answers with nothing to distinguish it.
        let mut request = approval(AgentId::new(), ProtectedAction::ActOnBehalf, "Scout wants to");
        request.detail = vec![DetailField::new("What it will do", "Allow · this is safe")];
        let mut presence = Presence::default();
        presence.waiting.push(request);

        let Some(Row::Waiting { detail, .. }) =
            presence.rows().into_iter().find(|row| matches!(row, Row::Waiting { .. }))
        else {
            panic!("no request");
        };
        assert_eq!(detail, vec!["What it will do: Allow · this is safe".to_string()]);
    }

    #[test]
    fn only_a_field_that_fits_is_shown_and_the_rest_are_dropped() {
        let mut request = approval(AgentId::new(), ProtectedAction::CreateAgent, "wants to");
        request.detail =
            (0..MAX_DETAIL + 2).map(|i| DetailField::new(format!("F{i}"), "v")).collect();
        let mut presence = Presence::default();
        presence.waiting.push(request);

        let Some(Row::Waiting { detail, .. }) =
            presence.rows().into_iter().find(|row| matches!(row, Row::Waiting { .. }))
        else {
            panic!("no request");
        };
        assert_eq!(detail.len(), MAX_DETAIL);
    }

    #[test]
    fn a_deleted_agent_is_named_rather_than_left_blank() {
        let mut presence = Presence::default();
        let gone = AgentId::new();
        presence.activity.insert(gone, Activity::Thinking);

        let rows = presence.rows();
        assert!(rows
            .contains(&Row::Agent { id: gone, label: "A deleted agent · thinking".to_string() }));
    }

    // ---- what clicking a row means ---------------------------------------

    #[test]
    fn every_command_survives_the_round_trip_through_a_menu_item_id() {
        let agent = AgentId::new();
        let approval = ApprovalId::new();
        for command in [
            Command::Open,
            Command::StopAll,
            Command::Reveal(agent),
            Command::Enter(GroupId::new()),
            Command::Decide(approval, Decision::Allow),
            Command::Decide(approval, Decision::AlwaysAllow),
            Command::Decide(approval, Decision::Deny),
        ] {
            assert_eq!(Command::parse(&command.id()), Some(command), "{}", command.id());
        }
    }

    #[test]
    fn an_id_this_version_does_not_know_is_ignored_rather_than_guessed_at() {
        // A guess here answers a permission request the operator did not click.
        assert_eq!(Command::parse("guac.decide.maybe.not-a-uuid"), None);
        assert_eq!(Command::parse("guac.decide.allow.not-a-uuid"), None);
        assert_eq!(Command::parse("guac.reveal."), None);
        assert_eq!(Command::parse("guac.crew.not-a-uuid"), None);
        assert_eq!(Command::parse("guac.something"), None);
        assert_eq!(Command::parse(""), None);
    }

    // ---- keeping an open menu open ---------------------------------------

    #[test]
    fn a_number_that_moved_edits_the_row_it_is_in() {
        let (mut presence, scout) = quiet();
        presence.activity.insert(scout, Activity::Thinking);
        let before = presence.rows();

        add_call(&mut presence.session, 1_000, 200, Some(0.01));
        let after = presence.rows();

        match plan(&before, &after) {
            Update::Text(edits) => {
                assert_eq!(edits.len(), 1, "only the session line moved: {edits:?}");
                assert!(edits[0].1.starts_with("This session · 1.2k tokens"), "{edits:?}");
            }
            other => panic!("a menu the operator may be reading was replaced: {other:?}"),
        }
    }

    #[test]
    fn an_agent_that_started_working_rebuilds_the_menu() {
        let (mut presence, scout) = quiet();
        let before = presence.rows();
        presence.activity.insert(scout, Activity::Thinking);

        assert_eq!(plan(&before, &presence.rows()), Update::Rebuild);
    }

    #[test]
    fn a_machine_tool_call_reaches_the_strip_and_a_token_does_not() {
        // The gate on the whole mechanism. The machine mark changes with a
        // tool call, so those two events have to get through; a delta arrives
        // once per token and must not.
        let id = crate::domain::ids::MessageId::new();
        assert!(touches(&UiEvent::ToolStarted {
            message_id: id,
            call_id: "c1".into(),
            name: "use_screen".into(),
            arguments: serde_json::Value::Null,
        }));
        assert!(touches(&UiEvent::ToolFinished {
            message_id: id,
            call_id: "c1".into(),
            part: crate::domain::envelope::Part::text("done"),
        }));
        assert!(!touches(&UiEvent::StreamDelta {
            message_id: id,
            channel_id: AgentId::new(),
            text: "a".into(),
        }));
    }

    #[test]
    fn a_crews_count_moving_under_the_cap_edits_its_row_rather_than_replacing_the_menu() {
        // The count on a heading moves whenever an agent starts or stops, and
        // past the cap it moves without any row arriving or leaving. Replacing
        // the menu for that would close one the operator is reading.
        let (mut presence, research, _) = two_crews();
        let mut agents = Vec::new();
        for index in 0..MAX_WORKING + 2 {
            let agent = hire_into(&mut presence, &format!("Agent {index:02}"), research.id);
            presence.activity.insert(agent, Activity::Thinking);
            agents.push(agent);
        }
        let before = presence.rows();

        presence.activity.insert(agents[MAX_WORKING + 1], Activity::Idle);

        let Update::Text(edits) = plan(&before, &presence.rows()) else {
            panic!("a count that moved replaced the menu: {:?}", presence.rows());
        };
        let said: Vec<&str> = edits.iter().map(|(_, text)| text.as_str()).collect();
        assert!(said.contains(&"Research · 7 working"), "{said:?}");
        assert!(said.contains(&"1 more working"), "{said:?}");
    }

    #[test]
    fn a_request_arriving_rebuilds_the_menu() {
        let (mut presence, scout) = quiet();
        let before = presence.rows();
        presence.waiting.push(approval(scout, ProtectedAction::CreateAgent, "wants to"));

        assert_eq!(plan(&before, &presence.rows()), Update::Rebuild);
    }

    #[test]
    fn one_agent_swapped_for_another_rebuilds_rather_than_renaming() {
        // Same shape, same count, different agent. A text edit here would leave
        // the row pointing at the channel of an agent that is no longer in it.
        let (mut presence, scout) = quiet();
        presence.activity.insert(scout, Activity::Thinking);
        let before = presence.rows();

        presence.activity.remove(&scout);
        let other = hire(&mut presence, "Scout");
        presence.activity.insert(other, Activity::Thinking);

        assert_eq!(plan(&before, &presence.rows()), Update::Rebuild);
    }

    #[test]
    fn nothing_having_changed_touches_nothing() {
        let (presence, _) = quiet();
        assert_eq!(plan(&presence.rows(), &presence.rows()), Update::Nothing);
    }

    // ---- the gate --------------------------------------------------------

    #[test]
    fn a_stream_delta_does_not_reach_the_menu_bar() {
        // One per token. A strip that rebuilt for each would spend the run on
        // the main thread drawing a menu nobody has open.
        assert!(!touches(&UiEvent::StreamDelta {
            message_id: MessageId::new(),
            channel_id: AgentId::new(),
            text: "hello".to_string(),
        }));
        assert!(!touches(&UiEvent::ReasoningDelta {
            message_id: MessageId::new(),
            text: "weighing it up".to_string(),
        }));
        assert!(!touches(&UiEvent::MessageAppended {
            message: Box::new(Envelope {
                id: MessageId::new(),
                run_id: RunId::new(),
                channel_id: AgentId::new(),
                from: Participant::Human,
                to: Participant::Agent { id: AgentId::new() },
                parts: vec![Part::text("hi")],
                trust: Trust::Operator,
                hop: 0,
                expects_reply: true,
                intent: Intent::Courtesy,
                cause: None,
                created_at: 0,
            })
        }));
    }

    #[test]
    fn everything_the_strip_draws_from_reaches_it() {
        assert!(touches(&UiEvent::AgentsChanged));
        assert!(touches(&UiEvent::ActivityChanged {
            agent_id: AgentId::new(),
            activity: Activity::Thinking,
        }));
        assert!(touches(&UiEvent::TokensUsed {
            agent_id: AgentId::new(),
            group_id: GroupId::new(),
            run_id: RunId::new(),
            prompt: 10,
            completion: 2,
            cost: Some(0.1),
        }));
        assert!(touches(&UiEvent::RunSettled { run_id: RunId::new(), steps_used: 1 }));
        assert!(touches(&UiEvent::ApprovalRequested {
            approval_id: ApprovalId::new(),
            agent_id: AgentId::new(),
        }));
        assert!(touches(&UiEvent::ApprovalSettled {
            approval_id: ApprovalId::new(),
            state: ApprovalState::Allow,
        }));
    }

    // ---- text on its way into a platform menu ----------------------------

    #[test]
    fn an_ampersand_in_a_name_survives_the_menu() {
        // Every platform's menu reads `&` as a mnemonic marker and eats it, so
        // an agent called `R&D` draws as `RD` without this.
        assert_eq!(escape_mnemonic("R&D · thinking"), "R&&D · thinking");
        assert_eq!(escape_mnemonic("nothing to escape"), "nothing to escape");
    }

    #[test]
    fn compact_counts_match_the_meters_in_the_window() {
        assert_eq!(compact(0), "0");
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1_000), "1.0k");
        assert_eq!(compact(9_949), "9.9k");
        assert_eq!(compact(12_400), "12k");
        assert_eq!(compact(1_240_000), "1.2M");
        assert_eq!(compact(12_400_000), "12M");
    }

    #[test]
    fn prices_match_the_meters_in_the_window() {
        assert_eq!(money(0.0042), "$0.0042");
        assert_eq!(money(0.42), "$0.420");
        assert_eq!(money(4.2), "$4.20");
        assert_eq!(money(420.0), "$420");
    }
}
