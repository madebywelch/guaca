# Plugins

A plugin is a server a crew signs in to once. After that, every agent in the
group is offered that server's tools on every turn, and none of them ever holds
the sign-in.

There are five: Neon, Cloudflare, Linear, Stripe and AgentMail. That is the
whole list, and it is a decision rather than a starting position.

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
3. **Its authorisation server lets an application register itself.** The next
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
publish protected-resource metadata, and its authorisation server has to publish
a registration endpoint. A vendor that stops has to be withdrawn rather than
debugged, and `scripts/plugins.sh` is what says so, because nothing offline can.

The five answer that question in three different shapes, which is what the
fallbacks in `oauth::discover` are for rather than defensiveness. Neon publishes
its resource metadata at the bare well-known path and Linear under the
endpoint's path. Stripe's authorisation server is `https://access.stripe.com/mcp`
— an issuer with a path, where RFC 8414 puts the well-known segment *before* it
and everybody's first guess puts it after. AgentMail's authorisation server is
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
3. Send the operator to the authorisation endpoint.

The port cannot be taken between choosing it and listening on it, because it was
never chosen: it was allocated. Dynamic registration is what makes the ordering
possible, and it is the same mechanism that decides who is on the list at all.

The device flow is not an option here anyway. None of the five advertises it,
and the MCP authorisation specification mandates authorisation code with PKCE.

## What a plugin asks for is the server's list, not Guaca's

Connecting Cloudflare shows a consent screen with four permissions on it: user
read, account read, Workers write, D1 write. That is not Guaca choosing a
cautious subset. Cloudflare's Workers Bindings server hardcodes that scope set
for every client that connects to it, and its authorisation-server metadata
publishes no `scopes_supported` at all, so Guaca sends no `scope` parameter and
has nothing to widen.

Where a server does publish a list, Guaca asks for all of it minus `*`, because
a scope it invented is a scope the server refuses in a browser window that
cannot explain why, and a wildcard is not what an operator agreed to by
connecting a database. `oauth::requested_scope` is the whole rule.

The way to reach more of a vendor's surface is therefore another entry on the
list, not another parameter on the request. Cloudflare publishes fifteen MCP
servers, one per product area; Workers Bindings is the one that makes things
rather than reads about them, and offering all fifteen would put a hundred tools
in front of a model that asked for "Cloudflare".

## What crosses which boundary

| Thing | Lives in | Ever reaches the model | Ever reaches the webview |
|---|---|---|---|
| Access token | `plugins.access_token` | No | No |
| Refresh token | `plugins.refresh_token` | No | No |
| Client registration | `plugins.client_id`, `client_secret` | No | No |
| Tool names and schemas | `plugins.tools` | Yes, as tool definitions | Names only |
| Which plugins are connected | `plugins` | Yes, one line each | Yes |

This is the boundary `connector_env` draws around a pasted secret, moved one
layer further out. A credential is at least handed to a sandbox, where the agent
can echo it; a plugin's grant never leaves the host process except onto the wire
back to the server that issued it. There is no field on `Plugin` for a token to
arrive in, and no command that returns one.

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
button. The prompt carries the warning instead. This is the open question worth
revisiting first.

**No operator-typed endpoint.** "Any MCP server" would mean an operator pasting
a URL that Guaca then sends a crew's tokens to. The set is closed for the same
reason the tool list is not.

**No revocation at the vendor on disconnect.** The grant is dropped locally. Not
every authorisation server publishes a revocation endpoint, and an operator who
wants the authorisation itself withdrawn has to do that where they granted it.

## Testing

Three layers, and each catches something the others cannot.

- **Unit**, in `mcp.rs`, `oauth.rs` and `llm/tools.rs`: the two body shapes a
  streamable-HTTP server may answer with, the RFC 7636 PKCE example, the RFC 8414
  well-known path insertion, and the tool-name split.
- **Scripted**, in `tests/plugins.rs`: the real `oauth`, `mcp`, `plugins` and
  store code against a server that publishes all four metadata documents and
  answers MCP as an event stream. Includes a full runtime turn, so the tool
  definitions, the dispatch and the grant being spent are proved to meet.
- **Live**, `./scripts/plugins.sh`: whether the five vendors still publish what
  this build expects. It runs `oauth::discover` — the same call a sign-in makes
  — rather than rebuilding the metadata URLs beside it, because a test with its
  own copy of RFC 8414 passes on a server this build cannot reach. Reaches the
  internet, authorises nothing, spends nothing.

The live one is the one that matters over time. Everything offline is a stub
agreeing with what this app believes MCP authorisation is, and the failure worth
catching is that belief going stale.
