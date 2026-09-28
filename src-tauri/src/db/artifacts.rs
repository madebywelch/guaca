//! Artifacts in the store: the row, its versions and its log.
//!
//! Every agent-facing call takes the crew and matches on it in the WHERE
//! clause, exactly as the calendar does, so an id from another crew comes back
//! as nothing rather than as a refusal. The calls that take no crew are the
//! operator's, who stands above that wall.
//!
//! Every write is one transaction that appends one row to `artifact_history`
//! and, where it made a version, moves the row's copy of the version with it.
//! Nothing else writes either table, which is what keeps the two in step.

use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};

use super::{Store, StoreError};
use crate::domain::artifact::{Actor, Artifact, Change, Entry, Owner, Source};
use crate::domain::ids::{AgentId, ArtifactId, GroupId};
use crate::domain::now_ms;

/// The list's read: the row, the owner as it is now, and who made the current
/// version. `gone` is an owner deleted or moved out of the crew, which the list
/// has to say rather than hide: ownership only moves by a decision.
const COLUMNS: &str = "SELECT r.id, r.group_id, r.owner_id, o.name,
        (o.lifecycle = 'terminated' OR o.group_id <> r.group_id),
        r.title, r.version, h.agent_id, h.actor, r.created_at, r.updated_at,
        h.sources, r.allowed_sources, h.condensed IS NOT NULL
      FROM artifacts r
      LEFT JOIN agents o ON o.id = r.owner_id
      JOIN artifact_history h
        ON h.artifact_id = r.id AND h.version = r.version AND h.page IS NOT NULL";

const LOG_COLUMNS: &str =
    "SELECT seq, at, change, agent_id, actor, version, owner, note FROM artifact_history";

fn corrupt(what: &str, raw: &str, err: impl std::fmt::Display) -> StoreError {
    StoreError::Corrupt(format!("bad {what} {raw:?}: {err}"))
}

fn actor(id: Option<String>, name: String) -> Result<Actor, StoreError> {
    match id {
        None => Ok(Actor::Operator),
        Some(raw) => {
            let id = raw.parse::<AgentId>().map_err(|e| corrupt("agent id", &raw, e))?;
            Ok(Actor::Agent { id, name })
        }
    }
}

type Read<T> = Result<Result<T, StoreError>, rusqlite::Error>;

fn row_to_artifact(row: &Row<'_>) -> Read<Artifact> {
    let id: String = row.get(0)?;
    let group: String = row.get(1)?;
    let owner_id: Option<String> = row.get(2)?;
    let owner_name: Option<String> = row.get(3)?;
    let gone: Option<bool> = row.get(4)?;
    let title: String = row.get(5)?;
    let version: u32 = row.get(6)?;
    let editor_id: Option<String> = row.get(7)?;
    let editor_name: String = row.get(8)?;
    let created_at: i64 = row.get(9)?;
    let updated_at: i64 = row.get(10)?;
    let declared: String = row.get(11)?;
    let allowed: Option<String> = row.get(12)?;
    let condensed: bool = row.get(13)?;
    Ok((|| {
        let sources: Vec<Source> = serde_json::from_str(&declared)
            .map_err(|e| corrupt("artifact sources", &declared, e))?;
        // Compared as stored, which is what an approval is of: the list the
        // operator read. Nothing re-serializes either side.
        let sources_allowed = !sources.is_empty() && allowed.as_deref() == Some(declared.as_str());
        let owner = match (owner_id, owner_name) {
            (Some(raw), Some(name)) => Some(Owner {
                id: raw.parse().map_err(|e| corrupt("agent id", &raw, e))?,
                name,
                gone: gone.unwrap_or(false),
            }),
            _ => None,
        };
        Ok(Artifact {
            id: id.parse().map_err(|e| corrupt("artifact id", &id, e))?,
            group_id: group.parse().map_err(|e| corrupt("group id", &group, e))?,
            owner,
            title,
            version,
            edited_by: actor(editor_id, editor_name)?,
            created_at,
            updated_at,
            sources,
            sources_allowed,
            condensed,
        })
    })())
}

fn row_to_entry(row: &Row<'_>) -> Read<Entry> {
    let seq: u32 = row.get(0)?;
    let at: i64 = row.get(1)?;
    let change: String = row.get(2)?;
    let agent_id: Option<String> = row.get(3)?;
    let name: String = row.get(4)?;
    let version: u32 = row.get(5)?;
    let owner: Option<String> = row.get(6)?;
    let note: String = row.get(7)?;
    Ok((|| {
        Ok(Entry {
            seq,
            at,
            change: Change::parse(&change)
                .ok_or_else(|| corrupt("artifact change", &change, "unknown"))?,
            by: actor(agent_id, name)?,
            version,
            owner,
            note,
        })
    })())
}

/// One row of the log, about to be written.
struct Written<'a> {
    change: Change,
    by: &'a Actor,
    version: u32,
    owner: Option<&'a str>,
    note: &'a str,
    /// What the version is, on the rows that made one.
    version_of: Option<Version<'a>>,
}

/// One version's contents: the page, the reads it declares as stored, and the
/// condensed view when it has one.
struct Version<'a> {
    page: &'a str,
    sources: &'a str,
    condensed: Option<&'a str>,
}

/// A version as read back, owned.
struct Stored {
    page: String,
    sources: String,
    condensed: Option<String>,
}

impl Stored {
    fn as_version(&self) -> Version<'_> {
        Version { page: &self.page, sources: &self.sources, condensed: self.condensed.as_deref() }
    }
}

/// Appends one row to the log. The only insert into `artifact_history`.
fn append(
    tx: &Transaction<'_>,
    id: ArtifactId,
    at: i64,
    row: Written<'_>,
) -> Result<(), StoreError> {
    debug_assert_eq!(row.change.makes_version(), row.version_of.is_some());
    let (page, sources, condensed) = match row.version_of {
        Some(Version { page, sources, condensed }) => (Some(page), Some(sources), condensed),
        None => (None, None, None),
    };
    tx.execute(
        "INSERT INTO artifact_history
            (artifact_id, seq, at, change, agent_id, actor, version, owner, note, page, sources,
             condensed)
         VALUES (?1,
                 (SELECT COALESCE(MAX(seq), 0) + 1 FROM artifact_history WHERE artifact_id = ?1),
                 ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            id.to_string(),
            at,
            row.change.as_str(),
            row.by.agent_id().map(|agent| agent.to_string()),
            row.by.stored_name(),
            row.version,
            row.owner,
            row.note,
            page,
            sources,
            condensed,
        ],
    )?;
    Ok(())
}

/// Where a row stands before a write: its version, and its owner's name now.
/// `None` when the row is not there, or is another crew's.
fn standing(
    tx: &Transaction<'_>,
    id: ArtifactId,
    group: Option<GroupId>,
) -> Result<Option<(u32, Option<String>)>, StoreError> {
    Ok(tx
        .query_row(
            "SELECT r.version, o.name FROM artifacts r LEFT JOIN agents o ON o.id = r.owner_id
              WHERE r.id = ?1 AND (?2 IS NULL OR r.group_id = ?2)",
            params![id.to_string(), group.map(|g| g.to_string())],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

fn version_at(
    tx: &Transaction<'_>,
    id: ArtifactId,
    version: u32,
) -> Result<Option<Stored>, StoreError> {
    Ok(tx
        .query_row(
            "SELECT page, sources, condensed FROM artifact_history
              WHERE artifact_id = ?1 AND version = ?2 AND page IS NOT NULL",
            params![id.to_string(), version],
            |row| Ok(Stored { page: row.get(0)?, sources: row.get(1)?, condensed: row.get(2)? }),
        )
        .optional()?)
}

fn declared(sources: &[Source]) -> Result<String, StoreError> {
    serde_json::to_string(sources)
        .map_err(|e| StoreError::Corrupt(format!("artifact sources did not serialize: {e}")))
}

/// What a new page is made of.
pub struct ArtifactDraft<'a> {
    pub title: &'a str,
    pub page: &'a str,
    pub sources: &'a [Source],
    /// What the status bar draws, if the page is meant to be pinned there.
    pub condensed: Option<&'a str>,
    pub note: &'a str,
}

/// What an edit changes. `None` keeps what the current version has: a rename
/// keeps the page, a new page keeps its reads, and so keeps them allowed, and
/// either keeps the condensed view.
pub struct ArtifactRevision<'a> {
    pub title: Option<&'a str>,
    pub page: Option<&'a str>,
    pub sources: Option<&'a [Source]>,
    pub condensed: Option<&'a str>,
    pub note: &'a str,
}

impl Store {
    /// A new artifact at version 1, owned by the agent that made it.
    pub fn create_artifact(
        &self,
        group: GroupId,
        owner: AgentId,
        owner_name: &str,
        draft: ArtifactDraft<'_>,
    ) -> Result<Artifact, StoreError> {
        let ArtifactDraft { title, page, sources, condensed, note } = draft;
        let sources = declared(sources)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = ArtifactId::new();
        let now = now_ms();
        tx.execute(
            "INSERT INTO artifacts (id, group_id, owner_id, title, version, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5)",
            params![id.to_string(), group.to_string(), owner.to_string(), title, now],
        )?;
        let by = Actor::Agent { id: owner, name: owner_name.to_string() };
        append(
            &tx,
            id,
            now,
            Written {
                change: Change::Created,
                by: &by,
                version: 1,
                owner: Some(owner_name),
                note,
                version_of: Some(Version { page, sources: &sources, condensed }),
            },
        )?;
        tx.commit()?;
        self.any_artifact(id)?
            .ok_or_else(|| StoreError::Corrupt(format!("artifact {id} vanished as it was written")))
    }

    /// One of a crew's artifacts. Another crew's is `None`, not an error.
    pub fn artifact(&self, id: ArtifactId, group: GroupId) -> Result<Option<Artifact>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("{COLUMNS} WHERE r.id = ?1 AND r.group_id = ?2"))?;
        match stmt
            .query_row(params![id.to_string(), group.to_string()], row_to_artifact)
            .optional()?
        {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Any artifact, for the operator.
    pub fn any_artifact(&self, id: ArtifactId) -> Result<Option<Artifact>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("{COLUMNS} WHERE r.id = ?1"))?;
        match stmt.query_row(params![id.to_string()], row_to_artifact).optional()? {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// A crew's artifacts, or every crew's for `None`, most recently changed
    /// first.
    pub fn artifacts(
        &self,
        group: Option<GroupId>,
        limit: u32,
    ) -> Result<Vec<Artifact>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "{COLUMNS} WHERE (?1 IS NULL OR r.group_id = ?1)
              ORDER BY r.updated_at DESC, r.id ASC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![group.map(|g| g.to_string()), limit], row_to_artifact)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    /// Artifacts whose title matches a search, most recently changed first.
    ///
    /// By title only. The page is markup and script, and a search for "total"
    /// that found every page with a variable named `total` in it would bury the
    /// one whose title the operator half remembered.
    pub fn matching_artifacts(
        &self,
        pattern: &str,
        limit: u32,
    ) -> Result<Vec<Artifact>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "{COLUMNS} WHERE r.title LIKE ?1 ESCAPE '\\'
              ORDER BY r.updated_at DESC, r.id ASC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![pattern, limit], row_to_artifact)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    /// A new version, with whatever the revision leaves out kept as it was.
    ///
    /// Open to any agent in the crew, not only the owner, and the owner is
    /// untouched: see `domain::artifact`.
    pub fn edit_artifact(
        &self,
        id: ArtifactId,
        group: GroupId,
        by: &Actor,
        revision: ArtifactRevision<'_>,
    ) -> Result<Option<Artifact>, StoreError> {
        let ArtifactRevision { title, page, sources, condensed, note } = revision;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((version, owner)) = standing(&tx, id, Some(group))? else {
            return Ok(None);
        };
        let kept = version_at(&tx, id, version)?.ok_or_else(|| {
            StoreError::Corrupt(format!("artifact {id} has no page at version {version}"))
        })?;
        let page = page.map(str::to_string).unwrap_or(kept.page);
        let sources = match sources {
            Some(sources) => declared(sources)?,
            None => kept.sources,
        };
        let condensed = condensed.map(str::to_string).or(kept.condensed);
        let next = version + 1;
        let now = now_ms();
        append(
            &tx,
            id,
            now,
            Written {
                change: Change::Edited,
                by,
                version: next,
                owner: owner.as_deref(),
                note,
                version_of: Some(Version {
                    page: &page,
                    sources: &sources,
                    condensed: condensed.as_deref(),
                }),
            },
        )?;
        tx.execute(
            "UPDATE artifacts SET version = ?2, updated_at = ?3, title = COALESCE(?4, title)
              WHERE id = ?1",
            params![id.to_string(), next, now, title],
        )?;
        tx.commit()?;
        self.artifact(id, group)
    }

    /// An agent taking ownership. Leaves the version and "last updated" where
    /// they were: ownership is not content.
    pub fn take_artifact(
        &self,
        id: ArtifactId,
        group: GroupId,
        agent: AgentId,
        name: &str,
        note: &str,
    ) -> Result<Option<Artifact>, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((version, _)) = standing(&tx, id, Some(group))? else {
            return Ok(None);
        };
        tx.execute(
            "UPDATE artifacts SET owner_id = ?2 WHERE id = ?1",
            params![id.to_string(), agent.to_string()],
        )?;
        let by = Actor::Agent { id: agent, name: name.to_string() };
        append(
            &tx,
            id,
            now_ms(),
            Written {
                change: Change::Took,
                by: &by,
                version,
                owner: Some(name),
                note,
                version_of: None,
            },
        )?;
        tx.commit()?;
        self.artifact(id, group)
    }

    /// The operator giving an artifact to an agent. The agent has to be a live
    /// member of the artifact's own crew: ownership outside the crew would be
    /// an owner who cannot reach the page.
    pub fn hand_artifact(
        &self,
        id: ArtifactId,
        to: AgentId,
    ) -> Result<Option<Artifact>, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((version, _)) = standing(&tx, id, None)? else {
            return Ok(None);
        };
        let name: Option<String> = tx
            .query_row(
                "SELECT a.name FROM agents a JOIN artifacts r ON r.group_id = a.group_id
                  WHERE r.id = ?1 AND a.id = ?2 AND a.lifecycle <> 'terminated'",
                params![id.to_string(), to.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let Some(name) = name else {
            return Err(StoreError::AgentNotInGroup(to));
        };
        tx.execute(
            "UPDATE artifacts SET owner_id = ?2 WHERE id = ?1",
            params![id.to_string(), to.to_string()],
        )?;
        append(
            &tx,
            id,
            now_ms(),
            Written {
                change: Change::Handed,
                by: &Actor::Operator,
                version,
                owner: Some(&name),
                note: "",
                version_of: None,
            },
        )?;
        tx.commit()?;
        self.any_artifact(id)
    }

    /// An earlier version put back, as a new one. `None` when the artifact or
    /// that version does not exist.
    pub fn restore_artifact(
        &self,
        id: ArtifactId,
        version: u32,
        by: &Actor,
    ) -> Result<Option<Artifact>, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((current, owner)) = standing(&tx, id, None)? else {
            return Ok(None);
        };
        let Some(earlier) = version_at(&tx, id, version)? else {
            return Ok(None);
        };
        let next = current + 1;
        let now = now_ms();
        let note = format!("Put back version {version}.");
        append(
            &tx,
            id,
            now,
            Written {
                change: Change::Restored,
                by,
                version: next,
                owner: owner.as_deref(),
                note: &note,
                version_of: Some(earlier.as_version()),
            },
        )?;
        tx.execute(
            "UPDATE artifacts SET version = ?2, updated_at = ?3 WHERE id = ?1",
            params![id.to_string(), next, now],
        )?;
        tx.commit()?;
        self.any_artifact(id)
    }

    /// The operator allowing the reads the current version declares.
    ///
    /// Stores the list exactly as the version holds it, which is what makes the
    /// approval one of that list and nothing else: see the migration. A page
    /// that declares none has nothing to allow, and is left as it is.
    pub fn allow_artifact_sources(&self, id: ArtifactId) -> Result<Option<Artifact>, StoreError> {
        self.allow_sources(id, None)
    }

    /// The same, only if the current version still declares `seen`: the list
    /// an approval card showed the operator before they answered it. A crewmate
    /// can edit the page while the card waits, and a yes to one list is not a
    /// yes to whatever replaced it. The artifact comes back either way, and
    /// `sources_allowed` on it says which happened.
    pub fn allow_artifact_sources_seen(
        &self,
        id: ArtifactId,
        seen: &[Source],
    ) -> Result<Option<Artifact>, StoreError> {
        self.allow_sources(id, Some(seen))
    }

    fn allow_sources(
        &self,
        id: ArtifactId,
        seen: Option<&[Source]>,
    ) -> Result<Option<Artifact>, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((version, owner)) = standing(&tx, id, None)? else {
            return Ok(None);
        };
        let Some(Stored { sources, .. }) = version_at(&tx, id, version)? else {
            return Ok(None);
        };
        let as_seen = match seen {
            None => true,
            Some(seen) => {
                let now: Vec<Source> = serde_json::from_str(&sources)
                    .map_err(|e| corrupt("artifact sources", &sources, e))?;
                now == seen
            }
        };
        if sources != "[]" && as_seen {
            tx.execute(
                "UPDATE artifacts SET allowed_sources = ?2 WHERE id = ?1",
                params![id.to_string(), sources],
            )?;
            append(
                &tx,
                id,
                now_ms(),
                Written {
                    change: Change::Allowed,
                    by: &Actor::Operator,
                    version,
                    owner: owner.as_deref(),
                    note: "",
                    version_of: None,
                },
            )?;
        }
        tx.commit()?;
        self.any_artifact(id)
    }

    /// The reads one version declares, or the current one's for `None`, and
    /// whether the operator allowed exactly that list.
    pub fn artifact_sources(
        &self,
        id: ArtifactId,
        version: Option<u32>,
    ) -> Result<Option<(Vec<Source>, bool)>, StoreError> {
        let conn = self.conn()?;
        let found: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT h.sources, r.allowed_sources
                   FROM artifact_history h JOIN artifacts r ON r.id = h.artifact_id
                  WHERE h.artifact_id = ?1 AND h.page IS NOT NULL
                    AND h.version = COALESCE(?2, r.version)",
                params![id.to_string(), version],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((declared, allowed)) = found else {
            return Ok(None);
        };
        let sources: Vec<Source> = serde_json::from_str(&declared)
            .map_err(|e| corrupt("artifact sources", &declared, e))?;
        let allowed = !sources.is_empty() && allowed.as_deref() == Some(declared.as_str());
        Ok(Some((sources, allowed)))
    }

    /// The whole log, oldest first, without the pages.
    pub fn artifact_log(&self, id: ArtifactId) -> Result<Vec<Entry>, StoreError> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare(&format!("{LOG_COLUMNS} WHERE artifact_id = ?1 ORDER BY seq ASC"))?;
        let rows = stmt.query_map(params![id.to_string()], row_to_entry)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    /// The page at one version, or at the current one for `None`.
    pub fn artifact_page(
        &self,
        id: ArtifactId,
        version: Option<u32>,
    ) -> Result<Option<String>, StoreError> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                "SELECT h.page FROM artifact_history h JOIN artifacts r ON r.id = h.artifact_id
                  WHERE h.artifact_id = ?1 AND h.page IS NOT NULL
                    AND h.version = COALESCE(?2, r.version)",
                params![id.to_string(), version],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// The current version's condensed view, if it has one. `None` as well for
    /// an artifact that is not there; the caller has already asked which.
    pub fn artifact_condensed(&self, id: ArtifactId) -> Result<Option<String>, StoreError> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                "SELECT h.condensed FROM artifact_history h JOIN artifacts r ON r.id = h.artifact_id
                  WHERE h.artifact_id = ?1 AND h.page IS NOT NULL AND h.version = r.version",
                params![id.to_string()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// Deletes one, with its whole history. The operator's call only.
    pub fn delete_artifact(&self, id: ArtifactId) -> Result<bool, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM artifact_history WHERE artifact_id = ?1", params![id.to_string()])?;
        let gone = tx.execute("DELETE FROM artifacts WHERE id = ?1", params![id.to_string()])? > 0;
        tx.commit()?;
        Ok(gone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::{AgentCard, CleanDraft};
    use crate::domain::group::CleanGroup;

    struct Fixture {
        store: Store,
        crew: GroupId,
        other: GroupId,
        rae: AgentCard,
        milo: AgentCard,
        juno: AgentCard,
        _dir: tempfile::TempDir,
    }

    fn agent(store: &Store, name: &str, group: GroupId) -> AgentCard {
        store
            .create_agent(&CleanDraft {
                group_id: Some(group),
                name: name.into(),
                avatar: "orb".into(),
                color: "#7fb069".into(),
                model: "test".into(),
                reasoning_effort: None,
                system_prompt: String::new(),
                skills: vec![],
            })
            .unwrap()
    }

    /// Rae and Milo in one crew, Juno in another.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("test.db")).unwrap();
        let crew = store.create_group(&CleanGroup { name: "Sales".into(), ..Default::default() });
        let other = store.create_group(&CleanGroup { name: "Ops".into(), ..Default::default() });
        let (crew, other) = (crew.unwrap().id, other.unwrap().id);
        let rae = agent(&store, "Rae", crew);
        let milo = agent(&store, "Milo", crew);
        let juno = agent(&store, "Juno", other);
        Fixture { store, crew, other, rae, milo, juno, _dir: dir }
    }

    fn by(card: &AgentCard) -> Actor {
        Actor::Agent { id: card.id, name: card.name.clone() }
    }

    fn made(f: &Fixture) -> Artifact {
        f.store
            .create_artifact(
                f.crew,
                f.rae.id,
                "Rae",
                ArtifactDraft {
                    title: "Pipeline",
                    page: "<p>v1</p>",
                    sources: &[],
                    condensed: None,
                    note: "First cut",
                },
            )
            .unwrap()
    }

    #[test]
    fn another_crews_artifact_is_not_found_rather_than_refused() {
        // An id is something a model can invent. One that landed on another
        // crew's page must not reach it, and must not confirm it exists.
        let f = fixture();
        let page = made(&f);
        assert_eq!(f.store.artifact(page.id, f.other).unwrap(), None);
        assert_eq!(
            f.store
                .edit_artifact(
                    page.id,
                    f.other,
                    &by(&f.juno),
                    ArtifactRevision {
                        title: None,
                        page: Some("<p>x</p>"),
                        sources: None,
                        condensed: None,
                        note: "mine now"
                    }
                )
                .unwrap(),
            None
        );
        assert_eq!(
            f.store.take_artifact(page.id, f.other, f.juno.id, "Juno", "why").unwrap(),
            None
        );
        assert!(f.store.artifacts(Some(f.other), 50).unwrap().is_empty());

        let untouched = f.store.artifact(page.id, f.crew).unwrap().unwrap();
        assert_eq!(untouched.version, 1);
        assert_eq!(untouched.owner.unwrap().id, f.rae.id);
        assert_eq!(f.store.artifact_log(page.id).unwrap().len(), 1, "nothing was logged");
    }

    #[test]
    fn the_operator_cannot_hand_an_artifact_to_somebody_outside_its_crew() {
        let f = fixture();
        let page = made(&f);
        assert!(matches!(
            f.store.hand_artifact(page.id, f.juno.id),
            Err(StoreError::AgentNotInGroup(_))
        ));
        let handed = f.store.hand_artifact(page.id, f.milo.id).unwrap().unwrap();
        assert_eq!(handed.owner.unwrap().id, f.milo.id);
        let log = f.store.artifact_log(page.id).unwrap();
        assert_eq!(log.last().unwrap().change, Change::Handed);
        assert_eq!(log.last().unwrap().by, Actor::Operator);
        assert_eq!(log.last().unwrap().owner.as_deref(), Some("Milo"));
    }

    #[test]
    fn a_crew_mate_edits_without_becoming_the_owner() {
        let f = fixture();
        let page = made(&f);
        let edited = f
            .store
            .edit_artifact(
                page.id,
                f.crew,
                &by(&f.milo),
                ArtifactRevision {
                    title: None,
                    page: Some("<p>v2</p>"),
                    sources: None,
                    condensed: None,
                    note: "Added Q4",
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(edited.version, 2);
        assert_eq!(edited.owner.as_ref().unwrap().id, f.rae.id, "an edit is not a take-over");
        assert_eq!(edited.edited_by, by(&f.milo), "and the list says who changed it");
        assert_eq!(f.store.artifact_page(page.id, None).unwrap().unwrap(), "<p>v2</p>");
        assert_eq!(f.store.artifact_page(page.id, Some(1)).unwrap().unwrap(), "<p>v1</p>");
    }

    #[test]
    fn a_rename_keeps_the_page_and_is_still_a_version() {
        let f = fixture();
        let page = made(&f);
        let renamed = f
            .store
            .edit_artifact(
                page.id,
                f.crew,
                &by(&f.rae),
                ArtifactRevision {
                    title: Some("Pipeline by stage"),
                    page: None,
                    sources: None,
                    condensed: None,
                    note: "Clearer",
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(renamed.title, "Pipeline by stage");
        assert_eq!(renamed.version, 2);
        assert_eq!(f.store.artifact_page(page.id, Some(2)).unwrap().unwrap(), "<p>v1</p>");
    }

    #[test]
    fn taking_ownership_moves_the_owner_and_nothing_about_the_page() {
        let f = fixture();
        let page = made(&f);
        let taken =
            f.store.take_artifact(page.id, f.crew, f.milo.id, "Milo", "Rae moved to Ops").unwrap();
        let taken = taken.unwrap();
        assert_eq!(taken.owner.unwrap().id, f.milo.id);
        assert_eq!(taken.version, 1, "ownership is not content");
        assert_eq!(taken.updated_at, page.updated_at, "and it does not make the page newer");

        let log = f.store.artifact_log(page.id).unwrap();
        let last = log.last().unwrap();
        assert_eq!(last.change, Change::Took);
        assert_eq!(last.note, "Rae moved to Ops");
        assert_eq!(log[0].owner.as_deref(), Some("Rae"), "the row before says who held it");
    }

    #[test]
    fn a_restore_is_a_new_version_and_the_one_it_replaced_is_kept() {
        let f = fixture();
        let page = made(&f);
        f.store
            .edit_artifact(
                page.id,
                f.crew,
                &by(&f.milo),
                ArtifactRevision {
                    title: None,
                    page: Some("<p>broken</p>"),
                    sources: None,
                    condensed: None,
                    note: "Oops",
                },
            )
            .unwrap();
        let back = f.store.restore_artifact(page.id, 1, &Actor::Operator).unwrap().unwrap();
        assert_eq!(back.version, 3);
        assert_eq!(f.store.artifact_page(page.id, None).unwrap().unwrap(), "<p>v1</p>");
        assert_eq!(f.store.artifact_page(page.id, Some(2)).unwrap().unwrap(), "<p>broken</p>");
        assert_eq!(f.store.restore_artifact(page.id, 9, &Actor::Operator).unwrap(), None);
    }

    #[test]
    fn an_owner_who_left_the_crew_is_still_named_and_said_to_be_gone() {
        let f = fixture();
        let page = made(&f);
        f.store.move_agent(f.rae.id, f.other, None).unwrap();
        let owner = f.store.artifact(page.id, f.crew).unwrap().unwrap().owner.unwrap();
        assert_eq!((owner.id, owner.gone), (f.rae.id, true));

        let second = f
            .store
            .create_artifact(
                f.crew,
                f.milo.id,
                "Milo",
                ArtifactDraft {
                    title: "Plan",
                    page: "<p>p</p>",
                    sources: &[],
                    condensed: None,
                    note: "First",
                },
            )
            .unwrap();
        f.store.discard_agent(f.milo.id, now_ms()).unwrap();
        let owner = f.store.artifact(second.id, f.crew).unwrap().unwrap().owner.unwrap();
        assert!(owner.gone, "a deleted owner is gone too");
    }

    #[test]
    fn the_log_keeps_the_names_agents_had_when_they_acted() {
        let f = fixture();
        let page = made(&f);
        f.store
            .conn()
            .unwrap()
            .execute("UPDATE agents SET name='Raegan' WHERE id=?1", params![f.rae.id.to_string()])
            .unwrap();
        let log = f.store.artifact_log(page.id).unwrap();
        assert_eq!(log[0].by, by(&f.rae), "the log is what happened, in the words of the moment");
        let listed = f.store.artifact(page.id, f.crew).unwrap().unwrap();
        assert_eq!(listed.owner.unwrap().name, "Raegan", "and the list is who it is now");
    }

    #[test]
    fn only_the_changes_that_make_a_version_may_carry_a_page() {
        // `Change::makes_version` and the table's CHECK constraints say the
        // same thing. This holds them together, both ways, for every change: a
        // row that made a version carries a page and the reads that page
        // declares, and no other row carries either.
        let f = fixture();
        let kept = made(&f);
        let conn = f.store.conn().unwrap();
        for (seq, change) in [
            Change::Created,
            Change::Edited,
            Change::Restored,
            Change::Took,
            Change::Handed,
            Change::Allowed,
        ]
        .into_iter()
        .enumerate()
        {
            let insert = |seq: usize, page: Option<&str>, sources: Option<&str>| {
                conn.execute(
                    "INSERT INTO artifact_history (artifact_id, seq, at, change, version, page, sources)
                     VALUES (?1, ?2, 0, ?3, 1, ?4, ?5)",
                    params![kept.id.to_string(), seq, change.as_str(), page, sources],
                )
            };
            let with = insert(100 + seq * 3, Some("<p></p>"), Some("[]"));
            let without = insert(101 + seq * 3, None, None);
            let half = insert(102 + seq * 3, Some("<p></p>"), None);
            assert_eq!(with.is_ok(), change.makes_version(), "{change:?} with a page");
            assert_eq!(without.is_ok(), !change.makes_version(), "{change:?} without one");
            assert!(half.is_err(), "{change:?}: a page without its reads is half a version");
        }
        // A condensed view is part of a version, so a row that made none
        // cannot carry one.
        let stray = conn.execute(
            "INSERT INTO artifact_history (artifact_id, seq, at, change, version, condensed)
             VALUES (?1, 900, 0, 'took', 1, '<b>3</b>')",
            params![kept.id.to_string()],
        );
        assert!(stray.is_err(), "a take-over carrying a condensed view");
    }

    #[test]
    fn a_condensed_view_is_kept_changed_and_put_back_with_its_version() {
        // The number on the status bar and the board behind it are one version
        // of one thing. An edit that leaves the strip out keeps it, and a
        // restore brings back the strip that version had, not today's.
        let f = fixture();
        let page = f
            .store
            .create_artifact(
                f.crew,
                f.rae.id,
                "Rae",
                ArtifactDraft {
                    title: "Pipeline",
                    page: "<p>v1</p>",
                    sources: &[],
                    condensed: Some("<b>4 deals</b>"),
                    note: "",
                },
            )
            .unwrap();
        assert!(page.condensed);
        let edit = |page_html: Option<&'static str>, condensed: Option<&'static str>| {
            f.store
                .edit_artifact(
                    page.id,
                    f.crew,
                    &by(&f.milo),
                    ArtifactRevision {
                        title: None,
                        page: page_html,
                        sources: None,
                        condensed,
                        note: "edit",
                    },
                )
                .unwrap()
                .unwrap()
        };
        edit(Some("<p>v2</p>"), None);
        assert_eq!(f.store.artifact_condensed(page.id).unwrap().as_deref(), Some("<b>4 deals</b>"));
        edit(None, Some("<b>5 deals</b>"));
        assert_eq!(f.store.artifact_condensed(page.id).unwrap().as_deref(), Some("<b>5 deals</b>"));

        let back = f.store.restore_artifact(page.id, 1, &Actor::Operator).unwrap().unwrap();
        assert_eq!(back.version, 4);
        assert_eq!(f.store.artifact_condensed(page.id).unwrap().as_deref(), Some("<b>4 deals</b>"));
        assert_eq!(f.store.artifact_page(page.id, None).unwrap().unwrap(), "<p>v1</p>");

        let plain = made(&f);
        assert!(!plain.condensed, "most pages have none");
        assert_eq!(f.store.artifact_condensed(plain.id).unwrap(), None);
    }

    #[test]
    fn a_card_allows_only_the_reads_it_showed() {
        // The card waits while the crew keeps working. A crewmate who changed
        // the reads in the meantime has changed what the operator would be
        // saying yes to, so the yes does not reach the new list.
        let f = fixture();
        let shown = reads("linear__list_issues");
        let page = f
            .store
            .create_artifact(
                f.crew,
                f.rae.id,
                "Rae",
                ArtifactDraft {
                    title: "Board",
                    page: "<p>v1</p>",
                    sources: &shown,
                    condensed: None,
                    note: "",
                },
            )
            .unwrap();
        f.store
            .edit_artifact(
                page.id,
                f.crew,
                &by(&f.milo),
                ArtifactRevision {
                    title: None,
                    page: None,
                    sources: Some(&reads("linear__list_projects")),
                    condensed: None,
                    note: "Projects instead",
                },
            )
            .unwrap();
        let after = f.store.allow_artifact_sources_seen(page.id, &shown).unwrap().unwrap();
        assert!(!after.sources_allowed, "the list changed under the card");
        assert_ne!(f.store.artifact_log(page.id).unwrap().last().unwrap().change, Change::Allowed);

        let current = f.store.any_artifact(page.id).unwrap().unwrap().sources;
        let allowed = f.store.allow_artifact_sources_seen(page.id, &current).unwrap().unwrap();
        assert!(allowed.sources_allowed, "and the list it did show is allowed");
    }

    fn reads(tool: &str) -> Vec<Source> {
        vec![Source { name: "issues".into(), tool: tool.into(), arguments: serde_json::json!({}) }]
    }

    #[test]
    fn allowing_reads_covers_exactly_the_list_the_operator_saw() {
        // The approval is of a list, compared as stored. Anything that changes
        // the list waits for the operator again, and nothing has to remember to
        // revoke it; putting back a version with the list they allowed needs
        // nothing new.
        let f = fixture();
        let page = f
            .store
            .create_artifact(
                f.crew,
                f.rae.id,
                "Rae",
                ArtifactDraft {
                    title: "Board",
                    page: "<p>v1</p>",
                    sources: &reads("linear__list_issues"),
                    condensed: None,
                    note: "",
                },
            )
            .unwrap();
        assert!(!page.sources_allowed, "declared is not allowed");
        assert!(f.store.allow_artifact_sources(page.id).unwrap().unwrap().sources_allowed);
        assert_eq!(
            f.store.artifact_log(page.id).unwrap().last().unwrap().change,
            Change::Allowed,
            "and the log says who allowed it"
        );

        let restyled = f
            .store
            .edit_artifact(
                page.id,
                f.crew,
                &by(&f.milo),
                ArtifactRevision {
                    title: None,
                    page: Some("<p>v2</p>"),
                    sources: None,
                    condensed: None,
                    note: "Tidier",
                },
            )
            .unwrap()
            .unwrap();
        assert!(restyled.sources_allowed, "a new page with the same reads keeps them");

        let widened = f
            .store
            .edit_artifact(
                page.id,
                f.crew,
                &by(&f.milo),
                ArtifactRevision {
                    title: None,
                    page: None,
                    sources: Some(&reads("linear__list_projects")),
                    condensed: None,
                    note: "Projects too",
                },
            )
            .unwrap()
            .unwrap();
        assert!(!widened.sources_allowed, "a changed list waits again");
        assert!(f.store.artifact_sources(page.id, Some(2)).unwrap().unwrap().1);
        assert!(!f.store.artifact_sources(page.id, None).unwrap().unwrap().1);

        let back = f.store.restore_artifact(page.id, 2, &Actor::Operator).unwrap().unwrap();
        assert!(back.sources_allowed, "the version the operator allowed reads again");
    }

    #[test]
    fn a_page_with_no_reads_has_nothing_to_allow() {
        let f = fixture();
        let page = made(&f);
        let after = f.store.allow_artifact_sources(page.id).unwrap().unwrap();
        assert!(!after.sources_allowed);
        assert_eq!(f.store.artifact_log(page.id).unwrap().len(), 1, "and nothing is logged");
    }

    #[test]
    fn disbanding_a_crew_takes_its_artifacts_and_their_history_with_it() {
        let f = fixture();
        let page = made(&f);
        f.store.discard_agent(f.rae.id, now_ms()).unwrap();
        f.store.discard_agent(f.milo.id, now_ms()).unwrap();
        f.store.delete_group(f.crew).unwrap();
        assert_eq!(f.store.any_artifact(page.id).unwrap(), None);
        assert!(f.store.artifact_log(page.id).unwrap().is_empty());
    }

    #[test]
    fn search_finds_an_artifact_by_its_title_in_every_crew() {
        let f = fixture();
        let page = made(&f);
        f.store
            .create_artifact(
                f.other,
                f.juno.id,
                "Juno",
                ArtifactDraft {
                    title: "Rota",
                    page: "<p>pipeline</p>",
                    sources: &[],
                    condensed: None,
                    note: "",
                },
            )
            .unwrap();
        let found = f.store.search("pipe", 10).unwrap().artifacts;
        assert_eq!(found.iter().map(|a| a.id).collect::<Vec<_>>(), vec![page.id]);
        // A wildcard in what was typed is a character, as it is for messages.
        assert!(f.store.search("%", 10).unwrap().artifacts.is_empty());
    }

    #[test]
    fn deleting_one_takes_its_whole_history() {
        let f = fixture();
        let page = made(&f);
        assert!(f.store.delete_artifact(page.id).unwrap());
        assert!(f.store.artifact_log(page.id).unwrap().is_empty());
        assert!(!f.store.delete_artifact(page.id).unwrap(), "a second delete finds nothing");
    }

    #[test]
    fn the_list_is_newest_change_first_and_crosses_crews_only_for_the_operator() {
        let f = fixture();
        let first = made(&f);
        let second = f
            .store
            .create_artifact(
                f.other,
                f.juno.id,
                "Juno",
                ArtifactDraft {
                    title: "Rota",
                    page: "<p>r</p>",
                    sources: &[],
                    condensed: None,
                    note: "First",
                },
            )
            .unwrap();
        f.store
            .edit_artifact(
                first.id,
                f.crew,
                &by(&f.rae),
                ArtifactRevision {
                    title: None,
                    page: Some("<p>v2</p>"),
                    sources: None,
                    condensed: None,
                    note: "Newer",
                },
            )
            .unwrap();
        let every: Vec<_> =
            f.store.artifacts(None, 50).unwrap().into_iter().map(|a| a.id).collect();
        assert_eq!(every, vec![first.id, second.id]);
        let crew: Vec<_> =
            f.store.artifacts(Some(f.other), 50).unwrap().into_iter().map(|a| a.id).collect();
        assert_eq!(crew, vec![second.id]);
    }
}
