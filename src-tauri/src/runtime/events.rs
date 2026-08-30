//! Events pushed from the runtime to the UI.
//!
//! Behind a trait so the runtime can be tested without a webview. The Tauri
//! implementation lives in `app.rs`; tests use [`RecordingSink`], which is what
//! makes the cascade tests assertable rather than a staring contest with a log.

use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::domain::approval::ApprovalState;
use crate::domain::envelope::{Envelope, Part, Participant};
use crate::domain::ids::{
    AgentId, ApprovalId, EscalationId, GroupId, MessageId, RepositoryId, RunId,
};

/// What an agent is doing right now, surfaced as the dot next to its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Activity {
    Idle,
    /// Mid-inference.
    Thinking,
    /// Has queued work it has not started. `depth` is the inbox backlog.
    Queued {
        depth: usize,
    },
    /// Parked mid-turn on a permission request. Its own state rather than
    /// `Thinking`, because the difference between a model that is working and
    /// one that is waiting on the operator is the difference between leaving it
    /// alone and going to answer it.
    AwaitingApproval,
    /// Not processing; messages accumulate.
    Paused,
}

impl Activity {
    /// Whether anything landing on this agent now would wait its turn.
    ///
    /// What a routine's `skip_if_working` is asking about. Deliberately read
    /// off the same value the dot beside the agent's name is drawn from, so
    /// "it says Thinking and the sweep was skipped" is one fact rather than
    /// two that agree on the day they were written.
    ///
    /// Idle is the only state where a delivery starts work immediately. A
    /// queue that is not empty is work already waiting, a request on the
    /// operator's desk is a turn parked mid-flight, and a paused agent takes
    /// nothing off its inbox at all: putting a firing behind any of those is
    /// the pile-up the option exists to avoid.
    pub fn is_working(self) -> bool {
        match self {
            Activity::Idle => false,
            Activity::Thinking
            | Activity::Queued { .. }
            | Activity::AwaitingApproval
            | Activity::Paused => true,
        }
    }
}

/// The single event channel the frontend subscribes to.
pub const CHANNEL: &str = "guac://event";

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum UiEvent {
    /// The agent roster changed. The UI refetches rather than patching, because
    /// the roster is small and a diff protocol here would be pure ceremony.
    AgentsChanged,

    /// A sign-in needs the operator's browser, and on this host the only one
    /// there is belongs to whoever is reading the page. A desktop opens the
    /// system browser itself and never emits this; a server has nobody at the
    /// machine, so the page is asked to open the URL and to show it as a link
    /// in case the browser refuses a window nobody clicked for.
    OpenUrl {
        url: String,
    },

    /// A complete message was persisted.
    MessageAppended {
        message: Box<Envelope>,
    },

    /// An agent began composing.
    ///
    /// `to` decides how the UI draws it. A reply to the operator becomes a
    /// bubble that fills in as text arrives; a reply to a peer becomes a quiet
    /// "writing to X" line, because the finished message is a collapsed wire
    /// row and streaming it into a bubble first means watching text appear and
    /// then vanish.
    StreamStarted {
        message_id: MessageId,
        channel_id: AgentId,
        agent_id: AgentId,
        run_id: RunId,
        to: Participant,
    },
    StreamDelta {
        message_id: MessageId,
        channel_id: AgentId,
        text: String,
    },
    /// Part of the model's own working, for as long as the turn lasts.
    ///
    /// Addressed to the placeholder rather than to a channel, and that is what
    /// makes it ephemeral for free: the UI files it under the agent that opened
    /// the stream and drops the lot when the stream ends, so a thought cannot
    /// outlive the turn that had it. Nothing here is persisted, and no channel
    /// id is carried because a thought is not filed anywhere: a turn writing to
    /// a peer streams into that peer's channel, while the operator watching
    /// this agent work is reading its own.
    ReasoningDelta {
        message_id: MessageId,
        text: String,
    },
    /// A tool call this turn has started, before anything is known about how
    /// it went.
    ///
    /// Addressed to the placeholder for the same reason `ReasoningDelta` is,
    /// and it buys the same thing: the webview files it under the agent that
    /// opened the stream and drops the lot when the stream ends, so what a turn
    /// was watched reaching for cannot outlive the turn. The record is the
    /// message that follows, which carries every one of these as a
    /// `Part::ToolCall`; this is only what that record looks like while it is
    /// still being made.
    ///
    /// Emitted before the call, not after it. A command can sit for a minute
    /// and a page load for several seconds, and the operator watching cannot
    /// otherwise tell a turn waiting on a machine from one that has stopped.
    ///
    /// Nothing here is new to the webview. The name and the arguments are the
    /// same bytes the transcript draws once the turn ends; they are the model's
    /// own words, and a credential's value is in neither.
    ToolStarted {
        message_id: MessageId,
        /// The provider's id for the call, which is what pairs this with the
        /// finish below. Two identical calls in one turn are two calls.
        call_id: String,
        name: String,
        arguments: serde_json::Value,
    },
    /// The same call, finished, as the record of it that the message will
    /// carry.
    ///
    /// The whole `Part::ToolCall` rather than the outcome alone, and that is
    /// what makes the chip drawn while a turn runs and the chip drawn
    /// afterward the same chip rather than two that agree today. A memory
    /// rewrite carries what it overwrote and nothing outside the runtime could
    /// supply it; the next thing a call has to say for itself will be the same,
    /// and an event listing the fields it happened to need would have to be
    /// remembered at that point.
    ToolFinished {
        message_id: MessageId,
        call_id: String,
        part: Part,
    },

    /// The placeholder is replaced by the persisted message that follows.
    StreamEnded {
        message_id: MessageId,
        channel_id: AgentId,
    },

    ActivityChanged {
        agent_id: AgentId,
        activity: Activity,
    },

    /// These channels were emptied. Anything holding their messages should
    /// drop them and read again.
    ChannelsCleared {
        agents: Vec<AgentId>,
    },

    /// A model call finished and reported what it cost.
    ///
    /// Emitted per call rather than per turn, so the number moves while an
    /// agent is still working rather than once it has finished.
    TokensUsed {
        agent_id: AgentId,
        group_id: GroupId,
        run_id: RunId,
        prompt: u32,
        completion: u32,
        /// Absent when the provider does not price calls, which is not the
        /// same as free and must not be added up as zero.
        cost: Option<f64>,
    },

    /// Every agent in a run has gone quiet.
    RunSettled {
        run_id: RunId,
        steps_used: u32,
    },

    /// An agent is waiting on the operator.
    ///
    /// The request itself travels in the transcript, as a part of the message
    /// that carries it. Only the id is here, because what the UI is missing at
    /// this point is not the wording but the fact that it is still live.
    ApprovalRequested {
        approval_id: ApprovalId,
        agent_id: AgentId,
    },
    /// Answered, timed out, or abandoned by a restart.
    ApprovalSettled {
        approval_id: ApprovalId,
        state: ApprovalState,
    },

    /// An agent has put something on the operator's desk and carried on.
    ///
    /// The id and the agent, for the reason `ApprovalRequested` carries those
    /// two and nothing else: what the UI is missing is not the wording, which
    /// it reads back whole, but that the desk it is drawing is now behind the
    /// store.
    EscalationRaised {
        escalation_id: EscalationId,
        agent_id: AgentId,
    },
    /// The operator has dealt with one, or it went with the agent that raised
    /// it. Not a settlement: nothing was answered and nothing was granted.
    EscalationCleared {
        escalation_id: EscalationId,
    },

    /// A coding job started, and the repository it is working in is now busy.
    ///
    /// The rail draws this because nothing else would. `code` returns as soon
    /// as the harness is up and the turn ends, so an agent goes idle while a
    /// coding agent works in its repository for twenty minutes: the crew looks
    /// stopped at exactly the moment it is building. The job outlives the turn,
    /// so it cannot be an [`Activity`], which is cleared when a turn ends.
    CodingJobStarted {
        agent_id: AgentId,
        repository_id: RepositoryId,
        repository: String,
    },
    /// One line of what a running coding job is doing.
    ///
    /// Ephemeral by construction, exactly as a turn's thinking is: addressed to
    /// the job rather than filed anywhere, held by the webview while the job
    /// runs and dropped when it ends. The record of what a job did is the
    /// message it delivers at the end; this is only what that looks like before
    /// it exists.
    ///
    /// Filtered on this side rather than forwarded raw. `pi`'s stream is tens
    /// of thousands of lines of deltas and cumulative usage, and putting that
    /// on the event channel would be a re-render per token for a panel nobody
    /// could read.
    CodingProgress {
        agent_id: AgentId,
        repository_id: RepositoryId,
        /// What it is doing: a tool name, or empty when it is talking.
        tool: String,
        /// The command, the path, or the sentence.
        detail: String,
    },
    /// That job ended, however it ended. The repository is free again.
    CodingJobFinished {
        agent_id: AgentId,
        repository_id: RepositoryId,
    },

    /// A coding job could not run, for a reason only the operator can fix.
    ///
    /// The agent is told too, in its own channel, and that is not enough. An
    /// expired credential on the operator's own machine is the operator's
    /// problem, and a sentence about it inside one agent's transcript is a
    /// sentence nobody reads: it cost a whole afternoon of silent no-ops, with
    /// every agent in the workspace dutifully reporting that nothing needed
    /// doing.
    ///
    /// Only for failures of the harness itself — it would not start, or it
    /// ended its own turn on an error. A job that ran and did the wrong thing
    /// is the agent's to report and belongs nowhere near a banner.
    CodingJobFailed {
        agent_id: AgentId,
        /// Named, because an operator with several repositories needs to know
        /// which one stopped.
        repository: String,
        /// And which program stopped, because the way out of the commonest
        /// failure here is the other one. A spent plan is not a thing the
        /// operator can fix from inside this app, and a banner that says a
        /// coding job failed without saying what was running it leaves them
        /// guessing which sign-in to go and look at.
        harness: String,
        /// The harness's own words. Guaca has no better description of a
        /// failure it did not cause and cannot interpret.
        reason: String,
    },

    /// One agent's schedule changed: it set a routine, edited one, canceled
    /// one, or one came due and moved to its next slot.
    ///
    /// Not `AgentsChanged`, which is what this used to borrow. The roster did
    /// not change, and the panel drawing the schedule was not listening for it
    /// anyway: an agent that scheduled something for itself left the operator
    /// reading a list that was drawn before the routine existed, which they
    /// could only fix by closing the panel and opening it again.
    RoutinesChanged {
        agent_id: AgentId,
    },

    /// Something moved on one crew's calendar.
    ///
    /// The crew rather than the agent, because the calendar is the crew's: an
    /// agent's write is read by every other agent in it and drawn on a surface
    /// that is showing every crew at once. Same argument `RoutinesChanged`
    /// makes about a panel drawn before the routine existed, one scope up.
    ///
    /// The operator's own edits do not come through here. Those are a command
    /// that hands the row straight back, and the view that made the call is the
    /// view that already has it.
    CalendarChanged {
        group_id: GroupId,
    },

    /// One agent's memory was rewritten by the agent itself.
    ///
    /// Same argument as `RoutinesChanged`, one panel over: the operator reads
    /// an agent's memory in the column beside it while the agent is working,
    /// and the agent rewrites that file mid-turn. Without this the page on
    /// screen is the one that was true when the panel was drawn, and the only
    /// way to find out otherwise is to click away and back.
    ///
    /// Only the runtime's own write emits it. The operator's edit comes back
    /// from `set_agent_memory` as what was actually stored, so the panel that
    /// made it already has the answer, and an event there would be a refetch
    /// to learn what the reply just said.
    MemoryChanged {
        agent_id: AgentId,
    },
    /// One agent appended a working note.
    ///
    /// Its own event rather than a second meaning for `MemoryChanged`, because
    /// the two panels refetch different things and folding them together would
    /// have every note an agent writes re-read a memory that has not moved.
    /// Notes are written far more often than memory, which is the whole design,
    /// so the cheap one must not drag the expensive one behind it.
    WorkingNotesChanged {
        agent_id: AgentId,
    },
}

pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: UiEvent);
}

/// Discards everything. For tests that do not assert on events.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, _event: UiEvent) {}
}

/// Keeps every event for inspection.
#[derive(Debug, Default)]
pub struct RecordingSink {
    events: Mutex<Vec<UiEvent>>,
}

impl RecordingSink {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn snapshot(&self) -> Vec<UiEvent> {
        self.events.lock().clone()
    }

    /// Concatenated stream text for one message, in arrival order.
    pub fn streamed_text(&self, message_id: MessageId) -> String {
        self.events
            .lock()
            .iter()
            .filter_map(|e| match e {
                UiEvent::StreamDelta { message_id: id, text, .. } if *id == message_id => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect()
    }

    /// The same for the reasoning that ran alongside it.
    pub fn streamed_reasoning(&self, message_id: MessageId) -> String {
        self.events
            .lock()
            .iter()
            .filter_map(|e| match e {
                UiEvent::ReasoningDelta { message_id: id, text } if *id == message_id => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect()
    }

    pub fn appended_messages(&self) -> Vec<Envelope> {
        self.events
            .lock()
            .iter()
            .filter_map(|e| match e {
                UiEvent::MessageAppended { message } => Some((**message).clone()),
                _ => None,
            })
            .collect()
    }

    pub fn count_of(&self, predicate: impl Fn(&UiEvent) -> bool) -> usize {
        self.events.lock().iter().filter(|e| predicate(e)).count()
    }
}

impl EventSink for RecordingSink {
    fn emit(&self, event: UiEvent) {
        self.events.lock().push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::envelope::{Intent, Participant, ToolOutcome, Trust};

    fn envelope(text: &str) -> Envelope {
        Envelope {
            id: MessageId::new(),
            run_id: RunId::new(),
            channel_id: AgentId::new(),
            from: Participant::Human,
            to: Participant::Agent { id: AgentId::new() },
            parts: vec![Part::text(text)],
            trust: Trust::Operator,
            hop: 0,
            expects_reply: true,
            intent: Intent::Courtesy,
            cause: None,
            created_at: 0,
        }
    }

    #[test]
    fn idle_is_the_only_state_where_work_starts_at_once() {
        // What a routine's `skip_if_working` is asking about, so every state
        // has to be answered for deliberately: a firing put behind a queue, a
        // permission request or a pause is the pile-up the option exists to
        // prevent, and only one of the five is the agent standing free.
        assert!(!Activity::Idle.is_working());
        assert!(Activity::Thinking.is_working());
        assert!(Activity::Queued { depth: 1 }.is_working());
        assert!(Activity::AwaitingApproval.is_working());
        assert!(Activity::Paused.is_working());
    }

    #[test]
    fn recording_sink_reassembles_stream_text_in_order() {
        let sink = RecordingSink::new();
        let id = MessageId::new();
        let other = MessageId::new();
        let channel = AgentId::new();

        for text in ["Hel", "lo, ", "world"] {
            sink.emit(UiEvent::StreamDelta {
                message_id: id,
                channel_id: channel,
                text: text.into(),
            });
        }
        // Interleaved traffic from a different agent must not bleed in.
        sink.emit(UiEvent::StreamDelta {
            message_id: other,
            channel_id: channel,
            text: "NOISE".into(),
        });

        assert_eq!(sink.streamed_text(id), "Hello, world");
        assert_eq!(sink.streamed_text(other), "NOISE");
    }

    #[test]
    fn a_thought_and_the_text_beside_it_are_kept_apart() {
        // They share a placeholder and arrive interleaved. A sink that mixed
        // them would put the model's working into the assertion that says what
        // the operator watched appear.
        let sink = RecordingSink::new();
        let id = MessageId::new();
        sink.emit(UiEvent::ReasoningDelta { message_id: id, text: "weighing it up".into() });
        sink.emit(UiEvent::StreamDelta {
            message_id: id,
            channel_id: AgentId::new(),
            text: "Yes.".into(),
        });

        assert_eq!(sink.streamed_text(id), "Yes.");
        assert_eq!(sink.streamed_reasoning(id), "weighing it up");
    }

    #[test]
    fn recording_sink_collects_appended_messages() {
        let sink = RecordingSink::new();
        sink.emit(UiEvent::MessageAppended { message: Box::new(envelope("one")) });
        sink.emit(UiEvent::AgentsChanged);
        sink.emit(UiEvent::MessageAppended { message: Box::new(envelope("two")) });

        let messages = sink.appended_messages();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].plain_text(), "one");
        assert_eq!(messages[1].plain_text(), "two");
    }

    #[test]
    fn events_serialize_with_a_discriminant_the_frontend_can_switch_on() {
        let json = serde_json::to_value(UiEvent::ActivityChanged {
            agent_id: AgentId::new(),
            activity: Activity::Queued { depth: 3 },
        })
        .unwrap();
        assert_eq!(json["type"], "activityChanged");
        assert_eq!(json["activity"]["state"], "queued");
        assert_eq!(json["activity"]["depth"], 3);
        assert!(json["agentId"].is_string(), "keys must reach the frontend camelCased");
    }

    #[test]
    fn stream_events_carry_the_channel_so_the_ui_can_route_without_a_lookup() {
        let json = serde_json::to_value(UiEvent::StreamStarted {
            message_id: MessageId::new(),
            channel_id: AgentId::new(),
            agent_id: AgentId::new(),
            run_id: RunId::new(),
            to: Participant::Human,
        })
        .unwrap();
        assert!(json["channelId"].is_string());
        assert!(json["messageId"].is_string());
    }

    #[test]
    fn a_live_tool_call_arrives_in_the_shape_the_chip_is_drawn_from() {
        // The live chip and the recorded one are built by the same rules in
        // `lib/trail.ts`, which reads a name, its arguments and an outcome. A
        // key that arrived under another spelling would draw as an unknown tool.
        let started = serde_json::to_value(UiEvent::ToolStarted {
            message_id: MessageId::new(),
            call_id: "call_1".into(),
            name: "run_command".into(),
            arguments: serde_json::json!({ "command": "ls" }),
        })
        .unwrap();
        assert_eq!(started["type"], "toolStarted");
        assert_eq!(started["callId"], "call_1");
        assert_eq!(started["name"], "run_command");
        assert_eq!(started["arguments"]["command"], "ls");
        assert!(started["messageId"].is_string());
        assert!(
            started["channelId"].is_null(),
            "a call is not filed anywhere, exactly like a thought"
        );

        let finished = serde_json::to_value(UiEvent::ToolFinished {
            message_id: MessageId::new(),
            call_id: "call_1".into(),
            part: Part::tool_call(
                "update_memory",
                serde_json::json!({ "content": "now" }),
                ToolOutcome::Failed { error: "no machine".into() },
            ),
        })
        .unwrap();
        assert_eq!(finished["type"], "toolFinished");
        assert_eq!(finished["part"]["type"], "toolCall");
        assert_eq!(finished["part"]["outcome"]["status"], "failed");
        assert_eq!(finished["part"]["outcome"]["error"], "no machine");
    }

    #[test]
    fn a_stream_says_who_it_is_addressed_to() {
        // Without this the UI streams a peer message into a bubble and then
        // replaces it with a collapsed row, which reads as text disappearing.
        let json = serde_json::to_value(UiEvent::StreamStarted {
            message_id: MessageId::new(),
            channel_id: AgentId::new(),
            agent_id: AgentId::new(),
            run_id: RunId::new(),
            to: Participant::Agent { id: AgentId::new() },
        })
        .unwrap();
        assert_eq!(json["to"]["kind"], "agent");
    }
}
