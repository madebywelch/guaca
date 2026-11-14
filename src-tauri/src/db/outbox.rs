//! The outbox: what to deliver to a person. See `domain/outbox.rs` for what a
//! row is and what earns one an interruption.
//!
//! Rows are written by the producers in their own commits, through [`enqueue`],
//! and withdrawn the same way when what they announce is answered or seen.
//! Everything else here is the dispatcher's: reading what is due, holding,
//! gathering, offering, and the one claim that decides which window shows it.

use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};

use super::{Store, StoreError};
use crate::domain::ids::AgentId;
use crate::domain::outbox::{
    Delivery, Destination, Draft, Gathered, Kind, Target, Why, EXPIRE_MS, GATHER_MS,
};
use crate::domain::outcome::Status as Ended;

/// Only the windows are delivered to yet.
const CLIENTS: &str = Destination::Clients.as_str();

/// Writes a row in the caller's commit. False when its key already has one,
/// which is how settling twice or raising an escalation twice in a day is one
/// interruption.
pub(super) fn enqueue(tx: &Connection, draft: &Draft, now: i64) -> Result<bool, StoreError> {
    let target = serde_json::to_string(&draft.target).expect("a target serializes");
    let written = tx.execute(
        "INSERT OR IGNORE INTO outbox (id,kind,destination,dedup_key,group_id,agent_id,run_id,
                                       ref_id,title,body,target,next_attempt_at,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?12)",
        params![
            uuid::Uuid::new_v4().to_string(),
            draft.kind.as_str(),
            CLIENTS,
            draft.dedup_key,
            draft.group_id.map(|id| id.to_string()),
            draft.agent_id.map(|id| id.to_string()),
            draft.run_id.map(|id| id.to_string()),
            draft.ref_id,
            draft.title,
            draft.body,
            target,
            now,
        ],
    )?;
    Ok(written == 1)
}

/// What a row calls an agent: its name now. None for one that is gone.
pub(super) fn name_of(tx: &Connection, agent: AgentId) -> Result<Option<String>, StoreError> {
    Ok(tx
        .query_row(
            "SELECT name FROM agents WHERE id=?1 AND lifecycle<>'terminated'",
            [agent.to_string()],
            |row| row.get(0),
        )
        .optional()?)
}

/// Withdraws what was waiting to announce a request or an escalation that has
/// just been answered, cleared or closed, in the commit that did it. A window
/// that claims it afterward is told it lost, which is what keeps a banner
/// from asking about something already settled in another window.
pub(super) fn withdraw_ref(tx: &Connection, ref_id: &str) -> Result<usize, StoreError> {
    Ok(tx.execute(
        "UPDATE outbox SET status='suppressed', last_error=?2 WHERE status='pending' AND ref_id=?1",
        params![ref_id, Why::Answered.as_str()],
    )?)
}

/// Withdraws what was waiting to announce runs the operator has now seen.
/// `runs` is a JSON list of run ids.
pub(super) fn withdraw_runs(tx: &Connection, runs: &str) -> Result<usize, StoreError> {
    Ok(tx.execute(
        "UPDATE outbox SET status='suppressed', last_error=?2
          WHERE status='pending' AND kind IN ('outcome','failure')
            AND run_id IN (SELECT value FROM json_each(?1))",
        params![runs, Why::Seen.as_str()],
    )?)
}

/// A row the dispatcher has to look at.
#[derive(Debug, Clone, PartialEq)]
pub struct Due {
    pub id: String,
    pub kind: Kind,
    pub agent_id: Option<AgentId>,
    /// How the run ended, for a row about a run.
    pub ended: Option<Ended>,
    /// How many times the dispatcher has acted on it: held it, gathered it,
    /// offered it. Zero is a row written since the last pass.
    pub attempts: u32,
    /// The gathering it is in, if it is in one.
    pub batch_id: Option<String>,
    pub title: String,
    pub body: String,
    pub target: Target,
    pub created_at: i64,
}

impl Due {
    pub fn gathered(&self) -> Gathered {
        Gathered { title: self.title.clone(), body: self.body.clone(), target: self.target }
    }
}

const DUE: &str = "o.id,o.kind,o.agent_id,c.status,o.attempts,o.batch_id,o.title,o.body,\
     o.target,o.created_at";

const FROM_DUE: &str = "FROM outbox o
     LEFT JOIN outcomes c ON c.run_id=o.run_id AND o.kind IN ('outcome','failure')
     WHERE o.status='pending' AND o.offered_at IS NULL AND o.destination='clients'";

fn corrupt(what: &str, e: impl std::fmt::Display) -> StoreError {
    StoreError::Corrupt(format!("{what}: {e}"))
}

fn read_due(row: &Row<'_>) -> rusqlite::Result<Result<Due, StoreError>> {
    let id: String = row.get(0)?;
    let kind: String = row.get(1)?;
    let agent: Option<String> = row.get(2)?;
    let ended: Option<String> = row.get(3)?;
    let attempts: u32 = row.get(4)?;
    let batch_id: Option<String> = row.get(5)?;
    let title: String = row.get(6)?;
    let body: String = row.get(7)?;
    let target: String = row.get(8)?;
    let created_at: i64 = row.get(9)?;
    Ok((|| {
        Ok(Due {
            id,
            kind: Kind::parse(&kind).ok_or_else(|| corrupt("unknown kind", &kind))?,
            agent_id: agent
                .map(|id| id.parse())
                .transpose()
                .map_err(|e| corrupt("bad agent", e))?,
            ended: ended
                .map(|status| {
                    Ended::parse(&status).ok_or_else(|| corrupt("unknown status", status))
                })
                .transpose()?,
            attempts,
            batch_id,
            title,
            body,
            target: serde_json::from_str(&target).map_err(|e| corrupt("bad target", e))?,
            created_at,
        })
    })())
}

fn read_delivery(row: &Row<'_>) -> rusqlite::Result<Result<Delivery, StoreError>> {
    let id: String = row.get(0)?;
    let kind: String = row.get(1)?;
    let title: String = row.get(2)?;
    let body: String = row.get(3)?;
    let target: String = row.get(4)?;
    let created_at: i64 = row.get(5)?;
    Ok((|| {
        Ok(Delivery {
            id,
            kind: Kind::parse(&kind).ok_or_else(|| corrupt("unknown kind", &kind))?,
            title,
            body,
            target: serde_json::from_str(&target).map_err(|e| corrupt("bad target", e))?,
            created_at,
        })
    })())
}

fn collect<T>(
    rows: impl Iterator<Item = rusqlite::Result<Result<T, StoreError>>>,
) -> Result<Vec<T>, StoreError> {
    let mut out = Vec::new();
    for row in rows {
        out.push(row??);
    }
    Ok(out)
}

impl Store {
    /// Rows whose time has come and that nobody has been offered yet, oldest
    /// first: new ones, held ones, and the rows of every gathering that is over.
    pub fn due_deliveries(&self, now: i64) -> Result<Vec<Due>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {DUE} {FROM_DUE} AND o.next_attempt_at<=?1 ORDER BY o.created_at, o.id"
        ))?;
        let rows = stmt.query_map([now], read_due)?;
        collect(rows)
    }

    /// The rows still in one gathering, oldest first.
    pub fn gathering(&self, batch: &str) -> Result<Vec<Due>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {DUE} {FROM_DUE} AND o.batch_id=?1 ORDER BY o.created_at, o.id"
        ))?;
        let rows = stmt.query_map([batch], read_due)?;
        collect(rows)
    }

    /// When the dispatcher next has something to do, if it has anything.
    pub fn next_delivery_at(&self) -> Result<Option<i64>, StoreError> {
        Ok(self.conn()?.query_row(
            "SELECT min(next_attempt_at) FROM outbox
              WHERE status='pending' AND offered_at IS NULL AND destination='clients'",
            [],
            |row| row.get(0),
        )?)
    }

    /// Looks at a row again at `until`.
    pub fn hold_delivery(&self, id: &str, until: i64) -> Result<(), StoreError> {
        self.conn()?.execute(
            "UPDATE outbox SET attempts=attempts+1, next_attempt_at=?2
              WHERE id=?1 AND status='pending'",
            params![id, until],
        )?;
        Ok(())
    }

    /// Puts a row in the gathering that is open, or opens one that closes
    /// [`GATHER_MS`] from now. Returns when it closes.
    ///
    /// Immediate, because two rows arriving at once must find the same
    /// gathering rather than each open their own.
    pub fn gather_delivery(&self, id: &str, now: i64) -> Result<i64, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let open: Option<(String, i64)> = tx
            .query_row(
                "SELECT batch_id, next_attempt_at FROM outbox
                  WHERE status='pending' AND offered_at IS NULL AND destination='clients'
                    AND batch_id IS NOT NULL AND next_attempt_at>?1
                  ORDER BY next_attempt_at, batch_id LIMIT 1",
                [now],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (batch, closes) = open.unwrap_or_else(|| (id.to_string(), now + GATHER_MS));
        tx.execute(
            "UPDATE outbox SET attempts=attempts+1, batch_id=?2, next_attempt_at=?3
              WHERE id=?1 AND status='pending'",
            params![id, batch, closes],
        )?;
        tx.commit()?;
        Ok(closes)
    }

    /// Offers a row to every window, saying `item`. With `carried`, it is the
    /// one item for a gathering: the rows it carries, which may be none, are
    /// offered and claimed with it, and it is the row they all name.
    ///
    /// None when the row is no longer pending: answered or seen in the moment
    /// since it was read, and then there is nothing to offer.
    pub fn offer_delivery(
        &self,
        id: &str,
        carried: Option<&[String]>,
        item: &Gathered,
        now: i64,
    ) -> Result<Option<Delivery>, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let target = serde_json::to_string(&item.target).expect("a target serializes");
        let gathered = carried.is_some();
        let offered = tx.execute(
            "UPDATE outbox SET title=?2, body=?3, target=?4, offered_at=?5, attempts=attempts+1,
                    batch_id=CASE WHEN ?6 THEN id ELSE batch_id END
              WHERE id=?1 AND status='pending' AND offered_at IS NULL",
            params![id, item.title, item.body, target, now, gathered],
        )?;
        if offered == 0 {
            return Ok(None);
        }
        if let Some(carried) = carried.filter(|carried| !carried.is_empty()) {
            tx.execute(
                "UPDATE outbox SET offered_at=?3, attempts=attempts+1, batch_id=?1
                  WHERE status='pending' AND id IN (SELECT value FROM json_each(?2))",
                params![id, serde_json::to_string(carried).expect("ids serialize"), now],
            )?;
        }
        let delivery = tx.query_row(
            "SELECT id,kind,title,body,target,created_at FROM outbox WHERE id=?1",
            [id],
            read_delivery,
        )??;
        tx.commit()?;
        Ok(Some(delivery))
    }

    /// Decides against these rows, saying why.
    pub fn suppress_deliveries(&self, ids: &[String], why: Why) -> Result<(), StoreError> {
        if ids.is_empty() {
            return Ok(());
        }
        self.conn()?.execute(
            "UPDATE outbox SET status='suppressed', last_error=?2
              WHERE status='pending' AND id IN (SELECT value FROM json_each(?1))",
            params![serde_json::to_string(ids).expect("ids serialize"), why.as_str()],
        )?;
        Ok(())
    }

    /// Lets go of whatever nobody claimed in a day. Not lost: what it said is
    /// still in For you and in the channel it was about.
    pub fn expire_deliveries(&self, now: i64) -> Result<usize, StoreError> {
        Ok(self.conn()?.execute(
            "UPDATE outbox SET status='expired' WHERE status='pending' AND created_at<=?1",
            [now - EXPIRE_MS],
        )?)
    }

    /// Everything offered and not yet claimed, oldest first: what a window
    /// that has just connected asks for, since the offer was made to whoever
    /// was listening at the time, which may have been nobody.
    pub fn pending_deliveries(&self) -> Result<Vec<Delivery>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,kind,title,body,target,created_at FROM outbox
              WHERE status='pending' AND offered_at IS NOT NULL AND destination='clients'
                AND (batch_id IS NULL OR batch_id=id)
              ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map([], read_delivery)?;
        collect(rows)
    }

    /// Takes an offered row for the window that asks first, with every row it
    /// carries. True for that window and false for every other, which is the
    /// whole of how two windows show one banner once.
    pub fn claim_delivery(&self, id: &str, now: i64) -> Result<bool, StoreError> {
        let claimed = self.conn()?.execute(
            "UPDATE outbox SET status='delivered', delivered_at=?2
              WHERE status='pending' AND offered_at IS NOT NULL
                AND (batch_id=?1 OR (id=?1 AND batch_id IS NULL))",
            params![id, now],
        )?;
        Ok(claimed > 0)
    }
}
