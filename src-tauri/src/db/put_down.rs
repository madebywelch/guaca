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
use crate::domain::ids::{AgentId, MessageId, RunId};
use crate::domain::now_ms;
use crate::domain::terminal::Harness;

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
}

/// A coding job a stopping host interrupted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutDownJob {
    pub agent: AgentId,
    pub harness: Harness,
    /// Empty when the program had not named its session yet, which only Codex
    /// does late enough for a stop to land first.
    pub session: String,
    pub directory: String,
    /// What it was asked, which is what a job without a session starts over on.
    pub task: String,
    /// Whether the operator asked for it rather than the agent, which decides
    /// what the agent is told when it finishes.
    pub by_operator: bool,
}

/// What a restart found.
#[derive(Debug, Default)]
pub struct Recovered {
    /// Messages to deliver again, oldest first.
    pub resumed: Vec<PutDown>,
    /// What each conversation being picked up had already spent, so its budget
    /// is the one it was measured against before the restart.
    pub steps: HashMap<RunId, u32>,
    pub jobs: Vec<PutDownJob>,
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
        whole: &[(RunId, u32)],
    ) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        for (id, records) in messages {
            write_put_down(&tx, *id, records)?;
        }
        for (run, steps) in whole {
            mark_whole(&tx, *run, *steps)?;
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
        whole: Option<u32>,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        Store::insert_message(&tx, envelope)?;
        tx.execute(
            "INSERT OR IGNORE INTO pending_runs (run_id,message_id) VALUES (?1,?2)",
            params![envelope.run_id.to_string(), envelope.id.to_string()],
        )?;
        write_put_down(&tx, envelope.id, &[])?;
        if let Some(steps) = whole {
            mark_whole(&tx, envelope.run_id, steps)?;
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

    /// Keeps a coding job a stopping host interrupted, for the next one.
    pub fn put_down_job(&self, job: &PutDownJob) -> Result<(), StoreError> {
        self.conn()?.execute(
            "INSERT INTO put_down_jobs (agent_id,harness,session_id,directory,task,origin)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(agent_id) DO UPDATE SET harness=excluded.harness,
                 session_id=excluded.session_id, directory=excluded.directory,
                 task=excluded.task, origin=excluded.origin",
            params![
                job.agent.to_string(),
                job.harness.as_str(),
                job.session,
                job.directory,
                job.task,
                if job.by_operator { "operator" } else { "agent" },
            ],
        )?;
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

        let whole: HashMap<RunId, u32> = {
            let mut stmt = tx.prepare("SELECT run_id, steps FROM put_down_runs")?;
            let rows =
                stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)))?;
            let mut whole = HashMap::new();
            for row in rows {
                let (run, steps) = row?;
                whole.insert(parse_run(&run)?, steps);
            }
            whole
        };

        let put: Vec<PutDown> = {
            let mut stmt = tx.prepare(&format!(
                "SELECT {ENVELOPE}, p.records FROM messages m
                   JOIN put_down p ON p.message_id = m.id
                  ORDER BY m.created_at, m.id"
            ))?;
            let rows =
                stmt.query_map([], |row| Ok((row_to_envelope(row)?, row.get::<_, String>(14)?)))?;
            let mut put = Vec::new();
            for row in rows {
                let (envelope, records) = row?;
                let records: Vec<MessageId> = serde_json::from_str(&records)
                    .map_err(|err| StoreError::Corrupt(format!("unreadable records: {err}")))?;
                put.push(PutDown { envelope: envelope?, records });
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

        let (resumed, cut): (Vec<PutDown>, Vec<PutDown>) =
            put.into_iter().partition(|put| whole.contains_key(&put.envelope.run_id));
        let picked: HashSet<RunId> = resumed.iter().map(|put| put.envelope.run_id).collect();

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
        for original in &anchors {
            Store::insert_message(&tx, &interrupted(original))?;
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

        let jobs: Vec<PutDownJob> = {
            let mut stmt = tx.prepare(
                "SELECT agent_id,harness,session_id,directory,task,origin FROM put_down_jobs",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })?;
            let mut jobs = Vec::new();
            for row in rows {
                let (agent, harness, session, directory, task, origin) = row?;
                jobs.push(PutDownJob {
                    agent: agent
                        .parse()
                        .map_err(|err| StoreError::Corrupt(format!("bad agent id: {err}")))?,
                    harness: Harness::parse(&harness),
                    session,
                    directory,
                    task,
                    by_operator: origin == "operator",
                });
            }
            jobs
        };
        tx.execute("DELETE FROM put_down_jobs", [])?;

        tx.commit()?;

        let steps = whole.into_iter().filter(|(run, _)| picked.contains(run)).collect();
        Ok(Recovered { resumed, steps, jobs, interrupted: anchors.len() })
    }
}

fn write_put_down(
    tx: &rusqlite::Connection,
    id: MessageId,
    records: &[MessageId],
) -> Result<(), StoreError> {
    let records = serde_json::to_string(records).expect("a list of ids serializes");
    tx.execute(
        "INSERT INTO put_down (message_id,records) VALUES (?1,?2)
         ON CONFLICT(message_id) DO UPDATE SET records=excluded.records",
        params![id.to_string(), records],
    )?;
    Ok(())
}

fn mark_whole(tx: &rusqlite::Connection, run: RunId, steps: u32) -> Result<(), StoreError> {
    tx.execute(
        "INSERT INTO put_down_runs (run_id,steps) VALUES (?1,?2)
         ON CONFLICT(run_id) DO UPDATE SET steps=max(steps, excluded.steps)",
        params![run.to_string(), steps],
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
    fn a_conversation_put_down_whole_is_picked_up_with_its_records_and_its_budget() {
        let (_dir, store, agent) = fixture();
        let run = RunId::new();
        let original = asked(&agent, run, "do the work");
        store.append_delivery(&original).unwrap();
        let record = MessageId::new();
        store.put_down(&[(original.id, &[record])], &[(run, 7)]).unwrap();

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
        store.append_put_down(&late, Some(0)).unwrap();

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

    #[test]
    fn a_put_down_job_is_read_back_once() {
        let (_dir, store, agent) = fixture();
        let job = PutDownJob {
            agent: agent.id,
            harness: Harness::Claude,
            session: "0b7c".into(),
            directory: "site".into(),
            task: "Fix the header".into(),
            by_operator: true,
        };
        store.put_down_job(&job).unwrap();

        assert_eq!(store.recover().unwrap().jobs, vec![job]);
        assert!(store.recover().unwrap().jobs.is_empty());
    }
}
