use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};

use super::{Store, StoreError};
use crate::domain::ids::AgentId;
use crate::domain::outbox::{
    Delivery, Destination, Draft, Gathered, Kind, Target, Why, EXPIRE_MS, GATHER_MS,
};
use crate::domain::outcome::Status as Ended;
use crate::push::{Next, Push, Report, Said};

const RELEASES: u32 = 2;

const CLIENTS: &str = Destination::Clients.as_str();

const ENDPOINT: &str = Destination::Endpoint.as_str();

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

pub(super) fn name_of(tx: &Connection, agent: AgentId) -> Result<Option<String>, StoreError> {
    Ok(tx
        .query_row(
            "SELECT name FROM agents WHERE id=?1 AND lifecycle<>'terminated'",
            [agent.to_string()],
            |row| row.get(0),
        )
        .optional()?)
}

pub(super) fn withdraw_ref(tx: &Connection, ref_id: &str) -> Result<usize, StoreError> {
    Ok(tx.execute(
        "UPDATE outbox SET status='suppressed', last_error=?2 WHERE status='pending' AND ref_id=?1",
        params![ref_id, Why::Answered.as_str()],
    )?)
}

pub(super) fn withdraw_runs(tx: &Connection, runs: &str) -> Result<usize, StoreError> {
    Ok(tx.execute(
        "UPDATE outbox SET status='suppressed', last_error=?2
          WHERE status='pending' AND kind IN ('outcome','failure')
            AND run_id IN (SELECT value FROM json_each(?1))",
        params![runs, Why::Seen.as_str()],
    )?)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Due {
    pub id: String,
    pub kind: Kind,
    pub agent_id: Option<AgentId>,
    pub ended: Option<Ended>,
    pub attempts: u32,
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

fn read_push(row: &Row<'_>) -> rusqlite::Result<Result<(Push, u32, bool), StoreError>> {
    let kind: String = row.get(1)?;
    let status: Option<String> = row.get(6)?;
    let push = Push {
        id: row.get(0)?,
        kind: Kind::Outcome,
        title: row.get(2)?,
        body: row.get(3)?,
        agent: row.get(4)?,
        group: row.get(5)?,
        status: status.as_deref().and_then(Ended::parse),
        created_at: row.get(7)?,
    };
    let attempts: u32 = row.get(8)?;
    let given_back: bool = row.get(9)?;
    Ok(match Kind::parse(&kind) {
        Some(kind) => Ok((Push { kind, ..push }, attempts, given_back)),
        None => Err(corrupt("unknown kind", &kind)),
    })
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
    pub fn due_deliveries(&self, now: i64) -> Result<Vec<Due>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {DUE} {FROM_DUE} AND o.next_attempt_at<=?1 ORDER BY o.created_at, o.id"
        ))?;
        let rows = stmt.query_map([now], read_due)?;
        collect(rows)
    }

    pub fn gathering(&self, batch: &str) -> Result<Vec<Due>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {DUE} {FROM_DUE} AND o.batch_id=?1 ORDER BY o.created_at, o.id"
        ))?;
        let rows = stmt.query_map([batch], read_due)?;
        collect(rows)
    }

    pub fn next_delivery_at(&self) -> Result<Option<i64>, StoreError> {
        Ok(self.conn()?.query_row(
            "SELECT min(next_attempt_at) FROM outbox
              WHERE status='pending'
                AND ((destination='clients' AND (offered_at IS NULL OR releases>0))
                     OR destination='endpoint')",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn hold_delivery(&self, id: &str, until: i64) -> Result<(), StoreError> {
        self.conn()?.execute(
            "UPDATE outbox SET attempts=attempts+1, next_attempt_at=?2
              WHERE id=?1 AND status='pending'",
            params![id, until],
        )?;
        Ok(())
    }

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
        let (batch, closes) =
            open.unwrap_or_else(|| (id.to_string(), now + crate::probe_clock::scaled(GATHER_MS)));
        tx.execute(
            "UPDATE outbox SET attempts=attempts+1, batch_id=?2, next_attempt_at=?3
              WHERE id=?1 AND status='pending'",
            params![id, batch, closes],
        )?;
        tx.commit()?;
        Ok(closes)
    }

    pub fn offer_delivery(
        &self,
        id: &str,
        carried: Option<&[String]>,
        item: &Gathered,
        push: Option<&Said>,
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
        if let Some(said) = push {
            let single = carried.is_none_or(<[String]>::is_empty);
            tx.execute(
                "INSERT OR IGNORE INTO outbox (id,kind,destination,dedup_key,group_id,agent_id,
                                               run_id,ref_id,title,body,target,next_attempt_at,
                                               created_at)
                 SELECT ?2,kind,?3,dedup_key,
                        CASE WHEN ?4 THEN group_id END,CASE WHEN ?4 THEN agent_id END,
                        CASE WHEN ?4 THEN run_id END,CASE WHEN ?4 THEN ref_id END,
                        ?5,?6,target,?7,?7
                   FROM outbox WHERE id=?1",
                params![
                    id,
                    uuid::Uuid::new_v4().to_string(),
                    ENDPOINT,
                    single,
                    said.title,
                    said.body,
                    now
                ],
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

    pub fn expire_deliveries(&self, now: i64) -> Result<usize, StoreError> {
        Ok(self.conn()?.execute(
            "UPDATE outbox SET status='expired' WHERE status='pending' AND created_at<=?1",
            [now - EXPIRE_MS],
        )?)
    }

    pub fn pending_deliveries(&self, page: Option<&str>) -> Result<Vec<Delivery>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,kind,title,body,target,created_at FROM outbox
              WHERE status='pending' AND offered_at IS NOT NULL AND destination='clients'
                AND (batch_id IS NULL OR batch_id=id)
                AND releases<?2 AND (released_by IS NULL OR released_by IS NOT ?1)
              ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map(params![page, RELEASES], read_delivery)?;
        collect(rows)
    }

    pub fn claim_delivery(
        &self,
        id: &str,
        page: Option<&str>,
        now: i64,
    ) -> Result<bool, StoreError> {
        let claimed = self.conn()?.execute(
            "UPDATE outbox SET status='delivered', delivered_at=?2, claimed_by=?3
              WHERE status='pending' AND offered_at IS NOT NULL AND destination='clients'
                AND (batch_id=?1 OR (id=?1 AND batch_id IS NULL))
                AND releases<?4 AND (released_by IS NULL OR released_by IS NOT ?3)",
            params![id, now, page, RELEASES],
        )?;
        Ok(claimed > 0)
    }

    pub fn release_delivery(
        &self,
        id: &str,
        page: &str,
        reclaim: i64,
        now: i64,
    ) -> Result<Option<(Delivery, u32)>, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let released = tx.execute(
            "UPDATE outbox SET status='pending', delivered_at=NULL, claimed_by=NULL,
                    released_by=?2, releases=releases+1, offered_at=?3,
                    next_attempt_at=CASE WHEN releases+1<?5 THEN ?3+?4 ELSE ?3 END
              WHERE status='delivered' AND destination='clients' AND claimed_by=?2
                AND (batch_id=?1 OR (id=?1 AND batch_id IS NULL))",
            params![id, page, now, reclaim, RELEASES],
        )?;
        if released == 0 {
            return Ok(None);
        }
        let given = tx.query_row(
            "SELECT id,kind,title,body,target,created_at FROM outbox WHERE id=?1",
            [id],
            read_delivery,
        )??;
        let releases: u32 =
            tx.query_row("SELECT releases FROM outbox WHERE id=?1", [id], |row| row.get(0))?;
        tx.commit()?;
        Ok(Some((given, releases)))
    }

    pub fn due_releases(&self, now: i64) -> Result<Vec<(Due, usize)>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {DUE},
                    (SELECT count(*) FROM outbox b WHERE b.batch_id=o.id AND b.destination='clients')
               FROM outbox o
               LEFT JOIN outcomes c ON c.run_id=o.run_id AND o.kind IN ('outcome','failure')
              WHERE o.status='pending' AND o.destination='clients' AND o.releases>0
                AND o.offered_at IS NOT NULL AND (o.batch_id IS NULL OR o.batch_id=o.id)
                AND o.next_attempt_at<=?1
              ORDER BY o.created_at, o.id"
        ))?;
        let rows = stmt.query_map([now], |row| {
            let count: usize = row.get(10)?;
            Ok(read_due(row)?.map(|due| (due, count.max(1))))
        })?;
        collect(rows)
    }

    pub fn settle_release(
        &self,
        id: &str,
        push: Option<&Said>,
        single: bool,
        expires: i64,
        now: i64,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let settled = tx.execute(
            "UPDATE outbox SET next_attempt_at=?2
              WHERE status='pending' AND destination='clients' AND releases>0
                AND next_attempt_at<=?3 AND (batch_id=?1 OR (id=?1 AND batch_id IS NULL))",
            params![id, expires, now],
        )?;
        if settled > 0 {
            if let Some(said) = push {
                tx.execute(
                    "INSERT INTO outbox (id,kind,destination,dedup_key,group_id,agent_id,run_id,
                                         ref_id,title,body,target,next_attempt_at,created_at)
                     SELECT ?2,kind,?3,dedup_key,
                            CASE WHEN ?4 THEN group_id END,CASE WHEN ?4 THEN agent_id END,
                            CASE WHEN ?4 THEN run_id END,CASE WHEN ?4 THEN ref_id END,
                            ?5,?6,target,?7,?7
                       FROM outbox WHERE id=?1
                     ON CONFLICT (dedup_key, destination) DO UPDATE
                        SET status='pending', last_error=NULL,
                            next_attempt_at=excluded.next_attempt_at
                      WHERE outbox.status='suppressed' AND outbox.last_error=?8",
                    params![
                        id,
                        uuid::Uuid::new_v4().to_string(),
                        ENDPOINT,
                        single,
                        said.title,
                        said.body,
                        now,
                        Why::Present.as_str()
                    ],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn due_pushes(&self, now: i64) -> Result<Vec<(Push, u32, bool)>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT o.id,o.kind,o.title,o.body,a.name,g.name,c.status,o.created_at,o.attempts,
                    EXISTS (SELECT 1 FROM outbox w
                             WHERE w.dedup_key=o.dedup_key AND w.destination='clients'
                               AND w.releases>0)
               FROM outbox o
               LEFT JOIN agents a ON a.id=o.agent_id
               LEFT JOIN groups g ON g.id=o.group_id
               LEFT JOIN outcomes c ON c.run_id=o.run_id AND o.kind IN ('outcome','failure')
              WHERE o.status='pending' AND o.destination='endpoint' AND o.next_attempt_at<=?1
              ORDER BY o.created_at, o.id",
        )?;
        let rows = stmt.query_map([now], read_push)?;
        collect(rows)
    }

    pub fn record_push(&self, id: &str, next: &Next, now: i64) -> Result<(), StoreError> {
        let (status, at, error) = match next {
            Next::Sent => ("delivered", None, None),
            Next::Again { at, error } => ("pending", Some(*at), Some(error.as_str())),
            Next::Failed { error } => ("failed", None, Some(error.as_str())),
        };
        self.conn()?.execute(
            "UPDATE outbox SET status=?2, attempts=attempts+1, offered_at=?3,
                    next_attempt_at=coalesce(?4, next_attempt_at), last_error=?5,
                    delivered_at=CASE WHEN ?2='delivered' THEN ?3 ELSE delivered_at END
              WHERE id=?1 AND status='pending' AND destination='endpoint'",
            params![id, status, now, at, error],
        )?;
        Ok(())
    }

    pub fn push_report(&self) -> Result<Report, StoreError> {
        let conn = self.conn()?;
        let last_sent_at: Option<i64> = conn.query_row(
            "SELECT max(delivered_at) FROM outbox
              WHERE destination='endpoint' AND status='delivered'",
            [],
            |row| row.get(0),
        )?;
        let last: Option<(i64, String, Option<String>, i64)> = conn
            .query_row(
                "SELECT offered_at,status,last_error,next_attempt_at FROM outbox
                  WHERE destination='endpoint' AND offered_at IS NOT NULL
                    AND status IN ('pending','delivered','failed')
                  ORDER BY offered_at DESC, rowid DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((attempted, status, error, next)) = last else {
            return Ok(Report { last_sent_at, ..Report::default() });
        };
        Ok(Report {
            last_sent_at,
            last_attempt_at: Some(attempted),
            last_error: if status == "delivered" { None } else { error },
            retry_at: (status == "pending").then_some(next),
        })
    }
}
