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
    (
        4,
        r#"
CREATE TABLE groups (
    id         TEXT    PRIMARY KEY,
    name       TEXT    NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX groups_name_unique ON groups (lower(name));

-- Every agent belongs to exactly one group, so there has to be one before the
-- column can be NOT NULL. This id is fixed rather than generated: the default
-- group is the one the UI hides while it is the only one, and a known id means
-- that check never depends on row order.
INSERT INTO groups (id, name, created_at)
VALUES ('00000000-0000-4000-8000-000000000001', 'Everyone', 0);

-- Rebuilt rather than ALTERed. SQLite refuses to ADD COLUMN when the column
-- carries both a REFERENCES clause and a non-NULL default, so the alternative
-- was to drop the foreign key and hope nothing ever writes a dangling group.
-- The rebuild is the documented way to add a constraint and behaves the same
-- whichever way `foreign_keys` happens to be set; migrations run on the
-- bootstrap connection, where it is off, which is what that procedure wants.
CREATE TABLE agents_new (
    id            TEXT    PRIMARY KEY,
    name          TEXT    NOT NULL,
    avatar        TEXT    NOT NULL,
    color         TEXT    NOT NULL,
    model         TEXT    NOT NULL,
    system_prompt TEXT    NOT NULL,
    skills        TEXT    NOT NULL DEFAULT '[]',
    lifecycle     TEXT    NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    group_id      TEXT    NOT NULL REFERENCES groups(id)
);

INSERT INTO agents_new
    (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id)
SELECT id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,
       '00000000-0000-4000-8000-000000000001'
  FROM agents;

DROP TABLE agents;
ALTER TABLE agents_new RENAME TO agents;

CREATE INDEX agents_group ON agents (group_id);

-- Names are addressable identifiers: an agent messages a peer by name, and
-- resolution is scoped to the sender's group. Uniqueness has to be scoped the
-- same way, or the scope of the index and the scope of the lookup disagree.
-- Global uniqueness would also stop two isolated groups from each having a
-- Manager, which is the obvious thing to want.
CREATE UNIQUE INDEX agents_live_name_unique
    ON agents (group_id, lower(name))
    WHERE lifecycle <> 'terminated';
"#,
    ),
    (
        5,
        r#"
-- A group is where a crew's inference settings belong. One group can run on a
-- local endpoint and another on a hosted one, and an agent inside a group still
-- overrides the model for itself. NULL means "inherit", which is why these are
-- nullable rather than defaulted: an empty string is a real value an operator
-- could set, and the two must stay distinguishable.
ALTER TABLE groups ADD COLUMN base_url      TEXT;
ALTER TABLE groups ADD COLUMN api_key       TEXT;
ALTER TABLE groups ADD COLUMN default_model TEXT;
"#,
    ),
    (
        6,
        r#"
-- The sandbox an agent uses as its computer. NULL means it has never been
-- given one. Stored rather than looked up by label so a rename or a Daytona
-- listing hiccup cannot detach an agent from work it left on a disk.
ALTER TABLE agents ADD COLUMN sandbox_id TEXT;
"#,
    ),
    (
        7,
        r#"
-- Computers moved from Daytona to E2B, whose sandboxes have internet access
-- without a plan upgrade. The ids left behind name sandboxes on a provider this
-- build no longer talks to, so they are cleared rather than left to 404 on
-- every check.
UPDATE agents SET sandbox_id = NULL;
"#,
    ),
    (
        8,
        r#"
-- Sandboxes are now created locked: envd refuses commands without a token, and
-- the public URLs refuse traffic without another. An id on its own no longer
-- reaches anything, so the tokens live beside it.
ALTER TABLE agents ADD COLUMN sandbox_envd_token    TEXT;
ALTER TABLE agents ADD COLUMN sandbox_traffic_token TEXT;

-- The sandboxes recorded before this have no tokens and cannot be reached, so
-- they are released rather than left as ids that fail on every use.
UPDATE agents SET sandbox_id = NULL;
"#,
    ),
];

/// The group every agent starts in, and the one the UI keeps out of the way
/// while it is the only one. Pinned so the check is an id comparison rather
/// than a name match or a count.
pub const DEFAULT_GROUP_ID: &str = "00000000-0000-4000-8000-000000000001";

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
    fn group_inference_settings_start_empty_and_mean_inherit() {
        // NULL and "" have to stay distinguishable: one means "use the app
        // default", the other is a value an operator deliberately blanked.
        let mut conn = memory();
        run(&mut conn).unwrap();
        let model: Option<String> = conn
            .query_row("SELECT default_model FROM groups WHERE id=?1", [DEFAULT_GROUP_ID], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(model, None, "a fresh group must inherit rather than pin a model");
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
        let insert = "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id)
                      VALUES (?1,?2,'avocado','#000','m','','[]',?3,1,0,0,'00000000-0000-4000-8000-000000000001')";
        conn.execute(insert, rusqlite::params!["a", "Manager", "active"]).unwrap();
        let clash = conn.execute(insert, rusqlite::params!["b", "manager", "active"]);
        assert!(clash.is_err(), "case-different duplicate must be rejected");
    }

    #[test]
    fn deleting_an_agent_frees_its_name() {
        let mut conn = memory();
        run(&mut conn).unwrap();
        let insert = "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id)
                      VALUES (?1,?2,'avocado','#000','m','','[]',?3,1,0,0,'00000000-0000-4000-8000-000000000001')";
        conn.execute(insert, rusqlite::params!["a", "Manager", "terminated"]).unwrap();
        conn.execute(insert, rusqlite::params!["b", "Manager", "active"])
            .expect("a terminated agent must not hold its name hostage");
    }

    #[test]
    fn existing_agents_are_moved_into_the_default_group() {
        // The upgrade path that matters: a database written before groups
        // existed must come out the other side with every agent in one.
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        for (version, sql) in MIGRATIONS.iter().take(3) {
            tx.execute_batch(sql).unwrap();
            tx.pragma_update(None, "user_version", *version).unwrap();
        }
        tx.commit().unwrap();
        conn.execute(
            "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at)
             VALUES ('a','Manager','avocado','#000','m','','[]','active',1,0,0)",
            [],
        )
        .unwrap();

        run(&mut conn).unwrap();

        let group: String =
            conn.query_row("SELECT group_id FROM agents WHERE id='a'", [], |r| r.get(0)).unwrap();
        assert_eq!(group, DEFAULT_GROUP_ID, "an agent must never be left without a group");
    }

    #[test]
    fn agent_names_are_unique_per_group_rather_than_globally() {
        // Two isolated groups each wanting a Manager is the ordinary case, and
        // the old global index made it impossible.
        let mut conn = memory();
        run(&mut conn).unwrap();
        conn.execute("INSERT INTO groups (id,name,created_at) VALUES ('g2','Research',0)", [])
            .unwrap();
        let insert = "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id)
                      VALUES (?1,?2,'avocado','#000','m','','[]','active',1,0,0,?3)";

        conn.execute(insert, rusqlite::params!["a", "Manager", DEFAULT_GROUP_ID]).unwrap();
        conn.execute(insert, rusqlite::params!["b", "Manager", "g2"])
            .expect("the same name in another group must be allowed");
        let clash = conn.execute(insert, rusqlite::params!["c", "manager", "g2"]);
        assert!(clash.is_err(), "a duplicate inside one group must still be rejected");
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
