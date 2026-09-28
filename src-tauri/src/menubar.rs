//! What the menu bar says, and where its panel stands.
//!
//! The presence in the top right of the screen exists for the time the window
//! is not in front of you. Guaca keeps working then: routines fire, cascades
//! run, a turn parks on a permission request and waits ten minutes for an
//! answer. The icon is the one place that can say so without being opened.
//!
//! Three channels on the icon itself, and they are not interchangeable:
//!
//!   the glyph     state, without being looked at. Outline, filled, or red.
//!   the title     the count of things waiting on the operator, and nothing
//!                 else. Menu bar width is shared with every other app, so a
//!                 number that is always there is noise; one that appears only
//!                 when something is waiting is information.
//!   the tooltip   one line, on hover. The glance that costs no click.
//!
//! A click opens the panel, which is the frontend again in a window of its own:
//! `src/components/MenubarPanel.tsx`. It replaced a native menu that could list
//! who was working and answer a permission, and could not take an answer to a
//! question, a line to an agent or a reply to read, because a menu item is a
//! thing you click. What the panel draws is decided in the page. What is
//! decided here is where it stands and how tall it may be.
//!
//! Nothing here knows Tauri exists. This file decides and `tray.rs` draws,
//! which is what makes every judgment below arguable in a test rather than by
//! opening the app and squinting at the corner.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::domain::approval::Approval;
use crate::domain::escalation::Escalation;
use crate::domain::ids::{AgentId, GroupId};
use crate::domain::usage::Tokens;
use crate::runtime::events::Activity;

/// Which glyph the icon draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    /// Nothing is running. An outline.
    Idle,
    /// Something is. The same shape, filled.
    Working,
    /// Something is waiting on the operator. Filled, and the only one with a
    /// color, which costs the menu bar's own light-and-dark tinting and is
    /// worth it exactly once.
    Attention,
}

/// Everything the icon shows without being clicked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    pub glyph: Glyph,
    /// Text beside the glyph, or nothing at all.
    pub title: Option<String>,
    pub tooltip: String,
}

/// One agent, as much of one as the icon needs.
///
/// Its crew is a field rather than a second map keyed by the same id: the two
/// are read from one row of the roster, and two maps that could disagree is an
/// agent counted in another crew.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub name: String,
    pub crew: GroupId,
}

/// A crew, as much of one as the icon needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Crew {
    pub id: GroupId,
    pub name: String,
}

/// Everything the icon is drawn from, as the window reports it.
///
/// The tray process holds no workspace: agents run in a host, and the window
/// is the client of it. So this arrives from `presenceOf` in
/// `src/lib/menubar.ts`, and every field here has to be one that function
/// writes. `ipc.contract.test.ts` holds the two lists equal, because a field
/// this side expects and that side never sends is not a missing number: the
/// whole report is refused at the door, and the icon stays idle whatever the
/// crew is doing.
///
/// Each field is the window's read of something that holds the truth rather
/// than a tally of events, so a missed event is corrected by the next report
/// instead of carried forward.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub roster: HashMap<AgentId, Member>,
    /// Every crew, in the order the crews' column draws them.
    pub crews: Vec<Crew>,
    pub activity: HashMap<AgentId, Activity>,
    /// Pending requests, oldest first.
    pub waiting: Vec<Approval>,
    /// Open escalations, oldest first.
    pub stuck: Vec<Escalation>,
    /// Decisions in For you that need an answer.
    pub decisions: Vec<crate::domain::decision::WorkDecision>,
    /// Spent since the window opened.
    pub session: Tokens,
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
    /// the only name distinguishes nobody. Nothing either for a crew that is not
    /// on the list, which is one that has been disbanded out from under a turn
    /// still finishing.
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

    /// The crew of every agent mid-inference or with work queued, in the
    /// crews' column's order, so each crew's agents are one run.
    ///
    /// A parked turn is not here: it is counted as waiting, and counting it
    /// twice would have the tooltip call a crew busy that is sitting on the
    /// operator.
    fn busy(&self) -> Vec<Option<&Crew>> {
        let mut crews: Vec<Option<&Crew>> = self
            .activity
            .iter()
            .filter(|(_, activity)| {
                matches!(activity, Activity::Thinking | Activity::Queued { .. })
            })
            .map(|(id, _)| self.crew_of(*id))
            .collect();
        crews.sort_by_key(|crew| self.crew_rank(*crew));
        crews
    }

    /// The glyph, the title and the tooltip.
    pub fn look(&self) -> Look {
        // One number, because the operator is answering one question with it:
        // is anything over there mine. A parked turn and an agent that has
        // stopped are different work and the same answer to that question, and
        // two numbers in the menu bar is the state nobody can read at a glance.
        let waiting = self.waiting.len() + self.stuck.len() + self.decisions.len();
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

        let state = if !self.decisions.is_empty() {
            format!("{waiting} items need your attention in For you")
        } else if waiting == 1 {
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
            // to open the panel, and the panel says where.
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
            // because the names would not fit and the panel has them.
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
}

/// The crews the working agents are spread over, in the order they appear.
///
/// Deduplicated by walking rather than by a set, which [`Presence::busy`] has
/// already earned: it comes back sorted by crew, so every crew's agents are one
/// run.
fn crews_working<'a>(busy: &[Option<&'a Crew>]) -> Vec<&'a Crew> {
    let mut crews: Vec<&Crew> = Vec::new();
    for &crew in busy.iter().flatten() {
        if crews.last().map(|last| last.id) != Some(crew.id) {
            crews.push(crew);
        }
    }
    crews
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
/// The same rule as `priced` in `components/Spend.tsx`, and it has to stay that
/// way: the icon, the panel and the group meters are readings of one number,
/// and an operator who saw them disagree would have no way to tell which was
/// lying.
fn priced(cost: Option<f64>) -> Option<f64> {
    cost.filter(|cost| *cost >= MIN_PRICE)
}

/// A price, at the precision the number deserves. The same rule as `money` in
/// `components/Spend.tsx`, so the surfaces cannot disagree about what a run
/// cost.
fn money(dollars: f64) -> String {
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
fn compact(tokens: u64) -> String {
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

// ---- where the panel stands -------------------------------------------------

/// How wide the panel is, in points.
///
/// About a phone's width, because what it holds is one column of the rows the
/// rail draws and the cards the desk draws, and both were written for about
/// that.
pub const PANEL_WIDTH: f64 = 360.0;

/// The tallest the panel grows before its body scrolls, whatever the screen
/// has room for. Past this it has stopped being a glance and become the window
/// drawn smaller.
const PANEL_TALLEST: f64 = 600.0;

/// The shortest it is drawn, so a screen with no room left under the menu bar
/// still gets a panel that can say something.
const PANEL_SHORTEST: f64 = 120.0;

/// How tall it is before the page has said how tall it wants to be.
pub const PANEL_FIRST: f64 = 240.0;

/// The gap between the menu bar and the panel's top edge.
const GAP: f64 = 6.0;

/// The least it keeps from a screen's edges.
const MARGIN: f64 = 8.0;

/// How tall a menu bar item is, in points, which is what tells the displays
/// apart when the icon's pixels could be read as being on two of them.
const MENU_BAR_ITEM: f64 = 24.0;

/// A rectangle on screen, measured from the top left of the main display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Frame {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }

    fn shrunk(self, by: f64) -> Frame {
        Frame { x: self.x / by, y: self.y / by, width: self.width / by, height: self.height / by }
    }
}

/// One display, as the panel's placement reads one. Everything in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    /// The whole display.
    pub bounds: Frame,
    /// What is left of it once the menu bar and the Dock have had theirs.
    pub area: Frame,
    /// Pixels to a point on this display.
    pub scale: f64,
}

/// Where the panel stands under an icon, and how tall it is. `None` when no
/// display claims the icon, and the panel stays where it last stood.
///
/// Under the icon and centered on it, pulled in from a screen's edge rather
/// than hanging off it, and no taller than the room below the menu bar.
///
/// The icon arrives in pixels, scaled by the display whose menu bar it is in,
/// and nothing says which display that is. Every display is asked whether the
/// icon, read at its own scale, is on it, and a laptop beside an older monitor
/// can say yes twice: the icon on the laptop, read at the monitor's scale, can
/// land inside the monitor. Of those, the one that reads the icon at a menu
/// bar's height is the one it is on, because the other reading is off by the
/// ratio of the two scales.
pub fn place(icon: Frame, screens: &[Screen], wanted: f64) -> Option<Frame> {
    let (icon, screen) = screens
        .iter()
        .filter_map(|screen| {
            let at = icon.shrunk(screen.scale);
            screen
                .bounds
                .contains(at.x + at.width / 2.0, at.y + at.height / 2.0)
                .then_some((at, screen))
        })
        .min_by(|(a, _), (b, _)| {
            (a.height - MENU_BAR_ITEM).abs().total_cmp(&(b.height - MENU_BAR_ITEM).abs())
        })?;

    let area = screen.area;
    let x = (icon.x + icon.width / 2.0 - PANEL_WIDTH / 2.0)
        .min(area.x + area.width - MARGIN - PANEL_WIDTH)
        .max(area.x + MARGIN);
    let y = (icon.y + icon.height).max(area.y) + GAP;
    let room = area.y + area.height - MARGIN - y;
    let height = wanted.min(PANEL_TALLEST).min(room).max(PANEL_SHORTEST);
    Some(Frame { x, y, width: PANEL_WIDTH, height })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::approval::{ApprovalState, DetailField, ProtectedAction, Request};
    use crate::domain::ids::{ApprovalId, RunId};

    /// The window hands a presence over in the same shape this file reads. A
    /// field that serializes under one name and deserializes under another is
    /// an icon that draws a box's crew as nobody.
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
            decisions: Vec::new(),
            session: Tokens { prompt: 3, completion: 2, cost: None, calls: 1 },
            running: 1,
        };
        let json = serde_json::to_value(&presence).unwrap();
        assert_eq!(json["activity"][agent.to_string()]["state"], "queued");
        let back: Presence = serde_json::from_value(json).unwrap();
        assert_eq!(back.roster[&agent].name, "Chef");
        assert_eq!(back.crews[0].name, "Kitchen");
        assert_eq!(back.activity[&agent], Activity::Queued { depth: 2 });
        assert_eq!(back.running, 1);
    }

    /// The presence in the shape `presenceOf` in `src/lib/menubar.ts` writes
    /// it, spelled out rather than serialized from this side. A round trip
    /// through this file's own types cannot notice a field the window never
    /// sends, and one such field turned every report away at the door: the
    /// icon sat at "nothing running" whatever the crew was doing.
    #[test]
    fn reads_a_presence_in_the_shape_the_window_sends() {
        let agent = AgentId::new();
        let crew = GroupId::new();
        let json = serde_json::json!({
            "roster": { agent.to_string(): { "name": "Chef", "crew": crew } },
            "crews": [{ "id": crew, "name": "Kitchen" }],
            "activity": { agent.to_string(): { "state": "thinking" } },
            "waiting": [],
            "stuck": [],
            "decisions": [],
            "session": { "prompt": 3, "completion": 2, "cost": null, "calls": 1 },
            "running": 1,
        });
        let presence: Presence = serde_json::from_value(json).expect("the window's presence");
        assert_eq!(presence.look().glyph, Glyph::Working);
    }

    fn approval(agent: AgentId, action: ProtectedAction, summary: &str) -> Approval {
        Approval {
            id: ApprovalId::new(),
            agent_id: agent,
            group_id: GroupId::new(),
            run_id: RunId::new(),
            request: Request::Permission { action },
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
    /// is the case where the icon names no crew anywhere: everything below
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

    /// A session of one model call that spent `tokens`, at `cost`.
    fn spent(tokens: u64, cost: Option<f64>) -> Tokens {
        Tokens { prompt: tokens, completion: 0, cost, calls: 1 }
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
    fn an_idle_workspace_takes_no_width_and_says_so_on_hover() {
        let (presence, _) = quiet();

        let look = presence.look();
        assert_eq!(look.glyph, Glyph::Idle);
        assert_eq!(look.title, None, "an idle icon takes no width in the menu bar");
        assert_eq!(look.tooltip, "Guaca · nothing running");
    }

    #[test]
    fn a_working_crew_fills_the_glyph_and_counts_who() {
        let (mut presence, scout) = quiet();
        let analyst = hire(&mut presence, "Analyst");
        presence.activity.insert(scout, Activity::Thinking);
        presence.activity.insert(analyst, Activity::Queued { depth: 3 });
        presence.running = 1;

        let look = presence.look();
        assert_eq!(look.glyph, Glyph::Working);
        assert_eq!(look.title, None, "working is not something to be pulled out of flow for");
        assert_eq!(look.tooltip, "Guaca · 2 agents working");
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
    }

    #[test]
    fn a_parked_turn_is_not_also_counted_as_working() {
        // It is waiting on the operator, which the count already says. A
        // tooltip that also called its crew busy would be describing a crew
        // that is sitting still.
        let (mut presence, scout) = quiet();
        presence.activity.insert(scout, Activity::AwaitingApproval);
        assert!(presence.busy().is_empty());
    }

    #[test]
    fn a_request_that_needs_answering_outranks_a_crew_that_is_working() {
        let (mut presence, scout) = quiet();
        presence.activity.insert(scout, Activity::Thinking);
        presence.waiting.push(approval(scout, ProtectedAction::CreateAgent, "Scout wants to"));

        assert_eq!(presence.look().glyph, Glyph::Attention);
    }

    #[test]
    fn paused_agents_are_not_running() {
        let (mut presence, scout) = quiet();
        presence.activity.insert(scout, Activity::Paused);

        assert_eq!(presence.look().glyph, Glyph::Idle);
    }

    #[test]
    fn a_deleted_agent_is_named_rather_than_left_blank() {
        let mut presence = Presence::default();
        let gone = AgentId::new();
        presence.waiting.push(approval(gone, ProtectedAction::CreateAgent, "wants to"));

        assert_eq!(presence.look().tooltip, "Guaca · A deleted agent is waiting on you");
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
    fn one_crew_is_named_nowhere_at_all() {
        // A name that is the only name distinguishes nobody. The same rule the
        // window draws the crews' column by.
        let (mut presence, scout) = quiet();
        presence.activity.insert(scout, Activity::Thinking);
        assert_eq!(presence.look().tooltip, "Guaca · 1 agent working");

        presence.stuck.push(escalation(scout, "no key", 1));
        assert_eq!(presence.look().tooltip, "Guaca · Scout is waiting on you");
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
    }

    #[test]
    fn the_tooltip_names_one_working_crew_and_counts_several() {
        // One line, and where is half of what the operator is deciding with it.
        // Named while a name is the answer; counted once the names would not
        // fit, because the panel under it has them.
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

    // ---- what it cost ----------------------------------------------------

    #[test]
    fn spend_is_shown_at_the_precision_the_number_deserves() {
        let (mut presence, _) = quiet();
        presence.session = spent(1_540, Some(0.0042));
        assert_eq!(presence.look().tooltip, "Guaca · nothing running · $0.0042 this session");
    }

    #[test]
    fn an_unpriced_provider_shows_a_count_and_never_a_zero() {
        // A local server prices nothing, which is not the same as charging
        // nothing, and `$0.00` beside a working crew is a lie either way.
        let (mut presence, _) = quiet();
        presence.session = spent(1_000, None);
        assert_eq!(presence.look().tooltip, "Guaca · nothing running · 1.0k tokens this session");
    }

    #[test]
    fn a_free_model_draws_no_price_rather_than_four_zeroes() {
        // A free model prices every call at a real zero, so the cost is
        // `Some(0.0)` and not `None`, and free inference over an afternoon stays
        // there. `$0.0000` in the menu bar is seven characters of a strip shared
        // with every other app, saying nothing.
        let (mut presence, _) = quiet();
        presence.session = spent(1_000, Some(0.0));
        assert_eq!(presence.look().tooltip, "Guaca · nothing running · 1.0k tokens this session");

        // And a paid call too small for `money` to render is the same nothing at
        // more precision.
        assert_eq!(spent_phrase(&spent(11, Some(0.000_02))).as_deref(), Some("11 tokens"));
        // One notch above the floor is a price, and it is drawn.
        assert_eq!(spent_phrase(&spent(11, Some(MIN_PRICE))).as_deref(), Some("$0.0001"));
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

    // ---- where the panel stands ------------------------------------------

    fn frame(x: f64, y: f64, width: f64, height: f64) -> Frame {
        Frame { x, y, width, height }
    }

    /// A 1512 by 982 point laptop at twice the pixels, with its menu bar and a
    /// Dock taking 70 points off the bottom.
    fn laptop() -> Screen {
        Screen {
            bounds: frame(0.0, 0.0, 1512.0, 982.0),
            area: frame(0.0, 37.0, 1512.0, 875.0),
            scale: 2.0,
        }
    }

    /// An icon in the laptop's menu bar at `x` points, as the tray reports it:
    /// in pixels.
    fn icon_at(x: f64) -> Frame {
        frame(x * 2.0, 0.0, 32.0 * 2.0, 37.0 * 2.0)
    }

    #[test]
    fn the_panel_hangs_centered_under_the_icon() {
        let at = place(icon_at(1000.0), &[laptop()], 300.0).expect("a place");
        assert_eq!(at.x, 1016.0 - PANEL_WIDTH / 2.0);
        assert_eq!(at.y, 37.0 + GAP);
        assert_eq!(at.width, PANEL_WIDTH);
        assert_eq!(at.height, 300.0, "a panel that fits is exactly as tall as it asked");
    }

    #[test]
    fn an_icon_near_the_edge_pulls_the_panel_in_rather_than_off_the_screen() {
        let right = place(icon_at(1480.0), &[laptop()], 300.0).expect("a place");
        assert_eq!(right.x + right.width, 1512.0 - MARGIN);

        let left = place(icon_at(4.0), &[laptop()], 300.0).expect("a place");
        assert_eq!(left.x, MARGIN);
    }

    #[test]
    fn the_panel_stops_growing_where_a_glance_would_stop() {
        let at = place(icon_at(1000.0), &[laptop()], 5_000.0).expect("a place");
        assert_eq!(at.height, PANEL_TALLEST);
    }

    #[test]
    fn a_short_screen_gets_the_room_it_has_and_not_more() {
        let mut short = laptop();
        short.area = frame(0.0, 37.0, 1512.0, 400.0);
        let at = place(icon_at(1000.0), &[short], 5_000.0).expect("a place");
        assert_eq!(at.y + at.height, 37.0 + 400.0 - MARGIN);
    }

    #[test]
    fn a_panel_that_has_not_measured_itself_is_still_drawn() {
        let at = place(icon_at(1000.0), &[laptop()], 0.0).expect("a place");
        assert_eq!(at.height, PANEL_SHORTEST);
    }

    #[test]
    fn the_icon_is_found_on_the_display_whose_menu_bar_it_is_in() {
        // A laptop at twice the pixels with a monitor at one to its right. The
        // icon at 1300 points on the laptop is 2600 pixels, which read at the
        // monitor's scale is inside the monitor as well: 1512 to 3432. Read
        // there it is 74 points tall, which is no menu bar.
        let monitor = Screen {
            bounds: frame(1512.0, 0.0, 1920.0, 1080.0),
            area: frame(1512.0, 25.0, 1920.0, 1055.0),
            scale: 1.0,
        };
        for screens in [[laptop(), monitor], [monitor, laptop()]] {
            let at = place(icon_at(1300.0), &screens, 300.0).expect("a place");
            assert_eq!(at.x, 1316.0 - PANEL_WIDTH / 2.0, "the order the displays are listed in");
            assert_eq!(at.y, 37.0 + GAP);
        }

        // And the monitor's own icon, 24 points tall at one pixel a point.
        let there = frame(3000.0, 0.0, 30.0, 24.0);
        let at = place(there, &[laptop(), monitor], 300.0).expect("a place");
        assert_eq!(at.x, 3015.0 - PANEL_WIDTH / 2.0);
        assert_eq!(at.y, 25.0 + GAP);
    }

    #[test]
    fn an_icon_on_no_display_places_nothing() {
        assert_eq!(place(frame(-9_000.0, -9_000.0, 10.0, 10.0), &[laptop()], 300.0), None);
    }
}
