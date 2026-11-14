//! What each finished run ended in. See `domain/outcome.rs` for what an outcome
//! is and why the runtime records it rather than a model.
//!
//! Every fact here is one the store already holds when the run settles: the
//! message the run began with, the owner's newest note to the operator, the
//! newest failed call, and what the run left on the desk. So the row is written
//! inside the commit that settles the run, and the runtime hands it nothing but
//! what only the runtime knows: how the run came to settle, and what it spent.

use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};

use super::store::row_to_envelope;
use super::{Store, StoreError};
use crate::domain::envelope::{Envelope, NoticeKind, Part, Participant};
use crate::domain::ids::{AgentId, GroupId, RunId};
use crate::domain::outcome::{
    first_sentence, headline_of, Need, Outcome, Settled, Status, Trigger, NO_NOTE, STOPPED,
};

const ENVELOPE: &str = "m.id,m.run_id,m.channel_id,m.from_kind,m.from_agent,m.to_kind,m.to_agent,\
     m.parts,m.trust,m.hop,m.expects_reply,m.intent,m.cause,m.created_at";

const COLUMNS: &str = "run_id,agent_id,group_id,trigger,trigger_ref,status,headline,note_id,needs,\
     steps,started_at,finished_at,seen_at";

/// The most a list returns, whatever it was asked for.
pub const LIST_MAX: u32 = 500;

/// A failed model call, as the runtime writes one: a notice of its own.
const FAILED: &str = "EXISTS (SELECT 1 FROM json_each(m.parts) \
     WHERE json_extract(value,'$.type')='notice' AND json_extract(value,'$.kind')='upstreamError')";

/// Writes the outcome of a run that has just settled, in the caller's commit.
///
/// Nothing for a run taken into another run's turn, whose answer is that run's
/// outcome; for a run whose owner is gone to the compost; and for a run this
/// file cannot say what started. Nothing either for a run that already has
/// one, so settling twice can never announce twice.
pub(super) fn record(
    tx: &Connection,
    run: RunId,
    settled: Settled,
    steps: u32,
    now: i64,
) -> Result<Option<Outcome>, StoreError> {
    if settled == Settled::Absorbed {
        return Ok(None);
    }
    let Some(start) = first(tx, "1=1", "ASC", params![run.to_string()])? else {
        return Ok(None);
    };
    let Some(Owner { agent: owner, group, trigger, trigger_ref }) = owned(tx, &start)? else {
        return Ok(None);
    };

    let note = first(
        tx,
        "m.from_kind='agent' AND m.from_agent=?2 AND m.to_kind='human'",
        "DESC",
        params![run.to_string(), owner.to_string()],
    )?;
    // Failed when the last word is the failure: a call that failed and was
    // followed by a closing note is a run that recovered, and calling it failed
    // would put the wrong word beside the right headline.
    let failure = first(tx, FAILED, "DESC", params![run.to_string()])?
        .filter(|failure| note.as_ref().is_none_or(|note| failure.created_at > note.created_at));
    let needs = left_open(tx, run)?;

    let quoted = note.as_ref().and_then(headline_of);
    let (status, headline, note_id) = match (settled, failure) {
        (Settled::Stopped, _) => {
            (Status::Stopped, quoted.unwrap_or_else(|| STOPPED.into()), note.as_ref().map(|n| n.id))
        }
        (_, Some(failure)) => {
            (Status::Failed, first_sentence(&notice_text(&failure)), Some(failure.id))
        }
        _ => (
            if needs.is_empty() { Status::Done } else { Status::NeedsYou },
            quoted.unwrap_or_else(|| NO_NOTE.into()),
            note.as_ref().map(|n| n.id),
        ),
    };

    let outcome = Outcome {
        run_id: run,
        agent_id: owner,
        group_id: group,
        trigger,
        trigger_ref,
        status,
        headline,
        note_id,
        needs,
        steps,
        started_at: start.created_at,
        finished_at: now,
        seen_at: None,
    };
    Ok(insert(tx, &outcome)?.then_some(outcome))
}

/// Writes `interrupted` for a run a restart is reporting, in the recovery's
/// commit. `original` is the message the run began with and `notice` the line
/// the restart wrote beside it, which is what the outcome points at: it is
/// where Try again is.
///
/// What it spent is counted from what the store kept, a usage row for every
/// call that answered and a notice for every one that failed, because the
/// runtime that counted it is the process that died.
pub(super) fn record_interrupted(
    tx: &Connection,
    original: &Envelope,
    notice: &Envelope,
    now: i64,
) -> Result<(), StoreError> {
    let Some(Owner { agent: owner, group, trigger, trigger_ref }) = owned(tx, original)? else {
        return Ok(());
    };
    let run = original.run_id.to_string();
    let steps: u32 = tx.query_row(
        &format!(
            "SELECT (SELECT count(*) FROM usage WHERE run_id=?1)
                  + (SELECT count(*) FROM messages m WHERE m.run_id=?1 AND {FAILED})"
        ),
        [&run],
        |row| row.get(0),
    )?;
    insert(
        tx,
        &Outcome {
            run_id: original.run_id,
            agent_id: owner,
            group_id: group,
            trigger,
            trigger_ref,
            status: Status::Interrupted,
            headline: first_sentence(&notice_text(notice)),
            note_id: Some(notice.id),
            needs: left_open(tx, original.run_id)?,
            steps,
            started_at: original.created_at,
            finished_at: now,
            seen_at: None,
        },
    )?;
    Ok(())
}

/// Who a run belongs to, and what started it.
struct Owner {
    agent: AgentId,
    group: GroupId,
    trigger: Trigger,
    trigger_ref: Option<String>,
}

/// The owner of a run that began with `start`. None for an owner that no
/// longer exists or has been deleted.
fn owned(tx: &Connection, start: &Envelope) -> Result<Option<Owner>, StoreError> {
    let Participant::Agent { id: owner } = start.to else { return Ok(None) };
    let Some((trigger, trigger_ref)) = Trigger::of(start) else { return Ok(None) };
    let group: Option<String> = tx
        .query_row(
            "SELECT group_id FROM agents WHERE id=?1 AND lifecycle<>'terminated'",
            [owner.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(group) = group else { return Ok(None) };
    let group = group.parse().map_err(|e| StoreError::Corrupt(format!("bad group id: {e}")))?;
    Ok(Some(Owner { agent: owner, group, trigger, trigger_ref }))
}

/// The first message of a run that matches `filter`, in `order`: the oldest
/// with `ASC`, the newest with `DESC`. `?1` in `args` is the run.
fn first(
    tx: &Connection,
    filter: &str,
    order: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Result<Option<Envelope>, StoreError> {
    let found = tx
        .query_row(
            &format!(
                "SELECT {ENVELOPE} FROM messages m WHERE m.run_id=?1 AND {filter}
                  ORDER BY m.created_at {order}, m.id {order} LIMIT 1"
            ),
            args,
            row_to_envelope,
        )
        .optional()?;
    found.transpose()
}

/// What a run filed that is still waiting on the operator: its decisions still
/// pending, then the escalation it raised if nobody has cleared it.
fn left_open(tx: &Connection, run: RunId) -> Result<Vec<Need>, StoreError> {
    let run = run.to_string();
    let mut needs = Vec::new();
    let mut decisions = tx.prepare(
        "SELECT id FROM decisions WHERE filed_run=?1 AND status='pending' ORDER BY created_at,id",
    )?;
    for id in decisions.query_map([&run], |row| row.get::<_, String>(0))? {
        let id = id?.parse().map_err(|e| StoreError::Corrupt(format!("bad decision id: {e}")))?;
        needs.push(Need::Decision { id });
    }
    let mut escalations =
        tx.prepare("SELECT id FROM escalations WHERE run_id=?1 AND cleared_at IS NULL")?;
    for id in escalations.query_map([&run], |row| row.get::<_, String>(0))? {
        let id = id?.parse().map_err(|e| StoreError::Corrupt(format!("bad escalation id: {e}")))?;
        needs.push(Need::Escalation { id });
    }
    Ok(needs)
}

fn notice_text(message: &Envelope) -> String {
    message
        .parts
        .iter()
        .find_map(|part| match part {
            Part::Notice { kind: NoticeKind::UpstreamError | NoticeKind::Interrupted, text } => {
                Some(text.clone())
            }
            _ => None,
        })
        .unwrap_or_default()
}

/// True when the row is new.
fn insert(tx: &Connection, outcome: &Outcome) -> Result<bool, StoreError> {
    let needs = (!outcome.needs.is_empty())
        .then(|| serde_json::to_string(&outcome.needs).expect("a list of ids serializes"));
    let written = tx.execute(
        &format!("INSERT OR IGNORE INTO outcomes ({COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)"),
        params![
            outcome.run_id.to_string(),
            outcome.agent_id.to_string(),
            outcome.group_id.to_string(),
            outcome.trigger.as_str(),
            outcome.trigger_ref,
            outcome.status.as_str(),
            outcome.headline,
            outcome.note_id.map(|id| id.to_string()),
            needs,
            outcome.steps,
            outcome.started_at,
            outcome.finished_at,
            outcome.seen_at,
        ],
    )?;
    Ok(written == 1)
}

fn read(row: &Row<'_>) -> rusqlite::Result<Result<Outcome, StoreError>> {
    let raw: [String; 3] = [row.get(0)?, row.get(1)?, row.get(2)?];
    let trigger: String = row.get(3)?;
    let trigger_ref: Option<String> = row.get(4)?;
    let status: String = row.get(5)?;
    let headline: String = row.get(6)?;
    let note: Option<String> = row.get(7)?;
    let needs: Option<String> = row.get(8)?;
    let steps: u32 = row.get(9)?;
    let started_at: i64 = row.get(10)?;
    let finished_at: i64 = row.get(11)?;
    let seen_at: Option<i64> = row.get(12)?;
    let corrupt =
        |what: &str, e: &dyn std::fmt::Display| StoreError::Corrupt(format!("{what}: {e}"));
    Ok((|| {
        Ok(Outcome {
            run_id: raw[0].parse().map_err(|e| corrupt("bad run id", &e))?,
            agent_id: raw[1].parse().map_err(|e| corrupt("bad agent id", &e))?,
            group_id: raw[2].parse().map_err(|e| corrupt("bad group id", &e))?,
            trigger: Trigger::parse(&trigger)
                .ok_or_else(|| StoreError::Corrupt(format!("unknown trigger {trigger:?}")))?,
            trigger_ref,
            status: Status::parse(&status)
                .ok_or_else(|| StoreError::Corrupt(format!("unknown status {status:?}")))?,
            headline,
            note_id: note
                .map(|id| id.parse())
                .transpose()
                .map_err(|e| corrupt("bad note id", &e))?,
            needs: needs
                .map(|raw| serde_json::from_str(&raw))
                .transpose()
                .map_err(|e| corrupt("unreadable needs", &e))?
                .unwrap_or_default(),
            steps,
            started_at,
            finished_at,
            seen_at,
        })
    })())
}

impl Store {
    /// Finished runs, newest first: everything since `since` in one group or
    /// all of them, or only what the operator has not seen.
    pub fn outcomes(
        &self,
        since: Option<i64>,
        group: Option<GroupId>,
        unseen_only: bool,
        limit: u32,
    ) -> Result<Vec<Outcome>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM outcomes
              WHERE (?1 IS NULL OR finished_at >= ?1)
                AND (?2 IS NULL OR group_id = ?2)
                AND (?3 = 0 OR seen_at IS NULL)
              ORDER BY finished_at DESC, run_id DESC
              LIMIT ?4"
        ))?;
        let rows = stmt.query_map(
            params![since, group.map(|g| g.to_string()), unseen_only, limit.min(LIST_MAX)],
            read,
        )?;
        let mut outcomes = Vec::new();
        for row in rows {
            outcomes.push(row??);
        }
        Ok(outcomes)
    }

    pub fn outcome(&self, run: RunId) -> Result<Option<Outcome>, StoreError> {
        let conn = self.conn()?;
        let found = conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM outcomes WHERE run_id=?1"),
                [run.to_string()],
                read,
            )
            .optional()?;
        found.transpose()
    }

    /// Marks unseen outcomes seen: the runs named, or every one that finished
    /// by `before`, or the runs named that finished by then. Returns the runs
    /// it marked, so every other window can drop the same rows.
    ///
    /// Bounded by a time rather than by "all", because a list on screen is a
    /// moment: a run that finished after it was drawn is one the operator has
    /// not read, and a Mark all seen that took it too would lose it.
    pub fn mark_outcomes_seen(
        &self,
        runs: Option<&[RunId]>,
        before: Option<i64>,
        now: i64,
    ) -> Result<Vec<RunId>, StoreError> {
        let names = runs.map(|runs| {
            serde_json::to_string(&runs.iter().map(ToString::to_string).collect::<Vec<_>>())
                .expect("a list of ids serializes")
        });
        self.mark_seen_where(
            "(?1 IS NULL OR run_id IN (SELECT value FROM json_each(?1)))
              AND (?2 IS NULL OR finished_at <= ?2)",
            params![names, before],
            now,
        )
    }

    /// Marks seen every outcome owned by the agent whose channel the operator
    /// is looking at.
    pub fn mark_channel_seen(&self, agent: AgentId, now: i64) -> Result<Vec<RunId>, StoreError> {
        self.mark_seen_where("agent_id=?1", params![agent.to_string()], now)
    }

    fn mark_seen_where(
        &self,
        filter: &str,
        args: &[&dyn rusqlite::ToSql],
        now: i64,
    ) -> Result<Vec<RunId>, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let marked: Vec<String> = {
            let mut stmt = tx.prepare(&format!(
                "SELECT run_id FROM outcomes WHERE seen_at IS NULL AND {filter} ORDER BY finished_at"
            ))?;
            let rows = stmt.query_map(args, |row| row.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        if marked.is_empty() {
            return Ok(Vec::new());
        }
        tx.execute(
            "UPDATE outcomes SET seen_at=?2 WHERE seen_at IS NULL
                AND run_id IN (SELECT value FROM json_each(?1))",
            params![serde_json::to_string(&marked).expect("a list of ids serializes"), now],
        )?;
        tx.commit()?;
        marked
            .iter()
            .map(|run| run.parse().map_err(|e| StoreError::Corrupt(format!("bad run id: {e}"))))
            .collect()
    }
}
