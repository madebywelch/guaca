//! Loop guard.
//!
//! Bidirectional agent messaging does not terminate on its own. Two agents
//! being polite at each other is an infinite loop that costs real money, and
//! a broadcast to N agents that each reply to the sender is a fan-in storm.
//! None of the protocols in the interoperability literature specify a
//! termination condition; they describe how to address a peer, not when to
//! stop. So Guac supplies one.
//!
//! Five independent limits, because each catches a different shape of runaway
//! and any one of them alone has a hole:
//!
//! | Limit             | Catches                                        |
//! |-------------------|------------------------------------------------|
//! | hop depth         | long delegation chains, A->B->C->D->...         |
//! | run step budget   | everything else, as a hard ceiling on spend     |
//! | per-pair sends    | two agents ping-ponging inside the hop budget   |
//! | content dedup     | an agent restating itself to make progress      |
//! | fan-out width     | one call blasting every agent in the directory  |
//!
//! Refusals are returned to the calling model as tool results, never silently
//! dropped. An agent that is told "you have already sent Chef 3 messages this
//! run" will stop. An agent whose message vanishes will retry.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::domain::envelope::{content_fingerprint, Participant};
use crate::domain::ids::{AgentId, RunId};
use crate::domain::now_ms;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardLimits {
    /// Maximum agent-to-agent forwards away from the operator's message.
    pub max_hops: u16,
    /// Maximum inference calls across all agents in one run.
    ///
    /// Counted per model call, not per agent turn: one turn can call the model
    /// several times as it works through tool results, and the model call is
    /// the thing that costs money.
    pub max_steps_per_run: u32,
    /// Maximum recipients accepted by a single `send_message` call.
    pub max_fanout_per_call: usize,
    /// Maximum messages one agent may send to one specific peer per run.
    pub max_sends_per_pair: u32,
    /// Maximum model calls inside a single turn as an agent works through tool
    /// results.
    ///
    /// Working a browser is a loop of read, act, read again, and each pass is
    /// one of these. Four was enough when the only tools were messaging and
    /// notes, and far too few once an agent could use a computer: it stopped
    /// mid-task, having done part of the work and reported none of it.
    #[serde(default = "default_tool_rounds")]
    pub max_tool_rounds: u16,
}

pub fn default_tool_rounds() -> u16 {
    24
}

impl Default for GuardLimits {
    fn default() -> Self {
        // Tuned against a real working session, not the introduction demo.
        //
        // A four-agent review cycle (operator -> Manager -> Researcher ->
        // Manager -> Critic -> Researcher -> Critic) is seven hops of genuine
        // progress, and a coordinator legitimately messages the same specialist
        // several times across one task. Limits of 4 hops and 3 sends per pair
        // cut exactly that pattern off partway: the coordinator could not
        // forward a document it had just been asked to forward, and the rest of
        // the crew spent the thread asking for content that could never arrive.
        //
        // The run budget is the real spend control. Hops and per-pair counts
        // exist to stop relay chains and ping-pong, so they only have to sit
        // above the depth of honest work.
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
    /// Clamps operator-supplied limits into a range the runtime can survive.
    ///
    /// Settings come from the UI, so treat them as untrusted. A zero step
    /// budget deadlocks every run; an unbounded one is an open wallet.
    pub fn sanitized(self) -> Self {
        Self {
            max_hops: self.max_hops.clamp(1, 16),
            max_steps_per_run: self.max_steps_per_run.clamp(1, 500),
            max_fanout_per_call: self.max_fanout_per_call.clamp(1, 64),
            max_sends_per_pair: self.max_sends_per_pair.clamp(1, 50),
            // The run budget is the real spend control, so this can be generous:
            // a browsing turn legitimately takes dozens of passes.
            max_tool_rounds: self.max_tool_rounds.clamp(1, 100),
        }
    }
}

/// Why a send was refused. Every variant carries the numbers, because the
/// message goes back to a model that needs to understand it has hit a wall.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Refusal {
    HopLimit {
        hop: u16,
        max: u16,
    },
    RunBudgetExhausted {
        used: u32,
        max: u32,
    },
    PairLimit {
        recipient: String,
        sent: u32,
        max: u32,
    },
    DuplicateContent {
        recipient: String,
    },
    FanOutTooWide {
        requested: usize,
        max: usize,
    },
    SelfAddressed,
    UnknownRecipient {
        recipient: String,
    },
    RecipientTerminated {
        recipient: String,
    },
    /// Both sides have spoken and neither is waiting. See `Runtime::run_turn`.
    ExchangeSettled {
        recipient: String,
    },
}

impl Refusal {
    /// Phrasing aimed at a model, not a developer. States the wall, the
    /// numbers, and what to do instead, so the agent reports back rather than
    /// retrying into the same refusal.
    pub fn explain(&self) -> String {
        match self {
            Refusal::HopLimit { hop, max } => format!(
                "Refused: this conversation is {hop} hops from the operator and the limit is {max}. \
                 Do not relay further. Summarize what you have and reply to whoever asked you."
            ),
            Refusal::RunBudgetExhausted { used, max } => format!(
                "Refused: this run has used its full budget of {max} model calls ({used} used). \
                 No further messages can be sent. Reply to the operator with what you have."
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
                 an acknowledgement is not work. Reply to the operator instead, and say there \
                 what you still need. If you are giving {recipient} something to do rather than \
                 thanking them, send it again with intent \"work\", saying plainly what you need \
                 done."
            ),
        }
    }

    /// Short form for the transcript chip.
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

/// A single proposed agent-to-agent send, evaluated before it is enqueued.
#[derive(Debug, Clone)]
pub struct SendRequest {
    pub from: AgentId,
    pub to: AgentId,
    /// Display name of the recipient, used only to phrase refusals.
    pub to_name: String,
    pub text: String,
    /// Hop count of the envelope the sender is currently processing.
    pub inbound_hop: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Allow { hop: u16 },
    Refuse(Refusal),
}

/// Mutable guard state for one run.
#[derive(Debug)]
pub struct RunState {
    limits: GuardLimits,
    steps_used: u32,
    fingerprints: HashSet<String>,
    pair_counts: HashMap<(AgentId, AgentId), u32>,
    last_touched: i64,
}

impl RunState {
    pub fn new(limits: GuardLimits) -> Self {
        Self {
            limits: limits.sanitized(),
            steps_used: 0,
            fingerprints: HashSet::new(),
            pair_counts: HashMap::new(),
            last_touched: now_ms(),
        }
    }

    pub fn steps_used(&self) -> u32 {
        self.steps_used
    }

    pub fn steps_remaining(&self) -> u32 {
        self.limits.max_steps_per_run.saturating_sub(self.steps_used)
    }

    /// Whether another model call is affordable, without claiming it.
    ///
    /// Lets a turn bail out before building a prompt or telling the UI a
    /// message is coming.
    pub fn has_budget(&self) -> bool {
        self.steps_remaining() > 0
    }

    pub fn limits(&self) -> GuardLimits {
        self.limits
    }

    /// Claims one model call. Returns false when the run is spent.
    ///
    /// Called immediately before each invocation, including the extra calls a
    /// turn makes while working through tool results, so the budget tracks
    /// real spend rather than intent.
    pub fn reserve_step(&mut self) -> bool {
        self.last_touched = now_ms();
        if self.steps_used >= self.limits.max_steps_per_run {
            return false;
        }
        self.steps_used += 1;
        true
    }

    /// Evaluates one proposed send and, when allowed, records it.
    ///
    /// Recording on allow is what makes the per-pair and dedup limits real, so
    /// this cannot be split into a pure check plus a separate commit without
    /// opening a race between two agents sending concurrently.
    pub fn evaluate(&mut self, req: &SendRequest) -> Verdict {
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

    /// Checks fan-out width before any individual recipient is evaluated.
    /// How many peers this agent has written to that have not written back.
    ///
    /// The number of replies it is still owed, which is the only thing worth
    /// waiting for. Waiting on "is anyone in this run busy" instead made an
    /// agent sit through peers that were merely finishing their own notes.
    pub fn awaiting(&self, me: AgentId) -> usize {
        self.pair_counts
            .keys()
            .filter(|(from, to)| *from == me && !self.has_written(*to, me))
            .map(|(_, to)| *to)
            .collect::<HashSet<_>>()
            .len()
    }

    /// Whether `from` has written to `to` at any point in this run.
    ///
    /// Batch membership cannot answer this. Replies arrive milliseconds apart
    /// and an actor drains whatever has landed, so three peers answering at
    /// once can be split across turns: two of them then look like agents this
    /// one has never spoken to, and get messages that demand answers.
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

/// All live runs.
///
/// Runs are reaped on a time and count basis rather than on completion,
/// because "complete" is not observable: an agent may still be mid-inference
/// when its last peer goes quiet. Holding state a little too long is cheap;
/// dropping it early re-arms every limit mid-cascade.
#[derive(Debug)]
pub struct GuardRegistry {
    runs: HashMap<RunId, RunState>,
    default_limits: GuardLimits,
    max_retained: usize,
    retain_for_ms: i64,
}

impl GuardRegistry {
    pub fn new(default_limits: GuardLimits) -> Self {
        Self {
            runs: HashMap::new(),
            default_limits: default_limits.sanitized(),
            max_retained: 256,
            retain_for_ms: 60 * 60 * 1000,
        }
    }

    pub fn set_limits(&mut self, limits: GuardLimits) {
        self.default_limits = limits.sanitized();
    }

    pub fn default_limits(&self) -> GuardLimits {
        self.default_limits
    }

    /// Gets or creates the state for a run, reaping stale entries first.
    pub fn run(&mut self, id: RunId) -> &mut RunState {
        self.reap();
        let limits = self.default_limits;
        self.runs.entry(id).or_insert_with(|| RunState::new(limits))
    }

    pub fn peek(&self, id: RunId) -> Option<&RunState> {
        self.runs.get(&id)
    }

    pub fn live_runs(&self) -> usize {
        self.runs.len()
    }

    fn reap(&mut self) {
        let cutoff = now_ms() - self.retain_for_ms;
        self.runs.retain(|_, state| state.last_touched >= cutoff);

        // Bound memory even if every run is recent: drop the oldest first.
        if self.runs.len() > self.max_retained {
            let mut by_age: Vec<(RunId, i64)> =
                self.runs.iter().map(|(id, s)| (*id, s.last_touched)).collect();
            by_age.sort_by_key(|(_, touched)| *touched);
            let excess = self.runs.len() - self.max_retained;
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
        // A saturating add means a maxed inbound hop stays maxed rather than
        // wrapping to 0 and re-opening the budget.
        let verdict = state.evaluate(&req(AgentId::new(), AgentId::new(), "x", u16::MAX));
        assert_eq!(verdict, Verdict::Refuse(Refusal::HopLimit { hop: u16::MAX, max: 4 }));
    }

    #[test]
    fn ping_pong_between_two_agents_terminates() {
        // The canonical runaway: A and B politely acknowledging each other.
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
        // This is the whole point of the broadcast feature, so it must not trip
        // the dedup check.
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
        // Refused on hops.
        assert!(matches!(state.evaluate(&req(a, b, "x", 5)), Verdict::Refuse(_)));
        // The pair counter must still be clean, otherwise a hop refusal would
        // silently eat quota the agent never used.
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
        // The scenario from the brief: Manager introduces itself to everyone,
        // everyone replies. This must complete rather than hit a limit.
        let limits = GuardLimits::default();
        let mut state = RunState::new(limits);
        let manager = AgentId::new();
        let peers: Vec<AgentId> = (0..4).map(|_| AgentId::new()).collect();

        // Two calls: one that emits the tool call, one that speaks after the
        // tool result comes back.
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
        // Observed in real use: operator -> Manager -> Researcher -> Manager
        // -> Critic -> Researcher -> Critic -> Researcher. Every step is honest
        // progress, and the defaults must let all of it through.
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
        // A manager assigning, following up, and forwarding to one specialist
        // is three to five messages. Refusing the fourth strands the work.
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
        // A turn that loops on tool results makes several model calls. If the
        // budget counted turns instead, one turn could bill many times over.
        let mut state = RunState::new(GuardLimits { max_steps_per_run: 3, ..permissive() });
        assert!(state.reserve_step(), "call 1: emits a tool call");
        assert!(state.reserve_step(), "call 2: emits another tool call");
        assert!(state.reserve_step(), "call 3: finally speaks");
        assert!(!state.reserve_step(), "a fourth call within the same turn must be denied");
        assert!(!state.has_budget());
    }

    #[test]
    fn registry_isolates_runs_from_each_other() {
        let mut reg = GuardRegistry::new(GuardLimits { max_steps_per_run: 1, ..permissive() });
        let (r1, r2) = (RunId::new(), RunId::new());

        assert!(reg.run(r1).reserve_step());
        assert!(!reg.run(r1).reserve_step());
        assert!(reg.run(r2).reserve_step(), "a fresh run gets its own budget");
    }

    #[test]
    fn registry_bounds_retained_runs() {
        let mut reg = GuardRegistry::new(GuardLimits::default());
        reg.max_retained = 4;
        for _ in 0..40 {
            reg.run(RunId::new()).reserve_step();
        }
        assert!(reg.live_runs() <= 5, "expected reaping, got {} runs", reg.live_runs());
    }

    #[test]
    fn registry_reaps_by_age() {
        let mut reg = GuardRegistry::new(GuardLimits::default());
        let old = RunId::new();
        reg.run(old).reserve_step();
        // Backdate past the retention window.
        reg.runs.get_mut(&old).unwrap().last_touched = now_ms() - (2 * 60 * 60 * 1000);
        reg.run(RunId::new());
        assert!(reg.peek(old).is_none(), "stale run should have been reaped");
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
