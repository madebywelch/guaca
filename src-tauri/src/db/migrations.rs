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
    (
        9,
        r#"
-- An agent's own schedule. It sets these for itself, so the row belongs to the
-- agent rather than to the operator.
--
-- `every_secs` NULL means it fires once and is done. A repeating routine keeps
-- its row and moves `next_run_at` forward, so a schedule survives restarts:
-- what is stored is when it is next due, not a timer someone has to hold.
CREATE TABLE routines (
    id          TEXT    PRIMARY KEY,
    agent_id    TEXT    NOT NULL REFERENCES agents(id),
    what        TEXT    NOT NULL,
    every_secs  INTEGER,
    next_run_at INTEGER NOT NULL,
    last_run_at INTEGER,
    created_at  INTEGER NOT NULL
);

-- The scheduler asks one question, repeatedly: what is due?
CREATE INDEX routines_due ON routines (next_run_at);
CREATE INDEX routines_agent ON routines (agent_id);
"#,
    ),
    (
        10,
        r#"
-- What each model call cost, as the provider counted it.
--
-- One row per call rather than a running total on the agent, because the
-- question an operator actually has is which run burned the tokens, and a
-- counter cannot answer it. Rows are small and a busy day is a few thousand.
--
-- `group_id` is denormalised on purpose: an agent can be moved between groups,
-- and what a group spent while an agent was in it does not move with it.
CREATE TABLE usage (
    id         INTEGER PRIMARY KEY,
    agent_id   TEXT    NOT NULL REFERENCES agents(id),
    group_id   TEXT    NOT NULL,
    run_id     TEXT    NOT NULL,
    model      TEXT    NOT NULL,
    prompt     INTEGER NOT NULL,
    completion INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX usage_group ON usage (group_id);
CREATE INDEX usage_run ON usage (run_id);
"#,
    ),
    (
        11,
        r#"
-- Dollars, when the provider prices the call. NULL for a local server, which
-- has nothing to charge: summing NULL as zero would quietly report that a crew
-- ran for free.
--
-- Its own migration rather than a column in the one above, which had already
-- run by the time this was wanted. A migration that has been applied anywhere
-- is finished: editing it leaves databases that ran the old version with a
-- schema no version number distinguishes from the new one.
ALTER TABLE usage ADD COLUMN cost REAL;
"#,
    ),
    (
        12,
        r#"
-- Accounts a crew can reach. Two kinds in one table, because they are one
-- concept to an operator and differ only in where the access physically lives.
--
-- `agent_id` is set for a sign-in and NULL for a key, and that is not a
-- convenience: a sign-in is cookies on one machine's disk, so it belongs to
-- that agent, while a key is a string any machine in the group can be handed.
-- Modelling both as group-wide would tell three agents they are signed in to
-- Gmail when one of them is.
--
-- `secret` holds a key's value. It never leaves this table except into the
-- environment of a command running inside a sandbox: not into a prompt, not
-- over IPC, not onto the sandbox's disk.
CREATE TABLE connectors (
    id           TEXT    PRIMARY KEY,
    group_id     TEXT    NOT NULL REFERENCES groups(id),
    agent_id     TEXT    REFERENCES agents(id),
    kind         TEXT    NOT NULL,
    service      TEXT    NOT NULL,
    account      TEXT    NOT NULL,
    url          TEXT    NOT NULL DEFAULT '',
    env_var      TEXT    NOT NULL DEFAULT '',
    secret       TEXT    NOT NULL DEFAULT '',
    note         TEXT    NOT NULL DEFAULT '',
    confirmed_at INTEGER,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);

-- The two questions asked of this table: what can this crew reach, and what
-- does this agent's machine hold.
CREATE INDEX connectors_group ON connectors (group_id);
CREATE INDEX connectors_agent ON connectors (agent_id);

-- One variable name per group. Two keys sharing a name is a machine where one
-- of them silently wins, and which one depends on row order.
CREATE UNIQUE INDEX connectors_env_unique
    ON connectors (group_id, env_var)
    WHERE kind = 'key';
"#,
    ),
    (
        13,
        r#"
-- Sign-ins stopped being something an operator declares. The browser already
-- knows what it is logged in to, and Chrome's remote interface will say, so
-- asking the machine beats asking the person: an agent signed in a moment ago
-- advertises it without anybody recording anything.
--
-- That leaves `connectors` holding one kind, so the columns that only a
-- declared sign-in used are gone. Rebuilt rather than ALTERed because dropping
-- a column referenced by a partial index is not something SQLite will do in
-- place, and because the rows that were sign-ins have to go: they are replaced
-- by detection, not migrated into it.
CREATE TABLE connectors_new (
    id         TEXT    PRIMARY KEY,
    group_id   TEXT    NOT NULL REFERENCES groups(id),
    service    TEXT    NOT NULL,
    account    TEXT    NOT NULL,
    env_var    TEXT    NOT NULL,
    secret     TEXT    NOT NULL DEFAULT '',
    note       TEXT    NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT INTO connectors_new (id,group_id,service,account,env_var,secret,note,created_at,updated_at)
SELECT id,group_id,service,account,env_var,secret,note,created_at,updated_at
  FROM connectors WHERE kind = 'key';

DROP TABLE connectors;
ALTER TABLE connectors_new RENAME TO connectors;

CREATE INDEX connectors_group ON connectors (group_id);
CREATE UNIQUE INDEX connectors_env_unique ON connectors (group_id, env_var);

-- What each machine's browser is signed in to, as last observed.
--
-- A cache of a fact that lives somewhere else, which is why the whole set for
-- an agent is replaced on every scan rather than merged: a row that lingers
-- after the operator logged out is worse than no row, because the crew keeps
-- routing work to an agent that will hit a login wall.
--
-- `first_seen_at` survives a replace so "signed in since Tuesday" stays true
-- across scans, and no cookie value is stored, ever: the name and the flags are
-- the whole signal and a session token is exactly what must not be kept.
CREATE TABLE signins (
    agent_id      TEXT    NOT NULL REFERENCES agents(id),
    domain        TEXT    NOT NULL,
    service       TEXT    NOT NULL,
    recognised    INTEGER NOT NULL DEFAULT 0,
    first_seen_at INTEGER NOT NULL,
    last_seen_at  INTEGER NOT NULL,
    PRIMARY KEY (agent_id, domain)
);

CREATE INDEX signins_agent ON signins (agent_id);
"#,
    ),
    (
        14,
        r#"
-- What an agent asked the operator for permission to do, and what they said.
--
-- One table for both questions, because they are the same fact read at two
-- times: `state` is the answer to "may it do this now", and a row that says
-- alwaysAllow is the answer to "must it ask again". A separate grants table
-- would let the two disagree about a decision the operator made once.
--
-- `summary` and `detail` are Guaca's own words for what was asked, written at
-- request time and never rewritten: the transcript has to keep saying what the
-- operator was actually shown, whatever the agent or its instructions became
-- afterwards.
CREATE TABLE approvals (
    id         TEXT    PRIMARY KEY,
    agent_id   TEXT    NOT NULL REFERENCES agents(id),
    group_id   TEXT    NOT NULL,
    run_id     TEXT    NOT NULL,
    action     TEXT    NOT NULL,
    summary    TEXT    NOT NULL,
    detail     TEXT    NOT NULL DEFAULT '[]',
    state      TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    decided_at INTEGER
);

-- The two questions asked of this table, both partial so they stay small: what
-- is still waiting on the operator, and what has this agent already been let
-- off asking about.
CREATE INDEX approvals_pending ON approvals (created_at) WHERE state = 'pending';
CREATE INDEX approvals_granted
    ON approvals (agent_id, action)
    WHERE state = 'alwaysAllow';
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
    fn two_connectors_in_one_group_cannot_claim_the_same_variable() {
        // Both would be written into the same machine's environment and one
        // would win by row order, so the agent would be handed a token for an
        // account nobody chose.
        let mut conn = memory();
        run(&mut conn).unwrap();
        let insert = "INSERT INTO connectors (id,group_id,service,account,env_var,secret,note,created_at,updated_at)
                      VALUES (?1,?2,?3,'me',?4,'s','',0,0)";

        conn.execute(insert, rusqlite::params!["a", DEFAULT_GROUP_ID, "GitHub", "TOKEN"]).unwrap();
        let clash =
            conn.execute(insert, rusqlite::params!["b", DEFAULT_GROUP_ID, "Linear", "TOKEN"]);
        assert!(clash.is_err(), "a duplicate variable name in one group must be rejected");

        // Another group is another set of machines, so the same name is fine.
        conn.execute("INSERT INTO groups (id,name,created_at) VALUES ('g2','Research',0)", [])
            .unwrap();
        conn.execute(insert, rusqlite::params!["c", "g2", "Linear", "TOKEN"])
            .expect("groups do not share an environment");
    }

    #[test]
    fn declared_sign_ins_are_dropped_rather_than_carried_into_detection() {
        // A database written before sign-ins were detected holds rows an
        // operator typed. Keeping them would mean the roster advertised an
        // account nobody had checked against the machine, which is the claim
        // detection exists to stop making. The credentials beside them stay.
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        for (version, sql) in MIGRATIONS.iter().take(12) {
            tx.execute_batch(sql).unwrap();
            tx.pragma_update(None, "user_version", *version).unwrap();
        }
        tx.commit().unwrap();

        let row = "INSERT INTO connectors (id,group_id,agent_id,kind,service,account,url,env_var,secret,note,created_at,updated_at)
                   VALUES (?1,?2,?3,?4,?5,'me',?6,?7,?8,'',0,0)";
        conn.execute(
            row,
            rusqlite::params![
                "keep",
                DEFAULT_GROUP_ID,
                None::<String>,
                "key",
                "GitHub",
                "",
                "TOKEN",
                "s"
            ],
        )
        .unwrap();
        conn.execute(
            row,
            rusqlite::params![
                "drop",
                DEFAULT_GROUP_ID,
                None::<String>,
                "signin",
                "Gmail",
                "https://x",
                "",
                ""
            ],
        )
        .unwrap();

        run(&mut conn).unwrap();

        let kept: Vec<String> = conn
            .prepare("SELECT id FROM connectors")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(kept, vec!["keep".to_string()], "only the credential survives");

        // And the detected table is there and empty, waiting for a scan.
        let signins: i64 =
            conn.query_row("SELECT count(*) FROM signins", [], |r| r.get(0)).unwrap();
        assert_eq!(signins, 0);
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
