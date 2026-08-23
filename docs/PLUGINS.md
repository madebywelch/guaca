# Plugins

A plugin is a server a crew signs in to once. After that, the agents that crew
chose are offered that server's tools on every turn, and none of them ever holds
the sign-in.

There are six: Neon, Cloudflare, Linear, Stripe, AgentMail and Google. That is
the whole list, and it is a decision rather than a starting position. Five of
them are the vendor's own server, and every count of "the five" below means
those; Google is the operator's own account, and *Google is a plugin whose
sign-in is the account's* is why.

## Why these, and why the list is short

The list this replaced was twelve brands and a text box. Each tile filled in an
environment variable name and a note, and then asked the operator for a token.
That is a worse offer than it looks, because after the token is pasted Guaca
knows nothing else: not what the service can do, not what an agent should call,
not whether the token is still good. The agent was handed a variable name and
left to work out an API from whatever it happened to know.

A server that publishes its own tools answers all of that itself. `tools/list`
is the documentation, the schema and the capability list in one call, and it is
current because it comes from the vendor at the moment of connecting.

Three conditions decide who is on the list, and all three are mechanical:

1. **It publishes its own tools.** `tools/list` at the moment of connecting,
   not a note in this repo about what the vendor's API can do.
2. **Those tools act on the operator's account.** A plugin is a thing a crew
   *has*, named in the prompt as something that reaches the real world.
3. **Its authorization server lets an application register itself.** The next
   section is why this one is not negotiable.

## Why Clerk was withdrawn

Clerk was on the list and is not any more, and the second condition is why.
`mcp.clerk.com` publishes two tools, `clerk_sdk_snippet` and
`list_clerk_sdk_snippets`, and both return SDK code samples. Nothing there
reads or writes an operator's Clerk account, so what the tile actually offered
was documentation reached through a connect button, sitting beside four servers
that can drop a database, deploy a Worker, close an issue and refund a payment.
It also failed the third condition on the merits: its server is public, so it
was the one entry on the list that never signed anything in, and the row said so
in the UI.

Documentation an agent reads is not nothing, but it is not this. It arrives
through the model's own training, through a docs tool the vendor already exposes
elsewhere, or through the web. What a plugin is for is the account.

Migration 25 deletes the rows. No grant is lost, because Clerk's server issued
none.

## Why dynamic client registration decides who is on the list

An OAuth client normally has to exist before it can be used: somebody signs in
to the vendor's dashboard, creates an application, and pastes a client id into
the code. Guaca cannot do that for a desktop app that anybody can build — there
is no Guaca account at Neon to register under, and a client id shipped in a
binary anybody can read is not a secret anyway.

RFC 7591 removes the problem. Guaca registers itself, on the spot, each time an
operator connects, with a redirect URI it has already bound. No vendor
relationship is needed and nothing is baked into the build.

So the third condition is mechanical rather than editorial: the server has to
publish protected-resource metadata, and its authorization server has to publish
a registration endpoint. A vendor that stops has to be withdrawn rather than
debugged, and `scripts/plugins.sh` is what says so, because nothing offline can.

The five answer that question in three different shapes, which is what the
fallbacks in `oauth::discover` are for rather than defensiveness. Neon publishes
its resource metadata at the bare well-known path and Linear under the
endpoint's path. Stripe's authorization server is `https://access.stripe.com/mcp`
— an issuer with a path, where RFC 8414 puts the well-known segment *before* it
and everybody's first guess puts it after. AgentMail's authorization server is
Clerk's, hosted at `clerk.console.agentmail.to`, which is a fair description of
what Clerk is for.

## The loopback redirect, and why it is allowed here

`subscription.rs` argues against a loopback redirect for the ChatGPT sign-in and
uses the device flow instead. Both of its objections are real: a fixed port may
already belong to something else, and a URL scheme is claimed by whichever build
registered last.

Neither applies here, because nothing is chosen in advance. The order is:

1. Bind `127.0.0.1:0`. The operating system hands back a port that is free by
   construction.
2. Register a client whose redirect URI names *that* port.
3. Send the operator to the authorization endpoint.

The port cannot be taken between choosing it and listening on it, because it was
never chosen: it was allocated. Dynamic registration is what makes the ordering
possible, and it is the same mechanism that decides who is on the list at all.

The device flow is not an option here anyway. None of the five advertises it,
and the MCP authorization specification mandates authorization code with PKCE.

## What a plugin asks for is the resource's list, not Guaca's

Guaca never invents a scope. A scope it invented is a scope the server refuses
in a browser window that cannot explain why, and the operator is left reading
`invalid_scope` on a vendor's error page.

Two documents publish a list, and they are not the same list.

| Document | RFC | What its `scopes_supported` means |
|---|---|---|
| Protected resource, at the MCP server | 9728 | What to ask for to reach *this resource* |
| Authorization server, at the issuer | 8414 | Everything the issuer can grant, for every resource behind it and every client registered by any means |

The resource's is the one asked for. The server's is the fallback for a resource
that says nothing, which is most of them.

AgentMail is why, and it was found the hard way. Its MCP server names three
scopes: `openid email profile`. The Clerk instance behind it lists seven, and
four of those are ones a vendor grants to clients it created by hand, not to one
that registered itself. Asking the resource's question of the authorization
server got every sign-in refused: *The OAuth 2.0 Client is not allowed to
request scope 'public_metadata'*. Linear has the same shape and had not broken
yet: its resource wants `read write` and its issuer also lists `openid email`.

Two rules sit on top of that, and both only ever name a scope the server itself
published:

- **`*` is dropped wherever it appears.** Neon offers one. An operator
  connecting a database has not agreed to hand over everything their account can
  do, and the named scopes beside it add up to the part Guaca needs.
- **`offline_access` is added when the authorization server names it.** It is
  not access to anything. It is the scope that decides whether a refresh token
  comes back, and without one a plugin works until the access token expires and
  then asks the operator to sign in again, every hour, for as long as they keep
  it connected.

`Discovered::requested_scope` is the whole rule, and the live half of
`tests/plugins.rs` is what keeps it honest: it asserts that every scope this
build would send is one the vendor publishes today. Offline tests cannot see a
vendor narrowing what a registered client may ask for, which is exactly how
AgentMail broke while all five discovered correctly.

Where neither document publishes a list, Guaca sends no `scope` parameter at
all, and the server applies its own default. Cloudflare is that case, and its
consent screen is a permission picker that opens on **read only**. The operator
widens it there, on the screen that says what each scope is, or leaves it and
has a crew that can look at the account without changing it.

The way to reach more of a vendor's surface is therefore the consent screen or
another entry on this list, never another parameter on the request.

## Cloudflare is the account, not a product area

Cloudflare publishes two kinds of MCP server and Guaca dials the one that is not
a product area.

The fifteen `*.mcp.cloudflare.com` hosts are one product each: Workers bindings,
DNS, Radar, observability, and so on. Curated, typed, and a fifteenth of the
account apiece. Picking one is deciding on the operator's behalf which fifteenth
their crew gets, with nothing on the tile saying which, and an agent that can
create a Worker and cannot read a DNS record spends the turn discovering that.
Offering several instead is a hundred tool definitions in front of a model that
asked for "Cloudflare", paid for on every turn by every agent in the crew.

`mcp.cloudflare.com` is the whole Cloudflare API — over 2,500 endpoints — behind
`search` and `execute`, plus `docs`. The model writes JavaScript against the
OpenAPI document and Cloudflare runs it in a sandboxed Worker, so the API
surface stays on the server: about a thousand tokens of context rather than the
million the same endpoints would cost as tool definitions. It is the only server
on the list that works this way, and it is the reason Cloudflare can be one
entry rather than fifteen.

Migration 26 deletes the rows from before the move. Nothing on one survives it:
the tokens were issued by the old host's issuer and the new one refuses them,
and the stored tool list names tools the new server does not have — which is
what the crew is offered on every turn until something re-reads it. Left in
place, an agent calls `cloudflare__workers_list`, takes a 401 from a host it was
never signed in to, and reports the operator's account as broken. Deleted, the
tile says "Connect" and one consent screen fixes it.

## What crosses which boundary

| Thing | Lives in | Ever reaches the model | Ever reaches the webview |
|---|---|---|---|
| Access token | `plugins.access_token` | No | No |
| Refresh token | `plugins.refresh_token` | No | No |
| Client registration | `plugins.client_id`, `client_secret` | No | No |
| Tool names and schemas | `plugins.tools` | Yes, as tool definitions | Names only |
| Which plugins are connected | `plugins` | Yes, one line each | Yes |
| The account token, for Google | `account.json`, read per call | No | No |

This is the boundary `connector_env` draws around a pasted secret, moved one
layer further out. A credential is at least handed to a sandbox, where the agent
can echo it; a plugin's grant never leaves the host process except onto the wire
back to the server that issued it. There is no field on `Plugin` for a token to
arrive in, and no command that returns one.

## Google is a plugin whose sign-in is the account's

Five of the six plugins are somebody else's server, and a crew signs in to each
one separately because there is nothing else it could do. Google is not a
server. It is the operator's own account at `guaca.bot`, which already holds the
Google grant, already refreshes it, and already knows which capabilities were
authorized. `PluginKind::account_backed` is the one bit that says so.

Running the ordinary flow for it would be wrong twice. It would send an operator
to a consent screen to authorize something they authorized when they signed in
to the account, and it would leave a per-group grant sitting beside a
per-account one for the same access, each expiring on its own clock, each
renewable independently, and only one of them the truth.

So the credential is the account's and the *decision to use it* stays the
group's. Connecting Google in a crew is what puts its tools in front of that
crew, and `PluginAccess` still decides which of its agents. Nothing else about a
plugin changes:

- The tool list is read once, on connect, the same way.
- The call still goes out of Guaca, never off an agent's machine.
- The token still never reaches a prompt, a transcript, an event or a sandbox.
- The reach check runs before the server is dialled, so an agent the operator
  did not choose is refused here rather than there, and so is a tool it was not
  given: `gmail_send` for one agent and `gmail_search` for another is the same
  pair of questions as everywhere else.

The row stores no grant, and that is the point rather than an omission: the
account rotates its own token, so a copy on the row would be a second thing to
keep fresh, a second thing to be stale, and a renewal path racing the account's.
`Runtime::account_token` reads a live one per call.

**A crew chooses which identity it uses.** A person can authorize the same
provider twice — a work Google and a personal one — and those are two grants at
`guaca.bot` with two ids. `plugins.connection` holds the one this crew means,
and it is part of the address: `/mcp/<id>` rather than `/mcp`. Two groups can
therefore hold two Google accounts at once, which is the case the single-grant
model could not express at all.

Empty means unnamed, and that is not a missing value. It is the account's
default, it is what every plugin connected before this column existed keeps
sending, and bare `/mcp` still answers with every connection's tools. An upgrade
that invented an id would silently repoint a working crew at a different
mailbox.

Changing it is `set_plugin_connection` rather than Disconnect and Connect,
because those are different acts: reconnecting replaces the row and loses the
per-tool switches the operator set. The tool list is re-read either way, because
two identities do not publish the same tools — a grant that can read mail and
not send it offers fewer.

**What the tools are is decided at `guaca.bot`, not here.** The server offers a
tool only when every scope it needs came back from Google, so a grant that can
read mail and not send it offers `gmail_search` and not `gmail_send`. A crew
that sees a tool it cannot use is a turn spent discovering a 403.

**Why the tools live there rather than here.** Guaca could have asked for a
Google access token and called Google itself: `/api/connectors/:provider/token`
exists and does exactly that. It would mean a live Google credential sitting on
a laptop for an hour at a time, and the app becoming responsible for it. Serving
tools instead keeps the token beside the refresh token that produced it, and
means the app needs no new machinery at all — it already speaks MCP.

## Signing in is one decision, and handing it out is another

A group holds the sign-in. Who may spend it is a second question, asked per
plugin, and until it was asked the answer was always "everybody in the crew".

That answer only holds while a crew is uniform, and a crew is not. Agents run on
different models at different competencies and cost, and they have different
jobs. The one that files issues has no business holding the account that issues
refunds, and an agent on a cheap model with Stripe in its tool list is a bad
trade whichever way the turn goes: it is either being asked to be careful with
something it cannot be careful with, or it is being paid for on every turn for a
capability it will never use.

So each connected plugin carries one of two answers:

- **Every agent.** The default, what every plugin connected before this existed
  keeps, and the right answer for most of them. It covers agents that do not
  exist yet: an agent hired next week gets it without anybody going back to the
  panel.
- **Only these agents.** A list, and it may be empty. An empty one means nobody,
  which is where an operator stands for the second between narrowing a plugin
  and ticking the first name.

Two states rather than a list with a sentinel, and the empty list is the reason.
"Everyone" is a decision about people who have not been hired, which no list of
today's ids can express, and a list that meant everyone when it was empty would
hand a plugin back to the whole crew at the moment the operator unticked the
last agent. `PluginAccess` is the type; `plugins.access` and `plugin_agents` are
the two columns it reads back out of.

### The rule is written once and read twice

`Store::plugin_tools` decides what an agent is *told* it has, and
`Store::plugin_reach` decides what it *gets*. Both paste the same SQL fragment,
`PLUGIN_REACHED_BY_AGENT`, because the two disagreeing is either a model calling
something it was never offered, or a model refused something the prompt told it
to use, which it will then try again with different arguments.

Filtering the tool definitions is not the enforcement. A model emits a tool name
it read somewhere often enough that a tool list has to be treated as a
description of what an agent has, never a fence: the call path asks the same
question again and refuses on its own. The refusal is a different sentence from
the one for a plugin nobody connected, and that matters more than it looks.
"Neon is not connected" sends the operator to a panel that says Disconnect, and
it never occurs to the agent to ask the peer who can. "Connected, but not for
you" names the way forward, which is the peer.

The peer is named for it too. An agent's roster already lists what each peer's
browser is signed in to, so that an agent asked for something it has no account
for can name the one who does rather than reporting that the crew cannot do it.
A plugin this agent does not have and a peer does is exactly that case, and it
is listed under exactly the same rule: only when this agent does not have it
itself, or the roster reads as a reason to delegate work it could do.

### What a change does not do

Reconnecting does not widen. `save_plugin` leaves `access` alone and keeps the
row's id, so an operator fixing a grant that was revoked at the vendor does not
silently hand Stripe back to the crew while they are fixing something else.

Retiring an agent takes its place with it, beside its approvals and its
sign-ins, and disconnecting a plugin takes every place on it. A row naming an
agent that no longer exists, or a plugin that is gone, is a standing permission
attached to nothing.

An access value this build does not recognize reads as a restriction, not as an
opening. Only the literal `everyone` widens a plugin past its named agents, in
the SQL and in `PluginAccess::from_row`. A permission that cannot be read has to
fail closed: a crew losing a plugin is visible and one click to fix, and a crew
silently gaining one is neither.

## And which of its tools, for which of them, which is a third decision

Signing in covers a server. It does not cover a capability, because a server
does not publish one kind of thing. Stripe lists the call that reads an invoice
beside the one that refunds it; Neon lists `run_sql` beside the call that
deletes a project; AgentMail lists reading a thread beside sending as the
operator. Until this existed, the only control over the second half of each of
those pairs was Disconnect, which also takes the first half away.

So each connected plugin carries a decision per tool, and it is the same
decision the plugin itself carries: every agent, or these agents and nobody
else. `plugins.tools` is still what the server published; `plugin_tool_access`
and `plugin_tool_agents` are the operator's answers about the ones they have
looked at.

**Everyone is the absence of a row.** A tool nobody has narrowed is callable by
every agent the plugin itself reaches, which is what every plugin connected
before this existed offers. It is the same reading `access` takes with
`everyone`, and for the same reason: the default has to cover what nobody has
seen yet. A vendor ships a tool between one connection and the next, and an
allow-list over tools would leave that tool switched off with nothing on screen
saying a decision had been made about it. Connecting again keeps the narrowings
and switches the new tool on.

**Inside a narrowed tool it is the other way round, and that is not an
inconsistency.** The named agents are stored and everybody else is refused. The
two defaults point at different unknowns: an unseen *tool* should behave like
the rest of the server the operator already authorized, and an unhired *agent*
must not inherit the one capability the operator went out of their way to fence
off. `PLUGIN_TOOL_REACHED_BY_AGENT` is both rules in one fragment.

**Nobody is a chosen list with nothing in it.** That is the whole of the old
two-way switch, kept: one click still switches a tool off for the crew, and the
panel still says so out loud. It is the same argument `PluginAccess` makes at
the plugin level, where the empty list is where an operator stands for the
second between narrowing something and ticking the first name. Migration 31
rewrites every row of `plugin_denied_tools` as exactly that and drops the table.

**Reconnecting does not widen one.** `save_plugin` replaces the tool list and
touches neither table, for the reason it leaves `access` alone: fixing a grant
that the vendor revoked is not a decision about what the crew may do, and one
that quietly handed `drop_project` back would undo the decision at the moment
the operator was fixing something else.

**Two agents on one plugin can have different halves of it, and that is the
point.** It is a matrix — five plugins, twenty tools, six agents — and the
objection to a matrix is real: nobody can hold one in their head, and nobody can
audit one. What makes it usable is that almost every cell is the default and is
never drawn. The panel asks the second question only about tools the operator
opened, and says nothing per tool while a tool is the default, because forty
rows repeating "offered to every agent in this group" is the default said forty
times and it buries the one row that is not. What an operator sees is the
narrowings they made.

The alternative is not a simpler control, it is a worse crew. One inbox, two
agents: the one that triages it reads and searches, the one that answers it
sends. A crew-wide switch could give both agents everything or take sending away
from both, and neither is the arrangement anybody wanted.

**The wider answer is given before the narrower one.** More than one refusal is
true at once, and the useful one covers the most ground. A tool narrowed to
nobody is off for the whole crew, so it is said before "you are not on this
plugin"; being off the plugin covers every tool on it, so it is said before
"this tool is not yours". `plugin_reach` asks in that order, and the four
refusals are four different sentences: `NotConnected` is the operator's to fix,
`ToolDenied` is nobody's, and `NotChosen` and `ToolNotChosen` are a peer's.

**The decision reaches the agent, in three places.** The tool never becomes a
definition, so the model cannot call it by accident. The prompt names it under
its plugin, the name alone with no description and no schema, because an agent
that is simply not shown `create_refund` answers "we cannot do refunds" to the
one person who could switch it back on — and under one of two headings, because
"nobody has this" and "a peer has this" send the turn to different places. And
the call path refuses it by name if the model emits it anyway, for the reason
the tool list is never the enforcement: a model names tools it read somewhere.

**The roster names the peer, per tool.** An agent that has Stripe and cannot
refund is exactly the case `reaches` exists for, and the plugin-level line says
nothing about it: this agent has Stripe. So a peer is named as *the Stripe
plugin's create_refund* when the gap is a tool, and as *the Stripe plugin* when
the gap is the whole thing. Only what this agent lacks and that peer holds, and
only what that peer can actually call: routing work to an agent that will be
refused in turn is the failure this exists to prevent, not one to commit.

The rule is read twice, like the one above it, but not from one SQL fragment.
The tool list is a JSON column, so `Store::plugin_tools` partitions it in Rust
and `Store::plugin_reach` asks in SQL. Both compare the server's own unprefixed
name to the same stored string, and store tests drive both refusals through both
queries so that the two cannot drift.

**A name on a tool the plugin does not reach grants nothing, and is kept.** The
two controls are set in either order, so a tool ticked for an agent before the
plugin was widened to them is a state to pass through rather than one to refuse.
The call path takes the intersection and the panel says which name is not
counting yet, because a permission panel naming an agent that would be refused
is the one thing it must not do.

## A tool name is `plugin__tool`

Two underscores, because MCP servers use one inside tool names constantly and
none of the five uses two. Split on the *first* separator, so a server tool
called `run__sql` keeps its own name whole.

A prefixed name that a provider would refuse — anything outside
`[A-Za-z0-9_-]{1,64}` — is dropped from the turn rather than renamed to fit.
Renaming would need a mapping back at call time, and a mapping nothing can see
is how a call lands on the wrong tool. The drop is logged.

## A plugin is named in the prompt as well as offered as tools

That is not a duplicate, and it is the same argument as an agent's own standing
routines. A tool list is read while deciding *how* to do something; the prompt
section is read while deciding whether it can be done at all, and that happens
first. An agent that skims twenty tool definitions and one that has been told its
crew has Neon behave differently when asked "can we check the database".

The section also carries the one thing a tool description cannot: these act on
the operator's real account. A database dropped through a plugin is dropped.

## A session per call

Every tool call opens a fresh MCP session: `initialize`, then the call. Keeping
the session would save a round trip and is not done, because a cached session is
a second thing that can go stale — the server can expire it, the token under it
can be refreshed, the crew can disconnect the plugin — and each of those
surfaces as a tool call that fails for no reason a model can act on.

The handshake is tens of milliseconds against a call that is already crossing the
internet to run somebody's SQL. This is the place to come back to if a
measurement ever says otherwise.

## Renewal happens twice, on purpose

Before the call, when the stored expiry says the grant is close to going; and
once more if the server refuses it anyway. The second is not belt and braces: a
token can be revoked at the vendor between one turn and the next, and nothing
local knows. One retry is the whole allowance — a refresh that does not fix it is
a sign-in the operator has to redo, and the refusal says exactly that.

## What is not here

**No approval gate on a plugin call.** `request_permission` covers acting in the
operator's name outside the workspace, and a plugin call qualifies. It is not
gated, because a prompt on every call would make plugins unusable, and the
existing gate is aimed at the case where a page an agent has just read chose the
button. The prompt carries the warning instead.

Narrowing a tool is not that gate and does not replace it. It is decided once,
in advance, by an operator who is looking at the whole tool list, and it never
interrupts anybody: an agent may call a tool or it may not, and nothing about
the particular call changes the answer. An approval gate is the other shape,
where the turn parks and the operator answers that one call, and it is still the
open question worth revisiting first. What this does remove is the worst case
that gate was being asked to cover, which is the one destructive tool on an
otherwise useful server, in the hands of the one agent that should not have it.

**No per-agent sign-in.** An agent is chosen from the crew's one grant; it does
not get its own. Two sign-ins to the same vendor for one group would be two sets
of tools under names a model cannot tell apart, and `plugins_kind_unique` says
so at the schema. An operator who wants two accounts at one vendor wants two
groups.

**No operator-typed endpoint.** "Any MCP server" would mean an operator pasting
a URL that Guaca then sends a crew's tokens to. The set is closed for the same
reason the tool list is not.

**No revocation at the vendor on disconnect.** The grant is dropped locally. Not
every authorization server publishes a revocation endpoint, and an operator who
wants the authorization itself withdrawn has to do that where they granted it.

## Testing

Three layers, and each catches something the others cannot.

- **Unit**, in `mcp.rs`, `oauth.rs` and `llm/tools.rs`: the two body shapes a
  streamable-HTTP server may answer with, the RFC 7636 PKCE example, the RFC 8414
  well-known path insertion, and the tool-name split.
- **Scripted**, in `tests/plugins.rs`: the real `oauth`, `mcp`, `plugins` and
  store code against a server that publishes all four metadata documents and
  answers MCP as an event stream. Includes a full runtime turn, so the tool
  definitions, the dispatch and the grant being spent are proved to meet, and
  one turn per axis where a model calls something it was not offered, plus one
  where two agents on one plugin are given different halves of it.
- **Live**, `./scripts/plugins.sh`: whether the five vendors still publish what
  this build expects. It runs `oauth::discover` — the same call a sign-in makes
  — rather than rebuilding the metadata URLs beside it, because a test with its
  own copy of RFC 8414 passes on a server this build cannot reach. Reaches the
  internet, authorizes nothing, spends nothing.

The live one is the one that matters over time. Everything offline is a stub
agreeing with what this app believes MCP authorization is, and the failure worth
catching is that belief going stale.
