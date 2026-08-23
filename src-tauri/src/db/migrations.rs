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
-- The rebuild is the documented way to add a constraint, and `run` turns
-- foreign key enforcement off around the whole migration sequence, which is
-- what that procedure wants and what a migration cannot arrange for itself.
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
    (
        15,
        r#"
-- What the sender said this message was for.
--
-- `expects_reply` answers "is anybody waiting on your words", which is what
-- makes cascades terminate. It was also being read as "is anybody asking you
-- for anything", and those came apart the moment an agent could instruct a
-- peer that had already answered: the instruction arrived with no reply
-- expected, so the recipient was told nothing needed doing and said nothing. A
-- real send to the operator's own address died exactly there.
--
-- Existing rows are courtesies by default, which is what they were: before
-- this column no message could carry declared work.
ALTER TABLE messages ADD COLUMN intent TEXT NOT NULL DEFAULT 'courtesy';
"#,
    ),
    (
        16,
        r#"
-- What makes a routine fire, and what to call it.
--
-- `every_secs` could say "every five hours" and could not say "every weekday"
-- or "every month": one is not a fixed number of seconds and the other is four
-- different numbers. Both are what an operator actually schedules, so the gap
-- becomes one case of a trigger rather than the only thing a routine can have.
--
-- `fires` is text rather than a number because the trigger after these is
-- "when a Linear issue is assigned to me", and that has to be a new value in
-- this column instead of a new column. `every:N` keeps the old meaning exactly,
-- so every existing row carries over unchanged.
--
-- `name` is blank on everything an agent set for itself, and blank stays legal:
-- a routine with no name is titled by what it does.
ALTER TABLE routines ADD COLUMN name TEXT NOT NULL DEFAULT '';
ALTER TABLE routines ADD COLUMN fires TEXT NOT NULL DEFAULT 'once';

UPDATE routines SET fires = 'every:' || every_secs WHERE every_secs IS NOT NULL;

-- Dropped rather than left as a second place the same fact could be written.
-- Neither index is on it, which is what makes this legal.
ALTER TABLE routines DROP COLUMN every_secs;
"#,
    ),
    (
        17,
        r#"
-- Agents the operator keeps at the top of the rail.
--
-- On the agent rather than in a preferences blob because it is a fact about
-- that agent and has to die with it: a name is free to reuse the moment an
-- agent is deleted, and whoever takes it next must not inherit a pin.
ALTER TABLE agents ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
"#,
    ),
    (
        18,
        r#"
-- A routine that is set up but not running.
--
-- Distinct from deleting it: an operator turning something off for a week
-- keeps the wording, the schedule and the history, and deleting was the only
-- way to stop it. Existing routines are active, which is what they were.
ALTER TABLE routines ADD COLUMN active INTEGER NOT NULL DEFAULT 1;

-- What a routine actually did.
--
-- `last_run_at` on the routine answers "is this thing alive" in one number and
-- stays; this answers "what has it been doing", which a single number cannot.
-- A test run is recorded the same way and marked as one, because the operator
-- asking whether it fired last Tuesday needs to know which of those they are
-- looking at.
--
-- `run_id` is not a foreign key: runs are not a table, they are what ties
-- together messages and usage rows, and this is the thread back to them.
CREATE TABLE routine_runs (
    id         INTEGER PRIMARY KEY,
    routine_id TEXT    NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
    run_id     TEXT    NOT NULL,
    kind       TEXT    NOT NULL,
    at         INTEGER NOT NULL
);

-- The one question asked of it: what has this routine done lately.
CREATE INDEX routine_runs_routine ON routine_runs (routine_id, at DESC);
"#,
    ),
    (
        19,
        r#"
-- One pair's exchange, for the thread the operator opens off a channel. A
-- message from A to B is filed in B's channel and the reply in A's, so neither
-- channel holds the back-and-forth and no existing index answers this. Ordered
-- by sender so each direction is one range scan; the two are unioned by
-- SQLite's OR optimisation. Partial for the same reason the old feed index
-- was: most traffic is with the operator and does not belong here.
CREATE INDEX messages_pair
    ON messages (from_agent, to_agent, created_at DESC, id DESC)
    WHERE from_kind = 'agent' AND to_kind = 'agent';
"#,
    ),
    (
        20,
        r#"
-- Where the operator put an agent in the rail.
--
-- The rail was ordered entirely by who spoke last, which is an order nobody
-- chose and one that moves under the hand reaching for it. This column is the
-- arrangement; activity now lends a row the top of its section while it works
-- and gives the place back. On the agent for the same reason `pinned` is: it is
-- a fact about that agent and has to die with it.
--
-- Backfilled in creation order, so an upgrade draws the rail it drew before,
-- and distinct from the start, so the first drag has somewhere to land. Ties
-- are still legal and are broken by `created_at`; a dense renumber on every
-- move keeps them rare rather than impossible.
ALTER TABLE agents ADD COLUMN rail_order INTEGER NOT NULL DEFAULT 0;

UPDATE agents SET rail_order = (
    SELECT COUNT(*)
      FROM agents AS earlier
     WHERE earlier.created_at < agents.created_at
        OR (earlier.created_at = agents.created_at AND earlier.rowid < agents.rowid)
);
"#,
    ),
    (
        21,
        r#"
-- A routine that is not waiting on a clock.
--
-- `fires` was made text so the trigger after the calendar ones would be a new
-- value rather than a new column, and that half held. This is the other half:
-- a trigger that is not a clock has no next firing at all, and `next_run_at`
-- was NOT NULL, so the only ways to store one were a sentinel date or a second
-- column. A sentinel is a date the operator eventually gets shown, and it is
-- one bad comparison away from firing something meant to wait for Stripe.
--
-- NULL says it plainly, and it says it to the scheduler for free: SQL compares
-- NULL to nothing, so `next_run_at <= now` skips these without the sweep
-- knowing what kinds of trigger exist.
--
-- SQLite cannot drop NOT NULL in place, so the table is rebuilt. Every routine
-- that exists today waits on a clock and carries its slot over unchanged.
--
-- The history survives the rebuild because migrations run on the bootstrap
-- connection, where `foreign_keys` is off. With it on, `DROP TABLE routines`
-- performs an implicit DELETE first and fires `routine_runs`' ON DELETE
-- CASCADE, taking every recorded firing with it. That is the same reason the
-- agents rebuild in migration 1 is written this way.
CREATE TABLE routines_new (
    id          TEXT    PRIMARY KEY,
    agent_id    TEXT    NOT NULL REFERENCES agents(id),
    name        TEXT    NOT NULL DEFAULT '',
    what        TEXT    NOT NULL,
    fires       TEXT    NOT NULL DEFAULT 'once',
    active      INTEGER NOT NULL DEFAULT 1,
    next_run_at INTEGER,
    last_run_at INTEGER,
    created_at  INTEGER NOT NULL
);

INSERT INTO routines_new (id,agent_id,name,what,fires,active,next_run_at,last_run_at,created_at)
SELECT id,agent_id,name,what,fires,active,next_run_at,last_run_at,created_at FROM routines;

DROP TABLE routines;
ALTER TABLE routines_new RENAME TO routines;

-- Both indexes go with the old table and are rebuilt. The due index is partial
-- now: a routine with no slot is never an answer to "what is due", so it has no
-- business in the index the scheduler reads on every tick.
CREATE INDEX routines_due ON routines (next_run_at) WHERE next_run_at IS NOT NULL;
CREATE INDEX routines_agent ON routines (agent_id);
"#,
    ),
    (
        22,
        r#"
-- An agent can be given a browser as well as a computer. They are different
-- things on different providers: the computer is a Linux machine with a screen,
-- worked by looking and pointing, and the browser is a hosted Chrome, worked by
-- asking the page. Only the session id is kept. The socket that drives it and
-- the URL the operator watches both change when a browser is replaced, so a
-- stored copy of either is a pane pointed at something that has gone.
ALTER TABLE agents ADD COLUMN browser_id TEXT;

-- And each of those has its own cookie jar, so a sign-in belongs to one of them
-- rather than to the agent. Rebuilt rather than altered, because the surface has
-- to join the primary key: an agent signed in to LinkedIn in both places is two
-- rows, and under the old key the second one could not be written. The scan of
-- one surface must also replace only that surface's rows, or asking the computer
-- what it holds would forget everything the browser reported.
--
-- Every existing row came from a machine, because that is all there was.
CREATE TABLE signins_next (
    agent_id      TEXT    NOT NULL REFERENCES agents(id),
    surface       TEXT    NOT NULL,
    domain        TEXT    NOT NULL,
    service       TEXT    NOT NULL,
    recognised    INTEGER NOT NULL DEFAULT 0,
    first_seen_at INTEGER NOT NULL,
    last_seen_at  INTEGER NOT NULL,
    PRIMARY KEY (agent_id, surface, domain)
);

INSERT INTO signins_next (agent_id,surface,domain,service,recognised,first_seen_at,last_seen_at)
SELECT agent_id,'computer',domain,service,recognised,first_seen_at,last_seen_at FROM signins;

DROP TABLE signins;
ALTER TABLE signins_next RENAME TO signins;

-- Dropping the table took its index with it. The one question asked of this
-- table is still "what does this agent reach".
CREATE INDEX signins_agent ON signins (agent_id);
"#,
    ),
    (
        23,
        r#"
-- A group is where a crew's settings live. It already carried an endpoint, a
-- key and a model; what was missing was the setting that decides which of those
-- are even read. A group can now name the provider that pays for its turns, so
-- one crew can run on a local server while another spends the ChatGPT plan, and
-- the app settings are what a group falls back to rather than what it obeys.
ALTER TABLE groups ADD COLUMN provider             TEXT;
ALTER TABLE groups ADD COLUMN subscription_model   TEXT;
ALTER TABLE groups ADD COLUMN request_timeout_secs INTEGER;

-- And the loop guard, which is a statement about one crew's work rather than
-- about the app: a pair drafting a document needs a handful of model calls, and
-- a crew working a browser through a long form needs an order of magnitude
-- more. NULL is inherit, per limit, so a group that has never been touched runs
-- on exactly the numbers it ran on yesterday.
ALTER TABLE groups ADD COLUMN max_hops           INTEGER;
ALTER TABLE groups ADD COLUMN max_steps_per_run  INTEGER;
ALTER TABLE groups ADD COLUMN max_fanout_per_call INTEGER;
ALTER TABLE groups ADD COLUMN max_sends_per_pair INTEGER;
ALTER TABLE groups ADD COLUMN max_tool_rounds    INTEGER;

-- One model column became two, and the split changes what the old one means.
-- It used to be "this group's model, whoever is paying"; it is now the model
-- for a key-paid turn, with the new column beside it for a subscription-paid
-- one. A group whose model is one of the subscription's own could only have
-- been running on the subscription, so it is copied across and keeps running on
-- what it was running on. The list is spelled out rather than read from the
-- code because this is a statement about the models that existed on the day
-- this migration ran, and it must not change when that list does.
UPDATE groups
   SET subscription_model = default_model
 WHERE default_model IN
       ('gpt-5.6-sol','gpt-5.6-terra','gpt-5.6-luna','gpt-5.5','gpt-5.4','gpt-5.4-mini');

-- A group that named an endpoint or a key was taken to have chosen one, because
-- until this column there was no way for it to say so. That reading is written
-- down here rather than left as a guess in the code that resolves a turn: a
-- guess there cannot be argued with, and it outvotes an operator who later
-- chooses to follow the app settings with an endpoint still in the box.
UPDATE groups
   SET provider = 'compatible'
 WHERE trim(coalesce(base_url, '')) <> '' OR trim(coalesce(api_key, '')) <> '';
"#,
    ),
    (
        24,
        r#"
-- Plugins: an MCP server a crew has signed in to, and the grant that signing in
-- produced. Beside `connectors` rather than instead of it, because the two are
-- different mechanisms with different blast radii: a connector is a secret the
-- operator pasted that ends up in the environment of a sandbox, and a plugin is
-- a grant Guaca holds and spends itself, on a call the machine never sees.
--
-- The grant columns are the reason this table is never selected whole. Nothing
-- reads `access_token`, `refresh_token` or `client_secret` except the code that
-- puts them on the wire back to the server that issued them, in the same way
-- `connector_env` is the only reader of a connector's secret.
CREATE TABLE plugins (
    id             TEXT    PRIMARY KEY,
    group_id       TEXT    NOT NULL REFERENCES groups(id),
    -- The slug from `domain::plugin::PluginKind`, not a free-text service name.
    -- The endpoint the runtime dials is derived from it, so a row naming
    -- something that is not in that enum is a row nothing can use.
    kind           TEXT    NOT NULL,
    account        TEXT    NOT NULL DEFAULT '',
    -- The server's own tool list, as it stood when the plugin was connected.
    -- Kept rather than re-read, because `tools/list` on every turn is a network
    -- round trip in front of every model call in the crew.
    tools          TEXT    NOT NULL DEFAULT '[]',
    client_id      TEXT    NOT NULL DEFAULT '',
    client_secret  TEXT    NOT NULL DEFAULT '',
    token_endpoint TEXT    NOT NULL DEFAULT '',
    access_token   TEXT    NOT NULL DEFAULT '',
    refresh_token  TEXT    NOT NULL DEFAULT '',
    expires_at     INTEGER,
    connected_at   INTEGER NOT NULL
);

CREATE INDEX plugins_group ON plugins (group_id);

-- One of each per crew. Two grants for the same server would put two copies of
-- every tool in front of the model, under names it cannot tell apart, and which
-- of the two a call landed on would depend on row order.
CREATE UNIQUE INDEX plugins_kind_unique ON plugins (group_id, kind);
"#,
    ),
    (
        25,
        r#"
-- Clerk was withdrawn from the plugin list. Its MCP server publishes two tools
-- and both return SDK snippets, so it acted on nothing and nothing it returned
-- was about the operator's account: it was documentation reached through a
-- consent screen. `PluginKind::from_slug` no longer answers to `clerk`, so these
-- rows are already invisible to every read; they are deleted rather than left
-- because a row nothing can resolve still holds the group's slot in
-- `plugins_kind_unique`, and a crew that reconnected a plugin under that name
-- would fail on a conflict with a row the UI never showed them.
--
-- No grant is lost. Clerk's server authorised nobody, so the token columns on
-- these rows are empty by construction.
DELETE FROM plugins WHERE kind = 'clerk';
"#,
    ),
    (
        26,
        r#"
-- Cloudflare moved from `bindings.mcp.cloudflare.com` to `mcp.cloudflare.com`:
-- from one product area of fifteen to the whole API behind `search` and
-- `execute`. `PluginKind::endpoint` is what the runtime dials, so an existing
-- row is now a grant issued by one server being spent against another.
--
-- Nothing on the row survives the move. The access and refresh tokens were
-- issued by the old issuer and the new one will refuse them; the stored tool
-- list names tools the new server does not have, and those are what the crew
-- is offered on every turn until something re-reads them. Left in place, an
-- agent calls `cloudflare__workers_list`, gets a 401 from a host it was never
-- signed in to, and reports it as the operator's account being broken.
--
-- Deleted rather than blanked, because an empty row still holds the group's
-- slot in `plugins_kind_unique`, and the tile the operator needs to click says
-- "Connect" only when there is no row at all. Reconnecting is one click and one
-- consent screen, and it is the only way to get a grant for the new server.
DELETE FROM plugins WHERE kind = 'cloudflare';
"#,
    ),
    (
        27,
        r#"
-- Who in a crew may call a plugin, which until now was "all of them". A group
-- signs in once and that was the whole decision, which only holds while a crew
-- is uniform. A crew is not: agents run on different models at different
-- competencies, and the one that files issues has no business holding the
-- account that issues refunds.
--
-- 'everyone' is the default here and the default in `PluginAccess::from_row`,
-- so every row that already exists keeps exactly the reach it had. Nothing an
-- operator connected yesterday narrows because this migration ran.
ALTER TABLE plugins ADD COLUMN access TEXT NOT NULL DEFAULT 'everyone';

-- The named agents. Only ever populated while `access` is 'chosen': a write
-- replaces the whole set, so flipping a plugin back to the whole crew leaves
-- nothing behind claiming otherwise. Nothing here is a memory of a choice the
-- operator has since changed.
--
-- No ON DELETE CASCADE. Foreign keys are off while migrations run, which is
-- what SQLite's table-rebuild procedure wants, so a later migration deleting a
-- plugin row would silently leave these behind; the deletes are written out at
-- every call site instead, beside the ones that already remove a group's
-- plugins and a retired agent's approvals.
CREATE TABLE plugin_agents (
    plugin_id TEXT NOT NULL REFERENCES plugins(id),
    agent_id  TEXT NOT NULL REFERENCES agents(id),
    PRIMARY KEY (plugin_id, agent_id)
);

-- Retiring an agent takes its permissions with it, and that read is by agent.
CREATE INDEX plugin_agents_agent ON plugin_agents (agent_id);
"#,
    ),
    (
        28,
        r#"
-- A computer and a browser are the operator's to hand out, one agent at a
-- time. Before this they belonged to the workspace: a key in settings meant
-- every agent in it was offered `run_command`, `use_screen` and `browse`, and
-- the first one to think of it made itself a machine. An operator who wanted a
-- crew where one agent reads the web and the rest only talk had no way to say
-- so, and no way to find out an agent had rented a machine except the bill.
ALTER TABLE agents ADD COLUMN has_computer INTEGER NOT NULL DEFAULT 0;
ALTER TABLE agents ADD COLUMN has_browser  INTEGER NOT NULL DEFAULT 0;

-- Backfilled from what each agent is holding rather than from the old rule.
-- The old rule was "everyone", and applying it here would upgrade a workspace
-- into exactly the state this column exists to end. What an agent already has
-- is the one thing that cannot be taken away silently: its machine's disk and
-- its browser profile are where the operator's sign-ins live, and an agent cut
-- off from them reports accounts it can see and cannot reach.
UPDATE agents SET has_computer = 1 WHERE sandbox_id IS NOT NULL;
UPDATE agents SET has_browser  = 1 WHERE browser_id IS NOT NULL;
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
    // Foreign keys off for the duration, which is what SQLite's own procedure
    // for rebuilding a table asks for and what a migration cannot do for
    // itself: the pragma is a no-op inside a transaction, and every migration
    // runs in one. With enforcement on, the `DROP TABLE` in a rebuild performs
    // an implicit DELETE first and fires the ON DELETE CASCADE of everything
    // pointing at that table, so migration 21 took every routine's recorded
    // firings with it.
    //
    // Nothing is lost by it here. Migrations are DDL written in this file, not
    // input, and the connection this runs on exists only to run them: the pool
    // the app actually works through turns enforcement on for every connection
    // in `Store::open`. Restored anyway, because tests call this directly.
    let enforced: bool = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    if enforced {
        conn.pragma_update(None, "foreign_keys", false)?;
    }
    let applied = apply(conn, latest_version());
    if enforced {
        // Best effort: whatever the migrations said is the answer worth having.
        let _ = conn.pragma_update(None, "foreign_keys", true);
    }
    applied
}

fn apply(conn: &mut Connection, target: i32) -> Result<i32, MigrationError> {
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
    fn a_withdrawn_plugin_takes_its_rows_and_leaves_the_others() {
        // The slot in `plugins_kind_unique` is the point: a row nothing can
        // resolve is invisible to every read but still owns (group, kind), so a
        // crew reconnecting under that name would fail on a conflict with a row
        // the UI never drew for them.
        let mut conn = memory();
        // Staged by hand rather than with `apply`, which takes the next
        // migration off the list without looking at the target and would run
        // the one being tested.
        for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v < 25) {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", *version).unwrap();
        }
        let insert = "INSERT INTO plugins (id,group_id,kind,account,tools,connected_at)
                      VALUES (?1,?2,?3,'','[]',0)";
        conn.execute(insert, rusqlite::params!["p1", DEFAULT_GROUP_ID, "clerk"]).unwrap();
        conn.execute(insert, rusqlite::params!["p2", DEFAULT_GROUP_ID, "neon"]).unwrap();

        run(&mut conn).unwrap();

        let left: Vec<String> = conn
            .prepare("SELECT kind FROM plugins ORDER BY kind")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(left, vec!["neon".to_string()]);
    }

    #[test]
    fn a_plugin_that_changed_server_loses_the_grant_it_had_for_the_old_one() {
        // Cloudflare's endpoint moved hosts, so the stored token was issued by
        // an issuer the new server does not share and the stored tool list
        // names tools it does not have. Both are read on every turn, and a row
        // that survives is an agent calling a tool that 401s against a host
        // nobody signed in to.
        let mut conn = memory();
        for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v < 26) {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", *version).unwrap();
        }
        let insert =
            "INSERT INTO plugins (id,group_id,kind,account,tools,access_token,connected_at)
                      VALUES (?1,?2,?3,'','[\"workers_list\"]','tok',0)";
        conn.execute(insert, rusqlite::params!["p1", DEFAULT_GROUP_ID, "cloudflare"]).unwrap();
        conn.execute(insert, rusqlite::params!["p2", DEFAULT_GROUP_ID, "linear"]).unwrap();

        run(&mut conn).unwrap();

        let left: Vec<String> = conn
            .prepare("SELECT kind FROM plugins ORDER BY kind")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(left, vec!["linear".to_string()], "and only Cloudflare's");
    }

    #[test]
    fn a_plugin_connected_before_agents_could_be_chosen_stays_the_whole_crew_s() {
        // The one thing this migration must not do. A crew whose Neon sign-in
        // worked yesterday has to work today, without the operator opening a
        // panel they have never seen to re-grant what they already had.
        let mut conn = memory();
        for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v < 27) {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", *version).unwrap();
        }
        conn.execute(
            "INSERT INTO plugins (id,group_id,kind,account,tools,connected_at)
             VALUES ('p1',?1,'neon','','[]',0)",
            rusqlite::params![DEFAULT_GROUP_ID],
        )
        .unwrap();

        run(&mut conn).unwrap();

        let access: String = conn
            .query_row("SELECT access FROM plugins WHERE id='p1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(access, "everyone");
        let named: i64 =
            conn.query_row("SELECT count(*) FROM plugin_agents", [], |row| row.get(0)).unwrap();
        assert_eq!(named, 0, "nobody was named, and nobody needs to be");
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
    fn a_group_pinned_to_a_subscription_model_keeps_running_on_it() {
        // The one column became two, and the split changed what the old one
        // means. A group whose model is one of the subscription's own was being
        // spent on the subscription; leaving it behind would move that crew to
        // whatever the app happens to run, silently and on the next turn.
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        for (version, sql) in MIGRATIONS.iter().take_while(|(v, _)| *v < 23) {
            tx.execute_batch(sql).unwrap();
            tx.pragma_update(None, "user_version", *version).unwrap();
        }
        tx.execute(
            "INSERT INTO groups (id,name,created_at,default_model) VALUES ('g1','Plan',1,'gpt-5.4')",
            [],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO groups (id,name,created_at,default_model)
             VALUES ('g2','Local',2,'local/qwen')",
            [],
        )
        .unwrap();
        tx.commit().unwrap();

        run(&mut conn).unwrap();

        let carried: Option<String> = conn
            .query_row("SELECT subscription_model FROM groups WHERE id='g1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(carried.as_deref(), Some("gpt-5.4"));

        // And a model that belongs to an endpoint is left where it was: copying
        // it would pin a crew to a model the subscription cannot run the moment
        // anyone moved it across.
        let untouched: Option<String> = conn
            .query_row("SELECT subscription_model FROM groups WHERE id='g2'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(untouched, None);
    }

    #[test]
    fn a_group_that_already_had_an_endpoint_is_written_down_as_choosing_one() {
        // What it was doing yesterday, said out loud, so that nothing has to
        // guess it tomorrow.
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        for (version, sql) in MIGRATIONS.iter().take_while(|(v, _)| *v < 23) {
            tx.execute_batch(sql).unwrap();
            tx.pragma_update(None, "user_version", *version).unwrap();
        }
        tx.execute(
            "INSERT INTO groups (id,name,created_at,base_url) VALUES ('g1','Local',1,'http://x/v1')",
            [],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO groups (id,name,created_at,api_key) VALUES ('g2','Keyed',2,'sk-x')",
            [],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO groups (id,name,created_at,default_model)
             VALUES ('g3','Model only',3,'some/model')",
            [],
        )
        .unwrap();
        tx.commit().unwrap();

        run(&mut conn).unwrap();

        let provider = |id: &str| -> Option<String> {
            conn.query_row("SELECT provider FROM groups WHERE id=?1", [id], |r| r.get(0)).unwrap()
        };
        assert_eq!(provider("g1").as_deref(), Some("compatible"));
        assert_eq!(provider("g2").as_deref(), Some("compatible"));
        // A group that only pinned a model never said anything about who pays,
        // and must keep saying nothing.
        assert_eq!(provider("g3"), None);
    }

    #[test]
    fn group_limits_start_unset_and_mean_inherit() {
        // A number here would be a limit nobody chose, and it would override the
        // app's the first time either was retuned.
        let mut conn = memory();
        run(&mut conn).unwrap();
        let (steps, provider): (Option<i64>, Option<String>) = conn
            .query_row(
                "SELECT max_steps_per_run, provider FROM groups WHERE id=?1",
                [DEFAULT_GROUP_ID],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(steps, None);
        assert_eq!(provider, None, "a fresh group is paid for however the app is");
    }

    #[test]
    fn an_agent_that_already_had_a_machine_keeps_it_when_the_two_places_become_a_decision() {
        // The upgrade cannot apply the old rule, because the old rule was
        // "every agent in the workspace" and that is the state this column
        // exists to end. It cannot apply the new default to everybody either:
        // an agent already holding a machine is holding the operator's sign-ins
        // on its disk, and one cut off from them reports accounts it can see
        // and cannot reach. What it is holding is the only honest answer.
        let mut conn = memory();
        for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v < 28) {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", *version).unwrap();
        }
        let insert = "INSERT INTO agents
                        (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,
                         created_at,updated_at,group_id,sandbox_id,browser_id)
                      VALUES (?1,?2,'avocado','#7fb069','m','','[]','active',1,0,0,?3,?4,?5)";
        let crew: [(&str, Option<&str>, Option<&str>); 3] = [
            ("Runner", Some("sb-1"), None),
            ("Reader", None, Some("kb-1")),
            ("Talker", None, None),
        ];
        for (name, sandbox, browser) in crew {
            conn.execute(insert, rusqlite::params![name, name, DEFAULT_GROUP_ID, sandbox, browser])
                .unwrap();
        }

        run(&mut conn).unwrap();

        let held = |name: &str| -> (i64, i64) {
            conn.query_row(
                "SELECT has_computer, has_browser FROM agents WHERE id=?1",
                rusqlite::params![name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(held("Runner"), (1, 0), "a machine it is using is not taken away");
        assert_eq!(held("Reader"), (0, 1), "and neither is a browser holding its cookies");
        assert_eq!(
            held("Talker"),
            (0, 0),
            "but an agent that never needed either is not handed both on upgrade"
        );
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
    fn an_existing_schedule_keeps_firing_on_exactly_the_gap_it_was_set_for() {
        // The upgrade that could quietly change what a crew is already doing:
        // a routine written as a number of seconds has to come out the other
        // side saying the same thing, and a one-shot has to stay a one-shot.
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        for (version, sql) in MIGRATIONS.iter().take(15) {
            tx.execute_batch(sql).unwrap();
            tx.pragma_update(None, "user_version", *version).unwrap();
        }
        tx.commit().unwrap();

        conn.execute(
            "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id)
             VALUES ('a','Manager','avocado','#000','m','','[]','active',1,0,0,?1)",
            [DEFAULT_GROUP_ID],
        )
        .unwrap();
        let row = "INSERT INTO routines (id,agent_id,what,every_secs,next_run_at,created_at)
                   VALUES (?1,'a',?2,?3,0,0)";
        conn.execute(row, rusqlite::params!["r1", "check the listings", Some(18_000)]).unwrap();
        conn.execute(row, rusqlite::params!["r2", "wake me", None::<u32>]).unwrap();

        run(&mut conn).unwrap();

        let fires = |id: &str| -> String {
            conn.query_row("SELECT fires FROM routines WHERE id=?1", [id], |r| r.get(0)).unwrap()
        };
        assert_eq!(fires("r1"), "every:18000", "a five-hour repeat stays a five-hour repeat");
        assert_eq!(fires("r2"), "once", "a one-shot must not become a repeat");

        let name: String =
            conn.query_row("SELECT name FROM routines WHERE id='r1'", [], |r| r.get(0)).unwrap();
        assert_eq!(name, "", "nothing invents a name for a routine an agent set");
    }

    #[test]
    fn rebuilding_the_routines_table_keeps_every_schedule_and_its_history() {
        // The one migration that drops a table other rows point at. With
        // foreign keys on, DROP TABLE fires `routine_runs`' ON DELETE CASCADE
        // and takes every recorded firing with it, so this checks the history
        // is still there and the slots came over unchanged.
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        for (version, sql) in MIGRATIONS.iter().take(20) {
            tx.execute_batch(sql).unwrap();
            tx.pragma_update(None, "user_version", *version).unwrap();
        }
        tx.commit().unwrap();

        conn.execute(
            "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id)
             VALUES ('a','Manager','avocado','#000','m','','[]','active',1,0,0,?1)",
            [DEFAULT_GROUP_ID],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO routines (id,agent_id,name,what,fires,active,next_run_at,last_run_at,created_at)
             VALUES ('r1','a','Sweep','check','weekdays',0,1750000000000,1740000000000,5)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO routine_runs (routine_id,run_id,kind,at) VALUES ('r1','run-1','test',7)",
            [],
        )
        .unwrap();

        run(&mut conn).unwrap();

        let (name, fires, active, next, last, created): (String, String, i64, i64, i64, i64) = conn
            .query_row(
                "SELECT name,fires,active,next_run_at,last_run_at,created_at FROM routines
                  WHERE id='r1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .unwrap();
        assert_eq!(
            (name.as_str(), fires.as_str(), active, next, last, created),
            ("Sweep", "weekdays", 0, 1750000000000, 1740000000000, 5),
            "every column came over as it was, including being switched off"
        );

        let runs: i64 =
            conn.query_row("SELECT count(*) FROM routine_runs", [], |r| r.get(0)).unwrap();
        assert_eq!(runs, 1, "the history must not be cascaded away by the rebuild");

        // And the point of the rebuild: a routine with no slot is now storable.
        conn.execute(
            "INSERT INTO routines (id,agent_id,name,what,fires,active,next_run_at,created_at)
             VALUES ('r2','a','Dunning','chase','event:stripe/invoice.payment_failed',1,NULL,0)",
            [],
        )
        .unwrap();
        let due: i64 = conn
            .query_row("SELECT count(*) FROM routines WHERE next_run_at <= ?1", [i64::MAX], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(due, 1, "and it is not due, however far ahead you look");
    }

    #[test]
    fn foreign_key_enforcement_is_off_only_while_migrations_run() {
        // A rebuild needs it off; everything after this must not inherit that.
        // The pool sets it per connection, but `run` is handed a connection
        // somebody else keeps, and leaving it off there disables every cascade
        // in the app.
        let mut conn = memory();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        run(&mut conn).unwrap();
        let on: bool = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
        assert!(on, "enforcement has to come back on");
    }

    #[test]
    fn an_existing_agent_is_not_pinned() {
        let mut conn = memory();
        run(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id)
             VALUES ('a','Manager','avocado','#000','m','','[]','active',1,0,0,?1)",
            [DEFAULT_GROUP_ID],
        )
        .unwrap();
        let pinned: i64 =
            conn.query_row("SELECT pinned FROM agents WHERE id='a'", [], |r| r.get(0)).unwrap();
        assert_eq!(pinned, 0, "an upgrade must not rearrange the rail");
    }

    #[test]
    fn an_upgrade_arranges_the_rail_in_the_order_it_was_already_drawn() {
        // The rail was ordered by who spoke last, and creation order underneath
        // that. Backfilling anything else would rearrange a workspace the
        // operator has been looking at for weeks, on launch, with no gesture.
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        for (version, sql) in MIGRATIONS.iter().take_while(|(v, _)| *v < 20) {
            tx.execute_batch(sql).unwrap();
            tx.pragma_update(None, "user_version", *version).unwrap();
        }
        tx.commit().unwrap();

        let row = "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,group_id)
                   VALUES (?1,?1,'avocado','#000','m','','[]','active',1,?2,?2,?3)";
        for (id, made) in [("late", 300), ("early", 100), ("middle", 200)] {
            conn.execute(row, rusqlite::params![id, made, DEFAULT_GROUP_ID]).unwrap();
        }

        run(&mut conn).unwrap();

        let arranged: Vec<(String, i64)> = conn
            .prepare("SELECT id, rail_order FROM agents ORDER BY rail_order")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            arranged,
            vec![("early".into(), 0), ("middle".into(), 1), ("late".into(), 2)],
            "an upgrade must draw the rail it drew before, and give every row its own place"
        );
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
