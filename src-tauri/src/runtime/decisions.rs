use super::guard::RunState;
use super::{AgentCard, Runs, Runtime, RuntimeError, UiEvent};
use crate::domain::decision::{self, DecisionAction, DecisionStatus, WorkDecision};
use crate::domain::envelope::{Envelope, Part, Participant, ToolOutcome};
use crate::domain::ids::{DecisionId, RunId};
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
        let mut runs = self.inner.runs.lock();
        let (decision, envelope) =
            self.inner.store.answer_decision(id, answer, resume, expected_updated_at, now_ms())?;
        if runs.draining {
            // Nothing on this host starts another turn, so the answer is the
            // next host's from the start, as `deliver` treats a message. Put
            // down here rather than queued, because the actor it would wait
            // behind may still be inside a tool call when the process ends.
            //
            // A second commit, after the one that recorded the answer: the
            // decision's own rows are written elsewhere. A process that ends
            // between the two reports the follow-through interrupted, and For
            // you offers to resume it.
            if let Err(err) = self.put_down_answer(&mut runs, &envelope) {
                tracing::error!(%err, decision = %id, "could not put a decision answer down; the next host reports it interrupted");
            }
            drop(runs);
            self.emit(UiEvent::MessageAppended { message: Box::new(envelope) });
        } else {
            self.book(&mut runs, envelope.run_id);
            drop(runs);
            self.enqueue_delivery(envelope);
        }
        self.emit(UiEvent::DecisionsChanged);
        tracing::info!(decision = %id, resume, "decision answer accepted and queued");
        Ok(decision)
    }

    fn put_down_answer(&self, runs: &mut Runs, envelope: &Envelope) -> Result<(), RuntimeError> {
        let Participant::Agent { id } = envelope.to else { return Ok(()) };
        let card = self.inner.store.get_agent(id)?.ok_or(RuntimeError::UnknownAgent(id))?;
        let state = RunState::new(self.limits_for(&card));
        self.inner.store.put_down(
            &[(envelope.id, &[])],
            &[(envelope.run_id, state.steps_used(), state.snapshot())],
        )?;
        self.hand_on(runs, envelope.run_id, state);
        Ok(())
    }

    pub fn snooze_decision(&self, id: DecisionId, until: i64) -> Result<(), RuntimeError> {
        self.inner.store.snooze_decision(id, until, now_ms())?;
        self.emit(UiEvent::DecisionsChanged);
        Ok(())
    }

    /// Claims the reminders that are due. The claim writes the outbox row
    /// that delivers them, so a batch nobody is connected for waits there
    /// rather than being spent. `DecisionReminder` is still said, for the
    /// windows that raise their own banner from it until they claim
    /// deliveries instead.
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

    /// The `decision` tool. `run` is the run whose turn called it, written on
    /// a decision it files so that run's outcome can say it left one open.
    pub(super) fn use_decision(
        &self,
        card: &AgentCard,
        run: RunId,
        action: DecisionAction,
        arguments: serde_json::Value,
    ) -> (String, Part) {
        let result = (|| -> Result<String, crate::db::StoreError> {
            let result = match action {
                DecisionAction::Request { topic, request, due_at } => {
                    let due = decision::deadline(due_at.as_deref())
                        .map_err(crate::db::StoreError::Decision)?;
                    let row = self.inner.store.file_decision(
                        card.id,
                        card.group_id,
                        &topic,
                        request,
                        due,
                        Some(run),
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
