//! Reading what a run *did*, and saying whether the machinery behaved.
//!
//! `eval.rs` reads the envelopes and asks whether a crew communicated sensibly.
//! This reads the event stream the UI is drawn from and asks the other half of
//! the question: did the run itself work. The two failures do not overlap. A
//! stream that opens and never closes leaves a placeholder on screen forever
//! while every message in the transcript is perfect. A settle that fires while
//! an agent is still thinking stops the spinner and then keeps talking. A
//! budget that counts something other than model calls bills a bounded run
//! several times over, and the messages it produced look exactly the same.
//!
//! Same discipline as `eval.rs`: every anomaly is decidable from the record.
//! Nothing is scored and nothing is timed, because an assertion with a wall
//! clock in it is a flake, and "the run took too long" is already what the
//! harness's settle timeout says.
//!
//! Four events carry a run: the messages, the streams, the token counts and
//! the settle. Stream deltas and ends carry only a message id and are matched
//! to the stream that announced them. Activity and approvals carry neither, so
//! they are taken for the agents this run touched: exact while one run is in
//! flight, which is what an end-to-end scenario is.

use std::collections::{HashMap, HashSet};

use crate::domain::approval::ApprovalState;
use crate::domain::envelope::{NoticeKind, Part, Participant, ToolOutcome};
use crate::domain::ids::{AgentId, ApprovalId, MessageId, RunId};
use crate::runtime::events::{Activity, UiEvent};

/// One thing the runtime reported, in the order it reported it.
#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    /// The operator gave an agent something to do.
    Asked {
        agent: AgentId,
    },
    /// The badge next to an agent's name changed.
    Doing {
        agent: AgentId,
        activity: Activity,
    },
    /// A placeholder opened for a message the operator can watch arrive.
    StreamOpened {
        agent: AgentId,
        message: MessageId,
        to: Participant,
    },
    StreamText {
        message: MessageId,
        chars: usize,
    },
    StreamClosed {
        message: MessageId,
    },
    /// One model call, as the provider counted it.
    Called {
        agent: AgentId,
        prompt: u32,
        completion: u32,
    },
    /// A message was persisted.
    Said {
        from: Participant,
        to: Participant,
        chars: usize,
    },
    /// A tool an agent used, and how it went.
    Used {
        agent: AgentId,
        tool: String,
        outcome: ToolOutcome,
    },
    /// Guaca speaking into a channel: a guard refusal, a failed call.
    Noticed {
        agent: AgentId,
        kind: NoticeKind,
        text: String,
    },
    /// An agent parked mid-turn on the operator.
    Parked {
        agent: AgentId,
        approval: ApprovalId,
    },
    Answered {
        approval: ApprovalId,
        state: ApprovalState,
    },
    /// Every agent in the run went quiet.
    Settled {
        steps: u32,
    },
}

/// Something about the way a run ran that an operator would call broken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anomaly {
    /// A placeholder the UI never got to replace. On screen this is a message
    /// that stays half-arrived for as long as the window is open.
    StreamLeftOpen { agent: String },
    /// Text for a stream that had already ended. This is what a retry looks
    /// like when it appends to the attempt it was meant to replace.
    TextAfterTheStreamEnded { agent: String },
    /// Text or an end for a stream nothing announced: the UI has nowhere to
    /// put it and drops it.
    UnannouncedStream { message: MessageId },
    /// The budget did not count what the run spent. Steps are reserved one per
    /// model call, so a run that reports a different number is either billing
    /// turns or losing calls, and both are how a bounded run overspends.
    BudgetMiscounted { steps: u32, calls: usize },
    /// The run was declared over while somebody was still in the middle of it.
    StillWorkingWhenTheRunSettled { agent: String, doing: &'static str },
    /// Something arrived after the run was declared over. The operator watched
    /// the spinner stop and then watched more text appear.
    ActedAfterTheRunSettled { agent: String },
    /// A turn parked on a person and was never released. The agent is holding
    /// a line open that nothing will ever close.
    ParkedWithoutAnAnswer { agent: String },
    /// Once per run, or the UI counts one run's spend twice.
    SettledMoreThanOnce { times: usize },
    /// Nothing ever said the run was over.
    NeverSettled,
    /// A tool raised an error rather than returning a verdict. A refusal is
    /// the guard working; this is something breaking.
    ToolFailed { agent: String, tool: String, error: String },
    /// Every attempt at a model call failed and the operator was told.
    CallFailed { agent: String },
}

impl Anomaly {
    pub fn explain(&self) -> String {
        match self {
            Anomaly::StreamLeftOpen { agent } => {
                format!("{agent} opened a stream and never closed it: the UI draws that forever")
            }
            Anomaly::TextAfterTheStreamEnded { agent } => {
                format!("{agent} streamed text into a placeholder that had already ended")
            }
            Anomaly::UnannouncedStream { message } => {
                format!("text arrived for stream {}, which nothing announced", message.short())
            }
            Anomaly::BudgetMiscounted { steps, calls } => format!(
                "the run reported {steps} steps for {calls} model calls: the budget counts calls"
            ),
            Anomaly::StillWorkingWhenTheRunSettled { agent, doing } => {
                format!("the run settled while {agent} was still {doing}")
            }
            Anomaly::ActedAfterTheRunSettled { agent } => {
                format!("{agent} acted after the run had been reported as finished")
            }
            Anomaly::ParkedWithoutAnAnswer { agent } => {
                format!("{agent} parked on the operator and was never released")
            }
            Anomaly::SettledMoreThanOnce { times } => {
                format!("the run settled {times} times; spend is counted once per settle")
            }
            Anomaly::NeverSettled => "the run never settled".into(),
            Anomaly::ToolFailed { agent, tool, error } => {
                format!("{agent}'s {tool} call failed: {error}")
            }
            Anomaly::CallFailed { agent } => {
                format!("{agent} could not reach the model, retries included")
            }
        }
    }
}

/// What one run's machinery did.
#[derive(Debug, Clone)]
pub struct Trajectory {
    pub run: RunId,
    pub records: Vec<Record>,
    /// Rendered for a failing assertion to print. Nobody can act on a count.
    pub ledger: String,
    names: HashMap<AgentId, String>,
}

impl Trajectory {
    /// Model calls the providers counted. The unit the budget is spent in.
    pub fn calls(&self) -> usize {
        self.records.iter().filter(|r| matches!(r, Record::Called { .. })).count()
    }

    /// Prompt and completion tokens, summed.
    pub fn tokens(&self) -> (u64, u64) {
        self.records.iter().fold((0, 0), |(p, c), record| match record {
            Record::Called { prompt, completion, .. } => {
                (p + *prompt as u64, c + *completion as u64)
            }
            _ => (p, c),
        })
    }

    /// What the run reported spending when it finished.
    pub fn steps(&self) -> Option<u32> {
        self.records.iter().find_map(|r| match r {
            Record::Settled { steps } => Some(*steps),
            _ => None,
        })
    }

    /// How many turns an agent took.
    ///
    /// A turn that parked on the operator comes back to `Thinking` when it is
    /// released, which is the same turn resuming rather than a second one. A
    /// count that read it as two would make every approval look like the agent
    /// had been woken twice.
    pub fn turns(&self, agent: AgentId) -> usize {
        let mut turns = 0;
        let mut was = Activity::Idle;
        for record in &self.records {
            let Record::Doing { agent: id, activity } = record else { continue };
            if *id != agent {
                continue;
            }
            if *activity == Activity::Thinking && was != Activity::AwaitingApproval {
                turns += 1;
            }
            was = *activity;
        }
        turns
    }

    /// The most agents that were mid-inference at the same moment.
    ///
    /// The claim the Rust runtime exists to make. Read from the interleaving
    /// rather than from a clock: five agents that each took 300ms and finished
    /// inside a second is an inference about a machine that was not busy, while
    /// five open turns at one point in the ledger is the thing itself.
    pub fn peak_concurrency(&self) -> usize {
        let mut thinking: HashSet<AgentId> = HashSet::new();
        let mut peak = 0;
        for record in &self.records {
            let Record::Doing { agent, activity } = record else { continue };
            match activity {
                Activity::Thinking => {
                    thinking.insert(*agent);
                }
                _ => {
                    thinking.remove(agent);
                }
            }
            peak = peak.max(thinking.len());
        }
        peak
    }

    /// Every tool used, in order, by name.
    pub fn tools(&self) -> Vec<&str> {
        self.records
            .iter()
            .filter_map(|r| match r {
                Record::Used { tool, .. } => Some(tool.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Every tool call that was turned away, worded as the model read it.
    pub fn refusals(&self) -> Vec<String> {
        self.records
            .iter()
            .flat_map(|r| match r {
                Record::Used { outcome: ToolOutcome::Refused { reason }, .. } => {
                    vec![reason.clone()]
                }
                Record::Used { outcome: ToolOutcome::Partial { refused, .. }, .. } => {
                    refused.iter().map(|r| r.reason.clone()).collect()
                }
                _ => Vec::new(),
            })
            .collect()
    }

    /// Everything wrong with the way this run ran.
    pub fn anomalies(&self) -> Vec<Anomaly> {
        let mut found = Vec::new();
        let name = |id: &AgentId| self.names.get(id).cloned().unwrap_or_else(|| "?".into());

        // Streams, walked in order: what is still open at the end is the
        // placeholder nobody will replace, and text after a close is the retry
        // bug that appends a second attempt onto the first one's half-sentence.
        let mut open: HashMap<MessageId, AgentId> = HashMap::new();
        let mut closed: HashMap<MessageId, AgentId> = HashMap::new();
        for record in &self.records {
            match record {
                Record::StreamOpened { agent, message, .. } => {
                    open.insert(*message, *agent);
                }
                Record::StreamClosed { message } => {
                    match open.remove(message) {
                        Some(agent) => {
                            closed.insert(*message, agent);
                        }
                        None if !closed.contains_key(message) => {
                            found.push(Anomaly::UnannouncedStream { message: *message })
                        }
                        None => {}
                    };
                }
                Record::StreamText { message, .. } => {
                    if let Some(agent) = closed.get(message) {
                        found.push(Anomaly::TextAfterTheStreamEnded { agent: name(agent) });
                    } else if !open.contains_key(message) {
                        found.push(Anomaly::UnannouncedStream { message: *message });
                    }
                }
                _ => {}
            }
        }
        let mut left_open: Vec<String> = open.values().map(name).collect();
        left_open.sort();
        found.extend(left_open.into_iter().map(|agent| Anomaly::StreamLeftOpen { agent }));

        // Tools that broke, and calls that never landed.
        for record in &self.records {
            match record {
                Record::Used { agent, tool, outcome: ToolOutcome::Failed { error } } => {
                    found.push(Anomaly::ToolFailed {
                        agent: name(agent),
                        tool: tool.clone(),
                        error: error.clone(),
                    })
                }
                Record::Noticed { agent, kind: NoticeKind::UpstreamError, .. } => {
                    found.push(Anomaly::CallFailed { agent: name(agent) })
                }
                _ => {}
            }
        }

        // A parked turn is a held line. Anything still pending here is one
        // nothing will ever release.
        let mut parked: HashMap<ApprovalId, AgentId> = HashMap::new();
        for record in &self.records {
            match record {
                Record::Parked { agent, approval } => {
                    parked.insert(*approval, *agent);
                }
                Record::Answered { approval, .. } => {
                    parked.remove(approval);
                }
                _ => {}
            }
        }
        let mut unanswered: Vec<String> = parked.values().map(name).collect();
        unanswered.sort();
        found.extend(unanswered.into_iter().map(|agent| Anomaly::ParkedWithoutAnAnswer { agent }));

        // The budget. A step is reserved per model call, so the number the run
        // reports has to be the number of calls it made. A call that failed
        // every attempt reserved its step and reported no usage, which is why
        // the failures are added back rather than ignored.
        //
        // A provider that counts nothing leaves this unanswerable rather than
        // wrong: with no usage reported at all there is nothing to compare.
        let calls = self.calls();
        let failed = self
            .records
            .iter()
            .filter(|r| matches!(r, Record::Noticed { kind: NoticeKind::UpstreamError, .. }))
            .count();
        if let Some(steps) = self.steps() {
            if calls > 0 && steps as usize != calls + failed {
                found.push(Anomaly::BudgetMiscounted { steps, calls });
            }
        }

        // The settle, and everything either side of it.
        let settles: Vec<usize> = self
            .records
            .iter()
            .enumerate()
            .filter_map(|(at, r)| matches!(r, Record::Settled { .. }).then_some(at))
            .collect();
        match settles.first() {
            None => found.push(Anomaly::NeverSettled),
            Some(&at) => {
                if settles.len() > 1 {
                    found.push(Anomaly::SettledMoreThanOnce { times: settles.len() });
                }

                let mut doing: HashMap<AgentId, Activity> = HashMap::new();
                for record in &self.records[..at] {
                    if let Record::Doing { agent, activity } = record {
                        doing.insert(*agent, *activity);
                    }
                }
                let mut stuck: Vec<(String, &'static str)> = doing
                    .into_iter()
                    .filter_map(|(agent, activity)| match activity {
                        Activity::Thinking => Some((name(&agent), "thinking")),
                        Activity::AwaitingApproval => {
                            Some((name(&agent), "waiting on the operator"))
                        }
                        _ => None,
                    })
                    .collect();
                stuck.sort();
                found.extend(
                    stuck.into_iter().map(|(agent, doing)| {
                        Anomaly::StillWorkingWhenTheRunSettled { agent, doing }
                    }),
                );

                // Work filed against a run that was already reported finished.
                let mut late: Vec<String> = self.records[at + 1..]
                    .iter()
                    .filter_map(|record| match record {
                        Record::Called { agent, .. }
                        | Record::StreamOpened { agent, .. }
                        | Record::Used { agent, .. } => Some(name(agent)),
                        Record::Said { from: Participant::Agent { id }, .. } => Some(name(id)),
                        _ => None,
                    })
                    .collect();
                late.sort();
                late.dedup();
                found.extend(
                    late.into_iter().map(|agent| Anomaly::ActedAfterTheRunSettled { agent }),
                );
            }
        }

        found
    }
}

/// Reads the events a run produced into something that can be asserted on.
///
/// `name_of` resolves an agent id for the ledger and the anomalies, so this can
/// be run against a scripted harness or a live session without knowing which.
pub fn read(events: &[UiEvent], run: RunId, name_of: &dyn Fn(AgentId) -> String) -> Trajectory {
    // Which stream belongs to which run, learned from every announcement in
    // the stream rather than only this run's: a delta for another run's stream
    // is somebody else's traffic, and calling it unannounced would report a
    // bug that is only this filter's own blind spot.
    let stream_runs: HashMap<MessageId, RunId> = events
        .iter()
        .filter_map(|e| match e {
            UiEvent::StreamStarted { message_id, run_id, .. } => Some((*message_id, *run_id)),
            _ => None,
        })
        .collect();

    // Who this run touched. Activity and approvals carry no run, so they are
    // read for these agents and nobody else.
    let mut involved: HashSet<AgentId> = HashSet::new();
    for event in events {
        match event {
            UiEvent::MessageAppended { message } if message.run_id == run => {
                involved.extend(message.from.agent_id());
                involved.extend(message.to.agent_id());
            }
            UiEvent::StreamStarted { agent_id, run_id, .. } if *run_id == run => {
                involved.insert(*agent_id);
            }
            UiEvent::TokensUsed { agent_id, run_id, .. } if *run_id == run => {
                involved.insert(*agent_id);
            }
            _ => {}
        }
    }

    let mut records: Vec<Record> = Vec::new();
    let mut names: HashMap<AgentId, String> = HashMap::new();
    let mut asked: HashSet<ApprovalId> = HashSet::new();
    let resolve = |id: AgentId, names: &mut HashMap<AgentId, String>| {
        names.entry(id).or_insert_with(|| name_of(id));
    };

    for event in events {
        match event {
            UiEvent::MessageAppended { message } if message.run_id == run => {
                for id in [message.from.agent_id(), message.to.agent_id()].into_iter().flatten() {
                    resolve(id, &mut names);
                }
                // The parts first: a tool ran and a notice was written before
                // the message carrying them was persisted.
                let channel = message.channel_id;
                for part in &message.parts {
                    match part {
                        Part::ToolCall { name: tool, outcome, .. } => {
                            let agent = message.from.agent_id().unwrap_or(channel);
                            resolve(agent, &mut names);
                            records.push(Record::Used {
                                agent,
                                tool: tool.clone(),
                                outcome: outcome.clone(),
                            });
                        }
                        Part::Notice { kind, text } => {
                            resolve(channel, &mut names);
                            records.push(Record::Noticed {
                                agent: channel,
                                kind: *kind,
                                text: text.clone(),
                            });
                        }
                        _ => {}
                    }
                }
                let text = message.plain_text();
                if matches!(message.from, Participant::Human) {
                    if let Some(agent) = message.to.agent_id() {
                        records.push(Record::Asked { agent });
                    }
                } else if !text.is_empty() {
                    records.push(Record::Said {
                        from: message.from,
                        to: message.to,
                        chars: text.chars().count(),
                    });
                }
            }
            UiEvent::StreamStarted { message_id, agent_id, run_id, to, .. } if *run_id == run => {
                resolve(*agent_id, &mut names);
                records.push(Record::StreamOpened {
                    agent: *agent_id,
                    message: *message_id,
                    to: *to,
                });
            }
            UiEvent::StreamDelta { message_id, text, .. } => {
                if stream_runs.get(message_id).is_some_and(|owner| *owner != run) {
                    continue;
                }
                let chars = text.chars().count();
                // Coalesced: a ledger with one line per token is unreadable,
                // and the question here is whether text arrived, not in how
                // many pieces.
                match records.last_mut() {
                    Some(Record::StreamText { message, chars: so_far })
                        if message == message_id =>
                    {
                        *so_far += chars
                    }
                    _ => records.push(Record::StreamText { message: *message_id, chars }),
                }
            }
            UiEvent::StreamEnded { message_id, .. } => {
                if stream_runs.get(message_id).is_some_and(|owner| *owner != run) {
                    continue;
                }
                records.push(Record::StreamClosed { message: *message_id });
            }
            UiEvent::TokensUsed { agent_id, run_id, prompt, completion, .. } if *run_id == run => {
                resolve(*agent_id, &mut names);
                records.push(Record::Called {
                    agent: *agent_id,
                    prompt: *prompt,
                    completion: *completion,
                });
            }
            UiEvent::ActivityChanged { agent_id, activity } if involved.contains(agent_id) => {
                resolve(*agent_id, &mut names);
                records.push(Record::Doing { agent: *agent_id, activity: *activity });
            }
            UiEvent::ApprovalRequested { approval_id, agent_id } if involved.contains(agent_id) => {
                resolve(*agent_id, &mut names);
                asked.insert(*approval_id);
                records.push(Record::Parked { agent: *agent_id, approval: *approval_id });
            }
            UiEvent::ApprovalSettled { approval_id, state } if asked.contains(approval_id) => {
                records.push(Record::Answered { approval: *approval_id, state: *state });
            }
            UiEvent::RunSettled { run_id, steps_used } if *run_id == run => {
                records.push(Record::Settled { steps: *steps_used });
            }
            _ => {}
        }
    }

    let ledger = render(run, &records, &names);
    Trajectory { run, records, ledger, names }
}

fn render(run: RunId, records: &[Record], names: &HashMap<AgentId, String>) -> String {
    let name = |id: &AgentId| names.get(id).cloned().unwrap_or_else(|| "?".into());
    let who = |p: &Participant| match p {
        Participant::Human => "the operator".to_string(),
        Participant::System => "Guaca".to_string(),
        Participant::Agent { id } => name(id),
    };
    // Streams are addressed by message id, which says nothing to a reader. The
    // agent that opened one is what a line about it should say.
    let mut owners: HashMap<MessageId, AgentId> = HashMap::new();
    for record in records {
        if let Record::StreamOpened { agent, message, .. } = record {
            owners.insert(*message, *agent);
        }
    }
    let owner = |message: &MessageId| match owners.get(message) {
        Some(agent) => name(agent),
        None => format!("stream {message}"),
    };

    // Named, because a failure that prints two of these has to say which is
    // which: the retry of a failed turn is its own run.
    let mut out = format!("run {}:\n", run.short());
    for record in records {
        let line = match record {
            Record::Asked { agent } => format!("the operator asks {}", name(agent)),
            Record::Doing { agent, activity } => format!("{} is {}", name(agent), badge(activity)),
            Record::StreamOpened { agent, to, .. } => {
                format!("{} starts writing to {}", name(agent), who(to))
            }
            Record::StreamText { message, chars } => {
                format!("{} streams {chars} characters", owner(message))
            }
            Record::StreamClosed { message } => format!("{} stops writing", owner(message)),
            Record::Called { agent, prompt, completion } => {
                format!("{} calls the model ({prompt} in, {completion} out)", name(agent))
            }
            Record::Said { from, to, chars } => {
                format!("{} -> {}: {chars} characters", who(from), who(to))
            }
            Record::Used { agent, tool, outcome } => {
                format!("{} uses {tool}: {}", name(agent), verdict(outcome))
            }
            Record::Noticed { agent, kind, text } => {
                format!("Guaca tells {} ({kind:?}): {text}", name(agent))
            }
            Record::Parked { agent, .. } => format!("{} waits for the operator", name(agent)),
            Record::Answered { state, .. } => format!("the operator answers: {state:?}"),
            Record::Settled { steps } => format!("the run settles after {steps} model calls"),
        };
        out.push_str("  ");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

fn badge(activity: &Activity) -> String {
    match activity {
        Activity::Idle => "idle".into(),
        Activity::Thinking => "thinking".into(),
        Activity::Queued { depth } => format!("queued ({depth} waiting)"),
        Activity::AwaitingApproval => "waiting on the operator".into(),
        Activity::Paused => "paused".into(),
    }
}

fn verdict(outcome: &ToolOutcome) -> String {
    match outcome {
        ToolOutcome::Ok { summary } => format!("ok, {summary}"),
        ToolOutcome::Partial { summary, refused } => {
            format!("partly, {summary} ({} refused)", refused.len())
        }
        ToolOutcome::Refused { reason } => format!("refused, {reason}"),
        ToolOutcome::Failed { error } => format!("failed, {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::envelope::{Envelope, Intent, Trust};
    use crate::domain::ids::GroupId;

    fn named(ids: &[(AgentId, &'static str)]) -> impl Fn(AgentId) -> String {
        let table: HashMap<AgentId, String> =
            ids.iter().map(|(id, name)| (*id, (*name).to_string())).collect();
        move |id| table.get(&id).cloned().unwrap_or_else(|| "someone".into())
    }

    fn appended(run: RunId, from: Participant, to: Participant, parts: Vec<Part>) -> UiEvent {
        let channel = to.agent_id().or_else(|| from.agent_id()).unwrap_or_default();
        UiEvent::MessageAppended {
            message: Box::new(Envelope {
                id: MessageId::new(),
                run_id: run,
                channel_id: channel,
                from,
                to,
                parts,
                trust: Trust::Peer,
                hop: 0,
                expects_reply: false,
                intent: Intent::Courtesy,
                cause: None,
                created_at: 0,
            }),
        }
    }

    /// The events one healthy turn produces, in the order the runtime emits
    /// them: told, thinking, a placeholder, tokens, text, the message, idle.
    fn one_clean_turn(run: RunId, agent: AgentId, message: MessageId) -> Vec<UiEvent> {
        vec![
            appended(
                run,
                Participant::Human,
                Participant::Agent { id: agent },
                vec![Part::text("say something")],
            ),
            UiEvent::ActivityChanged { agent_id: agent, activity: Activity::Thinking },
            UiEvent::StreamStarted {
                message_id: message,
                channel_id: agent,
                agent_id: agent,
                run_id: run,
                to: Participant::Human,
            },
            UiEvent::StreamDelta { message_id: message, channel_id: agent, text: "hel".into() },
            UiEvent::StreamDelta { message_id: message, channel_id: agent, text: "lo".into() },
            UiEvent::TokensUsed {
                agent_id: agent,
                group_id: GroupId::new(),
                run_id: run,
                prompt: 120,
                completion: 8,
                cost: None,
            },
            UiEvent::StreamEnded { message_id: message, channel_id: agent },
            appended(
                run,
                Participant::Agent { id: agent },
                Participant::Human,
                vec![Part::text("hello")],
            ),
            UiEvent::ActivityChanged { agent_id: agent, activity: Activity::Idle },
            UiEvent::RunSettled { run_id: run, steps_used: 1 },
        ]
    }

    #[test]
    fn a_turn_that_opened_a_stream_closed_it_and_settled_has_nothing_wrong_with_it() {
        let (run, agent) = (RunId::new(), AgentId::new());
        let events = one_clean_turn(run, agent, MessageId::new());

        let t = read(&events, run, &named(&[(agent, "Manager")]));
        assert_eq!(t.anomalies(), vec![]);
        assert_eq!(t.calls(), 1);
        assert_eq!(t.tokens(), (120, 8));
        assert_eq!(t.steps(), Some(1));
        assert_eq!(t.turns(agent), 1);
        assert!(t.ledger.contains("Manager calls the model (120 in, 8 out)"), "{}", t.ledger);
    }

    #[test]
    fn a_placeholder_that_never_closes_is_named_with_the_agent_holding_it() {
        // What the operator sees is a message that stays half-arrived until
        // the window is closed, and nothing in the transcript is wrong.
        let (run, agent) = (RunId::new(), AgentId::new());
        let mut events = one_clean_turn(run, agent, MessageId::new());
        events.retain(|e| !matches!(e, UiEvent::StreamEnded { .. }));

        let t = read(&events, run, &named(&[(agent, "Manager")]));
        assert_eq!(t.anomalies(), vec![Anomaly::StreamLeftOpen { agent: "Manager".into() }]);
    }

    #[test]
    fn a_retry_that_appends_to_the_attempt_it_replaced_is_caught() {
        // The reason a retry opens a new placeholder: text from the second
        // attempt landing in the first one's box reads as a sentence starting
        // over halfway through.
        let (run, agent) = (RunId::new(), AgentId::new());
        let message = MessageId::new();
        let mut events = one_clean_turn(run, agent, message);
        let ended = events
            .iter()
            .position(|e| matches!(e, UiEvent::StreamEnded { .. }))
            .expect("the clean turn closes its stream");
        events.insert(
            ended + 1,
            UiEvent::StreamDelta {
                message_id: message,
                channel_id: agent,
                text: "hello again".into(),
            },
        );

        let t = read(&events, run, &named(&[(agent, "Manager")]));
        assert_eq!(
            t.anomalies(),
            vec![Anomaly::TextAfterTheStreamEnded { agent: "Manager".into() }]
        );
    }

    #[test]
    fn text_for_a_placeholder_nothing_opened_is_text_the_ui_has_nowhere_to_put() {
        let (run, agent) = (RunId::new(), AgentId::new());
        let orphan = MessageId::new();
        let mut events = one_clean_turn(run, agent, MessageId::new());
        events.insert(
            3,
            UiEvent::StreamDelta { message_id: orphan, channel_id: agent, text: "lost".into() },
        );

        let t = read(&events, run, &named(&[(agent, "Manager")]));
        assert_eq!(t.anomalies(), vec![Anomaly::UnannouncedStream { message: orphan }]);
    }

    #[test]
    fn a_settle_that_fires_while_somebody_is_thinking_is_the_spinner_stopping_early() {
        let (run, manager, chef) = (RunId::new(), AgentId::new(), AgentId::new());
        let mut events = one_clean_turn(run, manager, MessageId::new());
        // Chef is in the run and never came back.
        events.insert(
            1,
            appended(
                run,
                Participant::Agent { id: manager },
                Participant::Agent { id: chef },
                vec![Part::text("look this up")],
            ),
        );
        events.insert(2, UiEvent::ActivityChanged { agent_id: chef, activity: Activity::Thinking });

        let t = read(&events, run, &named(&[(manager, "Manager"), (chef, "Chef")]));
        assert_eq!(
            t.anomalies(),
            vec![Anomaly::StillWorkingWhenTheRunSettled {
                agent: "Chef".into(),
                doing: "thinking"
            }]
        );
    }

    #[test]
    fn a_message_filed_against_a_finished_run_is_named() {
        let (run, agent) = (RunId::new(), AgentId::new());
        let mut events = one_clean_turn(run, agent, MessageId::new());
        events.push(appended(
            run,
            Participant::Agent { id: agent },
            Participant::Human,
            vec![Part::text("one more thing")],
        ));

        let t = read(&events, run, &named(&[(agent, "Manager")]));
        assert_eq!(
            t.anomalies(),
            vec![Anomaly::ActedAfterTheRunSettled { agent: "Manager".into() }]
        );
    }

    #[test]
    fn a_budget_that_reports_turns_rather_than_calls_is_caught() {
        // The defect this exists for: one turn working through tool results
        // makes several calls, and a budget counting turns bills one.
        let (run, agent) = (RunId::new(), AgentId::new());
        let mut events = one_clean_turn(run, agent, MessageId::new());
        for _ in 0..2 {
            events.insert(
                6,
                UiEvent::TokensUsed {
                    agent_id: agent,
                    group_id: GroupId::new(),
                    run_id: run,
                    prompt: 200,
                    completion: 12,
                    cost: None,
                },
            );
        }

        let t = read(&events, run, &named(&[(agent, "Manager")]));
        assert_eq!(t.calls(), 3);
        assert_eq!(t.anomalies(), vec![Anomaly::BudgetMiscounted { steps: 1, calls: 3 }]);
    }

    #[test]
    fn a_call_that_failed_every_attempt_still_spent_its_step() {
        // The failed call reserved a step and reported no usage. Counting only
        // what the provider billed would call the honest number a defect.
        let (run, agent) = (RunId::new(), AgentId::new());
        let mut events = one_clean_turn(run, agent, MessageId::new());
        events.insert(
            8,
            appended(
                run,
                Participant::System,
                Participant::Agent { id: agent },
                vec![Part::Notice {
                    kind: NoticeKind::UpstreamError,
                    text: "Manager could not reply: the provider is unavailable".into(),
                }],
            ),
        );
        let last = events.len() - 1;
        events[last] = UiEvent::RunSettled { run_id: run, steps_used: 2 };

        let t = read(&events, run, &named(&[(agent, "Manager")]));
        assert_eq!(t.anomalies(), vec![Anomaly::CallFailed { agent: "Manager".into() }]);
    }

    #[test]
    fn a_parked_turn_nobody_answered_is_a_held_line() {
        let (run, agent) = (RunId::new(), AgentId::new());
        let approval = ApprovalId::new();
        let mut events = one_clean_turn(run, agent, MessageId::new());
        events.insert(2, UiEvent::ApprovalRequested { approval_id: approval, agent_id: agent });

        let t = read(&events, run, &named(&[(agent, "Manager")]));
        assert_eq!(t.anomalies(), vec![Anomaly::ParkedWithoutAnAnswer { agent: "Manager".into() }]);

        // Answered, and the same run is clean.
        let mut answered = events.clone();
        answered.insert(
            3,
            UiEvent::ApprovalSettled { approval_id: approval, state: ApprovalState::Allow },
        );
        let t = read(&answered, run, &named(&[(agent, "Manager")]));
        assert_eq!(t.anomalies(), vec![]);
    }

    #[test]
    fn a_turn_that_parked_and_resumed_is_still_one_turn() {
        // The runtime puts a parked agent back to Thinking when the operator
        // answers. Counting that as a second turn would report every approval
        // as an agent that had been woken twice.
        let (run, agent) = (RunId::new(), AgentId::new());
        let mut events = one_clean_turn(run, agent, MessageId::new());
        events.splice(
            2..2,
            [
                UiEvent::ActivityChanged { agent_id: agent, activity: Activity::AwaitingApproval },
                UiEvent::ActivityChanged { agent_id: agent, activity: Activity::Thinking },
            ],
        );

        let t = read(&events, run, &named(&[(agent, "Manager")]));
        assert_eq!(t.turns(agent), 1, "{}", t.ledger);
    }

    #[test]
    fn another_runs_traffic_is_not_read_as_this_ones() {
        // Two runs interleave in one event stream, and the second run's stream
        // deltas must not be read as text nobody announced.
        let (mine, theirs) = (RunId::new(), RunId::new());
        let (me, them) = (AgentId::new(), AgentId::new());
        let mut events = one_clean_turn(mine, me, MessageId::new());
        events.splice(4..4, one_clean_turn(theirs, them, MessageId::new()));

        let t = read(&events, mine, &named(&[(me, "Manager"), (them, "Chef")]));
        assert_eq!(t.anomalies(), vec![]);
        assert_eq!(t.calls(), 1, "the other run's call is not this run's spend");
        assert!(!t.ledger.contains("Chef"), "and its agents are not in this ledger:\n{}", t.ledger);
    }

    #[test]
    fn a_fan_out_that_really_ran_at_once_shows_it_in_the_interleaving() {
        let run = RunId::new();
        let manager = AgentId::new();
        let peers: Vec<AgentId> = (0..3).map(|_| AgentId::new()).collect();
        let mut events = one_clean_turn(run, manager, MessageId::new());
        let mut fanned = Vec::new();
        for peer in &peers {
            fanned.push(appended(
                run,
                Participant::Agent { id: manager },
                Participant::Agent { id: *peer },
                vec![Part::text("go")],
            ));
        }
        for peer in &peers {
            fanned.push(UiEvent::ActivityChanged { agent_id: *peer, activity: Activity::Thinking });
        }
        for peer in &peers {
            fanned.push(UiEvent::ActivityChanged { agent_id: *peer, activity: Activity::Idle });
        }
        events.splice(1..1, fanned);

        let t = read(&events, run, &named(&[(manager, "Manager")]));
        assert_eq!(t.peak_concurrency(), 3, "three open turns at one point:\n{}", t.ledger);
    }

    #[test]
    fn a_run_nothing_ever_settled_says_so() {
        let (run, agent) = (RunId::new(), AgentId::new());
        let mut events = one_clean_turn(run, agent, MessageId::new());
        events.retain(|e| !matches!(e, UiEvent::RunSettled { .. }));

        let t = read(&events, run, &named(&[(agent, "Manager")]));
        assert_eq!(t.anomalies(), vec![Anomaly::NeverSettled]);
    }

    #[test]
    fn settling_twice_is_counted_because_spend_is_read_from_it() {
        let (run, agent) = (RunId::new(), AgentId::new());
        let mut events = one_clean_turn(run, agent, MessageId::new());
        events.push(UiEvent::RunSettled { run_id: run, steps_used: 1 });

        let t = read(&events, run, &named(&[(agent, "Manager")]));
        assert_eq!(t.anomalies(), vec![Anomaly::SettledMoreThanOnce { times: 2 }]);
    }

    #[test]
    fn a_tool_that_broke_is_told_apart_from_one_the_guard_refused() {
        let (run, agent) = (RunId::new(), AgentId::new());
        let mut events = one_clean_turn(run, agent, MessageId::new());
        events.insert(
            7,
            appended(
                run,
                Participant::Agent { id: agent },
                Participant::Human,
                vec![
                    Part::tool_call(
                        "send_message",
                        serde_json::json!({}),
                        ToolOutcome::Refused { reason: "Refused: hop limit reached.".into() },
                    ),
                    Part::tool_call(
                        "browse",
                        serde_json::json!({}),
                        ToolOutcome::Failed { error: "the machine went away".into() },
                    ),
                ],
            ),
        );

        let t = read(&events, run, &named(&[(agent, "Manager")]));
        assert_eq!(
            t.anomalies(),
            vec![Anomaly::ToolFailed {
                agent: "Manager".into(),
                tool: "browse".into(),
                error: "the machine went away".into()
            }]
        );
        assert_eq!(t.refusals(), vec!["Refused: hop limit reached.".to_string()]);
        assert_eq!(t.tools(), vec!["send_message", "browse"]);
    }
}
