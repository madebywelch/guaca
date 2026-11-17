use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::domain::approval::ApprovalState;
use crate::domain::envelope::{Envelope, Part, Participant};
use crate::domain::ids::{AgentId, ApprovalId, EscalationId, GroupId, MessageId, RunId};
use crate::domain::outbox::{Kind as OutboxKind, Target};
use crate::domain::outcome::Outcome;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Activity {
    Idle,
    Thinking,
    Queued { depth: usize },
    AwaitingApproval,
    Paused,
}

impl Activity {
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

pub const CHANNEL: &str = "guac://event";

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum UiEvent {
    DecisionsChanged,
    DecisionReminder {
        count: usize,
    },
    AgentsChanged,

    OpenUrl {
        url: String,
        client: Option<String>,
    },

    MessageAppended {
        message: Box<Envelope>,
    },

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
    ReasoningDelta {
        message_id: MessageId,
        text: String,
    },
    ToolStarted {
        message_id: MessageId,
        call_id: String,
        name: String,
        arguments: serde_json::Value,
    },
    ToolFinished {
        message_id: MessageId,
        call_id: String,
        part: Part,
    },

    StreamEnded {
        message_id: MessageId,
        channel_id: AgentId,
    },

    ActivityChanged {
        agent_id: AgentId,
        activity: Activity,
    },

    ChannelsCleared {
        agents: Vec<AgentId>,
    },

    TokensUsed {
        agent_id: AgentId,
        group_id: GroupId,
        run_id: RunId,
        prompt: u32,
        completion: u32,
        cached: u32,
        cost: Option<f64>,
    },

    RunSettled {
        run_id: RunId,
        steps_used: u32,
    },
    OutcomeRecorded {
        outcome: Box<Outcome>,
    },
    OutcomesSeen {
        run_ids: Vec<RunId>,
    },

    Delivery {
        id: String,
        kind: OutboxKind,
        title: String,
        body: String,
        target: Target,
        created_at: i64,
    },

    ApprovalRequested {
        approval_id: ApprovalId,
        agent_id: AgentId,
    },
    ApprovalSettled {
        approval_id: ApprovalId,
        state: ApprovalState,
    },

    EscalationRaised {
        escalation_id: EscalationId,
        agent_id: AgentId,
    },
    EscalationCleared {
        escalation_id: EscalationId,
    },

    RoutinesChanged {
        agent_id: AgentId,
    },

    CalendarChanged {
        group_id: GroupId,
    },

    ArtifactsChanged {
        group_id: GroupId,
    },

    MemoryChanged {
        agent_id: AgentId,
    },
    WorkingNotesChanged {
        agent_id: AgentId,
    },

    SettingsChanged {
        settings: Box<crate::config::RedactedConfig>,
    },

    SkillsChanged {
        scope: crate::domain::skill::Scope,
    },

    NotebookChanged {
        agent_id: AgentId,
    },
}

pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: UiEvent);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, _event: UiEvent) {}
}

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
