use super::{AgentCard, Runtime, RuntimeError, UiEvent};
use crate::domain::decision::{self, DecisionAction, DecisionStatus, WorkDecision};
use crate::domain::envelope::{Part, ToolOutcome};
use crate::domain::ids::DecisionId;
use crate::domain::now_ms;
use crate::llm::tools;

impl Runtime {
    pub fn answer_decision(
        &self,
        id: DecisionId,
        answer: &str,
        resume: bool,
        expected_updated_at: Option<i64>,
    ) -> Result<WorkDecision, RuntimeError> {
        let (decision, envelope) = {
            let mut runs = self.inner.runs.lock();
            let (decision, envelope) = self.inner.store.answer_decision(
                id,
                answer,
                resume,
                expected_updated_at,
                now_ms(),
            )?;
            *runs.outstanding.entry(envelope.run_id).or_insert(0) += 1;
            (decision, envelope)
        };
        self.enqueue_delivery(envelope);
        self.emit(UiEvent::DecisionsChanged);
        tracing::info!(decision = %id, resume, "decision answer accepted and queued");
        Ok(decision)
    }

    pub fn snooze_decision(&self, id: DecisionId, until: i64) -> Result<(), RuntimeError> {
        self.inner.store.snooze_decision(id, until, now_ms())?;
        self.emit(UiEvent::DecisionsChanged);
        Ok(())
    }

    pub fn sweep_decisions(&self, now: i64) {
        match self.inner.store.decision_reminders(now) {
            Ok(0) => {}
            Ok(count) => {
                self.emit(UiEvent::DecisionsChanged);
                self.emit(UiEvent::DecisionReminder { count });
            }
            Err(err) => {
                tracing::warn!(%err, "could not read decision reminders; retrying next tick")
            }
        }
    }

    pub(super) fn use_decision(
        &self,
        card: &AgentCard,
        action: DecisionAction,
        arguments: serde_json::Value,
    ) -> (String, Part) {
        let result = (|| -> Result<String, crate::db::StoreError> {
            let result = match action {
                DecisionAction::Request { topic, request, due_at } => {
                    let due = decision::deadline(due_at.as_deref())
                        .map_err(crate::db::StoreError::Decision)?;
                    let row = self.inner.store.request_decision(
                        card.id,
                        card.group_id,
                        &topic,
                        request,
                        due,
                        now_ms(),
                    )?;
                    serde_json::to_string(&row)
                }
                DecisionAction::List => {
                    serde_json::to_string(&self.inner.store.decisions(Some(card.id))?)
                }
                DecisionAction::Complete { id, outcome } => {
                    serde_json::to_string(&self.inner.store.finish_decision(
                        id,
                        card.id,
                        DecisionStatus::Completed,
                        &outcome,
                        now_ms(),
                    )?)
                }
                DecisionAction::Withdraw { id, outcome } => {
                    serde_json::to_string(&self.inner.store.finish_decision(
                        id,
                        card.id,
                        DecisionStatus::Withdrawn,
                        &outcome,
                        now_ms(),
                    )?)
                }
            };
            result.map_err(|err| crate::db::StoreError::Decision(err.to_string()))
        })();
        match result {
            Ok(body) => {
                self.emit(UiEvent::DecisionsChanged);
                (format!("Recorded in For you. No turn is waiting. Continue independent work; do not assume an unanswered choice. The stored state is authoritative, including an answer already received.\n{body}"), Part::tool_call(tools::DECISION, arguments, ToolOutcome::Ok { summary: "decision recorded".into() }))
            }
            Err(err) => (
                format!("The decision was not changed: {err}"),
                Part::tool_call(
                    tools::DECISION,
                    arguments,
                    ToolOutcome::Failed { error: err.to_string() },
                ),
            ),
        }
    }
}
