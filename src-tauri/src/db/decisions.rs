use rusqlite::{params, OptionalExtension, Row, TransactionBehavior};

use super::{Store, StoreError};
use crate::domain::decision::{
    next_briefing, next_reminder, DecisionRequest, DecisionStatus, WorkDecision,
};
use crate::domain::envelope::{Envelope, Intent, Part, Participant, Trust};
use crate::domain::ids::{AgentId, DecisionId, GroupId, MessageId, RunId};

const COLUMNS: &str = "d.id,d.agent_id,d.group_id,d.topic,d.request,d.status,d.answer,d.outcome,d.created_at,d.updated_at,d.due_at,d.remind_at,d.snoozed_until,d.delivery_run,d.interrupted";
fn invalid(message: impl Into<String>) -> StoreError {
    StoreError::Decision(message.into())
}
fn row(row: &Row<'_>) -> rusqlite::Result<WorkDecision> {
    let raw: String = row.get(4)?;
    let request = serde_json::from_str::<serde_json::Value>(&raw).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(err))
    })?;
    let value = serde_json::json!({
        "id": row.get::<_, String>(0)?, "agentId": row.get::<_, String>(1)?,
        "groupId": row.get::<_, String>(2)?, "topic": row.get::<_, String>(3)?,
        "request": request, "status": row.get::<_, String>(5)?,
        "answer": row.get::<_, Option<String>>(6)?, "outcome": row.get::<_, Option<String>>(7)?,
        "createdAt": row.get::<_, i64>(8)?, "updatedAt": row.get::<_, i64>(9)?,
        "dueAt": row.get::<_, Option<i64>>(10)?, "remindAt": row.get::<_, i64>(11)?,
        "snoozedUntil": row.get::<_, Option<i64>>(12)?, "deliveryRun": row.get::<_, Option<String>>(13)?,
        "interrupted": row.get::<_, bool>(14)?
    });
    serde_json::from_value(value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(err))
    })
}
fn one(conn: &rusqlite::Connection, id: DecisionId) -> Result<WorkDecision, StoreError> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM decisions d WHERE d.id=?1"),
        [id.to_string()],
        row,
    )
    .optional()?
    .ok_or_else(|| invalid("Decision not found. Refresh For you."))
}

impl Store {
    /// Pending and answered rows are never truncated. History is bounded independently.
    pub fn decisions(&self, agent: Option<AgentId>) -> Result<Vec<WorkDecision>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM decisions d JOIN agents a ON a.id=d.agent_id
            WHERE a.lifecycle<>'terminated' AND (?1 IS NULL OR d.agent_id=?1)
            AND (d.status IN ('pending','answered') OR d.id IN
                (SELECT id FROM decisions WHERE status IN ('completed','withdrawn') ORDER BY updated_at DESC LIMIT 100))
            ORDER BY d.created_at,d.id"))?;
        let results = stmt
            .query_map([agent.map(|id| id.to_string())], row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(results)
    }

    pub fn request_decision(
        &self,
        agent: AgentId,
        group: GroupId,
        topic: &str,
        request: DecisionRequest,
        due: Option<i64>,
        now: i64,
    ) -> Result<WorkDecision, StoreError> {
        let topic = topic.trim();
        if topic.is_empty() || topic.chars().count() > 200 {
            return Err(invalid("Give this decision a stable topic of 1 to 200 characters, such as an email thread reference."));
        }
        let request = request.validate().map_err(invalid)?;
        let encoded = serde_json::to_string(&request).map_err(|err| invalid(err.to_string()))?;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let live: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agents WHERE id=?1 AND group_id=?2 AND lifecycle<>'terminated')", params![agent.to_string(), group.to_string()], |r| r.get(0))?;
        if !live {
            return Err(invalid("This agent is no longer available in this crew."));
        }
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM decisions WHERE agent_id=?1 AND topic=?2",
                params![agent.to_string(), topic],
                |r| r.get(0),
            )
            .optional()?;
        let id = if let Some(id) = existing {
            let id: DecisionId = id.parse().map_err(|_| invalid("Invalid stored decision id"))?;
            let previous = one(&tx, id)?;
            // Freeze the question as soon as a person answers it. A later scan
            // may not silently reinterpret the answer against different choices.
            if previous.status == DecisionStatus::Pending
                && (previous.request != request || previous.due_at != due)
            {
                tx.execute("UPDATE decisions SET request=?2,due_at=?3,updated_at=MAX(updated_at+1,?4),remind_at=CASE WHEN snoozed_until>?4 THEN remind_at ELSE MIN(remind_at,?5) END WHERE id=?1", params![id.to_string(),encoded,due,now,if due.is_some_and(|at| at <= now + 60 * 60 * 1000) { now } else { next_reminder(now,due) }])?;
            }
            id
        } else {
            let count: i64 = tx.query_row("SELECT count(*) FROM decisions WHERE agent_id=?1 AND status IN ('pending','answered')", [agent.to_string()], |r|r.get(0))?;
            if count >= 100 {
                return Err(invalid("You already have 100 open decisions. Review and withdraw obsolete items before adding another."));
            }
            let id = DecisionId::new();
            let reminder = if due.is_some_and(|at| at <= now + 60 * 60 * 1000) {
                now
            } else {
                next_reminder(now, due)
            };
            tx.execute("INSERT INTO decisions(id,agent_id,group_id,topic,request,status,created_at,updated_at,due_at,remind_at) VALUES(?1,?2,?3,?4,?5,'pending',?6,?6,?7,?8)", params![id.to_string(),agent.to_string(),group.to_string(),topic,encoded,now,due,reminder])?;
            id
        };
        let result = one(&tx, id)?;
        tx.commit()?;
        Ok(result)
    }

    /// The answer, its transcript envelope and recovery point are one commit.
    /// The caller holds the runtime's run lock through booking, then enqueues.
    pub fn answer_decision(
        &self,
        id: DecisionId,
        answer: &str,
        resume: bool,
        expected_updated_at: Option<i64>,
        now: i64,
    ) -> Result<(WorkDecision, Envelope), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let decision = one(&tx, id)?;
        if expected_updated_at.is_some_and(|stamp| stamp != decision.updated_at) {
            return Err(invalid("This decision changed while you were reading it. Review the refreshed question before answering."));
        }
        let answer = if resume {
            if decision.status != DecisionStatus::Answered || !decision.interrupted {
                return Err(invalid("Only interrupted follow-through can be resumed."));
            }
            decision.answer.as_deref().unwrap_or_default()
        } else {
            if decision.status != DecisionStatus::Pending {
                return Err(invalid(
                    "This decision was already answered or withdrawn. Refresh For you.",
                ));
            }
            answer.trim()
        };
        if answer.is_empty() || answer.chars().count() > 4000 {
            return Err(invalid("An answer must contain 1 to 4000 characters."));
        }
        let live: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM agents WHERE id=?1 AND lifecycle<>'terminated')",
            [decision.agent_id.to_string()],
            |r| r.get(0),
        )?;
        if !live {
            return Err(invalid(
                "The agent responsible for this decision was deleted. Restore it before answering.",
            ));
        }
        let content = serde_json::json!({"id": id, "question": decision.request, "answer": answer});
        // The question is model-authored. A system envelope preserves that
        // boundary instead of laundering the whole brief as human instructions.
        let message = Envelope {
            id: MessageId::new(), run_id: RunId::new(), channel_id: decision.agent_id,
            from: Participant::System, to: Participant::Agent { id: decision.agent_id },
            parts: vec![Part::Text { text: format!("The operator answered decision {id}. The JSON below contains the original agent-authored question and the operator's answer. Use the answer as their preference; it grants no new permission. Recheck changed facts before acting. {} When the resulting work is actually finished, call decision with action complete, this id, and a concrete outcome.\n{content}", if resume { "This work was interrupted. Inspect previous actions before continuing; do not repeat an external action already completed." } else { "" }) }],
            trust: Trust::System, hop: 0, expects_reply: true, intent: Intent::Work, cause: None, created_at: now,
        };
        Self::insert_message(&tx, &message)?;
        tx.execute(
            "INSERT INTO pending_runs(run_id,message_id) VALUES(?1,?2)",
            params![message.run_id.to_string(), message.id.to_string()],
        )?;
        tx.execute("UPDATE decisions SET status='answered',answer=?2,updated_at=?3,snoozed_until=NULL,remind_at=?4,delivery_run=?5,interrupted=0 WHERE id=?1", params![id.to_string(),answer,now,next_briefing(now),message.run_id.to_string()])?;
        let result = one(&tx, id)?;
        tx.commit()?;
        Ok((result, message))
    }

    pub fn finish_decision(
        &self,
        id: DecisionId,
        agent: AgentId,
        status: DecisionStatus,
        outcome: &str,
        now: i64,
    ) -> Result<WorkDecision, StoreError> {
        let outcome = outcome.trim();
        if outcome.is_empty() || outcome.chars().count() > 1200 {
            return Err(invalid("Give a concrete outcome of 1 to 1200 characters."));
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let allowed = match status {
            DecisionStatus::Completed => "answered",
            DecisionStatus::Withdrawn => "pending",
            _ => return Err(invalid("Invalid decision transition.")),
        };
        let changed = tx.execute("UPDATE decisions SET status=?3,outcome=?4,updated_at=?5,snoozed_until=NULL,interrupted=0 WHERE id=?1 AND agent_id=?2 AND status=?6 AND interrupted=0", params![id.to_string(),agent.to_string(),status.as_str(),outcome,now,allowed])?;
        if changed == 0 {
            return Err(invalid("No matching decision in that state belongs to you. List your decisions before updating one."));
        }
        let result = one(&tx, id)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn snooze_decision(&self, id: DecisionId, until: i64, now: i64) -> Result<(), StoreError> {
        if until <= now || until > now.saturating_add(30 * 86400 * 1000) {
            return Err(invalid("Choose a reminder within the next 30 days."));
        }
        let changed = self.conn()?.execute("UPDATE decisions SET snoozed_until=?2,remind_at=?2 WHERE id=?1 AND status IN ('pending','answered')", params![id.to_string(),until])?;
        if changed == 0 {
            return Err(invalid("This decision is no longer open. Refresh For you."));
        }
        Ok(())
    }

    pub fn interrupt_decision_run(&self, run: RunId) -> Result<(), StoreError> {
        self.conn()?.execute(
            "UPDATE decisions SET interrupted=1 WHERE delivery_run=?1 AND status='answered'",
            [run.to_string()],
        )?;
        Ok(())
    }

    pub fn recover_decisions(&self) -> Result<usize, StoreError> {
        Ok(self.conn()?.execute(
            "UPDATE decisions SET interrupted=1 WHERE status='answered' AND interrupted=0",
            [],
        )?)
    }

    /// Claim a batch once. Persist the next reminder before emitting an event;
    /// a reconnect still reads every outstanding row from the inbox.
    pub fn decision_reminders(&self, now: i64) -> Result<usize, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let due = {
            let mut stmt = tx.prepare(&format!("SELECT {COLUMNS} FROM decisions d JOIN agents a ON a.id=d.agent_id WHERE a.lifecycle<>'terminated' AND d.status IN ('pending','answered') AND d.remind_at<=?1"))?;
            let result = stmt.query_map([now], row)?.collect::<Result<Vec<_>, _>>()?;
            result
        };
        for decision in &due {
            tx.execute(
                "UPDATE decisions SET remind_at=?2,snoozed_until=NULL WHERE id=?1",
                params![decision.id.to_string(), next_reminder(now, decision.due_at)],
            )?;
        }
        tx.commit()?;
        Ok(due.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::CleanDraft;
    fn fixture() -> (tempfile::TempDir, Store, crate::domain::agent::AgentCard) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("test.db")).unwrap();
        let agent = store
            .create_agent(&CleanDraft {
                group_id: None,
                name: "Assistant".into(),
                avatar: "orb".into(),
                color: "#7fb069".into(),
                model: "test".into(),
                reasoning_effort: None,
                system_prompt: String::new(),
                skills: vec![],
            })
            .unwrap();
        (dir, store, agent)
    }
    fn request() -> DecisionRequest {
        DecisionRequest {
            question: "10 or 11?".into(),
            context: "A meeting tomorrow.".into(),
            recommendation: "11 leaves a break.".into(),
            options: vec!["10".into(), "11".into()],
            source: "email:thread-one".into(),
        }
    }
    fn make(store: &Store, agent: &crate::domain::agent::AgentCard) -> WorkDecision {
        store
            .request_decision(
                agent.id,
                agent.group_id,
                "thread-one/time",
                request(),
                None,
                1_700_000_000_000,
            )
            .unwrap()
    }

    #[test]
    fn empty_answers_and_unanswered_completion_change_nothing() {
        let (_, s, a) = fixture();
        let d = make(&s, &a);
        assert!(s.answer_decision(d.id, " ", false, None, d.created_at + 1).is_err());
        assert!(s
            .finish_decision(d.id, a.id, DecisionStatus::Completed, "sent", d.created_at + 1)
            .is_err());
        assert_eq!(s.decisions(None).unwrap()[0].status, DecisionStatus::Pending);
    }
    #[test]
    fn acceptance_rolls_back_if_its_message_cannot_be_recorded() {
        let (_, s, a) = fixture();
        let d = make(&s, &a);
        s.conn().unwrap().execute_batch("CREATE TRIGGER reject_delivery BEFORE INSERT ON pending_runs BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
        assert!(s.answer_decision(d.id, "11", false, None, d.created_at + 1).is_err());
        assert_eq!(s.decisions(None).unwrap()[0].status, DecisionStatus::Pending);
        let messages: i64 =
            s.conn().unwrap().query_row("SELECT count(*) FROM messages", [], |r| r.get(0)).unwrap();
        assert_eq!(messages, 0);
    }
    #[test]
    fn unanswered_decisions_survive_reopen_and_late_answers_arrive_once() {
        let (dir, s, a) = fixture();
        let d = make(&s, &a);
        drop(s);
        let s = Store::open(&dir.path().join("test.db")).unwrap();
        s.expire_pending_approvals().unwrap();
        s.recover_decisions().unwrap();
        let (answered, envelope) =
            s.answer_decision(d.id, "11", false, None, d.created_at + 3 * 86400 * 1000).unwrap();
        assert_eq!(answered.answer.as_deref(), Some("11"));
        assert_eq!(envelope.trust, Trust::System);
        assert_eq!(envelope.from, Participant::System);
        assert!(s.get_message(envelope.id).unwrap().is_some());
        assert!(s
            .answer_decision(d.id, "10", false, None, d.created_at + 4 * 86400 * 1000)
            .is_err());
        assert_eq!(s.decisions(None).unwrap()[0].answer.as_deref(), Some("11"));
    }
    #[test]
    fn a_later_scan_preserves_age_snooze_and_the_question_that_was_answered() {
        let (_, s, a) = fixture();
        let d = make(&s, &a);
        s.snooze_decision(d.id, d.created_at + 7 * 86400 * 1000, d.created_at).unwrap();
        let again = s
            .request_decision(a.id, a.group_id, &d.topic, request(), None, d.created_at + 1)
            .unwrap();
        assert_eq!(again.id, d.id);
        assert_eq!(again.created_at, d.created_at);
        assert_eq!(again.snoozed_until, Some(d.created_at + 7 * 86400 * 1000));
        assert_eq!(again.remind_at, d.created_at + 7 * 86400 * 1000);
        s.answer_decision(d.id, "11", false, None, d.created_at + 2).unwrap();
        let mut changed = request();
        changed.question = "Pay 100 or 200?".into();
        let frozen = s
            .request_decision(a.id, a.group_id, &d.topic, changed, None, d.created_at + 3)
            .unwrap();
        assert_eq!(frozen.request.question, "10 or 11?");
        assert_eq!(frozen.answer.as_deref(), Some("11"));
    }
    #[test]
    fn a_changed_question_refuses_the_answer_to_its_previous_version() {
        let (_, s, a) = fixture();
        let d = make(&s, &a);
        let mut changed = request();
        changed.question = "Friday at 10 or 11?".into();
        let revised =
            s.request_decision(a.id, a.group_id, &d.topic, changed, None, d.created_at).unwrap();
        assert!(revised.updated_at > d.updated_at);
        assert!(s
            .answer_decision(d.id, "11", false, Some(d.updated_at), d.created_at + 2)
            .is_err());
        assert_eq!(s.decisions(None).unwrap()[0].status, DecisionStatus::Pending);
    }
    #[test]
    fn rescanning_completed_work_does_not_reopen_it() {
        let (_, s, a) = fixture();
        let d = make(&s, &a);
        s.answer_decision(d.id, "11", false, None, d.created_at + 1).unwrap();
        s.finish_decision(
            d.id,
            a.id,
            DecisionStatus::Completed,
            "Recorded 11 in the plan.",
            d.created_at + 2,
        )
        .unwrap();
        let scanned = make(&s, &a);
        assert_eq!(scanned.id, d.id);
        assert_eq!(scanned.status, DecisionStatus::Completed);
        assert_eq!(s.decisions(None).unwrap().len(), 1);
    }
    #[test]
    fn concurrent_scans_and_answers_have_one_winner() {
        let (_, s, a) = fixture();
        let mut jobs = vec![];
        for _ in 0..8 {
            let store = s.clone();
            let a = a.clone();
            jobs.push(std::thread::spawn(move || make(&store, &a)));
        }
        let rows: Vec<_> = jobs.into_iter().map(|j| j.join().unwrap()).collect();
        assert!(rows.iter().all(|row| row.id == rows[0].id));
        let mut jobs = vec![];
        for _ in 0..8 {
            let store = s.clone();
            let id = rows[0].id;
            jobs.push(std::thread::spawn(move || {
                store.answer_decision(id, "11", false, None, 1_700_000_000_001).is_ok()
            }));
        }
        assert_eq!(jobs.into_iter().filter_map(|j| j.join().ok()).filter(|ok| *ok).count(), 1);
    }
    #[test]
    fn one_agent_cannot_finish_anothers_decision() {
        let (_, s, a) = fixture();
        let d = make(&s, &a);
        s.answer_decision(d.id, "11", false, None, d.created_at + 1).unwrap();
        assert!(s
            .finish_decision(
                d.id,
                AgentId::new(),
                DecisionStatus::Completed,
                "sent",
                d.created_at + 2
            )
            .is_err());
        assert_eq!(s.decisions(None).unwrap()[0].status, DecisionStatus::Answered);
    }
    #[test]
    fn restart_keeps_answer_and_requires_explicit_resume_before_completion() {
        let (_, s, a) = fixture();
        let d = make(&s, &a);
        s.answer_decision(d.id, "11", false, None, d.created_at + 1).unwrap();
        assert_eq!(s.recover_decisions().unwrap(), 1);
        assert_eq!(s.recover_decisions().unwrap(), 0);
        assert!(s
            .finish_decision(d.id, a.id, DecisionStatus::Completed, "sent", d.created_at + 2)
            .is_err());
        let (resumed, _) = s.answer_decision(d.id, "", true, None, d.created_at + 3).unwrap();
        assert!(!resumed.interrupted);
        assert_eq!(resumed.answer.as_deref(), Some("11"));
        assert!(s.answer_decision(d.id, "", true, None, d.created_at + 4).is_err());
        let completed = s
            .finish_decision(
                d.id,
                a.id,
                DecisionStatus::Completed,
                "Meeting confirmed at 11.",
                d.created_at + 5,
            )
            .unwrap();
        assert_eq!(completed.status, DecisionStatus::Completed);
        assert_eq!(s.recover_decisions().unwrap(), 0);
    }
    #[test]
    fn withdrawal_is_a_receipt_and_cannot_hide_an_answer() {
        let (_, s, a) = fixture();
        let d = make(&s, &a);
        let withdrawn = s
            .finish_decision(
                d.id,
                a.id,
                DecisionStatus::Withdrawn,
                "Alex canceled.",
                d.created_at + 1,
            )
            .unwrap();
        assert_eq!(withdrawn.outcome.as_deref(), Some("Alex canceled."));
        assert!(s.answer_decision(d.id, "11", false, None, d.created_at + 2).is_err());
        assert_eq!(s.decisions(None).unwrap().len(), 1);
    }
    #[test]
    fn deadline_reminders_are_batched_and_snooze_suppresses_them() {
        let (_, s, a) = fixture();
        let now = 1_700_000_000_000;
        let d =
            s.request_decision(a.id, a.group_id, "time", request(), Some(now + 1000), now).unwrap();
        assert_eq!(s.decision_reminders(now).unwrap(), 1);
        assert_eq!(s.decision_reminders(now + 1).unwrap(), 0);
        s.snooze_decision(d.id, now + 10000, now + 2).unwrap();
        assert_eq!(s.decision_reminders(now + 9999).unwrap(), 0);
        assert_eq!(s.decision_reminders(now + 10000).unwrap(), 1);
        assert_eq!(s.decision_reminders(now + 10001).unwrap(), 0);
        assert!(s.decisions(None).unwrap()[0].snoozed_until.is_none());
    }
}
