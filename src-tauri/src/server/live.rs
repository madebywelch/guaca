//! The current render state, captured before a socket starts consuming events.
//! Only live text is held. Thinking is never replayed, and ended streams leave
//! immediately. The transcript remains the durable record.

use std::collections::HashMap;

use serde::Serialize;

use crate::domain::envelope::Participant;
use crate::domain::ids::{AgentId, MessageId, RepositoryId, RunId};
use crate::runtime::events::{Activity, UiEvent};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Snapshot {
    #[serde(rename = "type")]
    kind: &'static str,
    activity: HashMap<AgentId, Activity>,
    streams: HashMap<MessageId, Stream>,
    building: HashMap<AgentId, RepositoryId>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Stream {
    channel_id: AgentId,
    agent_id: AgentId,
    run_id: RunId,
    to: Participant,
    text: String,
    #[serde(skip)]
    full: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            kind: "liveSnapshot",
            activity: HashMap::new(),
            streams: HashMap::new(),
            building: HashMap::new(),
        }
    }
}

impl Snapshot {
    pub fn observe(&mut self, event: &UiEvent) {
        match event {
            UiEvent::ActivityChanged { agent_id, activity } => {
                self.activity.insert(*agent_id, *activity);
            }
            UiEvent::StreamStarted { message_id, channel_id, agent_id, run_id, to } => {
                self.streams.insert(
                    *message_id,
                    Stream {
                        channel_id: *channel_id,
                        agent_id: *agent_id,
                        run_id: *run_id,
                        to: *to,
                        text: String::new(),
                        full: false,
                    },
                );
            }
            UiEvent::StreamDelta { message_id, text, .. } => {
                if let Some(stream) = self.streams.get_mut(message_id) {
                    // A very large live reply can wait for its persisted form.
                    // The cache is bounded per active turn, not per socket.
                    if !stream.full && stream.text.len() + text.len() <= 512 * 1024 {
                        stream.text.push_str(text);
                    } else {
                        stream.full = true;
                    }
                }
            }
            UiEvent::StreamEnded { message_id, .. } => {
                self.streams.remove(message_id);
            }
            UiEvent::CodingJobStarted { agent_id, repository_id, .. } => {
                self.building.insert(*agent_id, *repository_id);
            }
            UiEvent::CodingJobFinished { agent_id, .. } => {
                self.building.remove(agent_id);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_snapshot_restores_live_text_and_forgets_a_finished_turn() {
        let mut live = Snapshot::default();
        let agent_id = AgentId::new();
        let message_id = MessageId::new();
        let run_id = RunId::new();
        live.observe(&UiEvent::StreamStarted {
            message_id,
            agent_id,
            channel_id: agent_id,
            run_id,
            to: Participant::Human,
        });
        live.observe(&UiEvent::StreamDelta {
            message_id,
            channel_id: agent_id,
            text: "hello".into(),
        });
        live.observe(&UiEvent::ReasoningDelta { message_id, text: "private thought".into() });
        let encoded = serde_json::to_string(&live).unwrap();
        assert!(encoded.contains("hello"));
        assert!(!encoded.contains("private thought"));
        live.observe(&UiEvent::StreamEnded { message_id, channel_id: agent_id });
        assert!(live.streams.is_empty());
    }
}
