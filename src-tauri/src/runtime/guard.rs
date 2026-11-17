use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::domain::envelope::{content_fingerprint, Participant};
use crate::domain::ids::{AgentId, RunId};
use crate::domain::now_ms;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardLimits {
    pub max_hops: u16,
    pub max_steps_per_run: u32,
    pub max_fanout_per_call: usize,
    pub max_sends_per_pair: u32,
    #[serde(default = "default_tool_rounds")]
    pub max_tool_rounds: u16,
}

pub fn default_tool_rounds() -> u16 {
    24
}

impl Default for GuardLimits {
    fn default() -> Self {
        Self {
            max_hops: 8,
            max_steps_per_run: 60,
            max_fanout_per_call: 8,
            max_sends_per_pair: 6,
            max_tool_rounds: default_tool_rounds(),
        }
    }
}

impl GuardLimits {
    pub fn sanitized(self) -> Self {
        Self {
            max_hops: self.max_hops.clamp(1, 16),
            max_steps_per_run: self.max_steps_per_run.clamp(1, 500),
            max_fanout_per_call: self.max_fanout_per_call.clamp(1, 64),
            max_sends_per_pair: self.max_sends_per_pair.clamp(1, 50),
            max_tool_rounds: self.max_tool_rounds.clamp(1, 100),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Refusal {
    HopLimit { hop: u16, max: u16 },
    RunBudgetExhausted { used: u32, max: u32 },
    PairLimit { recipient: String, sent: u32, max: u32 },
    DuplicateContent { recipient: String },
    FanOutTooWide { requested: usize, max: usize },
    SelfAddressed,
    UnknownRecipient { recipient: String },
    RecipientTerminated { recipient: String },
    ExchangeSettled { recipient: String },
}

impl Refusal {
    pub fn explain(&self) -> String {
        match self {
            Refusal::HopLimit { hop, max } => format!(
                "Refused: this conversation is {hop} hops from the operator and the limit is {max}. \
                 Do not relay further. Summarize what you have and reply to whoever asked you."
            ),
            Refusal::RunBudgetExhausted { used, max } => format!(
                "Refused: this run has used its full budget of {max} model calls ({used} used). \
                 No further messages can be sent. Stop here and make your final message what you \
                 have: what is done and what is left."
            ),
            Refusal::PairLimit { recipient, sent, max } => format!(
                "Refused: you have already sent {recipient} {sent} messages in this run and the \
                 limit is {max}. Stop messaging {recipient} and continue without them."
            ),
            Refusal::DuplicateContent { recipient } => format!(
                "Refused: you already sent {recipient} this exact message in this run. Repeating \
                 it will not produce a different reply. Move on."
            ),
            Refusal::FanOutTooWide { requested, max } => format!(
                "Refused: {requested} recipients in one call exceeds the limit of {max}. \
                 Send to at most {max} at a time."
            ),
            Refusal::SelfAddressed => {
                "Refused: you cannot send a message to yourself. Think it through instead."
                    .to_string()
            }
            Refusal::UnknownRecipient { recipient } => format!(
                "Refused: no agent named {recipient} exists. Call `directory` to see who is \
                 actually available."
            ),
            Refusal::RecipientTerminated { recipient } => {
                format!("Refused: {recipient} has been deleted and cannot receive messages.")
            }
            Refusal::ExchangeSettled { recipient } => format!(
                "Refused: you and {recipient} have both had your say in this run, neither of you \
                 is waiting on the other, and this message was sent as a courtesy. Acknowledging \
                 an acknowledgment is not work. Put what you still need in your final message \
                 instead. If you are giving {recipient} something to do rather than \
                 thanking them, send it again with intent \"work\", saying plainly what you need \
                 done."
            ),
        }
    }

    pub fn headline(&self) -> String {
        match self {
            Refusal::HopLimit { max, .. } => format!("hop limit ({max}) reached"),
            Refusal::RunBudgetExhausted { max, .. } => {
                format!("run budget ({max} model calls) spent")
            }
            Refusal::PairLimit { recipient, max, .. } => {
                format!("already sent {recipient} {max} messages")
            }
            Refusal::DuplicateContent { recipient } => format!("duplicate message to {recipient}"),
            Refusal::FanOutTooWide { max, .. } => format!("fan-out wider than {max}"),
            Refusal::SelfAddressed => "self-addressed message".to_string(),
            Refusal::UnknownRecipient { recipient } => format!("no agent named {recipient}"),
            Refusal::RecipientTerminated { recipient } => format!("{recipient} was deleted"),
            Refusal::ExchangeSettled { recipient } => {
                format!("nothing outstanding with {recipient}")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct SendRequest {
    pub from: AgentId,
    pub to: AgentId,
    pub to_name: String,
    pub text: String,
    pub inbound_hop: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Allow { hop: u16 },
    Refuse(Refusal),
}

#[derive(Debug)]
pub struct RunState {
    limits: GuardLimits,
    steps_used: u32,
    fingerprints: HashSet<String>,
    pair_counts: HashMap<(AgentId, AgentId), u32>,
    expecting: HashMap<(AgentId, AgentId), u32>,
    last_touched: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardSnapshot {
    version: u32,
    limits: GuardLimits,
    steps_used: u32,
    fingerprints: HashSet<String>,
    pair_counts: Vec<(AgentId, AgentId, u32)>,
    expecting: Vec<(AgentId, AgentId, u32)>,
}

impl RunState {
    pub fn snapshot(&self) -> GuardSnapshot {
        GuardSnapshot {
            version: 1,
            limits: self.limits,
            steps_used: self.steps_used,
            fingerprints: self.fingerprints.clone(),
            pair_counts: self.pair_counts.iter().map(|(&(a, b), &n)| (a, b, n)).collect(),
            expecting: self.expecting.iter().map(|(&(a, b), &n)| (a, b, n)).collect(),
        }
    }

    pub fn from_snapshot(snapshot: GuardSnapshot) -> Result<Self, &'static str> {
        if snapshot.version != 1 {
            return Err("unsupported guard snapshot version");
        }
        Ok(Self {
            limits: snapshot.limits,
            steps_used: snapshot.steps_used,
            fingerprints: snapshot.fingerprints,
            pair_counts: snapshot.pair_counts.into_iter().map(|(a, b, n)| ((a, b), n)).collect(),
            expecting: snapshot.expecting.into_iter().map(|(a, b, n)| ((a, b), n)).collect(),
            last_touched: now_ms(),
        })
    }
    pub fn new(limits: GuardLimits) -> Self {
        Self {
            limits: limits.sanitized(),
            steps_used: 0,
            fingerprints: HashSet::new(),
            pair_counts: HashMap::new(),
            expecting: HashMap::new(),
            last_touched: now_ms(),
        }
    }

    pub fn steps_used(&self) -> u32 {
        self.steps_used
    }

    pub fn steps_remaining(&self) -> u32 {
        self.limits.max_steps_per_run.saturating_sub(self.steps_used)
    }

    pub fn has_budget(&self) -> bool {
        self.steps_remaining() > 0
    }

    pub fn limits(&self) -> GuardLimits {
        self.limits
    }

    pub fn resume_at(&mut self, spent: u32) {
        self.steps_used = self.steps_used.max(spent);
    }

    pub fn reserve_step(&mut self) -> bool {
        self.last_touched = now_ms();
        if self.steps_used >= self.limits.max_steps_per_run {
            return false;
        }
        self.steps_used += 1;
        true
    }

    pub fn reserve_step_leaving(&mut self, kept: u32) -> bool {
        self.last_touched = now_ms();
        if self.steps_remaining() <= kept {
            return false;
        }
        self.steps_used += 1;
        true
    }

    pub fn release_step(&mut self) {
        self.last_touched = now_ms();
        self.steps_used = self.steps_used.saturating_sub(1);
    }

    pub fn evaluate(&mut self, req: &SendRequest) -> Verdict {
        let generation = self.reply_generation(req.from, req.to);
        let verdict = self.evaluate_pending(req);
        if matches!(verdict, Verdict::Allow { .. }) {
            self.retire_reply(req.from, req.to, generation);
        }
        verdict
    }

    pub fn evaluate_pending(&mut self, req: &SendRequest) -> Verdict {
        self.last_touched = now_ms();

        if req.from == req.to {
            return Verdict::Refuse(Refusal::SelfAddressed);
        }

        let hop = req.inbound_hop.saturating_add(1);
        if hop > self.limits.max_hops {
            return Verdict::Refuse(Refusal::HopLimit { hop, max: self.limits.max_hops });
        }

        if self.steps_used >= self.limits.max_steps_per_run {
            return Verdict::Refuse(Refusal::RunBudgetExhausted {
                used: self.steps_used,
                max: self.limits.max_steps_per_run,
            });
        }

        let pair = (req.from, req.to);
        let sent = self.pair_counts.get(&pair).copied().unwrap_or(0);
        if sent >= self.limits.max_sends_per_pair {
            return Verdict::Refuse(Refusal::PairLimit {
                recipient: req.to_name.clone(),
                sent,
                max: self.limits.max_sends_per_pair,
            });
        }

        let fingerprint = content_fingerprint(
            Participant::Agent { id: req.from },
            Participant::Agent { id: req.to },
            &req.text,
        );
        if !self.fingerprints.insert(fingerprint) {
            return Verdict::Refuse(Refusal::DuplicateContent { recipient: req.to_name.clone() });
        }

        self.pair_counts.insert(pair, sent + 1);
        Verdict::Allow { hop }
    }

    pub fn reply_generation(&self, from: AgentId, to: AgentId) -> Option<u32> {
        self.expecting.get(&(to, from)).copied()
    }

    pub fn retire_reply(&mut self, from: AgentId, to: AgentId, generation: Option<u32>) {
        if generation.is_some() && self.reply_generation(from, to) == generation {
            self.expecting.remove(&(to, from));
        }
    }

    pub fn note_sent(&mut self, from: AgentId, to: AgentId, expects_reply: bool) {
        if expects_reply {
            let generation = self.pair_counts.get(&(from, to)).copied().unwrap_or(0);
            self.expecting.insert((from, to), generation);
        }
    }

    pub fn awaited(&self, me: AgentId) -> HashSet<AgentId> {
        self.expecting.keys().filter(|(from, _)| *from == me).map(|(_, to)| *to).collect()
    }

    pub fn has_written(&self, from: AgentId, to: AgentId) -> bool {
        self.pair_counts.get(&(from, to)).copied().unwrap_or(0) > 0
    }

    pub fn check_fanout(&self, requested: usize) -> Option<Refusal> {
        if requested > self.limits.max_fanout_per_call {
            return Some(Refusal::FanOutTooWide {
                requested,
                max: self.limits.max_fanout_per_call,
            });
        }
        None
    }
}

#[derive(Debug, Default)]
pub struct GuardRegistry {
    runs: HashMap<RunId, RunState>,
    pinned: HashSet<RunId>,
    warned_live: bool,
    max_retained: usize,
    retain_for_ms: i64,
}

impl GuardRegistry {
    pub fn restore(&mut self, id: RunId, state: RunState) {
        self.runs.insert(id, state);
    }
    pub fn new() -> Self {
        Self {
            runs: HashMap::new(),
            pinned: HashSet::new(),
            warned_live: false,
            max_retained: 256,
            retain_for_ms: 60 * 60 * 1000,
        }
    }

    pub fn pin(&mut self, id: RunId) {
        self.pinned.insert(id);
        if self.pinned.len() > 1024 && !self.warned_live {
            tracing::warn!(count = self.pinned.len(), "live guard count exceeds 1024");
            self.warned_live = true;
        }
    }

    pub fn unpin(&mut self, id: RunId) {
        self.pinned.remove(&id);
        self.reap(None);
    }

    pub fn run_within(&mut self, id: RunId, limits: GuardLimits) -> &mut RunState {
        self.reap(None);
        self.runs.entry(id).or_insert_with(|| RunState::new(limits));
        self.reap(Some(id));
        self.runs.get_mut(&id).expect("the requested guard is retained")
    }

    pub fn peek(&self, id: RunId) -> Option<&RunState> {
        self.runs.get(&id)
    }

    pub fn live_runs(&self) -> usize {
        self.runs.len()
    }

    fn reap(&mut self, protected: Option<RunId>) {
        let cutoff = now_ms() - self.retain_for_ms;
        self.runs.retain(|id, state| {
            Some(*id) == protected || self.pinned.contains(id) || state.last_touched >= cutoff
        });

        let settled =
            self.runs.len() - self.runs.keys().filter(|id| self.pinned.contains(id)).count();
        if settled > self.max_retained {
            let mut by_age: Vec<(RunId, i64)> = self
                .runs
                .iter()
                .filter(|(id, _)| Some(**id) != protected && !self.pinned.contains(id))
                .map(|(id, s)| (*id, s.last_touched))
                .collect();
            by_age.sort_by_key(|(_, touched)| *touched);
            let excess = settled - self.max_retained;
            for (id, _) in by_age.into_iter().take(excess) {
                self.runs.remove(&id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(from: AgentId, to: AgentId, text: &str, hop: u16) -> SendRequest {
        SendRequest { from, to, to_name: "Peer".into(), text: text.into(), inbound_hop: hop }
    }

    fn permissive() -> GuardLimits {
        GuardLimits {
            max_hops: 100,
            max_steps_per_run: 500,
            max_fanout_per_call: 64,
            max_sends_per_pair: 50,
            max_tool_rounds: 24,
        }
    }

    #[test]
    fn snapshot_restores_limits_counts_fingerprints_and_reply_debts() {
        let limits = GuardLimits { max_sends_per_pair: 2, ..permissive() };
        let (from, to) = (AgentId::new(), AgentId::new());
        let mut state = RunState::new(limits);
        assert!(state.reserve_step());
        assert!(matches!(state.evaluate(&req(from, to, "first", 0)), Verdict::Allow { .. }));
        state.note_sent(from, to, true);
        let json = serde_json::to_string(&state.snapshot()).unwrap();
        let mut restored = RunState::from_snapshot(serde_json::from_str(&json).unwrap()).unwrap();
        assert_eq!(restored.limits(), state.limits());
        assert_eq!(restored.steps_used(), 1);
        assert_eq!(restored.awaited(from), HashSet::from([to]));
        assert!(matches!(
            restored.evaluate(&req(from, to, "first", 0)),
            Verdict::Refuse(Refusal::DuplicateContent { .. })
        ));
        assert!(matches!(restored.evaluate(&req(from, to, "second", 0)), Verdict::Allow { .. }));
        assert!(matches!(
            restored.evaluate(&req(from, to, "third", 0)),
            Verdict::Refuse(Refusal::PairLimit { .. })
        ));
        let mut future: serde_json::Value = serde_json::from_str(&json).unwrap();
        future["version"] = 2.into();
        assert!(RunState::from_snapshot(serde_json::from_value(future).unwrap()).is_err());
    }

    #[test]
    fn an_errand_never_takes_the_steps_kept_for_the_turn_that_sent_it() {
        let mut state = RunState::new(GuardLimits { max_steps_per_run: 5, ..permissive() });
        assert!(state.reserve_step_leaving(2));
        assert!(state.reserve_step_leaving(2));
        assert!(state.reserve_step_leaving(2));
        assert!(!state.reserve_step_leaving(2), "two are left and both are kept");
        assert_eq!(state.steps_used(), 3, "a refused claim spends nothing");
        assert!(state.reserve_step(), "the turn itself can still spend what was kept");
        assert!(state.reserve_step());
        assert!(!state.reserve_step());
    }

    #[test]
    fn a_normal_send_is_allowed_and_increments_the_hop() {
        let mut state = RunState::new(permissive());
        let verdict = state.evaluate(&req(AgentId::new(), AgentId::new(), "hello", 0));
        assert_eq!(verdict, Verdict::Allow { hop: 1 });
    }

    #[test]
    fn self_addressed_messages_are_refused() {
        let mut state = RunState::new(permissive());
        let a = AgentId::new();
        assert_eq!(
            state.evaluate(&req(a, a, "talking to myself", 0)),
            Verdict::Refuse(Refusal::SelfAddressed)
        );
    }

    #[test]
    fn hop_limit_stops_a_delegation_chain() {
        let mut state = RunState::new(GuardLimits { max_hops: 2, ..permissive() });
        let (a, b) = (AgentId::new(), AgentId::new());
        assert_eq!(state.evaluate(&req(a, b, "one", 0)), Verdict::Allow { hop: 1 });
        assert_eq!(state.evaluate(&req(a, b, "two", 1)), Verdict::Allow { hop: 2 });
        assert_eq!(
            state.evaluate(&req(a, b, "three", 2)),
            Verdict::Refuse(Refusal::HopLimit { hop: 3, max: 2 })
        );
    }

    #[test]
    fn hop_counter_cannot_be_overflowed_into_wrapping() {
        let mut state = RunState::new(GuardLimits { max_hops: 4, ..permissive() });
        let verdict = state.evaluate(&req(AgentId::new(), AgentId::new(), "x", u16::MAX));
        assert_eq!(verdict, Verdict::Refuse(Refusal::HopLimit { hop: u16::MAX, max: 4 }));
    }

    #[test]
    fn ping_pong_between_two_agents_terminates() {
        let mut state =
            RunState::new(GuardLimits { max_hops: 100, max_sends_per_pair: 3, ..permissive() });
        let (a, b) = (AgentId::new(), AgentId::new());

        let mut allowed = 0;
        for turn in 0..50 {
            let (from, to) = if turn % 2 == 0 { (a, b) } else { (b, a) };
            let text = format!("ack {turn}");
            match state.evaluate(&req(from, to, &text, turn as u16)) {
                Verdict::Allow { .. } => allowed += 1,
                Verdict::Refuse(_) => break,
            }
        }
        assert_eq!(allowed, 6, "3 sends each direction, then the pair limit bites");
    }

    #[test]
    fn identical_content_to_the_same_peer_is_refused_once_seen() {
        let mut state = RunState::new(permissive());
        let (a, b) = (AgentId::new(), AgentId::new());
        assert!(matches!(
            state.evaluate(&req(a, b, "Introduce yourself", 0)),
            Verdict::Allow { .. }
        ));
        assert_eq!(
            state.evaluate(&req(a, b, "  introduce   YOURSELF ", 0)),
            Verdict::Refuse(Refusal::DuplicateContent { recipient: "Peer".into() }),
            "normalization means reformatting is not a way around the check"
        );
    }

    #[test]
    fn the_same_text_to_different_peers_is_allowed() {
        let mut state = RunState::new(permissive());
        let manager = AgentId::new();
        for _ in 0..5 {
            let peer = AgentId::new();
            assert!(
                matches!(
                    state.evaluate(&req(manager, peer, "Hi, I'm Manager", 0)),
                    Verdict::Allow { .. }
                ),
                "an identical introduction to a different agent must be allowed"
            );
        }
    }

    #[test]
    fn a_refused_send_does_not_consume_the_pair_budget() {
        let mut state = RunState::new(GuardLimits { max_hops: 1, ..permissive() });
        let (a, b) = (AgentId::new(), AgentId::new());
        assert!(matches!(state.evaluate(&req(a, b, "x", 5)), Verdict::Refuse(_)));
        assert!(matches!(state.evaluate(&req(a, b, "x", 0)), Verdict::Allow { hop: 1 }));
    }

    #[test]
    fn run_budget_is_a_hard_ceiling_across_all_agents() {
        let mut state = RunState::new(GuardLimits { max_steps_per_run: 3, ..permissive() });
        assert!(state.reserve_step());
        assert!(state.reserve_step());
        assert!(state.reserve_step());
        assert!(!state.reserve_step(), "fourth turn must be denied");
        assert_eq!(state.steps_remaining(), 0);
    }

    #[test]
    fn a_call_the_operator_cut_short_gives_its_step_back() {
        let mut state = RunState::new(GuardLimits { max_steps_per_run: 3, ..permissive() });
        assert!(state.reserve_step());
        assert!(state.reserve_step());
        state.release_step();
        assert_eq!(state.steps_used(), 1, "only the call that answered is on the bill");
        assert_eq!(state.steps_remaining(), 2);
    }

    #[test]
    fn releasing_a_step_that_was_never_claimed_does_nothing() {
        let mut state = RunState::new(GuardLimits { max_steps_per_run: 2, ..permissive() });
        state.release_step();
        assert_eq!(state.steps_used(), 0);
        assert!(state.reserve_step());
        assert!(state.reserve_step());
        assert!(!state.reserve_step(), "the ceiling still holds");
    }

    #[test]
    fn sends_are_refused_once_the_run_budget_is_spent() {
        let mut state = RunState::new(GuardLimits { max_steps_per_run: 1, ..permissive() });
        assert!(state.reserve_step());
        assert_eq!(
            state.evaluate(&req(AgentId::new(), AgentId::new(), "hi", 0)),
            Verdict::Refuse(Refusal::RunBudgetExhausted { used: 1, max: 1 }),
            "no point enqueueing work that can never be processed"
        );
    }

    #[test]
    fn fanout_wider_than_the_limit_is_refused_before_any_recipient_is_touched() {
        let state = RunState::new(GuardLimits { max_fanout_per_call: 3, ..permissive() });
        assert_eq!(state.check_fanout(3), None);
        assert_eq!(state.check_fanout(4), Some(Refusal::FanOutTooWide { requested: 4, max: 3 }));
    }

    #[test]
    fn limits_are_clamped_into_a_survivable_range() {
        let zeroed = GuardLimits {
            max_hops: 0,
            max_steps_per_run: 0,
            max_fanout_per_call: 0,
            max_sends_per_pair: 0,
            max_tool_rounds: 0,
        }
        .sanitized();
        assert_eq!(zeroed.max_hops, 1);
        assert_eq!(zeroed.max_steps_per_run, 1);
        assert_eq!(zeroed.max_fanout_per_call, 1);
        assert_eq!(zeroed.max_sends_per_pair, 1);
        assert_eq!(zeroed.max_tool_rounds, 1, "a turn that cannot act is a turn that does nothing");

        let huge = GuardLimits {
            max_hops: u16::MAX,
            max_steps_per_run: u32::MAX,
            max_fanout_per_call: usize::MAX,
            max_sends_per_pair: u32::MAX,
            max_tool_rounds: u16::MAX,
        }
        .sanitized();
        assert_eq!(huge.max_hops, 16);
        assert_eq!(huge.max_steps_per_run, 500);
        assert_eq!(huge.max_fanout_per_call, 64);
        assert_eq!(huge.max_tool_rounds, 100);
        assert_eq!(huge.max_sends_per_pair, 50);
    }

    #[test]
    fn a_broadcast_storm_across_five_agents_stays_within_default_budget() {
        let limits = GuardLimits::default();
        let mut state = RunState::new(limits);
        let manager = AgentId::new();
        let peers: Vec<AgentId> = (0..4).map(|_| AgentId::new()).collect();

        assert!(state.reserve_step(), "manager's tool call");
        assert!(state.reserve_step(), "manager's follow-up");
        assert_eq!(state.check_fanout(peers.len()), None);
        for peer in &peers {
            assert!(
                matches!(
                    state.evaluate(&req(manager, *peer, "Hi, I'm Manager", 0)),
                    Verdict::Allow { hop: 1 }
                ),
                "the introduction must reach every peer"
            );
        }
        for (i, peer) in peers.iter().enumerate() {
            assert!(state.reserve_step(), "peer {i} must get a turn to reply");
            assert!(
                matches!(
                    state.evaluate(&req(*peer, manager, &format!("Hi Manager, I'm agent {i}"), 1)),
                    Verdict::Allow { hop: 2 }
                ),
                "peer {i} must be able to reply"
            );
        }
        assert!(state.reserve_step(), "manager processes the replies");
        assert!(state.steps_remaining() > 0, "the demo must not exhaust the default budget");
        assert!(
            state.steps_used() <= 12,
            "the demo should cost well under the ceiling, used {}",
            state.steps_used()
        );
    }

    #[test]
    fn a_realistic_review_cycle_is_not_cut_off_by_the_defaults() {
        let mut state = RunState::new(GuardLimits::default());
        let manager = AgentId::new();
        let researcher = AgentId::new();
        let critic = AgentId::new();

        let chain = [
            (manager, researcher, 0u16),
            (researcher, manager, 1),
            (manager, critic, 2),
            (critic, researcher, 3),
            (researcher, critic, 4),
            (critic, researcher, 5),
            (researcher, critic, 6),
        ];
        for (step, (from, to, inbound)) in chain.iter().enumerate() {
            let verdict = state.evaluate(&req(*from, *to, &format!("step {step}"), *inbound));
            assert!(
                matches!(verdict, Verdict::Allow { .. }),
                "step {step} of a normal review cycle was refused: {verdict:?}"
            );
        }
    }

    #[test]
    fn a_coordinator_can_message_the_same_specialist_across_a_task() {
        let mut state = RunState::new(GuardLimits::default());
        let (manager, researcher) = (AgentId::new(), AgentId::new());
        for round in 0..5 {
            let verdict =
                state.evaluate(&req(manager, researcher, &format!("task update {round}"), 0));
            assert!(
                matches!(verdict, Verdict::Allow { .. }),
                "message {round} to the same specialist was refused"
            );
        }
    }

    #[test]
    fn budget_counts_model_calls_so_a_tool_heavy_turn_cannot_overspend() {
        let mut state = RunState::new(GuardLimits { max_steps_per_run: 3, ..permissive() });
        assert!(state.reserve_step(), "call 1: emits a tool call");
        assert!(state.reserve_step(), "call 2: emits another tool call");
        assert!(state.reserve_step(), "call 3: finally speaks");
        assert!(!state.reserve_step(), "a fourth call within the same turn must be denied");
        assert!(!state.has_budget());
    }

    #[test]
    fn a_run_keeps_the_limits_it_started_on() {
        let mut reg = GuardRegistry::new();
        let (tight, loose) = (RunId::new(), RunId::new());

        let strict = GuardLimits { max_steps_per_run: 2, ..GuardLimits::default() };
        assert!(reg.run_within(tight, strict).reserve_step());
        assert!(reg.run_within(loose, GuardLimits::default()).reserve_step());

        assert!(reg.run_within(tight, strict).reserve_step());
        assert!(!reg.run_within(tight, GuardLimits::default()).reserve_step(), "spent is spent");
        assert!(reg.run_within(loose, strict).reserve_step(), "the other run is untouched");
    }

    #[test]
    fn registry_isolates_runs_from_each_other() {
        let mut reg = GuardRegistry::new();
        let one_call = GuardLimits { max_steps_per_run: 1, ..permissive() };
        let (r1, r2) = (RunId::new(), RunId::new());

        assert!(reg.run_within(r1, one_call).reserve_step());
        assert!(!reg.run_within(r1, one_call).reserve_step());
        assert!(reg.run_within(r2, one_call).reserve_step(), "a fresh run gets its own budget");
    }

    #[test]
    fn registry_bounds_retained_runs() {
        let mut reg = GuardRegistry::new();
        reg.max_retained = 4;
        for _ in 0..40 {
            reg.run_within(RunId::new(), GuardLimits::default()).reserve_step();
        }
        assert!(reg.live_runs() <= 5, "expected reaping, got {} runs", reg.live_runs());
    }

    #[test]
    fn registry_reaps_by_age() {
        let mut reg = GuardRegistry::new();
        let old = RunId::new();
        reg.run_within(old, GuardLimits::default()).reserve_step();
        reg.runs.get_mut(&old).unwrap().last_touched = now_ms() - (2 * 60 * 60 * 1000);
        reg.run_within(RunId::new(), GuardLimits::default());
        assert!(reg.peek(old).is_none(), "stale run should have been reaped");
    }

    #[test]
    fn booked_run_keeps_its_limits_under_more_than_256_live_guards() {
        let mut reg = GuardRegistry::new();
        let limits = GuardLimits { max_steps_per_run: 2, max_sends_per_pair: 2, ..permissive() };
        let first = RunId::new();
        let (from, to) = (AgentId::new(), AgentId::new());
        reg.pin(first);
        assert!(matches!(
            reg.run_within(first, limits).evaluate(&req(from, to, "work", 0)),
            Verdict::Allow { .. }
        ));
        assert!(reg.run_within(first, limits).reserve_step());
        for _ in 0..300 {
            let run = RunId::new();
            reg.pin(run);
            reg.run_within(run, limits).reserve_step();
        }
        assert!(reg.run_within(first, limits).has_written(from, to));
        assert!(matches!(
            reg.run_within(first, limits).evaluate(&req(from, to, "work", 0)),
            Verdict::Refuse(Refusal::DuplicateContent { .. })
        ));
        assert!(matches!(
            reg.run_within(first, limits).evaluate(&req(from, to, "more", 0)),
            Verdict::Allow { .. }
        ));
        assert!(matches!(
            reg.run_within(first, limits).evaluate(&req(from, to, "third", 0)),
            Verdict::Refuse(Refusal::PairLimit { .. })
        ));
        assert!(reg.run_within(first, limits).reserve_step());
        assert!(!reg.run_within(first, limits).reserve_step());
    }

    #[test]
    fn booked_run_survives_age_expiry_until_it_settles() {
        let mut reg = GuardRegistry::new();
        let run = RunId::new();
        reg.pin(run);
        reg.run_within(run, permissive()).reserve_step();
        reg.runs.get_mut(&run).unwrap().last_touched = now_ms() - 2 * 60 * 60 * 1000;
        reg.run_within(RunId::new(), permissive());
        assert_eq!(reg.peek(run).unwrap().steps_used(), 1);
        reg.unpin(run);
        reg.run_within(RunId::new(), permissive());
        assert!(reg.peek(run).is_none());
    }

    #[test]
    fn a_delivered_ask_is_awaited_until_anything_travels_back() {
        let mut state = RunState::new(permissive());
        let (manager, chef) = (AgentId::new(), AgentId::new());

        assert!(matches!(
            state.evaluate(&req(manager, chef, "make bread", 0)),
            Verdict::Allow { .. }
        ));
        state.note_sent(manager, chef, true);
        assert_eq!(state.awaited(manager), HashSet::from([chef]));

        assert!(matches!(state.evaluate(&req(chef, manager, "done", 1)), Verdict::Allow { .. }));
        assert!(state.awaited(manager).is_empty());
    }

    #[test]
    fn an_in_flight_reply_remains_owed_until_delivery_finishes_or_fails() {
        let mut state = RunState::new(permissive());
        let (manager, chef) = (AgentId::new(), AgentId::new());
        state.note_sent(manager, chef, true);
        assert!(matches!(
            state.evaluate_pending(&req(chef, manager, "done", 1)),
            Verdict::Allow { .. }
        ));
        assert_eq!(state.awaited(manager), HashSet::from([chef]));
        state.retire_reply(chef, manager, state.reply_generation(chef, manager));
        assert!(state.awaited(manager).is_empty());

        state.note_sent(manager, chef, true);
        assert!(matches!(
            state.evaluate_pending(&req(chef, manager, "failed delivery", 1)),
            Verdict::Allow { .. }
        ));
        state.retire_reply(chef, manager, state.reply_generation(chef, manager));
        assert!(state.awaited(manager).is_empty());
    }

    #[test]
    fn an_older_delivery_cannot_retire_a_new_instruction() {
        let mut state = RunState::new(permissive());
        let (manager, chef) = (AgentId::new(), AgentId::new());
        assert!(matches!(
            state.evaluate_pending(&req(manager, chef, "first", 0)),
            Verdict::Allow { .. }
        ));
        state.note_sent(manager, chef, true);
        let old = state.reply_generation(chef, manager);
        assert!(matches!(
            state.evaluate_pending(&req(manager, chef, "second", 0)),
            Verdict::Allow { .. }
        ));
        state.note_sent(manager, chef, true);
        state.retire_reply(chef, manager, old);
        assert_eq!(state.awaited(manager), HashSet::from([chef]));
    }

    #[test]
    fn a_reply_blocked_before_delivery_is_still_awaited() {
        use std::sync::{Arc, Barrier, Mutex};

        let (manager, chef) = (AgentId::new(), AgentId::new());
        let state = Arc::new(Mutex::new(RunState::new(permissive())));
        state.lock().unwrap().note_sent(manager, chef, true);
        let paused = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let sender = {
            let (state, paused, release) = (state.clone(), paused.clone(), release.clone());
            std::thread::spawn(move || {
                let generation = {
                    let mut state = state.lock().unwrap();
                    let generation = state.reply_generation(chef, manager);
                    assert!(matches!(
                        state.evaluate_pending(&req(chef, manager, "done", 1)),
                        Verdict::Allow { .. }
                    ));
                    generation
                };
                paused.wait();
                release.wait();
                state.lock().unwrap().retire_reply(chef, manager, generation);
            })
        };
        paused.wait();
        let awaited = state.lock().unwrap().awaited(manager);
        release.wait();
        sender.join().unwrap();
        assert_eq!(awaited, HashSet::from([chef]));
        assert!(state.lock().unwrap().awaited(manager).is_empty());
    }

    #[test]
    fn a_send_that_expects_nothing_is_not_awaited() {
        let mut state = RunState::new(permissive());
        let (manager, chef) = (AgentId::new(), AgentId::new());
        assert!(matches!(state.evaluate(&req(manager, chef, "thanks", 0)), Verdict::Allow { .. }));
        state.note_sent(manager, chef, false);
        assert!(state.awaited(manager).is_empty());
    }

    #[test]
    fn a_second_instruction_is_awaited_although_the_peer_answered_the_first() {
        let mut state = RunState::new(permissive());
        let (manager, chef) = (AgentId::new(), AgentId::new());

        assert!(matches!(
            state.evaluate(&req(manager, chef, "round one", 0)),
            Verdict::Allow { .. }
        ));
        state.note_sent(manager, chef, true);
        assert!(matches!(
            state.evaluate(&req(chef, manager, "answer one", 1)),
            Verdict::Allow { .. }
        ));
        assert!(state.awaited(manager).is_empty());

        assert!(matches!(
            state.evaluate(&req(manager, chef, "round two", 2)),
            Verdict::Allow { .. }
        ));
        state.note_sent(manager, chef, true);
        assert_eq!(state.awaited(manager), HashSet::from([chef]));
    }

    #[test]
    fn every_refusal_explains_itself_in_terms_a_model_can_act_on() {
        let cases = [
            Refusal::HopLimit { hop: 5, max: 4 },
            Refusal::RunBudgetExhausted { used: 24, max: 24 },
            Refusal::PairLimit { recipient: "Chef".into(), sent: 3, max: 3 },
            Refusal::DuplicateContent { recipient: "Chef".into() },
            Refusal::FanOutTooWide { requested: 20, max: 8 },
            Refusal::SelfAddressed,
            Refusal::UnknownRecipient { recipient: "Ghost".into() },
            Refusal::RecipientTerminated { recipient: "Ghost".into() },
        ];
        for case in cases {
            let text = case.explain();
            assert!(text.starts_with("Refused:"), "{case:?} -> {text}");
            assert!(text.len() > 40, "{case:?} explanation is too thin: {text}");
            assert!(!case.headline().is_empty());
        }
    }
}
