use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::domain::approval::Approval;
use crate::domain::decision::{questions, results};
use crate::domain::escalation::Escalation;
use crate::domain::ids::{AgentId, GroupId};
use crate::domain::usage::Tokens;
use crate::runtime::events::Activity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Idle,
    Working,
    Attention,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    pub glyph: Glyph,
    pub title: Option<String>,
    pub tooltip: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub name: String,
    pub crew: GroupId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Crew {
    pub id: GroupId,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub roster: HashMap<AgentId, Member>,
    pub crews: Vec<Crew>,
    pub activity: HashMap<AgentId, Activity>,
    pub waiting: Vec<Approval>,
    pub stuck: Vec<Escalation>,
    pub decisions: Vec<crate::domain::decision::WorkDecision>,
    pub session: Tokens,
    pub running: usize,
    pub finished: usize,
}

impl Presence {
    fn name_of(&self, id: AgentId) -> &str {
        self.roster.get(&id).map(|member| member.name.as_str()).unwrap_or(DELETED)
    }

    fn crew_named(&self, group: GroupId) -> Option<&Crew> {
        crew_named(&self.crews, group)
    }

    fn crew_of(&self, agent: AgentId) -> Option<&Crew> {
        self.crew_named(self.roster.get(&agent)?.crew)
    }

    fn crew_rank(&self, crew: Option<&Crew>) -> usize {
        let Some(crew) = crew else { return usize::MAX };
        self.crews.iter().position(|one| one.id == crew.id).unwrap_or(usize::MAX)
    }

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

    pub fn look(&self) -> Look {
        let waiting = self.waiting.len() + self.stuck.len() + self.decisions.len();
        let busy = self.busy();

        let glyph = if waiting > 0 {
            Glyph::Attention
        } else if !busy.is_empty() || self.running > 0 {
            Glyph::Working
        } else {
            Glyph::Idle
        };

        let title = (waiting > 0).then(|| waiting.to_string());

        let mut askers: Vec<(AgentId, GroupId)> = Vec::new();
        for asker in self
            .waiting
            .iter()
            .map(|approval| (approval.agent_id, approval.group_id))
            .chain(self.stuck.iter().map(|stuck| (stuck.agent_id, stuck.group_id)))
            .chain(self.decisions.iter().map(|decision| (decision.agent_id, decision.group_id)))
        {
            if !askers.iter().any(|(agent, _)| *agent == asker.0) {
                askers.push(asker);
            }
        }

        let state = if let [(agent, group)] = askers.as_slice() {
            let who = self.name_of(*agent);
            match self.crew_named(*group) {
                Some(crew) => format!("{who} in {} is waiting on you", crew.name),
                None => format!("{who} is waiting on you"),
            }
        } else if !askers.is_empty() {
            let names: Vec<&str> = askers.iter().map(|(agent, _)| self.name_of(*agent)).collect();
            who_is_waiting(&names)
        } else if busy.is_empty() {
            "nothing running".to_string()
        } else {
            let count = if busy.len() == 1 {
                "1 agent working".to_string()
            } else {
                format!("{} agents working", busy.len())
            };
            match crews_working(&busy).as_slice() {
                [] => count,
                [only] => format!("{count} in {}", only.name),
                several => format!("{count} in {} groups", several.len()),
            }
        };

        let state = match (waiting, self.finished) {
            (_, 0) => state,
            (0, finished) => format!("{state}, {} in For you", results(finished)),
            (waiting, finished) => {
                format!("{} and {} in For you", questions(waiting), results(finished))
            }
        };

        let mut tooltip = format!("Guaca · {state}");
        if let Some(spent) = spent_phrase(&self.session) {
            tooltip.push_str(" · ");
            tooltip.push_str(&spent);
            tooltip.push_str(" this session");
        }
        Look { glyph, title, tooltip }
    }
}

const NAMED: usize = 3;

const DELETED: &str = "A deleted agent";

fn who_is_waiting(asked: &[&str]) -> String {
    let gone = asked.iter().filter(|name| **name == DELETED).count();
    let folded = format!("{gone} deleted agents");
    let names: Vec<&str> = if gone < 2 {
        asked.to_vec()
    } else {
        let first = asked.iter().position(|name| *name == DELETED).unwrap_or(0);
        let mut kept: Vec<&str> = asked.iter().copied().filter(|name| *name != DELETED).collect();
        kept.insert(first, &folded);
        kept
    };
    let names = names.as_slice();
    let others = names.len().saturating_sub(NAMED - 1);
    let named = if names.len() <= NAMED { names } else { &names[..NAMED - 1] };
    let mut shown: Vec<String> = named
        .iter()
        .enumerate()
        .map(|(at, name)| {
            if at > 0 && *name == DELETED {
                "a deleted agent".to_string()
            } else {
                name.to_string()
            }
        })
        .collect();
    if names.len() > NAMED {
        shown.push(if others == 1 { "1 other".to_string() } else { format!("{others} others") });
    }
    match shown.split_last() {
        Some((last, [])) if gone > 1 => format!("{last} are waiting on you"),
        Some((last, [])) => format!("{last} is waiting on you"),
        Some((last, rest)) => format!("{} and {last} are waiting on you", rest.join(", ")),
        None => "nobody is waiting on you".to_string(),
    }
}

pub fn crew_named(crews: &[Crew], group: GroupId) -> Option<&Crew> {
    if crews.len() < 2 {
        return None;
    }
    crews.iter().find(|crew| crew.id == group)
}

fn crews_working<'a>(busy: &[Option<&'a Crew>]) -> Vec<&'a Crew> {
    let mut crews: Vec<&Crew> = Vec::new();
    for &crew in busy.iter().flatten() {
        if crews.last().map(|last| last.id) != Some(crew.id) {
            crews.push(crew);
        }
    }
    crews
}

fn spent_phrase(total: &Tokens) -> Option<String> {
    if total.calls == 0 {
        return None;
    }
    Some(match priced(total.cost) {
        Some(cost) => money(cost),
        None => format!("{} tokens", compact(total.total())),
    })
}

const MIN_PRICE: f64 = 0.0001;

fn priced(cost: Option<f64>) -> Option<f64> {
    cost.filter(|cost| *cost >= MIN_PRICE)
}

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

pub fn compact(tokens: u64) -> String {
    if tokens < 1_000 {
        return tokens.to_string();
    }
    let (unit, suffix) = if tokens < 1_000_000 { (1_000, "k") } else { (1_000_000, "M") };
    let units = tokens as f64 / unit as f64;
    if units >= 10.0 {
        return format!("{}{suffix}", units.round() as i64);
    }
    let exactly_half_a_tenth = tokens.is_multiple_of(unit / 4) && !tokens.is_multiple_of(unit / 2);
    if exactly_half_a_tenth {
        let tenths = (tokens * 10).div_ceil(unit);
        return format!("{}.{}{suffix}", tenths / 10, tenths % 10);
    }
    format!("{units:.1}{suffix}")
}

pub const PANEL_WIDTH: f64 = 360.0;

const PANEL_TALLEST: f64 = 600.0;

const PANEL_SHORTEST: f64 = 120.0;

pub const PANEL_FIRST: f64 = 240.0;

const GAP: f64 = 6.0;

const MARGIN: f64 = 8.0;

const MENU_BAR_ITEM: f64 = 24.0;

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    pub bounds: Frame,
    pub area: Frame,
    pub scale: f64,
}

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
            finished: 4,
        };
        let json = serde_json::to_value(&presence).unwrap();
        assert_eq!(json["activity"][agent.to_string()]["state"], "queued");
        let back: Presence = serde_json::from_value(json).unwrap();
        assert_eq!(back.roster[&agent].name, "Chef");
        assert_eq!(back.crews[0].name, "Kitchen");
        assert_eq!(back.activity[&agent], Activity::Queued { depth: 2 });
        assert_eq!(back.running, 1);
        assert_eq!(back.finished, 4);
    }

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
            "finished": 0,
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
            lapses_at: None,
        }
    }

    fn quiet() -> (Presence, AgentId) {
        let mut presence = Presence { crews: vec![crew("Everyone")], ..Default::default() };
        let scout = hire(&mut presence, "Scout");
        presence.activity.insert(scout, Activity::Idle);
        (presence, scout)
    }

    fn crew(name: &str) -> Crew {
        Crew { id: GroupId::new(), name: name.to_string() }
    }

    fn hire(presence: &mut Presence, name: &str) -> AgentId {
        let crew = presence.crews.first().map(|crew| crew.id).unwrap_or_default();
        hire_into(presence, name, crew)
    }

    fn hire_into(presence: &mut Presence, name: &str, crew: GroupId) -> AgentId {
        let id = AgentId::new();
        presence.roster.insert(id, Member { name: name.to_string(), crew });
        id
    }

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

    fn spent(tokens: u64, cost: Option<f64>) -> Tokens {
        Tokens { prompt: tokens, completion: 0, cost, calls: 1 }
    }

    #[test]
    fn an_agent_that_has_stopped_turns_the_glyph_without_anything_being_parked() {
        let (mut presence, scout) = quiet();
        presence.stuck.push(escalation(scout, "the deploy needs a key only you have", 2));

        let look = presence.look();
        assert_eq!(look.glyph, Glyph::Attention);
        assert_eq!(look.title.as_deref(), Some("1"));
        assert_eq!(look.tooltip, "Guaca · Scout is waiting on you");
    }

    #[test]
    fn the_title_is_one_number_over_both_kinds() {
        let (mut presence, scout) = quiet();
        presence.waiting.push(approval(scout, ProtectedAction::CreateAgent, "wants an agent"));
        presence.stuck.push(escalation(scout, "the tooling is down", 2));

        assert_eq!(presence.look().title.as_deref(), Some("2"));
        assert_eq!(presence.look().tooltip, "Guaca · Scout is waiting on you");
    }

    fn filed(agent: AgentId, group: GroupId) -> crate::domain::decision::WorkDecision {
        serde_json::from_value(serde_json::json!({
            "id": crate::domain::ids::DecisionId::new(),
            "agentId": agent,
            "groupId": group,
            "topic": "renewal",
            "request": { "question": "Renew the lease?" },
            "status": "pending",
            "answer": null,
            "outcome": null,
            "createdAt": 1,
            "updatedAt": 1,
            "dueAt": null,
            "remindAt": 2,
            "snoozedUntil": null,
            "deliveryRun": null,
            "interrupted": false,
        }))
        .unwrap()
    }

    #[test]
    fn the_tooltip_names_who_is_waiting_and_counts_questions_never_items() {
        let (mut presence, scout) = quiet();
        let group = presence.crews[0].id;
        presence.decisions.push(filed(scout, group));
        assert_eq!(presence.look().tooltip, "Guaca · Scout is waiting on you");

        let pip = hire(&mut presence, "Pip");
        presence.waiting.push(approval(pip, ProtectedAction::CreateAgent, "wants an agent"));
        assert_eq!(presence.look().tooltip, "Guaca · Pip and Scout are waiting on you");

        let mut gone = presence.clone();
        gone.waiting.push(approval(AgentId::new(), ProtectedAction::CreateAgent, "wants an agent"));
        assert_eq!(
            gone.look().tooltip,
            "Guaca · Pip, a deleted agent and Scout are waiting on you"
        );
        gone.waiting.push(approval(AgentId::new(), ProtectedAction::CreateAgent, "wants an agent"));
        assert_eq!(
            gone.look().tooltip,
            "Guaca · Pip, 2 deleted agents and Scout are waiting on you"
        );
        let mut only_gone = quiet().0;
        for _ in 0..2 {
            only_gone.waiting.push(approval(AgentId::new(), ProtectedAction::CreateAgent, "asks"));
        }
        assert_eq!(only_gone.look().tooltip, "Guaca · 2 deleted agents are waiting on you");

        let ada = hire(&mut presence, "Ada");
        presence.stuck.push(escalation(ada, "no key", 1));
        assert_eq!(presence.look().tooltip, "Guaca · Pip, Ada and Scout are waiting on you");

        let lee = hire(&mut presence, "Lee");
        presence.waiting.push(approval(lee, ProtectedAction::CreateAgent, "wants an agent"));
        assert_eq!(presence.look().tooltip, "Guaca · Pip, Lee and 2 others are waiting on you");
        assert_eq!(presence.look().title.as_deref(), Some("4"));

        presence.finished = 1;
        assert_eq!(presence.look().tooltip, "Guaca · 4 questions and 1 result in For you");
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
    fn finished_work_is_said_on_hover_and_never_on_the_icon() {
        let (mut presence, _) = quiet();
        presence.finished = 4;

        let look = presence.look();
        assert_eq!(look.glyph, Glyph::Idle);
        assert_eq!(look.title, None, "finished work takes no width in the menu bar");
        assert_eq!(look.tooltip, "Guaca · nothing running, 4 results in For you");
    }

    #[test]
    fn finished_work_follows_what_is_happening_rather_than_replacing_it() {
        let (mut presence, scout) = quiet();
        presence.activity.insert(scout, Activity::Thinking);
        presence.finished = 1;

        let look = presence.look();
        assert_eq!(look.glyph, Glyph::Working);
        assert_eq!(look.tooltip, "Guaca · 1 agent working, 1 result in For you");
    }

    #[test]
    fn what_needs_the_operator_and_what_finished_are_two_counts_in_one_line() {
        let (mut presence, scout) = quiet();
        presence.activity.insert(scout, Activity::AwaitingApproval);
        presence.waiting.push(approval(scout, ProtectedAction::CreateAgent, "Scout wants to"));
        presence.finished = 4;

        let look = presence.look();
        assert_eq!(look.glyph, Glyph::Attention);
        assert_eq!(look.title.as_deref(), Some("1"), "the count is still requests only");
        assert_eq!(look.tooltip, "Guaca · 1 question and 4 results in For you");

        presence.stuck.push(escalation(scout, "no key", 1));
        assert_eq!(presence.look().tooltip, "Guaca · 2 questions and 4 results in For you");
        assert_eq!(presence.look().title.as_deref(), Some("2"));
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

    fn two_crews() -> (Presence, Crew, Crew) {
        let research = crew("Research");
        let ops = crew("Ops");
        let presence =
            Presence { crews: vec![research.clone(), ops.clone()], ..Default::default() };
        (presence, research, ops)
    }

    #[test]
    fn one_crew_is_named_nowhere_at_all() {
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
        ask.group_id = ops.id;
        presence.activity.insert(scout, Activity::AwaitingApproval);
        presence.waiting.push(ask);

        assert_eq!(presence.look().tooltip, "Guaca · Scout in Ops is waiting on you");
    }

    #[test]
    fn the_tooltip_names_one_working_crew_and_counts_several() {
        let (mut presence, research, ops) = two_crews();
        let scout = hire_into(&mut presence, "Scout", research.id);
        let analyst = hire_into(&mut presence, "Analyst", research.id);
        presence.activity.insert(scout, Activity::Thinking);
        presence.activity.insert(analyst, Activity::Thinking);
        assert_eq!(presence.look().tooltip, "Guaca · 2 agents working in Research");

        let deploy = hire_into(&mut presence, "Deploy", ops.id);
        presence.activity.insert(deploy, Activity::Thinking);
        assert_eq!(presence.look().tooltip, "Guaca · 3 agents working in 2 groups");
    }

    #[test]
    fn spend_is_shown_at_the_precision_the_number_deserves() {
        let (mut presence, _) = quiet();
        presence.session = spent(1_540, Some(0.0042));
        assert_eq!(presence.look().tooltip, "Guaca · nothing running · $0.0042 this session");
    }

    #[test]
    fn an_unpriced_provider_shows_a_count_and_never_a_zero() {
        let (mut presence, _) = quiet();
        presence.session = spent(1_000, None);
        assert_eq!(presence.look().tooltip, "Guaca · nothing running · 1.0k tokens this session");
    }

    #[test]
    fn a_free_model_draws_no_price_rather_than_four_zeroes() {
        let (mut presence, _) = quiet();
        presence.session = spent(1_000, Some(0.0));
        assert_eq!(presence.look().tooltip, "Guaca · nothing running · 1.0k tokens this session");

        assert_eq!(spent_phrase(&spent(11, Some(0.000_02))).as_deref(), Some("11 tokens"));
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

    fn frame(x: f64, y: f64, width: f64, height: f64) -> Frame {
        Frame { x, y, width, height }
    }

    fn laptop() -> Screen {
        Screen {
            bounds: frame(0.0, 0.0, 1512.0, 982.0),
            area: frame(0.0, 37.0, 1512.0, 875.0),
            scale: 2.0,
        }
    }

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
pub struct PanelHost<T> {
    desired: Option<T>,
    generation: u64,
    adopted: Option<u64>,
}

impl<T> Default for PanelHost<T> {
    fn default() -> Self {
        Self { desired: None, generation: 0, adopted: None }
    }
}

impl<T: Clone + PartialEq> PanelHost<T> {
    pub fn request(&mut self, host: T) -> bool {
        if self.desired.as_ref() != Some(&host) {
            self.generation += 1;
            self.desired = Some(host);
            self.adopted = None;
        }
        self.adopted != Some(self.generation)
    }

    pub fn desired(&self) -> Option<(T, u64)> {
        self.desired.clone().map(|host| (host, self.generation))
    }

    pub fn acknowledge(&mut self, generation: u64) -> bool {
        if self.desired.is_none() || generation != self.generation {
            return false;
        }
        self.adopted = Some(generation);
        true
    }

    pub fn ready(&self) -> bool {
        self.desired.is_some() && self.adopted == Some(self.generation)
    }
}

pub fn update_icon<T: PartialEq>(
    drawn: &mut Option<T>,
    wanted: T,
    wear: impl FnOnce(&T) -> bool,
) -> bool {
    if drawn.as_ref() == Some(&wanted) {
        return true;
    }
    if wear(&wanted) {
        *drawn = Some(wanted);
        true
    } else {
        *drawn = None;
        false
    }
}

#[cfg(test)]
mod panel_host_tests {
    use super::PanelHost;

    #[test]
    fn failed_build_or_reload_retries_the_same_host() {
        let mut panel = PanelHost::default();
        assert!(panel.request("one"));
        assert!(!panel.ready());
        assert!(panel.request("one"));
        let (_, generation) = panel.desired().unwrap();
        assert!(panel.acknowledge(generation));
        assert!(panel.ready());
        assert!(!panel.request("one"));
    }

    #[test]
    fn stale_adoption_never_enables_the_old_panel() {
        let mut panel = PanelHost::default();
        assert!(panel.request("one"));
        let (_, old) = panel.desired().unwrap();
        assert!(panel.request("two"));
        assert!(!panel.acknowledge(old));
        assert!(!panel.ready());
        let (_, current) = panel.desired().unwrap();
        assert!(panel.acknowledge(current));
        assert!(panel.ready());
    }

    #[test]
    fn partial_icon_failure_is_retried_even_when_presence_reverts() {
        let mut drawn = Some("idle");
        assert!(!super::update_icon(&mut drawn, "busy", |_| false));
        let mut called = false;
        assert!(super::update_icon(&mut drawn, "idle", |_| {
            called = true;
            true
        }));
        assert!(called);
    }
}
