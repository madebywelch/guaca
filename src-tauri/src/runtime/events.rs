//! Events pushed from the runtime to the UI.
//!
//! Behind a trait so the runtime can be tested without a webview. The Tauri
//! implementation lives in `app.rs`; tests use [`RecordingSink`], which is what
//! makes the cascade tests assertable rather than a staring contest with a log.

use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;

use crate::domain::approval::ApprovalState;
use crate::domain::envelope::{Envelope, Participant};
use crate::domain::ids::{AgentId, ApprovalId, GroupId, MessageId, RunId};

/// What an agent is doing right now, surfaced as the dot next to its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

/// The single event channel the frontend subscribes to.
pub const CHANNEL: &str = "guac://event";

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum UiEvent {
    /// The agent roster changed. The UI refetches rather than patching, because
    /// the roster is small and a diff protocol here would be pure ceremony.
    AgentsChanged,

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
    use crate::domain::envelope::{Intent, Part, Participant, Trust};

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
