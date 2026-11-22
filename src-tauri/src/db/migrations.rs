use rusqlite::{Connection, Transaction, TransactionBehavior};

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
    (
        29,
        r#"
-- Which of a plugin's tools a crew may call, which until now was "all of
-- them". Connecting a server was one decision covering everything it publishes,
-- and a server does not publish one kind of thing: Stripe lists the call that
-- reads an invoice beside the one that refunds it, and Neon lists `run_sql`
-- beside the call that deletes a project. An operator who wanted the reading
-- and not the writing had one control, and it was Disconnect.
--
-- The refusals are what is written down, not the permissions. A row here is a
-- tool the operator switched off; everything else is on. That is the same
-- reading `access` takes with 'everyone', and for the same reason: the default
-- has to cover what nobody has seen yet. A vendor ships a tool between one
-- connection and the next, and an allow-list would leave it switched off with
-- nothing on screen saying a decision had been made about it.
--
-- Keyed by name rather than by an index into the stored list, because that
-- list is replaced wholesale every time the plugin is connected again. An
-- index would silently move a refusal onto whatever tool the vendor put in
-- that position.
--
-- No ON DELETE CASCADE, for the reason `plugin_agents` has none: foreign keys
-- are off while migrations run, so the deletes are written out at every call
-- site instead.
CREATE TABLE plugin_denied_tools (
    plugin_id TEXT NOT NULL REFERENCES plugins(id),
    tool      TEXT NOT NULL,
    PRIMARY KEY (plugin_id, tool)
);
"#,
    ),
    (
        30,
        r#"
-- Which authorized identity a plugin uses, for the one kind whose sign-in is
-- the operator's Guaca account rather than its own.
--
-- A person can authorize the same provider twice: a work Google and a personal
-- one are two grants at guaca.bot, with two ids, and revoking one says nothing
-- about the other. Until now a group had no way to say which it meant, so both
-- of an operator's crews reached whichever row came back first and the second
-- account was unreachable.
--
-- Empty means unnamed, which is the account's default connection and is what
-- every plugin connected before this column existed keeps doing. That matters:
-- an upgrade must not silently repoint a working crew at a different mailbox.
-- The endpoint is derived from it, so a value here is the difference between
-- `/mcp` and `/mcp/<id>`.
--
-- Only meaningful for an account-backed kind. The other five sign in per group
-- and their grant already names the identity it was issued to.
ALTER TABLE plugins ADD COLUMN connection TEXT NOT NULL DEFAULT '';
"#,
    ),
    (
        31,
        r#"
-- A tool was on or off for the whole crew, and that is one answer short. Two
-- agents share a plugin and want different halves of it: the one that triages
-- the inbox reads and searches, and the one that answers it sends. Under the
-- old shape the operator could give both of them everything, or take sending
-- away from both, and there was nothing in between.
--
-- So a tool takes the same two-state answer a plugin already takes, one level
-- down. 'everyone' is every agent the plugin itself reaches; 'chosen' is the
-- named ones and nobody else, and the empty list is a real state — it is what
-- the old table said, and it is where an operator stands for the second
-- between narrowing a tool and ticking the first name.
--
-- The absence of a row is still the permission. A tool nobody has touched is
-- on for whoever the plugin is on for, which is what every plugin connected
-- before either control existed does and what a tool the vendor ships next
-- month does. An allow-list over tools would switch that new tool off with
-- nothing on screen saying a decision had been taken; an allow-list over
-- *agents within a narrowed tool* is the opposite case and the right way
-- round, because an agent hired next week must not inherit the one capability
-- the operator went out of their way to fence off.
CREATE TABLE plugin_tool_access (
    plugin_id TEXT NOT NULL REFERENCES plugins(id),
    tool      TEXT NOT NULL,
    -- 'everyone' or 'chosen'. Compared, never parsed: see ACCESS_EVERYONE.
    access    TEXT NOT NULL,
    PRIMARY KEY (plugin_id, tool)
);

-- The named agents, per tool. Only ever populated while that tool's `access`
-- is 'chosen', because a write replaces the whole set.
--
-- No ON DELETE CASCADE, for the reason `plugin_agents` has none: foreign keys
-- are off while migrations run, so the deletes are written out at every call
-- site instead.
CREATE TABLE plugin_tool_agents (
    plugin_id TEXT NOT NULL REFERENCES plugins(id),
    tool      TEXT NOT NULL,
    agent_id  TEXT NOT NULL REFERENCES agents(id),
    PRIMARY KEY (plugin_id, tool, agent_id)
);

-- Retiring an agent takes its permissions with it, and that read is by agent.
CREATE INDEX plugin_tool_agents_agent ON plugin_tool_agents (agent_id);

-- Every refusal the old table held becomes a tool narrowed to nobody, which is
-- the same decision written in the new shape: switched off for the crew, and
-- one tick away from being switched on for one agent. Nothing an operator
-- decided yesterday changes because this migration ran.
INSERT INTO plugin_tool_access (plugin_id, tool, access)
     SELECT plugin_id, tool, 'chosen' FROM plugin_denied_tools;

DROP TABLE plugin_denied_tools;
"#,
    ),
    (
        32,
        r#"
-- The one British spelling left in the schema. Every other column this app
-- wrote is American (`color` has been on `agents` since migration 1), and the
-- Rust field and the IPC field this column feeds are `recognized` now, so
-- without this the four statements in `store.rs` name a column spelled the
-- other way and only positional row access hides it.
--
-- A rename, not a rebuild: nothing else references the column, and RENAME
-- COLUMN leaves every row and the primary key exactly where they are.
ALTER TABLE signins RENAME COLUMN recognised TO recognized;
"#,
    ),
    (
        33,
        r#"
-- An agent could stop and ask the operator two things, and both of them were
-- "may I". There was no way to ask "which of these", so an agent that needed a
-- judgment call wrote a message into a channel nobody was watching and then
-- either guessed or stalled. In a workspace with a dozen crews that is the
-- common case and it is invisible: nothing parks, nothing is counted, and the
-- operator finds out when the work comes back wrong.
--
-- The machinery for stopping a turn on a person already exists in this table,
-- so a question is a row in it rather than a second table with the same
-- lifecycle. Two columns, and no third to say which kind a row is: `action`
-- already discriminates, because a question stores the literal 'question'
-- there and that is not one of the two protected actions. A separate `kind`
-- column would be a second value that has to agree with the first, with
-- nothing keeping them in step.
--
-- Neither index needs touching. `approvals_pending` is on `created_at` and a
-- question waits exactly as a permission does; `approvals_granted` is
-- `WHERE state = 'alwaysAllow'`, which a question can never reach, because
-- there is no standing yes to a question and nothing to be let off asking.

-- What the operator may pick, as a JSON array. NULL is a question that takes a
-- written answer, and it is also every permission ever recorded, which is why
-- this cannot be NOT NULL with a default of '[]': an empty list already means
-- something on a question.
ALTER TABLE approvals ADD COLUMN options TEXT;

-- What they picked or wrote. Only ever set on a question: a verdict is a state
-- and lives in `state`, so filling this in for one would be recording the same
-- fact twice in two shapes.
ALTER TABLE approvals ADD COLUMN answer TEXT;
"#,
    ),
    (
        34,
        r#"
-- A routine that must not land on an agent which is already working. Off for
-- every routine that exists: a schedule an operator set last week goes on
-- firing exactly as it has been.
ALTER TABLE routines ADD COLUMN skip_if_working INTEGER NOT NULL DEFAULT 0;

-- A skipped firing is a row in this history with no run behind it. `run_id` is
-- the thread back to what a firing produced: the messages in the channel, the
-- model calls on the bill. A firing that was deliberately not delivered
-- produced neither, so the column has to be able to say so. An invented id
-- would read back as a delivery that spent nothing, which is the one thing
-- this history exists to tell apart.
--
-- SQLite cannot drop NOT NULL in place, so the table is rebuilt. Nothing
-- references it, so the DROP takes nothing else with it; the index does not
-- survive a rebuild and is made again below.
CREATE TABLE routine_runs_new (
    id         INTEGER PRIMARY KEY,
    routine_id TEXT    NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
    run_id     TEXT,
    kind       TEXT    NOT NULL,
    at         INTEGER NOT NULL
);

INSERT INTO routine_runs_new (id,routine_id,run_id,kind,at)
     SELECT id,routine_id,run_id,kind,at FROM routine_runs;

DROP TABLE routine_runs;
ALTER TABLE routine_runs_new RENAME TO routine_runs;

CREATE INDEX routine_runs_routine ON routine_runs (routine_id, at DESC);
"#,
    ),
    (
        35,
        r#"
-- Until now `kind` was a slug out of a closed enum, and the address the runtime
-- dialled was derived from it. That is still true of the six servers Guaca
-- ships, and it stays true of them: where a vendor's server lives is a decision
-- this build makes and re-makes on every release, so a stored copy would keep a
-- crew dialling the old host after the vendor moved — which is the failure
-- migration 26 exists to clean up after.
--
-- A server the operator added has nowhere else to keep it. `kind` holds the
-- name they gave it, which is also the prefix its tools are called by, and this
-- column holds the address. Empty means "the catalog knows where this is", so
-- every row written before today keeps meaning exactly what it meant, and a row
-- with neither a catalog slug nor an address is a row nothing can dial — which
-- is what a newer build's plugin looks like after a downgrade, and is skipped
-- rather than raised.
--
-- `plugins_kind_unique` needs no change and is doing more work than it was: it
-- was one row per vendor per crew, and it is now also what stops two servers in
-- one crew sharing a name, which would put two tool lists under one prefix and
-- make which one a call landed on depend on row order.
ALTER TABLE plugins ADD COLUMN endpoint TEXT NOT NULL DEFAULT '';
"#,
    ),
    (
        36,
        r#"
-- Headers the operator wrote, for a server they run themselves.
--
-- The third thing such a server can want after an address and a token, and the
-- one the catalog never needs: an `X-API-Key` because that is where its
-- framework looks, a pair of `Cf-Access-Client-*` because it is behind
-- Cloudflare Access, a tenant id because one deployment serves several. None of
-- them is discoverable, so there is nowhere for them to come from but the
-- person who deployed the thing.
--
-- A grant column in everything but name. Every value here is a secret and this
-- column is read by the one function that puts it on the wire, exactly as
-- `access_token` is, and `Plugin` carries the names without the values for the
-- same reason `connectors` does not return `secret`.
--
-- `'[]'` is a server that needs none, which is every row written before today
-- and almost every row written after it.
ALTER TABLE plugins ADD COLUMN headers TEXT NOT NULL DEFAULT '[]';
"#,
    ),
    (
        37,
        r#"
-- A directory on this machine that a crew may write code in. Scoped to the
-- group like everything else an agent can see, and holding no secret: a path is
-- not a credential, which is why it is a plain column and not the shape
-- `connectors` uses.
--
-- `path` is stored canonical and without a trailing separator, so the index
-- below can hold. Two spellings of one directory would be two repositories over
-- one tree, each with its own reach, and the operator would fix one and wonder
-- why nothing changed.
CREATE TABLE repositories (
    id         TEXT    PRIMARY KEY,
    group_id   TEXT    NOT NULL REFERENCES groups(id),
    name       TEXT    NOT NULL,
    path       TEXT    NOT NULL,
    note       TEXT    NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX repositories_group_path ON repositories (group_id, path);

-- Which agents may work in one. Named, always: there is no row shape here that
-- means everybody, because an agent hired next week must not inherit a working
-- tree the operator handed to somebody in particular. `domain/repository.rs`
-- argues it against the plugin tables next door, which do have an everybody and
-- have a different reason to.
CREATE TABLE repository_access (
    repository_id TEXT NOT NULL REFERENCES repositories(id),
    agent_id      TEXT NOT NULL REFERENCES agents(id),
    PRIMARY KEY (repository_id, agent_id)
);

-- Retiring an agent takes its repositories with it, and that read is by agent.
CREATE INDEX repository_access_agent ON repository_access (agent_id);
"#,
    ),
    (
        38,
        r#"
-- An agent works in at most one repository. That is a decision about
-- coordination rather than about permissions: two agents on one codebase settle
-- it between themselves in the crew they share, and one agent quietly holding
-- two is a change whose shape nobody can see until it lands in both.
--
-- A column rather than the junction table it replaces, because the rule is
-- "at most one" and a column is the only shape that cannot represent anything
-- else. It also makes the rail a tree: the repository is a heading and its
-- agents are under it, each drawn once, which a many-to-many cannot be.
ALTER TABLE agents ADD COLUMN repository_id TEXT REFERENCES repositories(id);

-- Whoever was named on one keeps it. An agent named on more than one keeps the
-- first it was given: the rows are in the order the operator ticked them, so
-- the first is the one they chose before there was a rule about it.
UPDATE agents SET repository_id = (
    SELECT repository_id FROM repository_access
     WHERE repository_access.agent_id = agents.id
     ORDER BY rowid LIMIT 1
);

DROP TABLE repository_access;
"#,
    ),
    (
        39,
        r#"
-- Working notes: what an agent is in the middle of, as against what it knows.
--
-- Memory is a file the agent rewrites, and that shape is right for what memory
-- holds: a small page of durable belief, reconciled against itself on every
-- write. It is the wrong shape for progress. A rewrite re-emits the whole page,
-- and copying a stale line forward is cheaper than deciding to drop it, so
-- progress written into memory ratchets: sixteen of this operator's twenty-three
-- agents had a "Waiting on" or "Status" section, and a fifth of everything in
-- every memory file was task state that had stopped being true.
--
-- So this is a table and not a second file, and the difference is the point.
-- One row per note, appended, never rewritten. The oldest fall off on their own
-- once there are enough of them, which means no agent ever has to decide to
-- forget: the operation LLMs are measurably worst at is the one this store does
-- not ask for.
--
-- A table rather than a file for a second reason. An append is a read-modify-
-- write, and an agent's memory is written from every thread it holds, so an
-- append to a file loses notes under exactly the concurrency this app has. An
-- INSERT does not.
CREATE TABLE working_notes (
    id       INTEGER PRIMARY KEY,
    agent_id TEXT    NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    at       INTEGER NOT NULL,
    body     TEXT    NOT NULL
);

-- The only question asked of it: what is this agent in the middle of. `id DESC`
-- rather than `at DESC` because two notes from one turn share a timestamp, and
-- the order they were written in is the order they are worth reading in.
CREATE INDEX working_notes_agent ON working_notes (agent_id, id DESC);
"#,
    ),
    (
        40,
        r#"
-- Which coding harness a job in this directory starts.
--
-- Two programs, because a subscription is spent by the program it was issued to
-- and by no other. `pi` holding an Anthropic OAuth credential and dialling the
-- Messages API with it is refused with `You're out of extra usage` while
-- `claude` on the same machine and the same account runs the work off the plan.
-- An operator whose ChatGPT plan is spent and whose Claude plan is not cannot be
-- helped by configuring one harness; they need the other program.
--
-- `'pi'` is what every row written before today was already running, so the
-- default is a statement of fact rather than a preference. Unrecognized values
-- read back as `pi` rather than failing the row: see `Harness::parse`.
ALTER TABLE repositories ADD COLUMN harness TEXT NOT NULL DEFAULT 'pi';
"#,
    ),
    (
        41,
        r#"
-- When an agent was thrown out, for the thirty days it can still be pulled back.
--
-- Deleting used to be one act: the machines destroyed, the memory removed, the
-- schedule, the sign-ins and every standing permission deleted, and the row
-- marked terminated. All of it on a button an operator presses by accident, on
-- an agent that had six months of memory in it, with nothing between the click
-- and the loss but a menu item.
--
-- So the destructive half now waits. A delete stamps this column and stops the
-- agent; everything it holds privately stays exactly where it is until the
-- thirty days are up, at which point the old act runs in full. `NULL` is both
-- ends of that: an agent nobody has deleted, and one whose wait is over and
-- whose things are already gone. Which of the two a row is, its `lifecycle`
-- says.
--
-- Nothing else changes about `terminated`, and that is why this is a column
-- rather than a fourth lifecycle. An agent in the compost is unreachable,
-- undiscoverable, out of the rail and out of every crew, exactly as a deleted
-- one has always been: every query in the store that asks `lifecycle <>
-- 'terminated'` is still asking the right question, including the partial
-- index that frees the name. What the column adds is a way back, which nothing
-- else was asking about.
ALTER TABLE agents ADD COLUMN discarded_at INTEGER;
"#,
    ),
    (
        42,
        r#"
-- Whether a coding job in this directory stops before it reaches outside it.
--
-- A push, a pull request, a merge or a release is the operator's own name going
-- somewhere git cannot take it back from. Until a job could be reached while it
-- was running there was no way to ask about one, so every job did all of them
-- unattended and the only boundary was the directory and the undo. There is a
-- way now, and this is where an operator says they want it used here.
--
-- `'open'` is what every job before today did, so the default is a statement of
-- fact rather than a preference. It is also the only safe default for a second
-- reason: `coding::APPENDED_PROMPT` tells a job that nobody will answer a
-- question, and a value that made that false everywhere at once would hold jobs
-- on a desk in workspaces nobody is watching.
--
-- Unrecognized values read back as `open` rather than failing the row, which is
-- `Harness::parse`'s reasoning plus one of its own: reading an unknown value as
-- the asking variant would park jobs over a string a downgrade wrote.
ALTER TABLE repositories ADD COLUMN gate TEXT NOT NULL DEFAULT 'open';
"#,
    ),
    (
        43,
        r#"
-- An escalation: work that has stopped and that only the operator can move.
--
-- There were two ways for an agent to reach a person and both of them park the
-- turn inside a tool call, waiting ten minutes for an answer. Both are right
-- for a decision the turn needs now. Neither is right for the case they kept
-- being reached for anyway, which is an agent that cannot go on at all: a
-- coding harness that will not start, a sign-in that has expired, a machine
-- only the operator can touch. None of that is answerable inside a turn and
-- none of it stops being true because ten minutes passed, so the agent wrote it
-- into its channel instead, clearly, addressed to somebody who was not reading
-- it. Five turns of one crew went that way before anybody noticed, and what
-- made it invisible is that the three surfaces built for exactly this -- the
-- count on a crew's circle, the desk, the menu bar -- are all fed from
-- `approvals`. Nothing had parked, so all three said the workspace was fine.
--
-- Nothing parks here either, and that is the difference rather than an
-- omission: the turn ends, no run booking is held, and so there is no window,
-- no expiry, and no cost to a row staying open for two days. Nothing is
-- answered either. Clearing is the operator saying they have dealt with it;
-- what actually unblocks the agent is a message in the channel this row opens.
--
-- One open row per agent. An agent that hits the same wall on six turns must
-- not fill the desk with six rows saying one thing, and must not be told to
-- keep quiet either: the sixth raise restates the line, counts, and leaves
-- `raised_at` exactly where it is. `raised_at` says how long this has been
-- true, `times` says how many turns have hit it, and the pair is what says "a
-- crew has been stuck for two days" rather than "an agent mentioned something".
CREATE TABLE escalations (
    id         TEXT    PRIMARY KEY,
    agent_id   TEXT    NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    group_id   TEXT    NOT NULL,
    run_id     TEXT    NOT NULL,
    summary    TEXT    NOT NULL,
    raised_at  INTEGER NOT NULL,
    said_at    INTEGER NOT NULL,
    times      INTEGER NOT NULL DEFAULT 1,
    cleared_at INTEGER
);

-- One open escalation per agent, enforced here rather than by whoever writes
-- one. A second open row is the failure the desk exists to prevent, one level
-- down: a queue that says six things when six is one thing said six times.
CREATE UNIQUE INDEX escalations_open_per_agent
    ON escalations (agent_id)
    WHERE cleared_at IS NULL;

-- What the desk, the crews' column and the menu bar all read: what is still
-- open, oldest first. Partial, so it stays the size of the problem rather than
-- the size of the history.
CREATE INDEX escalations_open ON escalations (raised_at) WHERE cleared_at IS NULL;
"#,
    ),
    (
        44,
        r#"
-- Where a coding job in this directory actually runs.
--
-- Until today it ran in the linked directory, which is the operator's own
-- checkout, and that made three things true at once. The job and the operator
-- shared one branch. Two agents in one codebase could not work at the same
-- time, because `Runtime::start_job` takes a lock per work tree and there was
-- only ever one. And a job that opened a pull request left the tree standing on
-- the branch it made, so the rail went on reporting a feature branch for the
-- weeks after it landed.
--
-- `'own'` gives each agent a linked git worktree of its own, off the same
-- repository, under the app's data directory. The operator's checkout is never
-- checked out, never switched and never cleaned; Guaca owns the other tree and
-- so can reset it to the default branch before every job.
--
-- `'shared'` is what every row written before today was doing, so that is what
-- they are backfilled with. This is the one column in this table whose SQL
-- default and whose Rust default disagree, and the disagreement is the point:
-- `ALTER TABLE ... DEFAULT` only ever runs against rows that already exist, and
-- moving somebody's jobs into a new directory is not an upgrade's decision to
-- take. What a *new* repository gets is `Bench::default`, which is `own`, and
-- `create_repository` always writes the value explicitly.
--
-- Unrecognized values read back as `shared` rather than failing the row, for
-- `Harness::parse`'s reason and `Gate::parse`'s: reading a string a downgrade
-- wrote as the variant that relocates work is the wrong direction to be wrong
-- in.
ALTER TABLE repositories ADD COLUMN bench TEXT NOT NULL DEFAULT 'shared';
"#,
    ),
    (
        45,
        r#"
-- Whether an agent's browser stops and asks before it acts in the operator's
-- name.
--
-- The gate this switches off fires on a press or a typed line, on a site the
-- browser holds a session for, in a turn that has already read a page. It was
-- written for an agent working one inbox, where the grant it collects lasts the
-- rest of the turn and the agent never leaves the domain. An agent doing
-- research leaves it on every cycle: search, read a result off-domain, come
-- back, ask again. The live report was a dialog every few seconds, which is a
-- dialog answered without reading.
--
-- `'open'` for every row, new and old, because giving an agent a browser was
-- already the decision about what that browser is signed in to. The SQL default
-- and `Consent::default` agree here, unlike `bench` above: this is not an
-- upgrade taking a decision, it is an upgrade recording the one the operator
-- took when they handed over the browser.
--
-- `'askBeforeActing'` is per agent rather than per site because per site is the
-- question nobody can answer. Which account may be posted to under which
-- instruction is a thing the model is told; a column of domains cannot hold it.
ALTER TABLE agents ADD COLUMN browser_consent TEXT NOT NULL DEFAULT 'open';
"#,
    ),
    (
        46,
        r#"
-- The crew's own calendar: dates it is answerable for.
--
-- Not the operator's real calendar, which is the Google plugin's job and is
-- read rather than kept. This is the half nothing in the app held. An agent
-- that learned a filing was due on the 15th, that a customer had moved a call,
-- or that a contract lapsed at month end had three stores to choose from and
-- all three were wrong: memory is what an agent knows and outlives the date, a
-- working note expires on its own and carries no moment, and a routine is work
-- the agent will do rather than a thing that is happening. What none of them
-- can be read as is "what is coming", which is the one question a calendar
-- answers.
--
-- Nothing here fires. A row is a fact, not a timer, and that is deliberate
-- rather than unfinished: `routines` is the table that wakes an agent up, and
-- folding the two together would turn every note about a customer's schedule
-- into an agent running at 3am.
--
-- `group_id` is the wall and is why this table is not keyed on the agent. A
-- crew shares one calendar, every agent in it reads the same list, and no agent
-- can reach another crew's. It is `REFERENCES groups(id)` without a cascade,
-- matching every other group-scoped table here: `Store::delete_group` takes the
-- rows out itself, in order, because these foreign keys are enforced.
--
-- `agent_id` is nullable, and NULL is the operator's own writes rather than a
-- deleted agent's: deleting an agent marks the row terminated and never removes
-- it, so this cascade is the same standing belt every other agent-scoped table
-- here wears. What is different is what a deletion does *not* do. A deleted
-- agent's memory, schedule and working notes go with it because they are the
-- agent's own; its occasions stay, because a board meeting does not stop
-- happening when the agent that heard about it is let go, and the calendar it
-- is on belongs to the crew.
--
-- `starts_at` is an instant for both kinds of row, and an all-day one holds
-- local midnight of its day with `all_day` set. That is what lets one column,
-- one index and one ORDER BY serve a deadline and a three o'clock call at once.
-- Midnight alone could not: it is a real time somebody might have chosen, and a
-- filing drawn as "12:00 AM" is a filing nobody reads as a deadline.
--
-- `minutes` is null for a moment with no stated end, which is most of them, and
-- is always null when `all_day` is set. A day has no length to state.
CREATE TABLE occasions (
    id         TEXT    PRIMARY KEY,
    group_id   TEXT    NOT NULL REFERENCES groups(id),
    agent_id   TEXT    REFERENCES agents(id) ON DELETE SET NULL,
    title      TEXT    NOT NULL,
    detail     TEXT    NOT NULL DEFAULT '',
    place      TEXT    NOT NULL DEFAULT '',
    starts_at  INTEGER NOT NULL,
    minutes    INTEGER,
    all_day    INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- The two questions anything asks. The operator's view is every crew across a
-- window of days, and an agent's is one crew across the fortnight in front of
-- it; neither is served well by the other's index.
CREATE INDEX occasions_when ON occasions (starts_at);
CREATE INDEX occasions_crew ON occasions (group_id, starts_at);
"#,
    ),
    (
        47,
        r#"
-- An agent remembers its own replies wherever they were filed. The pair
-- index orders by recipient before time, so it cannot bound a read of the
-- newest messages written to all recipients without sorting the whole past.
CREATE INDEX messages_author_time
    ON messages (from_agent, created_at DESC, id DESC)
    WHERE from_kind = 'agent';
"#,
    ),
    (
        48,
        r#"
-- Where a repository was cloned from, for the rows that were cloned at all.
--
-- NULL is a directory the operator picked on their own machine, which is what
-- every row before this column was and what every desktop row still is. A
-- value is a remote the workspace cloned for itself, into a directory of its
-- own, which is how a box gets a repository: there is no directory anybody
-- could have picked there. The credential that clone may hold is deliberately
-- not in this table; it lives in a file beside the settings, named for the
-- clone's directory, because this table is read into a type that crosses IPC.
ALTER TABLE repositories ADD COLUMN remote TEXT;
"#,
    ),
    (
        49,
        r#"
-- A conversation accepted but not yet settled. The first delivery is the
-- operator's recovery point; no side effect is automatically repeated.
CREATE TABLE pending_runs (
    run_id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE
);
"#,
    ),
    (
        50,
        r#"
CREATE TABLE group_imports (
    group_id TEXT PRIMARY KEY REFERENCES groups(id) ON DELETE CASCADE,
    reconnect TEXT NOT NULL
);
"#,
    ),
    (
        51,
        r#"
CREATE TABLE decisions (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    topic TEXT NOT NULL,
    request TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending','answered','completed','withdrawn')),
    answer TEXT,
    outcome TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    due_at INTEGER,
    remind_at INTEGER NOT NULL,
    snoozed_until INTEGER,
    delivery_run TEXT,
    interrupted INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX decisions_topic ON decisions(agent_id,topic);
CREATE INDEX decisions_reminder ON decisions(remind_at) WHERE status IN ('pending','answered');
"#,
    ),
    (
        52,
        r#"
CREATE TABLE connector_agents (
    connector_id TEXT NOT NULL REFERENCES connectors(id) ON DELETE CASCADE,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    PRIMARY KEY (connector_id, agent_id)
);
-- Preserve access for the agents who already received these credentials.
-- Future agents and newly created secrets require an explicit grant.
INSERT INTO connector_agents (connector_id, agent_id)
SELECT c.id, a.id FROM connectors c JOIN agents a ON a.group_id=c.group_id
WHERE a.discarded_at IS NULL;
"#,
    ),
    (
        53,
        r#"
ALTER TABLE agents ADD COLUMN reasoning_effort TEXT;
ALTER TABLE groups ADD COLUMN reasoning_effort TEXT;
"#,
    ),
    (
        54,
        r#"
-- Firecrawl joined the catalog, and a crew may already have added it by hand
-- under that name. `PluginKind::from_row` resolves a catalog slug whatever
-- address is beside it, so without this such a row would silently start
-- dialling the catalog's address with whatever credential it holds.
--
-- A row that is exactly what the tile would have written, an OAuth grant for
-- the catalog's address, becomes the catalog's: same resource, same issuer,
-- same tools, nothing to reconnect. Every other one is the operator's own
-- server (a pasted key, the keyless address, a program on the host) and keeps
-- working under a name that is not the catalog's. Its id, its agents and its
-- per-tool answers hang off the id and stay; only its tool prefix changes.
UPDATE plugins SET endpoint = ''
WHERE kind = 'firecrawl'
  AND endpoint = 'https://mcp.firecrawl.dev/v2/mcp-oauth'
  AND token_endpoint <> '';
UPDATE plugins SET kind = 'firecrawl_added'
WHERE kind = 'firecrawl'
  AND endpoint <> ''
  AND NOT EXISTS (
      SELECT 1 FROM plugins AS taken
      WHERE taken.group_id = plugins.group_id AND taken.kind = 'firecrawl_added'
  );
"#,
    ),
    (
        55,
        r#"
-- Supermemory joined the catalog, and this is migration 54 for its slug. The
-- tile's row is an OAuth grant for `https://mcp.supermemory.ai/mcp`. Any other
-- `supermemory` row (a pasted key, another address, a program on the host) is
-- the operator's own server and keeps its address under `supermemory_added`.
UPDATE plugins SET endpoint = ''
WHERE kind = 'supermemory'
  AND endpoint = 'https://mcp.supermemory.ai/mcp'
  AND token_endpoint <> '';
UPDATE plugins SET kind = 'supermemory_added'
WHERE kind = 'supermemory'
  AND endpoint <> ''
  AND NOT EXISTS (
      SELECT 1 FROM plugins AS taken
      WHERE taken.group_id = plugins.group_id AND taken.kind = 'supermemory_added'
  );
"#,
    ),
    (
        56,
        r#"
-- Off for every agent that exists. An errand spends the agent's model again,
-- so nobody is opted in by a newer build opening their workspace.
ALTER TABLE agents ADD COLUMN runs_errands INTEGER NOT NULL DEFAULT 0;
"#,
    ),
    (
        57,
        r#"
-- Pages a crew keeps. `domain/artifact.rs` argues the design; what matters for
-- the schema is three decisions.
--
-- `group_id` is the wall, as it is for `occasions`: every agent read and write
-- matches on it, and `Store::delete_group` takes these rows out itself, history
-- first, because these foreign keys are enforced.
--
-- `version` and `updated_at` on the row are a copy of the newest history row
-- that made a version. Kept here so the list, which is read on every turn of
-- every agent in the crew, is one indexed read rather than a join against the
-- whole log. The two are written in one transaction and nowhere else.
--
-- `owner_id` is a reference to a live row, and deliberately not the only record
-- of who owned it: an agent's row can be renamed, and the log below says who
-- held the page by the name they had then.
--
-- `allowed_sources` is the exact list of reads the operator allowed, as it was
-- stored. A page's reads run only while its current version declares that same
-- string, so any change to the list, by an edit or a restore, waits for the
-- operator again without anything having to remember to revoke it.
CREATE TABLE artifacts (
    id              TEXT    PRIMARY KEY,
    group_id        TEXT    NOT NULL REFERENCES groups(id),
    owner_id        TEXT    REFERENCES agents(id) ON DELETE SET NULL,
    title           TEXT    NOT NULL,
    version         INTEGER NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    allowed_sources TEXT
);
CREATE INDEX artifacts_crew ON artifacts (group_id, updated_at);

-- The log, and the versions, in one table. A version is a row that carries a
-- page, and the reads that page declares, `[]` for none; taking and handing
-- ownership and allowing reads are rows that carry neither. One table rather
-- than two because they are one sequence: "Milo edited it, then Juno took it
-- over" is read in order, and two tables would need their order reconstructed
-- from timestamps.
--
-- `agent_id` has no foreign key, and `actor` and `owner` are names rather than
-- ids. This table is the record of what happened, in the words of the moment,
-- and it has to keep reading after the agents it names are renamed, moved or
-- purged. `agent_id` NULL is the operator.
CREATE TABLE artifact_history (
    artifact_id TEXT    NOT NULL REFERENCES artifacts(id),
    seq         INTEGER NOT NULL,
    at          INTEGER NOT NULL,
    change      TEXT    NOT NULL,
    agent_id    TEXT,
    actor       TEXT    NOT NULL DEFAULT '',
    version     INTEGER NOT NULL,
    owner       TEXT,
    note        TEXT    NOT NULL DEFAULT '',
    page        TEXT,
    sources     TEXT,
    PRIMARY KEY (artifact_id, seq),
    CHECK ((change IN ('created', 'edited', 'restored')) = (page IS NOT NULL)),
    CHECK ((page IS NOT NULL) = (sources IS NOT NULL))
);
"#,
    ),
    (
        58,
        r#"
-- An agent works in a terminal of its own rather than in a repository a crew
-- linked. The repository row was a directory plus the two decisions about how
-- work happens in it, and by now each agent already worked in a worktree of
-- its own; the directory is the agent's outright, and the two decisions move
-- onto the agent. `domain/terminal.rs` has the argument.
--
-- Rebuilt rather than altered, because `repository_id` carries a REFERENCES
-- clause and SQLite refuses to drop a column that is part of a foreign key.
-- `run` turns enforcement off around the sequence, which is what the rebuild
-- wants: with it on, dropping the old table would cascade into every table
-- that points at an agent.
CREATE TABLE agents_new (
    id                    TEXT    PRIMARY KEY,
    name                  TEXT    NOT NULL,
    avatar                TEXT    NOT NULL,
    color                 TEXT    NOT NULL,
    model                 TEXT    NOT NULL,
    system_prompt         TEXT    NOT NULL,
    skills                TEXT    NOT NULL DEFAULT '[]',
    lifecycle             TEXT    NOT NULL,
    version               INTEGER NOT NULL DEFAULT 1,
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL,
    group_id              TEXT    NOT NULL REFERENCES groups(id),
    sandbox_id            TEXT,
    sandbox_envd_token    TEXT,
    sandbox_traffic_token TEXT,
    pinned                INTEGER NOT NULL DEFAULT 0,
    rail_order            INTEGER NOT NULL DEFAULT 0,
    browser_id            TEXT,
    has_computer          INTEGER NOT NULL DEFAULT 0,
    has_browser           INTEGER NOT NULL DEFAULT 0,
    discarded_at          INTEGER,
    browser_consent       TEXT    NOT NULL DEFAULT 'open',
    reasoning_effort      TEXT,
    runs_errands          INTEGER NOT NULL DEFAULT 0,
    -- A decision, like the two before it: taking a terminal away keeps the
    -- directory and these two answers.
    has_terminal          INTEGER NOT NULL DEFAULT 0,
    harness               TEXT    NOT NULL DEFAULT 'pi',
    gate                  TEXT    NOT NULL DEFAULT 'open'
);

-- An agent that was put in a repository was trusted with a shell there, and
-- keeps that trust and the two answers it worked under. What was in the
-- repository stays where it was on disk; the agent starts in an empty
-- directory of its own and clones what it needs.
INSERT INTO agents_new
    (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,
     group_id,sandbox_id,sandbox_envd_token,sandbox_traffic_token,pinned,rail_order,browser_id,
     has_computer,has_browser,discarded_at,browser_consent,reasoning_effort,runs_errands,
     has_terminal,harness,gate)
SELECT a.id,a.name,a.avatar,a.color,a.model,a.system_prompt,a.skills,a.lifecycle,a.version,
       a.created_at,a.updated_at,a.group_id,a.sandbox_id,a.sandbox_envd_token,
       a.sandbox_traffic_token,a.pinned,a.rail_order,a.browser_id,a.has_computer,a.has_browser,
       a.discarded_at,a.browser_consent,a.reasoning_effort,a.runs_errands,
       r.id IS NOT NULL, coalesce(r.harness, 'pi'), coalesce(r.gate, 'open')
  FROM agents a
  LEFT JOIN repositories r ON r.id = a.repository_id AND r.group_id = a.group_id;

DROP TABLE agents;
ALTER TABLE agents_new RENAME TO agents;

CREATE INDEX agents_group ON agents (group_id);
CREATE UNIQUE INDEX agents_live_name_unique
    ON agents (group_id, lower(name))
    WHERE lifecycle <> 'terminated';

DROP TABLE repositories;
"#,
    ),
    (
        59,
        r#"
-- The coding session each agent last ran, which a follow-up carries on.
--
-- A harness keeps its own conversation on disk, so continuing one is a new
-- process pointed at the old session rather than a new brief, with everything
-- the job had already read and decided. Only the last per agent: an agent runs
-- one job at a time. Stored rather than held in memory because a follow-up
-- often comes the next day.
CREATE TABLE coding_sessions (
    agent_id   TEXT    PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
    harness    TEXT    NOT NULL,
    session_id TEXT    NOT NULL,
    directory  TEXT    NOT NULL,
    updated_at INTEGER NOT NULL
);

-- What the operator chose inside each harness for one agent: the model, the
-- effort, and for pi which account pays. A row per agent per harness, because
-- the three programs share no model names and no effort words, and one field
-- would be broken by every switch between them. No row is the program's own
-- setting, which is what every job ran on before this table.
CREATE TABLE coding_tuning (
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    harness  TEXT NOT NULL,
    model    TEXT,
    effort   TEXT,
    pays     TEXT NOT NULL DEFAULT 'own',
    PRIMARY KEY (agent_id, harness)
);
"#,
    ),
    (
        60,
        r#"
-- A version's second document: the condensed view the status bar draws. On the
-- version row beside the page, because the two are written, read and put back
-- together, and a restore that brought back one without the other would pin an
-- old number beside a new page. Optional: most pages have none, and every
-- version written before this has none.
ALTER TABLE artifact_history
    ADD COLUMN condensed TEXT CHECK (condensed IS NULL OR page IS NOT NULL);
"#,
    ),
    (
        61,
        r#"
-- A decision's answer, as the part the transcript draws as the operator's
-- reply. It was written as one text part from Guaca: a paragraph telling the
-- agent what to do with the answer, then the request and the answer as JSON on
-- a line of their own, and every channel holding one drew all of it as a
-- bubble. The agent is sent the same paragraph either way, because
-- `decision::delivery` builds it from the fields kept here, byte for byte.
--
-- Only rows that are unmistakably that write are converted: from Guaca, one
-- text part, the sentence it always opened with, and a last line carrying
-- every field the part requires. A row this cannot read stays text and draws
-- as it did. A row it misread would fail to parse and take its channel with
-- it, and a malformed argument makes a JSON function raise rather than return
-- null, so every one of them sits behind a CASE: SQLite promises the order a
-- CASE is evaluated in and promises nothing about an AND.
WITH texts AS (
    SELECT id,
           CASE WHEN json_valid(parts) THEN
               CASE WHEN json_array_length(parts) = 1
                         AND json_extract(parts, '$[0].type') = 'text'
                    THEN json_extract(parts, '$[0].text') END
           END AS text
      FROM messages
     WHERE from_kind = 'system'
),
split AS (
    SELECT id,
           substr(text, 1, instr(text, char(10)) - 1) AS lead,
           substr(text, instr(text, char(10)) + 1) AS body
      FROM texts
     WHERE text GLOB 'The operator answered decision *'
       AND instr(text, char(10)) > 0
),
sound AS (
    SELECT id, lead, body
      FROM split
     WHERE CASE WHEN json_valid(body) THEN
               json_type(body, '$.id') = 'text'
               AND length(json_extract(body, '$.id')) = 36
               AND json_type(body, '$.answer') = 'text'
               AND json_type(body, '$.question') = 'object'
               AND json_type(body, '$.question.question') = 'text'
               AND coalesce(json_type(body, '$.question.options'), 'array') = 'array'
           END
)
UPDATE messages
   SET parts = (
       SELECT json_array(json_object(
                  'type', 'decision',
                  'id', json_extract(body, '$.id'),
                  'request', json(json_extract(body, '$.question')),
                  'answer', json_extract(body, '$.answer'),
                  'resumed', json(CASE WHEN instr(lead, 'This work was interrupted.') > 0
                                       THEN 'true' ELSE 'false' END)))
         FROM sound
        WHERE sound.id = messages.id)
 WHERE id IN (SELECT id FROM sound);
"#,
    ),
    (
        62,
        r#"
-- Work a stopping host put down, for the next one to pick up.
--
-- A host asked to stop lets every turn reach a boundary where everything it
-- did is written down, and puts its messages down here instead of ending them.
-- `pending_runs` stays what it was: the record of a conversation still owed an
-- answer, which a restart that was not a drain reports as interrupted.
--
-- A message the next host delivers again, and the records of the turn it was
-- in the middle of, oldest first, as a JSON array of message ids. Empty for a
-- message no turn had started.
CREATE TABLE put_down (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    records    TEXT NOT NULL DEFAULT '[]'
);

-- A conversation all of whose work was put down, and what it had spent. Only
-- these are picked up: a message put down in a conversation that still had a
-- turn in the middle of a tool call when the host stopped is part of work
-- nobody can vouch for, and is reported with the rest of it.
CREATE TABLE put_down_runs (
    run_id TEXT PRIMARY KEY,
    steps  INTEGER NOT NULL
);

-- A coding job a stopping host interrupted, for the next one to carry on in
-- the same session. An empty session is a job stopped before its program had
-- named one, which is started over from `task`.
CREATE TABLE put_down_jobs (
    agent_id   TEXT PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
    harness    TEXT NOT NULL,
    session_id TEXT NOT NULL,
    directory  TEXT NOT NULL,
    task       TEXT NOT NULL,
    origin     TEXT NOT NULL CHECK (origin IN ('agent', 'operator'))
);
"#,
    ),
    (
        63,
        r#"
-- An agent's own model for turns the ChatGPT subscription pays for.
--
-- The app and every group keep two model fields because the providers share no
-- model names. The agent kept one and sent it to whichever provider paid, so a
-- model chosen while a key paid was refused by name on every turn once its crew
-- moved to the subscription, and the other way round.
--
-- Which field a value already stored belongs in is decided once, from what its
-- crew pays with now:
--
-- - A value with a slash is an endpoint's. ChatGPT's model names never carry
--   one and OpenRouter's always do, so on the subscription it was refused on
--   every turn, and leaving it on the endpoint is the only reading that runs.
-- - Otherwise, on a crew the subscription pays for, it is the subscription's:
--   it is what those turns have been running on.
-- - On a crew a key pays for, it stays the endpoint's.
-- - On a crew that follows the app or runs on Claude, which of the two pays is
--   in settings this cannot read, so it goes in both and each provider keeps
--   running what it ran before.
ALTER TABLE agents ADD COLUMN subscription_model TEXT NOT NULL DEFAULT '';

UPDATE agents
   SET subscription_model = model
 WHERE model <> '' AND instr(model, '/') = 0
   AND coalesce((SELECT provider FROM groups WHERE groups.id = agents.group_id), '')
       <> 'compatible';

UPDATE agents
   SET model = ''
 WHERE subscription_model <> ''
   AND (SELECT provider FROM groups WHERE groups.id = agents.group_id) = 'chatgpt';
"#,
    ),
    (
        64,
        r#"
-- Where a crew is drawn: its circle in the column, its section in the rail, and
-- its place in every list of crews the app shows. The operator arranges it by
-- dragging a circle, for the reason agents got `rail_order` in 20: an order
-- nobody chose cannot be learned, and creation order was the only one there was.
--
-- Backfilled in creation order, so an upgrade draws the column it drew before,
-- and distinct from the start, so the first drag has somewhere to land.
ALTER TABLE groups ADD COLUMN rail_order INTEGER NOT NULL DEFAULT 0;

UPDATE groups SET rail_order = (
    SELECT COUNT(*)
      FROM groups AS earlier
     WHERE earlier.created_at < groups.created_at
        OR (earlier.created_at = groups.created_at AND earlier.rowid < groups.rowid)
);
"#,
    ),
    (
        65,
        r#"
-- The coding subsystem is gone: an agent no longer has a terminal, a harness
-- that writes its code, a gate on its pushes, a session to carry on or a
-- tuning per program, and a stopping host has no job to put down. What agents
-- cloned into their terminal directories stays on disk, untouched: it is
-- their work, and nothing here is the place to throw it away.
DROP TABLE coding_sessions;
DROP TABLE coding_tuning;
DROP TABLE put_down_jobs;
ALTER TABLE agents DROP COLUMN has_terminal;
ALTER TABLE agents DROP COLUMN harness;
ALTER TABLE agents DROP COLUMN gate;
"#,
    ),
    (
        66,
        r#"
CREATE TABLE approval_policies (
    id TEXT PRIMARY KEY,
    group_id TEXT REFERENCES groups(id) ON DELETE CASCADE,
    effect TEXT NOT NULL,
    class TEXT NOT NULL,
    tool TEXT,
    agents TEXT NOT NULL,
    conditions TEXT NOT NULL,
    wrote TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    version INTEGER NOT NULL
);
"#,
    ),
    (
        67,
        r#"
CREATE TABLE workspace_identity (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    id TEXT NOT NULL
);
CREATE TABLE resource_ledger (
    provider TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    origin TEXT NOT NULL CHECK (origin IN ('created', 'adopted')),
    provider_id TEXT,
    created_at INTEGER NOT NULL,
    released_at INTEGER,
    gone_at INTEGER,
    PRIMARY KEY (provider, resource_id)
);
CREATE TABLE pending_sandbox_resume (
    resource_id TEXT PRIMARY KEY
);
"#,
    ),
    (
        68,
        r#"
ALTER TABLE plugins ADD COLUMN grant_generation INTEGER NOT NULL DEFAULT 0;
"#,
    ),
    (69, r#"ALTER TABLE put_down_runs ADD COLUMN guard_snapshot TEXT;"#),
    (
        70,
        r#"
ALTER TABLE put_down ADD COLUMN absorbed TEXT NOT NULL DEFAULT '[]';
CREATE TABLE routine_runs_new (
    id INTEGER PRIMARY KEY,
    routine_id TEXT NOT NULL,
    run_id TEXT,
    kind TEXT NOT NULL,
    at INTEGER NOT NULL,
    error TEXT
);
INSERT INTO routine_runs_new (id,routine_id,run_id,kind,at)
SELECT id,routine_id,run_id,kind,at FROM routine_runs;
DROP TABLE routine_runs;
ALTER TABLE routine_runs_new RENAME TO routine_runs;
CREATE INDEX routine_runs_routine ON routine_runs (routine_id, at DESC);
CREATE TABLE routine_occurrences (
    routine_id TEXT NOT NULL,
    occurrence TEXT NOT NULL,
    message_id TEXT,
    owner_run TEXT,
    state TEXT NOT NULL,
    PRIMARY KEY (routine_id, occurrence)
);
CREATE INDEX routine_occurrences_message ON routine_occurrences (message_id);
CREATE TRIGGER routine_delete_history AFTER DELETE ON routines
WHEN NOT EXISTS (SELECT 1 FROM routine_occurrences WHERE routine_id=OLD.id AND state='completing')
BEGIN
    DELETE FROM routine_runs WHERE routine_id=OLD.id;
END;
"#,
    ),
    (
        71,
        r#"
CREATE TABLE onboarding_hires (
    key TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL,
    agent_id TEXT NOT NULL
);
CREATE TABLE operator_sends (
    key TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL,
    message_id TEXT NOT NULL,
    run_id TEXT NOT NULL
);
"#,
    ),
    (
        72,
        r#"
-- One row per finished run, written in the commit that settles it. See
-- domain/outcome.rs. No foreign key on group_id: the row says where the run
-- happened, and a group deleted later keeps nothing that would refuse it.
-- note_id is a pointer into messages and may outlive a cleared channel.
CREATE TABLE outcomes (
    run_id      TEXT    PRIMARY KEY,
    agent_id    TEXT    NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    group_id    TEXT    NOT NULL,
    trigger     TEXT    NOT NULL
        CHECK (trigger IN ('operator','routine','event','decision','retry')),
    trigger_ref TEXT,
    status      TEXT    NOT NULL
        CHECK (status IN ('done','nothing_new','needs_you','failed','stopped','interrupted')),
    headline    TEXT    NOT NULL,
    note_id     TEXT,
    needs       TEXT,
    steps       INTEGER NOT NULL DEFAULT 0,
    started_at  INTEGER NOT NULL,
    finished_at INTEGER NOT NULL,
    seen_at     INTEGER
);
-- What For you lists: unseen, newest first. Partial, so it stays the size of
-- what the operator has not read rather than of every run ever finished.
CREATE INDEX outcomes_unseen ON outcomes (finished_at) WHERE seen_at IS NULL;
-- What a time window reads, seen or not.
CREATE INDEX outcomes_finished ON outcomes (finished_at);
-- What marking a channel seen reads.
CREATE INDEX outcomes_agent ON outcomes (agent_id) WHERE seen_at IS NULL;
-- The run that filed a decision, so the outcome of that run can say it left
-- one open. Null on every decision filed before this.
ALTER TABLE decisions ADD COLUMN filed_run TEXT;
"#,
    ),
    (
        73,
        r#"
-- What to deliver to a person, one row per destination, written in the commit
-- of the thing it is about. See domain/outbox.rs. destination admits
-- 'endpoint' already, so an address the operator configures later is a value
-- rather than a migration. kind has no CHECK on purpose: it is the list most
-- likely to grow, and the reader refuses a value it does not know.
-- offered_at is when the windows were first offered a row; a claim takes only
-- an offered row. batch_id names the row that carries a gathered item, and is
-- set on that row and on every row it carries.
CREATE TABLE outbox (
    id              TEXT    PRIMARY KEY,
    kind            TEXT    NOT NULL,
    destination     TEXT    NOT NULL CHECK (destination IN ('clients','endpoint')),
    dedup_key       TEXT    NOT NULL,
    group_id        TEXT,
    agent_id        TEXT    REFERENCES agents(id) ON DELETE CASCADE,
    run_id          TEXT,
    ref_id          TEXT,
    title           TEXT    NOT NULL,
    body            TEXT    NOT NULL,
    target          TEXT    NOT NULL,
    status          TEXT    NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending','delivered','suppressed','expired','failed')),
    attempts        INTEGER NOT NULL DEFAULT 0,
    next_attempt_at INTEGER NOT NULL,
    last_error      TEXT,
    offered_at      INTEGER,
    batch_id        TEXT,
    created_at      INTEGER NOT NULL,
    delivered_at    INTEGER,
    UNIQUE (dedup_key, destination)
);
-- What the dispatcher reads: pending rows, by when they are next due.
CREATE INDEX outbox_due ON outbox (next_attempt_at) WHERE status='pending';
-- What answering a request, clearing an escalation or seeing a run withdraws.
CREATE INDEX outbox_ref ON outbox (ref_id) WHERE status='pending';
CREATE INDEX outbox_run ON outbox (run_id) WHERE status='pending';
"#,
    ),
    (
        74,
        r#"
-- What a decision was filed from, when Guaca filed it rather than an agent: a
-- permission or a question nobody answered in time is filed as a decision, and
-- origin_ref is the approvals row it came from. Both null on every decision an
-- agent filed itself.
ALTER TABLE decisions ADD COLUMN origin TEXT;
ALTER TABLE decisions ADD COLUMN origin_ref TEXT;
-- One action one agent may take once without asking, because the operator
-- allowed the decision a lapsed permission was filed as. Written ungranted when
-- the request lapses, so the fingerprint is the action as it was asked for and
-- never as it is later described; granted_at and expires_at are set by an
-- Allow, and used_at by the one request it lets through.
CREATE TABLE permission_grants (
    decision_id TEXT    PRIMARY KEY REFERENCES decisions(id) ON DELETE CASCADE,
    agent_id    TEXT    NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    fingerprint TEXT    NOT NULL,
    granted_at  INTEGER,
    expires_at  INTEGER,
    used_at     INTEGER
);
-- What a request reads: this agent's grants for this action, given and unused.
CREATE INDEX permission_grants_open ON permission_grants (agent_id, fingerprint)
    WHERE granted_at IS NOT NULL AND used_at IS NULL;
"#,
    ),
    (
        75,
        r#"
-- When a turn first took up a conversation still owed an answer. Null until
-- then, and a crash that finds it null has found a message no model has read
-- and no tool has acted on, which is delivered again rather than reported.
-- Rows written before this column existed are marked started, because
-- nothing recorded whether they were, and a guess the wrong way repeats work.
ALTER TABLE pending_runs ADD COLUMN started_at INTEGER;
UPDATE pending_runs SET started_at = CAST(strftime('%s','now') AS INTEGER) * 1000;
"#,
    ),
    (
        76,
        r#"
-- Of a call's prompt tokens, how many the provider read from its cache. A
-- share of `prompt`, never added to it. Zero for rows written before anything
-- read the count, which is what they would have said: nothing asked.
ALTER TABLE usage ADD COLUMN cached INTEGER NOT NULL DEFAULT 0;
"#,
    ),
    (
        77,
        r#"
-- How many of a routine's slots one firing stands for that came due while
-- the host was off, the late one included. Zero for a firing on time, which
-- is what every row written before this was: nothing counted, and nothing
-- said otherwise. See Routine::missed_before.
ALTER TABLE routine_runs ADD COLUMN missed INTEGER NOT NULL DEFAULT 0;
"#,
    ),
    (
        78,
        r#"
-- A window's claim, and a claim given back. claimed_by is the page that
-- claimed a row for the windows; a page that claimed and then could not show
-- it gives it back, and released_by is the last page that did. releases counts
-- them, because the first goes to the other pages and the second ends at the
-- push. Null and zero on every row written before this, all of which were
-- claimed by a page that could not give one back.
ALTER TABLE outbox ADD COLUMN claimed_by TEXT;
ALTER TABLE outbox ADD COLUMN released_by TEXT;
ALTER TABLE outbox ADD COLUMN releases INTEGER NOT NULL DEFAULT 0;
"#,
    ),
    (
        79,
        r#"
-- An overnight tidy of each agent's memory, switched on per group. See
-- domain/group.rs. tidy_since is when the operator switched it on and is null
-- while it is off: an agent never tidied counts what it finished from there,
-- not from the start of time. tidy_hour is kept while it is off, so switching
-- back on finds it where it was.
-- Two marks on the agent, because a night can be seen to without being read.
-- last_tidy_at is when the clock last saw to it, by handing it a tidy or by
-- finding it had finished nothing new; written at the handing over, so a tidy
-- cut short by a stop or a restart is not given again the same night.
-- tidied_through is where the next tidy's lines start: moved only by a tidy
-- that finished or stored a rewrite, so one that failed hands its lines on.
ALTER TABLE groups ADD COLUMN tidy_hour INTEGER NOT NULL DEFAULT 3;
ALTER TABLE groups ADD COLUMN tidy_since INTEGER;
ALTER TABLE agents ADD COLUMN last_tidy_at INTEGER;
ALTER TABLE agents ADD COLUMN tidied_through INTEGER;
"#,
    ),
    (
        80,
        r#"
CREATE TABLE onboarding_hire_agents (
    key TEXT NOT NULL,
    position INTEGER NOT NULL,
    agent_id TEXT NOT NULL,
    PRIMARY KEY (key, position)
);
INSERT INTO onboarding_hire_agents (key, position, agent_id)
    SELECT key, 0, agent_id FROM onboarding_hires;
ALTER TABLE onboarding_hires DROP COLUMN agent_id;
"#,
    ),
    (
        81,
        r#"
ALTER TABLE agents ADD COLUMN reads_web INTEGER NOT NULL DEFAULT 1;
UPDATE agents SET reads_web = 0;
"#,
    ),
];

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

pub fn run(conn: &mut Connection) -> Result<i32, MigrationError> {
    let enforced: bool = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    if enforced {
        conn.pragma_update(None, "foreign_keys", false)?;
    }
    let applied = apply(conn, latest_version());
    if enforced {
        let _ = conn.pragma_update(None, "foreign_keys", true);
    }
    applied
}

fn has_remote(conn: &Connection) -> Result<bool, rusqlite::Error> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('repositories') WHERE name='remote')",
        [],
        |row| row.get(0),
    )
}

fn has_table(conn: &Connection, name: &str) -> Result<bool, rusqlite::Error> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |row| row.get(0),
    )
}

fn apply(conn: &mut Connection, target: i32) -> Result<i32, MigrationError> {
    loop {
        let tx: Transaction<'_> = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: i32 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;

        if current > target {
            return Err(MigrationError::FromTheFuture { found: current, supported: target });
        }

        if matches!(current, 45 | 46) && has_remote(&tx)? {
            let consent: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('agents') WHERE name='browser_consent')",
                [], |row| row.get(0),
            )?;
            if !consent {
                tx.execute_batch(MIGRATIONS.iter().find(|(v, _)| *v == 45).unwrap().1)?;
            }
            if current == 46 && !has_table(&tx, "occasions")? {
                tx.execute_batch(MIGRATIONS.iter().find(|(v, _)| *v == 46).unwrap().1)?;
            }
        }

        let Some((version, sql)) = MIGRATIONS.iter().find(|(v, _)| *v > current) else {
            break;
        };

        let already_present = (*version == 48 && has_remote(&tx)?)
            || (*version == 49 && has_table(&tx, "pending_runs")?);
        if !already_present {
            tx.execute_batch(sql)
                .map_err(|source| MigrationError::Failed { version: *version, source })?;
        }
        tx.pragma_update(None, "user_version", *version)?;
        tx.commit()?;
        tracing::info!(version, "applied migration");
    }

    Ok(target)
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    const SHIPPED: &[(i32, &str)] = &[
        (1, "6a984cc487bc5647932cec3aad2cbd58359290b7797100c4ea9251fe4bb851eb"),
        (2, "dcbcd1421101cb230c404fd2361857082554b5a130b331e8ab90aa5705980828"),
        (3, "523f865331509e413a9fa3f86f4b6322656af0c0bc2eb20a0d89bdb340fb97c3"),
        (4, "74c16598ee3fc26ec8bcf28659c32eace110f8411d14534dba1730003fdc7207"),
        (5, "35ad2c5de7a9d749b4f936ff74013e002d37d31faa7fcb6aadd2e8954907dcb6"),
        (6, "5cf53b9b53d2f7a3a51e101b4c5ca087c3630798bb57fbae4a5c3487d55f2d76"),
        (7, "8a35dcd5fb8943d6ea322c3ee08365d6a78167bd45b30f638fec6bee67196523"),
        (8, "7022cb15242902fa65dffd35282713f4e77ac4428d281eab391dcec1d3ca572e"),
        (9, "3f73ce773bfe321c89cba58eb00cbe3ec000160dce9b2251464c3fb422f934dc"),
        (10, "ac70c6bfba910dcc11ef56242b35ab97ecb8b4651491a8ee9d036c6ebaeb6d12"),
        (11, "266484c92f6022cb8b53eeb38f6fabb12d9586749165086907a78c3ff3c012b6"),
        (12, "be222fcd66ab452c5b29267637b0203a6b0c82f6555f35d852f39c1e90e3ab26"),
        (13, "53c14b1735d8f5f4386c97e8c4ed5dd6bcfbf4f0fc929716695a3db0aa4ea78e"),
        (14, "f98f1ca920fc25c6dba8940f9b16687e053cb474e3f8c67d1136902a8946c7cb"),
        (15, "a7930c1b9d8422865d8fd07667bf951c16f12a9368620be7d0d5d68057fbf85c"),
        (16, "795ee36fa35b4f889be62f1a6abfc10b14d393f7311ef18ab9dd7c4ae64b8bd3"),
        (17, "2278fc1d279ea6d867c112d5dc7afac14dcd21caffff933ac85552c2678ef465"),
        (18, "4413427d31695a194b5d076da900fb69f4cf17f17bb8994218a8709de6150454"),
        (19, "935158f1f50f0baa2879ddd31aead2e95579628c540cdf30c16bcc7a4212e0a1"),
        (20, "19c24bf43fa70e735a75a22839c5bce18018c5a6485a9785f1e975461ec637e6"),
        (21, "2777afafab7c674f770919bb6018e3f196846418d6b90fa04e9b9eb06169155f"),
        (22, "6e371da188a98d7895ced49b816cb73486bf5fae5947db0804b5cfc491bae4c3"),
        (23, "5984b9a700da8ad5711925979f3cc6eded552ee98b1830dcd42cfc5a5e1094db"),
        (24, "83869063015527e8b66d6290a4ea9b1add68348566b1f39dfe12c5ea6ac5cb21"),
        (25, "a2990dc780c1561fc312f3526b033edf271763ba246c64f87c12b9f0d0611714"),
        (26, "0f8d7aa1387fd000e1ef2184da981b5944409b6e316d6996438f09a6f70990ab"),
        (27, "2b4824f288e2ab2f11fb385857ad68639920220e2634fd4d5da488289ead3af3"),
        (28, "c3016b9cfef9115df93bfc52c23330836e4a9e2e766f0d1a2c897295d5e51db1"),
        (29, "1b1c77dc40d4d8662e65fd61e26de203d7008969e5916f14a705bada63c4980f"),
        (30, "8ec350fa71c5c4c4f36dd5be8dc6c1502a03cb322e7327a17d6987ef9f927bac"),
        (31, "df536302a76787050cb93a8a05fafb370f6ca58cae97e01bd9efb58c25ef31b4"),
        (32, "e6b61c7201e9b4a634413e92e83996402def84ac0f16cde896914252099a72f2"),
        (33, "af78e5919839786f959f3e0e950cc1fc6a736bb3d9529c0465f8508837bd64b1"),
        (34, "4603ed626208cd5da0617261da90b828aa6c459591432681cf93a40a4ac862ab"),
        (35, "ec15b62c84fa7d2eb1ccec6881420ecaa62d9e03f6e583972477159349f3cae9"),
        (36, "e919b785d09f9aa6a5db930c8f575f3910ca9eb2e46ec0cb1c0b81955aac58e9"),
        (37, "e930578adb5423a51f053462a897da33e9dbbdedf0123c33b9407be9b1ea53af"),
        (38, "49315f4f60bbe7557b55917c5e2331d9cb63fe831f5800ec9715317a46de9c3d"),
        (39, "bb37b5279ee64f3df34e0d09d07ba4c785a6bf84c6c9121fb27ae7d1ec2adecb"),
        (40, "fb34a0348cea907c2de417633b2e18804d856c39f02444205211d207ed0398a2"),
        (41, "912160045314e652e496e1c01451e335784f67f3ef98dfe4c7471ba333294ccc"),
        (42, "d6d2c4218bdcbe0d5e05858c04a5a521997de17838ae9b2ced90504c11331fd3"),
        (43, "5c3e2d4f1c5d0b7330fd9ccbe5e6c91e443b10828fc7436b5c7d2a7eddfcb5e3"),
        (44, "4cbde248656b3bd75587346cfcaae00ea99161ee6205e209b685fb202b36db75"),
        (45, "a945038da4fda22e9491d2fc36829e062f67508b0a76f5c4561f13712cee741c"),
        (46, "e3bd5b9611e5fcbe9cb7d55acdf93343774db988f711d950602b70d94c921178"),
        (47, "e3c9028b394c6b17487576a4d443b26304732fb9c014490f73ff848e8d49a0c2"),
        (48, "46e62c0da668d820b68b86adf5bb26feae7fd1655ec8847e02e9d31efb2ab4bc"),
        (49, "f990316558a91c118948006ba4d316ac656359a382c285f332c31e0bb066c837"),
        (50, "0e52692e123098439d7ba9116ef48ea55cce42ebc6ffb34642593b38e640ac82"),
        (51, "fe8c735cd1020598a3e7d256d3bbae9dd096522289f88cd71ff0296de9539c16"),
        (52, "c5c96c8ab9dbbc1add1ada732005b8e4c5fc8ad451d52def1fb4f25c19c3deb2"),
        (53, "cddfc174fa882473ae5758cf19939ca51100c3fc8a22494bd5191b5fe9afd441"),
        (54, "169f08478086fb3114c22e97cab917e3a43bd817d46c1e9b1db5d530796b3f4b"),
        (55, "a3274d914e764fafb0d27215625b3bcbd39518ac4b3fcd8f41862fe00f6e1697"),
        (56, "95255c1336fc799e8f87364fb2c94e823099208426a5b98d5a8c4513dd9f5c72"),
        (57, "f32377c01b68ee006c76e2e6f8bca1d9eb1cda617cb76e02d842411be66edfc6"),
        (58, "7e4d837aad41f2ea81abc25d236e4f4fb13ab3b83460d13dd3a91695593f4a8b"),
        (59, "a14b8e213249fe76f96a80201265bbc005846e37f24bc023e5d7f74b48a69ea6"),
        (60, "8a1e86df9623ae5b53df07745b1fa67fd7de0d69213aa51b6e56462f0bf6d78d"),
        (61, "8a3fc408e6ce6368cf95fe7dfee0fbc2405f1b9ff072c7a739aa9ec7ad3068cd"),
        (62, "20acbbe8e39335fe0ec5e87edbc2f1de70a0f57524f74d3995c93b08800009ea"),
        (63, "0b9f5833935a9bd53e1eb3ca62457566bd4561fa0b8e3328fb16ce54011a200f"),
        (64, "48c5a2251ecef6f4e7756e2c8278df6517b5f4f7ff48ad100351526c7e6091c4"),
        (65, "31faf76ccf557ed3944c48af4e9a876a1318ac0cb17da630fa14725e5be7f2ae"),
        (66, "8233ae8683b9ad7d3b8cf31f78135ef2eb984ad07f504b8e272f57e3833d34ac"),
        (67, "6e2536e0d553f5f2a83802d1db3f6afa752c9211d4e7ae7084c56b5c8290c103"),
        (68, "d7cde8ec16835521384e8cac3184d19e18d861726c64206ba97d913544aa0495"),
        (69, "074679cc92ab20d20b345a0017b1fae1eb077d313c480832de805f71034ecdea"),
        (70, "46abf7cf6a95134a6129eaa4f81bb1d07ff1078b298e245a31a4b98679f041e0"),
        (71, "80068e13462a6b5993c518081eec46283cca62ccdb04e9b687005baf8025b37d"),
        (72, "2a1918451f4dd9c19675e5426158cd57694446b796dac352159c7853351b3d78"),
        (73, "3f9b707b4dbb236177ae9d4241f7357bf256a8d4006dc34b8c57fe65c9b4da5c"),
        (74, "49f7d0cbb84c98f6fb36d7e8f731d8e86692ac12f7eea43376dd14f43d7bd84f"),
        (75, "9a30d07aa20886f0f09b961e1f85de7aa9cb62ff26c6ecbe4f9547b2e04b4771"),
        (76, "dae436854f1fd4212d46ede62e65b957e757abb1e9caf09b8bd385f202113c47"),
        (77, "868e1c3a757dc982e46c4300d6422f5d96991751fb310d6b8c7eb7ffa8be9156"),
        (78, "fc778794fb370b571ac5821e98c871c8172fa9f933bb1bd99ca48b4b6b617b67"),
        (79, "554ec9b4ff4a5bc9998ea33a07febc1abff3b79c452f248276987f15b1105348"),
        (80, "cfb92921daf974ffd88d8415551947ce65b6214fb8d895b9add4c29de403e54b"),
        (81, "d3cf5b5521906920c01f3c1c2fb8deaed8822c20542d24c1dd23e7c6a6a27b12"),
    ];

    fn memory() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn web_reading_is_on_for_an_agent_made_after_it_and_off_for_one_made_before() {
        let mut conn = memory();
        for (version, sql) in MIGRATIONS.iter().take_while(|(v, _)| *v < 81) {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", *version).unwrap();
        }
        let insert = |conn: &Connection, id: &str| {
            conn.execute(
                "INSERT INTO agents (id,name,avatar,color,model,system_prompt,lifecycle,created_at,
                                     updated_at,group_id)
                 VALUES (?1,?1,'orb','#7fb069','m','','active',1,1,?2)",
                rusqlite::params![id, DEFAULT_GROUP_ID],
            )
            .unwrap();
        };
        insert(&conn, "before");
        run(&mut conn).unwrap();
        insert(&conn, "after");
        let reads = |id: &str| -> bool {
            conn.query_row("SELECT reads_web FROM agents WHERE id=?1", [id], |r| r.get(0)).unwrap()
        };
        assert!(!reads("before"), "an upgrade must not start an existing agent fetching pages");
        assert!(reads("after"));
    }

    #[test]
    fn a_shipped_migration_is_never_edited() {
        assert!(MIGRATIONS.len() >= SHIPPED.len(), "a shipped migration was removed");
        for (at, (version, sql)) in MIGRATIONS.iter().enumerate() {
            let digest: String =
                Sha256::digest(sql.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect();
            let line = format!("        ({version}, \"{digest}\"),");
            let Some(&(shipped, pinned)) = SHIPPED.get(at) else {
                panic!(
                    "migration {version} has no pin. Add this line at the end of SHIPPED:\n{line}"
                );
            };
            assert!(
                (shipped, pinned) == (*version, digest.as_str()),
                "migration {shipped} does not match its pin. A shipped migration is never edited, so \
                 undo the edit and append a new migration. If {version} is new and not yet shipped, \
                 replace its line in SHIPPED with:\n{line}"
            );
        }
    }

    #[test]
    fn plugin_grant_generation_preserves_legacy_rows_and_queries() {
        let mut conn = memory();
        for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v <= 67) {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", *version).unwrap();
        }
        conn.execute("INSERT INTO plugins (id,group_id,kind,access_token,refresh_token,connected_at) VALUES ('plugin',?1,'neon','access','refresh',1)", [DEFAULT_GROUP_ID]).unwrap();
        run(&mut conn).unwrap();
        let row: (String, String, i64) = conn
            .query_row(
                "SELECT access_token,refresh_token,grant_generation FROM plugins WHERE id='plugin'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, ("access".into(), "refresh".into(), 0));
        let old_query: String = conn
            .query_row("SELECT access_token FROM plugins WHERE id='plugin'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(old_query, "access");
        assert!(matches!(
            apply(&mut conn, 67),
            Err(MigrationError::FromTheFuture { found, supported: 67 }) if found == latest_version()
        ));
    }

    #[test]
    fn desktop_and_hosting_lineages_upgrade_without_losing_data() {
        for (version, hosting) in
            [(44, false), (45, false), (46, false), (47, false), (45, true), (46, true)]
        {
            let mut conn = memory();
            for (v, sql) in
                MIGRATIONS.iter().take_while(|(v, _)| *v <= if hosting { 44 } else { version })
            {
                conn.execute_batch(sql).unwrap();
                conn.pragma_update(None, "user_version", *v).unwrap();
            }
            if hosting {
                conn.execute_batch("ALTER TABLE repositories ADD COLUMN remote TEXT;").unwrap();
                if version == 46 {
                    conn.execute_batch("CREATE TABLE pending_runs (run_id TEXT PRIMARY KEY, message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE);").unwrap();
                    conn.execute("INSERT INTO messages (id,run_id,channel_id,from_kind,to_kind,parts,trust,hop,expects_reply,created_at) VALUES ('original-message','unfinished','channel','operator','system','[]','operator',0,0,0)", []).unwrap();
                    conn.execute(
                        "INSERT INTO pending_runs VALUES ('unfinished', 'original-message')",
                        [],
                    )
                    .unwrap();
                }
                conn.pragma_update(None, "user_version", version).unwrap();
            }
            conn.execute("INSERT INTO repositories (id,group_id,name,path,note,created_at,updated_at) VALUES ('repo',?1,'Code','/repo','keep me',1,1)", [DEFAULT_GROUP_ID]).unwrap();
            conn.execute("INSERT INTO agents (id,name,avatar,color,model,system_prompt,lifecycle,created_at,updated_at,group_id,repository_id) VALUES ('coder','Coder','orb','#7fb069','m','','active',1,1,?1,'repo')", [DEFAULT_GROUP_ID]).unwrap();
            if hosting {
                conn.execute(
                    "UPDATE repositories SET remote='https://github.com/person/code.git'",
                    [],
                )
                .unwrap();
            }
            if has_table(&conn, "occasions").unwrap() {
                conn.execute("INSERT INTO occasions (id,group_id,title,starts_at,created_at,updated_at) VALUES ('meeting',?1,'Preserve my calendar',1,1,1)", [DEFAULT_GROUP_ID]).unwrap();
            }
            run(&mut conn).unwrap();
            run(&mut conn).unwrap();
            assert!(conn
                .prepare("PRAGMA foreign_key_check")
                .unwrap()
                .query([])
                .unwrap()
                .next()
                .unwrap()
                .is_none());
            assert!(!has_table(&conn, "repositories").unwrap());
            let kept: i64 = conn
                .query_row("SELECT count(*) FROM agents WHERE id='coder'", [], |r| r.get(0))
                .unwrap();
            assert_eq!(kept, 1, "version {version}, hosting {hosting}");
            assert!(has_table(&conn, "occasions").unwrap());
            assert!(has_table(&conn, "pending_runs").unwrap());
            conn.prepare("SELECT browser_consent FROM agents").unwrap();
            if !hosting && version >= 46 {
                assert_eq!(
                    conn.query_row("SELECT title FROM occasions WHERE id='meeting'", [], |r| {
                        r.get::<_, String>(0)
                    })
                    .unwrap(),
                    "Preserve my calendar"
                );
            }
            if hosting && version == 46 {
                assert_eq!(
                    conn.query_row(
                        "SELECT message_id FROM pending_runs WHERE run_id='unfinished'",
                        [],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                    "original-message"
                );
            }
        }
    }

    #[test]
    fn a_permission_recorded_before_questions_existed_is_still_a_permission() {
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        for (version, sql) in MIGRATIONS.iter().take_while(|(v, _)| *v < 33) {
            tx.execute_batch(sql).unwrap();
            tx.pragma_update(None, "user_version", *version).unwrap();
        }
        tx.commit().unwrap();

        conn.execute(
            "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,
                                 version,created_at,updated_at,group_id)
             VALUES ('a','A','avocado','#000','m','','[]','active',1,0,0,?1)",
            rusqlite::params![DEFAULT_GROUP_ID],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO approvals (id,agent_id,group_id,run_id,action,summary,detail,state,
                                    created_at)
             VALUES ('ap','a',?1,'run','createAgent','Wants to hire','[]','alwaysAllow',0)",
            rusqlite::params![DEFAULT_GROUP_ID],
        )
        .unwrap();

        run(&mut conn).unwrap();

        let (action, options, answer): (String, Option<String>, Option<String>) = conn
            .query_row("SELECT action,options,answer FROM approvals WHERE id='ap'", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!(action, "createAgent", "an upgrade must not restate what was asked");
        assert_eq!(options, None);
        assert_eq!(answer, None, "nobody wrote an answer, so there must not be one");
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
    fn a_recovery_point_written_before_starts_were_recorded_counts_as_started() {
        let mut conn = memory();
        for (v, sql) in MIGRATIONS.iter().take_while(|(v, _)| *v <= 74) {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", *v).unwrap();
        }
        conn.execute("INSERT INTO messages (id,run_id,channel_id,from_kind,to_kind,parts,trust,hop,expects_reply,created_at) VALUES ('original','run','channel','human','system','[]','operator',0,0,0)", []).unwrap();
        conn.execute("INSERT INTO pending_runs (run_id,message_id) VALUES ('run','original')", [])
            .unwrap();
        run(&mut conn).unwrap();
        let started: Option<i64> = conn
            .query_row("SELECT started_at FROM pending_runs WHERE run_id='run'", [], |r| r.get(0))
            .unwrap();
        assert!(started.is_some());
    }

    #[test]
    fn a_withdrawn_plugin_takes_its_rows_and_leaves_the_others() {
        let mut conn = memory();
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
    fn a_plugin_connected_before_tools_could_be_switched_off_keeps_all_of_them() {
        let mut conn = memory();
        for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v < 29) {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", *version).unwrap();
        }
        conn.execute(
            "INSERT INTO plugins (id,group_id,kind,account,tools,connected_at)
             VALUES ('p1',?1,'neon','','[{\"name\":\"run_sql\",\"description\":\"\",
                                          \"inputSchema\":{}}]',0)",
            rusqlite::params![DEFAULT_GROUP_ID],
        )
        .unwrap();

        run(&mut conn).unwrap();

        let off: i64 = conn
            .query_row("SELECT count(*) FROM plugin_tool_access", [], |row| row.get(0))
            .unwrap();
        assert_eq!(off, 0, "a migration that switched anything off would break a working crew");
    }

    #[test]
    fn a_plugin_connected_before_a_server_could_be_added_keeps_dialling_the_vendor() {
        let mut conn = memory();
        for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v < 35) {
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

        let endpoint: String = conn
            .query_row("SELECT endpoint FROM plugins WHERE id='p1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(endpoint, "", "an address in the row is one the build can no longer change");
    }

    #[test]
    fn a_plugin_connected_before_headers_existed_sends_none() {
        let mut conn = memory();
        for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v < 36) {
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

        let headers: String = conn
            .query_row("SELECT headers FROM plugins WHERE id='p1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(headers, "[]");
    }

    #[test]
    fn a_server_added_by_hand_under_a_name_the_catalog_takes_keeps_its_address() {
        a_row_the_tile_did_not_write_keeps_its_address(
            54,
            crate::domain::plugin::PluginKind::Firecrawl,
            "https://www.firecrawl.dev/api/oauth/token",
            "https://mcp.firecrawl.dev/v2/mcp",
            "stdio:npx -y firecrawl-mcp",
        );
    }

    #[test]
    fn supermemory_takes_its_name_back_from_a_hand_added_row_the_way_firecrawl_did() {
        a_row_the_tile_did_not_write_keeps_its_address(
            55,
            crate::domain::plugin::PluginKind::Supermemory,
            "https://api.supermemory.ai/api/auth/oauth2/token",
            "https://memory.example.com/mcp",
            "stdio:node supermemory.js",
        );
    }

    fn a_row_the_tile_did_not_write_keeps_its_address(
        version: i32,
        vendor: crate::domain::plugin::PluginKind,
        token: &str,
        elsewhere: &str,
        program: &str,
    ) {
        use crate::domain::plugin::PluginKind;
        let (slug, oauth) = (vendor.slug(), vendor.endpoint());
        let added = format!("{slug}_added");
        let mut conn = memory();
        for (at, sql) in MIGRATIONS.iter().filter(|(v, _)| *v < version) {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", *at).unwrap();
        }
        for group in ["g1", "g2", "g3", "g4", "g5"] {
            conn.execute("INSERT INTO groups (id,name,created_at) VALUES (?1,?1,0)", [group])
                .unwrap();
        }
        conn.execute(
            "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,
                                 version,created_at,updated_at,group_id)
             VALUES ('a1','A','avocado','#000','m','','[]','active',1,0,0,'g2')",
            [],
        )
        .unwrap();
        let insert = "INSERT INTO plugins
                          (id,group_id,kind,account,tools,token_endpoint,access_token,
                           connected_at,endpoint)
                      VALUES (?1,?2,?3,'','[]',?4,'tok',0,?5)";
        for (id, group, kind, token_endpoint, endpoint) in [
            ("signed", "g1", slug, token, oauth),
            ("keyed", "g2", slug, "", elsewhere),
            ("pasted", "g3", slug, "", oauth),
            ("program", "g4", slug, "", program),
            ("crowded", "g5", slug, "", elsewhere),
            ("taken", "g5", added.as_str(), "", "https://example.com/mcp"),
            ("vendor", "g1", "neon", token, ""),
        ] {
            conn.execute(insert, rusqlite::params![id, group, kind, token_endpoint, endpoint])
                .unwrap();
        }
        conn.execute("INSERT INTO plugin_agents (plugin_id, agent_id) VALUES ('keyed','a1')", [])
            .unwrap();

        run(&mut conn).unwrap();

        let row = |id: &str| -> (String, String) {
            conn.query_row("SELECT kind, endpoint FROM plugins WHERE id=?1", [id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap()
        };
        let resolved = |id: &str| {
            let (kind, endpoint) = row(id);
            PluginKind::from_row(&kind, &endpoint).unwrap()
        };
        assert_eq!(row("signed"), (slug.to_string(), "".into()));
        assert_eq!(resolved("signed"), vendor, "a grant for the same resource");
        for (id, endpoint) in [("keyed", elsewhere), ("pasted", oauth), ("program", program)] {
            assert_eq!(
                resolved(id),
                PluginKind::Custom { slug: added.clone(), endpoint: endpoint.into() },
                "{id} is still the operator's own server"
            );
        }
        let named: i64 = conn
            .query_row("SELECT count(*) FROM plugin_agents WHERE plugin_id='keyed'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(named, 1, "who may spend it hangs off the id and survives the rename");
        assert_eq!(row("crowded").0, slug);
        assert_eq!(row("taken").0, added);
        assert_eq!(resolved("vendor"), PluginKind::Neon);
    }

    #[test]
    fn a_tool_switched_off_before_agents_could_be_named_stays_off_for_all_of_them() {
        let mut conn = memory();
        for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v < 31) {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", *version).unwrap();
        }
        conn.execute(
            "INSERT INTO plugins (id,group_id,kind,account,tools,connected_at)
             VALUES ('p1',?1,'neon','','[]',0)",
            rusqlite::params![DEFAULT_GROUP_ID],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO plugin_denied_tools (plugin_id,tool) VALUES ('p1','drop_project')",
            [],
        )
        .unwrap();

        run(&mut conn).unwrap();

        let access: String = conn
            .query_row(
                "SELECT access FROM plugin_tool_access WHERE plugin_id='p1' AND tool='drop_project'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(access, "chosen", "a refusal is a tool narrowed to nobody");
        let named: i64 = conn
            .query_row("SELECT count(*) FROM plugin_tool_agents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(named, 0, "nobody is nobody: a name here would hand the tool to somebody");
    }

    #[test]
    fn the_avatar_column_is_renamed_and_keeps_its_data() {
        let mut conn = memory();
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
    fn the_signin_column_is_renamed_and_keeps_its_data() {
        let mut conn = memory();
        for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v < 32) {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", *version).unwrap();
        }
        conn.execute(
            "INSERT INTO agents
             (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,
              created_at,updated_at,group_id)
             VALUES ('a','Manager','avocado','#000','m','','[]','active',1,0,0,?1)",
            rusqlite::params![DEFAULT_GROUP_ID],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO signins
             (agent_id,surface,domain,service,recognised,first_seen_at,last_seen_at)
             VALUES ('a','computer','github.com','GitHub',1,7,9)",
            [],
        )
        .unwrap();

        run(&mut conn).unwrap();

        let (service, recognized, since): (String, i64, i64) = conn
            .query_row(
                "SELECT service,recognized,first_seen_at FROM signins WHERE agent_id='a'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(service, "GitHub", "the rename must not drop the row");
        assert_eq!(recognized, 1, "nor the value the renamed column was holding");
        assert_eq!(since, 7, "nor when the sign-in was first seen");
    }

    #[test]
    fn group_inference_settings_start_empty_and_mean_inherit() {
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

        let untouched: Option<String> = conn
            .query_row("SELECT subscription_model FROM groups WHERE id='g2'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(untouched, None);
    }

    #[test]
    fn a_group_that_already_had_an_endpoint_is_written_down_as_choosing_one() {
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
        assert_eq!(provider("g3"), None);
    }

    #[test]
    fn group_limits_start_unset_and_mean_inherit() {
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
        let mut conn = memory();
        run(&mut conn).unwrap();
        let insert = "INSERT INTO connectors (id,group_id,service,account,env_var,secret,note,created_at,updated_at)
                      VALUES (?1,?2,?3,'me',?4,'s','',0,0)";

        conn.execute(insert, rusqlite::params!["a", DEFAULT_GROUP_ID, "GitHub", "TOKEN"]).unwrap();
        let clash =
            conn.execute(insert, rusqlite::params!["b", DEFAULT_GROUP_ID, "Linear", "TOKEN"]);
        assert!(clash.is_err(), "a duplicate variable name in one group must be rejected");

        conn.execute("INSERT INTO groups (id,name,created_at) VALUES ('g2','Research',0)", [])
            .unwrap();
        conn.execute(insert, rusqlite::params!["c", "g2", "Linear", "TOKEN"])
            .expect("groups do not share an environment");
    }

    #[test]
    fn declared_sign_ins_are_dropped_rather_than_carried_into_detection() {
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

        let signins: i64 =
            conn.query_row("SELECT count(*) FROM signins", [], |r| r.get(0)).unwrap();
        assert_eq!(signins, 0);
    }

    #[test]
    fn an_existing_schedule_keeps_firing_on_exactly_the_gap_it_was_set_for() {
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
    fn an_existing_schedule_keeps_its_firings_and_lands_on_the_ordinary_rule() {
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        for (version, sql) in MIGRATIONS.iter().take_while(|(v, _)| *v < 34) {
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
             VALUES ('r1','a','Sweep','check','weekdays',1,1750000000000,1740000000000,5)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO routine_runs (routine_id,run_id,kind,at) VALUES ('r1','run-1','scheduled',7)",
            [],
        )
        .unwrap();

        run(&mut conn).unwrap();

        let skipping: i64 = conn
            .query_row("SELECT skip_if_working FROM routines WHERE id='r1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(skipping, 0, "an upgrade must not start dropping an operator's firings");

        let (run_id, kind, at): (String, String, i64) = conn
            .query_row("SELECT run_id,kind,at FROM routine_runs", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!((run_id.as_str(), kind.as_str(), at), ("run-1", "scheduled", 7));

        conn.execute(
            "INSERT INTO routine_runs (routine_id,run_id,kind,at) VALUES ('r1',NULL,'skipped',9)",
            [],
        )
        .unwrap();
        let indexed: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master
                  WHERE type='index' AND name='routine_runs_routine'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(indexed, 1, "a rebuild takes the index with it unless it is made again");

        conn.pragma_update(None, "foreign_keys", true).unwrap();
        conn.execute("DELETE FROM routines WHERE id='r1'", []).unwrap();
        let left: i64 =
            conn.query_row("SELECT count(*) FROM routine_runs", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0, "history for a deleted routine is history nothing can draw");
    }

    #[test]
    fn foreign_key_enforcement_is_off_only_while_migrations_run() {
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
    fn an_upgrade_arranges_the_crews_in_the_order_they_were_already_drawn() {
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        for (version, sql) in MIGRATIONS.iter().take_while(|(v, _)| *v < 64) {
            tx.execute_batch(sql).unwrap();
            tx.pragma_update(None, "user_version", *version).unwrap();
        }
        tx.commit().unwrap();

        for (id, made) in [("late", 300), ("early", 100), ("twin-a", 200), ("twin-b", 200)] {
            conn.execute(
                "INSERT INTO groups (id,name,created_at) VALUES (?1,?1,?2)",
                rusqlite::params![id, made],
            )
            .unwrap();
        }

        run(&mut conn).unwrap();

        let arranged: Vec<(String, i64)> = conn
            .prepare("SELECT id, rail_order FROM groups ORDER BY rail_order")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            arranged,
            vec![
                (DEFAULT_GROUP_ID.into(), 0),
                ("early".into(), 1),
                ("twin-a".into(), 2),
                ("twin-b".into(), 3),
                ("late".into(), 4),
            ],
            "an upgrade must draw the column it drew before, and give every crew its own place"
        );
    }

    #[test]
    fn an_agent_that_worked_in_a_repository_keeps_its_other_answers_through_the_rebuild() {
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        for (version, sql) in MIGRATIONS.iter().take_while(|(v, _)| *v < 58) {
            tx.execute_batch(sql).unwrap();
            tx.pragma_update(None, "user_version", *version).unwrap();
        }
        tx.commit().unwrap();

        conn.execute(
            "INSERT INTO repositories (id,group_id,name,path,harness,gate,bench,created_at,updated_at)
             VALUES ('r1',?1,'guaca','/dev/guaca','codex','askBeforePushing','own',1,1)",
            rusqlite::params![DEFAULT_GROUP_ID],
        )
        .unwrap();
        for (id, repository, errands) in [("coder", Some("r1"), true), ("writer", None, false)] {
            conn.execute(
                "INSERT INTO agents (id,name,avatar,color,model,system_prompt,lifecycle,created_at,
                                     updated_at,group_id,repository_id,browser_consent,runs_errands)
                 VALUES (?1,?1,'orb','#7fb069','m','','active',1,1,?2,?3,'askBeforeActing',?4)",
                rusqlite::params![id, DEFAULT_GROUP_ID, repository, errands],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO working_notes (agent_id,at,body) VALUES ('coder',1,'halfway')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO artifacts (id,group_id,owner_id,title,version,created_at,updated_at)
             VALUES ('page',?1,'coder','Plan',1,1,1)",
            [DEFAULT_GROUP_ID],
        )
        .unwrap();

        run(&mut conn).unwrap();

        let consent = |id: &str| -> String {
            conn.query_row("SELECT browser_consent FROM agents WHERE id=?1", [id], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(consent("coder"), "askBeforeActing");
        assert_eq!(consent("writer"), "askBeforeActing");
        assert!(!has_table(&conn, "repositories").unwrap());
        let notes: i64 =
            conn.query_row("SELECT count(*) FROM working_notes", [], |r| r.get(0)).unwrap();
        assert_eq!(notes, 1, "the rebuild must not take what points at an agent with it");
        let owner: Option<String> = conn
            .query_row("SELECT owner_id FROM artifacts WHERE id='page'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(owner.as_deref(), Some("coder"));
        let errands = |id: &str| -> bool {
            conn.query_row("SELECT runs_errands FROM agents WHERE id=?1", [id], |r| r.get(0))
                .unwrap()
        };
        assert!(
            errands("coder") && !errands("writer"),
            "an errand opt-in is the operator's answer"
        );
        assert!(conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none());
        let clash = conn.execute(
            "INSERT INTO agents (id,name,avatar,color,model,system_prompt,lifecycle,created_at,
                                 updated_at,group_id)
             VALUES ('twin','CODER','orb','#7fb069','m','','active',1,1,?1)",
            [DEFAULT_GROUP_ID],
        );
        assert!(clash.is_err(), "the unique index on live names was not recreated");
    }

    #[test]
    fn an_agent_given_a_browser_before_this_column_existed_is_not_held_back_by_it() {
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        for (version, sql) in MIGRATIONS.iter().take_while(|(v, _)| *v < 45) {
            tx.execute_batch(sql).unwrap();
            tx.pragma_update(None, "user_version", *version).unwrap();
        }
        tx.commit().unwrap();

        conn.execute(
            "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,
                                 version,created_at,updated_at,group_id,has_browser)
             VALUES ('a1','Researcher','orb','#7fb069','m','','[]','active',1,1,1,?1,1)",
            rusqlite::params![DEFAULT_GROUP_ID],
        )
        .unwrap();

        run(&mut conn).unwrap();

        let consent: String = conn
            .query_row("SELECT browser_consent FROM agents WHERE id='a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(consent, "open", "an upgrade must not start asking about work already running");
    }

    #[test]
    fn an_agents_model_lands_in_the_field_of_the_provider_that_ran_it() {
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        for (version, sql) in MIGRATIONS.iter().take_while(|(v, _)| *v < 63) {
            tx.execute_batch(sql).unwrap();
            tx.pragma_update(None, "user_version", *version).unwrap();
        }
        tx.commit().unwrap();

        for (group, provider) in [
            ("plan", Some("chatgpt")),
            ("key", Some("compatible")),
            ("claude", Some("claude")),
            ("follows", None),
        ] {
            conn.execute(
                "INSERT INTO groups (id,name,created_at,provider) VALUES (?1,?1,0,?2)",
                rusqlite::params![group, provider],
            )
            .unwrap();
        }
        let agents = [
            ("engineer", "plan", "xiaomi/mimo-v2.6-pro"),
            ("assistant", "plan", "gpt-6-astra"),
            ("inheritor", "plan", ""),
            ("router", "key", "anthropic/claude-sonnet-4.5"),
            ("local", "key", "qwen2.5-coder"),
            ("kept", "claude", "gpt-6-astra"),
            ("unsure", "follows", "gpt-6-astra"),
            ("routed", "follows", "openai/gpt-5"),
        ];
        for (id, group, model) in agents {
            conn.execute(
                "INSERT INTO agents (id,name,avatar,color,model,system_prompt,lifecycle,created_at,
                                     updated_at,group_id)
                 VALUES (?1,?1,'orb','#7fb069',?2,'','active',1,1,?3)",
                rusqlite::params![id, model, group],
            )
            .unwrap();
        }

        run(&mut conn).unwrap();

        let read = |id: &str| -> (String, String) {
            conn.query_row("SELECT model, subscription_model FROM agents WHERE id=?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap()
        };
        let both = |model: &str| (model.to_string(), model.to_string());
        let endpoint = |model: &str| (model.to_string(), String::new());
        let plan = |model: &str| (String::new(), model.to_string());
        assert_eq!(read("engineer"), endpoint("xiaomi/mimo-v2.6-pro"), "off the subscription");
        assert_eq!(read("assistant"), plan("gpt-6-astra"));
        assert_eq!(read("inheritor"), endpoint(""), "blank still inherits on both");
        assert_eq!(read("router"), endpoint("anthropic/claude-sonnet-4.5"));
        assert_eq!(read("local"), endpoint("qwen2.5-coder"), "a key pays, whatever the name");
        assert_eq!(read("kept"), both("gpt-6-astra"));
        assert_eq!(read("unsure"), both("gpt-6-astra"));
        assert_eq!(read("routed"), endpoint("openai/gpt-5"));
    }

    #[test]
    fn a_failed_migration_leaves_the_version_untouched() {
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

    #[test]
    fn a_decision_answer_written_as_text_becomes_the_part_and_says_the_same_thing() {
        use crate::domain::decision::{delivery, DecisionRequest};
        use crate::domain::envelope::{Envelope, Intent, Part, Participant, Trust};
        use crate::domain::ids::{AgentId, DecisionId, MessageId, RunId};

        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        for (version, sql) in MIGRATIONS.iter().take_while(|(v, _)| *v < 61) {
            tx.execute_batch(sql).unwrap();
            tx.pragma_update(None, "user_version", *version).unwrap();
        }
        tx.commit().unwrap();

        let agent = AgentId::new();
        let request = DecisionRequest {
            question: "Which address goes in the footer?".into(),
            context: "FTC guidance says \"valid postal address\".\nTwo lines.".into(),
            recommendation: String::new(),
            options: vec![],
            source: "https://www.ftc.gov/business-guidance".into(),
        };
        let write = |from: Participant, text: String| {
            let message = Envelope {
                id: MessageId::new(),
                run_id: RunId::new(),
                channel_id: agent,
                from,
                to: Participant::Agent { id: agent },
                parts: vec![Part::text(text)],
                trust: Trust::System,
                hop: 0,
                expects_reply: true,
                intent: Intent::Work,
                cause: None,
                created_at: 1,
            };
            crate::db::Store::insert_message(&conn, &message).unwrap();
            message
        };
        let answered = DecisionId::new();
        let resumed = DecisionId::new();
        let answer = "No address.\nShow me the copy before \"sending\" anything.";
        let plain = write(Participant::System, delivery(answered, &request, answer, false));
        let again = write(Participant::System, delivery(resumed, &request, answer, true));
        let typed = write(Participant::Human, delivery(answered, &request, answer, false));
        let broken = write(
            Participant::System,
            "The operator answered decision x. Nothing below.\n{\"answer\":".into(),
        );
        let other = write(Participant::System, "The run was stopped.".into());

        run(&mut conn).unwrap();

        let parts = |message: &Envelope| -> Vec<Part> {
            let raw: String = conn
                .query_row(
                    "SELECT parts FROM messages WHERE id=?1",
                    [message.id.to_string()],
                    |r| r.get(0),
                )
                .unwrap();
            serde_json::from_str(&raw).unwrap()
        };
        assert_eq!(
            parts(&plain),
            vec![Part::Decision {
                id: answered,
                request: request.clone(),
                answer: answer.into(),
                resumed: false
            }]
        );
        assert_eq!(
            parts(&again),
            vec![Part::Decision { id: resumed, request, answer: answer.into(), resumed: true }]
        );
        for message in [&plain, &again] {
            let now = Envelope { parts: parts(message), ..message.clone() };
            assert_eq!(now.plain_text(), message.plain_text(), "the agent must read what it read");
        }
        for message in [&typed, &broken, &other] {
            assert_eq!(parts(message), message.parts, "not an answer this wrote");
        }
    }
}
