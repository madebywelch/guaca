//! Schema migrations.
//!
//! Forward-only, numbered, and applied inside a transaction keyed on SQLite's
//! `user_version`. No migration framework: the whole thing is a list and a
//! loop, which is auditable in one screen and cannot drift from what actually
//! ran.

use rusqlite::{Connection, Transaction, TransactionBehavior};

/// Ordered migrations. Append only. Never edit a shipped entry.
const MIGRATIONS: &[(i32, &str)] = &[
    (
        1,
        r#"
CREATE TABLE agents (
    id            TEXT    PRIMARY KEY,
    name          TEXT    NOT NULL,
    emoji         TEXT    NOT NULL,
    color         TEXT    NOT NULL,
    model         TEXT    NOT NULL,
    system_prompt TEXT    NOT NULL,
    skills        TEXT    NOT NULL DEFAULT '[]',
    lifecycle     TEXT    NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

-- Names must be unique among agents you can still reach, but deleting an
-- agent has to free its name for reuse. A partial index over the live rows
-- expresses exactly that, without a nullable tombstone column.
CREATE UNIQUE INDEX agents_live_name_unique
    ON agents (lower(name))
    WHERE lifecycle <> 'terminated';

CREATE TABLE messages (
    id          TEXT    PRIMARY KEY,
    run_id      TEXT    NOT NULL,
    channel_id  TEXT    NOT NULL,
    from_kind   TEXT    NOT NULL,
    from_agent  TEXT,
    to_kind     TEXT    NOT NULL,
    to_agent    TEXT,
    parts       TEXT    NOT NULL,
    trust       TEXT    NOT NULL,
    hop         INTEGER NOT NULL DEFAULT 0,
    expects_reply INTEGER NOT NULL DEFAULT 1,
    cause       TEXT,
    created_at  INTEGER NOT NULL
);

-- The transcript query: one channel, in order. `id` breaks ties so that two
-- messages written in the same millisecond still have a stable order.
CREATE INDEX messages_channel_time ON messages (channel_id, created_at, id);

-- The activity feed: every agent-to-agent message, newest first. Partial so
-- the index stays small when most traffic is with the operator.
CREATE INDEX messages_inter_agent
    ON messages (created_at DESC)
    WHERE from_kind = 'agent' AND to_kind = 'agent';

CREATE INDEX messages_run ON messages (run_id, created_at);
"#,
    ),
    (
        2,
        r#"
-- Avatars became hand-drawn characters, so the column no longer holds an
-- emoji. Renaming keeps the schema honest about what it stores.
ALTER TABLE agents RENAME COLUMN emoji TO avatar;
"#,
    ),
    (
        3,
        r#"
-- The activity view became a flow board covering the whole conversation, not
-- just peer traffic, so the index that served the old feed no longer matches
-- any query. The replacement covers the new one: everything except an agent's
-- private activity records, newest first.
DROP INDEX IF EXISTS messages_inter_agent;

CREATE INDEX messages_flow
    ON messages (created_at DESC, id DESC)
    WHERE to_kind <> 'system';
"#,
    ),
];

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("migration {version} failed: {source}")]
    Failed {
        version: i32,
        #[source]
        source: rusqlite::Error,
    },
    #[error("database is at version {found}, newer than this build supports ({supported})")]
    FromTheFuture { found: i32, supported: i32 },
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

pub fn latest_version() -> i32 {
    MIGRATIONS.last().map(|(v, _)| *v).unwrap_or(0)
}

/// Applies every migration newer than the database's current `user_version`.
///
/// Safe to call on every startup, and safe to call from two processes at once:
/// each migration runs inside an immediate transaction and the version is
/// re-read after the write lock is held. Reading the version first and then
/// opening a transaction would let two racing callers both see version 0 and
/// both try to create the tables.
pub fn run(conn: &mut Connection) -> Result<i32, MigrationError> {
    let target = latest_version();

    loop {
        // `Immediate` takes the write lock at BEGIN rather than at first write,
        // so the loser waits here instead of failing partway through a batch.
        let tx: Transaction<'_> = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: i32 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;

        if current > target {
            // Downgrading would silently corrupt data written by a newer build.
            // Refusing is the only safe move.
            return Err(MigrationError::FromTheFuture { found: current, supported: target });
        }

        let Some((version, sql)) = MIGRATIONS.iter().find(|(v, _)| *v > current) else {
            break;
        };

        tx.execute_batch(sql)
            .map_err(|source| MigrationError::Failed { version: *version, source })?;
        // `user_version` does not accept a bound parameter.
        tx.pragma_update(None, "user_version", *version)?;
        tx.commit()?;
        tracing::info!(version, "applied migration");
    }

    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn migrations_bring_a_blank_database_to_the_latest_version() {
        let mut conn = memory();
        assert_eq!(run(&mut conn).unwrap(), latest_version());
        let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, latest_version());
    }

    #[test]
    fn running_twice_is_a_no_op() {
        let mut conn = memory();
        run(&mut conn).unwrap();
        run(&mut conn).unwrap();
        let tables: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('agents','messages')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 2);
    }

    #[test]
    fn the_avatar_column_is_renamed_and_keeps_its_data() {
        let mut conn = memory();
        // Stop at version 1 so the rename can be observed happening.
        let tx = conn.transaction().unwrap();
        tx.execute_batch(MIGRATIONS[0].1).unwrap();
        tx.pragma_update(None, "user_version", 1).unwrap();
        tx.commit().unwrap();
        conn.execute(
            "INSERT INTO agents (id,name,emoji,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at)
             VALUES ('a','Manager','avocado','#000','m','','[]','active',1,0,0)",
            [],
        )
        .unwrap();

        run(&mut conn).unwrap();

        let avatar: String =
            conn.query_row("SELECT avatar FROM agents WHERE id='a'", [], |r| r.get(0)).unwrap();
        assert_eq!(avatar, "avocado", "the rename must not drop the value");
    }

    #[test]
    fn a_newer_database_is_refused_rather_than_downgraded() {
        let mut conn = memory();
        run(&mut conn).unwrap();
        conn.pragma_update(None, "user_version", latest_version() + 5).unwrap();
        assert!(matches!(run(&mut conn), Err(MigrationError::FromTheFuture { .. })));
    }

    #[test]
    fn live_agent_names_are_unique_case_insensitively() {
        let mut conn = memory();
        run(&mut conn).unwrap();
        let insert = "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at)
                      VALUES (?1,?2,'avocado','#000','m','','[]',?3,1,0,0)";
        conn.execute(insert, rusqlite::params!["a", "Manager", "active"]).unwrap();
        let clash = conn.execute(insert, rusqlite::params!["b", "manager", "active"]);
        assert!(clash.is_err(), "case-different duplicate must be rejected");
    }

    #[test]
    fn deleting_an_agent_frees_its_name() {
        let mut conn = memory();
        run(&mut conn).unwrap();
        let insert = "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at)
                      VALUES (?1,?2,'avocado','#000','m','','[]',?3,1,0,0)";
        conn.execute(insert, rusqlite::params!["a", "Manager", "terminated"]).unwrap();
        conn.execute(insert, rusqlite::params!["b", "Manager", "active"])
            .expect("a terminated agent must not hold its name hostage");
    }

    #[test]
    fn a_failed_migration_leaves_the_version_untouched() {
        // Simulates a half-applied migration by running a batch that fails
        // partway. The transaction must roll the whole thing back.
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        let result = tx.execute_batch("CREATE TABLE ok (x); CREATE TABLE ok (x);");
        assert!(result.is_err());
        drop(tx);
        let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, 0);
        let leftover: i64 = conn
            .query_row("SELECT count(*) FROM sqlite_master WHERE name='ok'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(leftover, 0, "rollback must remove the partially created table");
    }
}
