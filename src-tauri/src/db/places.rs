use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::store::row_to_envelope;
use super::{Store, StoreError};
use crate::domain::envelope::Envelope;
use crate::domain::ids::{AgentId, ApprovalId, GroupId, MessageId, RunId};
use crate::domain::now_ms;
use crate::domain::place::{
    AdapterId, HeardKey, Outgoing, OutsideUser, PlaceKey, Posted, Spot, ThreadKey,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Landing {
    pub adapter: AdapterId,
    pub spot: Spot,
    pub heard: HeardKey,
    pub heard_at: i64,
    pub direct: bool,
    pub agent: AgentId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Linked {
    pub person: OutsideUser,
    pub place: PlaceKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bound {
    pub group: GroupId,
    pub agent: AgentId,
    pub direct: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceRun {
    pub run: RunId,
    pub adapter: AdapterId,
    pub spot: Spot,
    pub heard: HeardKey,
    pub heard_at: i64,
    pub direct: bool,
    pub message: MessageId,
    pub started_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayState {
    Queued,
    Sending,
    Sent,
    Failed,
    Doubt,
    Skipped,
}

impl RelayState {
    pub fn as_str(self) -> &'static str {
        match self {
            RelayState::Queued => "queued",
            RelayState::Sending => "sending",
            RelayState::Sent => "sent",
            RelayState::Failed => "failed",
            RelayState::Doubt => "doubt",
            RelayState::Skipped => "skipped",
        }
    }

    fn parse(raw: &str) -> Result<Self, StoreError> {
        Ok(match raw {
            "queued" => RelayState::Queued,
            "sending" => RelayState::Sending,
            "sent" => RelayState::Sent,
            "failed" => RelayState::Failed,
            "doubt" => RelayState::Doubt,
            "skipped" => RelayState::Skipped,
            other => return Err(StoreError::Corrupt(format!("unknown relay state {other:?}"))),
        })
    }

    pub fn open(self) -> bool {
        matches!(self, RelayState::Queued | RelayState::Sending)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayRow {
    pub seq: i64,
    pub key: String,
    pub adapter: AdapterId,
    pub spot: Spot,
    pub run: Option<RunId>,
    pub state: RelayState,
    pub item: Outgoing,
    pub posted: Option<Posted>,
    pub attempts: u32,
    pub next_at: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRelay<'a> {
    pub key: &'a str,
    pub adapter: &'a AdapterId,
    pub spot: &'a Spot,
    pub run: Option<RunId>,
    pub state: RelayState,
    pub item: &'a Outgoing,
}

const RELAY_COLUMNS: &str =
    "seq,key,adapter,place,thread,run_id,state,item,posted,attempts,next_at,last_error,created_at";

fn corrupt(what: &str) -> impl Fn(String) -> StoreError + '_ {
    move |err| StoreError::Corrupt(format!("bad {what}: {err}"))
}

fn relay_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<RelayRow, StoreError>> {
    let run: Option<String> = row.get(5)?;
    let state: String = row.get(6)?;
    let item: String = row.get(7)?;
    let posted: Option<String> = row.get(8)?;
    let thread: Option<String> = row.get(4)?;
    let parsed = (|| {
        Ok(RelayRow {
            seq: row.get(0)?,
            key: row.get(1)?,
            adapter: AdapterId(row.get(2)?),
            spot: Spot { place: PlaceKey(row.get(3)?), thread: thread.map(ThreadKey) },
            run: run
                .map(|raw| raw.parse::<RunId>().map_err(|e| corrupt("run id")(e.to_string())))
                .transpose()?,
            state: RelayState::parse(&state)?,
            item: serde_json::from_str(&item).map_err(|e| corrupt("relay item")(e.to_string()))?,
            posted: posted.map(Posted),
            attempts: row.get(9)?,
            next_at: row.get(10)?,
            last_error: row.get(11)?,
            created_at: row.get(12)?,
        })
    })();
    Ok(parsed)
}

pub(super) fn land(
    tx: &Connection,
    landing: &Landing,
    message: MessageId,
    run: RunId,
) -> Result<(), StoreError> {
    let thread = landing.spot.thread.as_ref().map(|thread| thread.0.as_str());
    tx.execute(
        "INSERT INTO place_runs (run_id,adapter,place,thread,heard_key,heard_at,direct,message_id,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            run.to_string(),
            landing.adapter.0,
            landing.spot.place.0,
            thread,
            landing.heard.0,
            landing.heard_at,
            landing.direct,
            message.to_string(),
            now_ms(),
        ],
    )?;
    if let Some(thread) = thread {
        tx.execute(
            "INSERT OR IGNORE INTO place_threads (adapter,place,thread,agent_id) VALUES (?1,?2,?3,?4)",
            params![landing.adapter.0, landing.spot.place.0, thread, landing.agent.to_string()],
        )?;
    }
    raise(tx, &landing.adapter, &landing.spot.place, &landing.heard)
}

fn raise(
    tx: &Connection,
    adapter: &AdapterId,
    place: &PlaceKey,
    heard: &HeardKey,
) -> Result<(), StoreError> {
    tx.execute(
        "UPDATE places SET high_water=?3 WHERE adapter=?1 AND place=?2",
        params![adapter.0, place.0, heard.0],
    )?;
    Ok(())
}

impl Store {
    pub fn linked(&self, adapter: &AdapterId) -> Result<Option<Linked>, StoreError> {
        let conn = self.conn()?;
        let found: Option<(Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT person,linked_place FROM adapters WHERE id=?1",
                [&adapter.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(match found {
            Some((Some(person), Some(place))) => {
                Some(Linked { person: OutsideUser(person), place: PlaceKey(place) })
            }
            _ => None,
        })
    }

    pub fn link_person(
        &self,
        adapter: &AdapterId,
        person: &OutsideUser,
        place: &PlaceKey,
        group: GroupId,
        agent: AgentId,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO adapters (id,person,linked_place) VALUES (?1,?2,?3)
             ON CONFLICT(id) DO UPDATE SET person=excluded.person, linked_place=excluded.linked_place",
            params![adapter.0, person.0, place.0],
        )?;
        bind(&tx, adapter, place, group, agent, true)?;
        tx.commit()?;
        Ok(())
    }

    pub fn bind_place(
        &self,
        adapter: &AdapterId,
        place: &PlaceKey,
        group: GroupId,
        agent: AgentId,
        direct: bool,
    ) -> Result<(), StoreError> {
        let conn = self.conn()?;
        bind(&conn, adapter, place, group, agent, direct)
    }

    pub fn bound(
        &self,
        adapter: &AdapterId,
        place: &PlaceKey,
    ) -> Result<Option<Bound>, StoreError> {
        let conn = self.conn()?;
        let found: Option<(String, String, bool)> = conn
            .query_row(
                "SELECT group_id,agent_id,direct FROM places WHERE adapter=?1 AND place=?2",
                params![adapter.0, place.0],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        found
            .map(|(group, agent, direct)| {
                Ok(Bound {
                    group: group
                        .parse()
                        .map_err(|e: uuid::Error| corrupt("group id")(e.to_string()))?,
                    agent: agent
                        .parse()
                        .map_err(|e: uuid::Error| corrupt("agent id")(e.to_string()))?,
                    direct,
                })
            })
            .transpose()
    }

    pub fn bound_places(
        &self,
        adapter: &AdapterId,
    ) -> Result<Vec<(PlaceKey, Option<HeardKey>)>, StoreError> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT place,high_water FROM places WHERE adapter=?1 ORDER BY place")?;
        let rows = stmt.query_map([&adapter.0], |row| {
            Ok((PlaceKey(row.get(0)?), row.get::<_, Option<String>>(1)?.map(HeardKey)))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn high_water(
        &self,
        adapter: &AdapterId,
        place: &PlaceKey,
    ) -> Result<Option<HeardKey>, StoreError> {
        let conn = self.conn()?;
        let found: Option<Option<String>> = conn
            .query_row(
                "SELECT high_water FROM places WHERE adapter=?1 AND place=?2",
                params![adapter.0, place.0],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.flatten().map(HeardKey))
    }

    pub fn raise_high_water(
        &self,
        adapter: &AdapterId,
        place: &PlaceKey,
        heard: &HeardKey,
    ) -> Result<(), StoreError> {
        let conn = self.conn()?;
        raise(&conn, adapter, place, heard)
    }

    pub fn thread_agent(
        &self,
        adapter: &AdapterId,
        spot: &Spot,
    ) -> Result<Option<AgentId>, StoreError> {
        let Some(thread) = &spot.thread else { return Ok(None) };
        let conn = self.conn()?;
        let found: Option<String> = conn
            .query_row(
                "SELECT agent_id FROM place_threads WHERE adapter=?1 AND place=?2 AND thread=?3",
                params![adapter.0, spot.place.0, thread.0],
                |row| row.get(0),
            )
            .optional()?;
        found
            .map(|raw| raw.parse().map_err(|e: uuid::Error| corrupt("agent id")(e.to_string())))
            .transpose()
    }

    pub fn heard_before(&self, key: &str) -> Result<bool, StoreError> {
        let conn = self.conn()?;
        let found: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM operator_sends WHERE key=?1)
                 OR EXISTS(SELECT 1 FROM relay WHERE key=?1)",
            [key],
            |row| row.get(0),
        )?;
        Ok(found)
    }

    pub fn place_answer(
        &self,
        adapter: &AdapterId,
        heard: &HeardKey,
    ) -> Result<Option<(ApprovalId, String)>, StoreError> {
        let conn = self.conn()?;
        let found: Option<(String, String)> = conn
            .query_row(
                "SELECT approval_id,answer FROM place_answers WHERE adapter=?1 AND heard_key=?2",
                params![adapter.0, heard.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        found
            .map(|(id, answer)| {
                Ok((
                    id.parse().map_err(|e: uuid::Error| corrupt("approval id")(e.to_string()))?,
                    answer,
                ))
            })
            .transpose()
    }

    pub fn record_place_answer(
        &self,
        adapter: &AdapterId,
        place: &PlaceKey,
        heard: &HeardKey,
        approval: ApprovalId,
        answer: &str,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO place_answers (adapter,heard_key,approval_id,answer) VALUES (?1,?2,?3,?4)",
            params![adapter.0, heard.0, approval.to_string(), answer],
        )?;
        raise(&tx, adapter, place, heard)?;
        tx.commit()?;
        Ok(())
    }

    pub fn place_run(&self, run: RunId) -> Result<Option<PlaceRun>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("{PLACE_RUN_SELECT} WHERE p.run_id=?1"))?;
        let found = stmt.query_row([run.to_string()], place_run_row).optional()?;
        found.transpose()
    }

    pub fn open_place_runs(&self, adapter: &AdapterId) -> Result<Vec<PlaceRun>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "{PLACE_RUN_SELECT} WHERE p.adapter=?1 AND p.relayed=0 ORDER BY p.created_at"
        ))?;
        let rows = stmt.query_map([&adapter.0], place_run_row)?;
        rows.map(|row| row?).collect()
    }

    pub fn finish_place_run(&self, run: RunId) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute("UPDATE place_runs SET relayed=1 WHERE run_id=?1", [run.to_string()])?;
        Ok(())
    }

    pub fn replies_in_run(&self, run: RunId) -> Result<Vec<Envelope>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,intent,cause,created_at
               FROM messages WHERE run_id=?1 AND from_kind='agent' AND to_kind='human'
              ORDER BY created_at, rowid",
        )?;
        let rows = stmt.query_map([run.to_string()], row_to_envelope)?;
        rows.map(|row| row?).collect()
    }

    pub fn approvals_in_run(&self, run: RunId) -> Result<Vec<ApprovalId>, StoreError> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT id FROM approvals WHERE run_id=?1 ORDER BY created_at")?;
        let rows = stmt.query_map([run.to_string()], |row| row.get::<_, String>(0))?;
        rows.map(|row| row?.parse().map_err(|e: uuid::Error| corrupt("approval id")(e.to_string())))
            .collect()
    }

    pub fn queue_place_notice(
        &self,
        new: &NewRelay<'_>,
        heard: &HeardKey,
    ) -> Result<bool, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let written = insert_relay(&tx, new, true)?;
        raise(&tx, new.adapter, &new.spot.place, heard)?;
        tx.commit()?;
        Ok(written)
    }

    pub fn insert_relay(&self, new: &NewRelay<'_>) -> Result<bool, StoreError> {
        let conn = self.conn()?;
        insert_relay(&conn, new, false)
    }

    pub fn relay_rows(&self, adapter: &AdapterId) -> Result<Vec<RelayRow>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {RELAY_COLUMNS} FROM relay
              WHERE adapter=?1
                AND (state IN ('queued','sending')
                     OR run_id IN (SELECT run_id FROM place_runs WHERE adapter=?1 AND relayed=0))
              ORDER BY seq"
        ))?;
        let rows = stmt.query_map([&adapter.0], relay_row)?;
        rows.map(|row| row?).collect()
    }

    pub fn relay_row(&self, key: &str) -> Result<Option<RelayRow>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("SELECT {RELAY_COLUMNS} FROM relay WHERE key=?1"))?;
        stmt.query_row([key], relay_row).optional()?.transpose()
    }

    pub fn latest_ask(
        &self,
        adapter: &AdapterId,
        spot: &Spot,
    ) -> Result<Option<ApprovalId>, StoreError> {
        let conn = self.conn()?;
        let found: Option<String> = conn
            .query_row(
                "SELECT substr(key,5) FROM relay
                  WHERE adapter=?1 AND place=?2 AND key LIKE 'ask:%'
                    AND (thread IS NULL OR thread IS ?3)
                    AND state IN ('sent','doubt','sending','queued')
                  ORDER BY seq DESC LIMIT 1",
                params![adapter.0, spot.place.0, spot.thread.as_ref().map(|t| t.0.as_str())],
                |row| row.get(0),
            )
            .optional()?;
        found
            .map(|raw| raw.parse().map_err(|e: uuid::Error| corrupt("approval id")(e.to_string())))
            .transpose()
    }

    pub fn relay_to(&self, key: &str, state: RelayState) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute("UPDATE relay SET state=?2 WHERE key=?1", params![key, state.as_str()])?;
        Ok(())
    }

    pub fn relay_sent(&self, key: &str, posted: Option<&Posted>) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE relay SET state='sent', posted=?2, attempts=attempts+1 WHERE key=?1",
            params![key, posted.map(|p| p.0.as_str())],
        )?;
        Ok(())
    }

    pub fn relay_later(&self, key: &str, next_at: i64, error: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE relay SET state='queued', attempts=attempts+1, next_at=?2, last_error=?3 WHERE key=?1",
            params![key, next_at, error],
        )?;
        Ok(())
    }

    pub fn relay_failed(&self, key: &str, error: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE relay SET state='failed', attempts=attempts+1, last_error=?2 WHERE key=?1",
            params![key, error],
        )?;
        Ok(())
    }

    pub fn relay_hold(&self, key: &str, next_at: i64, error: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE relay SET attempts=attempts+1, next_at=?2, last_error=?3 WHERE key=?1",
            params![key, next_at, error],
        )?;
        Ok(())
    }

    pub fn relay_doubt(&self, key: &str, error: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE relay SET state='doubt', last_error=?2 WHERE key=?1",
            params![key, error],
        )?;
        Ok(())
    }

    pub fn relay_next_at(&self, adapter: &AdapterId) -> Result<Option<i64>, StoreError> {
        let conn = self.conn()?;
        Ok(conn.query_row(
            "SELECT min(next_at) FROM relay WHERE adapter=?1 AND state IN ('queued','sending')",
            [&adapter.0],
            |row| row.get(0),
        )?)
    }
}

const PLACE_RUN_SELECT: &str =
    "SELECT p.run_id,p.adapter,p.place,p.thread,p.heard_key,p.heard_at,p.direct,p.message_id,
            r.started_at,p.created_at
       FROM place_runs p LEFT JOIN pending_runs r ON r.run_id=p.run_id";

fn place_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<PlaceRun, StoreError>> {
    let run: String = row.get(0)?;
    let message: String = row.get(7)?;
    let thread: Option<String> = row.get(3)?;
    let adapter = AdapterId(row.get(1)?);
    let place = PlaceKey(row.get(2)?);
    let heard = HeardKey(row.get(4)?);
    let heard_at: i64 = row.get(5)?;
    let direct: bool = row.get(6)?;
    let started_at: Option<i64> = row.get(8)?;
    let created_at: i64 = row.get(9)?;
    Ok((|| {
        Ok(PlaceRun {
            run: run.parse().map_err(|e: uuid::Error| corrupt("run id")(e.to_string()))?,
            adapter,
            spot: Spot { place, thread: thread.map(ThreadKey) },
            heard,
            heard_at,
            direct,
            message: message
                .parse()
                .map_err(|e: uuid::Error| corrupt("message id")(e.to_string()))?,
            started_at,
            created_at,
        })
    })())
}

fn bind(
    conn: &Connection,
    adapter: &AdapterId,
    place: &PlaceKey,
    group: GroupId,
    agent: AgentId,
    direct: bool,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO places (adapter,place,group_id,agent_id,direct) VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(adapter,place) DO UPDATE SET group_id=excluded.group_id,
             agent_id=excluded.agent_id, direct=excluded.direct",
        params![adapter.0, place.0, group.to_string(), agent.to_string(), direct],
    )?;
    Ok(())
}

fn insert_relay(conn: &Connection, new: &NewRelay<'_>, ignore: bool) -> Result<bool, StoreError> {
    let item = serde_json::to_string(new.item).expect("a relay item serializes");
    let verb = if ignore { "INSERT OR IGNORE" } else { "INSERT" };
    let written = conn.execute(
        &format!(
            "{verb} INTO relay (key,adapter,place,thread,run_id,state,item,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)"
        ),
        params![
            new.key,
            new.adapter.0,
            new.spot.place.0,
            new.spot.thread.as_ref().map(|t| t.0.as_str()),
            new.run.map(|run| run.to_string()),
            new.state.as_str(),
            item,
            now_ms(),
        ],
    )?;
    Ok(written > 0)
}
