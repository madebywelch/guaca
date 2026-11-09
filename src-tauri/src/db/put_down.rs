//! Work a stopping host put down, and the one pass that reads it back.
//!
//! A restart used to end every conversation in flight: each was reported as
//! interrupted, with a button to send it again, because nothing could say
//! whether a tool call had finished before the process died. A host that is
//! asked to stop can do better than that, and this is the half of it that
//! outlives the process. `Runtime::drain` is the other half, and says what a
//! boundary is.
//!
//! What this file keeps is only what a drain could vouch for. A message is put
//! down with the records of the turn it was in the middle of, and a
//! conversation is marked whole once nothing of it is left running. Everything
//! else a restart finds, whether a crash or a turn still inside a tool call
//! when the host was stopped, is reported interrupted exactly as before.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, TransactionBehavior};

use super::store::row_to_envelope;
use super::{Store, StoreError};
use crate::domain::envelope::{Envelope, Intent, NoticeKind, Part, Participant, Trust};
use crate::domain::ids::{MessageId, RunId};
use crate::domain::now_ms;
use crate::runtime::guard::{GuardSnapshot, RunState};

const ENVELOPE: &str = "m.id,m.run_id,m.channel_id,m.from_kind,m.from_agent,m.to_kind,m.to_agent,\
     m.parts,m.trust,m.hop,m.expects_reply,m.intent,m.cause,m.created_at";

/// What a restart says about work it cannot pick up.
///
/// The same words a restart has always used, because the situation is the
/// same one: something may have happened that was never written down.
const INTERRUPTED: &str = "The backend restarted before this conversation finished. Previous \
     messages and files are preserved. Review any actions already taken before retrying; an \
     external action may have completed without its result being recorded.";

/// A message put down, as the next host reads it back.
#[derive(Debug, Clone, PartialEq)]
pub struct PutDown {
    pub envelope: Envelope,
    /// The records of the turn it was being answered in, oldest first. Empty
    /// for a message no turn had started, which is delivered as though it had
    /// just arrived.
    pub records: Vec<MessageId>,
    pub absorbed: Vec<MessageId>,
}

/// What a restart found.
#[derive(Debug, Default)]
pub struct Recovered {
    /// Messages to deliver again, oldest first.
    pub resumed: Vec<PutDown>,
    /// What each conversation being picked up had already spent, so its budget
    /// is the one it was measured against before the restart.
    pub steps: HashMap<RunId, u32>,
    pub guards: HashMap<RunId, RunState>,
    /// Conversations reported interrupted.
    pub interrupted: usize,
}

impl Recovered {
    pub fn runs(&self) -> HashSet<RunId> {
        self.resumed.iter().map(|put| put.envelope.run_id).collect()
    }
}

impl Store {
    /// Puts messages down and marks the conversations left with nothing else
    /// outstanding, in one commit.
    ///
    /// One commit because a mark without its messages would pick up a
    /// conversation with a hole in it. The caller holds the runtime's run lock
    /// across this, which is what makes `whole` true when it is written.
    pub fn put_down(
        &self,
        messages: &[(MessageId, &[MessageId])],
        whole: &[(RunId, u32, GuardSnapshot)],
    ) -> Result<(), StoreError> {
        self.put_down_with_absorbed(messages, whole, &[])
    }

    pub fn put_down_with_absorbed(
        &self,
        messages: &[(MessageId, &[MessageId])],
        whole: &[(RunId, u32, GuardSnapshot)],
        absorbed: &[MessageId],
    ) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        for (index, (id, records)) in messages.iter().enumerate() {
            write_put_down(&tx, *id, records, if index == 0 { absorbed } else { &[] })?;
        }
        for (run, steps, snapshot) in whole {
            mark_whole(&tx, *run, *steps, snapshot)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Accepts a message while the host is stopping, putting it down rather
    /// than queueing it: nothing in this process will start another turn.
    ///
    /// The same commit as an ordinary acceptance, with the put-down row added,
    /// so a message sent in the last seconds is neither lost nor reported
    /// interrupted when it never began.
    pub fn append_put_down(
        &self,
        envelope: &Envelope,
        whole: Option<(u32, GuardSnapshot)>,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        Store::insert_message(&tx, envelope)?;
        tx.execute(
            "INSERT OR IGNORE INTO pending_runs (run_id,message_id) VALUES (?1,?2)",
            params![envelope.run_id.to_string(), envelope.id.to_string()],
        )?;
        write_put_down(&tx, envelope.id, &[], &[])?;
        if let Some((steps, snapshot)) = whole {
            mark_whole(&tx, envelope.run_id, steps, &snapshot)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Takes back everything put down for one conversation: the operator
    /// stopped it, and work called off is never picked up.
    pub fn forget_put_down(&self, run: RunId) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM put_down WHERE message_id IN (SELECT id FROM messages WHERE run_id=?1)",
            [run.to_string()],
        )?;
        tx.execute("DELETE FROM put_down_runs WHERE run_id=?1", [run.to_string()])?;
        tx.commit()?;
        Ok(())
    }

    /// Reads back what the last host put down, and reports everything else it
    /// left as interrupted. Called once, before any actor starts.
    ///
    /// One commit, so a restart that dies partway through this repeats it
    /// rather than picking something up twice or reporting it twice. What is
    /// picked up keeps its `pending_runs` row: until it settles it is a
    /// conversation in flight like any other, and a crash in the middle of it
    /// is reported like any other.
    pub fn recover(&self) -> Result<Recovered, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let whole: HashMap<RunId, (u32, Option<RunState>)> = {
            let mut stmt = tx.prepare("SELECT run_id, steps, guard_snapshot FROM put_down_runs")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            let mut whole = HashMap::new();
            for row in rows {
                let (run, steps, snapshot) = row?;
                let state = snapshot
                    .map(|snapshot| {
                        let snapshot: GuardSnapshot =
                            serde_json::from_str(&snapshot).map_err(|err| {
                                StoreError::Corrupt(format!("unreadable guard snapshot: {err}"))
                            })?;
                        RunState::from_snapshot(snapshot)
                            .map_err(|err| StoreError::Corrupt(err.into()))
                    })
                    .transpose()?;
                whole.insert(parse_run(&run)?, (steps, state));
            }
            whole
        };

        let put: Vec<PutDown> = {
            let mut stmt = tx.prepare(&format!(
                "SELECT {ENVELOPE}, p.records, p.absorbed FROM messages m
                   JOIN put_down p ON p.message_id = m.id
                  ORDER BY m.created_at, m.id"
            ))?;
            let rows = stmt.query_map([], |row| {
                Ok((row_to_envelope(row)?, row.get::<_, String>(14)?, row.get::<_, String>(15)?))
            })?;
            let mut put = Vec::new();
            for row in rows {
                let (envelope, records, absorbed) = row?;
                let records: Vec<MessageId> = serde_json::from_str(&records)
                    .map_err(|err| StoreError::Corrupt(format!("unreadable records: {err}")))?;
                let absorbed = serde_json::from_str(&absorbed).map_err(|err| {
                    StoreError::Corrupt(format!("unreadable absorbed messages: {err}"))
                })?;
                put.push(PutDown { envelope: envelope?, records, absorbed });
            }
            put
        };

        let pending: Vec<Envelope> = {
            let mut stmt = tx.prepare(&format!(
                "SELECT {ENVELOPE} FROM messages m
                   JOIN pending_runs p ON p.message_id = m.id
                  ORDER BY m.created_at, m.id"
            ))?;
            let rows = stmt.query_map([], row_to_envelope)?;
            let mut pending = Vec::new();
            for row in rows {
                pending.push(row??);
            }
            pending
        };

        let (mut resumed, cut): (Vec<PutDown>, Vec<PutDown>) =
            put.into_iter().partition(|put| whole.contains_key(&put.envelope.run_id));
        let already: HashSet<_> = resumed.iter().map(|put| put.envelope.id).collect();
        {
            let mut stmt = tx.prepare(&format!(
                "SELECT {ENVELOPE} FROM messages m JOIN routine_occurrences o ON o.message_id=m.id WHERE o.state='pending' ORDER BY m.created_at,m.id"
            ))?;
            let rows = stmt.query_map([], row_to_envelope)?;
            for row in rows {
                let envelope = row??;
                if !already.contains(&envelope.id) {
                    resumed.push(PutDown { envelope, records: Vec::new(), absorbed: Vec::new() });
                }
            }
        }
        let picked: HashSet<RunId> = resumed.iter().map(|put| put.envelope.run_id).collect();
        let associated: HashSet<MessageId> =
            resumed.iter().flat_map(|put| put.absorbed.iter().copied()).collect();

        // Where each interrupted conversation's notice goes: the message it
        // began with, which is what **Try again** sends again. A conversation
        // only this file knows about began with the message put down for it.
        let mut anchors: Vec<Envelope> = Vec::new();
        let mut anchored: HashSet<RunId> = HashSet::new();
        for original in pending.iter().chain(cut.iter().map(|put| &put.envelope)) {
            if !picked.contains(&original.run_id) && anchored.insert(original.run_id) {
                anchors.push(original.clone());
            }
        }
        {
            let mut stmt = tx.prepare(&format!(
                "SELECT {ENVELOPE},o.owner_run FROM messages m JOIN routine_occurrences o ON o.message_id=m.id WHERE o.state='started'"
            ))?;
            let rows = stmt.query_map([], |row| {
                Ok((row_to_envelope(row)?, row.get::<_, Option<String>>(14)?))
            })?;
            for row in rows {
                let (original, owner) = row?;
                let original = original?;
                let owner_picked = owner
                    .as_deref()
                    .and_then(|value| value.parse::<RunId>().ok())
                    .is_some_and(|run| picked.contains(&run));
                if !(owner_picked && associated.contains(&original.id))
                    && !picked.contains(&original.run_id)
                    && anchored.insert(original.run_id)
                {
                    anchors.push(original);
                }
            }
        }
        for original in &anchors {
            Store::insert_message(&tx, &interrupted(original))?;
        }

        for original in &anchors {
            tx.execute("UPDATE routine_occurrences SET state='done' WHERE message_id=?1 AND state='started'", [original.id.to_string()])?;
        }

        for put in &resumed {
            tx.execute(
                "INSERT OR IGNORE INTO pending_runs (run_id,message_id) VALUES (?1,?2)",
                params![put.envelope.run_id.to_string(), put.envelope.id.to_string()],
            )?;
        }
        let keep: Vec<String> = picked.iter().map(ToString::to_string).collect();
        tx.execute(
            "DELETE FROM pending_runs WHERE run_id NOT IN (SELECT value FROM json_each(?1))",
            [serde_json::to_string(&keep).expect("a list of strings serializes")],
        )?;
        tx.execute("DELETE FROM put_down", [])?;
        tx.execute("DELETE FROM put_down_runs", [])?;

        tx.commit()?;

        let mut steps = HashMap::new();
        let mut guards = HashMap::new();
        for (run, (spent, state)) in whole {
            if picked.contains(&run) {
                steps.insert(run, spent);
                if let Some(state) = state {
                    guards.insert(run, state);
                }
            }
        }
        Ok(Recovered { resumed, steps, guards, interrupted: anchors.len() })
    }
}

fn write_put_down(
    tx: &rusqlite::Connection,
    id: MessageId,
    records: &[MessageId],
    absorbed: &[MessageId],
) -> Result<(), StoreError> {
    let records = serde_json::to_string(records).expect("a list of ids serializes");
    let absorbed = serde_json::to_string(absorbed).expect("a list of ids serializes");
    tx.execute(
        "INSERT INTO put_down (message_id,records,absorbed) VALUES (?1,?2,?3)
         ON CONFLICT(message_id) DO UPDATE SET records=excluded.records,absorbed=excluded.absorbed",
        params![id.to_string(), records, absorbed],
    )?;
    Ok(())
}

fn mark_whole(
    tx: &rusqlite::Connection,
    run: RunId,
    steps: u32,
    snapshot: &GuardSnapshot,
) -> Result<(), StoreError> {
    let snapshot = serde_json::to_string(snapshot).expect("guard snapshot serializes");
    tx.execute(
        "INSERT INTO put_down_runs (run_id,steps,guard_snapshot) VALUES (?1,?2,?3)
         ON CONFLICT(run_id) DO UPDATE SET steps=excluded.steps, guard_snapshot=excluded.guard_snapshot",
        params![run.to_string(), steps, snapshot],
    )?;
    Ok(())
}

fn parse_run(raw: &str) -> Result<RunId, StoreError> {
    raw.parse().map_err(|err| StoreError::Corrupt(format!("bad run id: {err}")))
}

fn interrupted(original: &Envelope) -> Envelope {
    Envelope {
        id: MessageId::new(),
        run_id: original.run_id,
        channel_id: original.channel_id,
        from: Participant::System,
        to: original.to,
        parts: vec![Part::Notice { kind: NoticeKind::Interrupted, text: INTERRUPTED.into() }],
        trust: Trust::System,
        hop: 0,
        expects_reply: false,
        intent: Intent::Courtesy,
        cause: Some(original.id),
        created_at: now_ms(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::{AgentCard, CleanDraft};

    fn fixture() -> (tempfile::TempDir, Store, AgentCard) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();
        let agent = store
            .create_agent(&CleanDraft {
                group_id: None,
                name: "Worker".into(),
                avatar: "avocado".into(),
                color: "#7fb069".into(),
                model: String::new(),
                subscription_model: None,
                reasoning_effort: None,
                system_prompt: String::new(),
                skills: vec![],
            })
            .unwrap();
        (dir, store, agent)
    }

    fn asked(agent: &AgentCard, run: RunId, text: &str) -> Envelope {
        Envelope {
            id: MessageId::new(),
            run_id: run,
            channel_id: agent.id,
            from: Participant::Human,
            to: Participant::Agent { id: agent.id },
            parts: vec![Part::text(text)],
            trust: Trust::Operator,
            hop: 0,
            expects_reply: true,
            intent: Intent::Work,
            cause: None,
            created_at: now_ms(),
        }
    }

    fn notices(store: &Store, agent: &AgentCard) -> Vec<Envelope> {
        store
            .channel_messages(agent.id, 50)
            .unwrap()
            .into_iter()
            .filter(|m| {
                matches!(m.parts.first(), Some(Part::Notice { kind: NoticeKind::Interrupted, .. }))
            })
            .collect()
    }

    #[test]
    fn legacy_whole_row_resumes_without_inventing_a_guard() {
        let (_dir, store, agent) = fixture();
        let run = RunId::new();
        let original = asked(&agent, run, "legacy");
        store.append_delivery(&original).unwrap();
        store
            .put_down(
                &[(original.id, &[])],
                &[(run, 3, RunState::new(Default::default()).snapshot())],
            )
            .unwrap();
        store.conn().unwrap().execute("UPDATE put_down_runs SET guard_snapshot=NULL", []).unwrap();
        let recovered = store.recover().unwrap();
        assert_eq!(recovered.steps.get(&run), Some(&3));
        assert!(!recovered.guards.contains_key(&run));
        assert_eq!(recovered.resumed.len(), 1);
    }

    #[test]
    fn failing_whole_marker_rolls_back_the_message_put_down() {
        let (_dir, store, agent) = fixture();
        let run = RunId::new();
        let original = asked(&agent, run, "atomic");
        store.append_delivery(&original).unwrap();
        store.conn().unwrap().execute_batch("CREATE TRIGGER fail_whole BEFORE INSERT ON put_down_runs BEGIN SELECT RAISE(ABORT, 'crash'); END;").unwrap();
        assert!(store
            .put_down(
                &[(original.id, &[])],
                &[(run, 0, RunState::new(Default::default()).snapshot())]
            )
            .is_err());
        let recovered = store.recover().unwrap();
        assert!(recovered.resumed.is_empty());
        assert_eq!(recovered.interrupted, 1);
    }

    #[test]
    fn unreadable_snapshot_leaves_recovery_rows_intact() {
        let (_dir, store, agent) = fixture();
        let run = RunId::new();
        let original = asked(&agent, run, "invalid");
        store.append_delivery(&original).unwrap();
        store
            .put_down(
                &[(original.id, &[])],
                &[(run, 0, RunState::new(Default::default()).snapshot())],
            )
            .unwrap();
        store.conn().unwrap().execute("UPDATE put_down_runs SET guard_snapshot='{}'", []).unwrap();
        assert!(store.recover().is_err());
        let conn = store.conn().unwrap();
        let count: u32 =
            conn.query_row("SELECT count(*) FROM put_down", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn concurrent_whole_markers_keep_each_snapshot_with_its_steps() {
        let (_dir, store, agent) = fixture();
        let run = RunId::new();
        let first = asked(&agent, run, "first");
        let second = asked(&agent, run, "second");
        store.append_delivery(&first).unwrap();
        store.append_delivery(&second).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = [first.id, second.id]
            .into_iter()
            .enumerate()
            .map(|(index, id)| {
                let store = store.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let steps = index as u32 + 1;
                    let mut state = RunState::new(Default::default());
                    state.resume_at(steps);
                    barrier.wait();
                    store.put_down(&[(id, &[])], &[(run, steps, state.snapshot())]).unwrap();
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let recovered = store.recover().unwrap();
        assert_eq!(recovered.resumed.len(), 2);
        assert_eq!(recovered.steps[&run], recovered.guards[&run].steps_used());
    }

    #[test]
    fn a_conversation_put_down_whole_is_picked_up_with_its_records_and_its_budget() {
        let (_dir, store, agent) = fixture();
        let run = RunId::new();
        let original = asked(&agent, run, "do the work");
        store.append_delivery(&original).unwrap();
        let record = MessageId::new();
        let mut state = RunState::new(Default::default());
        state.resume_at(7);
        store.put_down(&[(original.id, &[record])], &[(run, 7, state.snapshot())]).unwrap();

        let recovered = store.recover().unwrap();
        assert_eq!(recovered.interrupted, 0);
        assert!(notices(&store, &agent).is_empty(), "nothing to review: it carries on");
        assert_eq!(recovered.resumed.len(), 1);
        assert_eq!(recovered.resumed[0].envelope, original);
        assert_eq!(recovered.resumed[0].records, vec![record]);
        assert_eq!(recovered.steps.get(&run), Some(&7));

        // Picked up is in flight again, so a crash before it settles is the
        // ordinary kind of restart and is reported like one.
        let crashed = store.recover().unwrap();
        assert!(crashed.resumed.is_empty());
        assert_eq!(crashed.interrupted, 1);
        assert_eq!(notices(&store, &agent)[0].cause, Some(original.id));
    }

    #[test]
    fn a_conversation_that_was_still_running_somewhere_is_reported_and_none_of_it_picked_up() {
        let (_dir, store, agent) = fixture();
        let run = RunId::new();
        let original = asked(&agent, run, "do the work");
        store.append_delivery(&original).unwrap();
        let queued = asked(&agent, run, "and this");
        store.append_delivery(&queued).unwrap();
        // One message put down, and the conversation never marked whole: a turn
        // in it was still inside a tool call when the host stopped.
        store.put_down(&[(queued.id, &[])], &[]).unwrap();

        let recovered = store.recover().unwrap();
        assert!(recovered.resumed.is_empty());
        assert_eq!(recovered.interrupted, 1);
        let notices = notices(&store, &agent);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].cause, Some(original.id), "where Try again starts it over");
        assert_eq!(store.recover().unwrap().interrupted, 0, "reported once");
    }

    #[test]
    fn a_message_put_down_after_its_conversation_settled_is_reported_where_it_was_put_down() {
        let (_dir, store, agent) = fixture();
        let run = RunId::new();
        let original = asked(&agent, run, "a correction, taken into another turn");
        store.append_delivery(&original).unwrap();
        store.settle_run(run).unwrap();
        store.put_down(&[(original.id, &[])], &[]).unwrap();

        let recovered = store.recover().unwrap();
        assert_eq!(recovered.interrupted, 1);
        assert_eq!(notices(&store, &agent)[0].cause, Some(original.id));
    }

    #[test]
    fn a_message_sent_while_the_host_was_stopping_is_picked_up_and_not_reported() {
        let (_dir, store, agent) = fixture();
        let run = RunId::new();
        let late = asked(&agent, run, "sent during the update");
        store
            .append_put_down(&late, Some((0, RunState::new(Default::default()).snapshot())))
            .unwrap();

        let recovered = store.recover().unwrap();
        assert_eq!(recovered.interrupted, 0);
        assert_eq!(recovered.resumed.len(), 1);
        assert_eq!(recovered.resumed[0].envelope, late);
        assert!(recovered.resumed[0].records.is_empty());
    }

    #[test]
    fn a_restart_that_was_not_a_drain_reports_everything_as_before() {
        let (_dir, store, agent) = fixture();
        let original = asked(&agent, RunId::new(), "do the work");
        store.append_delivery(&original).unwrap();

        let recovered = store.recover().unwrap();
        assert!(recovered.resumed.is_empty());
        assert_eq!(recovered.interrupted, 1);
    }
}
