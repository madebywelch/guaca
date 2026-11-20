use std::collections::{HashMap, HashSet};
use std::path::Path;

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{named_params, params, OptionalExtension, Row, TransactionBehavior};

use crate::db::migrations;
use crate::domain::agent::{AgentCard, CleanDraft, Consent, Lifecycle};
use crate::domain::approval::{
    Approval, ApprovalState, DetailField, ProtectedAction, Request, QUESTION,
};
use crate::domain::connector::{CleanConnector, Connector};
use crate::domain::envelope::{Envelope, Intent, Part, Participant, Trust};
use crate::domain::escalation::{Escalation, Raised};
use crate::domain::group::{CleanGroup, Group, GroupInference, GroupLimits, InferenceOverrides};
use crate::domain::ids::{
    AgentId, ApprovalId, ConnectorId, EscalationId, GroupId, MessageId, OccasionId, PluginId,
    RoutineId, RunId,
};
use crate::domain::now_ms;
use crate::domain::occasion::{Clean as CleanOccasion, Occasion};
use crate::domain::outbox;
use crate::domain::outcome::{Outcome, Settled};
use crate::domain::plugin::{
    Headers, Plugin, PluginAccess, PluginKind, PluginTool, PluginToolCard, PluginToolset,
};
use crate::domain::routine::{
    EventTrigger, NextSlot, Routine, RoutineRun, RunKind, Trigger, UpcomingRun,
};
use crate::domain::search::{
    contains_fold, excerpt, like_pattern, links_in, FileHit, LinkHit, MessageHit, SearchHits,
};
use crate::domain::signin::{Signin, Surface};
use crate::domain::usage::{Tokens, UsageEntry};
use crate::domain::worknote::{Appended, WorkingNote, KEPT};
use crate::runtime::guard::GuardSnapshot;
use crate::secrets::Secret;

pub type Conn = PooledConnection<SqliteConnectionManager>;

#[derive(Debug, Clone, PartialEq)]
pub struct Outstanding {
    pub peer: String,
    pub at: i64,
    pub asked: String,
}

const ASK_GIST: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TidyDue {
    pub agent: AgentId,
    pub hour: u8,
    pub since: i64,
}

pub const TIDY_LINES: u32 = 30;

impl Outstanding {
    pub fn line(&self, now: i64) -> String {
        let mut gist: String = self.asked.split_whitespace().collect::<Vec<_>>().join(" ");
        if gist.chars().count() > ASK_GIST {
            gist = gist.chars().take(ASK_GIST).collect::<String>().trim_end().to_string();
            gist.push('…');
        }

        let waited = (now - self.at).max(0) / 1000;
        let ago = match waited {
            0..=90 => "just now".to_string(),
            91..=5_400 => format!("{} minutes ago", waited / 60),
            5_401..=172_800 => format!("{} hours ago", waited / 3_600),
            _ => format!("{} days ago", waited / 86_400),
        };

        if gist.is_empty() {
            format!("{}, {ago}", self.peer)
        } else {
            format!("{}, {ago}: {gist}", self.peer)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{0}")]
    Decision(String),
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
    #[error("at least one group must remain, so the last group cannot be deleted")]
    CannotDeleteLastGroup,
    #[error("no routine with id {0}")]
    RoutineNotFound(RoutineId),
    #[error("no connector with id {0}")]
    ConnectorNotFound(ConnectorId),
    #[error("no plugin with id {0}")]
    PluginNotFound(PluginId),
    #[error("agent {0} is not in this group, so it cannot be given this group's access")]
    AgentNotInGroup(AgentId),
    #[error(
        "this connector publishes no tool called {1:?}, so there is nothing to allow or deny; the \
         list on screen is older than the one the server last sent, so connect it again"
    )]
    PluginToolNotFound(PluginId, String),
    #[error("{plugin} does not offer a tool called {tool}. Use one of the {plugin} tools in your tool list.")]
    PluginToolNotOffered { plugin: String, tool: String },
    #[error("Guaca could not use its own records: the {plugin} tool list is unreadable. Ask the operator to reconnect {plugin}.")]
    PluginToolListUnreadable { plugin: String },
    #[error("no question with id {0}")]
    ApprovalNotFound(ApprovalId),
    #[error("that question was already answered ({})", state.as_str())]
    ApprovalSettled { state: ApprovalState },
    #[error("{0} is already used by another credential in this group")]
    DuplicateEnvVar(String),
}

fn bootstrap_lock() -> &'static parking_lot::Mutex<()> {
    static LOCK: std::sync::OnceLock<parking_lot::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| parking_lot::Mutex::new(()))
}

#[cfg(test)]
fn default_group_id() -> GroupId {
    migrations::DEFAULT_GROUP_ID.parse().expect("the pinned default group id is a valid uuid")
}

fn fallback_group(
    conn: &rusqlite::Connection,
    excluding: Option<GroupId>,
) -> Result<GroupId, StoreError> {
    let id: Option<String> = conn
        .query_row(
            "SELECT id FROM groups WHERE (?1 IS NULL OR id <> ?1)
         ORDER BY rail_order, created_at, rowid LIMIT 1",
            params![excluding.map(|id| id.to_string())],
            |row| row.get(0),
        )
        .optional()?;
    id.ok_or(StoreError::CannotDeleteLastGroup)?
        .parse()
        .map_err(|e| StoreError::Corrupt(format!("invalid group id: {e}")))
}

fn new_card(draft: &CleanDraft, rail_order: i32, fallback: GroupId) -> AgentCard {
    let now = now_ms();
    AgentCard {
        id: AgentId::new(),
        group_id: draft.group_id.unwrap_or(fallback),
        name: draft.name.clone(),
        avatar: draft.avatar.clone(),
        color: draft.color.clone(),
        model: draft.model.clone(),
        subscription_model: draft.subscription_model.clone().unwrap_or_default(),
        reasoning_effort: draft.reasoning_effort,
        system_prompt: draft.system_prompt.clone(),
        skills: draft.skills.clone(),
        sandbox_id: None,
        sandbox_envd_token: None,
        sandbox_traffic_token: None,
        browser_id: None,
        has_computer: false,
        has_browser: false,
        browser_consent: Consent::default(),
        runs_errands: false,
        lifecycle: Lifecycle::Active,
        pinned: false,
        rail_order,
        version: 1,
        created_at: now,
        updated_at: now,
        discarded_at: None,
    }
}

fn bottom_of_rail(conn: &rusqlite::Connection) -> Result<i32, StoreError> {
    let last: i32 =
        conn.query_row("SELECT coalesce(max(rail_order), -1) FROM agents", [], |row| row.get(0))?;
    Ok(last.saturating_add(1))
}

fn insert_agent(conn: &rusqlite::Connection, card: &AgentCard) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id,rail_order,reasoning_effort,subscription_model)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
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
            card.reasoning_effort.map(|e| e.as_str()),
            card.subscription_model,
        ],
    )
    .map_err(|e| classify(e, &card.name))?;
    Ok(())
}

fn classify(err: rusqlite::Error, name: &str) -> StoreError {
    if let rusqlite::Error::SqliteFailure(inner, _) = &err {
        if inner.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE {
            return StoreError::DuplicateName(name.to_string());
        }
    }
    StoreError::Sqlite(err)
}

const SEARCH_SCAN: u32 = 400;

const EXCERPT_CHARS: usize = 160;

#[derive(Debug, Clone, Copy)]
enum AgentFlag {
    HasComputer,
    HasBrowser,
    RunsErrands,
}

#[derive(Debug, Clone)]
pub struct Store {
    pool: Pool<SqliteConnectionManager>,
    outbox: std::sync::Arc<tokio::sync::Notify>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceState {
    Created,
    Adopted,
    Released,
    Gone,
}

#[derive(Debug, Clone, Copy)]
pub enum ResourceEvent {
    Release,
    ConfirmGone,
}

impl ResourceState {
    pub fn transition(self, event: ResourceEvent) -> Option<Self> {
        match (self, event) {
            (Self::Created, ResourceEvent::Release) => Some(Self::Released),
            (Self::Created | Self::Adopted | Self::Released, ResourceEvent::ConfirmGone) => {
                Some(Self::Gone)
            }
            _ => None,
        }
    }
}

impl Store {
    pub fn workspace_identity(&self) -> Result<String, StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO workspace_identity (singleton, id) VALUES (1, ?1)",
            [uuid::Uuid::new_v4().to_string()],
        )?;
        Ok(conn.query_row("SELECT id FROM workspace_identity WHERE singleton=1", [], |row| {
            row.get(0)
        })?)
    }

    pub fn resource_state(
        &self,
        provider: &str,
        resource: &str,
    ) -> Result<Option<ResourceState>, StoreError> {
        let conn = self.conn()?;
        let row: Option<(String, Option<i64>, Option<i64>)> = conn.query_row(
            "SELECT origin, released_at, gone_at FROM resource_ledger WHERE provider=?1 AND resource_id=?2",
            params![provider, resource], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        row.map(|(origin, released, gone)| match (origin.as_str(), released, gone) {
            (_, _, Some(_)) => Ok(ResourceState::Gone),
            ("created", Some(_), None) => Ok(ResourceState::Released),
            ("created", None, None) => Ok(ResourceState::Created),
            ("adopted", None, None) => Ok(ResourceState::Adopted),
            _ => Err(StoreError::Corrupt("invalid resource ledger state".into())),
        })
        .transpose()
    }

    pub fn record_resource(
        &self,
        provider: &str,
        resource: &str,
        adopted: bool,
    ) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO resource_ledger (provider, resource_id, origin, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![provider, resource, if adopted { "adopted" } else { "created" }, now_ms()],
        )?;
        Ok(())
    }

    pub fn record_created_profile(
        &self,
        profile: &str,
        provider_id: Option<&str>,
    ) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute("INSERT INTO resource_ledger (provider, resource_id, origin, provider_id, created_at) VALUES ('kernel-profile', ?1, 'created', ?2, ?3) ON CONFLICT(provider, resource_id) DO UPDATE SET origin='created', provider_id=excluded.provider_id, created_at=excluded.created_at, released_at=NULL, gone_at=NULL", params![profile, provider_id, now_ms()])?;
        Ok(())
    }

    pub fn created_profile_id(&self, profile: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn()?;
        Ok(conn.query_row("SELECT provider_id FROM resource_ledger WHERE provider='kernel-profile' AND resource_id=?1 AND origin='created' AND gone_at IS NULL", [profile], |row| row.get::<_, Option<String>>(0)).optional()?.flatten())
    }

    pub fn record_adopted_profile(
        &self,
        profile: &str,
        provider_id: Option<&str>,
    ) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO resource_ledger (provider, resource_id, origin, provider_id, created_at) VALUES ('kernel-profile', ?1, 'adopted', ?2, ?3) ON CONFLICT(provider, resource_id) DO UPDATE SET origin=CASE WHEN resource_ledger.origin='created' AND resource_ledger.gone_at IS NULL AND resource_ledger.provider_id IS NOT NULL AND resource_ledger.provider_id=excluded.provider_id THEN 'created' ELSE 'adopted' END, provider_id=excluded.provider_id, released_at=NULL, gone_at=NULL",
            params![profile, provider_id, now_ms()],
        )?;
        Ok(())
    }

    pub fn resource_snapshot(
        &self,
        provider: &str,
    ) -> Result<HashMap<String, ResourceState>, StoreError> {
        let conn = self.conn()?;
        let mut statement = conn.prepare("SELECT resource_id, origin, released_at, gone_at FROM resource_ledger WHERE provider=?1")?;
        let rows = statement.query_map([provider], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })?;
        let mut result = HashMap::new();
        for row in rows {
            let (id, origin, released, gone) = row?;
            let state = match (origin.as_str(), released, gone) {
                (_, _, Some(_)) => ResourceState::Gone,
                ("created", Some(_), None) => ResourceState::Released,
                ("created", None, None) => ResourceState::Created,
                ("adopted", None, None) => ResourceState::Adopted,
                _ => return Err(StoreError::Corrupt("invalid resource ledger state".into())),
            };
            result.insert(id, state);
        }
        Ok(result)
    }

    pub fn mark_resource_gone(&self, provider: &str, resource: &str) -> Result<(), StoreError> {
        if !self
            .resource_state(provider, resource)?
            .is_some_and(|state| state.transition(ResourceEvent::ConfirmGone).is_some())
        {
            return Ok(());
        }
        let conn = self.conn()?;
        conn.execute(
            "UPDATE resource_ledger SET released_at=coalesce(released_at, ?3), gone_at=coalesce(gone_at, ?3) WHERE provider=?1 AND resource_id=?2",
            params![provider, resource, now_ms()],
        )?;
        Ok(())
    }

    pub fn mark_resource_released(&self, provider: &str, resource: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute("UPDATE resource_ledger SET released_at=coalesce(released_at, ?3) WHERE provider=?1 AND resource_id=?2 AND origin='created' AND gone_at IS NULL", params![provider, resource, now_ms()])?;
        if changed == 0 {
            return Err(StoreError::Corrupt(
                "resource is not owned or has already been released".into(),
            ));
        }
        Ok(())
    }

    pub fn record_created_sandbox(
        &self,
        id: AgentId,
        sandbox: (&str, &str, &str),
    ) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO resource_ledger (provider, resource_id, origin, created_at) VALUES ('e2b', ?1, 'created', ?2)",
            params![sandbox.0, now_ms()],
        )?;
        let changed = tx.execute(
            "UPDATE agents SET sandbox_id=?2, sandbox_envd_token=?3, sandbox_traffic_token=?4 WHERE id=?1",
            params![id.to_string(), sandbox.0, sandbox.1, sandbox.2],
        )?;
        if changed == 0 {
            return Err(StoreError::AgentNotFound(id));
        }
        tx.commit()?;
        Ok(())
    }

    pub fn sandbox_resume_pending(&self, sandbox: &str) -> Result<bool, StoreError> {
        let conn = self.conn()?;
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pending_sandbox_resume WHERE resource_id=?1)",
            [sandbox],
            |row| row.get(0),
        )?)
    }

    pub fn begin_sandbox_resume(&self, sandbox: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO pending_sandbox_resume(resource_id) VALUES (?1)",
            [sandbox],
        )?;
        Ok(())
    }

    pub fn finish_sandbox_resume(
        &self,
        id: AgentId,
        sandbox: (&str, &str, &str),
    ) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let changed = tx.execute("UPDATE agents SET sandbox_id=?2, sandbox_envd_token=?3, sandbox_traffic_token=?4 WHERE id=?1", params![id.to_string(), sandbox.0, sandbox.1, sandbox.2])?;
        if changed == 0 {
            return Err(StoreError::AgentNotFound(id));
        }
        tx.execute("DELETE FROM pending_sandbox_resume WHERE resource_id=?1", [sandbox.0])?;
        tx.commit()?;
        Ok(())
    }

    pub fn clear_sandbox_resume(&self, sandbox: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM pending_sandbox_resume WHERE resource_id=?1", [sandbox])?;
        Ok(())
    }

    pub fn record_browser(
        &self,
        id: AgentId,
        browser: &str,
        adopted: bool,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO resource_ledger (provider, resource_id, origin, created_at) VALUES ('kernel', ?1, ?2, ?3)",
            params![browser, if adopted { "adopted" } else { "created" }, now_ms()],
        )?;
        let changed = tx.execute(
            "UPDATE agents SET browser_id=?2 WHERE id=?1",
            params![id.to_string(), browser],
        )?;
        if changed == 0 {
            return Err(StoreError::AgentNotFound(id));
        }
        tx.commit()?;
        Ok(())
    }

    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        {
            let _serialized = bootstrap_lock().lock();

            let mut bootstrap = rusqlite::Connection::open(path)?;
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

        let manager = SqliteConnectionManager::file(path).with_init(|conn| {
            conn.execute_batch(
                "PRAGMA busy_timeout = 5000;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA foreign_keys = ON;",
            )
        });

        Ok(Self { pool: Pool::builder().max_size(8).build(manager)?, outbox: Default::default() })
    }

    pub fn outbox_written(&self) -> std::sync::Arc<tokio::sync::Notify> {
        self.outbox.clone()
    }

    fn wake_outbox(&self) {
        self.outbox.notify_one();
    }

    pub fn conn(&self) -> Result<Conn, StoreError> {
        Ok(self.pool.get()?)
    }

    pub fn create_agent(&self, draft: &CleanDraft) -> Result<AgentCard, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let card = new_card(draft, bottom_of_rail(&tx)?, fallback_group(&tx, None)?);
        insert_agent(&tx, &card)?;
        tx.commit()?;
        Ok(card)
    }

    pub fn create_onboarding_agent(
        &self,
        key: &str,
        fingerprint: &str,
        draft: &CleanDraft,
    ) -> Result<(AgentCard, bool), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT fingerprint,agent_id FROM onboarding_hires WHERE key=?1",
                [key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((stored, id)) = existing {
            if stored != fingerprint {
                return Err(StoreError::Decision(
                    "This idempotency key was already used for another hire.".into(),
                ));
            }
            let card = tx.query_row(
                &format!("SELECT {AGENT_COLUMNS} FROM agents WHERE id=?1"),
                [id],
                row_to_card,
            )??;
            return Ok((card, false));
        }
        let card = new_card(draft, bottom_of_rail(&tx)?, fallback_group(&tx, None)?);
        insert_agent(&tx, &card)?;
        tx.execute(
            "INSERT INTO onboarding_hires (key,fingerprint,agent_id) VALUES (?1,?2,?3)",
            params![key, fingerprint, card.id.to_string()],
        )?;
        tx.commit()?;
        Ok((card, true))
    }

    pub fn create_agents(&self, drafts: &[CleanDraft]) -> Result<Vec<AgentCard>, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let first = bottom_of_rail(&tx)?;
        let fallback = fallback_group(&tx, None)?;
        let cards: Vec<AgentCard> = drafts
            .iter()
            .enumerate()
            .map(|(offset, draft)| new_card(draft, first.saturating_add(offset as i32), fallback))
            .collect();
        for card in &cards {
            insert_agent(&tx, card)?;
        }
        tx.commit()?;
        Ok(cards)
    }

    pub fn update_agent(&self, id: AgentId, draft: &CleanDraft) -> Result<AgentCard, StoreError> {
        let conn = self.conn()?;
        let now = now_ms();
        let changed = conn
            .execute(
                "UPDATE agents
                    SET name=?2, avatar=?3, color=?4, model=?5, system_prompt=?6, skills=?7,
                        version = version + 1, updated_at=?8,
                        group_id = coalesce(?9, group_id), reasoning_effort=?10,
                        subscription_model = coalesce(?11, subscription_model)
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
                    draft.reasoning_effort.map(|e| e.as_str()),
                    draft.subscription_model,
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

    pub fn discard_agent(&self, id: AgentId, at: i64) -> Result<AgentCard, StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE agents SET lifecycle='terminated', discarded_at=?2, updated_at=?2
              WHERE id=?1 AND lifecycle <> 'terminated'",
            params![id.to_string(), at],
        )?;
        if changed == 0 {
            return Err(StoreError::AgentNotFound(id));
        }
        self.get_agent(id)?.ok_or(StoreError::AgentNotFound(id))
    }

    pub fn restore_agent(&self, id: AgentId, name: &str) -> Result<AgentCard, StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE agents SET lifecycle='paused', discarded_at=NULL, name=?2, updated_at=?3
              WHERE id=?1 AND discarded_at IS NOT NULL",
            params![id.to_string(), name, now_ms()],
        )?;
        if changed == 0 {
            return Err(StoreError::AgentNotFound(id));
        }
        self.get_agent(id)?.ok_or(StoreError::AgentNotFound(id))
    }

    pub fn forget_discard(&self, id: AgentId) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute("UPDATE agents SET discarded_at=NULL WHERE id=?1", params![id.to_string()])?;
        Ok(())
    }

    pub fn discarded_agents(&self) -> Result<Vec<AgentCard>, StoreError> {
        self.discards(None)
    }

    pub fn expired_discards(&self, cutoff: i64) -> Result<Vec<AgentCard>, StoreError> {
        self.discards(Some(cutoff))
    }

    fn discards(&self, cutoff: Option<i64>) -> Result<Vec<AgentCard>, StoreError> {
        let conn = self.conn()?;
        let (clause, order) = match cutoff {
            Some(_) => ("AND discarded_at <= ?1", "discarded_at"),
            None => ("", "discarded_at DESC"),
        };
        let mut stmt = conn.prepare(&format!(
            "SELECT {AGENT_COLUMNS} FROM agents
               WHERE discarded_at IS NOT NULL {clause} ORDER BY {order}"
        ))?;
        let rows = match cutoff {
            Some(at) => stmt.query_map(params![at], row_to_card)?,
            None => stmt.query_map([], row_to_card)?,
        };
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    pub fn get_agent(&self, id: AgentId) -> Result<Option<AgentCard>, StoreError> {
        let conn = self.conn()?;
        conn.query_row(
            &format!("SELECT {AGENT_COLUMNS} FROM agents WHERE id=?1"),
            params![id.to_string()],
            row_to_card,
        )
        .optional()?
        .transpose()
    }

    pub fn list_agents(&self) -> Result<Vec<AgentCard>, StoreError> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare(&format!("SELECT {AGENT_COLUMNS} FROM agents ORDER BY rowid"))?;
        let rows = stmt.query_map([], row_to_card)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

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

    pub fn move_agent(
        &self,
        id: AgentId,
        group: GroupId,
        before: Option<AgentId>,
    ) -> Result<AgentCard, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        {
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

    pub fn clear_agent_sandbox_if(&self, id: AgentId, sandbox: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE agents SET sandbox_id=NULL, sandbox_envd_token=NULL, sandbox_traffic_token=NULL WHERE id=?1 AND sandbox_id=?2",
            params![id.to_string(), sandbox],
        )?;
        Ok(())
    }

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

    pub fn clear_agent_browser_if(&self, id: AgentId, browser: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE agents SET browser_id=NULL WHERE id=?1 AND browser_id=?2",
            params![id.to_string(), browser],
        )?;
        Ok(())
    }

    pub fn set_has_computer(&self, id: AgentId, given: bool) -> Result<(), StoreError> {
        self.set_flag(id, AgentFlag::HasComputer, given)
    }

    pub fn set_has_browser(&self, id: AgentId, given: bool) -> Result<(), StoreError> {
        self.set_flag(id, AgentFlag::HasBrowser, given)
    }

    pub fn set_runs_errands(&self, id: AgentId, given: bool) -> Result<(), StoreError> {
        self.set_flag(id, AgentFlag::RunsErrands, given)
    }

    pub fn set_browser_consent(&self, id: AgentId, consent: Consent) -> Result<(), StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE agents SET browser_consent=?2 WHERE id=?1",
            params![id.to_string(), consent.as_str()],
        )?;
        if changed == 0 {
            return Err(StoreError::AgentNotFound(id));
        }
        Ok(())
    }

    fn set_flag(&self, id: AgentId, flag: AgentFlag, value: bool) -> Result<(), StoreError> {
        let column = match flag {
            AgentFlag::HasComputer => "has_computer",
            AgentFlag::HasBrowser => "has_browser",
            AgentFlag::RunsErrands => "runs_errands",
        };
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

    pub fn upcoming_runs(&self, until: i64) -> Result<Vec<UpcomingRun>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT r.id, r.agent_id, a.group_id, r.name, r.next_run_at
               FROM routines r
               JOIN agents a ON a.id = r.agent_id
              WHERE r.next_run_at <= ?1 AND r.active = 1 AND a.lifecycle <> 'terminated'
              ORDER BY r.next_run_at, r.name, r.id",
        )?;
        let rows = stmt.query_map(params![until], row_to_upcoming_run)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

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

    pub fn event_routines(&self, event: &EventTrigger) -> Result<Vec<Routine>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT r.id,r.agent_id,r.name,r.what,r.fires,r.active,r.next_run_at,r.last_run_at,r.created_at,r.skip_if_working
               FROM routines r
               JOIN agents a ON a.id = r.agent_id
              WHERE r.fires = ?1 AND r.active = 1 AND a.lifecycle = 'active'
              ORDER BY r.created_at",
        )?;
        let rows = stmt.query_map(params![event.as_str()], row_to_routine)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    pub fn paused_event_routines(&self, event: &EventTrigger) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM routines r JOIN agents a ON a.id = r.agent_id
              WHERE r.fires = ?1 AND r.active = 1 AND a.lifecycle = 'paused'",
            params![event.as_str()],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

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

    #[allow(clippy::too_many_arguments)]
    pub fn claim_routine(
        &self,
        routine: &Routine,
        occurrence: &str,
        envelope: Option<&Envelope>,
        kind: RunKind,
        error: Option<&str>,
        now: i64,
        advance: bool,
        missed: u32,
    ) -> Result<bool, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if advance && routine.next_run_at.is_some() {
            let current: Option<i64> = tx
                .query_row(
                    "SELECT next_run_at FROM routines WHERE id=?1 AND active=1",
                    [routine.id.to_string()],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            if current != routine.next_run_at {
                return Ok(false);
            }
        }
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO routine_occurrences (routine_id,occurrence,message_id,state) VALUES (?1,?2,?3,?4)",
            params![routine.id.to_string(), occurrence, envelope.map(|e| e.id.to_string()), if envelope.is_some() { "pending" } else { "done" }],
        )?;
        if inserted == 0 {
            return Ok(false);
        }
        if let Some(envelope) = envelope {
            Self::insert_message(&tx, envelope)?;
            tx.execute(
                "INSERT INTO pending_runs (run_id,message_id) VALUES (?1,?2)",
                params![envelope.run_id.to_string(), envelope.id.to_string()],
            )?;
        }
        tx.execute(
            "INSERT INTO routine_runs (routine_id,run_id,kind,at,error,missed) VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                routine.id.to_string(),
                envelope.map(|e| e.run_id.to_string()),
                kind.as_str(),
                now,
                error,
                missed
            ],
        )?;
        if advance {
            match routine.after_running(now) {
                NextSlot::Due(next) => {
                    tx.execute(
                        "UPDATE routines SET next_run_at=?2,last_run_at=?3 WHERE id=?1",
                        params![routine.id.to_string(), next, now],
                    )?;
                }
                NextSlot::Waiting => {
                    tx.execute(
                        "UPDATE routines SET next_run_at=NULL,last_run_at=?2 WHERE id=?1",
                        params![routine.id.to_string(), now],
                    )?;
                }
                NextSlot::Done => {
                    tx.execute("UPDATE routine_occurrences SET state='completing' WHERE routine_id=?1 AND occurrence=?2", params![routine.id.to_string(), occurrence])?;
                    tx.execute("DELETE FROM routines WHERE id=?1", [routine.id.to_string()])?;
                    tx.execute("UPDATE routine_occurrences SET state=?3 WHERE routine_id=?1 AND occurrence=?2", params![routine.id.to_string(), occurrence, if envelope.is_some() { "pending" } else { "done" }])?;
                }
            }
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn tidy_due(&self) -> Result<Vec<TidyDue>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT a.id, g.tidy_hour, max(coalesce(a.last_tidy_at, 0), g.tidy_since)
               FROM agents a JOIN groups g ON g.id = a.group_id
              WHERE g.tidy_since IS NOT NULL AND a.lifecycle = 'active'",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u8>(1)?, row.get::<_, i64>(2)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (agent, hour, since) = row?;
            let agent = agent
                .parse()
                .map_err(|e| StoreError::Corrupt(format!("bad agent id {agent:?}: {e}")))?;
            out.push(TidyDue { agent, hour, since });
        }
        Ok(out)
    }

    pub fn claim_tidy(
        &self,
        agent: AgentId,
        struck: i64,
        now: i64,
        draw: impl FnOnce(Vec<(i64, String, crate::domain::outcome::Status)>) -> Envelope,
    ) -> Result<Option<Envelope>, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let seen: Option<(String, i64, i64)> = tx
            .query_row(
                "SELECT a.group_id, max(coalesce(a.last_tidy_at, 0), g.tidy_since),
                        max(coalesce(a.tidied_through, 0), g.tidy_since)
                   FROM agents a JOIN groups g ON g.id = a.group_id
                  WHERE a.id = ?1 AND a.lifecycle = 'active' AND g.tidy_since IS NOT NULL",
                [agent.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((group, _, since)) = seen.filter(|(_, seen, _)| *seen < struck) else {
            return Ok(None);
        };
        let mut stmt = tx.prepare(
            "SELECT finished_at, headline, status FROM outcomes
              WHERE agent_id = ?1 AND group_id = ?2 AND finished_at > ?3 AND status <> ?4
              ORDER BY finished_at DESC, run_id DESC
              LIMIT ?5",
        )?;
        let mut lines = Vec::new();
        for row in stmt.query_map(
            params![
                agent.to_string(),
                group,
                since,
                crate::domain::outcome::Status::NothingNew.as_str(),
                TIDY_LINES
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        )? {
            let (at, headline, status) = row?;
            let status = crate::domain::outcome::Status::parse(&status)
                .ok_or_else(|| StoreError::Corrupt(format!("unknown status {status:?}")))?;
            lines.push((at, headline, status));
        }
        drop(stmt);
        tx.execute(
            "UPDATE agents SET last_tidy_at = ?2 WHERE id = ?1",
            params![agent.to_string(), now],
        )?;
        if lines.is_empty() {
            tx.execute(
                "UPDATE agents SET tidied_through = ?2 WHERE id = ?1",
                params![agent.to_string(), now],
            )?;
            tx.commit()?;
            return Ok(None);
        }
        let envelope = draw(lines);
        Self::insert_message(&tx, &envelope)?;
        tx.execute(
            "INSERT INTO pending_runs (run_id,message_id) VALUES (?1,?2)",
            params![envelope.run_id.to_string(), envelope.id.to_string()],
        )?;
        tx.commit()?;
        Ok(Some(envelope))
    }

    pub fn tidied(&self, agent: AgentId) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE agents SET tidied_through = last_tidy_at WHERE id = ?1",
            [agent.to_string()],
        )?;
        Ok(())
    }

    pub fn start_deliveries(&self, messages: &[MessageId], owner: RunId) -> Result<(), StoreError> {
        if messages.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let now = now_ms();
        for message in messages {
            tx.execute(
                "UPDATE pending_runs SET started_at=?2 WHERE started_at IS NULL AND run_id=(SELECT run_id FROM messages WHERE id=?1)",
                params![message.to_string(), now],
            )?;
            tx.execute(
                "UPDATE routine_occurrences SET state='started',owner_run=?2 WHERE message_id=?1 AND state IN ('pending','started')",
                params![message.to_string(), owner.to_string()],
            )?;
            tx.execute(
                "UPDATE routine_runs SET run_id=?2 WHERE run_id=(SELECT run_id FROM messages WHERE id=?1) AND EXISTS (SELECT 1 FROM routine_occurrences WHERE message_id=?1 AND state='started')",
                params![message.to_string(), owner.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn fail_routine_delivery(&self, run: RunId) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE routine_runs SET kind='failed',error='The agent could not receive this firing' WHERE run_id=?1 AND EXISTS (SELECT 1 FROM routine_occurrences o JOIN messages m ON m.id=o.message_id WHERE m.run_id=?1 AND o.state='pending')",
            [run.to_string()],
        )?;
        tx.execute(
            "UPDATE routine_occurrences SET state='done' WHERE state='pending' AND message_id IN (SELECT id FROM messages WHERE run_id=?1)",
            [run.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

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

    pub fn routine_runs(&self, id: RoutineId, limit: usize) -> Result<Vec<RoutineRun>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT r.run_id, r.kind, r.at,
                    COALESCE(SUM(u.prompt),0), COALESCE(SUM(u.completion),0),
                    SUM(u.cost), COUNT(u.id), r.error, r.missed
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
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let deleted = tx.execute("DELETE FROM routines WHERE id=?1", params![id.to_string()])? > 0;
        if deleted {
            tx.execute("DELETE FROM routine_runs WHERE routine_id=?1", params![id.to_string()])?;
        }
        tx.commit()?;
        Ok(deleted)
    }

    pub fn delete_agent_routines(&self, agent: AgentId) -> Result<usize, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM routine_runs WHERE routine_id IN (SELECT id FROM routines WHERE agent_id=?1)", [agent.to_string()])?;
        let deleted = tx.execute("DELETE FROM routines WHERE agent_id=?1", [agent.to_string()])?;
        tx.commit()?;
        Ok(deleted)
    }

    pub fn append_working_note(
        &self,
        agent: AgentId,
        body: &str,
        at: i64,
    ) -> Result<Appended, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let held: Option<i64> = tx
            .query_row(
                "SELECT at FROM working_notes WHERE agent_id=?1 AND body=?2 ORDER BY id DESC \
                 LIMIT 1",
                params![agent.to_string(), body],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(at) = held {
            return Ok(Appended::AlreadyHeld { at });
        }
        tx.execute(
            "INSERT INTO working_notes (agent_id,at,body) VALUES (?1,?2,?3)",
            params![agent.to_string(), at, body],
        )?;
        tx.execute(
            "DELETE FROM working_notes
              WHERE agent_id=?1
                AND id NOT IN (
                    SELECT id FROM working_notes WHERE agent_id=?1 ORDER BY id DESC LIMIT ?2
                )",
            params![agent.to_string(), KEPT as i64],
        )?;
        tx.commit()?;
        Ok(Appended::Stored)
    }

    pub fn working_notes(&self, agent: AgentId) -> Result<Vec<WorkingNote>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT at, body FROM working_notes WHERE agent_id=?1 ORDER BY id ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![agent.to_string(), KEPT as i64], |row| {
            Ok(WorkingNote { at: row.get(0)?, body: row.get(1)? })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn clear_working_notes(&self, agent: AgentId) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        Ok(conn
            .execute("DELETE FROM working_notes WHERE agent_id=?1", params![agent.to_string()])?)
    }

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
        let mut conn = self.conn()?;
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
            lapses_at: None,
        };

        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
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
        let name = super::outbox::name_of(&tx, agent)?.unwrap_or_default();
        let question = matches!(approval.request, Request::Question { .. });
        super::outbox::enqueue(
            &tx,
            &outbox::for_request(approval.id, question, agent, group, run, &name),
            approval.created_at,
        )?;
        tx.commit()?;
        self.wake_outbox();

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

    pub fn settle_approval(
        &self,
        id: ApprovalId,
        state: ApprovalState,
    ) -> Result<Approval, StoreError> {
        self.settle_approval_with(id, state, None)
    }

    pub fn answer_approval(&self, id: ApprovalId, answer: &str) -> Result<Approval, StoreError> {
        self.settle_approval_with(id, ApprovalState::Answered, Some(answer))
    }

    fn settle_approval_with(
        &self,
        id: ApprovalId,
        state: ApprovalState,
        answer: Option<&str>,
    ) -> Result<Approval, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE approvals SET state=?2, decided_at=?3, answer=?4
              WHERE id=?1 AND state='pending'",
            params![id.to_string(), state.as_str(), now_ms(), answer],
        )?;
        if changed > 0 {
            super::outbox::withdraw_ref(&tx, &id.to_string())?;
        }
        tx.commit()?;
        if changed == 0 {
            return match self.get_approval(id)? {
                Some(existing) => Err(StoreError::ApprovalSettled { state: existing.state }),
                None => Err(StoreError::ApprovalNotFound(id)),
            };
        }
        self.get_approval(id)?.ok_or(StoreError::ApprovalNotFound(id))
    }

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

    pub fn approval_answers(&self, limit: u32) -> Result<HashMap<ApprovalId, String>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,answer FROM approvals WHERE answer IS NOT NULL
              ORDER BY created_at DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut out = HashMap::new();
        for row in rows {
            let (id, answer) = row?;
            let id = id.parse::<ApprovalId>().map_err(|e| StoreError::Corrupt(e.to_string()))?;
            out.insert(id, answer);
        }
        Ok(out)
    }

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

    pub fn expire_pending_approvals(&self) -> Result<usize, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE outbox SET status='suppressed', last_error=?1
              WHERE status='pending' AND ref_id IN (SELECT id FROM approvals WHERE state='pending')",
            params![outbox::Why::Answered.as_str()],
        )?;
        let expired = tx.execute(
            "UPDATE approvals SET state='expired', decided_at=?1 WHERE state='pending'",
            params![now_ms()],
        )?;
        tx.commit()?;
        Ok(expired)
    }

    pub fn delete_agent_approvals(&self, agent: AgentId) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute("DELETE FROM approvals WHERE agent_id=?1", params![agent.to_string()])?)
    }

    pub fn raise_escalation(
        &self,
        agent: AgentId,
        group: GroupId,
        run: RunId,
        summary: &str,
        at: i64,
    ) -> Result<Raised, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let open: Option<(String, String, i64, u32)> = tx
            .query_row(
                "SELECT id,run_id,raised_at,times FROM escalations
                  WHERE agent_id=?1 AND cleared_at IS NULL",
                params![agent.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;

        if let Some((id, run_id, raised_at, times)) = open {
            let id = id.parse::<EscalationId>().map_err(|e| StoreError::Corrupt(e.to_string()))?;
            let run_id = run_id.parse::<RunId>().map_err(|e| StoreError::Corrupt(e.to_string()))?;
            let times = times.saturating_add(1);
            tx.execute(
                "UPDATE escalations SET summary=?2, said_at=?3, times=?4 WHERE id=?1",
                params![id.to_string(), summary, at, times],
            )?;
            announce_escalation(&tx, id, agent, group, run_id, at)?;
            tx.commit()?;
            self.wake_outbox();
            return Ok(Raised::Again(Escalation {
                id,
                agent_id: agent,
                group_id: group,
                run_id,
                summary: summary.to_string(),
                raised_at,
                said_at: at,
                times,
                cleared_at: None,
            }));
        }

        let escalation = Escalation {
            id: EscalationId::new(),
            agent_id: agent,
            group_id: group,
            run_id: run,
            summary: summary.to_string(),
            raised_at: at,
            said_at: at,
            times: 1,
            cleared_at: None,
        };
        tx.execute(
            "INSERT INTO escalations (id,agent_id,group_id,run_id,summary,raised_at,said_at,times)
             VALUES (?1,?2,?3,?4,?5,?6,?6,1)",
            params![
                escalation.id.to_string(),
                agent.to_string(),
                group.to_string(),
                run.to_string(),
                escalation.summary,
                at,
            ],
        )?;
        announce_escalation(&tx, escalation.id, agent, group, run, at)?;
        tx.commit()?;
        self.wake_outbox();
        Ok(Raised::First(escalation))
    }

    pub fn open_escalation_for(&self, agent: AgentId) -> Result<Option<Escalation>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,agent_id,group_id,run_id,summary,raised_at,said_at,times,cleared_at
               FROM escalations WHERE agent_id=?1 AND cleared_at IS NULL",
        )?;
        match stmt.query_row(params![agent.to_string()], row_to_escalation).optional()? {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn open_escalations(&self, limit: u32) -> Result<Vec<Escalation>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT e.id,e.agent_id,e.group_id,e.run_id,e.summary,e.raised_at,e.said_at,e.times,
                    e.cleared_at
               FROM escalations e JOIN agents a ON a.id = e.agent_id
              WHERE e.cleared_at IS NULL AND a.lifecycle <> 'terminated'
              ORDER BY e.raised_at ASC, e.id ASC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], row_to_escalation)?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    pub fn clear_escalation(&self, id: EscalationId, at: i64) -> Result<bool, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let cleared = tx.execute(
            "UPDATE escalations SET cleared_at=?2 WHERE id=?1 AND cleared_at IS NULL",
            params![id.to_string(), at],
        )? > 0;
        super::outbox::withdraw_ref(&tx, &id.to_string())?;
        tx.commit()?;
        Ok(cleared)
    }

    pub fn clear_agent_escalations(&self, agent: AgentId, at: i64) -> Result<usize, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE outbox SET status='suppressed', last_error=?2
              WHERE status='pending' AND ref_id IN
                (SELECT id FROM escalations WHERE agent_id=?1 AND cleared_at IS NULL)",
            params![agent.to_string(), outbox::Why::Answered.as_str()],
        )?;
        let cleared = tx.execute(
            "UPDATE escalations SET cleared_at=?2 WHERE agent_id=?1 AND cleared_at IS NULL",
            params![agent.to_string(), at],
        )?;
        tx.commit()?;
        Ok(cleared)
    }

    pub fn create_occasion(&self, clean: &CleanOccasion) -> Result<Occasion, StoreError> {
        let conn = self.conn()?;
        let now = now_ms();
        let occasion = Occasion {
            id: OccasionId::new(),
            group_id: clean.group_id(),
            agent_id: clean.agent_id(),
            title: clean.title().to_string(),
            detail: clean.detail().to_string(),
            place: clean.place().to_string(),
            starts_at: clean.starts_at(),
            minutes: clean.minutes(),
            all_day: clean.all_day(),
            created_at: now,
            updated_at: now,
        };

        conn.execute(
            "INSERT INTO occasions
                (id,group_id,agent_id,title,detail,place,starts_at,minutes,all_day,
                 created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)",
            params![
                occasion.id.to_string(),
                occasion.group_id.to_string(),
                occasion.agent_id.map(|id| id.to_string()),
                occasion.title,
                occasion.detail,
                occasion.place,
                occasion.starts_at,
                occasion.minutes,
                occasion.all_day,
                now,
            ],
        )?;
        Ok(occasion)
    }

    pub fn occasion(&self, id: OccasionId, group: GroupId) -> Result<Option<Occasion>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("{OCCASION_COLUMNS} WHERE id=?1 AND group_id=?2"))?;
        match stmt
            .query_row(params![id.to_string(), group.to_string()], row_to_occasion)
            .optional()?
        {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn any_occasion(&self, id: OccasionId) -> Result<Option<Occasion>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("{OCCASION_COLUMNS} WHERE id=?1"))?;
        match stmt.query_row(params![id.to_string()], row_to_occasion).optional()? {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn update_occasion(
        &self,
        id: OccasionId,
        group: GroupId,
        clean: &CleanOccasion,
    ) -> Result<Option<Occasion>, StoreError> {
        let conn = self.conn()?;
        let now = now_ms();
        let changed = conn.execute(
            "UPDATE occasions
                SET title=?3, detail=?4, place=?5, starts_at=?6, minutes=?7, all_day=?8,
                    updated_at=?9
              WHERE id=?1 AND group_id=?2",
            params![
                id.to_string(),
                group.to_string(),
                clean.title(),
                clean.detail(),
                clean.place(),
                clean.starts_at(),
                clean.minutes(),
                clean.all_day(),
                now,
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        self.occasion(id, group)
    }

    pub fn delete_occasion(&self, id: OccasionId, group: GroupId) -> Result<bool, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute(
            "DELETE FROM occasions WHERE id=?1 AND group_id=?2",
            params![id.to_string(), group.to_string()],
        )? > 0)
    }

    pub fn occasions(
        &self,
        from: i64,
        until: i64,
        group: Option<GroupId>,
        limit: u32,
    ) -> Result<Vec<Occasion>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "{OCCASION_COLUMNS}
              WHERE starts_at >= ?1 AND starts_at < ?2
                AND (?3 IS NULL OR group_id = ?3)
              ORDER BY starts_at ASC, id ASC LIMIT ?4"
        ))?;
        let rows = stmt.query_map(
            params![from, until, group.map(|id| id.to_string()), limit],
            row_to_occasion,
        )?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    pub fn crew_calendar(
        &self,
        group: GroupId,
        from: i64,
        limit: u32,
    ) -> Result<Vec<Occasion>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "{OCCASION_COLUMNS}
              WHERE group_id=?1 AND starts_at >= ?2
              ORDER BY starts_at ASC, id ASC LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![group.to_string(), from, limit], row_to_occasion)?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    pub fn create_connector(&self, clean: &CleanConnector) -> Result<Connector, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = now_ms();
        let id = ConnectorId::new();
        tx.execute(
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
                now
            ],
        )
        .map_err(|e| classify_connector(e, &clean.env_var))?;
        write_connector_agents(&tx, id, &clean.agents)?;
        tx.commit()?;
        self.get_connector(id)?.ok_or(StoreError::ConnectorNotFound(id))
    }

    pub fn update_connector(
        &self,
        id: ConnectorId,
        agents: &[AgentId],
        secret: Option<&str>,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        write_connector_agents(&tx, id, agents)?;
        tx.execute(
            "UPDATE connectors SET secret=coalesce(?2,secret), updated_at=?3 WHERE id=?1",
            params![id.to_string(), secret, now_ms()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn agent_connectors(&self, agent: AgentId) -> Result<Vec<Connector>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "{CONNECTOR_COLUMNS} WHERE EXISTS (
            SELECT 1 FROM connector_agents ca JOIN agents a ON a.id=ca.agent_id
            WHERE ca.connector_id=connectors.id AND a.id=?1
              AND a.group_id=connectors.group_id AND a.discarded_at IS NULL)
            ORDER BY service, connectors.rowid"
        ))?;
        let rows = stmt.query_map(params![agent.to_string()], row_to_connector)?;
        rows.map(|row| row?).collect()
    }

    pub fn get_connector(&self, id: ConnectorId) -> Result<Option<Connector>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("{CONNECTOR_COLUMNS} WHERE id=?1"))?;
        match stmt.query_row(params![id.to_string()], row_to_connector).optional()? {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

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

    pub fn connector_env(&self, agent: AgentId) -> Result<crate::secrets::ProcessEnv, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT c.env_var, c.secret FROM connectors c
             JOIN connector_agents ca ON ca.connector_id=c.id
             JOIN agents a ON a.id=ca.agent_id AND a.group_id=c.group_id
             WHERE a.id=?1 AND a.discarded_at IS NULL AND c.secret <> ''",
        )?;
        let rows =
            stmt.query_map(params![agent.to_string()], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let values: std::collections::BTreeMap<String, String> = rows.collect::<Result<_, _>>()?;
        if values.keys().any(|name| crate::domain::connector::reserved(name)) {
            return Err(StoreError::Corrupt("A secret uses a process-control variable. Remove it from the group's Secrets tab and use the service's token variable".into()));
        }
        Ok(crate::secrets::ProcessEnv::new(values))
    }

    pub fn connector_names(&self) -> Result<Vec<String>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT DISTINCT env_var FROM connectors")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let names: Vec<String> = rows.collect::<Result<_, _>>()?;
        Ok(names.into_iter().filter(|name| !crate::domain::connector::reserved(name)).collect())
    }

    pub fn delete_connector(&self, id: ConnectorId) -> Result<bool, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute("DELETE FROM connectors WHERE id=?1", params![id.to_string()])? > 0)
    }

    pub fn outstanding_asks(&self, agent: AgentId) -> Result<Vec<Outstanding>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "WITH asked AS (
                 SELECT to_agent AS peer, max(created_at) AS at
                   FROM messages
                  WHERE from_agent = :me AND to_agent IS NOT NULL AND intent = 'work'
                  GROUP BY to_agent
             ),
             heard AS (
                 SELECT from_agent AS peer, max(created_at) AS at
                   FROM messages
                  WHERE to_agent = :me AND from_agent IS NOT NULL
                  GROUP BY from_agent
             )
             SELECT agents.name, asked.at, (
                 SELECT parts FROM messages
                  WHERE from_agent = :me AND to_agent = asked.peer
                    AND intent = 'work' AND created_at = asked.at
                  LIMIT 1
             )
               FROM asked
               JOIN agents ON agents.id = asked.peer
               LEFT JOIN heard ON heard.peer = asked.peer
              WHERE agents.lifecycle <> 'terminated'
                AND (heard.at IS NULL OR heard.at < asked.at)
              ORDER BY asked.at DESC",
        )?;

        let rows = stmt.query_map(named_params! { ":me": agent.to_string() }, |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, Option<String>>(2)?))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (peer, at, parts) = row?;
            let asked = parts
                .and_then(|raw| serde_json::from_str::<Vec<Part>>(&raw).ok())
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|part| part.as_plain_text())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            out.push(Outstanding { peer, at, asked });
        }
        Ok(out)
    }

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
            if let Some(plugin) = row?? {
                out.push(plugin);
            }
        }
        Ok(out)
    }

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
                continue;
            };
            out.entry(plugin).or_default().push(agent);
        }
        Ok(out)
    }

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
            let Ok(plugin) = plugin.parse::<PluginId>() else { continue };
            let key = (plugin, tool);
            let agents = named.remove(&key).unwrap_or_default();
            out.insert(key, PluginAccess::from_row(&access, agents));
        }
        Ok(out)
    }

    pub fn plugin_tools(
        &self,
        group: GroupId,
        agent: AgentId,
    ) -> Result<Vec<PluginToolset>, StoreError> {
        let conn = self.conn()?;
        let narrowed = self.tool_access(&conn, group)?;
        let mut stmt = conn.prepare(&format!(
            "SELECT id, kind, tools, endpoint FROM plugins
              WHERE group_id=:group AND {PLUGIN_REACHED_BY_AGENT}
              ORDER BY kind, rowid"
        ))?;
        let rows = stmt.query_map(
            named_params! { ":group": group.to_string(), ":agent": agent.to_string() },
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )?;

        let mut out = Vec::new();
        for row in rows {
            let (id, kind, tools, endpoint) = row?;
            let Some(kind) = PluginKind::from_row(&kind, &endpoint) else { continue };
            let tools: Vec<PluginTool> = serde_json::from_str(&tools)
                .map_err(|e| StoreError::Corrupt(format!("bad tool list for {kind:?}: {e}")))?;
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

    pub fn group_plugin_kinds(&self, group: GroupId) -> Result<Vec<PluginKind>, StoreError> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT kind, endpoint FROM plugins WHERE group_id=?1 ORDER BY kind")?;
        let rows = stmt.query_map(params![group.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (slug, endpoint) = row?;
            if let Some(kind) = PluginKind::from_row(&slug, &endpoint) {
                out.push(kind);
            }
        }
        Ok(out)
    }

    pub fn plugin_reach(
        &self,
        group: GroupId,
        agent: AgentId,
        kind: &PluginKind,
        tool: &str,
    ) -> Result<PluginReach, StoreError> {
        let conn = self.conn()?;
        let tx = conn.unchecked_transaction()?;
        let mut stmt = tx.prepare(
            "SELECT id,access_token,refresh_token,expires_at,client_id,client_secret,
                    token_endpoint,tools,headers,grant_generation
               FROM plugins WHERE group_id=:group AND kind=:kind",
        )?;
        let row = stmt
            .query_row(
                named_params! {
                    ":group": group.to_string(),
                    ":kind": kind.slug(),
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
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, i64>(9)?,
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
            tools,
            headers,
            generation,
        )) = row
        else {
            return Ok(PluginReach::NotConnected);
        };
        let tools: Vec<PluginTool> = serde_json::from_str(&tools).map_err(|err| {
            tracing::warn!(%err, plugin = %kind.slug(), "could not read stored plugin tool list");
            StoreError::PluginToolListUnreadable { plugin: kind.label().to_string() }
        })?;
        let Some(held) = tools.into_iter().find(|held| held.name == tool) else {
            return Err(StoreError::PluginToolNotOffered {
                plugin: kind.label().to_string(),
                tool: tool.to_string(),
            });
        };
        let (reached, tool_mine, tool_anyones) = tx.query_row(
            &format!(
                "SELECT {PLUGIN_REACHED_BY_AGENT}, {PLUGIN_TOOL_REACHED_BY_AGENT},
                            {PLUGIN_TOOL_REACHED_BY_ANYONE}
                       FROM plugins WHERE group_id=:group AND kind=:kind"
            ),
            named_params! {
                ":group": group.to_string(), ":kind": kind.slug(),
                ":agent": agent.to_string(), ":tool": tool,
            },
            |row| Ok((row.get::<_, bool>(0)?, row.get::<_, bool>(1)?, row.get::<_, bool>(2)?)),
        )?;
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

        let grant = (!access.is_empty()).then(|| crate::oauth::Grant {
            access_token: access.into(),
            refresh_token: (!refresh.is_empty()).then(|| refresh.into()),
            expires_at,
            client_id,
            client_secret: (!client_secret.is_empty()).then(|| client_secret.into()),
            token_endpoint,
        });

        let schema = Some(held.input_schema);

        Ok(PluginReach::Granted(Box::new(Reached {
            id,
            grant,
            generation,
            headers: Headers::decode(&headers),
            schema,
        })))
    }

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

    #[allow(clippy::too_many_arguments)]
    pub fn save_plugin(
        &self,
        group: GroupId,
        kind: &PluginKind,
        account: &str,
        tools: &[PluginTool],
        grant: Option<&crate::oauth::Grant>,
        connection: &str,
        headers: &Headers,
    ) -> Result<Plugin, StoreError> {
        let conn = self.conn()?;
        let id = PluginId::new();
        let encoded = serde_json::to_string(tools)
            .map_err(|e| StoreError::Corrupt(format!("tool list will not serialize: {e}")))?;

        conn.execute(
            "INSERT INTO plugins
                (id,group_id,kind,account,tools,client_id,client_secret,token_endpoint,
                 access_token,refresh_token,expires_at,connected_at,connection,endpoint,headers)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
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
                connected_at=excluded.connected_at,
                endpoint=excluded.endpoint,
                headers=excluded.headers,
                grant_generation=plugins.grant_generation+1",
            params![
                id.to_string(),
                group.to_string(),
                kind.slug(),
                account,
                encoded,
                grant.map(|g| g.client_id.as_str()).unwrap_or_default(),
                grant
                    .and_then(|g| g.client_secret.as_ref())
                    .map(Secret::expose)
                    .unwrap_or_default(),
                grant.map(|g| g.token_endpoint.as_str()).unwrap_or_default(),
                grant.map(|g| g.access_token.expose()).unwrap_or_default(),
                grant
                    .and_then(|g| g.refresh_token.as_ref())
                    .map(Secret::expose)
                    .unwrap_or_default(),
                grant.and_then(|g| g.expires_at),
                now_ms(),
                connection,
                kind.stored_endpoint(),
                headers.encode(),
            ],
        )?;

        self.group_plugins(group)?
            .into_iter()
            .find(|plugin| plugin.kind.slug() == kind.slug())
            .ok_or(StoreError::PluginNotFound(id))
    }

    pub fn plugin_dial(&self, id: PluginId) -> Result<Option<Dialed>, StoreError> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT kind,endpoint,headers,connection,access_token,refresh_token,expires_at,
                        client_id,client_secret,token_endpoint,grant_generation
                   FROM plugins WHERE id=?1",
                params![id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, i64>(10)?,
                    ))
                },
            )
            .optional()?;

        let Some((
            kind,
            endpoint,
            headers,
            connection,
            access,
            refresh,
            expires_at,
            client_id,
            client_secret,
            token_endpoint,
            generation,
        )) = row
        else {
            return Ok(None);
        };
        let Some(kind) = PluginKind::from_row(&kind, &endpoint) else { return Ok(None) };

        Ok(Some(Dialed {
            kind,
            grant: (!access.is_empty()).then(|| crate::oauth::Grant {
                access_token: access.into(),
                refresh_token: (!refresh.is_empty()).then(|| refresh.into()),
                expires_at,
                client_id,
                client_secret: (!client_secret.is_empty()).then(|| client_secret.into()),
                token_endpoint,
            }),
            headers: Headers::decode(&headers),
            connection,
            generation,
        }))
    }

    pub fn refresh_plugin_grant(
        &self,
        id: PluginId,
        generation: i64,
        grant: &crate::oauth::Grant,
    ) -> Result<bool, StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE plugins SET access_token=?2, refresh_token=?3, expires_at=?4, grant_generation=grant_generation+1 WHERE id=?1 AND grant_generation=?5",
            params![
                id.to_string(),
                grant.access_token.expose(),
                grant.refresh_token.as_ref().map(Secret::expose).unwrap_or_default(),
                grant.expires_at,
                generation,
            ],
        )?;
        Ok(changed == 1)
    }

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

    pub fn delete_agent_signins(&self, agent: AgentId) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute("DELETE FROM signins WHERE agent_id=?1", params![agent.to_string()])?)
    }

    pub fn list_groups(&self) -> Result<Vec<Group>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT g.id, g.name, g.created_at,
                    (SELECT count(*) FROM agents a
                      WHERE a.group_id = g.id AND a.lifecycle <> 'terminated'),
                    g.base_url, g.api_key, g.default_model,
                    g.provider, g.subscription_model, g.request_timeout_secs,
                    g.max_hops, g.max_steps_per_run, g.max_fanout_per_call,
                    g.max_sends_per_pair, g.max_tool_rounds, g.reasoning_effort,
                    g.tidy_since IS NOT NULL, g.tidy_hour
               FROM groups g
              ORDER BY g.rail_order, g.created_at, g.rowid",
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
        let tidy = draft.tidy.unwrap_or_default();
        let now = now_ms();
        conn.execute(
            "INSERT INTO groups (id,name,created_at,base_url,api_key,default_model,
                                 provider,subscription_model,request_timeout_secs,
                                 max_hops,max_steps_per_run,max_fanout_per_call,
                                 max_sends_per_pair,max_tool_rounds,reasoning_effort,
                                 tidy_hour,tidy_since,rail_order)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,
                     (SELECT coalesce(max(rail_order), -1) + 1 FROM groups))",
            params![
                id.to_string(),
                draft.name,
                now,
                over.base_url,
                draft.api_key.as_ref().and_then(Option::as_ref).map(Secret::expose),
                over.default_model,
                over.provider.map(|p| p.as_str()),
                over.subscription_model,
                over.request_timeout_secs,
                limits.max_hops,
                limits.max_steps_per_run,
                limits.max_fanout_per_call.map(|n| n as i64),
                limits.max_sends_per_pair,
                limits.max_tool_rounds,
                over.reasoning_effort.map(|e| e.as_str()),
                tidy.hour,
                tidy.on.then_some(now),
            ],
        )
        .map_err(|e| classify(e, &draft.name))?;
        self.get_group(id)?.ok_or(StoreError::GroupNotFound(id))
    }

    pub fn update_group(&self, id: GroupId, draft: &CleanGroup) -> Result<Group, StoreError> {
        let conn = self.conn()?;
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
                        max_tool_rounds      = CASE WHEN ?11 THEN ?16 ELSE max_tool_rounds END,
                        reasoning_effort     = CASE WHEN ?3 THEN ?17 ELSE reasoning_effort END,
                        tidy_hour            = CASE WHEN ?18 THEN ?19 ELSE tidy_hour END,
                        -- Kept while it stays on: switching on is the moment
                        -- an agent never tidied starts counting from, and
                        -- saving the group again is not switching it on.
                        tidy_since           = CASE WHEN NOT ?18 THEN tidy_since
                                                    WHEN ?20 THEN coalesce(tidy_since, ?21)
                                                    ELSE NULL END
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
                    draft.api_key.as_ref().and_then(Option::as_ref).map(Secret::expose),
                    draft.limits.is_some(),
                    limits.max_hops,
                    limits.max_steps_per_run,
                    limits.max_fanout_per_call.map(|n| n as i64),
                    limits.max_sends_per_pair,
                    limits.max_tool_rounds,
                    over.reasoning_effort.map(|e| e.as_str()),
                    draft.tidy.is_some(),
                    draft.tidy.map(|t| t.hour),
                    draft.tidy.is_some_and(|t| t.on),
                    now_ms(),
                ],
            )
            .map_err(|e| classify(e, &draft.name))?;
        if changed == 0 {
            return Err(StoreError::GroupNotFound(id));
        }
        self.get_group(id)?.ok_or(StoreError::GroupNotFound(id))
    }

    pub fn group_inference(&self, id: GroupId) -> Result<GroupInference, StoreError> {
        let conn = self.conn()?;
        let found = conn
            .query_row(
                "SELECT base_url, api_key, default_model, provider, subscription_model,
                        request_timeout_secs, reasoning_effort
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
                            reasoning_effort: read_effort(row, 6)?,
                            request_timeout_secs: row.get(5)?,
                        },
                        api_key: row.get::<_, Option<String>>(1)?.map(Secret::from),
                    })
                },
            )
            .optional()?;
        Ok(found.unwrap_or_default())
    }

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

    pub fn move_group(
        &self,
        id: GroupId,
        before: Option<GroupId>,
    ) -> Result<Vec<Group>, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        {
            let mut stmt =
                tx.prepare("SELECT id FROM groups ORDER BY rail_order, created_at, rowid")?;
            let mut order = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let moving = id.to_string();
            let from =
                order.iter().position(|g| *g == moving).ok_or(StoreError::GroupNotFound(id))?;
            if before == Some(id) {
                drop(stmt);
                drop(tx);
                return self.list_groups();
            }
            order.remove(from);

            let at = before
                .map(|b| b.to_string())
                .and_then(|b| order.iter().position(|g| *g == b))
                .unwrap_or(order.len());
            order.insert(at, moving);

            let mut renumber = tx.prepare("UPDATE groups SET rail_order=?2 WHERE id=?1")?;
            for (position, group) in order.iter().enumerate() {
                renumber.execute(params![group, position as i32])?;
            }
        }
        tx.commit()?;
        self.list_groups()
    }

    pub fn group_for_removal(&self, id: GroupId) -> Result<Group, StoreError> {
        let group = self.get_group(id)?.ok_or(StoreError::GroupNotFound(id))?;
        let conn = self.conn()?;
        fallback_group(&conn, Some(id))?;
        Ok(group)
    }

    pub fn delete_group(&self, id: GroupId) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (name, agents): (String, u32) = tx
            .query_row(
                "SELECT name, (SELECT count(*) FROM agents
                WHERE group_id=?1 AND lifecycle <> 'terminated') FROM groups WHERE id=?1",
                params![id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(StoreError::GroupNotFound(id))?;
        let fallback = fallback_group(&tx, Some(id))?;
        if agents > 0 {
            return Err(StoreError::GroupNotEmpty { name, agents });
        }

        tx.execute(
            "UPDATE agents SET group_id=?2 WHERE group_id=?1",
            params![id.to_string(), fallback.to_string()],
        )?;
        tx.execute("DELETE FROM connectors WHERE group_id=?1", params![id.to_string()])?;
        tx.execute(
            "DELETE FROM plugin_agents WHERE plugin_id IN
                 (SELECT id FROM plugins WHERE group_id=?1)",
            params![id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM plugin_tool_access WHERE plugin_id IN
                 (SELECT id FROM plugins WHERE group_id=?1)",
            params![id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM plugin_tool_agents WHERE plugin_id IN
                 (SELECT id FROM plugins WHERE group_id=?1)",
            params![id.to_string()],
        )?;
        tx.execute("DELETE FROM plugins WHERE group_id=?1", params![id.to_string()])?;
        tx.execute("DELETE FROM occasions WHERE group_id=?1", params![id.to_string()])?;
        tx.execute(
            "DELETE FROM artifact_history WHERE artifact_id IN
                 (SELECT id FROM artifacts WHERE group_id=?1)",
            params![id.to_string()],
        )?;
        tx.execute("DELETE FROM artifacts WHERE group_id=?1", params![id.to_string()])?;
        tx.execute("DELETE FROM groups WHERE id=?1", params![id.to_string()])?;
        tx.commit()?;
        Ok(())
    }

    pub fn append(&self, envelope: &Envelope) -> Result<(), StoreError> {
        let conn = self.conn()?;
        Self::insert_message(&conn, envelope)
    }

    pub(super) fn insert_message(
        conn: &rusqlite::Connection,
        envelope: &Envelope,
    ) -> Result<(), StoreError> {
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

    pub fn append_delivery(&self, envelope: &Envelope) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        Self::insert_message(&tx, envelope)?;
        tx.execute(
            "INSERT OR IGNORE INTO pending_runs (run_id,message_id) VALUES (?1,?2)",
            params![envelope.run_id.to_string(), envelope.id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn claim_operator_send(
        &self,
        key: &str,
        fingerprint: &str,
        envelope: &Envelope,
        accepting: bool,
    ) -> Result<(MessageId, RunId, bool), StoreError> {
        self.claim_send(key, fingerprint, envelope, accepting, None)
    }

    pub fn claim_operator_send_put_down(
        &self,
        key: &str,
        fingerprint: &str,
        envelope: &Envelope,
        steps: u32,
        snapshot: &GuardSnapshot,
    ) -> Result<(MessageId, RunId, bool), StoreError> {
        self.claim_send(key, fingerprint, envelope, true, Some((steps, snapshot)))
    }

    fn claim_send(
        &self,
        key: &str,
        fingerprint: &str,
        envelope: &Envelope,
        accepting: bool,
        put_down: Option<(u32, &GuardSnapshot)>,
    ) -> Result<(MessageId, RunId, bool), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(String, String, String)> = tx
            .query_row(
                "SELECT fingerprint,message_id,run_id FROM operator_sends WHERE key=?1",
                [key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((stored, message, run)) = existing {
            if stored != fingerprint {
                return Err(StoreError::Decision(
                    "This idempotency key was already used for another message.".into(),
                ));
            }
            return Ok((
                message.parse().map_err(|e| StoreError::Corrupt(format!("bad message id: {e}")))?,
                run.parse().map_err(|e| StoreError::Corrupt(format!("bad run id: {e}")))?,
                false,
            ));
        }
        if !accepting {
            return Err(StoreError::Decision(
                "The host is stopping. Try again when it is available.".into(),
            ));
        }
        Self::insert_message(&tx, envelope)?;
        tx.execute(
            "INSERT INTO pending_runs (run_id,message_id) VALUES (?1,?2)",
            params![envelope.run_id.to_string(), envelope.id.to_string()],
        )?;
        tx.execute(
            "INSERT INTO operator_sends (key,fingerprint,message_id,run_id) VALUES (?1,?2,?3,?4)",
            params![key, fingerprint, envelope.id.to_string(), envelope.run_id.to_string()],
        )?;
        if let Some((steps, snapshot)) = put_down {
            tx.execute(
                "INSERT INTO put_down (message_id,records,absorbed) VALUES (?1,'[]','[]')",
                [envelope.id.to_string()],
            )?;
            tx.execute(
                "INSERT INTO put_down_runs (run_id,steps,guard_snapshot) VALUES (?1,?2,?3)",
                params![
                    envelope.run_id.to_string(),
                    steps,
                    serde_json::to_string(snapshot).expect("guard snapshot serializes")
                ],
            )?;
        }
        tx.commit()?;
        Ok((envelope.id, envelope.run_id, true))
    }

    pub fn operator_send_receipt(
        &self,
        key: &str,
        fingerprint: &str,
    ) -> Result<Option<(MessageId, RunId)>, StoreError> {
        let conn = self.conn()?;
        let stored: Option<(String, String, String)> = conn
            .query_row(
                "SELECT fingerprint,message_id,run_id FROM operator_sends WHERE key=?1",
                [key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((saved, message, run)) = stored else { return Ok(None) };
        if saved != fingerprint {
            return Err(StoreError::Decision(
                "This idempotency key was already used for another message.".into(),
            ));
        }
        Ok(Some((
            message.parse().map_err(|e| StoreError::Corrupt(format!("bad message id: {e}")))?,
            run.parse().map_err(|e| StoreError::Corrupt(format!("bad run id: {e}")))?,
        )))
    }

    pub fn settle_run(&self, run: RunId) -> Result<(), StoreError> {
        self.settle_run_as(run, Settled::Finished, 0).map(drop)
    }

    pub fn settle_run_as(
        &self,
        run: RunId,
        settled: Settled,
        steps: u32,
    ) -> Result<Option<Outcome>, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let outcome =
            super::outcomes::record(&tx, run, settled, steps, now_ms()).unwrap_or_else(|err| {
                tracing::error!(%err, %run, "could not record what a run ended in");
                None
            });
        tx.execute("DELETE FROM pending_runs WHERE run_id=?1", [run.to_string()])?;
        tx.execute(
            "UPDATE routine_occurrences SET state='done' WHERE owner_run=?1 AND state='started'",
            [run.to_string()],
        )?;
        tx.commit()?;
        if outcome.is_some() {
            self.wake_outbox();
        }
        Ok(outcome)
    }

    pub fn stop_routine_run(&self, run: RunId) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute("UPDATE routine_occurrences SET state='done' WHERE message_id IN (SELECT id FROM messages WHERE run_id=?1) OR owner_run=?1", [run.to_string()])?;
        tx.execute("DELETE FROM pending_runs WHERE run_id=?1", [run.to_string()])?;
        tx.execute(
            "DELETE FROM put_down WHERE message_id IN (SELECT id FROM messages WHERE run_id=?1)",
            [run.to_string()],
        )?;
        tx.execute("DELETE FROM put_down_runs WHERE run_id=?1", [run.to_string()])?;
        tx.commit()?;
        Ok(())
    }

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

    pub fn operator_addressed(&self, run: RunId, agent: AgentId) -> Result<bool, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT 1 FROM messages
              WHERE run_id=?1 AND from_kind='human' AND to_agent=?2 LIMIT 1",
        )?;
        Ok(stmt.exists(params![run.to_string(), agent.to_string()])?)
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

    pub fn agent_file(
        &self,
        agent: AgentId,
        name: &str,
    ) -> Result<Option<crate::domain::attachment::Attachment>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT m.id, m.channel_id, m.from_kind, m.from_agent, m.created_at, part.value
               FROM messages m, json_each(m.parts) AS part
              WHERE ({READ_BY_AGENT} OR ({WRITTEN_BY_AGENT}))
                AND json_extract(part.value, '$.type')='file'
                AND json_extract(part.value, '$.name')=?2 COLLATE NOCASE
              ORDER BY m.created_at DESC, m.id DESC, part.key DESC LIMIT 1"
        ))?;
        stmt.query_row(params![agent.to_string(), name], row_to_file_hit)
            .optional()?
            .transpose()
            .map(|hit| hit.map(|hit| hit.file))
    }

    pub fn agent_history(
        &self,
        agent: AgentId,
        through: i64,
        limit: u32,
    ) -> Result<Vec<Envelope>, StoreError> {
        const SHOWN: &str = "EXISTS (
                 SELECT 1 FROM json_each(parts) AS part
                  WHERE json_extract(part.value, '$.type') IN ('file', 'routine', 'decision')
                     OR (json_extract(part.value, '$.type')='text'
                         AND trim(json_extract(part.value, '$.text'),
                                  ' ' || char(9) || char(10) || char(13)) <> '')
             )";
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "WITH visible AS (
                 SELECT id, created_at FROM (
                     SELECT id, created_at FROM messages
                      WHERE {READ_BY_AGENT} AND created_at<=?2 AND {SHOWN}
                      ORDER BY created_at DESC, id DESC LIMIT ?3
                 )
                 UNION
                 SELECT id, created_at FROM (
                     SELECT id, created_at FROM messages
                      WHERE {WRITTEN_BY_AGENT} AND {SHOWN}
                      ORDER BY created_at DESC, id DESC LIMIT ?3
                 )
             ), recent AS (
                 SELECT id FROM visible ORDER BY created_at DESC, id DESC LIMIT ?3
             )
             SELECT id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,intent,cause,created_at
               FROM messages JOIN recent USING (id)
              ORDER BY created_at DESC, id DESC"
        ))?;
        let rows = stmt.query_map(params![agent.to_string(), through, limit], row_to_envelope)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        out.reverse();
        Ok(out)
    }

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

    pub fn conversation_flow(
        &self,
        group: GroupId,
        limit: u32,
    ) -> Result<Vec<Envelope>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,intent,cause,created_at
               FROM messages
              WHERE to_kind <> 'system'
                AND channel_id IN (SELECT id FROM agents WHERE group_id=?1)
              ORDER BY created_at DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![group.to_string(), limit], row_to_envelope)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        out.reverse();
        Ok(out)
    }

    pub fn channel_messages_through(
        &self,
        channel: AgentId,
        through: MessageId,
        cap: u32,
    ) -> Result<Vec<Envelope>, StoreError> {
        let conn = self.conn()?;
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
            artifacts: self.matching_artifacts(&pattern, limit)?,
        })
    }

    fn matching_messages(&self, pattern: &str, limit: u32) -> Result<Vec<Envelope>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r"SELECT id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,intent,cause,created_at
                FROM messages
               WHERE to_kind <> 'system'
                 AND EXISTS (
                     SELECT 1 FROM json_each(messages.parts) AS part
                      WHERE (json_extract(part.value, '$.type') = 'text'
                             AND json_extract(part.value, '$.text') LIKE ?1 ESCAPE '\')
                         OR (json_extract(part.value, '$.type') = 'decision'
                             AND (json_extract(part.value, '$.answer') LIKE ?1 ESCAPE '\'
                                  OR json_extract(part.value, '$.request.question') LIKE ?1 ESCAPE '\'))
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

    pub fn record_usage(&self, entry: &UsageEntry) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO usage (agent_id,group_id,run_id,model,prompt,completion,cost,created_at,cached)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                entry.agent_id.to_string(),
                entry.group_id.to_string(),
                entry.run_id.to_string(),
                entry.model,
                entry.prompt,
                entry.completion,
                entry.cost,
                now_ms(),
                entry.cached,
            ],
        )?;
        Ok(())
    }

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
            if let Ok(group) = id.parse::<GroupId>() {
                out.insert(group, tokens);
            }
        }
        Ok(out)
    }

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

    pub fn delete_group_messages(&self, group: GroupId) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute(
            "DELETE FROM messages
              WHERE channel_id IN (SELECT id FROM agents WHERE group_id=?1)",
            params![group.to_string()],
        )?)
    }

    pub fn delete_group_routines(&self, group: GroupId) -> Result<usize, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM routine_runs WHERE routine_id IN (SELECT id FROM routines WHERE agent_id IN (SELECT id FROM agents WHERE group_id=?1))", [group.to_string()])?;
        let deleted = tx.execute(
            "DELETE FROM routines
              WHERE agent_id IN (SELECT id FROM agents WHERE group_id=?1)",
            params![group.to_string()],
        )?;
        tx.commit()?;
        Ok(deleted)
    }

    pub fn delete_group_working_notes(&self, group: GroupId) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute(
            "DELETE FROM working_notes
              WHERE agent_id IN (SELECT id FROM agents WHERE group_id=?1)",
            params![group.to_string()],
        )?)
    }

    pub fn delete_group_usage(&self, group: GroupId) -> Result<usize, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute("DELETE FROM usage WHERE group_id=?1", params![group.to_string()])?)
    }

    pub fn group_crew(&self, group: GroupId) -> Result<Vec<AgentCard>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {AGENT_COLUMNS}
               FROM agents WHERE group_id=?1 AND lifecycle <> 'terminated' ORDER BY rowid"
        ))?;
        let rows = stmt.query_map(params![group.to_string()], row_to_card)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

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

fn announce_escalation(
    tx: &rusqlite::Connection,
    id: EscalationId,
    agent: AgentId,
    group: GroupId,
    run: RunId,
    at: i64,
) -> Result<(), StoreError> {
    let name = super::outbox::name_of(tx, agent)?.unwrap_or_default();
    let day = crate::domain::local(at).map(|at| at.date_naive().to_string()).unwrap_or_default();
    super::outbox::enqueue(tx, &outbox::for_escalation(id, agent, group, run, &name, &day), at)?;
    Ok(())
}

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

pub(super) type RowResult<T> = Result<Result<T, StoreError>, rusqlite::Error>;

const READ_BY_AGENT: &str = "channel_id=?1";

const WRITTEN_BY_AGENT: &str = "from_kind='agent' AND from_agent=?1";

const AGENT_COLUMNS: &str = "id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id,sandbox_id,sandbox_envd_token,sandbox_traffic_token,pinned,rail_order,browser_id,has_computer,has_browser,browser_consent,discarded_at,reasoning_effort,runs_errands,subscription_model";

fn read_effort(
    row: &Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<crate::domain::effort::ReasoningEffort>> {
    row.get::<_, Option<String>>(index)?
        .map(|raw| {
            serde_json::from_value(serde_json::Value::String(raw)).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    rusqlite::types::Type::Text,
                    Box::new(err),
                )
            })
        })
        .transpose()
}

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
        let skills: Vec<String> = serde_json::from_str(&skills_raw).unwrap_or_default();

        Ok(AgentCard {
            id,
            name: row.get(1)?,
            avatar: row.get(2)?,
            color: row.get(3)?,
            model: row.get(4)?,
            subscription_model: row.get(24)?,
            reasoning_effort: read_effort(row, 22)?,
            system_prompt: row.get(5)?,
            skills,
            lifecycle,
            group_id: {
                let raw: String = row.get(11)?;
                raw.parse::<GroupId>()
                    .map_err(|e| StoreError::Corrupt(format!("bad group id {raw:?}: {e}")))?
            },
            sandbox_id: row.get(12)?,
            sandbox_envd_token: row.get::<_, Option<String>>(13)?.map(Secret::from),
            sandbox_traffic_token: row.get::<_, Option<String>>(14)?.map(Secret::from),
            pinned: row.get::<_, i64>(15)? != 0,
            rail_order: row.get(16)?,
            browser_id: row.get(17)?,
            has_computer: row.get::<_, i64>(18)? != 0,
            has_browser: row.get::<_, i64>(19)? != 0,
            browser_consent: Consent::parse(&row.get::<_, String>(20)?),
            runs_errands: row.get::<_, i64>(23)? != 0,
            version: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
            discarded_at: row.get(21)?,
        })
    })())
}

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

fn row_to_upcoming_run(row: &Row<'_>) -> RowResult<UpcomingRun> {
    let routine_raw: String = row.get(0)?;
    let agent_raw: String = row.get(1)?;
    let group_raw: String = row.get(2)?;

    Ok((|| {
        Ok(UpcomingRun {
            routine_id: routine_raw
                .parse::<RoutineId>()
                .map_err(|e| StoreError::Corrupt(format!("bad routine id {routine_raw:?}: {e}")))?,
            agent_id: agent_raw
                .parse::<AgentId>()
                .map_err(|e| StoreError::Corrupt(format!("bad agent id {agent_raw:?}: {e}")))?,
            group_id: group_raw
                .parse::<GroupId>()
                .map_err(|e| StoreError::Corrupt(format!("bad group id {group_raw:?}: {e}")))?,
            name: row.get(3)?,
            next_run_at: row.get(4)?,
        })
    })())
}

fn row_to_routine_run(row: &Row<'_>) -> RowResult<RoutineRun> {
    let run_raw: Option<String> = row.get(0)?;
    let kind_raw: String = row.get(1)?;

    Ok((|| {
        Ok(RoutineRun {
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
                cost: row.get::<_, Option<f64>>(5)?,
                calls: row.get::<_, i64>(6)? as u64,
            },
            error: row.get(7)?,
            missed: row.get::<_, i64>(8)?.try_into().unwrap_or(u32::MAX),
        })
    })())
}

const OCCASION_COLUMNS: &str = "SELECT id,group_id,agent_id,title,detail,place,starts_at,minutes,\
                                all_day,created_at,updated_at FROM occasions";

fn row_to_occasion(row: &Row<'_>) -> RowResult<Occasion> {
    let id_raw: String = row.get(0)?;
    let group_raw: String = row.get(1)?;
    let agent_raw: Option<String> = row.get(2)?;

    Ok((|| {
        Ok(Occasion {
            id: id_raw
                .parse::<OccasionId>()
                .map_err(|e| StoreError::Corrupt(format!("bad occasion id {id_raw:?}: {e}")))?,
            group_id: group_raw
                .parse::<GroupId>()
                .map_err(|e| StoreError::Corrupt(format!("bad group id {group_raw:?}: {e}")))?,
            agent_id: agent_raw
                .map(|raw| {
                    raw.parse::<AgentId>()
                        .map_err(|e| StoreError::Corrupt(format!("bad agent id {raw:?}: {e}")))
                })
                .transpose()?,
            title: row.get(3)?,
            detail: row.get(4)?,
            place: row.get(5)?,
            starts_at: row.get(6)?,
            minutes: row.get(7)?,
            all_day: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
        })
    })())
}

fn row_to_escalation(row: &Row<'_>) -> RowResult<Escalation> {
    let id_raw: String = row.get(0)?;
    let agent_raw: String = row.get(1)?;
    let group_raw: String = row.get(2)?;
    let run_raw: String = row.get(3)?;

    Ok((|| {
        Ok(Escalation {
            id: id_raw
                .parse::<EscalationId>()
                .map_err(|e| StoreError::Corrupt(format!("bad escalation id {id_raw:?}: {e}")))?,
            agent_id: agent_raw
                .parse::<AgentId>()
                .map_err(|e| StoreError::Corrupt(format!("bad agent id {agent_raw:?}: {e}")))?,
            group_id: group_raw
                .parse::<GroupId>()
                .map_err(|e| StoreError::Corrupt(format!("bad group id {group_raw:?}: {e}")))?,
            run_id: run_raw
                .parse::<RunId>()
                .map_err(|e| StoreError::Corrupt(format!("bad run id {run_raw:?}: {e}")))?,
            summary: row.get(4)?,
            raised_at: row.get(5)?,
            said_at: row.get(6)?,
            times: row.get(7)?,
            cleared_at: row.get(8)?,
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
            request: if action_raw == QUESTION {
                Request::Question {
                    options: match &options_raw {
                        Some(raw) => serde_json::from_str(raw).map_err(|e| {
                            StoreError::Corrupt(format!("bad question options: {e}"))
                        })?,
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
            detail: serde_json::from_str(&detail_raw)
                .map_err(|e| StoreError::Corrupt(format!("bad approval detail: {e}")))?,
            answer: row.get(11)?,
            state: ApprovalState::parse(&state_raw).ok_or_else(|| {
                StoreError::Corrupt(format!("unknown approval state {state_raw:?}"))
            })?,
            created_at: row.get(8)?,
            decided_at: row.get(9)?,
            lapses_at: None,
        })
    })())
}

const CONNECTOR_COLUMNS: &str =
    "SELECT id,group_id,service,account,env_var,(trim(secret) <> ''),note,\
                                 created_at,updated_at, \
    (SELECT json_group_array(ca.agent_id) FROM connector_agents ca \
     JOIN agents a ON a.id=ca.agent_id AND a.group_id=connectors.group_id \
     WHERE ca.connector_id=connectors.id AND a.discarded_at IS NULL) FROM connectors";

fn classify_connector(err: rusqlite::Error, env_var: &str) -> StoreError {
    if let rusqlite::Error::SqliteFailure(inner, _) = &err {
        if inner.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE {
            return StoreError::DuplicateEnvVar(env_var.to_string());
        }
    }
    StoreError::Sqlite(err)
}

fn write_connector_agents(
    conn: &rusqlite::Connection,
    id: ConnectorId,
    agents: &[AgentId],
) -> Result<(), StoreError> {
    let group: String = conn
        .query_row("SELECT group_id FROM connectors WHERE id=?1", params![id.to_string()], |row| {
            row.get(0)
        })
        .optional()?
        .ok_or(StoreError::ConnectorNotFound(id))?;
    for agent in agents {
        let valid: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM agents WHERE id=?1 AND group_id=?2 AND discarded_at IS NULL)",
            params![agent.to_string(), group], |row| row.get(0))?;
        if !valid {
            return Err(StoreError::AgentNotInGroup(*agent));
        }
    }
    conn.execute("DELETE FROM connector_agents WHERE connector_id=?1", params![id.to_string()])?;
    for agent in agents {
        conn.execute(
            "INSERT OR IGNORE INTO connector_agents (connector_id,agent_id) VALUES (?1,?2)",
            params![id.to_string(), agent.to_string()],
        )?;
    }
    Ok(())
}

fn row_to_connector(row: &Row<'_>) -> RowResult<Connector> {
    let id_raw: String = row.get(0)?;
    let group_raw: String = row.get(1)?;
    let secret_set: bool = row.get(5)?;

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
            secret_set,
            secret_hint: String::new(),
            agents: serde_json::from_str(&row.get::<_, String>(9)?)
                .map_err(|_| StoreError::Corrupt("invalid secret grants".into()))?,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
        })
    })())
}

#[derive(Debug)]
pub enum PluginReach {
    NotConnected,
    NotChosen,
    ToolDenied,
    ToolNotChosen,
    Granted(Box<Reached>),
}

#[derive(Debug)]
pub struct Reached {
    pub id: PluginId,
    pub grant: Option<crate::oauth::Grant>,
    pub generation: i64,
    pub headers: Headers,
    pub schema: Option<serde_json::Value>,
}

pub struct Dialed {
    pub kind: PluginKind,
    pub grant: Option<crate::oauth::Grant>,
    pub generation: i64,
    pub headers: Headers,
    pub connection: String,
}

const PLUGIN_COLUMNS: &str = "SELECT id,group_id,kind,account,tools,access_token,connected_at,\
     access,connection,endpoint,headers FROM plugins";

const PLUGIN_REACHED_BY_AGENT: &str = "(plugins.access = 'everyone'
     OR EXISTS (SELECT 1 FROM plugin_agents
                 WHERE plugin_agents.plugin_id = plugins.id
                   AND plugin_agents.agent_id = :agent))";

const PLUGIN_TOOL_REACHED_BY_AGENT: &str = "NOT EXISTS (SELECT 1 FROM plugin_tool_access
                 WHERE plugin_tool_access.plugin_id = plugins.id
                   AND plugin_tool_access.tool = :tool
                   AND plugin_tool_access.access <> 'everyone'
                   AND NOT EXISTS (SELECT 1 FROM plugin_tool_agents
                                    WHERE plugin_tool_agents.plugin_id = plugins.id
                                      AND plugin_tool_agents.tool = :tool
                                      AND plugin_tool_agents.agent_id = :agent))";

const PLUGIN_TOOL_REACHED_BY_ANYONE: &str = "NOT EXISTS (SELECT 1 FROM plugin_tool_access
                 WHERE plugin_tool_access.plugin_id = plugins.id
                   AND plugin_tool_access.tool = :tool
                   AND plugin_tool_access.access <> 'everyone'
                   AND NOT EXISTS (SELECT 1 FROM plugin_tool_agents
                                    WHERE plugin_tool_agents.plugin_id = plugins.id
                                      AND plugin_tool_agents.tool = :tool))";

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
    let endpoint_raw: String = row.get(9)?;
    let headers: String = row.get(10)?;

    Ok((|| {
        let Some(kind) = PluginKind::from_row(&kind_raw, &endpoint_raw) else { return Ok(None) };
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
            name: kind.label().to_string(),
            endpoint: kind.endpoint().to_string(),
            custom: kind.is_custom(),
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
            headers: Headers::decode(&headers).names(),
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
                reasoning_effort: read_effort(row, 15)?,
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
            tidy: crate::domain::group::Tidy { on: row.get(16)?, hour: row.get(17)? },
        })
    })())
}

pub(super) fn row_to_envelope(row: &Row<'_>) -> RowResult<Envelope> {
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
    use crate::domain::envelope::NoticeKind;
    use crate::domain::envelope::ToolOutcome;
    use crate::domain::ids::RunId;
    use crate::domain::plugin::HeaderPair;
    use crate::domain::routine::Cadence;

    #[test]
    fn resource_transitions_preserve_adopted_and_confirmed_gone() {
        use ResourceEvent::{ConfirmGone, Release};
        use ResourceState::{Adopted, Created, Gone, Released};
        for (state, release, gone) in [
            (Created, Some(Released), Some(Gone)),
            (Adopted, None, Some(Gone)),
            (Released, None, Some(Gone)),
            (Gone, None, None),
        ] {
            assert_eq!(state.transition(Release), release);
            assert_eq!(state.transition(ConfirmGone), gone);
        }
    }

    #[test]
    fn a_profile_name_conflict_never_reclaims_the_old_owner() {
        let f = fixture();
        f.store.record_resource("kernel-profile", "guac-agent", false).unwrap();
        f.store.record_adopted_profile("guac-agent", None).unwrap();
        assert_eq!(
            f.store.resource_state("kernel-profile", "guac-agent").unwrap(),
            Some(ResourceState::Adopted)
        );
        assert!(f.store.mark_resource_released("kernel-profile", "guac-agent").is_err());
    }

    #[test]
    fn profile_ownership_survives_only_a_matching_provider_id() {
        let f = fixture();
        f.store.record_created_profile("profile", Some("old-id")).unwrap();
        f.store.record_adopted_profile("profile", Some("old-id")).unwrap();
        assert_eq!(
            f.store.resource_state("kernel-profile", "profile").unwrap(),
            Some(ResourceState::Created)
        );
        f.store.record_adopted_profile("profile", Some("new-id")).unwrap();
        assert_eq!(
            f.store.resource_state("kernel-profile", "profile").unwrap(),
            Some(ResourceState::Adopted)
        );
        assert_eq!(f.store.created_profile_id("profile").unwrap(), None);
    }

    #[test]
    fn release_does_not_succeed_if_the_ledger_transition_did_not_land() {
        let f = fixture();
        f.store.record_resource("kernel-profile", "profile", false).unwrap();
        f.store.conn().unwrap().execute_batch("CREATE TRIGGER interrupt_release BEFORE UPDATE OF released_at ON resource_ledger BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(f.store.mark_resource_released("kernel-profile", "profile").is_err());
        assert_eq!(
            f.store.resource_state("kernel-profile", "profile").unwrap(),
            Some(ResourceState::Created)
        );
    }

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

    fn tool(name: &str) -> PluginTool {
        PluginTool {
            name: name.into(),
            description: String::new(),
            input_schema: serde_json::json!({ "type": "object" }),
        }
    }

    fn nobody() -> PluginAccess {
        PluginAccess::Chosen { agents: Vec::new() }
    }

    fn draft(name: &str) -> CleanDraft {
        CleanDraft {
            group_id: None,
            name: name.into(),
            avatar: "orb".into(),
            color: "#7fb069".into(),
            model: "anthropic/claude-sonnet-4.5".into(),
            subscription_model: None,
            reasoning_effort: None,
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

    #[test]
    fn onboarding_hire_survives_lost_reply_and_reopen() {
        let f = fixture();
        let (first, created) =
            f.store.create_onboarding_agent("hire", "same", &draft("Worker")).unwrap();
        assert!(created);
        let reopened = Store::open(&f._dir.path().join("guac.db")).unwrap();
        let (again, created) =
            reopened.create_onboarding_agent("hire", "same", &draft("Worker 2")).unwrap();
        assert!(!created);
        assert_eq!(first.id, again.id);
        assert_eq!(reopened.list_agents().unwrap().len(), 1);
        assert!(reopened.create_onboarding_agent("hire", "different", &draft("Other")).is_err());
    }

    #[test]
    fn keyed_send_is_atomic_and_survives_lost_reply_and_reopen() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Worker")).unwrap();
        let first = envelope(
            Participant::Human,
            Participant::Agent { id: agent.id },
            "do work",
            RunId::new(),
        );
        f.store.conn().unwrap().execute_batch("CREATE TRIGGER fail_claim BEFORE INSERT ON operator_sends BEGIN SELECT RAISE(ABORT, 'fail'); END;").unwrap();
        assert!(f.store.claim_operator_send("send", "same", &first, true).is_err());
        assert!(f.store.channel_messages(agent.id, 20).unwrap().is_empty());
        f.store.conn().unwrap().execute_batch("DROP TRIGGER fail_claim;").unwrap();
        assert_eq!(
            f.store.claim_operator_send("send", "same", &first, true).unwrap(),
            (first.id, first.run_id, true)
        );
        let reopened = Store::open(&f._dir.path().join("guac.db")).unwrap();
        let retry = envelope(
            Participant::Human,
            Participant::Agent { id: agent.id },
            "do work",
            RunId::new(),
        );
        assert_eq!(
            reopened.claim_operator_send("send", "same", &retry, false).unwrap(),
            (first.id, first.run_id, false)
        );
        assert!(reopened.claim_operator_send("send", "different", &retry, true).is_err());
        assert_eq!(reopened.channel_messages(agent.id, 20).unwrap().len(), 1);
    }

    #[test]
    fn interrupted_work_survives_reopening_and_is_reported_exactly_once() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Worker")).unwrap();
        let original = envelope(
            Participant::Human,
            Participant::Agent { id: agent.id },
            "do work",
            RunId::new(),
        );
        f.store.append_delivery(&original).unwrap();
        let mut followup = original.clone();
        followup.id = MessageId::new();
        f.store.append_delivery(&followup).unwrap();
        let reopened = Store::open(&f._dir.path().join("guac.db")).unwrap();
        assert_eq!(reopened.recover().unwrap().interrupted, 1);
        assert_eq!(reopened.recover().unwrap().interrupted, 0);
        let messages = reopened.channel_messages(agent.id, 20).unwrap();
        let notice = messages.iter().find(|m| m.cause == Some(original.id)).unwrap();
        assert!(matches!(notice.parts[0], Part::Notice { kind: NoticeKind::Interrupted, .. }));
        assert!(!notice.expects_reply);
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn settled_work_and_failed_acceptance_do_not_reappear_after_restart() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Worker")).unwrap();
        let original = envelope(
            Participant::Human,
            Participant::Agent { id: agent.id },
            "do work",
            RunId::new(),
        );
        f.store.append_delivery(&original).unwrap();
        f.store.settle_run(original.run_id).unwrap();
        assert!(
            f.store.append_delivery(&original).is_err(),
            "duplicate acceptance fails atomically"
        );
        assert_eq!(f.store.recover().unwrap().interrupted, 0);
        assert_eq!(f.store.channel_messages(agent.id, 20).unwrap().len(), 1);
    }

    fn work(from: AgentId, to: AgentId, text: &str) -> Envelope {
        let mut envelope = envelope(
            Participant::Agent { id: from },
            Participant::Agent { id: to },
            text,
            RunId::new(),
        );
        envelope.intent = Intent::Work;
        envelope
    }

    fn group_named(name: &str) -> CleanGroup {
        CleanGroup { name: name.into(), ..Default::default() }
    }

    fn draft_in(name: &str, group: GroupId) -> CleanDraft {
        CleanDraft { group_id: Some(group), ..draft(name) }
    }

    fn key_for(group: GroupId, env_var: &str, secret: &str) -> CleanConnector {
        CleanConnector {
            group_id: group,
            service: "GitHub".into(),
            account: "madebywelch".into(),
            env_var: env_var.into(),
            note: String::new(),
            agents: vec![],
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
        let f = fixture();
        let card = f.store.create_agent(&draft("Talker")).unwrap();
        assert!(!card.has_computer);
        assert!(!card.has_browser);

        f.store.set_has_computer(card.id, true).unwrap();
        let given = f.store.get_agent(card.id).unwrap().unwrap();
        assert!(given.has_computer);
        assert!(!given.has_browser, "the two are given separately or they are one switch");
        assert_eq!(given.version, card.version, "a grant is not an edit to the card");

        f.store.set_agent_sandbox(card.id, Some(("sb-1", "envd", "traffic"))).unwrap();
        f.store.set_has_computer(card.id, false).unwrap();
        let taken = f.store.get_agent(card.id).unwrap().unwrap();
        assert!(!taken.has_computer);
        assert_eq!(taken.sandbox_id.as_deref(), Some("sb-1"));
    }

    #[test]
    fn a_new_agents_browser_asks_nobody_and_the_operator_can_change_that() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Researcher")).unwrap();
        assert_eq!(card.browser_consent, Consent::Open);

        f.store.set_browser_consent(card.id, Consent::AskBeforeActing).unwrap();
        let held = f.store.get_agent(card.id).unwrap().unwrap();
        assert_eq!(held.browser_consent, Consent::AskBeforeActing);
        assert_eq!(held.version, card.version, "holding an agent back is not an edit to the card");

        f.store.set_has_browser(card.id, true).unwrap();
        f.store.set_has_browser(card.id, false).unwrap();
        assert_eq!(
            f.store.get_agent(card.id).unwrap().unwrap().browser_consent,
            Consent::AskBeforeActing,
        );

        f.store.set_browser_consent(card.id, Consent::Open).unwrap();
        assert_eq!(f.store.get_agent(card.id).unwrap().unwrap().browser_consent, Consent::Open);
    }

    #[test]
    fn a_new_agent_sends_no_errands_until_the_operator_says_so() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Coordinator")).unwrap();
        assert!(!card.runs_errands);

        f.store.set_runs_errands(card.id, true).unwrap();
        let given = f.store.get_agent(card.id).unwrap().unwrap();
        assert!(given.runs_errands);
        assert!(!given.has_computer && !given.has_browser, "errands are not a machine");
        assert_eq!(given.version, card.version, "a grant is not an edit to the card");

        f.store.set_runs_errands(card.id, false).unwrap();
        assert!(!f.store.get_agent(card.id).unwrap().unwrap().runs_errands);
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
        assert!(matches!(
            f.store.set_browser_consent(gone, Consent::AskBeforeActing),
            Err(StoreError::AgentNotFound(id)) if id == gone
        ));
        assert!(matches!(
            f.store.set_runs_errands(gone, true),
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
        assert!(stored.secret_hint.is_empty());
        f.store.update_connector(stored.id, &[agent.id], None).unwrap();
        let json = serde_json::to_string(&stored).unwrap();
        assert!(!json.contains("hunter2"), "a secret reached the wire: {json}");

        let env = f.store.connector_env(agent.id).unwrap();
        assert_eq!(env.get("GITHUB_TOKEN"), Some("ghp_hunter2"));
    }

    #[test]
    fn secret_grants_are_atomic_and_rechecked_against_live_membership() {
        let f = fixture();
        let mine = f.store.create_agent(&draft("Engineer")).unwrap();
        let peer = f.store.create_agent(&draft("Researcher")).unwrap();
        let other = f.store.create_group(&group_named("Other")).unwrap();
        let outsider = f.store.create_agent(&draft_in("Outsider", other.id)).unwrap();
        let clean = key_for(mine.group_id, "CLOUDFLARE_API_TOKEN", "original-token");
        let secret = f.store.create_connector(&clean).unwrap();
        assert!(secret.agents.is_empty());
        assert!(f.store.connector_env(mine.id).unwrap().is_empty());
        assert!(f.store.update_connector(secret.id, &[outsider.id], Some("wrong-token")).is_err());
        assert!(f.store.connector_env(outsider.id).unwrap().is_empty());
        f.store.update_connector(secret.id, &[mine.id, mine.id], None).unwrap();
        assert_eq!(f.store.agent_connectors(mine.id).unwrap()[0].agents, vec![mine.id]);
        assert!(f.store.agent_connectors(peer.id).unwrap().is_empty());
        assert_eq!(
            f.store.connector_env(mine.id).unwrap().get("CLOUDFLARE_API_TOKEN").unwrap(),
            "original-token"
        );
        assert!(f
            .store
            .update_connector(secret.id, &[mine.id, outsider.id], Some("wrong-token"))
            .is_err());
        assert_eq!(
            f.store.connector_env(mine.id).unwrap().get("CLOUDFLARE_API_TOKEN").unwrap(),
            "original-token"
        );
        f.store.update_connector(secret.id, &[mine.id], Some("rotated-token")).unwrap();
        assert_eq!(
            f.store.connector_env(mine.id).unwrap().get("CLOUDFLARE_API_TOKEN").unwrap(),
            "rotated-token"
        );
        f.store.update_agent(mine.id, &draft_in("Engineer", other.id)).unwrap();
        assert!(f.store.connector_env(mine.id).unwrap().is_empty());
        assert!(f.store.agent_connectors(mine.id).unwrap().is_empty());
        f.store.update_connector(secret.id, &[peer.id], None).unwrap();
        f.store.discard_agent(peer.id, 1_000).unwrap();
        assert!(f.store.connector_env(peer.id).unwrap().is_empty());
        f.store.update_connector(secret.id, &[], None).unwrap();
        assert!(f.store.get_connector(secret.id).unwrap().unwrap().agents.is_empty());
        f.store.delete_connector(secret.id).unwrap();
        assert!(f.store.connector_names().unwrap().is_empty());
    }

    #[test]
    fn adding_a_secret_with_a_foreign_recipient_creates_nothing() {
        let f = fixture();
        let mine = f.store.create_agent(&draft("Engineer")).unwrap();
        let mut clean = key_for(mine.group_id, "TOKEN", "private-token");
        clean.agents = vec![AgentId::new()];
        assert!(f.store.create_connector(&clean).is_err());
        assert!(f.store.group_connectors(mine.group_id).unwrap().is_empty());
    }

    #[test]
    fn migrating_secrets_preserves_current_recipients_but_not_future_agents() {
        let f = fixture();
        let mine = f.store.create_agent(&draft("Engineer")).unwrap();
        let discarded = f.store.create_agent(&draft("Discarded")).unwrap();
        f.store.discard_agent(discarded.id, 1_000).unwrap();
        f.store.create_connector(&key_for(mine.group_id, "TOKEN", "private-token")).unwrap();
        let mut conn = f.store.conn().unwrap();
        conn.execute_batch("ALTER TABLE agents DROP COLUMN tidied_through; ALTER TABLE agents DROP COLUMN last_tidy_at; ALTER TABLE groups DROP COLUMN tidy_since; ALTER TABLE groups DROP COLUMN tidy_hour; ALTER TABLE usage DROP COLUMN cached; ALTER TABLE pending_runs DROP COLUMN started_at; DROP TABLE permission_grants; ALTER TABLE decisions DROP COLUMN origin_ref; ALTER TABLE decisions DROP COLUMN origin; DROP TABLE outbox; DROP TABLE outcomes; ALTER TABLE decisions DROP COLUMN filed_run; DROP TABLE onboarding_hires; DROP TABLE operator_sends; DROP TRIGGER routine_delete_history; DROP TABLE routine_occurrences; ALTER TABLE plugins DROP COLUMN grant_generation; DROP TABLE pending_sandbox_resume; DROP TABLE resource_ledger; DROP TABLE workspace_identity; DROP TABLE approval_policies; ALTER TABLE groups DROP COLUMN rail_order; DROP TABLE put_down_runs; DROP TABLE put_down; DROP TABLE connector_agents; ALTER TABLE agents DROP COLUMN reasoning_effort; ALTER TABLE groups DROP COLUMN reasoning_effort; ALTER TABLE agents DROP COLUMN runs_errands; DROP TABLE artifact_history; DROP TABLE artifacts; ALTER TABLE agents ADD COLUMN repository_id TEXT; CREATE TABLE repositories (id TEXT PRIMARY KEY, group_id TEXT NOT NULL, harness TEXT NOT NULL DEFAULT 'pi', gate TEXT NOT NULL DEFAULT 'open'); PRAGMA user_version=51;").unwrap();
        migrations::run(&mut conn).unwrap();
        drop(conn);
        assert_eq!(f.store.connector_env(mine.id).unwrap().get("TOKEN").unwrap(), "private-token");
        assert!(f.store.connector_env(discarded.id).unwrap().is_empty());
        let later = f.store.create_agent(&draft("Later")).unwrap();
        assert!(f.store.connector_env(later.id).unwrap().is_empty());
    }

    #[test]
    fn a_discarded_agent_is_terminated_and_stamped() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Researcher")).unwrap();

        let gone = f.store.discard_agent(agent.id, 1_000).unwrap();
        assert_eq!(gone.lifecycle, Lifecycle::Terminated, "it has to be unreachable at once");
        assert_eq!(gone.discarded_at, Some(1_000));
        assert!(gone.discarded(), "and findable again, which is the whole of the compost");

        assert!(
            f.store.group_crew(agent.group_id).unwrap().is_empty(),
            "a composted agent must not be handed to a disband"
        );
    }

    #[test]
    fn a_discarded_agent_keeps_the_name_out_of_the_way() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Researcher")).unwrap();
        f.store.discard_agent(agent.id, 1).unwrap();

        let replacement = f.store.create_agent(&draft("Researcher")).unwrap();
        assert_eq!(replacement.name, "Researcher");
    }

    #[test]
    fn discarding_twice_does_not_buy_another_thirty_days() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Researcher")).unwrap();
        f.store.discard_agent(agent.id, 1_000).unwrap();

        let again = f.store.discard_agent(agent.id, 9_000);
        assert!(matches!(again, Err(StoreError::AgentNotFound(_))), "got {again:?}");
        assert_eq!(f.store.get_agent(agent.id).unwrap().unwrap().discarded_at, Some(1_000));
    }

    #[test]
    fn a_restore_brings_an_agent_back_paused_and_unstamped() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Researcher")).unwrap();
        f.store.discard_agent(agent.id, 1).unwrap();

        let back = f.store.restore_agent(agent.id, "Researcher").unwrap();
        assert_eq!(back.lifecycle, Lifecycle::Paused);
        assert_eq!(back.discarded_at, None);
        assert!(!back.discarded());
        assert_eq!(
            f.store.group_crew(agent.group_id).unwrap().len(),
            1,
            "and is in the crew again"
        );
    }

    #[test]
    fn a_restore_can_take_a_name_that_was_given_away() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Researcher")).unwrap();
        f.store.discard_agent(agent.id, 1).unwrap();
        f.store.create_agent(&draft("Researcher")).unwrap();

        let back = f.store.restore_agent(agent.id, "Researcher copy").unwrap();
        assert_eq!(back.name, "Researcher copy");
    }

    #[test]
    fn a_restore_is_refused_for_an_agent_whose_wait_is_over() {
        let f = fixture();
        let live = f.store.create_agent(&draft("Researcher")).unwrap();
        let spent = f.store.create_agent(&draft("Scribe")).unwrap();
        f.store.discard_agent(spent.id, 1).unwrap();
        f.store.forget_discard(spent.id).unwrap();

        for id in [live.id, spent.id] {
            let refused = f.store.restore_agent(id, "Whoever");
            assert!(matches!(refused, Err(StoreError::AgentNotFound(_))), "got {refused:?}");
        }
        assert_eq!(
            f.store.get_agent(spent.id).unwrap().unwrap().lifecycle,
            Lifecycle::Terminated,
            "a refused restore must not move the row"
        );
    }

    #[test]
    fn the_compost_reads_newest_first_and_leaves_out_whoever_has_already_gone() {
        let f = fixture();
        let old = f.store.create_agent(&draft("Scribe")).unwrap();
        let new = f.store.create_agent(&draft("Researcher")).unwrap();
        let spent = f.store.create_agent(&draft("Ghost")).unwrap();
        let live = f.store.create_agent(&draft("Manager")).unwrap();

        f.store.discard_agent(old.id, 100).unwrap();
        f.store.discard_agent(new.id, 900).unwrap();
        f.store.discard_agent(spent.id, 50).unwrap();
        f.store.forget_discard(spent.id).unwrap();

        let names: Vec<String> =
            f.store.discarded_agents().unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(
            names,
            vec!["Researcher", "Scribe"],
            "what was deleted a minute ago is what somebody came here for"
        );
        assert!(!names.contains(&live.name), "a live agent is not in the compost");
    }

    #[test]
    fn only_agents_past_the_deadline_are_swept() {
        let f = fixture();
        let due = f.store.create_agent(&draft("Scribe")).unwrap();
        let waiting = f.store.create_agent(&draft("Researcher")).unwrap();
        f.store.discard_agent(due.id, 100).unwrap();
        f.store.discard_agent(waiting.id, 900).unwrap();

        let expired: Vec<String> =
            f.store.expired_discards(500).unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(expired, vec!["Scribe"]);

        assert_eq!(f.store.expired_discards(900).unwrap().len(), 2);
    }

    #[test]
    fn a_discarded_agent_keeps_everything_a_restore_has_to_find() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Researcher")).unwrap();
        f.store.append_working_note(agent.id, "waiting on the vendor", 5).unwrap();
        f.store
            .replace_signins(
                agent.id,
                Surface::Computer,
                &[signin_at(agent.id, "Gmail", "mail.google.com", 1)],
            )
            .unwrap();

        f.store.discard_agent(agent.id, 1).unwrap();

        assert_eq!(f.store.working_notes(agent.id).unwrap().len(), 1);
        assert_eq!(f.store.agent_signins(agent.id).unwrap().len(), 1);
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
        assert!(!f.store.connector_env(mine.id).unwrap().contains("THEIRS"));
    }

    #[test]
    fn a_deleted_group_takes_its_credentials_with_it() {
        let f = fixture();
        let group = f.store.create_group(&group_named("Research")).unwrap();
        f.store.create_connector(&key_for(group.id, "TOKEN", "a")).unwrap();

        f.store.delete_group(group.id).expect("a group with credentials must still delete");
        assert!(f.store.group_connectors(group.id).unwrap().is_empty());
    }

    #[test]
    fn a_scan_replaces_what_a_machine_was_signed_in_to() {
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
        let conns: Vec<_> = (0..8).map(|_| f.store.conn().unwrap()).collect();
        for conn in &conns {
            let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
            assert_eq!(mode.to_lowercase(), "wal", "a connection came up outside WAL");

            let timeout: i64 = conn.query_row("PRAGMA busy_timeout", [], |r| r.get(0)).unwrap();
            assert!(timeout >= 5000, "busy timeout was not applied: {timeout}");
        }
    }

    #[test]
    fn deleting_everyone_keeps_other_crews_and_uses_a_surviving_group() {
        let f = fixture();
        let everyone = default_group_id();
        let mut groups = Vec::new();
        for name in ["Research", "Writing", "Planning"] {
            let group = f
                .store
                .create_group(&CleanGroup { name: name.into(), ..Default::default() })
                .unwrap();
            let mut d = draft(name);
            d.group_id = Some(group.id);
            f.store.create_agent(&d).unwrap();
            groups.push(group);
        }
        let gone = f.store.create_agent(&draft("Former resident")).unwrap();
        f.store.discard_agent(gone.id, now_ms()).unwrap();

        f.store.delete_group(everyone).expect("Everyone can go while another group remains");
        assert_eq!(f.store.list_groups().unwrap().len(), 3);
        for group in &groups {
            assert_eq!(f.store.get_group(group.id).unwrap().unwrap().agent_count, 1);
        }
        let fallback = groups[0].id;
        assert_eq!(f.store.get_agent(gone.id).unwrap().unwrap().group_id, fallback);
        assert_eq!(f.store.restore_agent(gone.id, &gone.name).unwrap().group_id, fallback);
        assert_eq!(f.store.create_agent(&draft("New resident")).unwrap().group_id, fallback);
        let batch = f.store.create_agents(&[draft("One"), draft("Two")]).unwrap();
        assert!(batch.iter().all(|card| card.group_id == fallback));
    }

    #[test]
    fn the_last_group_is_protected_even_after_everyone_is_deleted() {
        let f = fixture();
        let everyone = default_group_id();
        assert!(matches!(f.store.delete_group(everyone), Err(StoreError::CannotDeleteLastGroup)));
        let group = f
            .store
            .create_group(&CleanGroup { name: "Research".into(), ..Default::default() })
            .unwrap();
        f.store.delete_group(everyone).unwrap();
        assert!(matches!(
            f.store.group_for_removal(group.id),
            Err(StoreError::CannotDeleteLastGroup)
        ));
        assert!(matches!(f.store.delete_group(group.id), Err(StoreError::CannotDeleteLastGroup)));
        assert!(matches!(f.store.delete_group(everyone), Err(StoreError::GroupNotFound(_))));
        assert_eq!(f.store.list_groups().unwrap().len(), 1);
    }

    #[test]
    fn concurrent_deletions_leave_one_group() {
        let f = fixture();
        let group = f
            .store
            .create_group(&CleanGroup { name: "Research".into(), ..Default::default() })
            .unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                barrier.wait();
                f.store.delete_group(default_group_id())
            });
            let second = scope.spawn(|| {
                barrier.wait();
                f.store.delete_group(group.id)
            });
            let outcomes = [first.join().unwrap(), second.join().unwrap()];
            assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
            assert_eq!(
                outcomes
                    .iter()
                    .filter(|result| matches!(result, Err(StoreError::CannotDeleteLastGroup)))
                    .count(),
                1
            );
        });
        assert_eq!(f.store.list_groups().unwrap().len(), 1);
    }

    #[test]
    fn a_group_emptied_by_deleting_its_agents_can_then_be_deleted() {
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

        let orphans: Vec<_> =
            store.list_agents().unwrap().into_iter().filter(|c| c.group_id == group.id).collect();
        assert!(orphans.is_empty(), "no agent may point at a group that is gone");
    }

    #[test]
    fn a_calls_cached_tokens_are_written_beside_its_prompt() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        f.store
            .record_usage(&UsageEntry {
                agent_id: card.id,
                group_id: card.group_id,
                run_id: RunId::new(),
                model: "test/model".into(),
                prompt: 11264,
                completion: 80,
                cached: 9984,
                cost: None,
            })
            .unwrap();
        let conn = f.store.conn().unwrap();
        let (prompt, cached): (u32, u32) = conn
            .query_row("SELECT prompt, cached FROM usage", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((prompt, cached), (11264, 9984));
    }

    #[test]
    fn usage_is_written_and_summed_against_the_real_schema() {
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
                cached: 0,
                cost: Some(0.25),
            })
            .unwrap();
        f.store
            .record_usage(&UsageEntry {
                agent_id: card.id,
                group_id: card.group_id,
                run_id: run,
                model: "local/model".into(),
                prompt: 10,
                completion: 5,
                cached: 0,
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

        assert!(f.store.usage_by_run(&[]).unwrap().is_empty());
    }

    #[test]
    fn editing_a_routine_leaves_its_next_firing_alone_unless_asked() {
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

        f.store.delete_routine(made.id).unwrap();
        assert!(matches!(
            f.store.update_routine(made.id, "", "anything", clock(Cadence::Once), Some(1), false),
            Err(StoreError::RoutineNotFound(_))
        ));
    }

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

        assert_eq!(moved.version, c.version, "a move is not an edit");
        assert_eq!(moved.updated_at, c.updated_at);
        assert_eq!(moved.rail_order, 0);

        assert_eq!(
            arrangement(&f).into_iter().map(|(_, order)| order).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn a_move_with_nothing_to_land_in_front_of_goes_to_the_end_of_that_group() {
        let f = fixture();
        let research = f.store.create_group(&group_named("Research")).unwrap();

        let scout = f.store.create_agent(&draft("Scout")).unwrap();
        let mut second = draft("Reader");
        second.group_id = Some(research.id);
        f.store.create_agent(&second).unwrap();
        let cook = f.store.create_agent(&draft("Cook")).unwrap();

        let moved = f.store.move_agent(cook.id, research.id, None).unwrap();
        assert_eq!(moved.group_id, research.id, "one call moved it and placed it");
        assert_eq!(
            arrangement(&f).into_iter().map(|(name, _)| name).collect::<Vec<_>>(),
            vec!["Scout", "Reader", "Cook"]
        );

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

    fn column(f: &Fixture) -> Vec<String> {
        f.store.list_groups().unwrap().into_iter().map(|g| g.name).collect()
    }

    #[test]
    fn moving_a_crew_that_is_not_there_is_an_error_rather_than_a_silent_renumber() {
        let f = fixture();
        let research = f.store.create_group(&group_named("Research")).unwrap();
        assert!(matches!(
            f.store.move_group(GroupId::new(), Some(research.id)),
            Err(StoreError::GroupNotFound(_))
        ));
        assert_eq!(column(&f), vec!["Everyone", "Research"]);
    }

    #[test]
    fn a_crew_dropped_on_itself_stays_where_it_is() {
        let f = fixture();
        let research = f.store.create_group(&group_named("Research")).unwrap();
        f.store.create_group(&group_named("Kitchen")).unwrap();

        let drawn = f.store.move_group(research.id, Some(research.id)).unwrap();
        assert_eq!(drawn.len(), 3, "answered with every crew, not an empty list");
        assert_eq!(column(&f), vec!["Everyone", "Research", "Kitchen"]);
    }

    #[test]
    fn a_move_with_nothing_to_land_in_front_of_goes_to_the_end_of_the_column() {
        let f = fixture();
        let research = f.store.create_group(&group_named("Research")).unwrap();
        let kitchen = f.store.create_group(&group_named("Kitchen")).unwrap();

        f.store.move_group(default_group_id(), None).unwrap();
        assert_eq!(column(&f), vec!["Research", "Kitchen", "Everyone"]);

        f.store.delete_group(kitchen.id).unwrap();
        f.store.move_group(research.id, Some(kitchen.id)).unwrap();
        assert_eq!(column(&f), vec!["Everyone", "Research"]);
    }

    #[test]
    fn a_move_puts_a_crew_in_front_of_the_one_it_was_dropped_on() {
        let f = fixture();
        f.store.create_group(&group_named("Research")).unwrap();
        let kitchen = f.store.create_group(&group_named("Kitchen")).unwrap();

        let drawn = f.store.move_group(kitchen.id, Some(default_group_id())).unwrap();
        assert_eq!(
            drawn.into_iter().map(|g| g.name).collect::<Vec<_>>(),
            vec!["Kitchen", "Everyone", "Research"],
            "the answer is the order that now stands"
        );
        assert_eq!(column(&f), vec!["Kitchen", "Everyone", "Research"]);

        let conn = f.store.conn().unwrap();
        let slots: Vec<i32> = conn
            .prepare("SELECT rail_order FROM groups ORDER BY rail_order")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(slots, vec![0, 1, 2]);
    }

    #[test]
    fn crews_arrive_at_the_bottom_of_the_column_whatever_was_arranged_above_them() {
        let f = fixture();
        let research = f.store.create_group(&group_named("Research")).unwrap();
        f.store.move_group(research.id, Some(default_group_id())).unwrap();
        f.store.create_group(&group_named("Kitchen")).unwrap();
        assert_eq!(column(&f), vec!["Research", "Everyone", "Kitchen"]);
    }

    #[test]
    fn an_agent_nobody_placed_joins_the_crew_at_the_top_of_the_column() {
        let f = fixture();
        let research = f.store.create_group(&group_named("Research")).unwrap();
        f.store.move_group(research.id, Some(default_group_id())).unwrap();

        let card = f.store.create_agent(&draft("Scout")).unwrap();
        assert_eq!(card.group_id, research.id);
    }

    #[test]
    fn pinning_moves_a_row_on_screen_and_nothing_a_peer_can_see() {
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

        let on = f.store.set_routine_active(made.id, true).unwrap();
        assert_eq!(on.next_run_at, made.next_run_at);
        assert_eq!(f.store.due_routines(2_000).unwrap().len(), 1);
    }

    #[test]
    fn upcoming_runs_leave_out_a_routine_that_is_switched_off() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        let off = f
            .store
            .create_routine(card.id, "Off", "nothing", clock(Cadence::Daily), Some(1_000), false)
            .unwrap();
        f.store.set_routine_active(off.id, false).unwrap();

        assert!(f.store.upcoming_runs(i64::MAX).unwrap().is_empty());
    }

    #[test]
    fn upcoming_runs_leave_out_a_routine_waiting_on_an_event() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        let paid = Trigger::parse("event:stripe/invoice.paid").unwrap();
        f.store.create_routine(card.id, "Thank them", "say thanks", paid, None, false).unwrap();

        assert!(f.store.upcoming_runs(i64::MAX).unwrap().is_empty());
    }

    #[test]
    fn upcoming_runs_leave_out_the_routines_of_a_deleted_agent() {
        let f = fixture();
        let gone = f.store.create_agent(&draft("Gone")).unwrap();
        f.store
            .create_routine(gone.id, "Sweep", "check", clock(Cadence::Daily), Some(1_000), false)
            .unwrap();
        f.store.discard_agent(gone.id, 500).unwrap();

        assert!(f.store.upcoming_runs(i64::MAX).unwrap().is_empty());
    }

    #[test]
    fn upcoming_runs_stop_at_the_moment_asked_for() {
        let f = fixture();
        let card = f.store.create_agent(&draft("Scout")).unwrap();
        let at_edge = f
            .store
            .create_routine(card.id, "Edge", "check", clock(Cadence::Daily), Some(2_000), false)
            .unwrap();
        f.store
            .create_routine(card.id, "Later", "check", clock(Cadence::Daily), Some(2_001), false)
            .unwrap();

        let upcoming = f.store.upcoming_runs(2_000).unwrap();
        let ids: Vec<_> = upcoming.iter().map(|run| run.routine_id).collect();
        assert_eq!(ids, vec![at_edge.id], "the edge itself is included");
    }

    #[test]
    fn upcoming_runs_come_soonest_first_across_agents_then_by_name() {
        let f = fixture();
        let scout = f.store.create_agent(&draft("Scout")).unwrap();
        let clerk = f.store.create_agent(&draft("Clerk")).unwrap();
        f.store
            .create_routine(scout.id, "Late", "check", clock(Cadence::Daily), Some(3_000), false)
            .unwrap();
        f.store
            .create_routine(clerk.id, "Beta", "check", clock(Cadence::Daily), Some(2_000), false)
            .unwrap();
        f.store
            .create_routine(scout.id, "Alpha", "check", clock(Cadence::Once), Some(2_000), false)
            .unwrap();
        f.store
            .create_routine(clerk.id, "Early", "check", clock(Cadence::Daily), Some(1_000), false)
            .unwrap();

        let upcoming = f.store.upcoming_runs(i64::MAX).unwrap();
        let order: Vec<_> =
            upcoming.iter().map(|run| (run.name.as_str(), run.next_run_at)).collect();
        assert_eq!(
            order,
            vec![("Early", 1_000), ("Alpha", 2_000), ("Beta", 2_000), ("Late", 3_000)]
        );
        assert_eq!(upcoming[0].agent_id, clerk.id);
        assert_eq!(upcoming[0].group_id, clerk.group_id);
        assert_eq!(upcoming[1].agent_id, scout.id);
    }

    #[test]
    fn a_history_says_which_firings_were_tests() {
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

        f.store.routine_ran(&made, 5_000).unwrap();
        let after = f.store.get_routine(made.id).unwrap().unwrap();
        assert_eq!(after.trigger, trigger, "the trigger survives the round trip");
        assert_eq!(after.next_run_at, None, "still holding no slot");
        assert_eq!(after.last_run_at, Some(5_000), "and it recorded having run");
    }

    #[test]
    fn an_event_finds_every_live_routine_standing_on_it_and_nothing_else() {
        let f = fixture();
        let scout = f.store.create_agent(&draft("Scout")).unwrap();
        let clerk = f.store.create_agent(&draft("Clerk")).unwrap();
        let paused = f.store.create_agent(&draft("Paused")).unwrap();
        let paid =
            |service: &str| Trigger::parse(&format!("event:{service}/invoice.paid")).unwrap();

        let standing = f
            .store
            .create_routine(scout.id, "Thank them", "say thanks", paid("stripe"), None, false)
            .unwrap();
        let also = f
            .store
            .create_routine(clerk.id, "Book it", "post the ledger", paid("Stripe"), None, false)
            .unwrap();
        let off = f
            .store
            .create_routine(scout.id, "Off", "nothing", paid("stripe"), None, false)
            .unwrap();
        f.store.set_routine_active(off.id, false).unwrap();
        f.store
            .create_routine(paused.id, "Asleep", "nothing", paid("stripe"), None, false)
            .unwrap();
        f.store.set_lifecycle(paused.id, Lifecycle::Paused).unwrap();
        f.store
            .create_routine(
                scout.id,
                "Other topic",
                "nothing",
                Trigger::parse("event:stripe/invoice.voided").unwrap(),
                None,
                false,
            )
            .unwrap();

        let event = EventTrigger::parse("STRIPE/invoice.paid").unwrap();
        let found: Vec<_> =
            f.store.event_routines(&event).unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(found, vec![standing.id, also.id], "oldest first, live and on only");
    }

    #[test]
    fn a_schedule_lists_what_is_due_soonest_and_what_is_waiting_last() {
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
                    cached: 0,
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

        f.store.append_working_note(scholar.id, "waiting on the listings", 1_000).unwrap();
        f.store.append_working_note(bystander.id, "waiting on Robert", 1_000).unwrap();

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
                cached: 0,
                cost: Some(0.01),
            })
            .unwrap();

        assert_eq!(f.store.delete_group_messages(other.id).unwrap(), 1);
        assert_eq!(f.store.delete_group_routines(other.id).unwrap(), 1);
        assert_eq!(f.store.delete_group_working_notes(other.id).unwrap(), 1);
        assert_eq!(f.store.delete_group_usage(other.id).unwrap(), 1);
        assert!(f.store.agent_routines(scholar.id).unwrap().is_empty());
        assert!(
            f.store.working_notes(scholar.id).unwrap().is_empty(),
            "a reset crew still waiting on the conversation it can no longer read"
        );
        assert_eq!(
            f.store.working_notes(bystander.id).unwrap().len(),
            1,
            "clearing one group must not empty another crew's notes"
        );
        assert!(
            !f.store.usage_by_group().unwrap().contains_key(&other.id),
            "a reset group has spent nothing, or the meter keeps counting what is gone"
        );
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
        let f = fixture();
        let default = default_group_id();
        let mut d = draft("Resident");
        d.group_id = Some(default);
        f.store.create_agent(&d).unwrap();

        assert!(matches!(
            f.store.group_for_removal(default),
            Err(StoreError::CannotDeleteLastGroup)
        ));

        let group = f
            .store
            .create_group(&CleanGroup { name: "Research".into(), ..Default::default() })
            .unwrap();
        let mut d = draft("Scholar");
        d.group_id = Some(group.id);
        f.store.create_agent(&d).unwrap();
        assert!(f.store.group_for_removal(default).is_ok());
        assert!(f.store.group_for_removal(group.id).is_ok());
        assert!(
            matches!(f.store.delete_group(group.id), Err(StoreError::GroupNotEmpty { .. })),
            "the emptiness check has to stay on delete_group"
        );
    }

    #[test]
    fn a_disband_takes_the_live_crew_and_not_the_agents_already_deleted() {
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

        let bystander = f.store.create_agent(&draft("Bystander")).unwrap();

        let crew = f.store.group_crew(group.id).unwrap();
        assert_eq!(crew.iter().map(|c| c.id).collect::<Vec<_>>(), vec![live.id]);
        assert!(!crew.iter().any(|c| c.id == bystander.id));

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
    fn reasoning_effort_persists_and_clears_independently_of_the_model() {
        use crate::domain::effort::ReasoningEffort as Effort;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("effort.db");
        let store = Store::open(&path).unwrap();
        let mut group_draft = CleanGroup {
            name: "Thinkers".into(),
            inference: Some(InferenceOverrides {
                reasoning_effort: Some(Effort::High),
                ..Default::default()
            }),
            ..Default::default()
        };
        let group = store.create_group(&group_draft).unwrap();
        let mut agent_draft = draft("Thinker");
        agent_draft.group_id = Some(group.id);
        agent_draft.model.clear();
        agent_draft.reasoning_effort = Some(Effort::Low);
        let agent = store.create_agent(&agent_draft).unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.get_agent(agent.id).unwrap().unwrap().reasoning_effort, Some(Effort::Low));
        assert_eq!(
            store.get_group(group.id).unwrap().unwrap().inference.reasoning_effort,
            Some(Effort::High)
        );
        let app = crate::config::InferenceConfig {
            reasoning_effort: Effort::Medium,
            ..Default::default()
        };
        assert_eq!(
            store.group_inference(group.id).unwrap().apply(&app).reasoning_effort,
            Effort::High
        );
        agent_draft.reasoning_effort = None;
        assert_eq!(store.update_agent(agent.id, &agent_draft).unwrap().reasoning_effort, None);
        group_draft.inference.as_mut().unwrap().reasoning_effort = Some(Effort::Auto);
        store.update_group(group.id, &group_draft).unwrap();
        assert_eq!(
            store.group_inference(group.id).unwrap().apply(&app).reasoning_effort,
            Effort::Auto
        );
        group_draft.inference.as_mut().unwrap().reasoning_effort = None;
        store.update_group(group.id, &group_draft).unwrap();
        assert_eq!(
            store.group_inference(group.id).unwrap().apply(&app).reasoning_effort,
            Effort::Medium
        );
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
                    reasoning_effort: None,
                    request_timeout_secs: Some(600),
                }),
                api_key: Some(Some("sk-group-9999".into())),
                limits: Some(GroupLimits { max_steps_per_run: Some(9), ..Default::default() }),
                tidy: None,
            })
            .unwrap();

        assert_eq!(group.inference.provider, Some(crate::config::Provider::Compatible));
        assert_eq!(group.inference.base_url.as_deref(), Some("http://localhost:1234/v1"));
        assert_eq!(group.inference.subscription_model.as_deref(), Some("gpt-5.4"));
        assert_eq!(group.inference.request_timeout_secs, Some(600));
        assert_eq!(group.limits.max_steps_per_run, Some(9));
        assert_eq!(group.limits.max_hops, None, "an unset limit inherits");

        assert!(group.api_key_set);
        assert_eq!(group.api_key_hint, "...9999");
        let json = serde_json::to_string(&group).unwrap();
        assert!(!json.contains("sk-group"), "a group card leaked its key: {json}");
        assert_eq!(
            store.group_inference(group.id).unwrap().api_key.as_ref().map(Secret::expose),
            Some("sk-group-9999")
        );
        assert_eq!(store.group_limits(group.id).unwrap().max_steps_per_run, Some(9));
    }

    #[test]
    fn an_absent_block_leaves_that_half_of_a_groups_settings_alone() {
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
                tidy: None,
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
                    tidy: None,
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
        for card in &cards {
            assert_eq!(f.store.get_agent(card.id).unwrap().as_ref(), Some(card));
        }
    }

    #[test]
    fn a_hired_crew_lands_at_the_bottom_of_the_rail_in_the_order_it_was_picked() {
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

        let order: Vec<String> =
            f.store.list_agents().unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(
            order,
            ["Chief of Staff", "Executive Assistant", "Paralegal", "Bookkeeper", "QA Tester"]
        );
    }

    #[test]
    fn a_crew_that_cannot_be_written_in_full_is_not_written_at_all() {
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
        let f = fixture();
        for name in ["A", "B", "C", "D", "E"] {
            f.store.create_agent(&draft(name)).unwrap();
        }
        let names: Vec<String> =
            f.store.list_agents().unwrap().iter().map(|c| c.name.clone()).collect();
        assert_eq!(names, vec!["A", "B", "C", "D", "E"]);

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
    fn only_the_agent_the_operator_wrote_to_counts_as_addressed_on_that_run() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let chef = f.store.create_agent(&draft("Chef")).unwrap();
        let agent = |id| Participant::Agent { id };

        let run = RunId::new();
        f.store.append(&envelope(Participant::Human, agent(manager.id), "cost it", run)).unwrap();
        f.store.append(&envelope(agent(manager.id), agent(chef.id), "cost the soup", run)).unwrap();

        assert!(f.store.operator_addressed(run, manager.id).unwrap());
        assert!(
            !f.store.operator_addressed(run, chef.id).unwrap(),
            "the operator never wrote to Chef; Manager did, on their behalf"
        );

        let later = RunId::new();
        f.store.append(&envelope(Participant::Human, agent(chef.id), "directly", later)).unwrap();
        assert!(f.store.operator_addressed(later, chef.id).unwrap());
        assert!(!f.store.operator_addressed(later, manager.id).unwrap());
    }

    #[test]
    fn agent_history_keeps_its_own_answers_without_reading_another_agents_private_work() {
        let f = fixture();
        let writer = f.store.create_agent(&draft("Writer")).unwrap().id;
        let peer = f.store.create_agent(&draft("Peer")).unwrap().id;
        let other = f.store.create_agent(&draft("Other")).unwrap().id;
        let agent = |id| Participant::Agent { id };
        let run = RunId::new();
        let exchanges = [
            (agent(writer), Participant::Human, "Earlier report"),
            (agent(peer), agent(writer), "Queued question"),
            (agent(writer), agent(peer), "My previous answer"),
            (agent(writer), Participant::Human, "Work finished after the question queued"),
            (agent(peer), Participant::System, "Private working notes"),
            (agent(peer), agent(other), "Somebody else's conversation"),
            (agent(peer), agent(writer), "Later incoming message"),
        ];
        for (i, (from, to, text)) in exchanges.into_iter().enumerate() {
            let mut message = envelope(from, to, text, run);
            message.created_at = i as i64;
            f.store.append(&message).unwrap();
        }

        let history = f.store.agent_history(writer, 1, 40).unwrap();
        let texts: Vec<_> = history.iter().map(Envelope::plain_text).collect();
        assert_eq!(texts, exchanges[..4].iter().map(|(_, _, text)| *text).collect::<Vec<_>>());
        assert_eq!(history[2].to, agent(peer), "the outgoing answer keeps its recipient");
        assert_eq!(history.len(), 4, "a message in both windows is included only once");

        let newest = f.store.agent_history(writer, 1, 2).unwrap();
        assert_eq!(
            newest.iter().map(|message| message.id).collect::<Vec<_>>(),
            history[2..].iter().map(|message| message.id).collect::<Vec<_>>()
        );
        assert!(f.store.agent_history(writer, 1, 0).unwrap().is_empty());
    }

    #[test]
    fn agent_history_filters_a_backlog_before_applying_its_limit() {
        let f = fixture();
        let writer = f.store.create_agent(&draft("Writer")).unwrap().id;
        let run = RunId::new();
        for i in 0..100 {
            let mut message = envelope(
                Participant::Human,
                Participant::Agent { id: writer },
                &format!("message {i}"),
                run,
            );
            message.created_at = i;
            f.store.append(&message).unwrap();
        }
        let history = f.store.agent_history(writer, 1, 2).unwrap();
        assert_eq!(
            history.iter().map(Envelope::plain_text).collect::<Vec<_>>(),
            vec!["message 0", "message 1"],
            "newer queued messages must not displace the context this turn can read"
        );
    }

    #[test]
    fn a_turn_that_called_a_tool_leaves_the_history_window_to_messages() {
        let f = fixture();
        let writer = f.store.create_agent(&draft("Writer")).unwrap().id;
        let me = Participant::Agent { id: writer };
        let run = RunId::new();
        let mut at = 0;
        let mut file = |mut message: Envelope| {
            message.created_at = at;
            at += 1;
            f.store.append(&message).unwrap();
        };
        for i in 0..25 {
            file(envelope(Participant::Human, me, &format!("question {i}"), run));
            let mut record = envelope(me, Participant::System, "", run);
            record.parts = vec![Part::tool_call(
                "note_progress",
                serde_json::json!({ "note": format!("working on question {i}") }),
                ToolOutcome::Ok { summary: "Noted.".into() },
            )];
            file(record);
            file(envelope(me, Participant::Human, &format!("answer {i}"), run));
        }

        let history = f.store.agent_history(writer, i64::MAX, 40).unwrap();
        let drawn: Vec<String> =
            history.iter().map(Envelope::plain_text).filter(|text| !text.is_empty()).collect();
        assert_eq!(drawn.len(), 40, "the window is forty messages, not forty rows: {drawn:?}");
        assert_eq!(drawn.first().map(String::as_str), Some("question 5"));
        assert_eq!(drawn.last().map(String::as_str), Some("answer 24"));
        assert_eq!(history.len(), 40, "and nothing the prompt will not draw came back with them");
    }

    #[test]
    fn what_the_history_window_leaves_out_is_what_the_prompt_would_not_draw() {
        use crate::domain::decision::DecisionRequest;
        use crate::domain::ids::{DecisionId, RoutineId};

        let f = fixture();
        let writer = f.store.create_agent(&draft("Writer")).unwrap().id;
        let me = Participant::Agent { id: writer };
        let call = || {
            Part::tool_call(
                "directory",
                serde_json::json!({}),
                ToolOutcome::Ok { summary: "1 agent(s): Writer".into() },
            )
        };
        let samples: Vec<(Participant, Participant, Vec<Part>)> = vec![
            (Participant::Human, me, vec![Part::text("words")]),
            (Participant::Human, me, vec![Part::text(" \n\t ")]),
            (me, Participant::System, vec![call()]),
            (me, Participant::System, vec![call(), Part::text("Checked who is here.")]),
            (
                Participant::System,
                me,
                vec![Part::Notice { kind: NoticeKind::GuardStop, text: "hop limit".into() }],
            ),
            (
                Participant::Human,
                me,
                vec![Part::Json { name: "data".into(), value: serde_json::json!({ "a": 1 }) }],
            ),
            (
                Participant::Human,
                me,
                vec![Part::File(Attachment {
                    digest: "aaa".into(),
                    name: "budget.pdf".into(),
                    mime: "application/pdf".into(),
                    bytes: 2048,
                })],
            ),
            (
                Participant::System,
                me,
                vec![Part::Routine {
                    routine_id: RoutineId::new(),
                    name: "Morning".into(),
                    what: "post the summary".into(),
                    payload: None,
                }],
            ),
            (
                Participant::System,
                me,
                vec![Part::Decision {
                    id: DecisionId::new(),
                    request: DecisionRequest {
                        question: "Which footer address?".into(),
                        context: String::new(),
                        recommendation: "Use the office.".into(),
                        options: vec![],
                        source: String::new(),
                    },
                    answer: "Leave the address out.".into(),
                    resumed: false,
                }],
            ),
            (
                Participant::System,
                me,
                vec![Part::Tidy { reported: vec!["- done".into()], notes: vec![] }],
            ),
        ];
        for (i, (from, to, parts)) in samples.into_iter().enumerate() {
            let mut message = envelope(from, to, "", RunId::new());
            message.parts = parts;
            message.created_at = i as i64;
            f.store.append(&message).unwrap();
            let drawn = !message.is_tidy()
                && (!message.plain_text().is_empty()
                    || message.parts.iter().any(|part| matches!(part, Part::File(_))));
            let kept = f
                .store
                .agent_history(writer, i64::MAX, 40)
                .unwrap()
                .iter()
                .any(|seen| seen.id == message.id);
            assert_eq!(kept, drawn, "the window and the prompt disagree on {:?}", message.parts);
        }
    }

    fn tidying(f: &Fixture, name: &str) -> AgentCard {
        let group = f
            .store
            .create_group(&CleanGroup {
                name: format!("{name}'s group"),
                tidy: Some(crate::domain::group::Tidy { on: true, hour: 3 }),
                ..Default::default()
            })
            .unwrap();
        f.store.create_agent(&CleanDraft { group_id: Some(group.id), ..draft(name) }).unwrap()
    }

    fn ended(f: &Fixture, agent: &AgentCard, headline: &str, status: &str, at: i64) {
        f.store
            .conn()
            .unwrap()
            .execute(
                "INSERT INTO outcomes (run_id,agent_id,group_id,trigger,status,headline,
                                       started_at,finished_at)
                 VALUES (?1,?2,?3,'operator',?4,?5,?6,?6)",
                params![
                    RunId::new().to_string(),
                    agent.id.to_string(),
                    agent.group_id.to_string(),
                    status,
                    headline,
                    at
                ],
            )
            .unwrap();
    }

    fn night(after: i64, nights: i64) -> i64 {
        crate::domain::group::Tidy::last_struck(3, after + nights * 86_400_000).unwrap()
    }

    fn claim(f: &Fixture, agent: &AgentCard, struck: i64) -> Option<Vec<String>> {
        f.store
            .claim_tidy(agent.id, struck, struck + 60_000, |lines| {
                let mut tidy = envelope(
                    Participant::System,
                    Participant::Agent { id: agent.id },
                    "",
                    RunId::new(),
                );
                tidy.parts = vec![Part::Tidy {
                    reported: lines.into_iter().map(|(_, headline, _)| headline).collect(),
                    notes: vec![],
                }];
                tidy
            })
            .unwrap()
            .map(|tidy| match &tidy.parts[0] {
                Part::Tidy { reported, .. } => reported.clone(),
                _ => unreachable!(),
            })
    }

    #[test]
    fn a_tidy_is_handed_its_own_runs_that_said_something_newest_first_and_thirty_at_most() {
        let f = fixture();
        let me = tidying(&f, "Writer");
        assert!(f.store.get_group(me.group_id).unwrap().unwrap().tidy.on);
        let start = crate::domain::now_ms();

        ended(&f, &me, "before the switch", "done", start - 60_000);
        for i in 0..35 {
            ended(&f, &me, &format!("run {i}"), "done", start + 1_000 + i);
        }
        ended(&f, &me, "found nothing", "nothing_new", start + 2_000);
        ended(&f, &me, "failed", "failed", start + 3_000);
        let other = f
            .store
            .create_agent(&CleanDraft { group_id: Some(me.group_id), ..draft("Other") })
            .unwrap();
        ended(&f, &other, "someone else's", "done", start + 4_000);

        let handed = claim(&f, &me, night(start, 1)).expect("there was work since the switch");
        assert_eq!(handed.len(), TIDY_LINES as usize);
        assert_eq!(handed[0], "failed", "newest first, and a failure is a line like any other");
        assert_eq!(handed[1], "run 34");
        assert_eq!(handed.last().map(String::as_str), Some("run 6"));
        for left_out in ["before the switch", "found nothing", "someone else's"] {
            assert!(!handed.iter().any(|line| line == left_out), "{left_out}: {handed:?}");
        }
    }

    #[test]
    fn a_night_is_handed_over_once_and_a_night_with_nothing_is_seen_to_all_the_same() {
        let f = fixture();
        let me = tidying(&f, "Writer");
        let start = crate::domain::now_ms();
        let first = night(start, 1);

        assert_eq!(claim(&f, &me, first), None, "nothing finished since the switch");
        ended(&f, &me, "late", "done", first + 7 * 3_600_000);
        assert_eq!(claim(&f, &me, first), None);
        let second = night(start, 2);
        assert_eq!(claim(&f, &me, second), Some(vec!["late".to_string()]));
        assert_eq!(claim(&f, &me, second), None, "the same night is not handed over twice");
    }

    #[test]
    fn the_next_tidy_counts_from_the_last_one_that_was_read() {
        let f = fixture();
        let me = tidying(&f, "Writer");
        let start = crate::domain::now_ms();
        ended(&f, &me, "first week", "done", start + 1_000);

        assert_eq!(claim(&f, &me, night(start, 1)), Some(vec!["first week".to_string()]));
        assert_eq!(claim(&f, &me, night(start, 2)), Some(vec!["first week".to_string()]));
        f.store.tidied(me.id).unwrap();
        assert_eq!(claim(&f, &me, night(start, 3)), None, "read, and nothing since");
        ended(&f, &me, "second week", "done", night(start, 3) + 3_600_000);
        assert_eq!(claim(&f, &me, night(start, 4)), Some(vec!["second week".to_string()]));
    }

    #[test]
    fn nobody_is_tidied_with_the_switch_off_or_while_paused() {
        let f = fixture();
        let me = tidying(&f, "Writer");
        let start = crate::domain::now_ms();
        ended(&f, &me, "work", "done", start + 1_000);
        let first_on = f.store.tidy_due().unwrap()[0].since;
        assert!(first_on <= start);

        f.store.set_lifecycle(me.id, Lifecycle::Paused).unwrap();
        assert!(f.store.tidy_due().unwrap().is_empty());
        assert_eq!(claim(&f, &me, night(start, 1)), None);
        f.store.set_lifecycle(me.id, Lifecycle::Active).unwrap();

        let group = f.store.get_group(me.group_id).unwrap().unwrap();
        let switch = |on: bool| {
            f.store
                .update_group(
                    group.id,
                    &CleanGroup {
                        name: group.name.clone(),
                        tidy: Some(crate::domain::group::Tidy { on, hour: 5 }),
                        ..Default::default()
                    },
                )
                .unwrap()
        };
        assert_eq!(switch(false).tidy, crate::domain::group::Tidy { on: false, hour: 5 });
        assert!(f.store.tidy_due().unwrap().is_empty());
        assert_eq!(claim(&f, &me, night(start, 1)), None);
        let renamed = f
            .store
            .update_group(group.id, &CleanGroup { name: "Renamed".into(), ..Default::default() })
            .unwrap();
        assert_eq!(renamed.tidy, crate::domain::group::Tidy { on: false, hour: 5 });

        let back_on = crate::domain::now_ms();
        switch(true);
        let due = f.store.tidy_due().unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].hour, 5);
        assert!(due[0].since >= back_on, "{} counts from the first time on", due[0].since);
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
        send(agent(a.id), Participant::System, "tool trail");

        let flow: Vec<String> = f
            .store
            .conversation_flow(a.group_id, 50)
            .unwrap()
            .iter()
            .map(Envelope::plain_text)
            .collect();
        assert_eq!(flow, vec!["you asked", "peer msg", "answer"]);
        assert!(b.group_id == a.group_id, "both land in the default group");
    }

    #[test]
    fn the_flow_is_one_crew_and_not_the_workspace() {
        let f = fixture();
        let other = f.store.create_group(&group_named("Ops")).unwrap();
        let mine = f.store.create_agent(&draft("A")).unwrap();
        let theirs = f.store.create_agent(&draft_in("B", other.id)).unwrap();
        let run = RunId::new();

        let mut at = 1_000;
        let mut send = |to, text: &str| {
            let mut e = envelope(Participant::Human, to, text, run);
            e.created_at = at;
            at += 1;
            f.store.append(&e).unwrap();
        };
        send(Participant::Agent { id: mine.id }, "mine");
        send(Participant::Agent { id: theirs.id }, "theirs");

        let read = |group| -> Vec<String> {
            f.store.conversation_flow(group, 50).unwrap().iter().map(Envelope::plain_text).collect()
        };
        assert_eq!(read(mine.group_id), vec!["mine"]);
        assert_eq!(read(other.id), vec!["theirs"]);
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
        let flow: Vec<String> = f
            .store
            .conversation_flow(a.group_id, 50)
            .unwrap()
            .iter()
            .map(Envelope::plain_text)
            .collect();
        assert_eq!(flow, vec!["m0", "m1", "m2", "m3", "m4"]);
    }

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
    fn a_decision_answer_is_found_by_its_words_and_not_by_its_keys() {
        use crate::domain::decision::DecisionRequest;
        use crate::domain::ids::DecisionId;

        let s = searchable();
        let mut answer =
            envelope(Participant::System, Participant::Agent { id: s.writer }, "", RunId::new());
        answer.parts = vec![Part::Decision {
            id: DecisionId::new(),
            request: DecisionRequest {
                question: "Which footer address?".into(),
                context: String::new(),
                recommendation: "Use the office.".into(),
                options: vec![],
                source: String::new(),
            },
            answer: "Leave the address out.".into(),
            resumed: false,
        }];
        s.f.store.append(&answer).unwrap();

        for query in ["address out", "footer address"] {
            let hits = s.f.store.search(query, 20).unwrap();
            let found: Vec<MessageId> = hits.messages.iter().map(|m| m.id).collect();
            assert_eq!(found, vec![answer.id], "{query:?}");
        }
        for key in ["recommendation", "resumed", "question"] {
            assert!(s.f.store.search(key, 20).unwrap().messages.is_empty(), "{key:?}");
        }
    }

    #[test]
    fn a_json_key_is_not_a_word_anybody_wrote() {
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
        assert!(s.f.store.search("%", 20).unwrap().messages.is_empty());
        assert!(s.f.store.search("_", 20).unwrap().messages.is_empty());
    }

    #[test]
    fn a_file_is_found_by_name_and_appears_once_however_often_it_was_sent() {
        let s = searchable();
        let run = RunId::new();
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
        assert_eq!(urls, vec!["https://example.com/budget-q3"]);
        assert_eq!(hits.links[0].channel_id, s.writer);
    }

    #[test]
    fn a_routine_is_found_by_what_it_says_or_by_its_title() {
        let s = searchable();
        let hits = s.f.store.search("budget", 20).unwrap();
        assert_eq!(hits.routines.len(), 1);
        assert_eq!(hits.routines[0].what, "post the budget summary");

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
    fn every_terminal_approval_state_wins_against_every_later_settlement() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Manager")).unwrap();
        let states = [
            ApprovalState::Allow,
            ApprovalState::AlwaysAllow,
            ApprovalState::Deny,
            ApprovalState::Answered,
            ApprovalState::Expired,
        ];
        for first in states {
            for second in states {
                let request = asks(&f.store, &agent, &[]);
                let settled = if first == ApprovalState::Answered {
                    f.store.answer_approval(request.id, "first").unwrap()
                } else {
                    f.store.settle_approval(request.id, first).unwrap()
                };
                let result = if second == ApprovalState::Answered {
                    f.store.answer_approval(request.id, "second")
                } else {
                    f.store.settle_approval(request.id, second)
                };
                assert!(
                    matches!(result, Err(StoreError::ApprovalSettled { state }) if state == first)
                );
                let persisted = f.store.get_approval(request.id).unwrap().unwrap();
                assert_eq!(persisted.state, first);
                assert_eq!(persisted.answer, settled.answer);
                assert_eq!(persisted.decided_at, settled.decided_at);
            }
        }
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
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let written = asks(&f.store, &manager, &[]);

        f.store.answer_approval(written.id, "Northwind").unwrap();
        let refused = f.store.answer_approval(written.id, "Contoso");
        assert!(matches!(
            refused,
            Err(StoreError::ApprovalSettled { state: ApprovalState::Answered })
        ));
        assert_eq!(
            refused.unwrap_err().to_string(),
            "that question was already answered (answered)"
        );
        let missing = ApprovalId::new();
        assert_eq!(
            f.store.answer_approval(missing, "Contoso").unwrap_err().to_string(),
            format!("no question with id {missing}")
        );
        assert_eq!(
            f.store.get_approval(written.id).unwrap().unwrap().answer.as_deref(),
            Some("Northwind")
        );
    }

    #[test]
    fn a_question_is_never_a_standing_grant_and_never_shows_up_as_one() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        asks(&f.store, &manager, &["Northwind"]);

        assert!(f.store.standing_grants(manager.id).unwrap().is_empty());
        assert!(!f.store.has_standing_grant(manager.id, ProtectedAction::CreateAgent).unwrap());
        assert!(!f.store.has_standing_grant(manager.id, ProtectedAction::ActOnBehalf).unwrap());
    }

    #[test]
    fn a_question_waits_in_the_same_queue_a_permission_does() {
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
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();

        let first = ask(&f.store, &manager);
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
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let request = ask(&f.store, &manager);
        f.store.settle_approval(request.id, ApprovalState::AlwaysAllow).unwrap();

        assert_eq!(f.store.delete_agent_approvals(manager.id).unwrap(), 1);
        assert!(!f.store.has_standing_grant(manager.id, ProtectedAction::CreateAgent).unwrap());
        assert!(f.store.get_approval(request.id).unwrap().is_none());
    }

    fn occasion_at(store: &Store, group: GroupId, title: &str, starts_at: i64) -> Occasion {
        let clean = CleanOccasion::new(
            group,
            None,
            title,
            "",
            "",
            crate::domain::occasion::When { starts_at, all_day: false },
            Some(30),
        )
        .unwrap();
        store.create_occasion(&clean).unwrap()
    }

    #[test]
    fn an_occasion_is_written_and_read_back_whole() {
        let f = fixture();
        let crew = f.store.create_group(&group_named("Ops")).unwrap();
        let agent = f.store.create_agent(&draft_in("Manager", crew.id)).unwrap();

        let clean = CleanOccasion::new(
            crew.id,
            Some(agent.id),
            "Board call",
            "Bring the Q3 numbers",
            "Zoom",
            crate::domain::occasion::When { starts_at: 1_800_000_000_000, all_day: false },
            Some(60),
        )
        .unwrap();
        let written = f.store.create_occasion(&clean).unwrap();

        let back = f.store.occasion(written.id, crew.id).unwrap().unwrap();
        assert_eq!(back, written);
        assert_eq!(back.agent_id, Some(agent.id));
        assert_eq!(back.detail, "Bring the Q3 numbers");
        assert_eq!(back.place, "Zoom");
        assert_eq!(back.minutes, Some(60));
        assert!(!back.all_day);
    }

    #[test]
    fn a_whole_day_reads_back_as_one_rather_than_as_midnight() {
        let f = fixture();
        let crew = f.store.create_group(&group_named("Finance")).unwrap();
        let when = crate::domain::occasion::parse_when("2026-04-15").unwrap();
        let clean = CleanOccasion::new(crew.id, None, "Filing due", "", "", when, None).unwrap();

        let back = f.store.create_occasion(&clean).unwrap();
        assert!(back.all_day);
        assert_eq!(back.minutes, None);
    }

    #[test]
    fn one_crew_cannot_move_another_crews_meeting() {
        let f = fixture();
        let ours = f.store.create_group(&group_named("Ops")).unwrap();
        let theirs = f.store.create_group(&group_named("Legal")).unwrap();

        let mine = occasion_at(&f.store, theirs.id, "Deposition", 1_800_000_000_000);

        let moved = CleanOccasion::new(
            theirs.id,
            None,
            "Deposition",
            "",
            "",
            crate::domain::occasion::When { starts_at: 1_900_000_000_000, all_day: false },
            None,
        )
        .unwrap();

        assert_eq!(f.store.update_occasion(mine.id, ours.id, &moved).unwrap(), None);
        assert!(!f.store.delete_occasion(mine.id, ours.id).unwrap());

        let untouched = f.store.occasion(mine.id, theirs.id).unwrap().unwrap();
        assert_eq!(untouched.starts_at, mine.starts_at);
    }

    #[test]
    fn a_crew_cannot_even_read_another_crews_calendar() {
        let f = fixture();
        let ours = f.store.create_group(&group_named("Ops")).unwrap();
        let theirs = f.store.create_group(&group_named("Legal")).unwrap();
        let hidden = occasion_at(&f.store, theirs.id, "Deposition", 1_800_000_000_000);

        assert_eq!(f.store.occasion(hidden.id, ours.id).unwrap(), None);
        assert!(f.store.crew_calendar(ours.id, 0, 50).unwrap().is_empty());
    }

    #[test]
    fn the_operators_view_crosses_every_crew_and_a_filtered_one_does_not() {
        let f = fixture();
        let ops = f.store.create_group(&group_named("Ops")).unwrap();
        let legal = f.store.create_group(&group_named("Legal")).unwrap();
        occasion_at(&f.store, ops.id, "Board call", 1_800_000_000_000);
        occasion_at(&f.store, legal.id, "Deposition", 1_800_100_000_000);

        let every = f.store.occasions(0, i64::MAX, None, 50).unwrap();
        assert_eq!(every.len(), 2);
        assert_eq!(every[0].title, "Board call", "soonest first");

        let one = f.store.occasions(0, i64::MAX, Some(legal.id), 50).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].title, "Deposition");
    }

    #[test]
    fn a_window_is_half_open_so_two_of_them_neither_drop_nor_double() {
        let f = fixture();
        let crew = f.store.create_group(&group_named("Ops")).unwrap();
        let edge = 1_800_000_000_000;
        occasion_at(&f.store, crew.id, "On the boundary", edge);

        assert!(f.store.occasions(0, edge, None, 50).unwrap().is_empty());
        assert_eq!(f.store.occasions(edge, edge + 1, None, 50).unwrap().len(), 1);
    }

    #[test]
    fn an_agents_own_read_starts_where_it_is_asked_to_and_looks_no_further_back() {
        let f = fixture();
        let crew = f.store.create_group(&group_named("Ops")).unwrap();
        occasion_at(&f.store, crew.id, "Last month", 1_700_000_000_000);
        occasion_at(&f.store, crew.id, "Next week", 1_800_000_000_000);

        let ahead = f.store.crew_calendar(crew.id, 1_750_000_000_000, 50).unwrap();
        assert_eq!(ahead.len(), 1);
        assert_eq!(ahead[0].title, "Next week");
    }

    #[test]
    fn an_occasion_outlives_the_agent_that_noticed_it() {
        let f = fixture();
        let crew = f.store.create_group(&group_named("Ops")).unwrap();
        let agent = f.store.create_agent(&draft_in("Manager", crew.id)).unwrap();

        let clean = CleanOccasion::new(
            crew.id,
            Some(agent.id),
            "Board call",
            "",
            "",
            crate::domain::occasion::When { starts_at: 1_800_000_000_000, all_day: false },
            None,
        )
        .unwrap();
        let written = f.store.create_occasion(&clean).unwrap();

        f.store.set_lifecycle(agent.id, Lifecycle::Terminated).unwrap();

        let kept = f.store.occasion(written.id, crew.id).unwrap().unwrap();
        assert_eq!(kept.title, "Board call");
        assert_eq!(kept.agent_id, Some(agent.id), "who noticed it is still worth drawing");
    }

    #[test]
    fn disbanding_a_crew_takes_its_calendar_with_it() {
        let f = fixture();
        let crew = f.store.create_group(&group_named("Ops")).unwrap();
        occasion_at(&f.store, crew.id, "Board call", 1_800_000_000_000);

        f.store.delete_group(crew.id).unwrap();
        assert!(f.store.occasions(0, i64::MAX, None, 50).unwrap().is_empty());
    }

    fn stuck(store: &Store, agent: &AgentCard, summary: &str, at: i64) -> Raised {
        store.raise_escalation(agent.id, agent.group_id, RunId::new(), summary, at).unwrap()
    }

    #[test]
    fn an_escalation_goes_up_once_and_is_read_back_whole() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let raised = stuck(&f.store, &manager, "the deploy needs a key only you have", 1_000);

        assert!(matches!(raised, Raised::First(_)));
        let open = f.store.open_escalation_for(manager.id).unwrap().unwrap();
        assert_eq!(open.summary, "the deploy needs a key only you have");
        assert_eq!(open.raised_at, 1_000);
        assert_eq!(open.times, 1);
        assert_eq!(open.cleared_at, None);
    }

    #[test]
    fn raising_it_again_counts_the_turn_and_leaves_the_stamp_where_it_is() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        stuck(&f.store, &manager, "the tooling is down", 1_000);
        stuck(&f.store, &manager, "the tooling is down", 2_000);
        let third = stuck(&f.store, &manager, "the tooling is still down", 3_000);

        let Raised::Again(one) = third else { panic!("a second raise is not a second row") };
        assert_eq!(one.raised_at, 1_000, "the age is the news and must not move");
        assert_eq!(one.said_at, 3_000, "and when it was last said is a second question");
        assert_eq!(one.times, 3);
        assert_eq!(one.summary, "the tooling is still down", "the latest wording stands");
        assert_eq!(one.run_id, f.store.open_escalation_for(manager.id).unwrap().unwrap().run_id);
        assert_eq!(f.store.open_escalations(50).unwrap().len(), 1);
    }

    #[test]
    fn two_agents_stuck_are_two_rows() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let critic = f.store.create_agent(&draft("Critic")).unwrap();
        stuck(&f.store, &manager, "no key", 1_000);
        stuck(&f.store, &critic, "no machine", 2_000);

        let open = f.store.open_escalations(50).unwrap();
        assert_eq!(open.len(), 2);
        assert_eq!(open[0].agent_id, manager.id, "oldest first: the age is what is being read");
    }

    #[test]
    fn clearing_one_takes_it_off_the_desk_and_frees_the_agent_to_raise_another() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let Raised::First(one) = stuck(&f.store, &manager, "no key", 1_000) else {
            panic!("the first raise is the first row")
        };

        assert!(f.store.clear_escalation(one.id, 5_000).unwrap());
        assert!(f.store.open_escalations(50).unwrap().is_empty());
        assert!(f.store.open_escalation_for(manager.id).unwrap().is_none());

        let next = stuck(&f.store, &manager, "no machine either", 9_000);
        assert!(matches!(next, Raised::First(_)));
        assert_eq!(f.store.open_escalation_for(manager.id).unwrap().unwrap().raised_at, 9_000);
    }

    #[test]
    fn clearing_one_twice_changes_nothing_the_second_time() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let Raised::First(one) = stuck(&f.store, &manager, "no key", 1_000) else {
            panic!("the first raise is the first row")
        };

        assert!(f.store.clear_escalation(one.id, 5_000).unwrap());
        assert!(!f.store.clear_escalation(one.id, 8_000).unwrap(), "already off the desk");
    }

    #[test]
    fn a_composted_agents_escalation_is_not_on_the_desk() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        stuck(&f.store, &manager, "no key", 1_000);
        f.store.discard_agent(manager.id, 2_000).unwrap();

        assert!(f.store.open_escalations(50).unwrap().is_empty());
    }

    #[test]
    fn a_discarded_agents_escalation_is_cleared_rather_than_left_open() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        stuck(&f.store, &manager, "no key", 1_000);

        assert_eq!(f.store.clear_agent_escalations(manager.id, 2_000).unwrap(), 1);
        assert!(f.store.open_escalation_for(manager.id).unwrap().is_none());
        assert_eq!(
            f.store.clear_agent_escalations(manager.id, 3_000).unwrap(),
            0,
            "and there is nothing left to clear"
        );
    }

    #[test]
    fn expiring_approvals_leaves_escalations_open_and_clearing_them_leaves_decisions() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Manager")).unwrap();
        let request = ask(&f.store, &agent);
        let Raised::First(escalation) = stuck(&f.store, &agent, "blocked", 1_000) else {
            panic!("first escalation");
        };
        assert_eq!(f.store.expire_pending_approvals().unwrap(), 1);
        assert_eq!(f.store.open_escalation_for(agent.id).unwrap().unwrap().id, escalation.id);
        assert!(f.store.clear_escalation(escalation.id, 2_000).unwrap());
        assert_eq!(
            f.store.get_approval(request.id).unwrap().unwrap().state,
            ApprovalState::Expired
        );
    }

    #[test]
    fn an_escalation_is_not_an_approval_and_shares_no_queue_with_one() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        ask(&f.store, &manager);
        stuck(&f.store, &manager, "no key", 1_000);

        assert_eq!(f.store.pending_approvals(50).unwrap().len(), 1);
        assert_eq!(f.store.open_escalations(50).unwrap().len(), 1);
    }

    #[test]
    fn concurrent_raises_from_one_agent_are_one_row() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();

        std::thread::scope(|scope| {
            for i in 0..8 {
                let store = f.store.clone();
                let card = manager.clone();
                scope.spawn(move || {
                    store
                        .raise_escalation(
                            card.id,
                            card.group_id,
                            RunId::new(),
                            &format!("stuck {i}"),
                            1_000 + i,
                        )
                        .expect("a raise must never be refused by the index");
                });
            }
        });

        let open = f.store.open_escalations(50).unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].times, 8);
    }

    #[test]
    fn a_standing_grant_can_be_taken_back() {
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

        assert_eq!(f.store.revoke_grant(manager.id, ProtectedAction::CreateAgent).unwrap(), 0);
    }

    #[test]
    fn revoking_leaves_answered_requests_alone() {
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
    fn the_answers_read_carries_what_was_said_and_only_what_was_said() {
        let f = fixture();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let question = asks(&f.store, &manager, &["Northwind", "Contoso"]);
        let once = ask(&f.store, &manager);
        let lapsed = asks(&f.store, &manager, &[]);

        assert!(
            f.store.approval_answers(500).unwrap().is_empty(),
            "nothing has been said to anything yet"
        );

        f.store.answer_approval(question.id, "Contoso").unwrap();
        f.store.settle_approval(once.id, ApprovalState::Allow).unwrap();
        f.store.settle_approval(lapsed.id, ApprovalState::Expired).unwrap();

        let answers = f.store.approval_answers(500).unwrap();
        assert_eq!(answers.get(&question.id).map(String::as_str), Some("Contoso"));
        assert_eq!(answers.get(&once.id), None, "a verdict is a state, not a value");
        assert_eq!(answers.get(&lapsed.id), None, "nobody said anything to the one that lapsed");
        assert_eq!(answers.len(), 1);
    }

    #[test]
    fn a_plugin_row_naming_something_this_build_has_never_heard_of_is_skipped() {
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
        let f = fixture();
        let group = f
            .store
            .create_group(&CleanGroup { name: "Crew".into(), ..Default::default() })
            .unwrap();
        f.store
            .save_plugin(group.id, &PluginKind::Neon, "", &[], None, "", &Headers::none())
            .unwrap();

        f.store.delete_group(group.id).unwrap();
        assert!(f.store.group_plugins(group.id).unwrap().is_empty());
        assert!(matches!(
            f.store.plugin_reach(group.id, AgentId::new(), &PluginKind::Neon, "run_sql").unwrap(),
            PluginReach::NotConnected
        ));
    }

    #[test]
    fn a_server_the_operator_added_keeps_its_address_and_a_catalog_one_does_not() {
        let f = fixture();
        let group = default_group_id();
        let mine = PluginKind::custom("Home Assistant", "https://ha.example.com/mcp").unwrap();

        f.store
            .save_plugin(group, &mine, "", &[tool("run_sql")], None, "", &Headers::none())
            .unwrap();
        f.store
            .save_plugin(
                group,
                &PluginKind::Neon,
                "",
                &[tool("run_sql")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();

        let held = f.store.group_plugins(group).unwrap();
        let added = held.iter().find(|held| held.custom).expect("the added one comes back");
        assert_eq!(added.kind, mine);
        assert_eq!(added.name, "home_assistant");
        assert_eq!(added.endpoint, "https://ha.example.com/mcp");

        let vendor = held.iter().find(|held| !held.custom).unwrap();
        assert_eq!(vendor.endpoint, PluginKind::Neon.endpoint());
        let stored: String = f
            .store
            .conn()
            .unwrap()
            .query_row("SELECT endpoint FROM plugins WHERE kind='neon'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(stored, "", "a catalog row's address belongs to the build, not to the row");
    }

    #[test]
    fn a_row_this_build_cannot_dial_is_skipped_rather_than_failing_every_turn() {
        let f = fixture();
        let group = default_group_id();
        f.store
            .save_plugin(
                group,
                &PluginKind::Neon,
                "",
                &[tool("run_sql")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();
        f.store
            .conn()
            .unwrap()
            .execute(
                "INSERT INTO plugins (id,group_id,kind,account,tools,connected_at,endpoint)
                 VALUES ('p9',?1,'from-the-future','','[]',0,'')",
                params![group.to_string()],
            )
            .unwrap();

        let agent = f.store.create_agent(&draft("Manager")).unwrap();
        let held = f.store.group_plugins(group).unwrap();
        assert_eq!(held.len(), 1, "the unreadable row is dropped and the readable one is not");
        assert_eq!(f.store.plugin_tools(group, agent.id).unwrap().len(), 1);
    }

    #[test]
    fn headers_go_out_with_the_grant_and_come_back_as_names_alone() {
        let f = fixture();
        let group = default_group_id();
        let agent = f.store.create_agent(&draft("Manager")).unwrap();
        let kind = PluginKind::custom("Home Assistant", "https://ha.example.com/mcp").unwrap();
        let headers =
            Headers::parse(&[HeaderPair { name: "X-API-Key".into(), value: "hand-minted".into() }])
                .unwrap();

        let saved =
            f.store.save_plugin(group, &kind, "", &[tool("turn_on")], None, "", &headers).unwrap();
        assert_eq!(saved.headers, vec!["x-api-key".to_string()]);

        let PluginReach::Granted(reached) =
            f.store.plugin_reach(group, agent.id, &kind, "turn_on").unwrap()
        else {
            panic!("the crew can call it")
        };
        assert_eq!(reached.headers, headers, "the call path gets the values");

        let dialed = f.store.plugin_dial(saved.id).unwrap().expect("the row it was written as");
        assert_eq!(dialed.headers, headers);
        assert_eq!(dialed.kind, kind);

        let again =
            f.store.save_plugin(group, &kind, "", &[tool("turn_on")], None, "", &Headers::none());
        assert!(again.unwrap().headers.is_empty());
    }

    #[test]
    fn what_a_tool_was_published_with_comes_back_with_the_grant_that_calls_it() {
        let f = fixture();
        let group = default_group_id();
        let agent = f.store.create_agent(&draft("Manager")).unwrap();
        f.store
            .save_plugin(
                group,
                &PluginKind::Neon,
                "",
                &[PluginTool {
                    name: "run_sql".into(),
                    description: "Run a query.".into(),
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": { "region": { "x-mcp-header": "Region" } },
                    }),
                }],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();

        let PluginReach::Granted(reached) =
            f.store.plugin_reach(group, agent.id, &PluginKind::Neon, "run_sql").unwrap()
        else {
            panic!("the crew can call it")
        };
        assert_eq!(reached.schema.unwrap()["properties"]["region"]["x-mcp-header"], "Region");

        let error = f.store.plugin_reach(group, agent.id, &PluginKind::Neon, "gone").unwrap_err();
        assert_eq!(
            error.to_string(),
            "Neon does not offer a tool called gone. Use one of the Neon tools in your tool list."
        );
        assert_eq!(
            crate::plugins::PluginError::from(error).to_string(),
            "Neon does not offer a tool called gone. Use one of the Neon tools in your tool list."
        );
    }

    #[test]
    fn discovery_precedes_policy_and_reconnection_replaces_the_callable_set() {
        let f = fixture();
        let group = default_group_id();
        let first = f.store.create_agent(&draft("First")).unwrap();
        let peer = f.store.create_agent(&draft("Peer")).unwrap();
        let save = |tools: &[PluginTool]| {
            f.store
                .save_plugin(group, &PluginKind::Neon, "", tools, None, "", &Headers::none())
                .unwrap()
        };
        let plugin = save(&[tool("offered"), tool("removed"), tool("disabled"), tool("peer")]);
        f.store.set_plugin_tool(plugin.id, "disabled", &nobody()).unwrap();
        f.store
            .set_plugin_tool(plugin.id, "peer", &PluginAccess::Chosen { agents: vec![peer.id] })
            .unwrap();
        assert!(matches!(
            f.store.plugin_reach(group, first.id, &PluginKind::Neon, "offered").unwrap(),
            PluginReach::Granted(_)
        ));
        assert!(matches!(
            f.store.plugin_reach(group, first.id, &PluginKind::Neon, "disabled").unwrap(),
            PluginReach::ToolDenied
        ));
        assert!(matches!(
            f.store.plugin_reach(group, first.id, &PluginKind::Neon, "peer").unwrap(),
            PluginReach::ToolNotChosen
        ));
        assert!(f.store.plugin_reach(group, first.id, &PluginKind::Neon, "invented").is_err());
        for name in ["", "run sql"] {
            assert!(f.store.plugin_reach(group, first.id, &PluginKind::Neon, name).is_err());
        }
        save(&[tool("offered"), tool("disabled"), tool("peer"), tool("new")]);
        assert!(f.store.plugin_reach(group, first.id, &PluginKind::Neon, "removed").is_err());
        assert!(matches!(
            f.store.plugin_reach(group, first.id, &PluginKind::Neon, "new").unwrap(),
            PluginReach::Granted(_)
        ));
        assert!(matches!(
            f.store.plugin_reach(group, first.id, &PluginKind::Neon, "disabled").unwrap(),
            PluginReach::ToolDenied
        ));
        assert!(matches!(
            f.store.plugin_reach(group, first.id, &PluginKind::Neon, "peer").unwrap(),
            PluginReach::ToolNotChosen
        ));
        save(&[tool("removed"), tool("disabled")]);
        f.store.set_plugin_tool(plugin.id, "removed", &nobody()).unwrap();
        save(&[tool("disabled")]);
        assert!(f.store.plugin_reach(group, first.id, &PluginKind::Neon, "removed").is_err());
        save(&[tool("removed"), tool("disabled")]);
        assert!(matches!(
            f.store.plugin_reach(group, first.id, &PluginKind::Neon, "removed").unwrap(),
            PluginReach::ToolDenied
        ));
        f.store
            .conn()
            .unwrap()
            .execute("UPDATE plugins SET tools='not json' WHERE id=?1", [plugin.id.to_string()])
            .unwrap();
        let error =
            f.store.plugin_reach(group, first.id, &PluginKind::Neon, "disabled").unwrap_err();
        assert_eq!(
            crate::plugins::PluginError::from(error).to_string(),
            "Guaca could not use its own records: the Neon tool list is unreadable. Ask the operator to reconnect Neon."
        );
    }

    #[test]
    fn a_plugin_that_needed_no_sign_in_reports_no_grant_rather_than_an_empty_one() {
        let f = fixture();
        let group = default_group_id();
        f.store
            .save_plugin(
                group,
                &PluginKind::Neon,
                "",
                &[tool("run_sql")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();

        let agent = f.store.create_agent(&draft("Manager")).unwrap();
        let PluginReach::Granted(reached) =
            f.store.plugin_reach(group, agent.id, &PluginKind::Neon, "run_sql").unwrap()
        else {
            panic!("a connected plugin is reachable by the crew it was connected for")
        };
        assert!(reached.grant.is_none());
        assert!(!f.store.group_plugins(group).unwrap()[0].signed_in);
    }

    #[test]
    fn a_plugin_is_the_whole_crew_s_until_somebody_says_otherwise() {
        let f = fixture();
        let group = default_group_id();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        f.store
            .save_plugin(
                group,
                &PluginKind::Neon,
                "",
                &[tool("run_sql")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();

        assert_eq!(f.store.group_plugins(group).unwrap()[0].access, PluginAccess::Everyone);
        assert_eq!(f.store.plugin_tools(group, manager.id).unwrap().len(), 1);

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
            .save_plugin(
                group,
                &PluginKind::Stripe,
                "",
                &[tool("refund")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();

        let saved = f
            .store
            .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![revenue.id] })
            .unwrap();
        assert_eq!(saved.access, PluginAccess::Chosen { agents: vec![revenue.id] });

        assert_eq!(f.store.plugin_tools(group, revenue.id).unwrap().len(), 1);
        assert!(f.store.plugin_tools(group, scribe.id).unwrap().is_empty());

        assert!(matches!(
            f.store.plugin_reach(group, revenue.id, &PluginKind::Stripe, "refund").unwrap(),
            PluginReach::Granted(_)
        ));
        assert!(matches!(
            f.store.plugin_reach(group, scribe.id, &PluginKind::Stripe, "refund").unwrap(),
            PluginReach::NotChosen
        ));
    }

    #[test]
    fn a_plugin_narrowed_to_nobody_is_a_plugin_nobody_gets() {
        let f = fixture();
        let group = default_group_id();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let plugin = f
            .store
            .save_plugin(
                group,
                &PluginKind::Stripe,
                "",
                &[tool("refund")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();

        f.store.set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![] }).unwrap();

        assert!(f.store.plugin_tools(group, manager.id).unwrap().is_empty());
        assert!(matches!(
            f.store.plugin_reach(group, manager.id, &PluginKind::Stripe, "refund").unwrap(),
            PluginReach::NotChosen
        ));
    }

    #[test]
    fn opening_a_plugin_back_up_leaves_nothing_behind_that_still_names_anybody() {
        let f = fixture();
        let group = default_group_id();
        let revenue = f.store.create_agent(&draft("Revenue")).unwrap();
        let plugin = f
            .store
            .save_plugin(group, &PluginKind::Stripe, "", &[], None, "", &Headers::none())
            .unwrap();

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
        let f = fixture();
        let other = f
            .store
            .create_group(&CleanGroup { name: "Elsewhere".into(), ..Default::default() })
            .unwrap();
        let outsider = f
            .store
            .create_agent(&CleanDraft { group_id: Some(other.id), ..draft("Outsider") })
            .unwrap();
        let plugin = f
            .store
            .save_plugin(
                default_group_id(),
                &PluginKind::Stripe,
                "",
                &[],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();

        let refused = f
            .store
            .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![outsider.id] })
            .unwrap_err();
        assert!(matches!(refused, StoreError::AgentNotInGroup(id) if id == outsider.id));
    }

    #[test]
    fn connecting_again_does_not_hand_a_narrowed_plugin_back_to_the_crew() {
        let f = fixture();
        let group = default_group_id();
        let revenue = f.store.create_agent(&draft("Revenue")).unwrap();
        let scribe = f.store.create_agent(&draft("Scribe")).unwrap();
        let plugin = f
            .store
            .save_plugin(
                group,
                &PluginKind::Stripe,
                "",
                &[tool("refund")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();
        f.store
            .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![revenue.id] })
            .unwrap();

        let again = f
            .store
            .save_plugin(
                group,
                &PluginKind::Stripe,
                "",
                &[tool("refund")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();

        assert_eq!(again.access, PluginAccess::Chosen { agents: vec![revenue.id] });
        assert!(f.store.plugin_tools(group, scribe.id).unwrap().is_empty());
    }

    #[test]
    fn retiring_an_agent_takes_its_place_on_a_plugin_with_it() {
        let f = fixture();
        let group = default_group_id();
        let revenue = f.store.create_agent(&draft("Revenue")).unwrap();
        let plugin = f
            .store
            .save_plugin(group, &PluginKind::Stripe, "", &[], None, "", &Headers::none())
            .unwrap();
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
        let f = fixture();
        let group = default_group_id();
        let revenue = f.store.create_agent(&draft("Revenue")).unwrap();
        let plugin = f
            .store
            .save_plugin(group, &PluginKind::Stripe, "", &[], None, "", &Headers::none())
            .unwrap();
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
        let f = fixture();
        let group = default_group_id();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        f.store
            .save_plugin(
                group,
                &PluginKind::Neon,
                "",
                &[tool("run_sql"), tool("drop_project")],
                None,
                "",
                &Headers::none(),
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
        let f = fixture();
        let group = default_group_id();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let plugin = f
            .store
            .save_plugin(
                group,
                &PluginKind::Neon,
                "",
                &[tool("run_sql"), tool("drop_project")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();

        let saved = f.store.set_plugin_tool(plugin.id, "drop_project", &nobody()).unwrap();
        assert_eq!(
            saved.tools.iter().find(|tool| tool.name == "drop_project").map(|tool| &tool.access),
            Some(&nobody()),
            "the panel draws what was stored"
        );

        let offered = f.store.plugin_tools(group, manager.id).unwrap().remove(0);
        assert_eq!(
            offered.offered.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["run_sql"]
        );
        assert_eq!(offered.withheld, vec!["drop_project".to_string()]);

        assert!(matches!(
            f.store.plugin_reach(group, manager.id, &PluginKind::Neon, "drop_project").unwrap(),
            PluginReach::ToolDenied
        ));
        assert!(matches!(
            f.store.plugin_reach(group, manager.id, &PluginKind::Neon, "run_sql").unwrap(),
            PluginReach::Granted(_)
        ));
    }

    #[test]
    fn a_tool_switched_off_is_off_for_the_agent_the_plugin_was_narrowed_to() {
        let f = fixture();
        let group = default_group_id();
        let revenue = f.store.create_agent(&draft("Revenue")).unwrap();
        let scribe = f.store.create_agent(&draft("Scribe")).unwrap();
        let plugin = f
            .store
            .save_plugin(
                group,
                &PluginKind::Stripe,
                "",
                &[tool("charges"), tool("refund")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();

        f.store
            .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![revenue.id] })
            .unwrap();
        f.store.set_plugin_tool(plugin.id, "refund", &nobody()).unwrap();

        assert!(matches!(
            f.store.plugin_reach(group, revenue.id, &PluginKind::Stripe, "refund").unwrap(),
            PluginReach::ToolDenied
        ));
        assert!(matches!(
            f.store.plugin_reach(group, scribe.id, &PluginKind::Stripe, "refund").unwrap(),
            PluginReach::ToolDenied,
        ));
        assert!(matches!(
            f.store.plugin_reach(group, scribe.id, &PluginKind::Stripe, "charges").unwrap(),
            PluginReach::NotChosen
        ));
    }

    #[test]
    fn two_agents_on_one_plugin_get_different_tools() {
        let f = fixture();
        let group = default_group_id();
        let triage = f.store.create_agent(&draft("Triage")).unwrap();
        let reply = f.store.create_agent(&draft("Reply")).unwrap();
        let plugin = f
            .store
            .save_plugin(
                group,
                &PluginKind::Agentmail,
                "",
                &[tool("read_thread"), tool("send")],
                None,
                "",
                &Headers::none(),
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

        let told = f.store.plugin_tools(group, triage.id).unwrap().remove(0);
        assert_eq!(
            told.offered.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["read_thread"]
        );
        assert_eq!(told.elsewhere, vec!["send".to_string()]);
        assert!(told.withheld.is_empty(), "somebody has it, so nobody is the wrong answer");

        assert!(matches!(
            f.store.plugin_reach(group, triage.id, &PluginKind::Agentmail, "send").unwrap(),
            PluginReach::ToolNotChosen
        ));
        assert!(matches!(
            f.store.plugin_reach(group, reply.id, &PluginKind::Agentmail, "send").unwrap(),
            PluginReach::Granted(_)
        ));
        assert!(matches!(
            f.store.plugin_reach(group, reply.id, &PluginKind::Agentmail, "read_thread").unwrap(),
            PluginReach::ToolNotChosen
        ));
    }

    #[test]
    fn a_tool_nobody_has_is_refused_before_one_somebody_else_has() {
        let f = fixture();
        let group = default_group_id();
        let one = f.store.create_agent(&draft("One")).unwrap();
        let two = f.store.create_agent(&draft("Two")).unwrap();
        let plugin = f
            .store
            .save_plugin(
                group,
                &PluginKind::Stripe,
                "",
                &[tool("refund"), tool("charges")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();

        f.store.set_plugin_tool(plugin.id, "refund", &nobody()).unwrap();
        f.store
            .set_plugin_tool(plugin.id, "charges", &PluginAccess::Chosen { agents: vec![two.id] })
            .unwrap();

        assert!(matches!(
            f.store.plugin_reach(group, one.id, &PluginKind::Stripe, "refund").unwrap(),
            PluginReach::ToolDenied
        ));
        assert!(matches!(
            f.store.plugin_reach(group, one.id, &PluginKind::Stripe, "charges").unwrap(),
            PluginReach::ToolNotChosen
        ));

        f.store
            .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![two.id] })
            .unwrap();
        assert!(matches!(
            f.store.plugin_reach(group, one.id, &PluginKind::Stripe, "refund").unwrap(),
            PluginReach::ToolDenied
        ));
        assert!(matches!(
            f.store.plugin_reach(group, one.id, &PluginKind::Stripe, "charges").unwrap(),
            PluginReach::NotChosen
        ));
    }

    #[test]
    fn a_tool_narrowed_to_an_agent_the_plugin_does_not_reach_grants_nothing() {
        let f = fixture();
        let group = default_group_id();
        let inside = f.store.create_agent(&draft("Inside")).unwrap();
        let outside = f.store.create_agent(&draft("Outside")).unwrap();
        let plugin = f
            .store
            .save_plugin(
                group,
                &PluginKind::Neon,
                "",
                &[tool("run_sql")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();

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
            f.store.plugin_reach(group, outside.id, &PluginKind::Neon, "run_sql").unwrap(),
            PluginReach::NotChosen
        ));
        assert!(matches!(
            f.store.plugin_reach(group, inside.id, &PluginKind::Neon, "run_sql").unwrap(),
            PluginReach::ToolNotChosen
        ));
        assert!(f.store.plugin_tools(group, inside.id).unwrap()[0].offered.is_empty());
    }

    #[test]
    fn a_tool_may_not_be_narrowed_to_an_agent_from_another_crew() {
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
            .save_plugin(
                default_group_id(),
                &PluginKind::Neon,
                "",
                &[tool("run_sql")],
                None,
                "",
                &Headers::none(),
            )
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
        let f = fixture();
        let group = default_group_id();
        let gone = f.store.create_agent(&draft("Gone")).unwrap();
        let plugin = f
            .store
            .save_plugin(
                group,
                &PluginKind::Neon,
                "",
                &[tool("run_sql")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();
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
        let f = fixture();
        let group = default_group_id();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let plugin = f
            .store
            .save_plugin(
                group,
                &PluginKind::Neon,
                "",
                &[tool("run_sql")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();

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
        let f = fixture();
        let plugin = f
            .store
            .save_plugin(
                default_group_id(),
                &PluginKind::Neon,
                "",
                &[tool("run_sql")],
                None,
                "",
                &Headers::none(),
            )
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
        let f = fixture();
        let group = default_group_id();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let published = [tool("run_sql"), tool("drop_project")];
        let plugin = f
            .store
            .save_plugin(group, &PluginKind::Neon, "", &published, None, "", &Headers::none())
            .unwrap();
        f.store.set_plugin_tool(plugin.id, "drop_project", &nobody()).unwrap();

        let again = f
            .store
            .save_plugin(
                group,
                &PluginKind::Neon,
                "",
                &[tool("run_sql"), tool("drop_project"), tool("list_branches")],
                None,
                "",
                &Headers::none(),
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
        let f = fixture();
        let group = default_group_id();
        let plugin = f
            .store
            .save_plugin(
                group,
                &PluginKind::Neon,
                "",
                &[tool("run_sql")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();
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
            .save_plugin(
                group.id,
                &PluginKind::Neon,
                "",
                &[tool("run_sql")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();
        let hand = f
            .store
            .create_agent(&CleanDraft { group_id: Some(group.id), ..draft("Hand") })
            .unwrap();
        f.store
            .set_plugin_tool(plugin.id, "run_sql", &PluginAccess::Chosen { agents: vec![hand.id] })
            .unwrap();
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
        let f = fixture();
        let group = default_group_id();
        let manager = f.store.create_agent(&draft("Manager")).unwrap();
        let plugin = f
            .store
            .save_plugin(
                group,
                &PluginKind::Neon,
                "",
                &[tool("run_sql")],
                None,
                "",
                &Headers::none(),
            )
            .unwrap();
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
            f.store.plugin_reach(group, manager.id, &PluginKind::Neon, "run_sql").unwrap(),
            PluginReach::NotChosen
        ));
        assert_eq!(
            f.store.group_plugins(group).unwrap()[0].access,
            PluginAccess::Chosen { agents: vec![] }
        );
    }

    #[test]
    fn working_notes_come_back_oldest_first() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Manager")).unwrap();
        for (i, body) in
            ["asked the paralegal", "handed over the scope", "waiting on Robert"].iter().enumerate()
        {
            f.store.append_working_note(agent.id, body, 1_000 + i as i64).unwrap();
        }

        let bodies: Vec<String> =
            f.store.working_notes(agent.id).unwrap().into_iter().map(|n| n.body).collect();
        assert_eq!(bodies, ["asked the paralegal", "handed over the scope", "waiting on Robert"]);
    }

    #[test]
    fn the_oldest_working_notes_fall_off_once_there_are_too_many() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Manager")).unwrap();
        for i in 0..(KEPT + 6) {
            f.store.append_working_note(agent.id, &format!("note {i}"), 1_000 + i as i64).unwrap();
        }

        let notes = f.store.working_notes(agent.id).unwrap();
        assert_eq!(notes.len(), KEPT, "the bound is not being enforced");
        assert_eq!(notes.first().unwrap().body, "note 6", "the wrong end fell off");
        assert_eq!(notes.last().unwrap().body, format!("note {}", KEPT + 5));
    }

    #[test]
    fn notes_written_in_the_same_millisecond_keep_the_order_they_were_written() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Manager")).unwrap();
        for i in 0..(KEPT + 2) {
            f.store.append_working_note(agent.id, &format!("note {i}"), 7).unwrap();
        }

        let bodies: Vec<String> =
            f.store.working_notes(agent.id).unwrap().into_iter().map(|n| n.body).collect();
        assert_eq!(bodies.first().unwrap(), "note 2");
        assert_eq!(bodies.last().unwrap(), &format!("note {}", KEPT + 1));
    }

    #[test]
    fn a_line_the_agent_already_holds_is_not_stored_again() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Manager")).unwrap();
        assert_eq!(
            f.store.append_working_note(agent.id, "waiting on Robert", 1_000).unwrap(),
            Appended::Stored
        );
        assert_eq!(
            f.store.append_working_note(agent.id, "waiting on Robert", 9_000).unwrap(),
            Appended::AlreadyHeld { at: 1_000 },
            "the repeat has to come back with the stamp of the note that already says it"
        );

        let notes = f.store.working_notes(agent.id).unwrap();
        assert_eq!(notes.len(), 1, "the repeat was stored anyway");
        assert_eq!(
            notes[0].at, 1_000,
            "the stamp moved forward, which is the age hiding exactly when it matters"
        );
    }

    #[test]
    fn one_agents_note_does_not_block_anothers() {
        let f = fixture();
        let a = f.store.create_agent(&draft("Manager")).unwrap();
        let b = f.store.create_agent(&draft("Chef")).unwrap();
        f.store.append_working_note(a.id, "waiting on the vendor", 1_000).unwrap();

        assert_eq!(
            f.store.append_working_note(b.id, "waiting on the vendor", 1_000).unwrap(),
            Appended::Stored
        );
    }

    #[test]
    fn a_note_that_has_already_fallen_off_the_end_can_be_written_again() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Manager")).unwrap();
        f.store.append_working_note(agent.id, "the first thing", 1).unwrap();
        for i in 0..KEPT {
            f.store.append_working_note(agent.id, &format!("note {i}"), 1_000 + i as i64).unwrap();
        }

        assert_eq!(
            f.store.append_working_note(agent.id, "the first thing", 9_000).unwrap(),
            Appended::Stored
        );
    }

    #[test]
    fn one_agents_notes_are_not_anothers() {
        let f = fixture();
        let a = f.store.create_agent(&draft("Manager")).unwrap();
        let b = f.store.create_agent(&draft("Chef")).unwrap();
        f.store.append_working_note(a.id, "mine", 1).unwrap();
        f.store.append_working_note(b.id, "also mine", 1).unwrap();

        assert_eq!(f.store.working_notes(a.id).unwrap().len(), 1);
        assert_eq!(f.store.working_notes(b.id).unwrap()[0].body, "also mine");
    }

    #[test]
    fn trimming_one_agent_leaves_every_other_agents_notes_alone() {
        let f = fixture();
        let busy = f.store.create_agent(&draft("Manager")).unwrap();
        let quiet = f.store.create_agent(&draft("Chef")).unwrap();
        f.store.append_working_note(quiet.id, "still here", 1).unwrap();
        for i in 0..(KEPT * 2) {
            f.store.append_working_note(busy.id, &format!("note {i}"), 1_000 + i as i64).unwrap();
        }

        assert_eq!(f.store.working_notes(quiet.id).unwrap().len(), 1);
        assert_eq!(f.store.working_notes(busy.id).unwrap().len(), KEPT);
    }

    #[test]
    fn clearing_takes_every_note_and_leaves_the_agent() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Manager")).unwrap();
        f.store.append_working_note(agent.id, "waiting on Robert", 1).unwrap();

        assert_eq!(f.store.clear_working_notes(agent.id).unwrap(), 1);
        assert!(f.store.working_notes(agent.id).unwrap().is_empty());
        assert!(f.store.get_agent(agent.id).unwrap().is_some(), "the agent went with its notes");
    }

    #[test]
    fn an_agent_with_no_notes_reads_as_empty_rather_than_an_error() {
        let f = fixture();
        let agent = f.store.create_agent(&draft("Manager")).unwrap();
        assert!(f.store.working_notes(agent.id).unwrap().is_empty());
    }

    #[test]
    fn a_work_message_with_no_answer_is_what_the_agent_is_waiting_on() {
        let f = fixture();
        let group = f.store.create_group(&group_named("Platform")).unwrap();
        let pm = f.store.create_agent(&draft_in("Product", group.id)).unwrap();
        let sre = f.store.create_agent(&draft_in("SRE", group.id)).unwrap();

        f.store.append(&work(pm.id, sre.id, "Ship the readiness package by Friday.")).unwrap();

        let waiting = f.store.outstanding_asks(pm.id).unwrap();
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].peer, "SRE");
        assert!(waiting[0].asked.contains("readiness package"));

        assert!(f.store.outstanding_asks(sre.id).unwrap().is_empty());
    }

    #[test]
    fn anything_back_from_the_peer_closes_it() {
        let f = fixture();
        let group = f.store.create_group(&group_named("Platform")).unwrap();
        let pm = f.store.create_agent(&draft_in("Product", group.id)).unwrap();
        let sre = f.store.create_agent(&draft_in("SRE", group.id)).unwrap();

        f.store.append(&work(pm.id, sre.id, "Ship it.")).unwrap();
        f.store.append(&work(sre.id, pm.id, "Shipped.")).unwrap();

        assert!(f.store.outstanding_asks(pm.id).unwrap().is_empty());
    }

    #[test]
    fn asking_again_after_an_answer_is_outstanding_again() {
        let f = fixture();
        let group = f.store.create_group(&group_named("Platform")).unwrap();
        let pm = f.store.create_agent(&draft_in("Product", group.id)).unwrap();
        let sre = f.store.create_agent(&draft_in("SRE", group.id)).unwrap();

        f.store.append(&work(pm.id, sre.id, "First.")).unwrap();
        f.store.append(&work(sre.id, pm.id, "Done.")).unwrap();
        f.store.append(&work(pm.id, sre.id, "Second.")).unwrap();

        let waiting = f.store.outstanding_asks(pm.id).unwrap();
        assert_eq!(waiting.len(), 1);
        assert!(waiting[0].asked.contains("Second"), "the newest ask is the live one");
    }

    #[test]
    fn a_courtesy_is_not_something_anybody_is_waiting_for() {
        let f = fixture();
        let group = f.store.create_group(&group_named("Platform")).unwrap();
        let pm = f.store.create_agent(&draft_in("Product", group.id)).unwrap();
        let sre = f.store.create_agent(&draft_in("SRE", group.id)).unwrap();

        let mut thanks = work(pm.id, sre.id, "Thanks, received.");
        thanks.intent = Intent::Courtesy;
        f.store.append(&thanks).unwrap();

        assert!(f.store.outstanding_asks(pm.id).unwrap().is_empty());
    }

    #[test]
    fn a_retired_peer_drops_off_the_list() {
        let f = fixture();
        let group = f.store.create_group(&group_named("Platform")).unwrap();
        let pm = f.store.create_agent(&draft_in("Product", group.id)).unwrap();
        let sre = f.store.create_agent(&draft_in("SRE", group.id)).unwrap();

        f.store.append(&work(pm.id, sre.id, "Ship it.")).unwrap();
        f.store.set_lifecycle(sre.id, Lifecycle::Terminated).unwrap();

        assert!(f.store.outstanding_asks(pm.id).unwrap().is_empty());
    }

    #[test]
    fn the_line_says_how_long_rather_than_when() {
        let now = now_ms();
        let one = Outstanding {
            peer: "SRE".into(),
            at: now - 2 * 3_600 * 1000,
            asked: "Ship the readiness package".into(),
        };
        let line = one.line(now);
        assert!(line.contains("SRE"), "{line}");
        assert!(line.contains("2 hours ago"), "{line}");
        assert!(line.contains("readiness package"), "{line}");
    }

    #[test]
    fn a_long_ask_is_cut_to_something_recognisable() {
        let now = now_ms();
        let one = Outstanding { peer: "SRE".into(), at: now, asked: "x".repeat(400) };
        assert!(one.line(now).chars().count() < 200, "the list is on every turn");
        assert!(one.line(now).ends_with('…'));
    }
}
