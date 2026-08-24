//! SQLite-backed persistence.
//!
//! One pool, WAL mode, plain SQL. There is no ORM because there are two tables
//! and eleven queries, and hiding those behind a query builder would add a
//! dependency and remove the ability to read what actually hits the disk.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{named_params, params, OptionalExtension, Row};

use crate::db::migrations;
use crate::domain::agent::{AgentCard, CleanDraft, Lifecycle};
use crate::domain::approval::{
    Approval, ApprovalState, DetailField, ProtectedAction, Request, QUESTION,
};
use crate::domain::connector::{CleanConnector, Connector};
use crate::domain::envelope::{Envelope, Intent, Part, Participant, Trust};
use crate::domain::group::{CleanGroup, Group, GroupInference, GroupLimits, InferenceOverrides};
use crate::domain::ids::{
    AgentId, ApprovalId, ConnectorId, GroupId, MessageId, PluginId, RepositoryId, RoutineId, RunId,
};
use crate::domain::now_ms;
use crate::domain::plugin::{
    Plugin, PluginAccess, PluginKind, PluginTool, PluginToolCard, PluginToolset,
};
use crate::domain::repository::{CleanRepository, Repository};
use crate::domain::routine::{NextSlot, Routine, RoutineRun, RunKind, Trigger};
use crate::domain::search::{
    contains_fold, excerpt, like_pattern, links_in, FileHit, LinkHit, MessageHit, SearchHits,
};
use crate::domain::signin::{Signin, Surface};
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
    #[error("no connector with id {0}")]
    ConnectorNotFound(ConnectorId),
    #[error("no plugin with id {0}")]
    PluginNotFound(PluginId),
    #[error("agent {0} is not in this group, so it cannot be given one of the group's plugins")]
    AgentNotInGroup(AgentId),
    #[error(
        "this plugin publishes no tool called {1:?}, so there is nothing to allow or deny; the \
         list on screen is older than the one the server last sent, so connect it again"
    )]
    PluginToolNotFound(PluginId, String),
    #[error("no permission request with id {0}")]
    ApprovalNotFound(ApprovalId),
    #[error("that request was already answered ({})", state.as_str())]
    ApprovalSettled { state: ApprovalState },
    #[error("{0} is already used by another credential in this group")]
    DuplicateEnvVar(String),
    #[error("no repository with id {0}")]
    RepositoryNotFound(RepositoryId),
    #[error(
        "this crew already has {0} linked; give it to whoever needs it instead of adding it again"
    )]
    DuplicateRepository(String),
    #[error(
        "agent {0} is not in this group, so it cannot be given one of the group's repositories"
    )]
    AgentNotInGroupForRepository(AgentId),
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

/// A fresh card from a validated draft. Everything an agent goes on to acquire
/// (a machine, its tokens, a pin) starts empty: those are things it did, not
/// things anybody wrote down.
fn new_card(draft: &CleanDraft, rail_order: i32) -> AgentCard {
    let now = now_ms();
    AgentCard {
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
        browser_id: None,
        has_computer: false,
        has_browser: false,
        lifecycle: Lifecycle::Active,
        pinned: false,
        rail_order,
        version: 1,
        created_at: now,
        updated_at: now,
    }
}

/// The slot after the last row in the rail.
///
/// Read rather than defaulted, and read on whatever connection is doing the
/// writing: a new agent must not land in the middle of an arrangement the
/// operator made, and a column default cannot know what the last row is. Taking
/// it on the transaction is what stops a concurrent create from handing out the
/// same slot twice.
fn bottom_of_rail(conn: &rusqlite::Connection) -> Result<i32, StoreError> {
    let last: i32 =
        conn.query_row("SELECT coalesce(max(rail_order), -1) FROM agents", [], |row| row.get(0))?;
    Ok(last.saturating_add(1))
}

/// Takes a `&Connection` rather than a `&Store` so one insert serves both a
/// single create and a batch inside a transaction; `Transaction` derefs here.
fn insert_agent(conn: &rusqlite::Connection, card: &AgentCard) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id,rail_order)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
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
            card.rail_order,
        ],
    )
    .map_err(|e| classify(e, &card.name))?;
    Ok(())
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

/// How many matching messages a search reads before it cuts each category.
///
/// Wider than any list it produces, because links are pulled out of the same
/// rows: the URL somebody wants is rarely in the first few messages that
/// mention its host. A search that finds the newest few hundred matches is the
/// one an operator asked for; finding the oldest of ten thousand is a report.
const SEARCH_SCAN: u32 = 400;

/// How much of a message a result row shows.
const EXCERPT_CHARS: usize = 160;

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
            let _serialized = bootstrap_lock().lock();

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
        let card = new_card(draft, bottom_of_rail(&conn)?);
        insert_agent(&conn, &card)?;
        Ok(card)
    }

    /// Creates a whole crew, or none of it.
    ///
    /// Written in one transaction because the alternative fails halfway. Names
    /// are unique per group, and the realistic way a batch dies on its fourth
    /// row is another window taking a name between the check and the write,
    /// which no amount of validating up front prevents. Leaving three agents
    /// behind and reporting an error about a fourth gives the operator a
    /// workspace they did not ask for and no list of what landed.
    pub fn create_agents(&self, drafts: &[CleanDraft]) -> Result<Vec<AgentCard>, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        // One read, then consecutive slots. Asking for the bottom once per
        // agent would give a whole crew the same answer, because none of them
        // is written until the transaction commits, and the rail would then
        // order them by the tiebreak rather than by the order they were picked.
        let first = bottom_of_rail(&tx)?;
        let cards: Vec<AgentCard> = drafts
            .iter()
            .enumerate()
            .map(|(offset, draft)| new_card(draft, first.saturating_add(offset as i32)))
            .collect();
        for card in &cards {
            insert_agent(&tx, card)?;
        }
        tx.commit()?;
        Ok(cards)
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
            "SELECT id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id,sandbox_id,sandbox_envd_token,sandbox_traffic_token,pinned,rail_order,browser_id,has_computer,has_browser
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
            "SELECT id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id,sandbox_id,sandbox_envd_token,sandbox_traffic_token,pinned,rail_order,browser_id,has_computer,has_browser
               FROM agents ORDER BY rowid",
        )?;
        let rows = stmt.query_map([], row_to_card)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    /// Keeps an agent at the top of the rail, or lets it back down.
    ///
    /// Deliberately not part of `update_agent`: where a row is drawn is not an
    /// edit to the card, and bumping the version for it would tell every peer
    /// the agent changed under them when nothing about it did.
    pub fn set_agent_pinned(&self, id: AgentId, pinned: bool) -> Result<AgentCard, StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE agents SET pinned=?2 WHERE id=?1",
            params![id.to_string(), i64::from(pinned)],
        )?;
        if changed == 0 {
            return Err(StoreError::AgentNotFound(id));
        }
        self.get_agent(id)?.ok_or(StoreError::AgentNotFound(id))
    }

    /// Puts an agent where the operator dropped it.
    ///
    /// One call, because a drag is one gesture that can be both a reorder and a
    /// move between groups, and doing it as two writes leaves a state where the
    /// agent has arrived in the group but not in the place it was dropped.
    ///
    /// `before` is the row it lands in front of, and `None` means the end of
    /// `group`. Naming the moved agent itself asks for nothing and does nothing.
    /// Otherwise honoured only while that row is still live and still in `group`:
    /// the operator dropped onto something they could see, and a row deleted in
    /// the meantime must not cost them the half of the intent that still holds.
    ///
    /// Renumbers every live agent densely rather than finding a gap between two
    /// neighbors. A workspace holds tens of agents, so the write is trivial,
    /// and a scheme with gaps has a state where the gap is used up that this one
    /// does not have. Not an edit: like `pinned`, this is where a row is drawn,
    /// so the version does not move and no peer is told.
    pub fn move_agent(
        &self,
        id: AgentId,
        group: GroupId,
        before: Option<AgentId>,
    ) -> Result<AgentCard, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        {
            // The arrangement as it stands, which is what `before` refers to.
            // Terminated agents are left out and left alone: they are not in the
            // rail, so numbering them would spend positions on rows nobody can
            // see and would move them under a later reader.
            let mut stmt = tx.prepare(
                "SELECT id, group_id FROM agents
                  WHERE lifecycle <> 'terminated'
                  ORDER BY rail_order, created_at, rowid",
            )?;
            let rows = stmt
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let moving = id.to_string();
            if !rows.iter().any(|(row_id, _)| *row_id == moving) {
                return Err(StoreError::AgentNotFound(id));
            }

            let target = group.to_string();
            // A row dropped on itself is not a move. Caught here as well as in
            // the UI because the fallback for an anchor that is not in the group
            // is the end of it, so letting this through would spend the
            // operator's arrangement on a gesture that asked for nothing.
            if before == Some(id) && rows.iter().any(|(r, g)| *r == moving && *g == target) {
                return self.get_agent(id)?.ok_or(StoreError::AgentNotFound(id));
            }
            let mut order: Vec<(String, String)> =
                rows.iter().filter(|(row_id, _)| *row_id != moving).cloned().collect();

            let anchor = before.map(|b| b.to_string()).filter(|b| {
                order.iter().any(|(row_id, row_group)| row_id == b && *row_group == target)
            });

            let at = match anchor {
                Some(row) => order.iter().position(|(r, _)| *r == row).unwrap_or(order.len()),
                // The end of the group rather than the end of the rail: this is
                // one sequence over every group, and appending past the last
                // group would put the agent below crews it is not in.
                None => order
                    .iter()
                    .rposition(|(_, row_group)| *row_group == target)
                    .map_or(order.len(), |last| last + 1),
            };
            order.insert(at, (moving.clone(), target.clone()));

            let mut renumber = tx.prepare("UPDATE agents SET rail_order=?2 WHERE id=?1")?;
            for (position, (row, _)) in order.iter().enumerate() {
                renumber.execute(params![row, position as i32])?;
            }
            tx.execute("UPDATE agents SET group_id=?2 WHERE id=?1", params![moving, target])?;
        }
        tx.commit()?;
        self.get_agent(id)?.ok_or(StoreError::AgentNotFound(id))
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

    /// Records which hosted browser is this agent's browser.
    ///
    /// Separate from `set_agent_sandbox` rather than one call taking both,
    /// because they are provisioned independently: an agent that only ever
    /// browses never costs a machine, and one that only ever runs commands
    /// never costs a browser. A single setter would have to be given the value
    /// it is not changing, and the caller that guessed wrong would silently
    /// release the other one.
    pub fn set_agent_browser(&self, id: AgentId, browser: Option<&str>) -> Result<(), StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE agents SET browser_id=?2 WHERE id=?1",
            params![id.to_string(), browser],
        )?;
        if changed == 0 {
            return Err(StoreError::AgentNotFound(id));
        }
        Ok(())
    }

    /// Records that the operator has given this agent a computer, or taken it
    /// back.
    ///
    /// A decision, not an acquisition, which is why it is here rather than
    /// folded into `set_agent_sandbox`: the machine under it is made, slept,
    /// reclaimed and made again, and none of that is the operator changing
    /// their mind. It does not bump the card version for the same reason
    /// `set_agent_pinned` does not: peers are told what an agent is for, not
    /// what it is allowed to rent.
    ///
    /// Taking it back deliberately leaves `sandbox_id` alone. The disk is where
    /// the operator's sign-ins live, and giving the computer back has to find
    /// them there rather than starting a stranger.
    pub fn set_has_computer(&self, id: AgentId, given: bool) -> Result<(), StoreError> {
        self.set_flag(id, "has_computer", given)
    }

    /// The same decision about the browser.
    pub fn set_has_browser(&self, id: AgentId, given: bool) -> Result<(), StoreError> {
        self.set_flag(id, "has_browser", given)
    }

    /// One boolean column on one agent. The column name is a literal from the
    /// two callers above and never a value from outside this file.
    fn set_flag(&self, id: AgentId, column: &str, value: bool) -> Result<(), StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            &format!("UPDATE agents SET {column}=?2 WHERE id=?1"),
            params![id.to_string(), i64::from(value)],
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

    /// Files a new routine.
    ///
    /// `first_run_at` is `None` for a trigger that does not wait on the clock:
    /// an event routine holds no slot, and the column is NULL rather than a
    /// date far enough away to look like never.
    pub fn create_routine(
        &self,
        agent: AgentId,
        name: &str,
        what: &str,
        trigger: Trigger,
        first_run_at: Option<i64>,
        skip_if_working: bool,
    ) -> Result<Routine, StoreError> {
        let conn = self.conn()?;
        let routine = Routine {
            id: RoutineId::new(),
            agent_id: agent,
            name: name.trim().to_string(),
            what: what.trim().to_string(),
            trigger,
            active: true,
            skip_if_working,
            next_run_at: first_run_at,
            last_run_at: None,
            created_at: now_ms(),
        };
        conn.execute(
            "INSERT INTO routines (id,agent_id,name,what,fires,next_run_at,created_at,skip_if_working)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                routine.id.to_string(),
                agent.to_string(),
                routine.name,
                routine.what,
                routine.trigger.as_str(),
                routine.next_run_at,
                routine.created_at,
                i64::from(routine.skip_if_working),
            ],
        )?;
        Ok(routine)
    }

    pub fn get_routine(&self, id: RoutineId) -> Result<Option<Routine>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,agent_id,name,what,fires,active,next_run_at,last_run_at,created_at,skip_if_working
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
        name: &str,
        what: &str,
        trigger: Trigger,
        next_run_at: Option<i64>,
        skip_if_working: bool,
    ) -> Result<Routine, StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE routines SET name=?2, what=?3, fires=?4, next_run_at=?5, skip_if_working=?6
              WHERE id=?1",
            params![
                id.to_string(),
                name.trim(),
                what.trim(),
                trigger.as_str(),
                next_run_at,
                i64::from(skip_if_working),
            ],
        )?;
        if changed == 0 {
            return Err(StoreError::RoutineNotFound(id));
        }

        let mut stmt = conn.prepare(
            "SELECT id,agent_id,name,what,fires,active,next_run_at,last_run_at,created_at,skip_if_working
               FROM routines WHERE id=?1",
        )?;
        stmt.query_row(params![id.to_string()], row_to_routine)?
    }

    pub fn agent_routines(&self, agent: AgentId) -> Result<Vec<Routine>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            // Soonest first, then whatever holds no slot. NULL sorts before
            // everything in SQLite, which would put a routine waiting on an
            // event above one firing in ten minutes.
            "SELECT id,agent_id,name,what,fires,active,next_run_at,last_run_at,created_at,skip_if_working
               FROM routines WHERE agent_id=?1
              ORDER BY next_run_at IS NULL, next_run_at, created_at",
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
    ///
    /// A routine with no slot is not due and never will be, which is the whole
    /// mechanism keeping a trigger that is not a clock out of this sweep: SQL
    /// compares NULL to nothing, so `next_run_at <= now` excludes it without
    /// this query needing to know what kinds of trigger exist.
    pub fn due_routines(&self, now: i64) -> Result<Vec<Routine>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT r.id,r.agent_id,r.name,r.what,r.fires,r.active,r.next_run_at,r.last_run_at,r.created_at,r.skip_if_working
               FROM routines r
               JOIN agents a ON a.id = r.agent_id
              WHERE r.next_run_at <= ?1 AND r.active = 1 AND a.lifecycle = 'active'
              ORDER BY r.next_run_at",
        )?;
        let rows = stmt.query_map(params![now], row_to_routine)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    /// Records that a routine ran, and what became of its slot.
    ///
    /// A one-shot is removed rather than left with a time in the past, so the
    /// scheduler never has to reason about whether something already happened.
    /// A routine that holds no slot keeps its row: reading "nothing on the
    /// clock" and "finished" off the same answer would delete an event routine
    /// the first time it fired.
    pub fn routine_ran(&self, routine: &Routine, now: i64) -> Result<(), StoreError> {
        let conn = self.conn()?;
        match routine.after_running(now) {
            NextSlot::Due(next) => {
                conn.execute(
                    "UPDATE routines SET next_run_at=?2, last_run_at=?3 WHERE id=?1",
                    params![routine.id.to_string(), next, now],
                )?;
            }
            NextSlot::Waiting => {
                conn.execute(
                    "UPDATE routines SET next_run_at=NULL, last_run_at=?2 WHERE id=?1",
                    params![routine.id.to_string(), now],
                )?;
            }
            NextSlot::Done => {
                conn.execute("DELETE FROM routines WHERE id=?1", params![routine.id.to_string()])?;
            }
        }
        Ok(())
    }

    /// Turns a routine off, or back on.
    ///
    /// Deliberately not part of `update_routine`: switching something off is
    /// not an edit to what it says, and it must not move the next firing. A
    /// routine turned back on comes due at the slot it was already holding, or
    /// at the next one the trigger allows if that has gone by; the scheduler
    /// does the second part on its own, because an overdue slot fires once.
    pub fn set_routine_active(&self, id: RoutineId, active: bool) -> Result<Routine, StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE routines SET active=?2 WHERE id=?1",
            params![id.to_string(), i64::from(active)],
        )?;
        if changed == 0 {
            return Err(StoreError::RoutineNotFound(id));
        }
        drop(conn);
        self.get_routine(id)?.ok_or(StoreError::RoutineNotFound(id))
    }

    /// Records that a routine fired, and under which run.
    ///
    /// Separate from `routine_ran`, which moves the schedule: a test run fires
    /// without the schedule moving at all, and both have to appear in the
    /// history or "did it run on Tuesday" has no answer.
    ///
    /// `run` is `None` for a firing that was skipped, which started no run and
    /// therefore has nothing to thread back to.
    pub fn record_routine_run(
        &self,
        id: RoutineId,
        run: Option<RunId>,
        kind: RunKind,
        at: i64,
    ) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO routine_runs (routine_id,run_id,kind,at) VALUES (?1,?2,?3,?4)",
            params![id.to_string(), run.map(|run| run.to_string()), kind.as_str(), at],
        )?;
        Ok(())
    }

    /// What a routine has done lately, newest first, and what each firing spent.
    ///
    /// The spend is joined rather than stored on the row. A firing's cost is
    /// not known when it is recorded and keeps moving until the run settles, so
    /// a column would be a snapshot of a number that was still changing; the
    /// model calls are already filed under the run id. Read at the moment the
    /// operator looks, it also answers the question the history is actually for:
    /// a firing that bought no model call is a routine that did not run, and
    /// nothing else in the row tells the two apart.
    pub fn routine_runs(&self, id: RoutineId, limit: usize) -> Result<Vec<RoutineRun>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT r.run_id, r.kind, r.at,
                    COALESCE(SUM(u.prompt),0), COALESCE(SUM(u.completion),0),
                    SUM(u.cost), COUNT(u.id)
               FROM routine_runs r
               LEFT JOIN usage u ON u.run_id = r.run_id
              WHERE r.routine_id=?1
              GROUP BY r.id
              ORDER BY r.at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![id.to_string(), limit as i64], row_to_routine_run)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
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

    // ---- approvals -------------------------------------------------------

    /// Records a request and returns it pending.
    #[allow(clippy::too_many_arguments)]
    pub fn create_approval(
        &self,
        agent: AgentId,
        group: GroupId,
        run: RunId,
        request: Request,
        summary: &str,
        detail: &[DetailField],
    ) -> Result<Approval, StoreError> {
        let conn = self.conn()?;
        // NULL for a permission, and for a question that takes a written
        // answer. An empty array is a real state on a question and is not this.
        let options = match &request {
            Request::Question { options } => {
                Some(serde_json::to_string(options).unwrap_or_else(|_| "[]".into()))
            }
            Request::Permission { .. } => None,
        };
        let approval = Approval {
            id: ApprovalId::new(),
            agent_id: agent,
            group_id: group,
            run_id: run,
            request,
            summary: summary.to_string(),
            detail: detail.to_vec(),
            state: ApprovalState::Pending,
            answer: None,
            created_at: now_ms(),
            decided_at: None,
        };

        conn.execute(
            "INSERT INTO approvals (id,agent_id,group_id,run_id,action,summary,detail,state,created_at,options)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                approval.id.to_string(),
                agent.to_string(),
                group.to_string(),
                run.to_string(),
                approval.request.as_str(),
                approval.summary,
                serde_json::to_string(&approval.detail).unwrap_or_else(|_| "[]".into()),
                approval.state.as_str(),
                approval.created_at,
                options,
            ],
        )?;

        Ok(approval)
    }

    pub fn get_approval(&self, id: ApprovalId) -> Result<Option<Approval>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,agent_id,group_id,run_id,action,summary,detail,state,created_at,decided_at,
                    options,answer
               FROM approvals WHERE id=?1",
        )?;
        match stmt.query_row(params![id.to_string()], row_to_approval).optional()? {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Settles a request, and only from pending.
    ///
    /// The `state='pending'` in the WHERE clause is the whole point: the
    /// operator clicking Allow and the turn's own timeout both land here, and
    /// whichever arrives second must not overwrite the first. Losing that race
    /// is how a request the agent has already given up on gets recorded as
    /// granted, leaving a standing grant for an action that never happened.
    pub fn settle_approval(
        &self,
        id: ApprovalId,
        state: ApprovalState,
    ) -> Result<Approval, StoreError> {
        self.settle_approval_with(id, state, None)
    }

    /// The same, carrying what the operator said.
    ///
    /// Separate from [`Self::settle_approval`] rather than an extra argument on
    /// it, because three of the four things that settle a row have nothing to
    /// say: a verdict is the state itself, and a timeout and a stop are not
    /// anybody speaking. Only a question's answer is a value.
    pub fn answer_approval(&self, id: ApprovalId, answer: &str) -> Result<Approval, StoreError> {
        self.settle_approval_with(id, ApprovalState::Answered, Some(answer))
    }

    fn settle_approval_with(
        &self,
        id: ApprovalId,
        state: ApprovalState,
        answer: Option<&str>,
    ) -> Result<Approval, StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE approvals SET state=?2, decided_at=?3, answer=?4
              WHERE id=?1 AND state='pending'",
            params![id.to_string(), state.as_str(), now_ms(), answer],
        )?;
        if changed == 0 {
            return match self.get_approval(id)? {
                Some(existing) => Err(StoreError::ApprovalSettled { state: existing.state }),
                None => Err(StoreError::ApprovalNotFound(id)),
            };
        }
        self.get_approval(id)?.ok_or(StoreError::ApprovalNotFound(id))
    }

    /// Whether this agent has already been let off asking about this action.
    pub fn has_standing_grant(
        &self,
        agent: AgentId,
        action: ProtectedAction,
    ) -> Result<bool, StoreError> {
        let conn = self.conn()?;
        let count: i64 = conn.query_row(
            "SELECT count(*) FROM approvals
              WHERE agent_id=?1 AND action=?2 AND state='alwaysAllow'",
            params![agent.to_string(), action.as_str()],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// What this agent has been let off asking about.
    pub fn standing_grants(&self, agent: AgentId) -> Result<Vec<ProtectedAction>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT action FROM approvals
              WHERE agent_id=?1 AND state='alwaysAllow' ORDER BY action",
        )?;
        let rows = stmt.query_map(params![agent.to_string()], |row| row.get::<_, String>(0))?;

        let mut out = Vec::new();
        for row in rows {
            let raw = row?;
            out.push(
                ProtectedAction::parse(&raw).ok_or_else(|| {
                    StoreError::Corrupt(format!("unknown protected action {raw:?}"))
                })?,
            );
        }
        Ok(out)
    }

    /// Takes a standing grant back, so the next attempt asks again.
    ///
    /// The rows go rather than moving to another state. Every state this table
    /// has is a thing the operator did at the time, and a revoked grant is not
    /// one of them: recording it as denied or expired would put words in their
    /// mouth about a request they actually allowed. What is lost is one line of
    /// history; what is kept is that no state ever means two things.
    pub fn revoke_grant(
        &self,
        agent: AgentId,
        action: ProtectedAction,
    ) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute(
            "DELETE FROM approvals WHERE agent_id=?1 AND action=?2 AND state='alwaysAllow'",
            params![agent.to_string(), action.as_str()],
        )?)
    }

    /// What every recent request came to, for the UI to draw its widgets from.
    ///
    /// Bounded because this seeds a lookup table in the webview and a workspace
    /// that has been running for a year should not send a year of decisions to
    /// draw the four requests still on screen.
    pub fn approval_states(
        &self,
        limit: u32,
    ) -> Result<HashMap<ApprovalId, ApprovalState>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT id,state FROM approvals ORDER BY created_at DESC, id DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut out = HashMap::new();
        for row in rows {
            let (id, state) = row?;
            let id = id.parse::<ApprovalId>().map_err(|e| StoreError::Corrupt(e.to_string()))?;
            let state = ApprovalState::parse(&state)
                .ok_or_else(|| StoreError::Corrupt(format!("unknown approval state {state:?}")))?;
            out.insert(id, state);
        }
        Ok(out)
    }

    /// What one run is still waiting on the operator to answer.
    ///
    /// Read from the row rather than from the runtime's map of waiters, because
    /// the map is keyed by request and carries no run: the row is where the two
    /// are related, and it is the record everywhere else in this subsystem.
    pub fn pending_approvals_for_run(&self, run: RunId) -> Result<Vec<ApprovalId>, StoreError> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT id FROM approvals WHERE run_id=?1 AND state='pending'")?;
        let rows = stmt.query_map(params![run.to_string()], |row| row.get::<_, String>(0))?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row?.parse::<ApprovalId>().map_err(|e| StoreError::Corrupt(e.to_string()))?);
        }
        Ok(out)
    }

    /// Every request still waiting on the operator, oldest first.
    ///
    /// Read whole rather than as ids, because the caller is the menu bar and
    /// what it needs is the wording: a row that says only that something is
    /// pending is a row that cannot be answered without opening the window.
    ///
    /// Oldest first so the request that has least of its ten minutes left is
    /// the one at the top, and bounded for the same reason as
    /// [`Self::approval_states`].
    pub fn pending_approvals(&self, limit: u32) -> Result<Vec<Approval>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,agent_id,group_id,run_id,action,summary,detail,state,created_at,decided_at,
                    options,answer
               FROM approvals WHERE state='pending'
              ORDER BY created_at ASC, id ASC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], row_to_approval)?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    /// Expires everything still waiting. Called at startup.
    ///
    /// The turn that raised a request is holding a channel in memory, so a
    /// restart takes every waiter with it. Left pending, those rows would draw
    /// live buttons that answer nothing.
    pub fn expire_pending_approvals(&self) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute(
            "UPDATE approvals SET state='expired', decided_at=?1 WHERE state='pending'",
            params![now_ms()],
        )?)
    }

    /// Removes an agent's requests along with the agent, standing grants
    /// included: a name can be reused, and the next holder of it inherits
    /// nothing the operator granted this one.
    pub fn delete_agent_approvals(&self, agent: AgentId) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute("DELETE FROM approvals WHERE agent_id=?1", params![agent.to_string()])?)
    }

    // ---- connectors ------------------------------------------------------

    pub fn create_connector(&self, clean: &CleanConnector) -> Result<Connector, StoreError> {
        let conn = self.conn()?;
        let now = now_ms();
        let id = ConnectorId::new();

        conn.execute(
            "INSERT INTO connectors
                (id,group_id,service,account,env_var,secret,note,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8)",
            params![
                id.to_string(),
                clean.group_id.to_string(),
                clean.service,
                clean.account,
                clean.env_var,
                clean.secret,
                clean.note,
                now,
            ],
        )
        .map_err(|e| classify_connector(e, &clean.env_var))?;

        self.get_connector(id)?.ok_or(StoreError::ConnectorNotFound(id))
    }

    pub fn get_connector(&self, id: ConnectorId) -> Result<Option<Connector>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("{CONNECTOR_COLUMNS} WHERE id=?1"))?;
        match stmt.query_row(params![id.to_string()], row_to_connector).optional()? {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Every credential one crew holds. Secrets are reported as set or not,
    /// never returned: this is what the UI and the prompt are both built from.
    pub fn group_connectors(&self, group: GroupId) -> Result<Vec<Connector>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(&format!("{CONNECTOR_COLUMNS} WHERE group_id=?1 ORDER BY service, rowid"))?;
        let rows = stmt.query_map(params![group.to_string()], row_to_connector)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    /// The credentials one group's machines are given, as environment
    /// variables.
    ///
    /// The only path a stored secret takes out of this table, and it leads
    /// straight into a sandbox. Nothing here is ever rendered into a prompt or
    /// returned over IPC, which is why it is a separate query rather than a
    /// field on `Connector`.
    pub fn connector_env(
        &self,
        group: GroupId,
    ) -> Result<std::collections::BTreeMap<String, String>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT env_var, secret FROM connectors
              WHERE group_id=?1 AND env_var <> '' AND secret <> ''",
        )?;
        let rows = stmt.query_map(params![group.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = std::collections::BTreeMap::new();
        for row in rows {
            let (name, value) = row?;
            out.insert(name, value);
        }
        Ok(out)
    }

    pub fn delete_connector(&self, id: ConnectorId) -> Result<bool, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute("DELETE FROM connectors WHERE id=?1", params![id.to_string()])? > 0)
    }

    // ---- repositories ----------------------------------------------------

    /// Links a directory to a crew. Nobody is given it here.
    ///
    /// Adding and handing out are two decisions, exactly as connecting a plugin
    /// and choosing who may spend it are. A repository that arrived already
    /// reaching the crew would hand the operator's own source to every agent in
    /// it at the moment they linked it, which is the one thing this feature must
    /// not do by accident.
    ///
    /// `clean.path` is expected canonical: [`crate::repo::verify`] is what makes
    /// it so, and the unique index below is what needs it to have happened.
    pub fn create_repository(&self, clean: &CleanRepository) -> Result<Repository, StoreError> {
        let conn = self.conn()?;
        let now = now_ms();
        let id = RepositoryId::new();

        conn.execute(
            "INSERT INTO repositories (id,group_id,name,path,note,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?6)",
            params![
                id.to_string(),
                clean.group_id.to_string(),
                clean.name,
                clean.path,
                clean.note,
                now,
            ],
        )
        .map_err(|e| classify_repository(e, &clean.path))?;

        self.get_repository(id)?.ok_or(StoreError::RepositoryNotFound(id))
    }

    pub fn get_repository(&self, id: RepositoryId) -> Result<Option<Repository>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("{REPOSITORY_COLUMNS} WHERE id=?1"))?;
        let found = stmt.query_row(params![id.to_string()], row_to_repository).optional()?;
        match found {
            Some(row) => {
                let mut repo = row?;
                repo.reach = self.repository_reach(&conn, id)?;
                Ok(Some(repo))
            }
            None => Ok(None),
        }
    }

    /// Every repository one crew has linked, with who may work in each.
    ///
    /// What the panel is drawn from and what the operator answers with. Two
    /// queries rather than a join, because a repository nobody has been given is
    /// a row the panel has to show: an inner join drops exactly the state the
    /// operator is in the middle of fixing.
    pub fn group_repositories(&self, group: GroupId) -> Result<Vec<Repository>, StoreError> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare(&format!("{REPOSITORY_COLUMNS} WHERE group_id=?1 ORDER BY name, rowid"))?;
        let rows = stmt.query_map(params![group.to_string()], row_to_repository)?;

        let mut reach: HashMap<RepositoryId, Vec<AgentId>> = HashMap::new();
        let mut names = conn.prepare(
            "SELECT repository_access.repository_id, repository_access.agent_id
               FROM repository_access
               JOIN repositories ON repositories.id = repository_access.repository_id
              WHERE repositories.group_id = ?1
              ORDER BY repository_access.rowid",
        )?;
        let pairs = names.query_map(params![group.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for pair in pairs {
            let (repo, agent) = pair?;
            let (Ok(repo), Ok(agent)) = (repo.parse::<RepositoryId>(), agent.parse::<AgentId>())
            else {
                continue;
            };
            reach.entry(repo).or_default().push(agent);
        }

        let mut out = Vec::new();
        for row in rows {
            let mut repo = row??;
            repo.reach = reach.remove(&repo.id).unwrap_or_default();
            out.push(repo);
        }
        Ok(out)
    }

    /// Every repository in the workspace, with who may work in each.
    ///
    /// Workspace-wide rather than per group because the rail is: it draws
    /// crews and their contents from one roster, and a second read per crew
    /// would make the number of round trips the number of crews. The caller
    /// filters by group exactly as it already does for agents.
    pub fn repositories(&self) -> Result<Vec<Repository>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("{REPOSITORY_COLUMNS} ORDER BY name, rowid"))?;
        let rows = stmt.query_map([], row_to_repository)?;

        let mut reach: HashMap<RepositoryId, Vec<AgentId>> = HashMap::new();
        let mut names =
            conn.prepare("SELECT repository_id, agent_id FROM repository_access ORDER BY rowid")?;
        let pairs =
            names.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        for pair in pairs {
            let (repo, agent) = pair?;
            let (Ok(repo), Ok(agent)) = (repo.parse::<RepositoryId>(), agent.parse::<AgentId>())
            else {
                continue;
            };
            reach.entry(repo).or_default().push(agent);
        }

        let mut out = Vec::new();
        for row in rows {
            let mut repo = row??;
            repo.reach = reach.remove(&repo.id).unwrap_or_default();
            out.push(repo);
        }
        Ok(out)
    }

    /// The repositories one agent may actually work in.
    ///
    /// Read on the hot path: it decides what the turn is offered and what the
    /// prompt says, and those two have to be one answer. Separate from
    /// [`Store::group_repositories`] for the reason `plugin_tools` is separate
    /// from `group_plugins`.
    ///
    /// The group is compared as well as the name, and that is not belt and
    /// braces. An agent moved to another crew keeps its rows here until
    /// something clears them, and a repository is the crew's rather than the
    /// agent's: reach that survived the move would be one crew's source open to
    /// another crew's agent, which is the one thing group scoping exists to stop.
    pub fn agent_repositories(&self, agent: AgentId) -> Result<Vec<Repository>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "{REPOSITORY_COLUMNS}
              WHERE {REPOSITORY_REACHED_BY_AGENT}
                AND group_id = (SELECT group_id FROM agents WHERE id = :agent)
              ORDER BY name, rowid"
        ))?;
        let rows =
            stmt.query_map(named_params! { ":agent": agent.to_string() }, row_to_repository)?;
        let mut out = Vec::new();
        for row in rows {
            let mut repo = row??;
            repo.reach = self.repository_reach(&conn, repo.id)?;
            out.push(repo);
        }
        Ok(out)
    }

    /// Renames one, or rewrites the line its agents read.
    ///
    /// The path is not among them, and that is the design. The path is what the
    /// row *is*: reach was granted for that directory, so editing it in place
    /// would move every named agent's boundary without anything on screen
    /// saying a decision had been taken. A different directory is a different
    /// repository, linked and handed out on purpose.
    pub fn update_repository(
        &self,
        id: RepositoryId,
        name: &str,
        note: &str,
    ) -> Result<Repository, StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE repositories SET name=?2, note=?3, updated_at=?4 WHERE id=?1",
            params![id.to_string(), name, note, now_ms()],
        )?;
        if changed == 0 {
            return Err(StoreError::RepositoryNotFound(id));
        }
        self.get_repository(id)?.ok_or(StoreError::RepositoryNotFound(id))
    }

    /// Unlinks a repository. Nothing on disk is touched.
    ///
    /// Worth saying out loud because the button is next to a path: this drops
    /// Guaca's record of a directory and every agent's reach into it. The
    /// operator's files are their own.
    pub fn delete_repository(&self, id: RepositoryId) -> Result<bool, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM repository_access WHERE repository_id=?1",
            params![id.to_string()],
        )?;
        let gone = tx.execute("DELETE FROM repositories WHERE id=?1", params![id.to_string()])?;
        tx.commit()?;
        Ok(gone > 0)
    }

    /// Gives one agent a repository, or takes it back.
    ///
    /// One agent at a time rather than a whole list, because that is what the
    /// operator does: a list write would need the panel to send back every name
    /// it believes in, and a panel that is one tick behind would quietly revoke
    /// somebody while granting somebody else.
    ///
    /// An agent from another crew is refused rather than stored. A row that
    /// `agent_repositories` filters out on every read is a name the panel draws
    /// as granted and the runtime treats as absent, which is the disagreement
    /// this whole table exists to avoid.
    pub fn set_repository_access(
        &self,
        id: RepositoryId,
        agent: AgentId,
        allowed: bool,
    ) -> Result<Repository, StoreError> {
        let conn = self.conn()?;
        let group: String = conn
            .query_row(
                "SELECT group_id FROM repositories WHERE id=?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StoreError::RepositoryNotFound(id))?;

        if allowed {
            let theirs: Option<String> = conn
                .query_row(
                    "SELECT group_id FROM agents WHERE id=?1",
                    params![agent.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            if theirs.as_deref() != Some(group.as_str()) {
                return Err(StoreError::AgentNotInGroupForRepository(agent));
            }
            conn.execute(
                "INSERT OR IGNORE INTO repository_access (repository_id, agent_id) VALUES (?1, ?2)",
                params![id.to_string(), agent.to_string()],
            )?;
        } else {
            conn.execute(
                "DELETE FROM repository_access WHERE repository_id=?1 AND agent_id=?2",
                params![id.to_string(), agent.to_string()],
            )?;
        }

        self.get_repository(id)?.ok_or(StoreError::RepositoryNotFound(id))
    }

    /// Drops one agent's reach into every repository that named it.
    ///
    /// Called when an agent is retired, for the reason its plugin permissions
    /// are: a name in a panel that points at nobody is a decision the operator
    /// cannot see the shape of, and a retired agent must not leave a working
    /// tree looking like it is handed out to somebody.
    pub fn delete_agent_repository_access(&self, agent: AgentId) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute(
            "DELETE FROM repository_access WHERE agent_id=?1",
            params![agent.to_string()],
        )?)
    }

    fn repository_reach(
        &self,
        conn: &rusqlite::Connection,
        id: RepositoryId,
    ) -> Result<Vec<AgentId>, StoreError> {
        let mut stmt = conn.prepare(
            "SELECT agent_id FROM repository_access WHERE repository_id=?1 ORDER BY rowid",
        )?;
        let rows = stmt.query_map(params![id.to_string()], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            if let Ok(agent) = row?.parse::<AgentId>() {
                out.push(agent);
            }
        }
        Ok(out)
    }

    // ---- plugins ---------------------------------------------------------

    /// The plugins one crew has connected. No grant is on this type and there
    /// is no command that returns one.
    pub fn group_plugins(&self, group: GroupId) -> Result<Vec<Plugin>, StoreError> {
        let conn = self.conn()?;
        let chosen = self.chosen_agents(&conn, group)?;
        let tools = self.tool_access(&conn, group)?;
        let mut stmt =
            conn.prepare(&format!("{PLUGIN_COLUMNS} WHERE group_id=?1 ORDER BY kind, rowid"))?;
        let rows =
            stmt.query_map(params![group.to_string()], |row| row_to_plugin(row, &chosen, &tools))?;
        let mut out = Vec::new();
        for row in rows {
            // A row whose kind is not one this build knows is skipped rather
            // than raised. It can only come from a newer build writing to the
            // same file, and a crew losing one tool list is better than every
            // agent in it losing its turn.
            if let Some(plugin) = row?? {
                out.push(plugin);
            }
        }
        Ok(out)
    }

    /// Every named agent in one group's plugins, by the plugin that named them.
    ///
    /// One query for the whole group rather than one per row: this is read to
    /// draw a settings panel and to build a roster, and a query per plugin
    /// would be five for a screen that shows five tiles.
    fn chosen_agents(
        &self,
        conn: &Conn,
        group: GroupId,
    ) -> Result<HashMap<PluginId, Vec<AgentId>>, StoreError> {
        let mut stmt = conn.prepare(
            "SELECT plugin_agents.plugin_id, plugin_agents.agent_id
               FROM plugin_agents
               JOIN plugins ON plugins.id = plugin_agents.plugin_id
              WHERE plugins.group_id = ?1
              ORDER BY plugin_agents.rowid",
        )?;
        let rows = stmt.query_map(params![group.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut out: HashMap<PluginId, Vec<AgentId>> = HashMap::new();
        for row in rows {
            let (plugin, agent) = row?;
            let (Ok(plugin), Ok(agent)) = (plugin.parse::<PluginId>(), agent.parse::<AgentId>())
            else {
                // An id that will not parse names nothing, so it grants
                // nothing. Dropping it is the fail-closed reading, and the
                // alternative is a settings panel nobody can open.
                continue;
            };
            out.entry(plugin).or_default().push(agent);
        }
        Ok(out)
    }

    /// Who may call each narrowed tool, by the plugin and tool it belongs to.
    ///
    /// The narrowings are what is stored, so a tool missing from this map is
    /// one every agent the plugin reaches may call: that is what every plugin
    /// connected before this control existed offers, and what a tool a vendor
    /// ships next month offers. One query for the whole group, for the reason
    /// [`Store::chosen_agents`] is one: this is read to draw a panel and to
    /// build a turn, and neither wants five queries for five tiles.
    ///
    /// Two statements rather than a join, because the empty set is meaningful
    /// and a join cannot return one. A tool narrowed to nobody has a row in
    /// `plugin_tool_access` and none in `plugin_tool_agents`, and an inner join
    /// would drop it back to "everyone" — the one mistake this whole shape is
    /// built to make impossible.
    fn tool_access(
        &self,
        conn: &Conn,
        group: GroupId,
    ) -> Result<HashMap<(PluginId, String), PluginAccess>, StoreError> {
        let mut named: HashMap<(PluginId, String), Vec<AgentId>> = HashMap::new();
        let mut stmt = conn.prepare(
            "SELECT plugin_tool_agents.plugin_id, plugin_tool_agents.tool,
                    plugin_tool_agents.agent_id
               FROM plugin_tool_agents
               JOIN plugins ON plugins.id = plugin_tool_agents.plugin_id
              WHERE plugins.group_id = ?1
              ORDER BY plugin_tool_agents.rowid",
        )?;
        let rows = stmt.query_map(params![group.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?;
        for row in rows {
            let (plugin, tool, agent) = row?;
            // An id that will not parse names nothing, so it grants nothing.
            // Dropping it is the fail-closed reading: the tool keeps its
            // narrowing and loses one name, rather than the narrowing being
            // lost with it.
            let (Ok(plugin), Ok(agent)) = (plugin.parse::<PluginId>(), agent.parse::<AgentId>())
            else {
                continue;
            };
            named.entry((plugin, tool)).or_default().push(agent);
        }

        let mut stmt = conn.prepare(
            "SELECT plugin_tool_access.plugin_id, plugin_tool_access.tool,
                    plugin_tool_access.access
               FROM plugin_tool_access
               JOIN plugins ON plugins.id = plugin_tool_access.plugin_id
              WHERE plugins.group_id = ?1",
        )?;
        let rows = stmt.query_map(params![group.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?;

        let mut out: HashMap<(PluginId, String), PluginAccess> = HashMap::new();
        for row in rows {
            let (plugin, tool, access) = row?;
            // An id that will not parse names no plugin this can be read
            // against: every reader of this map looks up a row whose own id
            // parsed, and a row whose id does not parse raises rather than
            // being offered. Dropping it here loses nothing.
            let Ok(plugin) = plugin.parse::<PluginId>() else { continue };
            let key = (plugin, tool);
            let agents = named.remove(&key).unwrap_or_default();
            out.insert(key, PluginAccess::from_row(&access, agents));
        }
        Ok(out)
    }

    /// What one agent can call, with the schemas the model is shown, and what
    /// it cannot, split by whose it is instead.
    ///
    /// Separate from `group_plugins` for the reason `connector_env` is separate
    /// from `group_connectors`: this is the bulk nothing but the turn needs,
    /// and it is read on the hot path rather than to draw a list.
    ///
    /// Filtered by agent rather than by group, on both axes, because what is
    /// offered to a model has to be what that model may actually call. The
    /// prompt is built from the same list on the same turn, so an agent is
    /// never told about a plugin or a tool it would be refused.
    ///
    /// The ones it cannot call come back beside them rather than being dropped
    /// here, because the prompt names them: an agent that is simply not shown
    /// `refund` answers "we cannot do refunds", when the true answer is either
    /// that the operator switched refunds off and can switch them back on, or
    /// that the agent next to it does refunds. Which of those it is decides
    /// which list it lands in. Nothing about either reaches the tool
    /// definitions.
    ///
    /// The tool half of the rule is applied here, in Rust, and in SQL in
    /// [`Store::plugin_reach`]. Both compare the server's own unprefixed name
    /// to the same stored string, and a test drives one refusal through both.
    /// It is not the fragment `PLUGIN_REACHED_BY_AGENT` is, because the tool
    /// list is a JSON column: there is nothing for SQL to filter here without
    /// taking the tools apart inside the database.
    pub fn plugin_tools(
        &self,
        group: GroupId,
        agent: AgentId,
    ) -> Result<Vec<PluginToolset>, StoreError> {
        let conn = self.conn()?;
        let narrowed = self.tool_access(&conn, group)?;
        let mut stmt = conn.prepare(&format!(
            "SELECT id, kind, tools FROM plugins
              WHERE group_id=:group AND {PLUGIN_REACHED_BY_AGENT}
              ORDER BY kind, rowid"
        ))?;
        let rows = stmt.query_map(
            named_params! { ":group": group.to_string(), ":agent": agent.to_string() },
            |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            },
        )?;

        let mut out = Vec::new();
        for row in rows {
            let (id, kind, tools) = row?;
            let Some(kind) = PluginKind::from_slug(&kind) else { continue };
            let tools: Vec<PluginTool> = serde_json::from_str(&tools)
                .map_err(|e| StoreError::Corrupt(format!("bad tool list for {kind:?}: {e}")))?;
            // Raised rather than skipped, so that a row whose id cannot be read
            // fails the turn instead of quietly losing every narrowing filed
            // against it. `group_plugins` reads the same id the same way.
            let id = id
                .parse::<PluginId>()
                .map_err(|e| StoreError::Corrupt(format!("bad plugin id {id:?}: {e}")))?;

            let mut set = PluginToolset {
                kind,
                offered: Vec::new(),
                withheld: Vec::new(),
                elsewhere: Vec::new(),
            };
            for tool in tools {
                match narrowed.get(&(id, tool.name.clone())) {
                    Some(access) if !access.allows(agent) => {
                        // Nobody's, or somebody else's. The two are different
                        // sentences in the prompt and different refusals on the
                        // call path, and this is the same order `plugin_reach`
                        // asks in: what is true of everybody first.
                        if access.allows_nobody() {
                            set.withheld.push(tool.name);
                        } else {
                            set.elsewhere.push(tool.name);
                        }
                    }
                    _ => set.offered.push(tool),
                }
            }
            out.push(set);
        }
        Ok(out)
    }

    /// What one agent gets when it calls one tool on one of its crew's plugins.
    ///
    /// The only path a stored token takes out of this table, and it leads
    /// straight back to the server that issued it. Nothing here reaches a
    /// prompt, a transcript, an event or the webview.
    ///
    /// Five answers rather than an `Option`, because four of them are refusals
    /// an agent reads mid-turn and they call for different things. "Nobody has
    /// connected this" is something to tell the operator about; "you are not
    /// one of the agents it was connected for" and "this tool is not one of
    /// yours" are things to hand to a peer; "this tool is switched off" is
    /// neither, because no peer has it either. Collapsing any two would have an
    /// agent asking an operator to connect a plugin they are looking at in the
    /// settings panel, or handing a peer work nobody in the crew can do.
    ///
    /// The tool is asked about here and not only where the definitions are
    /// built, because a model names tools it was never offered.
    ///
    /// Asked most-general first, because more than one of these is true at
    /// once and the widest true answer is the useful one. A tool narrowed to
    /// nobody is off for the whole crew, so an agent that is also not on the
    /// plugin is told that rather than sent to a peer who would be refused in
    /// turn. A plugin this agent is not on is refused before one of its tools
    /// is, because that answer covers every tool the plugin has rather than
    /// this one.
    pub fn plugin_reach(
        &self,
        group: GroupId,
        agent: AgentId,
        kind: PluginKind,
        tool: &str,
    ) -> Result<PluginReach, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT id,access_token,refresh_token,expires_at,client_id,client_secret,
                    token_endpoint,{PLUGIN_REACHED_BY_AGENT},
                    {PLUGIN_TOOL_REACHED_BY_AGENT},{PLUGIN_TOOL_REACHED_BY_ANYONE}
               FROM plugins WHERE group_id=:group AND kind=:kind",
        ))?;
        let row = stmt
            .query_row(
                named_params! {
                    ":group": group.to_string(),
                    ":kind": kind.slug(),
                    ":agent": agent.to_string(),
                    ":tool": tool,
                },
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, i64>(7)? != 0,
                        row.get::<_, i64>(8)? != 0,
                        row.get::<_, i64>(9)? != 0,
                    ))
                },
            )
            .optional()?;

        let Some((
            id,
            access,
            refresh,
            expires_at,
            client_id,
            client_secret,
            token_endpoint,
            reached,
            tool_mine,
            tool_anyones,
        )) = row
        else {
            return Ok(PluginReach::NotConnected);
        };
        if !tool_anyones {
            return Ok(PluginReach::ToolDenied);
        }
        if !reached {
            return Ok(PluginReach::NotChosen);
        }
        if !tool_mine {
            return Ok(PluginReach::ToolNotChosen);
        }
        let id = id
            .parse::<PluginId>()
            .map_err(|e| StoreError::Corrupt(format!("bad plugin id {id:?}: {e}")))?;

        // An empty access token is a plugin that needed no sign-in, not a
        // broken one. A server that authorizes everybody stores no grant, and
        // reporting one with nothing in it would send the operator to a browser
        // to fix something that is working, and put `Bearer ` on every call.
        let grant = (!access.is_empty()).then(|| crate::oauth::Grant {
            access_token: access,
            refresh_token: (!refresh.is_empty()).then_some(refresh),
            expires_at,
            client_id,
            client_secret: (!client_secret.is_empty()).then_some(client_secret),
            token_endpoint,
        });
        Ok(PluginReach::Granted { id, grant })
    }

    /// Narrows a plugin to some of the crew, or opens it to all of it.
    ///
    /// The whole set is replaced, never merged. A caller that sent the agents
    /// it knew about would narrow a plugin by forgetting one, and the panel
    /// that sends this renders what it last read.
    pub fn set_plugin_access(
        &self,
        id: PluginId,
        access: &PluginAccess,
    ) -> Result<Plugin, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;

        let group: String = tx
            .query_row("SELECT group_id FROM plugins WHERE id=?1", params![id.to_string()], |row| {
                row.get(0)
            })
            .optional()?
            .ok_or(StoreError::PluginNotFound(id))?;

        if let PluginAccess::Chosen { agents } = access {
            for agent in agents {
                // Refused rather than filtered out. An agent from another crew
                // could never reach this plugin anyway, because the turn asks
                // by group as well as by agent, so storing one would be a row
                // that means nothing and a settings panel drawing a name that
                // grants nothing.
                let known: Option<i64> = tx
                    .query_row(
                        "SELECT 1 FROM agents
                          WHERE id=?1 AND group_id=?2 AND lifecycle<>'terminated'",
                        params![agent.to_string(), group],
                        |row| row.get(0),
                    )
                    .optional()?;
                if known.is_none() {
                    return Err(StoreError::AgentNotInGroup(*agent));
                }
            }
        }

        tx.execute(
            "UPDATE plugins SET access=?2 WHERE id=?1",
            params![id.to_string(), access.as_str()],
        )?;
        tx.execute("DELETE FROM plugin_agents WHERE plugin_id=?1", params![id.to_string()])?;
        if let PluginAccess::Chosen { agents } = access {
            for agent in agents {
                // A set, so a name sent twice is one row rather than a
                // conflict. Nothing about who may call this depends on how
                // many times the caller listed them.
                tx.execute(
                    "INSERT OR IGNORE INTO plugin_agents (plugin_id, agent_id) VALUES (?1, ?2)",
                    params![id.to_string(), agent.to_string()],
                )?;
            }
        }
        tx.commit()?;

        let group = group
            .parse::<GroupId>()
            .map_err(|e| StoreError::Corrupt(format!("bad group id {group:?}: {e}")))?;
        self.group_plugins(group)?
            .into_iter()
            .find(|plugin| plugin.id == id)
            .ok_or(StoreError::PluginNotFound(id))
    }

    /// Chooses which of a crew's agents may call one of a plugin's tools.
    ///
    /// One named tool, and the whole answer for it. Naming the tool is what
    /// keeps this from being the whole-set write [`Store::set_plugin_access`]
    /// is: a panel that sent every tool it could see would drop a narrowing
    /// filed against a tool the vendor has temporarily stopped publishing.
    /// Inside the named tool the set is replaced rather than merged, for the
    /// reason it is there — a caller sending the agents it happened to know
    /// would narrow the tool by forgetting somebody, and the panel that sends
    /// this renders what it last read.
    ///
    /// [`PluginAccess::Everyone`] deletes the rows rather than storing them.
    /// The absence of a row is the permission, so a tool put back to everyone
    /// has to look like a tool nobody ever touched: kept as a row it would be a
    /// stored `everyone` that a later reader has to treat as equivalent to
    /// absence, which is two ways to say one thing and one of them wrong.
    ///
    /// Refused for a tool this plugin does not publish. Storing one would be a
    /// decision that names nothing, in a table nothing can show the operator,
    /// and it means the panel is looking at a list older than the server's.
    pub fn set_plugin_tool(
        &self,
        id: PluginId,
        tool: &str,
        access: &PluginAccess,
    ) -> Result<Plugin, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;

        let row: Option<(String, String)> = tx
            .query_row(
                "SELECT group_id, tools FROM plugins WHERE id=?1",
                params![id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (group, tools) = row.ok_or(StoreError::PluginNotFound(id))?;

        let published: Vec<PluginTool> = serde_json::from_str(&tools)
            .map_err(|e| StoreError::Corrupt(format!("bad tool list for plugin {id}: {e}")))?;
        if !published.iter().any(|published| published.name == tool) {
            return Err(StoreError::PluginToolNotFound(id, tool.to_string()));
        }

        if let PluginAccess::Chosen { agents } = access {
            for agent in agents {
                // Refused rather than filtered out, for the reason
                // `set_plugin_access` refuses one: a row naming an agent from
                // another crew grants nothing and draws a name in a settings
                // panel that means nothing.
                let known: Option<i64> = tx
                    .query_row(
                        "SELECT 1 FROM agents
                          WHERE id=?1 AND group_id=?2 AND lifecycle<>'terminated'",
                        params![agent.to_string(), group],
                        |row| row.get(0),
                    )
                    .optional()?;
                if known.is_none() {
                    return Err(StoreError::AgentNotInGroup(*agent));
                }
            }
        }

        tx.execute(
            "DELETE FROM plugin_tool_agents WHERE plugin_id=?1 AND tool=?2",
            params![id.to_string(), tool],
        )?;
        match access {
            PluginAccess::Everyone => {
                tx.execute(
                    "DELETE FROM plugin_tool_access WHERE plugin_id=?1 AND tool=?2",
                    params![id.to_string(), tool],
                )?;
            }
            PluginAccess::Chosen { agents } => {
                tx.execute(
                    "INSERT INTO plugin_tool_access (plugin_id, tool, access) VALUES (?1, ?2, ?3)
                     ON CONFLICT(plugin_id, tool) DO UPDATE SET access=excluded.access",
                    params![id.to_string(), tool, access.as_str()],
                )?;
                for agent in agents {
                    // A set, so a name sent twice is one row rather than a
                    // conflict. Nothing about who may call this depends on how
                    // many times the caller listed them.
                    tx.execute(
                        "INSERT OR IGNORE INTO plugin_tool_agents (plugin_id, tool, agent_id)
                         VALUES (?1, ?2, ?3)",
                        params![id.to_string(), tool, agent.to_string()],
                    )?;
                }
            }
        }
        tx.commit()?;

        let group = group
            .parse::<GroupId>()
            .map_err(|e| StoreError::Corrupt(format!("bad group id {group:?}: {e}")))?;
        self.group_plugins(group)?
            .into_iter()
            .find(|plugin| plugin.id == id)
            .ok_or(StoreError::PluginNotFound(id))
    }

    /// Drops one agent's place on every plugin and every tool that named it.
    ///
    /// Called when an agent is retired, for the reason its approvals are: a
    /// permission the operator gave to an agent that no longer exists is a row
    /// nothing can act on and a name in a settings panel that points at nobody.
    ///
    /// Both tables, and the tool one matters more than it looks: a tool left
    /// naming a retired agent and nobody else is a tool the panel draws as
    /// narrowed to somebody, that in fact nobody can call, and the refusal an
    /// agent gets for it would send it to a peer that is gone.
    pub fn delete_agent_plugin_access(&self, agent: AgentId) -> Result<usize, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let mut removed =
            tx.execute("DELETE FROM plugin_agents WHERE agent_id=?1", params![agent.to_string()])?;
        removed += tx.execute(
            "DELETE FROM plugin_tool_agents WHERE agent_id=?1",
            params![agent.to_string()],
        )?;
        tx.commit()?;
        Ok(removed)
    }

    /// Records a connection, replacing whatever was there for that server.
    ///
    /// Replace rather than reject: connecting again is how an operator fixes a
    /// grant that was revoked at the vendor, and the unique index makes an
    /// insert the wrong shape for that.
    ///
    /// `access` is not in the update, and the row keeps its id, so who may call
    /// the plugin survives the sign-in being redone. Renewing a grant is not a
    /// decision about the crew, and a reconnection that quietly handed Stripe
    /// back to everybody would undo one silently, at the moment the operator
    /// was fixing something else.
    #[allow(clippy::too_many_arguments)]
    pub fn save_plugin(
        &self,
        group: GroupId,
        kind: PluginKind,
        account: &str,
        tools: &[PluginTool],
        grant: Option<&crate::oauth::Grant>,
        // Which authorized identity at the account this crew is using, for an
        // account-backed kind. Empty is the account's default, which is what a
        // row written before connections existed keeps meaning.
        connection: &str,
    ) -> Result<Plugin, StoreError> {
        let conn = self.conn()?;
        let id = PluginId::new();
        let encoded = serde_json::to_string(tools)
            .map_err(|e| StoreError::Corrupt(format!("tool list will not serialize: {e}")))?;

        conn.execute(
            "INSERT INTO plugins
                (id,group_id,kind,account,tools,client_id,client_secret,token_endpoint,
                 access_token,refresh_token,expires_at,connected_at,connection)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
             ON CONFLICT(group_id,kind) DO UPDATE SET
                account=excluded.account,
                connection=excluded.connection,
                tools=excluded.tools,
                client_id=excluded.client_id,
                client_secret=excluded.client_secret,
                token_endpoint=excluded.token_endpoint,
                access_token=excluded.access_token,
                refresh_token=excluded.refresh_token,
                expires_at=excluded.expires_at,
                connected_at=excluded.connected_at",
            params![
                id.to_string(),
                group.to_string(),
                kind.slug(),
                account,
                encoded,
                grant.map(|g| g.client_id.as_str()).unwrap_or_default(),
                grant.and_then(|g| g.client_secret.as_deref()).unwrap_or_default(),
                grant.map(|g| g.token_endpoint.as_str()).unwrap_or_default(),
                grant.map(|g| g.access_token.as_str()).unwrap_or_default(),
                grant.and_then(|g| g.refresh_token.as_deref()).unwrap_or_default(),
                grant.and_then(|g| g.expires_at),
                now_ms(),
                connection,
            ],
        )?;

        self.group_plugins(group)?
            .into_iter()
            .find(|plugin| plugin.kind == kind)
            .ok_or(StoreError::PluginNotFound(id))
    }

    /// Writes back a renewed grant, leaving everything else alone.
    pub fn refresh_plugin_grant(
        &self,
        id: PluginId,
        grant: &crate::oauth::Grant,
    ) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE plugins SET access_token=?2, refresh_token=?3, expires_at=?4 WHERE id=?1",
            params![
                id.to_string(),
                grant.access_token,
                grant.refresh_token.clone().unwrap_or_default(),
                grant.expires_at,
            ],
        )?;
        Ok(())
    }

    /// Forgets a plugin, its grant, and who was allowed to spend it.
    ///
    /// The named agents go first, and not only because the foreign key would
    /// refuse otherwise: a row naming a plugin that is gone would be a standing
    /// permission attached to nothing, waiting to attach itself to whatever
    /// took the id next.
    pub fn delete_plugin(&self, id: PluginId) -> Result<bool, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM plugin_agents WHERE plugin_id=?1", params![id.to_string()])?;
        tx.execute("DELETE FROM plugin_tool_access WHERE plugin_id=?1", params![id.to_string()])?;
        tx.execute("DELETE FROM plugin_tool_agents WHERE plugin_id=?1", params![id.to_string()])?;
        let removed = tx.execute("DELETE FROM plugins WHERE id=?1", params![id.to_string()])?;
        tx.commit()?;
        Ok(removed > 0)
    }

    // ---- sign-ins --------------------------------------------------------

    /// Records what one of an agent's two places turned out to be signed in to.
    ///
    /// Replaces that surface's whole set rather than merging, because this is a
    /// cache of something that lives elsewhere: an entry that outlives the
    /// logout it should have noticed keeps the crew routing work to an agent
    /// that will hit a login wall. `first_seen_at` is carried across so
    /// "signed in since Tuesday" survives a rescan.
    ///
    /// Scoped to the surface, and that is the whole reason the column exists. A
    /// computer and a browser are scanned independently and at different
    /// moments; a replace that took the agent's whole set would mean asking one
    /// what it holds erases everything the other reported, so an agent's
    /// accounts would flicker between two halves of the truth depending on
    /// which scan ran last.
    pub fn replace_signins(
        &self,
        agent: AgentId,
        surface: Surface,
        found: &[Signin],
    ) -> Result<Vec<Signin>, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        {
            let mut earliest = tx.prepare(
                "SELECT first_seen_at FROM signins WHERE agent_id=?1 AND surface=?2 AND domain=?3",
            )?;
            let mut insert = tx.prepare(
                "INSERT INTO signins
                   (agent_id,surface,domain,service,recognized,first_seen_at,last_seen_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
            )?;

            let mut carried: Vec<i64> = Vec::new();
            for signin in found {
                let since: Option<i64> = earliest
                    .query_row(params![agent.to_string(), surface.as_str(), signin.domain], |row| {
                        row.get(0)
                    })
                    .optional()?;
                carried.push(since.unwrap_or(signin.first_seen_at));
            }

            tx.execute(
                "DELETE FROM signins WHERE agent_id=?1 AND surface=?2",
                params![agent.to_string(), surface.as_str()],
            )?;

            for (signin, since) in found.iter().zip(carried) {
                insert.execute(params![
                    agent.to_string(),
                    surface.as_str(),
                    signin.domain,
                    signin.service,
                    signin.recognized as i64,
                    since,
                    signin.last_seen_at,
                ])?;
            }
        }
        tx.commit()?;
        self.agent_signins(agent)
    }

    /// Everything one agent reaches, wherever it holds the session.
    pub fn agent_signins(&self, agent: AgentId) -> Result<Vec<Signin>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT agent_id,surface,domain,service,recognized,first_seen_at,last_seen_at
               FROM signins WHERE agent_id=?1 ORDER BY service",
        )?;
        let rows = stmt.query_map(params![agent.to_string()], row_to_signin)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    /// What every machine in one group is signed in to.
    ///
    /// Terminated agents are excluded: their sandboxes are destroyed, so a row
    /// left behind would put an unreachable account on the roster.
    pub fn group_signins(&self, group: GroupId) -> Result<Vec<Signin>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT s.agent_id,s.surface,s.domain,s.service,s.recognized,s.first_seen_at,
                      s.last_seen_at
               FROM signins s
               JOIN agents a ON a.id = s.agent_id
              WHERE a.group_id=?1 AND a.lifecycle <> 'terminated'
              ORDER BY s.service",
        )?;
        let rows = stmt.query_map(params![group.to_string()], row_to_signin)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    /// Forgets what a machine was signed in to. Called when the agent, and
    /// therefore its sandbox and its cookies, are destroyed.
    pub fn delete_agent_signins(&self, agent: AgentId) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute("DELETE FROM signins WHERE agent_id=?1", params![agent.to_string()])?)
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
                    g.base_url, g.api_key, g.default_model,
                    g.provider, g.subscription_model, g.request_timeout_secs,
                    g.max_hops, g.max_steps_per_run, g.max_fanout_per_call,
                    g.max_sends_per_pair, g.max_tool_rounds
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
        let over = draft.inference.clone().unwrap_or_default();
        let limits = draft.limits.unwrap_or_default();
        conn.execute(
            "INSERT INTO groups (id,name,created_at,base_url,api_key,default_model,
                                 provider,subscription_model,request_timeout_secs,
                                 max_hops,max_steps_per_run,max_fanout_per_call,
                                 max_sends_per_pair,max_tool_rounds)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                id.to_string(),
                draft.name,
                now_ms(),
                over.base_url,
                draft.api_key.clone().flatten(),
                over.default_model,
                over.provider.map(|p| p.as_str()),
                over.subscription_model,
                over.request_timeout_secs,
                limits.max_hops,
                limits.max_steps_per_run,
                limits.max_fanout_per_call.map(|n| n as i64),
                limits.max_sends_per_pair,
                limits.max_tool_rounds,
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
        // Each block is written or left alone as a whole, which is what the
        // draft's two rules say. Per-column CASEs inside a block would let a
        // caller half-write a group's settings, and the second half of a save
        // that failed halfway is not a state anything here can report.
        let over = draft.inference.clone().unwrap_or_default();
        let limits = draft.limits.unwrap_or_default();
        let changed = conn
            .execute(
                "UPDATE groups
                    SET name=?2,
                        base_url             = CASE WHEN ?3 THEN ?4  ELSE base_url END,
                        default_model        = CASE WHEN ?3 THEN ?5  ELSE default_model END,
                        provider             = CASE WHEN ?3 THEN ?6  ELSE provider END,
                        subscription_model   = CASE WHEN ?3 THEN ?7  ELSE subscription_model END,
                        request_timeout_secs = CASE WHEN ?3 THEN ?8  ELSE request_timeout_secs END,
                        api_key              = CASE WHEN ?9 THEN ?10 ELSE api_key END,
                        max_hops             = CASE WHEN ?11 THEN ?12 ELSE max_hops END,
                        max_steps_per_run    = CASE WHEN ?11 THEN ?13 ELSE max_steps_per_run END,
                        max_fanout_per_call  = CASE WHEN ?11 THEN ?14 ELSE max_fanout_per_call END,
                        max_sends_per_pair   = CASE WHEN ?11 THEN ?15 ELSE max_sends_per_pair END,
                        max_tool_rounds      = CASE WHEN ?11 THEN ?16 ELSE max_tool_rounds END
                  WHERE id=?1",
                params![
                    id.to_string(),
                    draft.name,
                    draft.inference.is_some(),
                    over.base_url,
                    over.default_model,
                    over.provider.map(|p| p.as_str()),
                    over.subscription_model,
                    over.request_timeout_secs,
                    draft.api_key.is_some(),
                    draft.api_key.clone().flatten(),
                    draft.limits.is_some(),
                    limits.max_hops,
                    limits.max_steps_per_run,
                    limits.max_fanout_per_call.map(|n| n as i64),
                    limits.max_sends_per_pair,
                    limits.max_tool_rounds,
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
                "SELECT base_url, api_key, default_model, provider, subscription_model,
                        request_timeout_secs
                   FROM groups WHERE id=?1",
                params![id.to_string()],
                |row| {
                    let provider: Option<String> = row.get(3)?;
                    Ok(GroupInference {
                        overrides: InferenceOverrides {
                            base_url: row.get(0)?,
                            default_model: row.get(2)?,
                            provider: provider.as_deref().and_then(crate::config::Provider::parse),
                            subscription_model: row.get(4)?,
                            request_timeout_secs: row.get(5)?,
                        },
                        api_key: row.get(1)?,
                    })
                },
            )
            .optional()?;
        Ok(found.unwrap_or_default())
    }

    /// A group's loop limits, unresolved. A field it does not set is `None`,
    /// and the runtime layers what is left over the app's.
    ///
    /// Its own query rather than a field on the one above: this is read on
    /// every send and every model call, and the inference settings are read
    /// once a turn. Reading either does not want to carry the other's cost, and
    /// the key must not be read into memory on a path that does not need it.
    pub fn group_limits(&self, id: GroupId) -> Result<GroupLimits, StoreError> {
        let conn = self.conn()?;
        let found = conn
            .query_row(
                "SELECT max_hops, max_steps_per_run, max_fanout_per_call, max_sends_per_pair,
                        max_tool_rounds
                   FROM groups WHERE id=?1",
                params![id.to_string()],
                |row| {
                    Ok(GroupLimits {
                        max_hops: row.get(0)?,
                        max_steps_per_run: row.get(1)?,
                        max_fanout_per_call: row.get::<_, Option<i64>>(2)?.map(|n| n as usize),
                        max_sends_per_pair: row.get(3)?,
                        max_tool_rounds: row.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(found.unwrap_or_default())
    }

    pub fn get_group(&self, id: GroupId) -> Result<Option<Group>, StoreError> {
        Ok(self.list_groups()?.into_iter().find(|g| g.id == id))
    }

    /// The refusals that do not depend on the group being empty: it has to
    /// exist, and the default group has to stay, because every agent has to be
    /// in one.
    ///
    /// Separate from `delete_group` so disbanding a crew can refuse before it
    /// starts destroying machines. A disband that killed a group's computers
    /// and browsers and then found the group itself could not go would have
    /// spent the irreversible half of the work on a call that fails.
    pub fn group_for_removal(&self, id: GroupId) -> Result<Group, StoreError> {
        let group = self.get_group(id)?.ok_or(StoreError::GroupNotFound(id))?;
        if id == default_group_id() {
            return Err(StoreError::CannotDeleteDefaultGroup);
        }
        Ok(group)
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
        let group = self.group_for_removal(id)?;
        if group.agent_count > 0 {
            return Err(StoreError::GroupNotEmpty { name: group.name, agents: group.agent_count });
        }

        let default = default_group_id();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE agents SET group_id=?2 WHERE group_id=?1",
            params![id.to_string(), default.to_string()],
        )?;
        // The group's accounts go with it. They are scoped to it, so moving
        // them would hand another crew credentials nobody gave it, and leaving
        // them would fail the foreign key on the row below.
        conn.execute("DELETE FROM connectors WHERE group_id=?1", params![id.to_string()])?;
        // And the directories it was working in. Guaca's record of them only:
        // nothing on the operator's disk is touched by disbanding a crew. Who
        // was named on one goes first, or the foreign key refuses the line after
        // it, which is the same ordering the plugin tables below need.
        conn.execute(
            "DELETE FROM repository_access WHERE repository_id IN
                 (SELECT id FROM repositories WHERE group_id=?1)",
            params![id.to_string()],
        )?;
        conn.execute("DELETE FROM repositories WHERE group_id=?1", params![id.to_string()])?;
        // And its plugins, for both of those reasons and one more: the row
        // holds a grant against the operator's own Neon or Cloudflare account,
        // and a disbanded crew is not a reason to keep one. Whoever was named
        // on one goes first, along with whatever was switched off on it, or the
        // foreign key refuses the line below.
        conn.execute(
            "DELETE FROM plugin_agents WHERE plugin_id IN
                 (SELECT id FROM plugins WHERE group_id=?1)",
            params![id.to_string()],
        )?;
        conn.execute(
            "DELETE FROM plugin_tool_access WHERE plugin_id IN
                 (SELECT id FROM plugins WHERE group_id=?1)",
            params![id.to_string()],
        )?;
        conn.execute(
            "DELETE FROM plugin_tool_agents WHERE plugin_id IN
                 (SELECT id FROM plugins WHERE group_id=?1)",
            params![id.to_string()],
        )?;
        conn.execute("DELETE FROM plugins WHERE group_id=?1", params![id.to_string()])?;
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
            "INSERT INTO messages (id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,intent,cause,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
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
                envelope.intent.as_str(),
                envelope.cause.map(|c| c.to_string()),
                envelope.created_at,
            ],
        )?;
        Ok(())
    }

    /// The newest `limit` messages in a channel, returned oldest-first for
    /// direct rendering.
    /// One message, by id. Used to put a failed turn back on its feet: what to
    /// deliver again is exactly what was delivered before.
    pub fn get_message(&self, id: MessageId) -> Result<Option<Envelope>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,intent,cause,created_at
               FROM messages WHERE id=?1",
        )?;
        match stmt.query_row(params![id.to_string()], row_to_envelope).optional()? {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn channel_messages(
        &self,
        channel: AgentId,
        limit: u32,
    ) -> Result<Vec<Envelope>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,intent,cause,created_at
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

    /// What two agents said to each other, oldest first.
    ///
    /// Read from the messages rather than assembled from either channel, and
    /// that is the whole point of it existing. A send is filed under the
    /// recipient and the reply under the sender, so each channel holds one half
    /// of the exchange; worse, an automatic reply leaves no trace at all in the
    /// channel of the agent that wrote it, since only explicit tool calls are
    /// recorded there. A thread built from one side would be missing messages
    /// nobody could account for.
    ///
    /// Both directions, because a conversation is not directional. Agent
    /// activity records are excluded by the `to_kind` predicate: they are
    /// bookkeeping filed against `system`, not something said to a peer.
    pub fn pair_messages(
        &self,
        a: AgentId,
        b: AgentId,
        limit: u32,
    ) -> Result<Vec<Envelope>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,intent,cause,created_at
               FROM messages
              WHERE from_kind='agent' AND to_kind='agent'
                AND ((from_agent=?1 AND to_agent=?2) OR (from_agent=?2 AND to_agent=?1))
              ORDER BY created_at DESC, id DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![a.to_string(), b.to_string(), limit], row_to_envelope)?;
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
            "SELECT id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,intent,cause,created_at
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

    /// A channel's newest messages, guaranteed to reach back through one of
    /// them.
    ///
    /// What a search result needs in order to be openable: the transcript is
    /// normally read as "the newest few hundred", and a hit older than that
    /// would open its channel without itself in it. Still bounded, so a hit
    /// older than `cap` messages opens the channel at its newest end rather
    /// than pulling a year of history into the webview.
    pub fn channel_messages_through(
        &self,
        channel: AgentId,
        through: MessageId,
        cap: u32,
    ) -> Result<Vec<Envelope>, StoreError> {
        let conn = self.conn()?;
        // A row value, so the cut is made on exactly the ordering the channel
        // is drawn in. Comparing `created_at` on its own would drop whatever
        // was written in the same millisecond as the target, and a reply and
        // the record of the call that produced it are usually that close.
        let mut stmt = conn.prepare(
            r"SELECT id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,intent,cause,created_at
                FROM messages
               WHERE channel_id=?1
                 AND (created_at, id) >= (SELECT created_at, id FROM messages WHERE id=?2)
               ORDER BY created_at DESC, id DESC LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![channel.to_string(), through.to_string(), cap], row_to_envelope)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        out.reverse();
        Ok(out)
    }

    // ---- search ----------------------------------------------------------

    /// Everything stored here that matches a query.
    ///
    /// Four categories from three statements, because a file and a link are a
    /// message read from a different angle: both are found by the scan that
    /// finds the message and neither is stored anywhere of its own. Ordering
    /// across the categories is left to the caller, since which one a person
    /// wants first depends on what they were doing when they started typing.
    ///
    /// An empty query is not an error and not empty-handed: it matches
    /// everything, so what comes back is the newest of each kind.
    pub fn search(&self, query: &str, limit: u32) -> Result<SearchHits, StoreError> {
        let pattern = like_pattern(query);
        let want = limit as usize;

        let scanned = self.matching_messages(&pattern, SEARCH_SCAN)?;
        let mut messages = Vec::new();
        let mut links = Vec::new();
        let mut seen_urls = HashSet::new();

        for envelope in &scanned {
            let body = envelope.plain_text();

            if messages.len() < want {
                messages.push(MessageHit {
                    id: envelope.id,
                    channel_id: envelope.channel_id,
                    from: envelope.from,
                    to: envelope.to,
                    excerpt: excerpt(&body, query, EXCERPT_CHARS),
                    created_at: envelope.created_at,
                });
            }

            for url in links_in(&body) {
                if links.len() >= want {
                    break;
                }
                // The message matched, which does not mean this URL did: a
                // message can mention a subject and link somewhere unrelated.
                if !contains_fold(url, query) || !seen_urls.insert(url.to_string()) {
                    continue;
                }
                links.push(LinkHit {
                    url: url.to_string(),
                    message_id: envelope.id,
                    channel_id: envelope.channel_id,
                    created_at: envelope.created_at,
                });
            }
        }

        Ok(SearchHits {
            messages,
            links,
            files: self.matching_files(&pattern, want)?,
            routines: self.matching_routines(&pattern, limit)?,
        })
    }

    /// Messages with matching text, newest first.
    ///
    /// Matched part by part rather than against the stored blob. `parts` is
    /// JSON, and a substring search over it also matches the keys: "text",
    /// "name" and "type" are in every row, so searching for any of them
    /// returned the entire transcript.
    fn matching_messages(&self, pattern: &str, limit: u32) -> Result<Vec<Envelope>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r"SELECT id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,intent,cause,created_at
                FROM messages
               WHERE to_kind <> 'system'
                 AND EXISTS (
                     SELECT 1 FROM json_each(messages.parts) AS part
                      WHERE json_extract(part.value, '$.type') = 'text'
                        AND json_extract(part.value, '$.text') LIKE ?1 ESCAPE '\'
                 )
               ORDER BY created_at DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit], row_to_envelope)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    /// Attachments with matching names, newest first and one row per file.
    fn matching_files(&self, pattern: &str, limit: usize) -> Result<Vec<FileHit>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r"SELECT m.id, m.channel_id, m.from_kind, m.from_agent, m.created_at, part.value
                FROM messages m, json_each(m.parts) AS part
               WHERE m.to_kind <> 'system'
                 AND json_extract(part.value, '$.type') = 'file'
                 AND json_extract(part.value, '$.name') LIKE ?1 ESCAPE '\'
               ORDER BY m.created_at DESC, m.id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, SEARCH_SCAN], row_to_file_hit)?;

        // Deduplicated by digest, keeping the newest appearance. The bytes are
        // addressed by content, so the same document sent to three agents is
        // one file and belongs on one row.
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for row in rows {
            let hit = row??;
            if out.len() >= limit {
                break;
            }
            if seen.insert(hit.file.digest.clone()) {
                out.push(hit);
            }
        }
        Ok(out)
    }

    /// Routines whose name or instruction matches, in the order they will next
    /// fire.
    ///
    /// Both columns, because either is what somebody would type: a routine
    /// carries a title and the instruction it delivers, and only one of them is
    /// ever filled in when an agent sets its own schedule. Switched-off ones are
    /// included, since finding one is usually how it gets switched back on.
    fn matching_routines(&self, pattern: &str, limit: u32) -> Result<Vec<Routine>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r"SELECT id,agent_id,name,what,fires,active,next_run_at,last_run_at,created_at,skip_if_working
                FROM routines
               WHERE what LIKE ?1 ESCAPE '\' OR name LIKE ?1 ESCAPE '\'
               ORDER BY next_run_at IS NULL, next_run_at ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit], row_to_routine)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
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

    /// Every routine held by a group's agents.
    pub fn delete_group_routines(&self, group: GroupId) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute(
            "DELETE FROM routines
              WHERE agent_id IN (SELECT id FROM agents WHERE group_id=?1)",
            params![group.to_string()],
        )?)
    }

    /// Everything a group has spent.
    ///
    /// Booked against the group rather than the agent, so an agent that has
    /// since moved elsewhere does not take its old group's bill with it.
    pub fn delete_group_usage(&self, group: GroupId) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute("DELETE FROM usage WHERE group_id=?1", params![group.to_string()])?)
    }

    /// Every live agent in a group, whole cards.
    ///
    /// Terminated ones are left out. They have already been retired: their
    /// machines are destroyed and their memories gone, and they are only still
    /// filed under the group so old transcripts render. Handing one back to a
    /// disband would mean a second attempt to kill a sandbox that no longer
    /// exists, which is the failure the operator would then be shown.
    pub fn group_crew(&self, group: GroupId) -> Result<Vec<AgentCard>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id,sandbox_id,sandbox_envd_token,sandbox_traffic_token,pinned,rail_order,browser_id,has_computer,has_browser
               FROM agents WHERE group_id=?1 AND lifecycle <> 'terminated' ORDER BY rowid",
        )?;
        let rows = stmt.query_map(params![group.to_string()], row_to_card)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    /// The agents a group holds, deleted ones included.
    ///
    /// Deleted agents still have transcripts and notes on disk, so a reset that
    /// skipped them would leave the group half cleared.
    pub fn group_agent_ids(&self, group: GroupId) -> Result<Vec<AgentId>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT id FROM agents WHERE group_id=?1 ORDER BY rowid")?;
        let rows = stmt.query_map(params![group.to_string()], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            let raw = row?;
            out.push(
                raw.parse::<AgentId>()
                    .map_err(|e| StoreError::Corrupt(format!("bad agent id {raw:?}: {e}")))?,
            );
        }
        Ok(out)
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
            pinned: row.get::<_, i64>(15)? != 0,
            rail_order: row.get(16)?,
            browser_id: row.get(17)?,
            has_computer: row.get::<_, i64>(18)? != 0,
            has_browser: row.get::<_, i64>(19)? != 0,
            version: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
        })
    })())
}

/// One attachment, out of the message that carried it.
///
/// The part is deserialized rather than read field by field, so the shape a
/// message was written with is the shape it is read back with, and a part that
/// is not a file cannot be mistaken for one.
fn row_to_file_hit(row: &Row<'_>) -> RowResult<FileHit> {
    let id_raw: String = row.get(0)?;
    let channel_raw: String = row.get(1)?;
    let from_kind: String = row.get(2)?;
    let from_agent: Option<String> = row.get(3)?;
    let created_at: i64 = row.get(4)?;
    let part_raw: String = row.get(5)?;

    Ok((|| {
        let part: Part = serde_json::from_str(&part_raw)
            .map_err(|e| StoreError::Corrupt(format!("unreadable part: {e}")))?;
        let Part::File(file) = part else {
            return Err(StoreError::Corrupt("a part matched as a file but is not one".into()));
        };

        Ok(FileHit {
            file,
            message_id: id_raw
                .parse()
                .map_err(|e| StoreError::Corrupt(format!("bad message id: {e}")))?,
            channel_id: channel_raw
                .parse()
                .map_err(|e| StoreError::Corrupt(format!("bad channel id: {e}")))?,
            from: participant_from_columns(&from_kind, from_agent)?,
            created_at,
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
            name: row.get(2)?,
            what: row.get(3)?,
            trigger: {
                let raw: String = row.get(4)?;
                Trigger::parse(&raw)
                    .ok_or_else(|| StoreError::Corrupt(format!("unknown trigger {raw:?}")))?
            },
            active: row.get::<_, i64>(5)? != 0,
            next_run_at: row.get(6)?,
            last_run_at: row.get(7)?,
            created_at: row.get(8)?,
            skip_if_working: row.get::<_, i64>(9)? != 0,
        })
    })())
}

fn row_to_routine_run(row: &Row<'_>) -> RowResult<RoutineRun> {
    let run_raw: Option<String> = row.get(0)?;
    let kind_raw: String = row.get(1)?;

    Ok((|| {
        Ok(RoutineRun {
            // Absent on a firing that was skipped: nothing ran, so there is no
            // run to point at.
            run_id: run_raw
                .map(|raw| {
                    raw.parse::<RunId>()
                        .map_err(|e| StoreError::Corrupt(format!("bad run id {raw:?}: {e}")))
                })
                .transpose()?,
            kind: RunKind::parse(&kind_raw)
                .ok_or_else(|| StoreError::Corrupt(format!("unknown run kind {kind_raw:?}")))?,
            at: row.get(2)?,
            spent: Tokens {
                prompt: row.get::<_, i64>(3)? as u64,
                completion: row.get::<_, i64>(4)? as u64,
                // NULL where nothing was priced, which is not the same as free
                // and must not be summed as zero.
                cost: row.get::<_, Option<f64>>(5)?,
                calls: row.get::<_, i64>(6)? as u64,
            },
        })
    })())
}

fn row_to_approval(row: &Row<'_>) -> RowResult<Approval> {
    let id_raw: String = row.get(0)?;
    let agent_raw: String = row.get(1)?;
    let group_raw: String = row.get(2)?;
    let run_raw: String = row.get(3)?;
    let action_raw: String = row.get(4)?;
    let detail_raw: String = row.get(6)?;
    let state_raw: String = row.get(7)?;
    let options_raw: Option<String> = row.get(10)?;

    Ok((|| {
        Ok(Approval {
            id: id_raw
                .parse::<ApprovalId>()
                .map_err(|e| StoreError::Corrupt(format!("bad approval id {id_raw:?}: {e}")))?,
            agent_id: agent_raw
                .parse::<AgentId>()
                .map_err(|e| StoreError::Corrupt(format!("bad agent id {agent_raw:?}: {e}")))?,
            group_id: group_raw
                .parse::<GroupId>()
                .map_err(|e| StoreError::Corrupt(format!("bad group id {group_raw:?}: {e}")))?,
            run_id: run_raw
                .parse::<RunId>()
                .map_err(|e| StoreError::Corrupt(format!("bad run id {run_raw:?}: {e}")))?,
            // One column says which kind this is, because a question stores a
            // token no protected action can be. Both halves are decoded here
            // and nowhere else, so the shape the rest of the app sees is the
            // enum rather than the pair of columns behind it.
            request: if action_raw == QUESTION {
                Request::Question {
                    options: match &options_raw {
                        Some(raw) => serde_json::from_str(raw).map_err(|e| {
                            StoreError::Corrupt(format!("bad question options: {e}"))
                        })?,
                        // A question that takes a written answer.
                        None => Vec::new(),
                    },
                }
            } else {
                Request::Permission {
                    action: ProtectedAction::parse(&action_raw).ok_or_else(|| {
                        StoreError::Corrupt(format!("unknown protected action {action_raw:?}"))
                    })?,
                }
            },
            summary: row.get(5)?,
            // The wording is what the operator reads, and a request whose
            // fields would not parse is one nobody should be asked to answer.
            detail: serde_json::from_str(&detail_raw)
                .map_err(|e| StoreError::Corrupt(format!("bad approval detail: {e}")))?,
            answer: row.get(11)?,
            state: ApprovalState::parse(&state_raw).ok_or_else(|| {
                StoreError::Corrupt(format!("unknown approval state {state_raw:?}"))
            })?,
            created_at: row.get(8)?,
            decided_at: row.get(9)?,
        })
    })())
}

/// The column list every connector read uses, kept in one place so a new column
/// cannot be added to one query and forgotten in the other.
const CONNECTOR_COLUMNS: &str = "SELECT id,group_id,service,account,env_var,secret,note,\
                                 created_at,updated_at FROM connectors";

/// Maps the unique index on `(group_id, env_var)` onto something an operator
/// can act on. The raw driver message names an index, not a decision.
fn classify_connector(err: rusqlite::Error, env_var: &str) -> StoreError {
    if let rusqlite::Error::SqliteFailure(inner, _) = &err {
        if inner.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE {
            return StoreError::DuplicateEnvVar(env_var.to_string());
        }
    }
    StoreError::Sqlite(err)
}

fn row_to_connector(row: &Row<'_>) -> RowResult<Connector> {
    let id_raw: String = row.get(0)?;
    let group_raw: String = row.get(1)?;
    let secret: String = row.get(5)?;

    Ok((|| {
        Ok(Connector {
            id: id_raw
                .parse::<ConnectorId>()
                .map_err(|e| StoreError::Corrupt(format!("bad connector id {id_raw:?}: {e}")))?,
            group_id: group_raw
                .parse::<GroupId>()
                .map_err(|e| StoreError::Corrupt(format!("bad group id {group_raw:?}: {e}")))?,
            service: row.get(2)?,
            account: row.get(3)?,
            env_var: row.get(4)?,
            note: row.get(6)?,
            secret_set: !secret.trim().is_empty(),
            secret_hint: crate::config::hint_for(&secret),
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
        })
    })())
}

/// What one agent gets when it names one of its crew's plugins.
///
/// The refusals are separate because they are read by a model mid-turn and they
/// have different answers: one is the operator's to fix, two are a peer's to
/// do, and one is nobody's.
#[derive(Debug)]
pub enum PluginReach {
    /// No row: this crew has not connected it, and no agent can.
    NotConnected,
    /// Connected, and this agent is not one of the ones it was connected for.
    NotChosen,
    /// Connected, and the operator narrowed this tool to nobody. Off for the
    /// whole crew, so there is no peer to ask.
    ToolDenied,
    /// Connected and this agent's, but this tool was narrowed to other agents.
    /// Somebody in the crew has it.
    ToolNotChosen,
    /// The row, and the grant to spend against it. `None` for a server that
    /// authorized nobody because it asked for nothing.
    Granted { id: PluginId, grant: Option<crate::oauth::Grant> },
}

const REPOSITORY_COLUMNS: &str =
    "SELECT id,group_id,name,path,note,created_at,updated_at FROM repositories";

/// The one rule that decides whether an agent may work in a repository.
///
/// Written once and read in one place today, in the same shape as
/// [`PLUGIN_REACHED_BY_AGENT`] so the two read alike where they sit beside each
/// other. It has no `everyone` branch and must never grow one: a plugin
/// defaults open because a crew's Linear account is usually the crew's, and a
/// working tree does not, because an agent hired next week must not inherit the
/// operator's own source. `domain/repository.rs` is the long version.
const REPOSITORY_REACHED_BY_AGENT: &str = "EXISTS (SELECT 1 FROM repository_access
     WHERE repository_access.repository_id = repositories.id
       AND repository_access.agent_id = :agent)";

/// A unique-index failure here is one directory linked twice, and the operator
/// wants to be told which rather than told the database said no.
fn classify_repository(err: rusqlite::Error, path: &str) -> StoreError {
    if let rusqlite::Error::SqliteFailure(inner, _) = &err {
        if inner.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE {
            return StoreError::DuplicateRepository(path.to_string());
        }
    }
    StoreError::Sqlite(err)
}

/// Reach is left empty here and filled in by the caller.
///
/// It is a second query against a second table, so a row mapper cannot produce
/// it, and a `Repository` that quietly claimed nobody could reach it would be
/// indistinguishable from one nobody has been given.
fn row_to_repository(row: &Row<'_>) -> RowResult<Repository> {
    let id_raw: String = row.get(0)?;
    let group_raw: String = row.get(1)?;

    Ok((|| {
        Ok(Repository {
            id: id_raw
                .parse::<RepositoryId>()
                .map_err(|e| StoreError::Corrupt(format!("bad repository id {id_raw:?}: {e}")))?,
            group_id: group_raw
                .parse::<GroupId>()
                .map_err(|e| StoreError::Corrupt(format!("bad group id {group_raw:?}: {e}")))?,
            name: row.get(2)?,
            path: row.get(3)?,
            note: row.get(4)?,
            reach: Vec::new(),
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    })())
}

const PLUGIN_COLUMNS: &str =
    "SELECT id,group_id,kind,account,tools,access_token,connected_at,access,connection FROM plugins";

/// The one rule that decides whether an agent is offered a plugin's tools and
/// whether a call it makes may spend the grant.
///
/// Written once and pasted into both queries, because a turn where the two
/// disagree is either a model calling something it was never told it had, or a
/// model refused something it was told it had and told to try again. Named
/// parameters rather than numbered so the fragment does not care where it lands
/// in the statement around it.
///
/// The comparison is what makes it fail closed: only the literal `everyone`
/// widens a plugin past its named agents, so a value written by a build this
/// one has never met restricts rather than opens.
const PLUGIN_REACHED_BY_AGENT: &str = "(plugins.access = 'everyone'
     OR EXISTS (SELECT 1 FROM plugin_agents
                 WHERE plugin_agents.plugin_id = plugins.id
                   AND plugin_agents.agent_id = :agent))";

/// Whether this agent may call one named tool, given that it may call the
/// plugin at all.
///
/// The narrowings are what is stored, so a tool with no row is one every agent
/// the plugin reaches may call: a plugin nobody has narrowed offers everything
/// it published, and so does a tool the vendor added after the operator last
/// looked. An allow-list over tools would switch off every tool a server
/// started publishing between one connection and the next, with nothing on
/// screen saying a decision had been taken.
///
/// Inside a narrowed tool it is the other way round, and deliberately: the
/// named agents are stored and everybody else is refused, so an agent hired
/// next week does not inherit the one capability the operator fenced off. The
/// same comparison against the literal `everyone` makes it fail closed, for the
/// reason [`PLUGIN_REACHED_BY_AGENT`] does.
const PLUGIN_TOOL_REACHED_BY_AGENT: &str = "NOT EXISTS (SELECT 1 FROM plugin_tool_access
                 WHERE plugin_tool_access.plugin_id = plugins.id
                   AND plugin_tool_access.tool = :tool
                   AND plugin_tool_access.access <> 'everyone'
                   AND NOT EXISTS (SELECT 1 FROM plugin_tool_agents
                                    WHERE plugin_tool_agents.plugin_id = plugins.id
                                      AND plugin_tool_agents.tool = :tool
                                      AND plugin_tool_agents.agent_id = :agent))";

/// Whether anybody at all may call one named tool.
///
/// The same question with the agent taken out, and it is what separates the two
/// tool refusals. A tool narrowed to nobody is off for the crew, and an agent
/// told to ask a peer about one spends a turn proving that no peer has it.
const PLUGIN_TOOL_REACHED_BY_ANYONE: &str = "NOT EXISTS (SELECT 1 FROM plugin_tool_access
                 WHERE plugin_tool_access.plugin_id = plugins.id
                   AND plugin_tool_access.tool = :tool
                   AND plugin_tool_access.access <> 'everyone'
                   AND NOT EXISTS (SELECT 1 FROM plugin_tool_agents
                                    WHERE plugin_tool_agents.plugin_id = plugins.id
                                      AND plugin_tool_agents.tool = :tool))";

/// `None` for a row naming a plugin this build does not have, which is what a
/// downgrade leaves behind. Corrupt rows still raise: a tool list that will not
/// parse is a bug here, not a version skew.
///
/// `chosen` is the row's named agents and `narrowed` is who may call each of
/// its tools, both read separately because both are second tables. A plugin
/// missing from the first map is one with nobody named; a tool missing from the
/// second is one every agent the plugin reaches may call, which is where every
/// tool starts.
fn row_to_plugin(
    row: &Row<'_>,
    chosen: &HashMap<PluginId, Vec<AgentId>>,
    narrowed: &HashMap<(PluginId, String), PluginAccess>,
) -> RowResult<Option<Plugin>> {
    let id_raw: String = row.get(0)?;
    let group_raw: String = row.get(1)?;
    let kind_raw: String = row.get(2)?;
    let tools_raw: String = row.get(4)?;
    let access: String = row.get(5)?;
    let reach: String = row.get(7)?;
    let connection: String = row.get(8)?;

    Ok((|| {
        let Some(kind) = PluginKind::from_slug(&kind_raw) else { return Ok(None) };
        let tools: Vec<PluginTool> = serde_json::from_str(&tools_raw)
            .map_err(|e| StoreError::Corrupt(format!("bad tool list for {kind_raw:?}: {e}")))?;
        let id = id_raw
            .parse::<PluginId>()
            .map_err(|e| StoreError::Corrupt(format!("bad plugin id {id_raw:?}: {e}")))?;

        Ok(Some(Plugin {
            id,
            group_id: group_raw
                .parse::<GroupId>()
                .map_err(|e| StoreError::Corrupt(format!("bad group id {group_raw:?}: {e}")))?,
            kind,
            account: row.get(3)?,
            tools: tools
                .into_iter()
                .map(|tool| PluginToolCard {
                    access: narrowed
                        .get(&(id, tool.name.clone()))
                        .cloned()
                        .unwrap_or(PluginAccess::Everyone),
                    name: tool.name,
                    description: tool.description,
                })
                .collect(),
            access: PluginAccess::from_row(&reach, chosen.get(&id).cloned().unwrap_or_default()),
            connection,
            signed_in: !access.is_empty(),
            connected_at: row.get(6)?,
        }))
    })())
}

fn row_to_signin(row: &Row<'_>) -> RowResult<Signin> {
    let agent_raw: String = row.get(0)?;

    Ok((|| {
        Ok(Signin {
            agent_id: agent_raw
                .parse::<AgentId>()
                .map_err(|e| StoreError::Corrupt(format!("bad agent id {agent_raw:?}: {e}")))?,
            surface: Surface::parse(&row.get::<_, String>(1)?),
            domain: row.get(2)?,
            service: row.get(3)?,
            recognized: row.get::<_, i64>(4)? != 0,
            first_seen_at: row.get(5)?,
            last_seen_at: row.get(6)?,
        })
    })())
}

fn row_to_group(row: &Row<'_>) -> RowResult<Group> {
    let id_raw: String = row.get(0)?;
    let api_key: Option<String> = row.get(5)?;
    let provider: Option<String> = row.get(7)?;

    Ok((|| {
        Ok(Group {
            id: id_raw
                .parse::<GroupId>()
                .map_err(|e| StoreError::Corrupt(format!("bad group id {id_raw:?}: {e}")))?,
            name: row.get(1)?,
            created_at: row.get(2)?,
            agent_count: row.get::<_, i64>(3)?.max(0) as u32,
            inference: InferenceOverrides {
                base_url: row.get(4)?,
                default_model: row.get(6)?,
                provider: provider.as_deref().and_then(crate::config::Provider::parse),
                subscription_model: row.get(8)?,
                request_timeout_secs: row.get(9)?,
            },
            api_key_set: api_key.as_deref().is_some_and(|k| !k.trim().is_empty()),
            api_key_hint: crate::config::hint_for(api_key.as_deref().unwrap_or_default()),
            limits: GroupLimits {
                max_hops: row.get(10)?,
                max_steps_per_run: row.get(11)?,
                max_fanout_per_call: row.get::<_, Option<i64>>(12)?.map(|n| n as usize),
                max_sends_per_pair: row.get(13)?,
                max_tool_rounds: row.get(14)?,
            },
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
    let intent_raw: String = row.get(11)?;
    let cause_raw: Option<String> = row.get(12)?;
    let created_at: i64 = row.get(13)?;

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
            // Anything unrecognized reads as a courtesy, which is the same
            // conservative default the parser uses: a stored word nobody
            // defined must not be the one that wakes an agent to act.
            intent: Intent::parse(&intent_raw),
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
    use crate::domain::attachment::Attachment;
    use crate::domain::envelope::channel_for;
    use crate::domain::ids::RunId;
    use crate::domain::routine::{Cadence, EventTrigger};

    /// A trigger on the clock, which is what most of these are about.
    fn clock(cadence: Cadence) -> Trigger {
        Trigger::Clock(cadence)
    }

    struct Fixture {
        store: Store,
        _dir: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();
        Fixture { store, _dir: dir }
    }

    /// One tool on a connected plugin. What it does is never read here; what
    /// matters is whether it is offered at all.
    fn tool(name: &str) -> PluginTool {
        PluginTool {
            name: name.into(),
            description: String::new(),
            input_schema: serde_json::json!({ "type": "object" }),
        }
    }

    /// A tool narrowed to nobody: what the old two-way switch called off.
    fn nobody() -> PluginAccess {
        PluginAccess::Chosen { agents: Vec::new() }
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
            intent: Intent::Courtesy,
            cause: None,
            created_at: now_ms(),
        }
    }

    fn group_named(name: &str) -> CleanGroup {
        CleanGroup { name: name.into(), ..Default::default() }
    }

    fn repo_at(group: GroupId, path: &str) -> CleanRepository {
        CleanRepository {
            group_id: group,
            name: path.rsplit('/').next().unwrap_or(path).into(),
            path: path.into(),
            note: String::new(),
        }
    }

    fn key_for(group: GroupId, env_var: &str, secret: &str) -> CleanConnector {
        CleanConnector {
            group_id: group,
            service: "GitHub".into(),
            account: "madebywelch".into(),
            env_var: env_var.into(),
            note: String::new(),
            secret: secret.into(),
        }
    }

    fn signin_at(agent: AgentId, service: &str, domain: &str, at: i64) -> Signin {
        Signin {
            surface: Surface::Computer,
            agent_id: agent,
            domain: domain.into(),
            service: service.into(),
            recognized: true,
            first_seen_at: at,
            last_seen_at: at,
        }
    }

    #[test]
    fn an_agent_is_made_with_neither_place_and_keeps_the_one_it_is_given() {
        // A new agent gets nothing, including one another agent made: handing
        // out a machine is the operator's, and a crew that could grant itself
        // one by hiring would route around the decision entirely.
        let f = fixture();
        let card = f.store.create_agent(&draft("Talker")).unwrap();
        assert!(!card.has_computer);
        assert!(!card.has_browser);

        f.store.set_has_computer(card.id, true).unwrap();
        let given = f.store.get_agent(card.id).unwrap().unwrap();
        assert!(given.has_computer);
        assert!(!given.has_browser, "the two are given separately or they are one switch");
        assert_eq!(given.version, card.version, "a grant is not an edit to the card");

        // Taken back, and the machine underneath deliberately left where it is:
        // the disk holds whatever the operator signed it in to, and giving the
        // computer back has to find those sessions rather than a stranger.
        f.store.set_agent_sandbox(card.id, Some(("sb-1", "envd", "traffic"))).unwrap();
        f.store.set_has_computer(card.id, false).unwrap();
        let taken = f.store.get_agent(card.id).unwrap().unwrap();
        assert!(!taken.has_computer);
        assert_eq!(taken.sandbox_id.as_deref(), Some("sb-1"));
    }

    #[test]
    fn a_grant_cannot_be_written_against_an_agent_that_is_not_there() {
        let f = fixture();
        let gone = AgentId::new();
        assert!(matches!(
            f.store.set_has_computer(gone, true),
            Err(StoreError::AgentNotFound(id)) if id == gone
        ));
        assert!(matches!(
            f.store.set_has_browser(gone, true),
            Err(StoreError::AgentNotFound(id)) if id == gone
        ));
    }

    #[test]
    fn a_connector_round_trips_without_its_secret_coming_back() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Researcher")).unwrap();
        let stored = f
            .store
            .create_connector(&key_for(agent.group_id, "GITHUB_TOKEN", "ghp_hunter2"))
            .unwrap();

        assert!(stored.secret_set, "the operator has to be able to see that one is set");
        assert_eq!(stored.secret_hint, "...ter2");
        let json = serde_json::to_string(&stored).unwrap();
        assert!(!json.contains("hunter2"), "a secret reached the wire: {json}");

        // And the value is still there, on the one path that is allowed to see it.
        let env = f.store.connector_env(agent.group_id).unwrap();
        assert_eq!(env.get("GITHUB_TOKEN").map(String::as_str), Some("ghp_hunter2"));
    }

    #[test]
    fn a_second_credential_cannot_take_a_variable_name_already_in_use() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Researcher")).unwrap();
        f.store.create_connector(&key_for(agent.group_id, "TOKEN", "a")).unwrap();

        let clash = f.store.create_connector(&key_for(agent.group_id, "TOKEN", "b"));
        assert!(
            matches!(clash, Err(StoreError::DuplicateEnvVar(ref name)) if name == "TOKEN"),
            "expected a named refusal, got {clash:?}"
        );
    }

    #[test]
    fn a_crew_cannot_reach_another_crews_credentials() {
        let f = fixture();
        let mine = f.store.create_agent(&draft("Researcher")).unwrap();
        let other_group = f.store.create_group(&group_named("Research")).unwrap();

        f.store.create_connector(&key_for(mine.group_id, "MINE", "a")).unwrap();
        f.store.create_connector(&key_for(other_group.id, "THEIRS", "b")).unwrap();

        assert_eq!(f.store.group_connectors(mine.group_id).unwrap().len(), 1);
        assert!(!f.store.connector_env(mine.group_id).unwrap().contains_key("THEIRS"));
    }

    #[test]
    fn a_deleted_group_takes_its_credentials_with_it() {
        // Not tidiness: connectors carry a foreign key to the group, so leaving
        // them makes the delete fail outright.
        let f = fixture();
        let group = f.store.create_group(&group_named("Research")).unwrap();
        f.store.create_connector(&key_for(group.id, "TOKEN", "a")).unwrap();

        f.store.delete_group(group.id).expect("a group with credentials must still delete");
        assert!(f.store.group_connectors(group.id).unwrap().is_empty());
    }

    #[test]
    fn a_scan_replaces_what_a_machine_was_signed_in_to() {
        // A cache of something that lives on the machine, so a logout has to
        // remove the row. Left behind, the crew keeps routing work to an agent
        // that will hit a login wall.
        let f = fixture();
        let agent = f.store.create_agent(&draft("Researcher")).unwrap();

        f.store
            .replace_signins(
                agent.id,
                Surface::Computer,
                &[
                    signin_at(agent.id, "LinkedIn", "linkedin.com", 100),
                    signin_at(agent.id, "GitHub", "github.com", 100),
                ],
            )
            .unwrap();
        assert_eq!(f.store.agent_signins(agent.id).unwrap().len(), 2);

        // Logged out of GitHub; the next scan only sees LinkedIn.
        let after = f
            .store
            .replace_signins(
                agent.id,
                Surface::Computer,
                &[signin_at(agent.id, "LinkedIn", "linkedin.com", 200)],
            )
            .unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].service, "LinkedIn");
    }

    #[test]
    fn rescanning_keeps_how_long_a_session_has_been_there() {
        // "Signed in since Tuesday" has to survive a scan, or every refresh
        // makes every session look brand new.
        let f = fixture();
        let agent = f.store.create_agent(&draft("Researcher")).unwrap();

        f.store
            .replace_signins(
                agent.id,
                Surface::Computer,
                &[signin_at(agent.id, "LinkedIn", "linkedin.com", 100)],
            )
            .unwrap();
        let again = f
            .store
            .replace_signins(
                agent.id,
                Surface::Computer,
                &[signin_at(agent.id, "LinkedIn", "linkedin.com", 900)],
            )
            .unwrap();

        assert_eq!(again[0].first_seen_at, 100, "the first sighting must not move");
        assert_eq!(again[0].last_seen_at, 900, "and the latest one must");
    }

    #[test]
    fn a_group_sees_every_live_machines_sessions_and_no_dead_ones() {
        let f = fixture();
        let mine = f.store.create_agent(&draft("Researcher")).unwrap();
        let peer = f.store.create_agent(&draft("Scribe")).unwrap();
        let gone = f.store.create_agent(&draft("Ghost")).unwrap();

        for (agent, service) in [(mine.id, "LinkedIn"), (peer.id, "GitHub"), (gone.id, "Gmail")] {
            f.store
                .replace_signins(
                    agent,
                    Surface::Computer,
                    &[signin_at(agent, service, "x.example", 1)],
                )
                .unwrap();
        }
        f.store.set_lifecycle(gone.id, Lifecycle::Terminated).unwrap();

        let visible: Vec<String> =
            f.store.group_signins(mine.group_id).unwrap().into_iter().map(|s| s.service).collect();
        assert!(visible.contains(&"LinkedIn".to_string()));
        assert!(visible.contains(&"GitHub".to_string()));
        assert!(
            !visible.contains(&"Gmail".to_string()),
            "a deleted agent's machine is destroyed, so its sessions are not on the roster"
        );
    }

    #[test]
    fn deleting_an_agent_forgets_what_its_browser_held() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Researcher")).unwrap();
        f.store
            .replace_signins(
                agent.id,
                Surface::Computer,
                &[signin_at(agent.id, "LinkedIn", "linkedin.com", 1)],
            )
            .unwrap();

        assert_eq!(f.store.delete_agent_signins(agent.id).unwrap(), 1);
        assert!(f.store.agent_signins(agent.id).unwrap().is_empty());
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
            .create_group(&CleanGroup { name: "Research".into(), ..Default::default() })
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
    fn usage_is_written_and_summed_against_the_real_schema() {
        // The bug this covers: every query here was written against a table
        // that a later edit was supposed to have given a `cost` column, and
        // nothing in the suite ever ran one of them. The app compiled, shipped
        // and failed on the first group that had spent anything.
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        let run = RunId::new();

        f.store
            .record_usage(&UsageEntry {
                agent_id: card.id,
                group_id: card.group_id,
                run_id: run,
                model: "test/model".into(),
                prompt: 1000,
                completion: 200,
                cost: Some(0.25),
            })
            .unwrap();
        // A local server prices nothing, and that is not the same as free.
        f.store
            .record_usage(&UsageEntry {
                agent_id: card.id,
                group_id: card.group_id,
                run_id: run,
                model: "local/model".into(),
                prompt: 10,
                completion: 5,
                cost: None,
            })
            .unwrap();

        let by_group = f.store.usage_by_group().unwrap();
        let group = by_group.get(&card.group_id).expect("the group spent something");
        assert_eq!(group.prompt, 1010);
        assert_eq!(group.completion, 205);
        assert_eq!(group.calls, 2, "calls, not turns");
        assert_eq!(group.cost, Some(0.25));

        let by_run = f.store.usage_by_run(&[run]).unwrap();
        assert_eq!(by_run.get(&run).unwrap().total(), 1215);

        // Asking about nothing is not an error, and must not build a query
        // with an empty IN list.
        assert!(f.store.usage_by_run(&[]).unwrap().is_empty());
    }

    #[test]
    fn editing_a_routine_leaves_its_next_firing_alone_unless_asked() {
        // Correcting a typo in what a routine says must not silently reset the
        // schedule it is keeping.
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        let made = f
            .store
            .create_routine(
                card.id,
                "Listings sweep",
                "check the listings",
                clock(Cadence::Every(3600)),
                Some(1_000_000),
                false,
            )
            .unwrap();

        let fixed = f
            .store
            .update_routine(
                made.id,
                "Listings sweep",
                "check the listings and say what is new",
                clock(Cadence::Every(3600)),
                made.next_run_at,
                false,
            )
            .unwrap();
        assert_eq!(fixed.what, "check the listings and say what is new");
        assert_eq!(fixed.next_run_at, made.next_run_at, "the schedule did not move");
        assert_eq!(fixed.trigger, clock(Cadence::Every(3600)));
        assert_eq!(fixed.name, "Listings sweep");

        // And a routine that is gone is a clear error rather than a silent
        // success, so an operator editing a stale screen is told.
        f.store.delete_routine(made.id).unwrap();
        assert!(matches!(
            f.store.update_routine(made.id, "", "anything", clock(Cadence::Once), Some(1), false),
            Err(StoreError::RoutineNotFound(_))
        ));
    }

    /// The arrangement, read back the way the rail reads it.
    fn arrangement(f: &Fixture) -> Vec<(String, i32)> {
        let mut agents: Vec<_> = f
            .store
            .list_agents()
            .unwrap()
            .into_iter()
            .filter(|a| a.lifecycle != Lifecycle::Terminated)
            .map(|a| (a.name, a.rail_order, a.created_at))
            .collect();
        agents.sort_by_key(|(name, order, created)| (*order, *created, name.clone()));
        agents.into_iter().map(|(name, order, _)| (name, order)).collect()
    }

    #[test]
    fn agents_arrive_at_the_bottom_of_the_rail_in_the_order_they_were_made() {
        // The rail used to float whoever spoke last, so where a new agent
        // landed did not matter. It does now: an agent that arrived in the
        // middle of an arrangement would look like the arrangement moved.
        let f = fixture();
        for name in ["First", "Second", "Third"] {
            f.store.create_agent(&draft(name)).unwrap();
        }
        assert_eq!(
            arrangement(&f),
            vec![("First".to_string(), 0), ("Second".to_string(), 1), ("Third".to_string(), 2)]
        );
    }

    #[test]
    fn a_move_puts_a_row_in_front_of_the_one_it_was_dropped_on() {
        let f = fixture();
        let a = f.store.create_agent(&draft("First")).unwrap();
        f.store.create_agent(&draft("Second")).unwrap();
        let c = f.store.create_agent(&draft("Third")).unwrap();

        let moved = f.store.move_agent(c.id, c.group_id, Some(a.id)).unwrap();
        assert_eq!(
            arrangement(&f).into_iter().map(|(name, _)| name).collect::<Vec<_>>(),
            vec!["Third", "First", "Second"]
        );

        // Where a row is drawn, and nothing a peer reads. Same reasoning as a
        // pin: bumping the version would tell every peer the card was rewritten.
        assert_eq!(moved.version, c.version, "a move is not an edit");
        assert_eq!(moved.updated_at, c.updated_at);
        assert_eq!(moved.rail_order, 0);

        // Densely renumbered, so the next drop has a whole position to land in
        // rather than a gap that can run out.
        assert_eq!(
            arrangement(&f).into_iter().map(|(_, order)| order).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn a_move_with_nothing_to_land_in_front_of_goes_to_the_end_of_that_group() {
        // Not the end of the rail. One sequence covers every group, so
        // appending past the last row would file the agent below crews it is
        // not in and the rail would draw it under their heading.
        let f = fixture();
        let research = f.store.create_group(&group_named("Research")).unwrap();

        let scout = f.store.create_agent(&draft("Scout")).unwrap();
        let mut second = draft("Reader");
        second.group_id = Some(research.id);
        f.store.create_agent(&second).unwrap();
        let cook = f.store.create_agent(&draft("Cook")).unwrap();

        // Everyone in the default group, then Research, then Cook back in the
        // default one: the arrangement interleaves groups until someone moves.
        let moved = f.store.move_agent(cook.id, research.id, None).unwrap();
        assert_eq!(moved.group_id, research.id, "one call moved it and placed it");
        assert_eq!(
            arrangement(&f).into_iter().map(|(name, _)| name).collect::<Vec<_>>(),
            vec!["Scout", "Reader", "Cook"]
        );

        // And landing in front of a row in another group ignores the anchor
        // rather than dragging the agent somewhere it was not dropped.
        f.store.move_agent(cook.id, research.id, Some(scout.id)).unwrap();
        assert_eq!(
            f.store.get_agent(cook.id).unwrap().unwrap().group_id,
            research.id,
            "the group half of the intent still holds"
        );
        assert_eq!(
            arrangement(&f).into_iter().map(|(name, _)| name).collect::<Vec<_>>(),
            vec!["Scout", "Reader", "Cook"]
        );
    }

    #[test]
    fn a_row_dropped_on_itself_stays_where_it_is() {
        // The fallback for an anchor that is not in the group is the end of it,
        // so a null gesture that reached this far would move the row to the
        // bottom of the rail: the one outcome the operator did not ask for.
        let f = fixture();
        f.store.create_agent(&draft("First")).unwrap();
        let middle = f.store.create_agent(&draft("Middle")).unwrap();
        f.store.create_agent(&draft("Last")).unwrap();

        f.store.move_agent(middle.id, middle.group_id, Some(middle.id)).unwrap();
        assert_eq!(
            arrangement(&f).into_iter().map(|(name, _)| name).collect::<Vec<_>>(),
            vec!["First", "Middle", "Last"]
        );
    }

    #[test]
    fn a_move_leaves_terminated_agents_out_of_the_numbering() {
        // A terminated agent is not in the rail, so a position spent on it is a
        // position the operator cannot drop into, and renumbering one would move
        // a row in an old transcript's sidebar order for no reason.
        let f = fixture();
        let gone = f.store.create_agent(&draft("Gone")).unwrap();
        let here = f.store.create_agent(&draft("Here")).unwrap();
        f.store.set_lifecycle(gone.id, Lifecycle::Terminated).unwrap();

        f.store.move_agent(here.id, here.group_id, None).unwrap();
        assert_eq!(f.store.get_agent(here.id).unwrap().unwrap().rail_order, 0);
        assert_eq!(
            f.store.get_agent(gone.id).unwrap().unwrap().rail_order,
            gone.rail_order,
            "a terminated agent's place was not rewritten"
        );
    }

    #[test]
    fn moving_an_agent_that_is_not_there_is_an_error_rather_than_a_silent_renumber() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        let group = card.group_id;
        f.store.set_lifecycle(card.id, Lifecycle::Terminated).unwrap();
        assert!(matches!(
            f.store.move_agent(card.id, group, None),
            Err(StoreError::AgentNotFound(_))
        ));
    }

    #[test]
    fn pinning_moves_a_row_on_screen_and_nothing_a_peer_can_see() {
        // The version is how a peer notices a card changed under it. Where the
        // operator likes the row drawn is not a change to the card, and
        // bumping it would say a card was rewritten when it was not.
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        assert!(!card.pinned, "nothing arrives pinned");

        let pinned = f.store.set_agent_pinned(card.id, true).unwrap();
        assert!(pinned.pinned);
        assert_eq!(pinned.version, card.version, "a pin is not an edit");
        assert_eq!(pinned.updated_at, card.updated_at);

        assert!(f.store.get_agent(card.id).unwrap().unwrap().pinned, "and it is durable");
        assert!(!f.store.set_agent_pinned(card.id, false).unwrap().pinned);
    }

    #[test]
    fn a_calendar_trigger_survives_the_round_trip_through_the_database() {
        // The column is text, so a trigger that stored as something nothing can
        // parse would be a schedule that silently never fires again.
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        for cadence in
            [Cadence::Once, Cadence::Weekdays, Cadence::Weekly, Cadence::Monthly, Cadence::Daily]
        {
            let made = f
                .store
                .create_routine(card.id, "", "check", clock(cadence), Some(1_000_000), false)
                .unwrap();
            let read = f.store.get_routine(made.id).unwrap().unwrap();
            assert_eq!(read.trigger, clock(cadence));
            assert_eq!(read.name, "", "a routine an agent set has no name to invent");
        }
    }

    #[test]
    fn a_weekday_routine_that_fires_advances_to_the_next_weekday_not_the_next_day() {
        // `routine_ran` is the only thing that moves a schedule forward, and it
        // used to add a fixed gap. A Friday routine has to land on Monday.
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        let friday = friday_at_nine();
        let made = f
            .store
            .create_routine(card.id, "", "check", clock(Cadence::Weekdays), Some(friday), false)
            .unwrap();

        f.store.routine_ran(&made, friday + 1000).unwrap();

        let moved = f.store.get_routine(made.id).unwrap().unwrap();
        let expected = Cadence::Weekdays.next_after(friday, friday + 1000).unwrap();
        assert_eq!(moved.next_run_at, Some(expected));
        assert_eq!(moved.last_run_at, Some(friday + 1000));
        assert!(expected - friday > 2 * 86_400_000, "it skipped the weekend");
    }

    #[test]
    fn a_routine_that_is_switched_off_is_not_due_and_keeps_everything_else() {
        // Deleting was the only way to stop something, which threw away the
        // wording and the history along with the schedule.
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        let made = f
            .store
            .create_routine(
                card.id,
                "Sweep",
                "check the listings",
                clock(Cadence::Daily),
                Some(1_000),
                false,
            )
            .unwrap();
        assert!(made.active, "a routine arrives running");
        assert_eq!(f.store.due_routines(2_000).unwrap().len(), 1);

        let off = f.store.set_routine_active(made.id, false).unwrap();
        assert!(!off.active);
        assert!(f.store.due_routines(2_000).unwrap().is_empty(), "an inactive routine never fires");
        assert_eq!(off.next_run_at, made.next_run_at, "and its slot did not move");
        assert_eq!(off.what, made.what);

        // Back on, still holding the slot it was holding. The scheduler fires
        // an overdue slot once, which is what "resume" has to mean.
        let on = f.store.set_routine_active(made.id, true).unwrap();
        assert_eq!(on.next_run_at, made.next_run_at);
        assert_eq!(f.store.due_routines(2_000).unwrap().len(), 1);
    }

    #[test]
    fn a_history_says_which_firings_were_tests() {
        // "Did it run on Tuesday" has no answer if a button press and a real
        // firing look the same in the list.
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        let made = f
            .store
            .create_routine(card.id, "", "check", clock(Cadence::Daily), Some(1_000), false)
            .unwrap();

        let scheduled = RunId::new();
        let tested = RunId::new();
        f.store.record_routine_run(made.id, Some(scheduled), RunKind::Scheduled, 1_000).unwrap();
        f.store.record_routine_run(made.id, Some(tested), RunKind::Test, 2_000).unwrap();

        let history = f.store.routine_runs(made.id, 20).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].run_id, Some(tested), "newest first");
        assert_eq!(history[0].kind, RunKind::Test);
        assert_eq!(history[1].kind, RunKind::Scheduled);
    }

    #[test]
    fn a_skipped_firing_is_in_the_history_with_no_run_behind_it() {
        // The alternative to recording it is a gap, and a gap in this list is
        // also what a scheduler that has stopped working looks like. What it
        // must not do is invent a run: that reads back identically to a
        // delivery that spent nothing, which is the one thing this history is
        // for telling apart.
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        let made = f
            .store
            .create_routine(card.id, "", "check", clock(Cadence::Daily), Some(1_000), true)
            .unwrap();

        f.store.record_routine_run(made.id, None, RunKind::Skipped, 1_000).unwrap();

        let history = f.store.routine_runs(made.id, 20).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].kind, RunKind::Skipped);
        assert_eq!(history[0].run_id, None, "nothing ran, so there is nothing to point at");
        assert_eq!(history[0].spent.calls, 0);
        assert_eq!(history[0].spent.cost, None, "unpriced, not free");
    }

    #[test]
    fn skipping_a_firing_is_kept_on_the_routine_through_an_edit() {
        // Set by an operator in the panel or by an agent through `schedule`,
        // and read on every sweep. A row that lost it on an unrelated edit is a
        // routine that quietly starts stacking again.
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        let made = f
            .store
            .create_routine(
                card.id,
                "Listings sweep",
                "check the listings",
                clock(Cadence::Every(3600)),
                Some(1_000_000),
                true,
            )
            .unwrap();
        assert!(made.skip_if_working);
        assert!(f.store.get_routine(made.id).unwrap().unwrap().skip_if_working);
        assert!(f.store.agent_routines(card.id).unwrap()[0].skip_if_working);
        assert!(f.store.due_routines(2_000_000).unwrap()[0].skip_if_working);

        let edited = f
            .store
            .update_routine(
                made.id,
                "Listings sweep",
                "check the listings twice",
                clock(Cadence::Every(3600)),
                made.next_run_at,
                false,
            )
            .unwrap();
        assert!(!edited.skip_if_working, "and it comes back off when that is what was saved");
    }

    #[test]
    fn deleting_a_routine_takes_its_history_with_it() {
        // Rows pointing at a routine that no longer exists are rows nothing can
        // ever draw, and the id is free to be reused.
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        let made = f
            .store
            .create_routine(card.id, "", "check", clock(Cadence::Daily), Some(1_000), false)
            .unwrap();
        f.store.record_routine_run(made.id, Some(RunId::new()), RunKind::Scheduled, 1_000).unwrap();

        f.store.delete_routine(made.id).unwrap();
        assert!(f.store.routine_runs(made.id, 20).unwrap().is_empty());
    }

    #[test]
    fn a_routine_waiting_on_an_event_is_never_due_and_survives_firing() {
        // The scheduler asks one question: what is due. A trigger that is not a
        // clock has to answer "not me" without the query knowing it exists, and
        // it must not be deleted like a one-shot when it does fire: it fires
        // every time its event happens.
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        let trigger = Trigger::Event(EventTrigger {
            service: "stripe".into(),
            topic: "invoice.payment_failed".into(),
        });
        let made = f
            .store
            .create_routine(card.id, "Dunning", "chase it", trigger.clone(), None, false)
            .unwrap();

        assert_eq!(made.next_run_at, None, "it holds no slot");
        assert!(
            f.store.due_routines(i64::MAX).unwrap().is_empty(),
            "and no moment, however far ahead, makes it due"
        );

        // Fired anyway, which today is the operator pressing Test run.
        f.store.routine_ran(&made, 5_000).unwrap();
        let after = f.store.get_routine(made.id).unwrap().unwrap();
        assert_eq!(after.trigger, trigger, "the trigger survives the round trip");
        assert_eq!(after.next_run_at, None, "still holding no slot");
        assert_eq!(after.last_run_at, Some(5_000), "and it recorded having run");
    }

    #[test]
    fn a_schedule_lists_what_is_due_soonest_and_what_is_waiting_last() {
        // NULL sorts first in SQLite, so the panel drew a routine waiting on an
        // event above one firing in ten minutes.
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        f.store
            .create_routine(
                card.id,
                "Waiting",
                "chase it",
                Trigger::Event(EventTrigger { service: "stripe".into(), topic: "x.y".into() }),
                None,
                false,
            )
            .unwrap();
        f.store
            .create_routine(card.id, "Later", "b", clock(Cadence::Daily), Some(9_000), false)
            .unwrap();
        f.store
            .create_routine(card.id, "Sooner", "a", clock(Cadence::Daily), Some(1_000), false)
            .unwrap();

        let names: Vec<String> =
            f.store.agent_routines(card.id).unwrap().into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["Sooner", "Later", "Waiting"]);
    }

    #[test]
    fn a_firing_reports_what_it_spent_and_a_firing_that_did_nothing_says_so() {
        // The history's job is "has this been working". A delivery that bought
        // no model call is a routine that did not run, and the row is otherwise
        // identical to one that did.
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        let made = f
            .store
            .create_routine(card.id, "", "check", clock(Cadence::Daily), Some(1_000), false)
            .unwrap();

        let worked = RunId::new();
        let silent = RunId::new();
        f.store.record_routine_run(made.id, Some(worked), RunKind::Scheduled, 1_000).unwrap();
        f.store.record_routine_run(made.id, Some(silent), RunKind::Scheduled, 2_000).unwrap();
        for (prompt, completion, cost) in [(900u32, 100u32, Some(0.002)), (400, 50, Some(0.001))] {
            f.store
                .record_usage(&UsageEntry {
                    agent_id: card.id,
                    group_id: card.group_id,
                    run_id: worked,
                    model: "test/model".into(),
                    prompt,
                    completion,
                    cost,
                })
                .unwrap();
        }

        let history = f.store.routine_runs(made.id, 20).unwrap();
        assert_eq!(history.len(), 2, "one row per firing, whatever it spent");
        assert_eq!(history[0].run_id, Some(silent), "newest first");
        assert_eq!(history[0].spent.calls, 0, "nothing was spent, so nothing ran");
        assert_eq!(history[0].spent.cost, None, "and unpriced is not free");
        assert_eq!(history[1].spent.calls, 2, "model calls, not turns");
        assert_eq!(history[1].spent.total(), 1450);
        assert_eq!(history[1].spent.cost, Some(0.003));
    }

    #[test]
    fn a_one_shot_that_fires_leaves_no_row_behind() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        let made = f
            .store
            .create_routine(card.id, "", "wake me", clock(Cadence::Once), Some(1_000), false)
            .unwrap();
        f.store.routine_ran(&made, 2_000).unwrap();
        assert!(f.store.get_routine(made.id).unwrap().is_none());
    }

    /// 2025-01-03 at nine in the morning, wherever this is running.
    fn friday_at_nine() -> i64 {
        use chrono::{Local, NaiveDate, TimeZone};
        let naive = NaiveDate::from_ymd_opt(2025, 1, 3)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .expect("nine in the morning exists");
        Local.from_local_datetime(&naive).earliest().unwrap().timestamp_millis()
    }

    #[test]
    fn clearing_a_group_empties_its_crew_and_leaves_everyone_elses_channels() {
        let f = fixture();
        let other = f
            .store
            .create_group(&CleanGroup { name: "Research".into(), ..Default::default() })
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

        f.store
            .create_routine(
                scholar.id,
                "",
                "check the listings",
                clock(Cadence::Every(3600)),
                Some(1),
                false,
            )
            .unwrap();
        f.store
            .record_usage(&UsageEntry {
                agent_id: scholar.id,
                group_id: other.id,
                run_id: run,
                model: "test/model".into(),
                prompt: 10,
                completion: 2,
                cost: Some(0.01),
            })
            .unwrap();

        assert_eq!(f.store.delete_group_messages(other.id).unwrap(), 1);
        assert_eq!(f.store.delete_group_routines(other.id).unwrap(), 1);
        assert_eq!(f.store.delete_group_usage(other.id).unwrap(), 1);
        assert!(f.store.agent_routines(scholar.id).unwrap().is_empty());
        assert!(
            !f.store.usage_by_group().unwrap().contains_key(&other.id),
            "a reset group has spent nothing, or the meter keeps counting what is gone"
        );
        // A deleted agent is still one of the group's, so a reset takes its
        // transcript too rather than leaving half of one behind.
        assert!(f.store.group_agent_ids(other.id).unwrap().contains(&scholar.id));
        assert!(f.store.channel_messages(scholar.id, 50).unwrap().is_empty());
        assert_eq!(
            f.store.channel_messages(bystander.id, 50).unwrap().len(),
            1,
            "clearing one group must not touch another"
        );
    }

    #[test]
    fn a_disband_is_told_the_group_cannot_go_before_it_takes_anything() {
        // The failure path this covers: a disband kills every computer and
        // browser in a crew and only then asks whether the group itself may be
        // deleted. The first group may not, and the operator would be left with
        // the machines gone, the agents gone and the group still there.
        let f = fixture();
        let default = default_group_id();
        let mut d = draft("Resident");
        d.group_id = Some(default);
        f.store.create_agent(&d).unwrap();

        assert!(matches!(
            f.store.group_for_removal(default),
            Err(StoreError::CannotDeleteDefaultGroup)
        ));

        // The same question answered for a group that can go, while it is still
        // full: the check is about the group, not about whether it is empty.
        let group = f
            .store
            .create_group(&CleanGroup { name: "Research".into(), ..Default::default() })
            .unwrap();
        let mut d = draft("Scholar");
        d.group_id = Some(group.id);
        f.store.create_agent(&d).unwrap();
        assert!(f.store.group_for_removal(group.id).is_ok());
        assert!(
            matches!(f.store.delete_group(group.id), Err(StoreError::GroupNotEmpty { .. })),
            "the emptiness check has to stay on delete_group"
        );
    }

    #[test]
    fn a_disband_takes_the_live_crew_and_not_the_agents_already_deleted() {
        // A terminated agent's sandbox is already destroyed and its memory is
        // already gone. Handing one back would make a disband try to kill a
        // machine that is not there and show the operator that failure.
        let f = fixture();
        let group = f
            .store
            .create_group(&CleanGroup { name: "Research".into(), ..Default::default() })
            .unwrap();

        let mut live = draft("Scholar");
        live.group_id = Some(group.id);
        let live = f.store.create_agent(&live).unwrap();

        let mut gone = draft("Departed");
        gone.group_id = Some(group.id);
        let gone = f.store.create_agent(&gone).unwrap();
        f.store.set_lifecycle(gone.id, Lifecycle::Terminated).unwrap();

        // And somebody in another crew, who a disband must never reach.
        let bystander = f.store.create_agent(&draft("Bystander")).unwrap();

        let crew = f.store.group_crew(group.id).unwrap();
        assert_eq!(crew.iter().map(|c| c.id).collect::<Vec<_>>(), vec![live.id]);
        assert!(!crew.iter().any(|c| c.id == bystander.id));

        // What the command does after retiring them, which is the reason the
        // count above has to be right: the group is empty and goes.
        f.store.set_lifecycle(live.id, Lifecycle::Terminated).unwrap();
        assert!(f.store.group_crew(group.id).unwrap().is_empty());
        f.store.delete_group(group.id).expect("a disbanded group must be deletable");
    }

    #[test]
    fn a_group_with_live_agents_still_refuses_to_go() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();
        let group =
            store.create_group(&CleanGroup { name: "Busy".into(), ..Default::default() }).unwrap();
        let mut d = draft("Busy One");
        d.group_id = Some(group.id);
        store.create_agent(&d).unwrap();

        assert!(matches!(store.delete_group(group.id), Err(StoreError::GroupNotEmpty { .. })));
    }

    #[test]
    fn a_groups_settings_round_trip_and_the_key_only_travels_one_way() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();

        let group = store
            .create_group(&CleanGroup {
                name: "Local".into(),
                inference: Some(InferenceOverrides {
                    provider: Some(crate::config::Provider::Compatible),
                    base_url: Some("http://localhost:1234/v1".into()),
                    default_model: Some("local/qwen".into()),
                    subscription_model: Some("gpt-5.4".into()),
                    request_timeout_secs: Some(600),
                }),
                api_key: Some(Some("sk-group-9999".into())),
                limits: Some(GroupLimits { max_steps_per_run: Some(9), ..Default::default() }),
            })
            .unwrap();

        assert_eq!(group.inference.provider, Some(crate::config::Provider::Compatible));
        assert_eq!(group.inference.base_url.as_deref(), Some("http://localhost:1234/v1"));
        assert_eq!(group.inference.subscription_model.as_deref(), Some("gpt-5.4"));
        assert_eq!(group.inference.request_timeout_secs, Some(600));
        assert_eq!(group.limits.max_steps_per_run, Some(9));
        assert_eq!(group.limits.max_hops, None, "an unset limit inherits");

        // The card the UI sees says a key is set and nothing more, and the
        // runtime's read is the only place the value exists.
        assert!(group.api_key_set);
        assert_eq!(group.api_key_hint, "...9999");
        let json = serde_json::to_string(&group).unwrap();
        assert!(!json.contains("sk-group"), "a group card leaked its key: {json}");
        assert_eq!(
            store.group_inference(group.id).unwrap().api_key.as_deref(),
            Some("sk-group-9999")
        );
        assert_eq!(store.group_limits(group.id).unwrap().max_steps_per_run, Some(9));
    }

    #[test]
    fn an_absent_block_leaves_that_half_of_a_groups_settings_alone() {
        // What lets the editor save a name without holding the key, and what
        // stops a caller that knows nothing about limits from clearing them.
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();

        let group = store
            .create_group(&CleanGroup {
                name: "Research".into(),
                inference: Some(InferenceOverrides {
                    default_model: Some("local/qwen".into()),
                    ..Default::default()
                }),
                api_key: Some(Some("sk-group-9999".into())),
                limits: Some(GroupLimits { max_hops: Some(3), ..Default::default() }),
            })
            .unwrap();

        let renamed = store
            .update_group(group.id, &CleanGroup { name: "Reading".into(), ..Default::default() })
            .unwrap();

        assert_eq!(renamed.name, "Reading");
        assert_eq!(renamed.inference.default_model.as_deref(), Some("local/qwen"));
        assert_eq!(renamed.limits.max_hops, Some(3));
        assert!(renamed.api_key_set);
    }

    #[test]
    fn a_present_block_of_nulls_puts_a_group_back_on_the_app_settings() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();

        let group = store
            .create_group(&CleanGroup {
                name: "Research".into(),
                inference: Some(InferenceOverrides {
                    provider: Some(crate::config::Provider::Chatgpt),
                    subscription_model: Some("gpt-5.4".into()),
                    ..Default::default()
                }),
                limits: Some(GroupLimits { max_hops: Some(3), ..Default::default() }),
                ..Default::default()
            })
            .unwrap();

        let cleared = store
            .update_group(
                group.id,
                &CleanGroup {
                    name: "Research".into(),
                    inference: Some(InferenceOverrides::default()),
                    limits: Some(GroupLimits::default()),
                    api_key: None,
                },
            )
            .unwrap();

        assert_eq!(cleared.inference, InferenceOverrides::default());
        assert_eq!(cleared.limits, GroupLimits::default());
        assert_eq!(store.group_inference(group.id).unwrap(), GroupInference::default());
        assert_eq!(store.group_limits(group.id).unwrap(), GroupLimits::default());
    }

    #[test]
    fn a_provider_written_by_a_newer_build_reads_as_inherit() {
        // A group whose provider nothing here recognizes has to keep working on
        // the app settings. Refusing to read the row would take a whole crew
        // offline over a column this build has never heard of.
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();
        let group = store.create_group(&group_named("Future")).unwrap();
        store
            .conn()
            .unwrap()
            .execute(
                "UPDATE groups SET provider='anthropic-native' WHERE id=?1",
                params![group.id.to_string()],
            )
            .unwrap();

        assert_eq!(store.get_group(group.id).unwrap().unwrap().inference.provider, None);
        assert_eq!(store.group_inference(group.id).unwrap().overrides.provider, None);
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
    fn a_crew_written_in_one_go_is_readable_as_a_crew() {
        let f = fixture();
        let crew = [draft("Manager"), draft("Researcher"), draft("Critic")];
        let cards = f.store.create_agents(&crew).unwrap();

        assert_eq!(cards.len(), 3);
        let names: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Manager", "Researcher", "Critic"], "order is the order asked for");
        assert_eq!(f.store.list_agents().unwrap().len(), 3);
        // Every one is a real card, not a half-written row.
        for card in &cards {
            assert_eq!(f.store.get_agent(card.id).unwrap().as_ref(), Some(card));
        }
    }

    #[test]
    fn a_hired_crew_lands_at_the_bottom_of_the_rail_in_the_order_it_was_picked() {
        // The rail is an arrangement the operator made by hand, so a crew has
        // to arrive under it rather than inside it, and every hire needs its own
        // slot. Asking for the bottom once per agent would hand the whole batch
        // the same answer, because none of them is written until the commit.
        let f = fixture();
        let first = f.store.create_agent(&draft("Chief of Staff")).unwrap();
        let second = f.store.create_agent(&draft("Executive Assistant")).unwrap();

        let crew = [draft("Paralegal"), draft("Bookkeeper"), draft("QA Tester")];
        let hired = f.store.create_agents(&crew).unwrap();

        let slots: Vec<i32> = hired.iter().map(|c| c.rail_order).collect();
        let bottom = second.rail_order;
        assert_eq!(
            slots,
            vec![bottom + 1, bottom + 2, bottom + 3],
            "a hired crew shared a slot or landed inside the arrangement: {slots:?}"
        );
        assert!(slots.iter().all(|slot| *slot > first.rail_order));

        // And the rail agrees: `list_agents` reads back in the arrangement.
        let order: Vec<String> =
            f.store.list_agents().unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(
            order,
            ["Chief of Staff", "Executive Assistant", "Paralegal", "Bookkeeper", "QA Tester"]
        );
    }

    #[test]
    fn a_crew_that_cannot_be_written_in_full_is_not_written_at_all() {
        // The realistic failure: a name taken between the check and the write.
        // Three agents landing plus an error about a fourth is a workspace the
        // operator did not ask for and no list of what arrived.
        let f = fixture();
        f.store.create_agent(&draft("Critic")).unwrap();

        let crew = [draft("Manager"), draft("Researcher"), draft("critic")];
        let err = f.store.create_agents(&crew).unwrap_err();
        assert!(
            matches!(&err, StoreError::DuplicateName(name) if name == "critic"),
            "expected DuplicateName, got {err:?}"
        );

        let left: Vec<String> =
            f.store.list_agents().unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(left, ["Critic"], "the rolled-back hire left agents behind: {left:?}");
    }

    #[test]
    fn hiring_nobody_writes_nothing_and_is_not_an_error() {
        let f = fixture();
        assert!(f.store.create_agents(&[]).unwrap().is_empty());
        assert!(f.store.list_agents().unwrap().is_empty());
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

    // ---- search ----------------------------------------------------------

    /// A workspace with something of every searchable kind in it.
    struct Searchable {
        f: Fixture,
        writer: AgentId,
    }

    fn searchable() -> Searchable {
        let f = fixture();
        let writer = f.store.create_agent(&draft("Scribe")).unwrap();
        let run = RunId::new();
        let mut at = 1_000;

        let mut send = |parts: Vec<Part>| {
            let mut e = envelope(Participant::Human, Participant::Agent { id: writer.id }, "", run);
            e.parts = parts;
            e.created_at = at;
            at += 1;
            f.store.append(&e).unwrap();
        };

        send(vec![Part::text("The quarterly budget is signed off.")]);
        send(vec![Part::text("Nothing to do with money at all.")]);
        send(vec![Part::text(
            "Sources: https://example.com/budget-q3 and https://example.com/other",
        )]);
        send(vec![
            Part::text("here is the deck"),
            Part::File(Attachment {
                digest: "aaa".into(),
                name: "budget.pdf".into(),
                mime: "application/pdf".into(),
                bytes: 2048,
            }),
        ]);
        f.store
            .create_routine(
                writer.id,
                "",
                "post the budget summary",
                clock(Cadence::Every(3600)),
                Some(5_000),
                false,
            )
            .unwrap();
        f.store
            .create_routine(
                writer.id,
                "Watering",
                "the plants",
                clock(Cadence::Daily),
                Some(4_000),
                false,
            )
            .unwrap();

        Searchable { f, writer: writer.id }
    }

    #[test]
    fn a_message_is_found_by_its_text() {
        let s = searchable();
        let hits = s.f.store.search("budget", 20).unwrap();
        let found: Vec<&str> = hits.messages.iter().map(|m| m.excerpt.as_str()).collect();
        assert_eq!(found.len(), 2, "the two bodies that say it: {found:?}");
        assert!(found.iter().all(|e| e.to_lowercase().contains("budget")), "{found:?}");
        assert!(hits.messages.iter().all(|m| m.channel_id == s.writer), "a hit says where to go");
    }

    #[test]
    fn a_json_key_is_not_a_word_anybody_wrote() {
        // `parts` is stored as JSON, so a substring search over the blob also
        // matches its keys. Every row has "text", "type" and "name" in it, and
        // searching for any of them used to return the whole transcript.
        let s = searchable();
        for key in ["text", "type", "name", "digest", "mime"] {
            let hits = s.f.store.search(key, 20).unwrap();
            assert!(
                hits.messages.is_empty(),
                "{key:?} matched {} messages nobody wrote it in",
                hits.messages.len()
            );
        }
    }

    #[test]
    fn matching_ignores_case_and_a_wildcard_is_a_character() {
        let s = searchable();
        assert_eq!(s.f.store.search("QUARTERLY", 20).unwrap().messages.len(), 1);
        // Unescaped this is `%%%`, which matches every row and reports the
        // whole database as a hit for a query nothing contains.
        assert!(s.f.store.search("%", 20).unwrap().messages.is_empty());
        assert!(s.f.store.search("_", 20).unwrap().messages.is_empty());
    }

    #[test]
    fn a_file_is_found_by_name_and_appears_once_however_often_it_was_sent() {
        let s = searchable();
        let run = RunId::new();
        // The same document again, to a second agent. Content addressing means
        // this is one file, so a result list showing it twice is showing the
        // same row twice.
        let other = s.f.store.create_agent(&draft("Critic")).unwrap();
        let mut again = envelope(Participant::Human, Participant::Agent { id: other.id }, "", run);
        again.parts = vec![Part::File(Attachment {
            digest: "aaa".into(),
            name: "budget.pdf".into(),
            mime: "application/pdf".into(),
            bytes: 2048,
        })];
        again.created_at = 9_000;
        s.f.store.append(&again).unwrap();

        let hits = s.f.store.search("budget.pdf", 20).unwrap();
        assert_eq!(hits.files.len(), 1, "one document, one row: {:?}", hits.files);
        assert_eq!(hits.files[0].file.name, "budget.pdf");
        assert_eq!(hits.files[0].file.bytes, 2048);
        assert_eq!(hits.files[0].channel_id, other.id, "the newest copy is the one to open");
    }

    #[test]
    fn a_link_is_pulled_out_of_the_message_that_carried_it() {
        let s = searchable();
        let hits = s.f.store.search("budget", 20).unwrap();
        let urls: Vec<&str> = hits.links.iter().map(|l| l.url.as_str()).collect();
        // The other URL is in a message that matched, which is not the same as
        // the URL matching: a message can mention a subject and link elsewhere.
        assert_eq!(urls, vec!["https://example.com/budget-q3"]);
        assert_eq!(hits.links[0].channel_id, s.writer);
    }

    #[test]
    fn a_routine_is_found_by_what_it_says_or_by_its_title() {
        let s = searchable();
        let hits = s.f.store.search("budget", 20).unwrap();
        assert_eq!(hits.routines.len(), 1);
        assert_eq!(hits.routines[0].what, "post the budget summary");

        // A routine an operator titled is found by the title, which is the part
        // they see in the list and therefore the part they will type.
        let titled = s.f.store.search("watering", 20).unwrap();
        assert_eq!(titled.routines.len(), 1);
        assert_eq!(titled.routines[0].name, "Watering");
    }

    #[test]
    fn an_agents_own_activity_record_is_not_a_message_to_search() {
        let s = searchable();
        let mut record = envelope(
            Participant::Agent { id: s.writer },
            Participant::System,
            "budget bookkeeping",
            RunId::new(),
        );
        record.created_at = 8_000;
        s.f.store.append(&record).unwrap();

        let hits = s.f.store.search("bookkeeping", 20).unwrap();
        assert!(hits.messages.is_empty(), "a tool trail is not something anybody said");
    }

    #[test]
    fn an_empty_query_comes_back_with_the_newest_of_each_kind() {
        // The palette opens before anybody types. Empty-handed there reads as
        // a workspace with nothing in it.
        let s = searchable();
        let hits = s.f.store.search("", 20).unwrap();
        assert_eq!(hits.messages.len(), 4);
        assert_eq!(hits.files.len(), 1);
        assert_eq!(hits.links.len(), 2);
        assert_eq!(hits.routines.len(), 2);
        assert!(
            hits.messages[0].created_at > hits.messages[1].created_at,
            "newest first, so the palette opens on what just happened"
        );
    }

    #[test]
    fn a_limit_cuts_every_category() {
        let s = searchable();
        let hits = s.f.store.search("", 1).unwrap();
        assert_eq!(hits.messages.len(), 1);
        assert_eq!(hits.links.len(), 1);
        assert_eq!(hits.routines.len(), 1);
    }

    #[test]
    fn a_channel_window_can_be_widened_to_reach_an_old_message() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Manager")).unwrap();
        let run = RunId::new();
        let mut first = None;
        for i in 0..50 {
            let mut e = envelope(
                Participant::Human,
                Participant::Agent { id: card.id },
                &format!("m{i}"),
                run,
            );
            e.created_at = 1_000 + i as i64;
            f.store.append(&e).unwrap();
            if i == 0 {
                first = Some(e.id);
            }
        }

        let window = f.store.channel_messages_through(card.id, first.unwrap(), 1000).unwrap();
        assert_eq!(window.len(), 50, "the whole channel, because the target is at the bottom");
        assert_eq!(window[0].plain_text(), "m0");
        assert_eq!(window[49].plain_text(), "m49");
    }

    #[test]
    fn widening_to_a_message_that_is_gone_returns_nothing_rather_than_everything() {
        // The caller falls back to the newest window. Returning the whole
        // channel here would make a cleared history look like a successful
        // jump to a message that no longer exists.
        let f = fixture();
        let card = f.store.create_agent(&draft("Manager")).unwrap();
        f.store
            .append(&envelope(
                Participant::Human,
                Participant::Agent { id: card.id },
                "still here",
                RunId::new(),
            ))
            .unwrap();

        let window = f.store.channel_messages_through(card.id, MessageId::new(), 1000).unwrap();
        assert!(window.is_empty());
    }

    #[test]
    fn a_pair_thread_holds_both_directions_and_nobody_else() {
        // The reason this query exists: neither agent's channel has the
        // exchange. A to B is filed under B, B's answer under A, and the
        // operator opening the thread expects to read it in order.
        let f = fixture();
        let a = f.store.create_agent(&draft("A")).unwrap();
        let b = f.store.create_agent(&draft("B")).unwrap();
        let c = f.store.create_agent(&draft("C")).unwrap();
        let run = RunId::new();
        let agent = |id| Participant::Agent { id };

        let mut at = 1_000;
        let mut send = |from, to, text: &str| {
            let mut e = envelope(from, to, text, run);
            e.created_at = at;
            at += 1;
            f.store.append(&e).unwrap();
        };
        send(Participant::Human, agent(a.id), "operator, not part of it");
        send(agent(a.id), agent(b.id), "ask");
        send(agent(b.id), agent(a.id), "answer");
        send(agent(a.id), agent(c.id), "someone else entirely");
        // Bookkeeping A filed against itself. Filed to `system`, so not said to
        // anyone, and it carries A's private working notes.
        send(agent(a.id), Participant::System, "tool trail");
        send(agent(a.id), agent(b.id), "thanks");

        let thread: Vec<String> = f
            .store
            .pair_messages(a.id, b.id, 50)
            .unwrap()
            .iter()
            .map(Envelope::plain_text)
            .collect();
        assert_eq!(thread, vec!["ask", "answer", "thanks"]);

        // And the pair is unordered: which one you clicked from does not change
        // what was said.
        let reversed: Vec<String> = f
            .store
            .pair_messages(b.id, a.id, 50)
            .unwrap()
            .iter()
            .map(Envelope::plain_text)
            .collect();
        assert_eq!(reversed, thread);
    }

    #[test]
    fn a_pair_thread_limit_keeps_the_newest_messages_in_order() {
        let f = fixture();
        let a = f.store.create_agent(&draft("A")).unwrap();
        let b = f.store.create_agent(&draft("B")).unwrap();
        let run = RunId::new();
        let agent = |id| Participant::Agent { id };

        for i in 0..10 {
            let mut e = envelope(agent(a.id), agent(b.id), &format!("m{i}"), run);
            e.created_at = 1_000 + i as i64;
            f.store.append(&e).unwrap();
        }

        let texts: Vec<String> = f
            .store
            .pair_messages(a.id, b.id, 3)
            .unwrap()
            .iter()
            .map(Envelope::plain_text)
            .collect();
        assert_eq!(texts, vec!["m7", "m8", "m9"]);
    }

    #[test]
    fn a_pair_with_nothing_between_them_is_empty_rather_than_everything() {
        let f = fixture();
        let a = f.store.create_agent(&draft("A")).unwrap();
        let b = f.store.create_agent(&draft("B")).unwrap();
        let run = RunId::new();
        f.store
            .append(&envelope(
                Participant::Human,
                Participant::Agent { id: a.id },
                "only the operator",
                run,
            ))
            .unwrap();

        assert!(f.store.pair_messages(a.id, b.id, 50).unwrap().is_empty());
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
            Part::tool_call(
                "send_message",
                serde_json::json!({"to": "Chef"}),
                ToolOutcome::Refused { reason: "duplicate".into() },
            ),
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
                "INSERT INTO messages (id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,intent,cause,created_at)
                 VALUES (?1,?2,?3,'agent',NULL,'human',NULL,'[]','peer',0,1,'courtesy',NULL,1)",
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

    // ---- approvals -------------------------------------------------------

    /// One request from `agent`, pending.
    fn ask(store: &Store, agent: &AgentCard) -> Approval {
        store
            .create_approval(
                agent.id,
                agent.group_id,
                RunId::new(),
                Request::Permission { action: ProtectedAction::CreateAgent },
                "Manager wants to create an agent called Scout",
                &[DetailField::new("Name", "Scout")],
            )
            .unwrap()
    }

    /// One question from `agent`, pending.
    fn asks(store: &Store, agent: &AgentCard, options: &[&str]) -> Approval {
        store
            .create_approval(
                agent.id,
                agent.group_id,
                RunId::new(),
                Request::Question { options: options.iter().map(|it| (*it).to_string()).collect() },
                "Which vendor?",
                &[],
            )
            .unwrap()
    }

    #[test]
    fn a_question_comes_back_as_a_question_with_the_choices_it_was_given() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let written = asks(&f.store, &manager, &["Northwind", "Contoso"]);

        let read = f.store.get_approval(written.id).unwrap().unwrap();
        assert_eq!(
            read.request,
            Request::Question { options: vec!["Northwind".to_string(), "Contoso".to_string()] }
        );
        assert_eq!(read.answer, None, "nothing is answered until somebody answers it");
    }

    #[test]
    fn a_question_with_no_choices_is_one_that_takes_a_written_answer() {
        // An empty list and an absent list are the same thing to read back and
        // must not be: the column is NULL for a permission, which has no list
        // at all, and an empty list is a real state on a question.
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let written = asks(&f.store, &manager, &[]);

        let read = f.store.get_approval(written.id).unwrap().unwrap();
        assert_eq!(read.request, Request::Question { options: Vec::new() });
    }

    #[test]
    fn a_permission_is_still_a_permission_and_carries_no_choices() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let written = ask(&f.store, &manager);

        let read = f.store.get_approval(written.id).unwrap().unwrap();
        assert_eq!(read.request, Request::Permission { action: ProtectedAction::CreateAgent });
        assert_eq!(read.request.action(), Some(ProtectedAction::CreateAgent));
    }

    #[test]
    fn an_answer_is_recorded_beside_the_state_that_says_it_was_answered() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let written = asks(&f.store, &manager, &["Northwind", "Contoso"]);

        let settled = f.store.answer_approval(written.id, "Northwind").unwrap();
        assert_eq!(settled.state, ApprovalState::Answered);
        assert_eq!(settled.answer.as_deref(), Some("Northwind"));
    }

    #[test]
    fn a_question_that_has_been_answered_cannot_be_answered_again() {
        // The same race the verdict path has: the operator's answer and the
        // turn's own timeout both land here, and whichever is second must not
        // overwrite the first.
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let written = asks(&f.store, &manager, &[]);

        f.store.answer_approval(written.id, "Northwind").unwrap();
        assert!(matches!(
            f.store.answer_approval(written.id, "Contoso"),
            Err(StoreError::ApprovalSettled { state: ApprovalState::Answered })
        ));
        assert_eq!(
            f.store.get_approval(written.id).unwrap().unwrap().answer.as_deref(),
            Some("Northwind")
        );
    }

    #[test]
    fn a_question_is_never_a_standing_grant_and_never_shows_up_as_one() {
        // `alwaysAllow` is the one state that outlives its turn, and there is
        // no standing yes to a question: nothing to be let off asking, because
        // asking is the whole of what it does.
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        asks(&f.store, &manager, &["Northwind"]);

        assert!(f.store.standing_grants(manager.id).unwrap().is_empty());
        assert!(!f.store.has_standing_grant(manager.id, ProtectedAction::CreateAgent).unwrap());
        assert!(!f.store.has_standing_grant(manager.id, ProtectedAction::ActOnBehalf).unwrap());
    }

    #[test]
    fn a_question_waits_in_the_same_queue_a_permission_does() {
        // One desk, one queue. A question left out of this read would be a
        // parked turn the operator has no list of.
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        ask(&f.store, &manager);
        asks(&f.store, &manager, &["Northwind", "Contoso"]);

        let waiting = f.store.pending_approvals(10).unwrap();
        assert_eq!(waiting.len(), 2);
        assert!(waiting.iter().any(|it| it.request.action().is_some()));
        assert!(waiting.iter().any(|it| it.request.action().is_none()));
    }

    #[test]
    fn a_standing_grant_belongs_to_the_one_agent_it_was_given_to() {
        // "Always allow" is an answer about the agent that asked. Reading it as
        // a workspace-wide setting would let an agent the operator has never
        // been asked about act on somebody else's permission.
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let critic = f.store.create_agent(&draft("Critic")).unwrap();

        let request = ask(&f.store, &manager);
        assert!(!f.store.has_standing_grant(manager.id, ProtectedAction::CreateAgent).unwrap());

        f.store.settle_approval(request.id, ApprovalState::AlwaysAllow).unwrap();
        assert!(f.store.has_standing_grant(manager.id, ProtectedAction::CreateAgent).unwrap());
        assert!(!f.store.has_standing_grant(critic.id, ProtectedAction::CreateAgent).unwrap());
    }

    #[test]
    fn allowing_once_grants_nothing_standing() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let request = ask(&f.store, &manager);

        f.store.settle_approval(request.id, ApprovalState::Allow).unwrap();
        assert!(
            !f.store.has_standing_grant(manager.id, ProtectedAction::CreateAgent).unwrap(),
            "one yes must not become every yes"
        );
    }

    #[test]
    fn a_request_can_only_be_answered_once() {
        // The operator's click and the turn's own timeout both land here. The
        // loser must not overwrite the winner: a request the agent has already
        // given up on, recorded as granted, is a standing grant for something
        // that never happened.
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let request = ask(&f.store, &manager);

        let allowed = f.store.settle_approval(request.id, ApprovalState::Allow).unwrap();
        assert_eq!(allowed.state, ApprovalState::Allow);
        assert!(allowed.decided_at.is_some());

        let second = f.store.settle_approval(request.id, ApprovalState::Expired);
        assert!(
            matches!(second, Err(StoreError::ApprovalSettled { state: ApprovalState::Allow })),
            "{second:?}"
        );
        assert_eq!(
            f.store.get_approval(request.id).unwrap().unwrap().state,
            ApprovalState::Allow,
            "the first answer has to survive the second"
        );
    }

    #[test]
    fn a_restart_closes_what_was_still_waiting_and_leaves_the_rest() {
        // Nothing holds a parked turn across a restart, so a pending row is a
        // question nobody can answer any more.
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let answered = ask(&f.store, &manager);
        let waiting = ask(&f.store, &manager);
        f.store.settle_approval(answered.id, ApprovalState::Deny).unwrap();

        assert_eq!(f.store.expire_pending_approvals().unwrap(), 1);
        assert_eq!(
            f.store.get_approval(waiting.id).unwrap().unwrap().state,
            ApprovalState::Expired
        );
        assert_eq!(
            f.store.get_approval(answered.id).unwrap().unwrap().state,
            ApprovalState::Deny,
            "a decision the operator made is not a casualty of a restart"
        );
    }

    #[test]
    fn what_is_still_waiting_is_read_whole_and_oldest_first() {
        // The menu bar reads this to offer an answer without the window. Whole,
        // because a row that says only that something is pending cannot be
        // answered from a menu; oldest first, because the request with least of
        // its ten minutes left is the one worth putting at the top.
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();

        let first = ask(&f.store, &manager);
        // `created_at` is milliseconds, and two rows written in the same
        // millisecond would make the order the tiebreak rather than the clock.
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = ask(&f.store, &manager);
        let answered = ask(&f.store, &manager);
        f.store.settle_approval(answered.id, ApprovalState::Allow).unwrap();

        let waiting = f.store.pending_approvals(50).unwrap();
        assert_eq!(
            waiting.iter().map(|a| a.id).collect::<Vec<_>>(),
            vec![first.id, second.id],
            "an answered request is not waiting on anybody"
        );
        assert_eq!(
            waiting[0].summary, "Manager wants to create an agent called Scout",
            "the wording is the whole reason this is read as rows and not ids"
        );
        assert_eq!(waiting[0].detail, vec![DetailField::new("Name", "Scout")]);

        assert_eq!(f.store.pending_approvals(1).unwrap().len(), 1, "and it is bounded");

        f.store.expire_pending_approvals().unwrap();
        assert!(f.store.pending_approvals(50).unwrap().is_empty());
    }

    #[test]
    fn deleting_an_agent_takes_the_permission_it_was_given() {
        // A deleted agent frees its name, and the next agent to hold it must
        // not inherit what the operator allowed somebody else.
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let request = ask(&f.store, &manager);
        f.store.settle_approval(request.id, ApprovalState::AlwaysAllow).unwrap();

        assert_eq!(f.store.delete_agent_approvals(manager.id).unwrap(), 1);
        assert!(!f.store.has_standing_grant(manager.id, ProtectedAction::CreateAgent).unwrap());
        assert!(f.store.get_approval(request.id).unwrap().is_none());
    }

    #[test]
    fn a_standing_grant_can_be_taken_back() {
        // "Always allow" is one click about every future request. A permission
        // that could only ever be given would make that click a decision to
        // agonize over, which is the opposite of what it is for.
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let request = ask(&f.store, &manager);
        f.store.settle_approval(request.id, ApprovalState::AlwaysAllow).unwrap();
        assert_eq!(
            f.store.standing_grants(manager.id).unwrap(),
            vec![ProtectedAction::CreateAgent]
        );

        assert_eq!(f.store.revoke_grant(manager.id, ProtectedAction::CreateAgent).unwrap(), 1);
        assert!(f.store.standing_grants(manager.id).unwrap().is_empty());
        assert!(!f.store.has_standing_grant(manager.id, ProtectedAction::CreateAgent).unwrap());

        // And revoking again is a no-op rather than an error: the operator
        // clicking twice has already got what they asked for.
        assert_eq!(f.store.revoke_grant(manager.id, ProtectedAction::CreateAgent).unwrap(), 0);
    }

    #[test]
    fn revoking_leaves_answered_requests_alone() {
        // Only the standing grant goes. A one-off yes was an answer about one
        // request and stays in the record as one.
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let once = ask(&f.store, &manager);
        let standing = ask(&f.store, &manager);
        f.store.settle_approval(once.id, ApprovalState::Allow).unwrap();
        f.store.settle_approval(standing.id, ApprovalState::AlwaysAllow).unwrap();

        f.store.revoke_grant(manager.id, ProtectedAction::CreateAgent).unwrap();

        assert_eq!(f.store.get_approval(once.id).unwrap().unwrap().state, ApprovalState::Allow);
        assert!(f.store.get_approval(standing.id).unwrap().is_none());
    }

    #[test]
    fn the_state_table_carries_what_the_widgets_need_and_survives_a_round_trip() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let request = ask(&f.store, &manager);

        let stored = f.store.get_approval(request.id).unwrap().unwrap();
        assert_eq!(stored, request, "what was written is what comes back");
        assert_eq!(stored.detail, vec![DetailField::new("Name", "Scout")]);

        let states = f.store.approval_states(500).unwrap();
        assert_eq!(states.get(&request.id), Some(&ApprovalState::Pending));

        f.store.settle_approval(request.id, ApprovalState::Deny).unwrap();
        assert_eq!(
            f.store.approval_states(500).unwrap().get(&request.id),
            Some(&ApprovalState::Deny)
        );
    }

    #[test]
    fn a_plugin_row_naming_something_this_build_has_never_heard_of_is_skipped() {
        // What a downgrade leaves behind, and what a withdrawn plugin leaves
        // behind on a database migration 25 has not reached: a row naming a
        // kind this build has no variant for. Raising would cost every agent in
        // the crew its turn, over a tool list one of them was not going to use.
        let f = fixture();
        let group = default_group_id();
        f.store
            .conn()
            .unwrap()
            .execute(
                "INSERT INTO plugins (id,group_id,kind,account,tools,connected_at)
                 VALUES (?1,?2,'sourdough','','[]',0)",
                params![PluginId::new().to_string(), group.to_string()],
            )
            .unwrap();

        let agent = f.store.create_agent(&draft("Manager")).unwrap();
        assert!(f.store.group_plugins(group).unwrap().is_empty());
        assert!(f.store.plugin_tools(group, agent.id).unwrap().is_empty());
    }

    #[test]
    fn deleting_a_group_takes_its_plugins_and_their_grants_with_it() {
        // Two reasons, and the second one bites first: a grant left behind is a
        // token against the operator's own account with nothing on screen that
        // owns it, and the row itself fails the foreign key on the delete.
        let f = fixture();
        let group = f
            .store
            .create_group(&CleanGroup { name: "Crew".into(), ..Default::default() })
            .unwrap();
        f.store.save_plugin(group.id, PluginKind::Neon, "", &[], None, "").unwrap();

        f.store.delete_group(group.id).unwrap();
        assert!(f.store.group_plugins(group.id).unwrap().is_empty());
        assert!(matches!(
            f.store.plugin_reach(group.id, AgentId::new(), PluginKind::Neon, "run_sql").unwrap(),
            PluginReach::NotConnected
        ));
    }

    #[test]
    fn a_plugin_that_needed_no_sign_in_reports_no_grant_rather_than_an_empty_one() {
        // What `connect` writes for a server that asked for nothing. A blank
        // access token read back as a grant would send the operator to a
        // browser to fix something that is working, and would make every call
        // carry `Bearer `. The kind is incidental: the column is what decides.
        let f = fixture();
        let group = default_group_id();
        f.store.save_plugin(group, PluginKind::Neon, "", &[], None, "").unwrap();

        let agent = f.store.create_agent(&draft("Manager")).unwrap();
        let PluginReach::Granted { grant, .. } =
            f.store.plugin_reach(group, agent.id, PluginKind::Neon, "run_sql").unwrap()
        else {
            panic!("a connected plugin is reachable by the crew it was connected for")
        };
        assert!(grant.is_none());
        assert!(!f.store.group_plugins(group).unwrap()[0].signed_in);
    }

    #[test]
    fn a_plugin_is_the_whole_crew_s_until_somebody_says_otherwise() {
        // The default, and the reading every plugin connected before this
        // existed keeps. An operator who has never opened this control has a
        // crew that works exactly as it did.
        let f = fixture();
        let group = default_group_id();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        f.store.save_plugin(group, PluginKind::Neon, "", &[tool("run_sql")], None, "").unwrap();

        assert_eq!(f.store.group_plugins(group).unwrap()[0].access, PluginAccess::Everyone);
        assert_eq!(f.store.plugin_tools(group, manager.id).unwrap().len(), 1);

        // Including an agent hired after the plugin was connected, which is the
        // half a list of ids could not express.
        let hired = f.store.create_agent(&draft("Researcher")).unwrap();
        assert_eq!(f.store.plugin_tools(group, hired.id).unwrap().len(), 1);
    }

    #[test]
    fn a_narrowed_plugin_is_offered_to_the_chosen_and_to_nobody_else() {
        let f = fixture();
        let group = default_group_id();
        let revenue = f.store.create_agent(&draft("Revenue")).unwrap();
        let scribe = f.store.create_agent(&draft("Scribe")).unwrap();
        let plugin = f
            .store
            .save_plugin(group, PluginKind::Stripe, "", &[tool("refund")], None, "")
            .unwrap();

        let saved = f
            .store
            .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![revenue.id] })
            .unwrap();
        assert_eq!(saved.access, PluginAccess::Chosen { agents: vec![revenue.id] });

        assert_eq!(f.store.plugin_tools(group, revenue.id).unwrap().len(), 1);
        assert!(f.store.plugin_tools(group, scribe.id).unwrap().is_empty());

        // And the tool list and the grant agree, because a model can name a
        // tool it was never offered.
        assert!(matches!(
            f.store.plugin_reach(group, revenue.id, PluginKind::Stripe, "refund").unwrap(),
            PluginReach::Granted { .. }
        ));
        assert!(matches!(
            f.store.plugin_reach(group, scribe.id, PluginKind::Stripe, "refund").unwrap(),
            PluginReach::NotChosen
        ));
    }

    #[test]
    fn a_plugin_narrowed_to_nobody_is_a_plugin_nobody_gets() {
        // Reachable in the UI on the way to naming the first agent, so it has
        // to mean what it says. An empty list read as "everyone" would hand the
        // plugin back to the crew at the moment the last name was removed.
        let f = fixture();
        let group = default_group_id();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let plugin = f
            .store
            .save_plugin(group, PluginKind::Stripe, "", &[tool("refund")], None, "")
            .unwrap();

        f.store.set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![] }).unwrap();

        assert!(f.store.plugin_tools(group, manager.id).unwrap().is_empty());
        assert!(matches!(
            f.store.plugin_reach(group, manager.id, PluginKind::Stripe, "refund").unwrap(),
            PluginReach::NotChosen
        ));
    }

    #[test]
    fn opening_a_plugin_back_up_leaves_nothing_behind_that_still_names_anybody() {
        // The stored state has to be what is true. A remembered list would be a
        // decision the operator has already changed, waiting to come back.
        let f = fixture();
        let group = default_group_id();
        let revenue = f.store.create_agent(&draft("Revenue")).unwrap();
        let plugin = f.store.save_plugin(group, PluginKind::Stripe, "", &[], None, "").unwrap();

        f.store
            .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![revenue.id] })
            .unwrap();
        let opened = f.store.set_plugin_access(plugin.id, &PluginAccess::Everyone).unwrap();

        assert_eq!(opened.access, PluginAccess::Everyone);
        let left: i64 = f
            .store
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM plugin_agents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn an_agent_from_another_crew_cannot_be_named_on_a_plugin() {
        // It could never reach it — the turn asks by group as well as by agent
        // — so the row would grant nothing and draw as a name in a panel that
        // means nothing.
        let f = fixture();
        let other = f
            .store
            .create_group(&CleanGroup { name: "Elsewhere".into(), ..Default::default() })
            .unwrap();
        let outsider = f
            .store
            .create_agent(&CleanDraft { group_id: Some(other.id), ..draft("Outsider") })
            .unwrap();
        let plugin =
            f.store.save_plugin(default_group_id(), PluginKind::Stripe, "", &[], None, "").unwrap();

        let refused = f
            .store
            .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![outsider.id] })
            .unwrap_err();
        assert!(matches!(refused, StoreError::AgentNotInGroup(id) if id == outsider.id));
    }

    #[test]
    fn connecting_again_does_not_hand_a_narrowed_plugin_back_to_the_crew() {
        // Reconnecting is how an operator fixes a grant revoked at the vendor.
        // It is not a decision about who may use it, and one that quietly
        // widened the plugin would undo the narrowing at the moment the
        // operator was fixing something else.
        let f = fixture();
        let group = default_group_id();
        let revenue = f.store.create_agent(&draft("Revenue")).unwrap();
        let scribe = f.store.create_agent(&draft("Scribe")).unwrap();
        let plugin = f
            .store
            .save_plugin(group, PluginKind::Stripe, "", &[tool("refund")], None, "")
            .unwrap();
        f.store
            .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![revenue.id] })
            .unwrap();

        let again = f
            .store
            .save_plugin(group, PluginKind::Stripe, "", &[tool("refund")], None, "")
            .unwrap();

        assert_eq!(again.access, PluginAccess::Chosen { agents: vec![revenue.id] });
        assert!(f.store.plugin_tools(group, scribe.id).unwrap().is_empty());
    }

    #[test]
    fn retiring_an_agent_takes_its_place_on_a_plugin_with_it() {
        let f = fixture();
        let group = default_group_id();
        let revenue = f.store.create_agent(&draft("Revenue")).unwrap();
        let plugin = f.store.save_plugin(group, PluginKind::Stripe, "", &[], None, "").unwrap();
        f.store
            .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![revenue.id] })
            .unwrap();

        assert_eq!(f.store.delete_agent_plugin_access(revenue.id).unwrap(), 1);
        assert_eq!(
            f.store.group_plugins(group).unwrap()[0].access,
            PluginAccess::Chosen { agents: vec![] },
            "the plugin stays narrowed; it just names nobody"
        );
    }

    #[test]
    fn disconnecting_a_narrowed_plugin_leaves_no_permission_behind() {
        // The foreign key would refuse the delete, and a row naming a plugin
        // that is gone is a standing permission attached to nothing.
        let f = fixture();
        let group = default_group_id();
        let revenue = f.store.create_agent(&draft("Revenue")).unwrap();
        let plugin = f.store.save_plugin(group, PluginKind::Stripe, "", &[], None, "").unwrap();
        f.store
            .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![revenue.id] })
            .unwrap();

        assert!(f.store.delete_plugin(plugin.id).unwrap());
        let left: i64 = f
            .store
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM plugin_agents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn a_plugin_arrives_with_everything_it_published_switched_on() {
        // The default, and what every plugin connected before this control
        // existed keeps. An operator who never opens the tool list has a crew
        // that works exactly as it did.
        let f = fixture();
        let group = default_group_id();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        f.store
            .save_plugin(
                group,
                PluginKind::Neon,
                "",
                &[tool("run_sql"), tool("drop_project")],
                None,
                "",
            )
            .unwrap();

        let drawn = f.store.group_plugins(group).unwrap().remove(0);
        assert!(
            drawn.tools.iter().all(|tool| tool.access == PluginAccess::Everyone),
            "{:?}",
            drawn.tools
        );

        let offered = f.store.plugin_tools(group, manager.id).unwrap().remove(0);
        assert_eq!(offered.offered.len(), 2);
        assert!(offered.withheld.is_empty());
        assert!(offered.elsewhere.is_empty());
    }

    #[test]
    fn a_switched_off_tool_leaves_the_turn_and_the_call_path_saying_the_same_thing() {
        // The one that matters. A model names tools it was never offered, so
        // the definitions being right is not the enforcement: the call path
        // asks again. The two disagreeing is either a tool an agent is offered
        // and refused, or one it is refused and could have had.
        let f = fixture();
        let group = default_group_id();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let plugin = f
            .store
            .save_plugin(
                group,
                PluginKind::Neon,
                "",
                &[tool("run_sql"), tool("drop_project")],
                None,
                "",
            )
            .unwrap();

        let saved = f.store.set_plugin_tool(plugin.id, "drop_project", &nobody()).unwrap();
        assert_eq!(
            saved.tools.iter().find(|tool| tool.name == "drop_project").map(|tool| &tool.access),
            Some(&nobody()),
            "the panel draws what was stored"
        );

        // Told: one tool, and the other named as switched off rather than
        // silently missing.
        let offered = f.store.plugin_tools(group, manager.id).unwrap().remove(0);
        assert_eq!(
            offered.offered.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["run_sql"]
        );
        assert_eq!(offered.withheld, vec!["drop_project".to_string()]);

        // And got: the same answer, from the other query.
        assert!(matches!(
            f.store.plugin_reach(group, manager.id, PluginKind::Neon, "drop_project").unwrap(),
            PluginReach::ToolDenied
        ));
        assert!(matches!(
            f.store.plugin_reach(group, manager.id, PluginKind::Neon, "run_sql").unwrap(),
            PluginReach::Granted { .. }
        ));
    }

    #[test]
    fn a_tool_switched_off_is_off_for_the_agent_the_plugin_was_narrowed_to() {
        // The two axes are not one axis. Being chosen for a plugin is not being
        // handed every tool on it, and the refusal an agent gets says the thing
        // that is true of the whole crew rather than sending it to a peer who
        // would be refused in turn.
        let f = fixture();
        let group = default_group_id();
        let revenue = f.store.create_agent(&draft("Revenue")).unwrap();
        let scribe = f.store.create_agent(&draft("Scribe")).unwrap();
        let plugin = f
            .store
            .save_plugin(
                group,
                PluginKind::Stripe,
                "",
                &[tool("charges"), tool("refund")],
                None,
                "",
            )
            .unwrap();

        f.store
            .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![revenue.id] })
            .unwrap();
        f.store.set_plugin_tool(plugin.id, "refund", &nobody()).unwrap();

        assert!(matches!(
            f.store.plugin_reach(group, revenue.id, PluginKind::Stripe, "refund").unwrap(),
            PluginReach::ToolDenied
        ));
        assert!(matches!(
            f.store.plugin_reach(group, scribe.id, PluginKind::Stripe, "refund").unwrap(),
            PluginReach::ToolDenied,
        ));
        assert!(matches!(
            f.store.plugin_reach(group, scribe.id, PluginKind::Stripe, "charges").unwrap(),
            PluginReach::NotChosen
        ));
    }

    #[test]
    fn two_agents_on_one_plugin_get_different_tools() {
        // The thing a crew-wide switch could not say, and the reason this axis
        // grew a list. One sign-in, one inbox: the agent that triages it reads
        // and the agent that answers it sends, and neither is offered the
        // other's half.
        let f = fixture();
        let group = default_group_id();
        let triage = f.store.create_agent(&draft("Triage")).unwrap();
        let reply = f.store.create_agent(&draft("Reply")).unwrap();
        let plugin = f
            .store
            .save_plugin(
                group,
                PluginKind::Agentmail,
                "",
                &[tool("read_thread"), tool("send")],
                None,
                "",
            )
            .unwrap();

        f.store
            .set_plugin_tool(
                plugin.id,
                "read_thread",
                &PluginAccess::Chosen { agents: vec![triage.id] },
            )
            .unwrap();
        f.store
            .set_plugin_tool(plugin.id, "send", &PluginAccess::Chosen { agents: vec![reply.id] })
            .unwrap();

        // Told: one each, and the other named as somebody else's rather than
        // as switched off. The two lists are what the prompt says differently.
        let told = f.store.plugin_tools(group, triage.id).unwrap().remove(0);
        assert_eq!(
            told.offered.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["read_thread"]
        );
        assert_eq!(told.elsewhere, vec!["send".to_string()]);
        assert!(told.withheld.is_empty(), "somebody has it, so nobody is the wrong answer");

        // And got: the same answer, from the other query, and the refusal is
        // the one that sends the turn to a peer.
        assert!(matches!(
            f.store.plugin_reach(group, triage.id, PluginKind::Agentmail, "send").unwrap(),
            PluginReach::ToolNotChosen
        ));
        assert!(matches!(
            f.store.plugin_reach(group, reply.id, PluginKind::Agentmail, "send").unwrap(),
            PluginReach::Granted { .. }
        ));
        assert!(matches!(
            f.store.plugin_reach(group, reply.id, PluginKind::Agentmail, "read_thread").unwrap(),
            PluginReach::ToolNotChosen
        ));
    }

    #[test]
    fn a_tool_nobody_has_is_refused_before_one_somebody_else_has() {
        // Both refusals send the turn somewhere, and only one of them is worth
        // going. "Nobody has this" is true of the whole crew and stops the
        // asking; "not yours" spends a peer's turn as well if it is wrong.
        let f = fixture();
        let group = default_group_id();
        let one = f.store.create_agent(&draft("One")).unwrap();
        let two = f.store.create_agent(&draft("Two")).unwrap();
        let plugin = f
            .store
            .save_plugin(
                group,
                PluginKind::Stripe,
                "",
                &[tool("refund"), tool("charges")],
                None,
                "",
            )
            .unwrap();

        f.store.set_plugin_tool(plugin.id, "refund", &nobody()).unwrap();
        f.store
            .set_plugin_tool(plugin.id, "charges", &PluginAccess::Chosen { agents: vec![two.id] })
            .unwrap();

        assert!(matches!(
            f.store.plugin_reach(group, one.id, PluginKind::Stripe, "refund").unwrap(),
            PluginReach::ToolDenied
        ));
        assert!(matches!(
            f.store.plugin_reach(group, one.id, PluginKind::Stripe, "charges").unwrap(),
            PluginReach::ToolNotChosen
        ));

        // And the wider answer wins over the narrower one when both are true:
        // an agent that is off the plugin entirely and asks for a tool nobody
        // has is told the thing that is true of everybody.
        f.store
            .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![two.id] })
            .unwrap();
        assert!(matches!(
            f.store.plugin_reach(group, one.id, PluginKind::Stripe, "refund").unwrap(),
            PluginReach::ToolDenied
        ));
        // Off the plugin covers every tool on it, so it is said before one of
        // them is.
        assert!(matches!(
            f.store.plugin_reach(group, one.id, PluginKind::Stripe, "charges").unwrap(),
            PluginReach::NotChosen
        ));
    }

    #[test]
    fn a_tool_narrowed_to_an_agent_the_plugin_does_not_reach_grants_nothing() {
        // The two answers compose rather than override. An operator sets them
        // in either order, so a name on the tool before the plugin is widened
        // is a state to pass through, not one to refuse: it just does not grant
        // anything on its own.
        let f = fixture();
        let group = default_group_id();
        let inside = f.store.create_agent(&draft("Inside")).unwrap();
        let outside = f.store.create_agent(&draft("Outside")).unwrap();
        let plugin =
            f.store.save_plugin(group, PluginKind::Neon, "", &[tool("run_sql")], None, "").unwrap();

        f.store
            .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![inside.id] })
            .unwrap();
        f.store
            .set_plugin_tool(
                plugin.id,
                "run_sql",
                &PluginAccess::Chosen { agents: vec![outside.id] },
            )
            .unwrap();

        assert!(matches!(
            f.store.plugin_reach(group, outside.id, PluginKind::Neon, "run_sql").unwrap(),
            PluginReach::NotChosen
        ));
        assert!(matches!(
            f.store.plugin_reach(group, inside.id, PluginKind::Neon, "run_sql").unwrap(),
            PluginReach::ToolNotChosen
        ));
        assert!(f.store.plugin_tools(group, inside.id).unwrap()[0].offered.is_empty());
    }

    #[test]
    fn a_tool_may_not_be_narrowed_to_an_agent_from_another_crew() {
        // The boundary `set_plugin_access` draws, one level down. A name from
        // another group grants nothing here either, because the turn asks by
        // group as well, so storing one is a row that means nothing and a
        // settings panel drawing a name that does nothing.
        let f = fixture();
        let elsewhere = f
            .store
            .create_group(&CleanGroup { name: "Elsewhere".into(), ..Default::default() })
            .unwrap();
        let stranger = f
            .store
            .create_agent(&CleanDraft { group_id: Some(elsewhere.id), ..draft("Stranger") })
            .unwrap();
        let plugin = f
            .store
            .save_plugin(default_group_id(), PluginKind::Neon, "", &[tool("run_sql")], None, "")
            .unwrap();

        let refused = f
            .store
            .set_plugin_tool(
                plugin.id,
                "run_sql",
                &PluginAccess::Chosen { agents: vec![stranger.id] },
            )
            .unwrap_err();
        assert!(matches!(refused, StoreError::AgentNotInGroup(id) if id == stranger.id));
        // And nothing was written on the way to refusing: a narrowing that
        // survived a rejected write is a tool switched off by an error.
        let left: i64 = f
            .store
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM plugin_tool_access", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn retiring_an_agent_takes_the_tools_it_was_named_on() {
        // A tool naming a retired agent and nobody else draws as narrowed to
        // somebody and is callable by nobody, and the refusal an agent gets for
        // it points at a peer that is gone.
        let f = fixture();
        let group = default_group_id();
        let gone = f.store.create_agent(&draft("Gone")).unwrap();
        let plugin =
            f.store.save_plugin(group, PluginKind::Neon, "", &[tool("run_sql")], None, "").unwrap();
        f.store
            .set_plugin_tool(plugin.id, "run_sql", &PluginAccess::Chosen { agents: vec![gone.id] })
            .unwrap();

        f.store.delete_agent_plugin_access(gone.id).unwrap();

        let drawn = f.store.group_plugins(group).unwrap().remove(0);
        assert_eq!(drawn.tools[0].access, nobody(), "the narrowing stays, the name does not");
        let left: i64 = f
            .store
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM plugin_tool_agents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn switching_a_tool_back_on_leaves_nothing_behind_that_still_refuses_it() {
        // The stored state has to be what is true. A remembered refusal is a
        // decision the operator has already changed, waiting to come back the
        // next time anything reads the table.
        let f = fixture();
        let group = default_group_id();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let plugin =
            f.store.save_plugin(group, PluginKind::Neon, "", &[tool("run_sql")], None, "").unwrap();

        f.store
            .set_plugin_tool(
                plugin.id,
                "run_sql",
                &PluginAccess::Chosen { agents: vec![manager.id] },
            )
            .unwrap();
        let back = f.store.set_plugin_tool(plugin.id, "run_sql", &PluginAccess::Everyone).unwrap();

        assert_eq!(back.tools[0].access, PluginAccess::Everyone);
        assert!(f.store.plugin_tools(group, manager.id).unwrap()[0].withheld.is_empty());
        // Both tables, because everyone is stored as the absence of a row: a
        // narrowing kept beside a stored `everyone` is two ways to say one
        // thing, and the next reader picks one of them.
        let conn = f.store.conn().unwrap();
        for table in ["plugin_tool_access", "plugin_tool_agents"] {
            let left: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0))
                .unwrap();
            assert_eq!(left, 0, "{table}");
        }
    }

    #[test]
    fn a_tool_the_server_never_published_cannot_be_switched_off() {
        // A refusal filed against a name nothing publishes is a row no panel
        // can show and no call can hit. It means the list on screen is older
        // than the one the server last sent, which is what the error says.
        let f = fixture();
        let plugin = f
            .store
            .save_plugin(default_group_id(), PluginKind::Neon, "", &[tool("run_sql")], None, "")
            .unwrap();

        let refused = f.store.set_plugin_tool(plugin.id, "drop_project", &nobody()).unwrap_err();
        assert!(
            matches!(&refused, StoreError::PluginToolNotFound(id, name)
                if *id == plugin.id && name == "drop_project"),
            "{refused}"
        );
        assert!(refused.to_string().contains("connect it again"), "{refused}");
    }

    #[test]
    fn connecting_again_does_not_switch_a_denied_tool_back_on() {
        // Reconnecting is how an operator fixes a grant revoked at the vendor,
        // exactly as it is for who may use the plugin. One that quietly handed
        // `drop_project` back would undo the decision at the moment the
        // operator was fixing something else.
        let f = fixture();
        let group = default_group_id();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let published = [tool("run_sql"), tool("drop_project")];
        let plugin =
            f.store.save_plugin(group, PluginKind::Neon, "", &published, None, "").unwrap();
        f.store.set_plugin_tool(plugin.id, "drop_project", &nobody()).unwrap();

        // And a tool the vendor started publishing since arrives switched on,
        // which is the half an allow-list could not express: nobody has made a
        // decision about it, and the decision that was made was about the
        // plugin.
        let again = f
            .store
            .save_plugin(
                group,
                PluginKind::Neon,
                "",
                &[tool("run_sql"), tool("drop_project"), tool("list_branches")],
                None,
                "",
            )
            .unwrap();

        let off: Vec<&str> = again
            .tools
            .iter()
            .filter(|t| t.access != PluginAccess::Everyone)
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(off, ["drop_project"]);
        assert_eq!(
            f.store.plugin_tools(group, manager.id).unwrap()[0].offered.len(),
            2,
            "the new tool is offered and the refused one is not"
        );
    }

    #[test]
    fn disconnecting_a_plugin_takes_the_tools_it_had_switched_off_with_it() {
        // A refusal naming a plugin that is gone is a decision attached to
        // nothing, waiting to attach itself to whatever takes the id next.
        let f = fixture();
        let group = default_group_id();
        let plugin =
            f.store.save_plugin(group, PluginKind::Neon, "", &[tool("run_sql")], None, "").unwrap();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        f.store
            .set_plugin_tool(
                plugin.id,
                "run_sql",
                &PluginAccess::Chosen { agents: vec![manager.id] },
            )
            .unwrap();

        assert!(f.store.delete_plugin(plugin.id).unwrap());
        let conn = f.store.conn().unwrap();
        for table in ["plugin_tool_access", "plugin_tool_agents"] {
            let left: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0))
                .unwrap();
            assert_eq!(left, 0, "{table}");
        }
    }

    #[test]
    fn deleting_a_group_takes_what_its_plugins_had_switched_off_too() {
        let f = fixture();
        let group = f
            .store
            .create_group(&CleanGroup { name: "Crew".into(), ..Default::default() })
            .unwrap();
        let plugin = f
            .store
            .save_plugin(group.id, PluginKind::Neon, "", &[tool("run_sql")], None, "")
            .unwrap();
        let hand = f
            .store
            .create_agent(&CleanDraft { group_id: Some(group.id), ..draft("Hand") })
            .unwrap();
        f.store
            .set_plugin_tool(plugin.id, "run_sql", &PluginAccess::Chosen { agents: vec![hand.id] })
            .unwrap();
        // A group is only deleted once it is empty, so the crew is gone by the
        // time this runs. The rows naming them must not outlive either.
        f.store.set_lifecycle(hand.id, Lifecycle::Terminated).unwrap();

        f.store.delete_group(group.id).unwrap();
        let conn = f.store.conn().unwrap();
        for table in ["plugin_tool_access", "plugin_tool_agents"] {
            let left: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0))
                .unwrap();
            assert_eq!(left, 0, "{table}");
        }
    }

    #[test]
    fn an_access_value_this_build_does_not_know_shuts_the_plugin_rather_than_opening_it() {
        // What a downgrade leaves behind. A permission that cannot be read has
        // to fail closed: the crew losing a plugin is visible and fixable, and
        // a crew silently gaining one is neither.
        let f = fixture();
        let group = default_group_id();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let plugin =
            f.store.save_plugin(group, PluginKind::Neon, "", &[tool("run_sql")], None, "").unwrap();
        f.store
            .conn()
            .unwrap()
            .execute(
                "UPDATE plugins SET access='whoever' WHERE id=?1",
                params![plugin.id.to_string()],
            )
            .unwrap();

        assert!(f.store.plugin_tools(group, manager.id).unwrap().is_empty());
        assert!(matches!(
            f.store.plugin_reach(group, manager.id, PluginKind::Neon, "run_sql").unwrap(),
            PluginReach::NotChosen
        ));
        assert_eq!(
            f.store.group_plugins(group).unwrap()[0].access,
            PluginAccess::Chosen { agents: vec![] }
        );
    }

    // ---- repositories ----------------------------------------------------

    #[test]
    fn a_linked_repository_reaches_nobody_until_somebody_is_named() {
        // The whole grant model in one assertion. Linking is not handing out,
        // and there is no value of `reach` that means everybody: an agent that
        // has not been named is refused, and so is one hired after the fact.
        let f = fixture();
        let group = f.store.create_group(&group_named("Platform")).unwrap();
        let repo = f.store.create_repository(&repo_at(group.id, "/dev/guaca")).unwrap();

        assert!(repo.reach.is_empty());

        let later = f
            .store
            .create_agent(&CleanDraft { group_id: Some(group.id), ..draft("Hired Later") })
            .unwrap();
        assert!(!repo.reaches(later.id));
        assert!(f.store.agent_repositories(later.id).unwrap().is_empty());
    }

    #[test]
    fn one_agent_is_named_at_a_time_and_the_others_are_untouched() {
        let f = fixture();
        let group = f.store.create_group(&group_named("Platform")).unwrap();
        let ada =
            f.store.create_agent(&CleanDraft { group_id: Some(group.id), ..draft("Ada") }).unwrap();
        let grace = f
            .store
            .create_agent(&CleanDraft { group_id: Some(group.id), ..draft("Grace") })
            .unwrap();
        let repo = f.store.create_repository(&repo_at(group.id, "/dev/guaca")).unwrap();

        let after = f.store.set_repository_access(repo.id, ada.id, true).unwrap();
        assert_eq!(after.reach, vec![ada.id]);
        assert_eq!(f.store.agent_repositories(ada.id).unwrap().len(), 1);
        assert!(f.store.agent_repositories(grace.id).unwrap().is_empty());

        let back = f.store.set_repository_access(repo.id, ada.id, false).unwrap();
        assert!(back.reach.is_empty());
        assert!(f.store.agent_repositories(ada.id).unwrap().is_empty());
    }

    #[test]
    fn naming_the_same_agent_twice_is_not_two_grants() {
        // The panel sends a change rather than a state, and a double click is a
        // change sent twice. A second row would make revoking take two clicks,
        // one of which does nothing visible.
        let f = fixture();
        let group = f.store.create_group(&group_named("Platform")).unwrap();
        let ada =
            f.store.create_agent(&CleanDraft { group_id: Some(group.id), ..draft("Ada") }).unwrap();
        let repo = f.store.create_repository(&repo_at(group.id, "/dev/guaca")).unwrap();

        f.store.set_repository_access(repo.id, ada.id, true).unwrap();
        let twice = f.store.set_repository_access(repo.id, ada.id, true).unwrap();
        assert_eq!(twice.reach, vec![ada.id]);
    }

    #[test]
    fn an_agent_in_another_crew_is_refused_rather_than_stored() {
        // A row that every read filters out is a name the panel draws as
        // granted and the runtime treats as absent. Refusing is what keeps the
        // two from ever disagreeing.
        let f = fixture();
        let mine = f.store.create_group(&group_named("Platform")).unwrap();
        let theirs = f.store.create_group(&group_named("Growth")).unwrap();
        let outsider = f
            .store
            .create_agent(&CleanDraft { group_id: Some(theirs.id), ..draft("Outsider") })
            .unwrap();
        let repo = f.store.create_repository(&repo_at(mine.id, "/dev/guaca")).unwrap();

        let refused = f.store.set_repository_access(repo.id, outsider.id, false);
        assert!(refused.is_ok(), "taking one back must always work, even for a stranger");

        match f.store.set_repository_access(repo.id, outsider.id, true) {
            Err(StoreError::AgentNotInGroupForRepository(id)) => assert_eq!(id, outsider.id),
            other => panic!("expected a refusal naming the agent, got {other:?}"),
        }
    }

    #[test]
    fn moving_an_agent_out_of_the_crew_takes_the_repository_with_it() {
        // Reach is the crew's decision about the crew's source. A row that
        // survived a move would be one crew's working tree open to another
        // crew's agent, which is what group scoping exists to stop.
        let f = fixture();
        let mine = f.store.create_group(&group_named("Platform")).unwrap();
        let theirs = f.store.create_group(&group_named("Growth")).unwrap();
        let ada =
            f.store.create_agent(&CleanDraft { group_id: Some(mine.id), ..draft("Ada") }).unwrap();
        let repo = f.store.create_repository(&repo_at(mine.id, "/dev/guaca")).unwrap();
        f.store.set_repository_access(repo.id, ada.id, true).unwrap();
        assert_eq!(f.store.agent_repositories(ada.id).unwrap().len(), 1);

        f.store.move_agent(ada.id, theirs.id, None).unwrap();
        assert!(
            f.store.agent_repositories(ada.id).unwrap().is_empty(),
            "a moved agent must not keep its old crew's source"
        );
    }

    #[test]
    fn one_directory_cannot_be_linked_to_a_crew_twice() {
        let f = fixture();
        let group = f.store.create_group(&group_named("Platform")).unwrap();
        f.store.create_repository(&repo_at(group.id, "/dev/guaca")).unwrap();

        match f.store.create_repository(&repo_at(group.id, "/dev/guaca")) {
            Err(StoreError::DuplicateRepository(path)) => assert_eq!(path, "/dev/guaca"),
            other => panic!("expected the path to be named back, got {other:?}"),
        }
    }

    #[test]
    fn two_crews_can_work_in_the_same_directory() {
        // The index is per group on purpose. Two crews on one codebase is an
        // ordinary shape, and each one's reach is its own.
        let f = fixture();
        let mine = f.store.create_group(&group_named("Platform")).unwrap();
        let theirs = f.store.create_group(&group_named("Growth")).unwrap();

        f.store.create_repository(&repo_at(mine.id, "/dev/guaca")).unwrap();
        f.store.create_repository(&repo_at(theirs.id, "/dev/guaca")).unwrap();

        assert_eq!(f.store.group_repositories(mine.id).unwrap().len(), 1);
        assert_eq!(f.store.group_repositories(theirs.id).unwrap().len(), 1);
    }

    #[test]
    fn a_repository_nobody_has_still_appears_on_the_panel() {
        // An inner join would drop exactly the row the operator is in the
        // middle of fixing, and a panel that hides it reads as a link that
        // failed.
        let f = fixture();
        let group = f.store.create_group(&group_named("Platform")).unwrap();
        f.store.create_repository(&repo_at(group.id, "/dev/nobodys")).unwrap();

        let listed = f.store.group_repositories(group.id).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].reach.is_empty());
    }

    #[test]
    fn renaming_one_leaves_its_path_and_its_reach_alone() {
        let f = fixture();
        let group = f.store.create_group(&group_named("Platform")).unwrap();
        let ada =
            f.store.create_agent(&CleanDraft { group_id: Some(group.id), ..draft("Ada") }).unwrap();
        let repo = f.store.create_repository(&repo_at(group.id, "/dev/guaca")).unwrap();
        f.store.set_repository_access(repo.id, ada.id, true).unwrap();

        let renamed = f.store.update_repository(repo.id, "the app", "run ./scripts/ci.sh").unwrap();
        assert_eq!(renamed.name, "the app");
        assert_eq!(renamed.note, "run ./scripts/ci.sh");
        assert_eq!(renamed.path, "/dev/guaca", "the path is what the row is");
        assert_eq!(renamed.reach, vec![ada.id], "and who has it survives a rename");
    }

    #[test]
    fn retiring_an_agent_takes_its_reach_with_it() {
        // A name is free to reuse the moment an agent is deleted, so a row left
        // behind would hand the operator's source to whoever takes the name.
        let f = fixture();
        let group = f.store.create_group(&group_named("Platform")).unwrap();
        let ada =
            f.store.create_agent(&CleanDraft { group_id: Some(group.id), ..draft("Ada") }).unwrap();
        let repo = f.store.create_repository(&repo_at(group.id, "/dev/guaca")).unwrap();
        f.store.set_repository_access(repo.id, ada.id, true).unwrap();

        assert_eq!(f.store.delete_agent_repository_access(ada.id).unwrap(), 1);
        assert!(f.store.group_repositories(group.id).unwrap()[0].reach.is_empty());
    }

    #[test]
    fn unlinking_takes_every_name_on_it() {
        // The foreign key would refuse the delete otherwise, and a repository
        // that cannot be unlinked is a path the operator is stuck with.
        let f = fixture();
        let group = f.store.create_group(&group_named("Platform")).unwrap();
        let ada =
            f.store.create_agent(&CleanDraft { group_id: Some(group.id), ..draft("Ada") }).unwrap();
        let repo = f.store.create_repository(&repo_at(group.id, "/dev/guaca")).unwrap();
        f.store.set_repository_access(repo.id, ada.id, true).unwrap();

        assert!(f.store.delete_repository(repo.id).unwrap());
        assert!(f.store.group_repositories(group.id).unwrap().is_empty());
        assert!(f.store.agent_repositories(ada.id).unwrap().is_empty());
    }

    #[test]
    fn disbanding_a_crew_unlinks_what_it_was_working_in() {
        let f = fixture();
        let group = f.store.create_group(&group_named("Platform")).unwrap();
        let repo = f.store.create_repository(&repo_at(group.id, "/dev/guaca")).unwrap();

        f.store.delete_group(group.id).unwrap();
        assert!(f.store.get_repository(repo.id).unwrap().is_none());
    }
}
