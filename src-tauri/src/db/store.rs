//! SQLite-backed persistence.
//!
//! One pool, WAL mode, plain SQL. There is no ORM because there are two tables
//! and eleven queries, and hiding those behind a query builder would add a
//! dependency and remove the ability to read what actually hits the disk.

use std::collections::HashMap;
use std::path::Path;

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, OptionalExtension, Row};

use crate::db::migrations;
use crate::domain::agent::{AgentCard, CleanDraft, Lifecycle};
use crate::domain::envelope::{Envelope, Part, Participant, Trust};
use crate::domain::group::{CleanGroup, Group, GroupInference};
use crate::domain::ids::{AgentId, GroupId, MessageId, RoutineId, RunId};
use crate::domain::now_ms;
use crate::domain::routine::Routine;
use crate::domain::usage::{Tokens, UsageEntry};

pub type Conn = PooledConnection<SqliteConnectionManager>;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("connection pool error: {0}")]
    Pool(#[from] r2d2::Error),
    #[error("migration error: {0}")]
    Migration(#[from] migrations::MigrationError),
    #[error("stored row is malformed: {0}")]
    Corrupt(String),
    #[error("no agent with id {0}")]
    AgentNotFound(AgentId),
    #[error("an agent named {0:?} already exists")]
    DuplicateName(String),
    #[error("no group with id {0}")]
    GroupNotFound(GroupId),
    #[error("{name:?} still has {agents} agent(s); move or delete them before deleting the group")]
    GroupNotEmpty { name: String, agents: u32 },
    #[error("every agent has to be in a group, so the first one cannot be deleted")]
    CannotDeleteDefaultGroup,
    #[error("no routine with id {0}")]
    RoutineNotFound(RoutineId),
}

/// Guards the one-time-per-file setup inside `Store::open`.
fn bootstrap_lock() -> &'static parking_lot::Mutex<()> {
    static LOCK: std::sync::OnceLock<parking_lot::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| parking_lot::Mutex::new(()))
}

/// The group an agent lands in when nothing says otherwise. Parsed from the
/// migration's pinned constant so the two can never drift apart.
fn default_group_id() -> GroupId {
    migrations::DEFAULT_GROUP_ID.parse().expect("the pinned default group id is a valid uuid")
}

/// Maps SQLite's unique-constraint failure onto a domain error, so callers get
/// "that name is taken" instead of a raw driver string.
fn classify(err: rusqlite::Error, name: &str) -> StoreError {
    if let rusqlite::Error::SqliteFailure(inner, _) = &err {
        if inner.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE {
            return StoreError::DuplicateName(name.to_string());
        }
    }
    StoreError::Sqlite(err)
}

#[derive(Debug, Clone)]
pub struct Store {
    pool: Pool<SqliteConnectionManager>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Two kinds of setting, applied in two places.
        //
        // `journal_mode` belongs to the database file, not to a connection, and
        // switching it takes an exclusive lock. It has to happen once, alone,
        // before the pool exists. Letting the pool's connections race for that
        // lock logs "database is locked" and silently leaves some of them on
        // the rollback journal: a busy timeout does not save you, because
        // SQLite treats a shared/exclusive conflict inside a single process as
        // a deadlock and fails immediately rather than waiting.
        //
        // WAL is what lets the UI read a transcript while agents are mid-write.
        // Without it every read queues behind the writer and the window
        // stutters whenever a cascade is running.
        {
            // One thread at a time, process-wide. Both steps below take an
            // exclusive lock on the file, and SQLite reports a shared vs
            // exclusive conflict *inside one process* as a deadlock rather than
            // waiting, so a busy timeout does not save the losers. Racing here
            // used to fail on the journal_mode switch alone; once the migration
            // list grew long enough to hold the lock for a few milliseconds, the
            // same race started failing there too.
            let _serialised = bootstrap_lock().lock();

            let mut bootstrap = rusqlite::Connection::open(path)?;
            // Needed before the migration below, so a second process opening
            // the same database waits for the write lock rather than failing.
            bootstrap.busy_timeout(std::time::Duration::from_secs(5))?;
            let mode: String =
                bootstrap.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
            if !mode.eq_ignore_ascii_case("wal") {
                return Err(StoreError::Corrupt(format!(
                    "could not switch {} to WAL mode (got {mode:?})",
                    path.display()
                )));
            }
            migrations::run(&mut bootstrap)?;
        }

        // Everything below is per-connection and needs no exclusive lock, so it
        // is safe to run on all of them at once.
        let manager = SqliteConnectionManager::file(path).with_init(|conn| {
            conn.execute_batch(
                "PRAGMA busy_timeout = 5000;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA foreign_keys = ON;",
            )
        });

        Ok(Self { pool: Pool::builder().max_size(8).build(manager)? })
    }

    pub fn conn(&self) -> Result<Conn, StoreError> {
        Ok(self.pool.get()?)
    }

    // ---- agents ----------------------------------------------------------

    pub fn create_agent(&self, draft: &CleanDraft) -> Result<AgentCard, StoreError> {
        let conn = self.conn()?;
        let now = now_ms();
        let card = AgentCard {
            id: AgentId::new(),
            group_id: draft.group_id.unwrap_or_else(default_group_id),
            name: draft.name.clone(),
            avatar: draft.avatar.clone(),
            color: draft.color.clone(),
            model: draft.model.clone(),
            system_prompt: draft.system_prompt.clone(),
            skills: draft.skills.clone(),
            sandbox_id: None,
            sandbox_envd_token: None,
            sandbox_traffic_token: None,
            lifecycle: Lifecycle::Active,
            version: 1,
            created_at: now,
            updated_at: now,
        };

        conn.execute(
            "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                card.id.to_string(),
                card.name,
                card.avatar,
                card.color,
                card.model,
                card.system_prompt,
                serde_json::to_string(&card.skills).unwrap_or_else(|_| "[]".into()),
                card.lifecycle.as_str(),
                card.version,
                card.created_at,
                card.updated_at,
                card.group_id.to_string(),
            ],
        )
        .map_err(|e| classify(e, &card.name))?;

        Ok(card)
    }

    /// Applies an operator edit and bumps the card version.
    ///
    /// The version bump is what lets a peer notice the card changed under it,
    /// which is the only reason A2A's Update phase exists.
    pub fn update_agent(&self, id: AgentId, draft: &CleanDraft) -> Result<AgentCard, StoreError> {
        let conn = self.conn()?;
        let now = now_ms();
        let changed = conn
            .execute(
                // `coalesce` is what makes an omitted group mean "do not move
                // it" rather than "move it to the default", which would
                // silently relocate an agent on an unrelated edit.
                "UPDATE agents
                    SET name=?2, avatar=?3, color=?4, model=?5, system_prompt=?6, skills=?7,
                        version = version + 1, updated_at=?8,
                        group_id = coalesce(?9, group_id)
                  WHERE id=?1",
                params![
                    id.to_string(),
                    draft.name,
                    draft.avatar,
                    draft.color,
                    draft.model,
                    draft.system_prompt,
                    serde_json::to_string(&draft.skills).unwrap_or_else(|_| "[]".into()),
                    now,
                    draft.group_id.map(|g| g.to_string()),
                ],
            )
            .map_err(|e| classify(e, &draft.name))?;

        if changed == 0 {
            return Err(StoreError::AgentNotFound(id));
        }
        self.get_agent(id)?.ok_or(StoreError::AgentNotFound(id))
    }

    pub fn set_lifecycle(&self, id: AgentId, state: Lifecycle) -> Result<AgentCard, StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE agents SET lifecycle=?2, updated_at=?3 WHERE id=?1",
            params![id.to_string(), state.as_str(), now_ms()],
        )?;
        if changed == 0 {
            return Err(StoreError::AgentNotFound(id));
        }
        self.get_agent(id)?.ok_or(StoreError::AgentNotFound(id))
    }

    pub fn get_agent(&self, id: AgentId) -> Result<Option<AgentCard>, StoreError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id,sandbox_id,sandbox_envd_token,sandbox_traffic_token
               FROM agents WHERE id=?1",
            params![id.to_string()],
            row_to_card,
        )
        .optional()?
        .transpose()
    }

    /// Every agent, terminated ones included.
    ///
    /// Terminated agents still appear in old transcripts, so the frontend needs
    /// their name, avatar, and color to render history. It filters them out of
    /// the sidebar itself.
    pub fn list_agents(&self) -> Result<Vec<AgentCard>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id,sandbox_id,sandbox_envd_token,sandbox_traffic_token
               FROM agents ORDER BY rowid",
        )?;
        let rows = stmt.query_map([], row_to_card)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    /// Records which sandbox is this agent's computer.
    ///
    /// Deliberately not part of `update_agent`: provisioning is not an operator
    /// edit and must not bump the card version, which peers use to notice that
    /// an agent changed under them.
    pub fn set_agent_sandbox(
        &self,
        id: AgentId,
        sandbox: Option<(&str, &str, &str)>,
    ) -> Result<(), StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE agents SET sandbox_id=?2, sandbox_envd_token=?3, sandbox_traffic_token=?4
               WHERE id=?1",
            params![
                id.to_string(),
                sandbox.map(|s| s.0),
                sandbox.map(|s| s.1),
                sandbox.map(|s| s.2),
            ],
        )?;
        if changed == 0 {
            return Err(StoreError::AgentNotFound(id));
        }
        Ok(())
    }

    /// The traffic token for a sandbox, by sandbox id.
    ///
    /// The viewer proxy holds no state of its own: it is handed a sandbox in a
    /// URL and asks here, so a token never has to travel through the webview.
    pub fn sandbox_traffic_token(&self, sandbox: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                "SELECT sandbox_traffic_token FROM agents WHERE sandbox_id=?1",
                params![sandbox],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    // ---- routines --------------------------------------------------------

    pub fn create_routine(
        &self,
        agent: AgentId,
        what: &str,
        every_secs: Option<u32>,
        first_run_at: i64,
    ) -> Result<Routine, StoreError> {
        let conn = self.conn()?;
        let routine = Routine {
            id: RoutineId::new(),
            agent_id: agent,
            what: what.trim().to_string(),
            every_secs,
            next_run_at: first_run_at,
            last_run_at: None,
            created_at: now_ms(),
        };
        conn.execute(
            "INSERT INTO routines (id,agent_id,what,every_secs,next_run_at,created_at)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                routine.id.to_string(),
                agent.to_string(),
                routine.what,
                routine.every_secs,
                routine.next_run_at,
                routine.created_at,
            ],
        )?;
        Ok(routine)
    }

    pub fn get_routine(&self, id: RoutineId) -> Result<Option<Routine>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,agent_id,what,every_secs,next_run_at,last_run_at,created_at
               FROM routines WHERE id=?1",
        )?;
        match stmt.query_row(params![id.to_string()], row_to_routine).optional()? {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Rewrites a routine in place.
    ///
    /// `next_run_at` is passed rather than derived: an operator editing "every
    /// six hours" into "every hour" usually wants the next one in an hour, and
    /// an operator fixing a typo does not want the schedule to move at all.
    /// The caller knows which of the two it is doing.
    pub fn update_routine(
        &self,
        id: RoutineId,
        what: &str,
        every_secs: Option<u32>,
        next_run_at: i64,
    ) -> Result<Routine, StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE routines SET what=?2, every_secs=?3, next_run_at=?4 WHERE id=?1",
            params![id.to_string(), what.trim(), every_secs, next_run_at],
        )?;
        if changed == 0 {
            return Err(StoreError::RoutineNotFound(id));
        }

        let mut stmt = conn.prepare(
            "SELECT id,agent_id,what,every_secs,next_run_at,last_run_at,created_at
               FROM routines WHERE id=?1",
        )?;
        stmt.query_row(params![id.to_string()], row_to_routine)?
    }

    pub fn agent_routines(&self, agent: AgentId) -> Result<Vec<Routine>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,agent_id,what,every_secs,next_run_at,last_run_at,created_at
               FROM routines WHERE agent_id=?1 ORDER BY next_run_at",
        )?;
        let rows = stmt.query_map(params![agent.to_string()], row_to_routine)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    /// Everything due to run, oldest first.
    ///
    /// Only for agents that can still act: a routine belonging to a deleted or
    /// paused agent would otherwise fire into nothing, repeatedly.
    pub fn due_routines(&self, now: i64) -> Result<Vec<Routine>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT r.id,r.agent_id,r.what,r.every_secs,r.next_run_at,r.last_run_at,r.created_at
               FROM routines r
               JOIN agents a ON a.id = r.agent_id
              WHERE r.next_run_at <= ?1 AND a.lifecycle = 'active'
              ORDER BY r.next_run_at",
        )?;
        let rows = stmt.query_map(params![now], row_to_routine)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    /// Records that a routine ran, and when it is next due.
    ///
    /// A one-shot is removed rather than left with a time in the past, so the
    /// scheduler never has to reason about whether something already happened.
    pub fn routine_ran(&self, routine: &Routine, now: i64) -> Result<(), StoreError> {
        let conn = self.conn()?;
        match routine.after_running(now) {
            Some(next) => {
                conn.execute(
                    "UPDATE routines SET next_run_at=?2, last_run_at=?3 WHERE id=?1",
                    params![routine.id.to_string(), next, now],
                )?;
            }
            None => {
                conn.execute("DELETE FROM routines WHERE id=?1", params![routine.id.to_string()])?;
            }
        }
        Ok(())
    }

    pub fn delete_routine(&self, id: RoutineId) -> Result<bool, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute("DELETE FROM routines WHERE id=?1", params![id.to_string()])? > 0)
    }

    /// Removes an agent's schedule along with the agent.
    pub fn delete_agent_routines(&self, agent: AgentId) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute("DELETE FROM routines WHERE agent_id=?1", params![agent.to_string()])?)
    }

    // ---- groups ----------------------------------------------------------

    /// Every group, with its live agent count, oldest first.
    ///
    /// The default group sorts first because it was created at timestamp 0 by
    /// the migration, which is what keeps it at the top of the rail.
    pub fn list_groups(&self) -> Result<Vec<Group>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT g.id, g.name, g.created_at,
                    (SELECT count(*) FROM agents a
                      WHERE a.group_id = g.id AND a.lifecycle <> 'terminated'),
                    g.base_url, g.api_key, g.default_model
               FROM groups g
              ORDER BY g.created_at, g.rowid",
        )?;
        let rows = stmt.query_map([], row_to_group)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    pub fn create_group(&self, draft: &CleanGroup) -> Result<Group, StoreError> {
        let conn = self.conn()?;
        let id = GroupId::new();
        conn.execute(
            "INSERT INTO groups (id,name,created_at,base_url,api_key,default_model)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                id.to_string(),
                draft.name,
                now_ms(),
                draft.base_url.clone().flatten(),
                draft.api_key.clone().flatten(),
                draft.default_model.clone().flatten(),
            ],
        )
        .map_err(|e| classify(e, &draft.name))?;
        self.get_group(id)?.ok_or(StoreError::GroupNotFound(id))
    }

    /// Applies an operator edit. An override the draft does not mention is left
    /// exactly as it was, which is what lets the UI show a redacted key without
    /// erasing it on the next save.
    pub fn update_group(&self, id: GroupId, draft: &CleanGroup) -> Result<Group, StoreError> {
        let conn = self.conn()?;
        let changed = conn
            .execute(
                "UPDATE groups
                    SET name=?2,
                        base_url      = CASE WHEN ?3 THEN ?4 ELSE base_url END,
                        api_key       = CASE WHEN ?5 THEN ?6 ELSE api_key END,
                        default_model = CASE WHEN ?7 THEN ?8 ELSE default_model END
                  WHERE id=?1",
                params![
                    id.to_string(),
                    draft.name,
                    draft.base_url.is_some(),
                    draft.base_url.clone().flatten(),
                    draft.api_key.is_some(),
                    draft.api_key.clone().flatten(),
                    draft.default_model.is_some(),
                    draft.default_model.clone().flatten(),
                ],
            )
            .map_err(|e| classify(e, &draft.name))?;
        if changed == 0 {
            return Err(StoreError::GroupNotFound(id));
        }
        self.get_group(id)?.ok_or(StoreError::GroupNotFound(id))
    }

    /// A group's raw inference overrides, key included.
    ///
    /// Separate from `list_groups` on purpose: that shape crosses IPC and must
    /// never carry the key, and this one never leaves the runtime.
    pub fn group_inference(&self, id: GroupId) -> Result<GroupInference, StoreError> {
        let conn = self.conn()?;
        let found = conn
            .query_row(
                "SELECT base_url, api_key, default_model FROM groups WHERE id=?1",
                params![id.to_string()],
                |row| {
                    Ok(GroupInference {
                        base_url: row.get(0)?,
                        api_key: row.get(1)?,
                        default_model: row.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(found.unwrap_or_default())
    }

    pub fn get_group(&self, id: GroupId) -> Result<Option<Group>, StoreError> {
        Ok(self.list_groups()?.into_iter().find(|g| g.id == id))
    }

    /// Deletes a group that no live agent is in.
    ///
    /// Refuses while it still holds agents that can act, rather than moving
    /// them: relocating an agent would quietly put it inside a boundary it was
    /// deliberately kept out of, and deleting it would destroy work on what
    /// reads like a tidy-up. The operator decides.
    ///
    /// Deleted agents are a different matter. They are kept only so their
    /// transcripts still render, and they cannot be reached or act, so they are
    /// moved to the default group rather than holding a group open forever.
    /// Counting them was why deleting every agent in a group still reported
    /// three agents in it.
    pub fn delete_group(&self, id: GroupId) -> Result<(), StoreError> {
        let group = self.get_group(id)?.ok_or(StoreError::GroupNotFound(id))?;
        if group.agent_count > 0 {
            return Err(StoreError::GroupNotEmpty { name: group.name, agents: group.agent_count });
        }

        let default = default_group_id();
        if id == default {
            return Err(StoreError::CannotDeleteDefaultGroup);
        }

        let conn = self.conn()?;
        conn.execute(
            "UPDATE agents SET group_id=?2 WHERE group_id=?1",
            params![id.to_string(), default.to_string()],
        )?;
        conn.execute("DELETE FROM groups WHERE id=?1", params![id.to_string()])?;
        Ok(())
    }

    // ---- messages --------------------------------------------------------

    pub fn append(&self, envelope: &Envelope) -> Result<(), StoreError> {
        let conn = self.conn()?;
        let (from_kind, from_agent) = participant_columns(envelope.from);
        let (to_kind, to_agent) = participant_columns(envelope.to);
        let parts = serde_json::to_string(&envelope.parts)
            .map_err(|e| StoreError::Corrupt(format!("parts are not serializable: {e}")))?;

        conn.execute(
            "INSERT INTO messages (id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,cause,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                envelope.id.to_string(),
                envelope.run_id.to_string(),
                envelope.channel_id.to_string(),
                from_kind,
                from_agent,
                to_kind,
                to_agent,
                parts,
                envelope.trust.as_str(),
                envelope.hop,
                envelope.expects_reply,
                envelope.cause.map(|c| c.to_string()),
                envelope.created_at,
            ],
        )?;
        Ok(())
    }

    /// The newest `limit` messages in a channel, returned oldest-first for
    /// direct rendering.
    pub fn channel_messages(
        &self,
        channel: AgentId,
        limit: u32,
    ) -> Result<Vec<Envelope>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,cause,created_at
               FROM messages WHERE channel_id=?1
              ORDER BY created_at DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![channel.to_string(), limit], row_to_envelope)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        out.reverse();
        Ok(out)
    }

    /// The conversation as a whole, oldest last, for the flow board.
    ///
    /// Includes the operator's messages and the replies back to them, not just
    /// peer traffic: a flow that starts partway through, at the first agent to
    /// agent message, hides who set it off. An agent's private activity records
    /// are excluded, since they are bookkeeping rather than a message passing
    /// between two participants.
    pub fn conversation_flow(&self, limit: u32) -> Result<Vec<Envelope>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,cause,created_at
               FROM messages WHERE to_kind <> 'system'
              ORDER BY created_at DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], row_to_envelope)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        out.reverse();
        Ok(out)
    }

    // ---- usage -----------------------------------------------------------

    /// Records what one model call cost.
    ///
    /// Best-effort by design: a call that produced real work must not be
    /// reported as a failure because its accounting row would not write.
    pub fn record_usage(&self, entry: &UsageEntry) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO usage (agent_id,group_id,run_id,model,prompt,completion,cost,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                entry.agent_id.to_string(),
                entry.group_id.to_string(),
                entry.run_id.to_string(),
                entry.model,
                entry.prompt,
                entry.completion,
                entry.cost,
                now_ms(),
            ],
        )?;
        Ok(())
    }

    /// Everything each group has spent, ever.
    pub fn usage_by_group(&self) -> Result<HashMap<GroupId, Tokens>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT group_id, SUM(prompt), SUM(completion), SUM(cost), COUNT(*)
               FROM usage GROUP BY group_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                Tokens {
                    prompt: row.get::<_, i64>(1)? as u64,
                    completion: row.get::<_, i64>(2)? as u64,
                    cost: row.get::<_, Option<f64>>(3)?,
                    calls: row.get::<_, i64>(4)? as u64,
                },
            ))
        })?;

        let mut out = HashMap::new();
        for row in rows {
            let (id, tokens) = row?;
            // A row whose group id no longer parses is accounting for a group
            // that is gone. It stays in the table and out of the summary.
            if let Ok(group) = id.parse::<GroupId>() {
                out.insert(group, tokens);
            }
        }
        Ok(out)
    }

    /// What each run cost, for the runs given. Empty input, empty answer.
    pub fn usage_by_run(&self, runs: &[RunId]) -> Result<HashMap<RunId, Tokens>, StoreError> {
        if runs.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self.conn()?;
        let slots = std::iter::repeat_n("?", runs.len()).collect::<Vec<_>>().join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT run_id, SUM(prompt), SUM(completion), SUM(cost), COUNT(*) FROM usage
              WHERE run_id IN ({slots}) GROUP BY run_id"
        ))?;
        let ids: Vec<String> = runs.iter().map(RunId::to_string).collect();
        let rows = stmt.query_map(rusqlite::params_from_iter(ids), |row| {
            Ok((
                row.get::<_, String>(0)?,
                Tokens {
                    prompt: row.get::<_, i64>(1)? as u64,
                    completion: row.get::<_, i64>(2)? as u64,
                    cost: row.get::<_, Option<f64>>(3)?,
                    calls: row.get::<_, i64>(4)? as u64,
                },
            ))
        })?;

        let mut out = HashMap::new();
        for row in rows {
            let (id, tokens) = row?;
            if let Ok(run) = id.parse::<RunId>() {
                out.insert(run, tokens);
            }
        }
        Ok(out)
    }

    /// Empties every channel in a group: a fresh start for a whole crew.
    ///
    /// Agents survive, along with their notes and their machines. Only what was
    /// said goes, which is also what an agent reads back as its history, so the
    /// crew genuinely starts over rather than carrying a conversation it can no
    /// longer be shown.
    ///
    /// Agents cannot message across a group boundary, so every message a group
    /// produced is filed in a channel belonging to one of its agents. Deleted
    /// agents are included: their transcripts are part of what this group said.
    pub fn delete_group_messages(&self, group: GroupId) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute(
            "DELETE FROM messages
              WHERE channel_id IN (SELECT id FROM agents WHERE group_id=?1)",
            params![group.to_string()],
        )?)
    }

    pub fn delete_channel_messages(&self, channel: AgentId) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute("DELETE FROM messages WHERE channel_id=?1", params![channel.to_string()])?)
    }

    /// Newest message timestamp per agent, counting both ends of a message.
    ///
    /// An agent that messaged a peer has that message filed in the *peer's*
    /// channel, so grouping by channel alone would leave the sender looking
    /// idle. Both endpoints are considered.
    pub fn last_activity(&self) -> Result<HashMap<AgentId, i64>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT agent, MAX(created_at) FROM (
                 SELECT from_agent AS agent, created_at FROM messages WHERE from_agent IS NOT NULL
                 UNION ALL
                 SELECT to_agent AS agent, created_at FROM messages WHERE to_agent IS NOT NULL
             ) GROUP BY agent",
        )?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let at: i64 = row.get(1)?;
            Ok((id, at))
        })?;

        let mut out = HashMap::new();
        for row in rows {
            let (raw, at) = row?;
            match raw.parse::<AgentId>() {
                Ok(id) => {
                    out.insert(id, at);
                }
                // A malformed id should not sink the whole sidebar ordering.
                Err(err) => tracing::warn!(%err, id = %raw, "skipping unparseable agent id"),
            }
        }
        Ok(out)
    }

    pub fn count_messages(&self) -> Result<i64, StoreError> {
        let conn = self.conn()?;
        Ok(conn.query_row("SELECT count(*) FROM messages", [], |r| r.get(0))?)
    }
}

// ---- row mapping ---------------------------------------------------------

fn participant_columns(p: Participant) -> (&'static str, Option<String>) {
    match p {
        Participant::Human => ("human", None),
        Participant::System => ("system", None),
        Participant::Agent { id } => ("agent", Some(id.to_string())),
    }
}

fn participant_from_columns(kind: &str, agent: Option<String>) -> Result<Participant, StoreError> {
    match kind {
        "human" => Ok(Participant::Human),
        "system" => Ok(Participant::System),
        "agent" => {
            let raw = agent.ok_or_else(|| {
                StoreError::Corrupt("row has from/to kind 'agent' but no agent id".into())
            })?;
            raw.parse::<AgentId>()
                .map(|id| Participant::Agent { id })
                .map_err(|e| StoreError::Corrupt(format!("bad agent id {raw:?}: {e}")))
        }
        other => Err(StoreError::Corrupt(format!("unknown participant kind {other:?}"))),
    }
}

/// Returns a nested Result so a malformed row surfaces as a domain error
/// rather than being coerced into a rusqlite error with no context.
type RowResult<T> = Result<Result<T, StoreError>, rusqlite::Error>;

fn row_to_card(row: &Row<'_>) -> RowResult<AgentCard> {
    let id_raw: String = row.get(0)?;
    let skills_raw: String = row.get(6)?;
    let lifecycle_raw: String = row.get(7)?;

    Ok((|| {
        let id = id_raw
            .parse::<AgentId>()
            .map_err(|e| StoreError::Corrupt(format!("bad agent id {id_raw:?}: {e}")))?;
        let lifecycle = Lifecycle::parse(&lifecycle_raw)
            .ok_or_else(|| StoreError::Corrupt(format!("unknown lifecycle {lifecycle_raw:?}")))?;
        // A malformed skills blob should not make the agent unloadable; the
        // card is still usable without it.
        let skills: Vec<String> = serde_json::from_str(&skills_raw).unwrap_or_default();

        Ok(AgentCard {
            id,
            name: row.get(1)?,
            avatar: row.get(2)?,
            color: row.get(3)?,
            model: row.get(4)?,
            system_prompt: row.get(5)?,
            skills,
            lifecycle,
            group_id: {
                let raw: String = row.get(11)?;
                raw.parse::<GroupId>()
                    .map_err(|e| StoreError::Corrupt(format!("bad group id {raw:?}: {e}")))?
            },
            sandbox_id: row.get(12)?,
            sandbox_envd_token: row.get(13)?,
            sandbox_traffic_token: row.get(14)?,
            version: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
        })
    })())
}

fn row_to_routine(row: &Row<'_>) -> RowResult<Routine> {
    let id_raw: String = row.get(0)?;
    let agent_raw: String = row.get(1)?;

    Ok((|| {
        Ok(Routine {
            id: id_raw
                .parse::<RoutineId>()
                .map_err(|e| StoreError::Corrupt(format!("bad routine id {id_raw:?}: {e}")))?,
            agent_id: agent_raw
                .parse::<AgentId>()
                .map_err(|e| StoreError::Corrupt(format!("bad agent id {agent_raw:?}: {e}")))?,
            what: row.get(2)?,
            every_secs: row.get(3)?,
            next_run_at: row.get(4)?,
            last_run_at: row.get(5)?,
            created_at: row.get(6)?,
        })
    })())
}

fn row_to_group(row: &Row<'_>) -> RowResult<Group> {
    let id_raw: String = row.get(0)?;
    let api_key: Option<String> = row.get(5)?;

    Ok((|| {
        Ok(Group {
            id: id_raw
                .parse::<GroupId>()
                .map_err(|e| StoreError::Corrupt(format!("bad group id {id_raw:?}: {e}")))?,
            name: row.get(1)?,
            created_at: row.get(2)?,
            agent_count: row.get::<_, i64>(3)?.max(0) as u32,
            base_url: row.get(4)?,
            default_model: row.get(6)?,
            api_key_set: api_key.as_deref().is_some_and(|k| !k.trim().is_empty()),
            api_key_hint: crate::config::hint_for(api_key.as_deref().unwrap_or_default()),
        })
    })())
}

fn row_to_envelope(row: &Row<'_>) -> RowResult<Envelope> {
    let id_raw: String = row.get(0)?;
    let run_raw: String = row.get(1)?;
    let channel_raw: String = row.get(2)?;
    let from_kind: String = row.get(3)?;
    let from_agent: Option<String> = row.get(4)?;
    let to_kind: String = row.get(5)?;
    let to_agent: Option<String> = row.get(6)?;
    let parts_raw: String = row.get(7)?;
    let trust_raw: String = row.get(8)?;
    let hop: u16 = row.get(9)?;
    let expects_reply: bool = row.get(10)?;
    let cause_raw: Option<String> = row.get(11)?;
    let created_at: i64 = row.get(12)?;

    Ok((|| {
        let parts: Vec<Part> = serde_json::from_str(&parts_raw)
            .map_err(|e| StoreError::Corrupt(format!("unreadable parts: {e}")))?;
        let trust = Trust::parse(&trust_raw)
            .ok_or_else(|| StoreError::Corrupt(format!("unknown trust {trust_raw:?}")))?;

        Ok(Envelope {
            id: id_raw.parse().map_err(|e| StoreError::Corrupt(format!("bad message id: {e}")))?,
            run_id: run_raw.parse().map_err(|e| StoreError::Corrupt(format!("bad run id: {e}")))?,
            channel_id: channel_raw
                .parse()
                .map_err(|e| StoreError::Corrupt(format!("bad channel id: {e}")))?,
            from: participant_from_columns(&from_kind, from_agent)?,
            to: participant_from_columns(&to_kind, to_agent)?,
            parts,
            trust,
            hop,
            expects_reply,
            cause: match cause_raw {
                Some(raw) => Some(
                    raw.parse::<MessageId>()
                        .map_err(|e| StoreError::Corrupt(format!("bad cause id: {e}")))?,
                ),
                None => None,
            },
            created_at,
        })
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::envelope::channel_for;
    use crate::domain::ids::RunId;

    struct Fixture {
        store: Store,
        _dir: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();
        Fixture { store, _dir: dir }
    }

    fn draft(name: &str) -> CleanDraft {
        CleanDraft {
            group_id: None,
            name: name.into(),
            avatar: "avocado".into(),
            color: "#7fb069".into(),
            model: "anthropic/claude-sonnet-4.5".into(),
            system_prompt: "be useful".into(),
            skills: vec!["coordination".into()],
        }
    }

    fn envelope(from: Participant, to: Participant, text: &str, run: RunId) -> Envelope {
        Envelope {
            id: MessageId::new(),
            run_id: run,
            channel_id: channel_for(from, to).expect("test envelopes must be routable"),
            from,
            to,
            parts: vec![Part::text(text)],
            trust: Trust::Peer,
            hop: 1,
            expects_reply: true,
            cause: None,
            created_at: now_ms(),
        }
    }

    #[test]
    fn every_pooled_connection_comes_up_configured() {
        let f = fixture();
        // Hold them all at once so the pool is forced to open its full size.
        let conns: Vec<_> = (0..8).map(|_| f.store.conn().unwrap()).collect();
        for conn in &conns {
            let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
            assert_eq!(mode.to_lowercase(), "wal", "a connection came up outside WAL");

            let timeout: i64 = conn.query_row("PRAGMA busy_timeout", [], |r| r.get(0)).unwrap();
            assert!(timeout >= 5000, "busy timeout was not applied: {timeout}");
        }
    }

    #[test]
    fn a_group_emptied_by_deleting_its_agents_can_then_be_deleted() {
        // The bug this covers: deleting every agent in a group left it
        // reporting three agents and refusing to go, because the rows are kept
        // for their transcripts and were still being counted.
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();

        let group = store
            .create_group(&CleanGroup {
                name: "Research".into(),
                base_url: None,
                default_model: None,
                api_key: None,
            })
            .unwrap();

        for name in ["One", "Two", "Three"] {
            let mut d = draft(name);
            d.group_id = Some(group.id);
            let card = store.create_agent(&d).unwrap();
            store.set_lifecycle(card.id, Lifecycle::Terminated).unwrap();
        }

        assert_eq!(
            store.get_group(group.id).unwrap().unwrap().agent_count,
            0,
            "deleted agents are not agents you have"
        );
        store.delete_group(group.id).expect("an emptied group must be deletable");

        // Their transcripts still have to render, so they keep a group.
        let orphans: Vec<_> =
            store.list_agents().unwrap().into_iter().filter(|c| c.group_id == group.id).collect();
        assert!(orphans.is_empty(), "no agent may point at a group that is gone");
    }

    #[test]
    fn editing_a_routine_leaves_its_next_firing_alone_unless_asked() {
        // Correcting a typo in what a routine says must not silently reset the
        // schedule it is keeping.
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        let made =
            f.store.create_routine(card.id, "check the listings", Some(3600), 1_000_000).unwrap();

        let fixed = f
            .store
            .update_routine(
                made.id,
                "check the listings and say what is new",
                Some(3600),
                made.next_run_at,
            )
            .unwrap();
        assert_eq!(fixed.what, "check the listings and say what is new");
        assert_eq!(fixed.next_run_at, made.next_run_at, "the schedule did not move");
        assert_eq!(fixed.every_secs, Some(3600));

        // And a routine that is gone is a clear error rather than a silent
        // success, so an operator editing a stale screen is told.
        f.store.delete_routine(made.id).unwrap();
        assert!(matches!(
            f.store.update_routine(made.id, "anything", None, 1),
            Err(StoreError::RoutineNotFound(_))
        ));
    }

    #[test]
    fn clearing_a_group_empties_its_crew_and_leaves_everyone_elses_channels() {
        let f = fixture();
        let other = f
            .store
            .create_group(&CleanGroup {
                name: "Research".into(),
                base_url: None,
                default_model: None,
                api_key: None,
            })
            .unwrap();

        let mut d = draft("Scholar");
        d.group_id = Some(other.id);
        let scholar = f.store.create_agent(&d).unwrap();
        let bystander = f.store.create_agent(&draft("Manager")).unwrap();

        let run = RunId::new();
        let agent = |id| Participant::Agent { id };
        f.store.append(&envelope(Participant::Human, agent(scholar.id), "in scope", run)).unwrap();
        f.store
            .append(&envelope(Participant::Human, agent(bystander.id), "untouched", run))
            .unwrap();

        // A deleted agent's transcript is part of what the group said, so it
        // goes too: leaving it behind is what "start fresh" is not.
        f.store.set_lifecycle(scholar.id, Lifecycle::Terminated).unwrap();

        assert_eq!(f.store.delete_group_messages(other.id).unwrap(), 1);
        assert!(f.store.channel_messages(scholar.id, 50).unwrap().is_empty());
        assert_eq!(
            f.store.channel_messages(bystander.id, 50).unwrap().len(),
            1,
            "clearing one group must not touch another"
        );
    }

    #[test]
    fn a_group_with_live_agents_still_refuses_to_go() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();
        let group = store
            .create_group(&CleanGroup {
                name: "Busy".into(),
                base_url: None,
                default_model: None,
                api_key: None,
            })
            .unwrap();
        let mut d = draft("Busy One");
        d.group_id = Some(group.id);
        store.create_agent(&d).unwrap();

        assert!(matches!(store.delete_group(group.id), Err(StoreError::GroupNotEmpty { .. })));
    }

    #[test]
    fn opening_the_same_database_concurrently_does_not_fail() {
        // Regression: journal_mode is a file-level setting behind an exclusive
        // lock. When several connections raced to set it, the losers failed
        // outright rather than waiting, because SQLite reports a shared vs
        // exclusive conflict inside one process as a deadlock.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("guac.db");

        let results: Vec<Result<Store, StoreError>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8).map(|_| scope.spawn(|| Store::open(&path))).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        for result in &results {
            assert!(result.is_ok(), "concurrent open failed: {:?}", result.as_ref().err());
        }
        let mode: String = results[0]
            .as_ref()
            .unwrap()
            .conn()
            .unwrap()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
    }

    #[test]
    fn opening_creates_the_schema_and_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("guac.db");
        let store = Store::open(&path).unwrap();
        store.create_agent(&draft("Manager")).unwrap();
        drop(store);

        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.list_agents().unwrap().len(), 1, "data must survive a restart");
    }

    #[test]
    fn create_and_read_back_an_agent() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Manager")).unwrap();
        assert_eq!(card.version, 1);
        assert_eq!(card.lifecycle, Lifecycle::Active);

        let fetched = f.store.get_agent(card.id).unwrap().unwrap();
        assert_eq!(fetched, card, "round trip must be lossless");
    }

    #[test]
    fn duplicate_live_names_are_rejected_with_a_domain_error() {
        let f = fixture();
        f.store.create_agent(&draft("Manager")).unwrap();
        let err = f.store.create_agent(&draft("manager")).unwrap_err();
        assert!(
            matches!(&err, StoreError::DuplicateName(name) if name == "manager"),
            "expected DuplicateName, got {err:?}"
        );
    }

    #[test]
    fn deleting_an_agent_frees_the_name_and_keeps_the_transcript() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Manager")).unwrap();
        let run = RunId::new();
        f.store
            .append(&envelope(Participant::Human, Participant::Agent { id: card.id }, "hi", run))
            .unwrap();

        f.store.set_lifecycle(card.id, Lifecycle::Terminated).unwrap();

        let reused = f.store.create_agent(&draft("Manager")).unwrap();
        assert_ne!(reused.id, card.id);
        assert_eq!(
            f.store.channel_messages(card.id, 50).unwrap().len(),
            1,
            "history of a deleted agent must remain readable"
        );
    }

    #[test]
    fn update_bumps_the_version_and_persists_edits() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Manager")).unwrap();
        let mut edit = draft("Coordinator");
        edit.color = "#ff0000".into();

        let updated = f.store.update_agent(card.id, &edit).unwrap();
        assert_eq!(updated.version, 2, "peers detect card changes by version");
        assert_eq!(updated.name, "Coordinator");
        assert_eq!(updated.color, "#ff0000");
        assert_eq!(updated.created_at, card.created_at, "creation time must not move");
        assert!(updated.updated_at >= card.updated_at);
    }

    #[test]
    fn updating_a_missing_agent_reports_not_found() {
        let f = fixture();
        let err = f.store.update_agent(AgentId::new(), &draft("Ghost")).unwrap_err();
        assert!(matches!(err, StoreError::AgentNotFound(_)));
    }

    #[test]
    fn renaming_onto_a_taken_name_is_refused() {
        let f = fixture();
        f.store.create_agent(&draft("Manager")).unwrap();
        let chef = f.store.create_agent(&draft("Chef")).unwrap();
        let err = f.store.update_agent(chef.id, &draft("Manager")).unwrap_err();
        assert!(matches!(err, StoreError::DuplicateName(_)));
    }

    #[test]
    fn agents_list_in_creation_order() {
        let f = fixture();
        for name in ["A", "B", "C"] {
            f.store.create_agent(&draft(name)).unwrap();
        }
        assert_eq!(
            f.store.list_agents().unwrap().iter().map(|x| x.name.clone()).collect::<Vec<_>>(),
            vec!["A", "B", "C"]
        );
    }

    #[test]
    fn agents_created_in_the_same_millisecond_keep_their_creation_order() {
        // `created_at` is not a total order at millisecond resolution, and
        // falling back to the id sorts by random UUID. Ordering by rowid gives
        // exact insertion order, which is what the sidebar has to show. Rows are
        // never hard-deleted, so rowids are never reused.
        let f = fixture();
        for name in ["A", "B", "C", "D", "E"] {
            f.store.create_agent(&draft(name)).unwrap();
        }
        let names: Vec<String> =
            f.store.list_agents().unwrap().iter().map(|c| c.name.clone()).collect();
        assert_eq!(names, vec!["A", "B", "C", "D", "E"]);

        // And it must not drift between reads.
        let first = f.store.list_agents().unwrap();
        for _ in 0..5 {
            assert_eq!(f.store.list_agents().unwrap(), first, "ordering must be deterministic");
        }
    }

    #[test]
    fn envelopes_round_trip_with_all_participant_shapes() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Manager")).unwrap();
        let peer = f.store.create_agent(&draft("Chef")).unwrap();
        let run = RunId::new();
        let agent = |id| Participant::Agent { id };

        let cases = [
            envelope(Participant::Human, agent(card.id), "from human", run),
            envelope(agent(card.id), Participant::Human, "to human", run),
            envelope(agent(card.id), agent(peer.id), "agent to agent", run),
            envelope(Participant::System, agent(card.id), "system notice", run),
        ];
        for case in &cases {
            f.store.append(case).unwrap();
        }

        let manager_channel = f.store.channel_messages(card.id, 50).unwrap();
        assert_eq!(manager_channel.len(), 3, "human traffic and system notices file under Manager");

        let chef_channel = f.store.channel_messages(peer.id, 50).unwrap();
        assert_eq!(chef_channel.len(), 1);
        assert_eq!(chef_channel[0].from, agent(card.id));
        assert_eq!(chef_channel[0].plain_text(), "agent to agent");
    }

    #[test]
    fn channel_messages_come_back_oldest_first() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Manager")).unwrap();
        let run = RunId::new();
        for i in 0..5 {
            let mut e = envelope(
                Participant::Human,
                Participant::Agent { id: card.id },
                &format!("m{i}"),
                run,
            );
            e.created_at = 1_000 + i as i64;
            f.store.append(&e).unwrap();
        }
        let texts: Vec<String> =
            f.store.channel_messages(card.id, 50).unwrap().iter().map(|e| e.plain_text()).collect();
        assert_eq!(texts, vec!["m0", "m1", "m2", "m3", "m4"]);
    }

    #[test]
    fn channel_limit_keeps_the_newest_messages_in_order() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Manager")).unwrap();
        let run = RunId::new();
        for i in 0..10 {
            let mut e = envelope(
                Participant::Human,
                Participant::Agent { id: card.id },
                &format!("m{i}"),
                run,
            );
            e.created_at = 1_000 + i as i64;
            f.store.append(&e).unwrap();
        }
        let texts: Vec<String> =
            f.store.channel_messages(card.id, 3).unwrap().iter().map(|e| e.plain_text()).collect();
        assert_eq!(
            texts,
            vec!["m7", "m8", "m9"],
            "a limited window must be the newest, still ordered"
        );
    }

    #[test]
    fn the_flow_covers_the_whole_conversation_but_not_bookkeeping() {
        let f = fixture();
        let a = f.store.create_agent(&draft("A")).unwrap();
        let b = f.store.create_agent(&draft("B")).unwrap();
        let run = RunId::new();
        let agent = |id| Participant::Agent { id };

        // Explicit timestamps: these are written in the same millisecond, and
        // the tie-break is by id, which is a random UUID.
        let mut at = 1_000;
        let mut send = |from, to, text: &str| {
            let mut e = envelope(from, to, text, run);
            e.created_at = at;
            at += 1;
            f.store.append(&e).unwrap();
        };
        send(Participant::Human, agent(a.id), "you asked");
        send(agent(a.id), agent(b.id), "peer msg");
        send(agent(b.id), Participant::Human, "answer");
        // An agent's own activity record is not a message between participants.
        send(agent(a.id), Participant::System, "tool trail");

        let flow: Vec<String> =
            f.store.conversation_flow(50).unwrap().iter().map(Envelope::plain_text).collect();
        assert_eq!(flow, vec!["you asked", "peer msg", "answer"]);
    }

    #[test]
    fn the_flow_comes_back_in_order() {
        let f = fixture();
        let a = f.store.create_agent(&draft("A")).unwrap();
        let run = RunId::new();
        for i in 0..5 {
            let mut e = envelope(
                Participant::Human,
                Participant::Agent { id: a.id },
                &format!("m{i}"),
                run,
            );
            e.created_at = 1_000 + i as i64;
            f.store.append(&e).unwrap();
        }
        let flow: Vec<String> =
            f.store.conversation_flow(50).unwrap().iter().map(Envelope::plain_text).collect();
        assert_eq!(flow, vec!["m0", "m1", "m2", "m3", "m4"]);
    }

    #[test]
    fn structured_parts_survive_the_round_trip() {
        use crate::domain::envelope::{NoticeKind, ToolOutcome};
        let f = fixture();
        let card = f.store.create_agent(&draft("Manager")).unwrap();
        let mut e =
            envelope(Participant::Human, Participant::Agent { id: card.id }, "x", RunId::new());
        e.parts = vec![
            Part::text("hello"),
            Part::Json { name: "report".into(), value: serde_json::json!({"ok": true, "n": 3}) },
            Part::Notice { kind: NoticeKind::GuardStop, text: "hop limit".into() },
            Part::ToolCall {
                name: "send_message".into(),
                arguments: serde_json::json!({"to": "Chef"}),
                outcome: ToolOutcome::Refused { reason: "duplicate".into() },
            },
        ];
        f.store.append(&e).unwrap();

        let back = f.store.channel_messages(card.id, 1).unwrap().pop().unwrap();
        assert_eq!(back.parts, e.parts, "every part variant must survive storage");
    }

    #[test]
    fn cause_and_hop_are_preserved_for_replay() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Manager")).unwrap();
        let cause = MessageId::new();
        let mut e =
            envelope(Participant::Human, Participant::Agent { id: card.id }, "x", RunId::new());
        e.cause = Some(cause);
        e.hop = 3;
        f.store.append(&e).unwrap();

        let back = f.store.channel_messages(card.id, 1).unwrap().pop().unwrap();
        assert_eq!(back.cause, Some(cause));
        assert_eq!(back.hop, 3);
    }

    #[test]
    fn a_corrupt_participant_row_surfaces_as_corrupt_not_a_panic() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Manager")).unwrap();
        {
            let conn = f.store.conn().unwrap();
            conn.execute(
                "INSERT INTO messages (id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,cause,created_at)
                 VALUES (?1,?2,?3,'agent',NULL,'human',NULL,'[]','peer',0,1,NULL,1)",
                params![MessageId::new().to_string(), RunId::new().to_string(), card.id.to_string()],
            )
            .unwrap();
        }
        let err = f.store.channel_messages(card.id, 10).unwrap_err();
        assert!(matches!(err, StoreError::Corrupt(_)), "got {err:?}");
    }

    #[test]
    fn last_activity_counts_both_ends_of_a_message() {
        let f = fixture();
        let a = f.store.create_agent(&draft("A")).unwrap();
        let b = f.store.create_agent(&draft("B")).unwrap();
        let quiet = f.store.create_agent(&draft("Quiet")).unwrap();
        let run = RunId::new();
        let agent = |id| Participant::Agent { id };

        let mut msg = envelope(agent(a.id), agent(b.id), "hello", run);
        msg.created_at = 5_000;
        f.store.append(&msg).unwrap();

        let seen = f.store.last_activity().unwrap();
        assert_eq!(seen.get(&a.id), Some(&5_000), "the sender must not look idle");
        assert_eq!(seen.get(&b.id), Some(&5_000), "nor the recipient");
        assert_eq!(seen.get(&quiet.id), None, "an agent with no traffic has no entry");
    }

    #[test]
    fn last_activity_reports_the_newest_message() {
        let f = fixture();
        let a = f.store.create_agent(&draft("A")).unwrap();
        let run = RunId::new();
        for at in [1_000, 9_000, 4_000] {
            let mut msg = envelope(Participant::Human, Participant::Agent { id: a.id }, "x", run);
            msg.created_at = at;
            f.store.append(&msg).unwrap();
        }
        assert_eq!(f.store.last_activity().unwrap().get(&a.id), Some(&9_000));
    }

    #[test]
    fn clearing_a_channel_leaves_other_channels_alone() {
        let f = fixture();
        let a = f.store.create_agent(&draft("A")).unwrap();
        let b = f.store.create_agent(&draft("B")).unwrap();
        let run = RunId::new();
        f.store
            .append(&envelope(Participant::Human, Participant::Agent { id: a.id }, "x", run))
            .unwrap();
        f.store
            .append(&envelope(Participant::Human, Participant::Agent { id: b.id }, "y", run))
            .unwrap();

        assert_eq!(f.store.delete_channel_messages(a.id).unwrap(), 1);
        assert!(f.store.channel_messages(a.id, 10).unwrap().is_empty());
        assert_eq!(f.store.channel_messages(b.id, 10).unwrap().len(), 1);
    }

    #[test]
    fn concurrent_writers_do_not_deadlock() {
        // Agents write from independent tasks. WAL plus a busy timeout should
        // make this boring; if it is not, it fails here and not in the field.
        let f = fixture();
        let card = f.store.create_agent(&draft("Manager")).unwrap();
        let run = RunId::new();

        std::thread::scope(|scope| {
            for _ in 0..8 {
                let store = f.store.clone();
                scope.spawn(move || {
                    for i in 0..25 {
                        let e = envelope(
                            Participant::Human,
                            Participant::Agent { id: card.id },
                            &format!("m{i}"),
                            run,
                        );
                        store.append(&e).unwrap();
                    }
                });
            }
        });

        assert_eq!(f.store.count_messages().unwrap(), 200);
    }
}
