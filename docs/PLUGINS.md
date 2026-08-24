# Plugins

A plugin is a server a crew signs in to once. After that, the agents that crew
chose are offered that server's tools on every turn, and none of them ever holds
the sign-in.

Six come with the app: Neon, Cloudflare, Linear, Stripe, AgentMail and Google.
Five of them are the vendor's own server, and every count of "the five" below
means those; Google is the operator's own account, and *Google is a plugin whose
sign-in is the account's* is why.

A crew can also have a server nobody vouched for. *A server the operator added*
is the whole of that, and the short version is that it is not a second
mechanism: the operator supplies the two things the catalog was supplying, and
everything after those two strings is the code the six go through.

## Why these six, and why the catalog is short

The list this replaced was twelve brands and a text box. Each tile filled in an
environment variable name and a note, and then asked the operator for a token.
That is a worse offer than it looks, because after the token is pasted Guaca
knows nothing else: not what the service can do, not what an agent should call,
not whether the token is still good. The agent was handed a variable name and
left to work out an API from whatever it happened to know.

A server that publishes its own tools answers all of that itself. `tools/list`
is the documentation, the schema and the capability list in one call, and it is
current because it comes from the vendor at the moment of connecting.

Three conditions decide who is *shipped*, and all three are mechanical:

1. **It publishes its own tools.** `tools/list` at the moment of connecting,
   not a note in this repo about what the vendor's API can do.
2. **Those tools act on the operator's account.** A plugin is a thing a crew
   *has*, named in the prompt as something that reaches the real world.
3. **Its authorization server lets an application register itself.** The next
   section is why this one is not negotiable.

None of the three is a condition on a server the operator adds, and that is the
point of the split rather than a gap in it. The catalog is a claim Guaca makes
about a server: somebody checked, so a tile carries a name, a sentence and a
sign-in that works on one click. An added server carries no claim, and the row
says so. What it *gets* is identical, because none of the mechanism ever
depended on the claim.

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

## Why dynamic client registration decides who is shipped

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

**The issuer is recorded before the browser opens, and checked when it comes
back.** RFC 9207, and the attack it exists for is a mix-up: a code minted by an
authorization server the operator does not use, presented to the one they do. A
redirect naming a different issuer is refused before the code is spent — and
before `error`, `error_description` or `error_uri` are read, because on a mix-up
those are attacker-controlled text and showing them is how an operator is talked
into the next step. Compared byte for byte, with no case folding, port elision
or trailing-slash tidying: each of those makes two different issuers compare
equal, which is the whole of what is being checked.

## What a plugin asks for is the resource's list, not Guaca's

Guaca never invents a scope. A scope it invented is a scope the server refuses
in a browser window that cannot explain why, and the operator is left reading
`invalid_scope` on a vendor's error page.

Three places publish a list, and they are not the same list.

| Where | RFC | What its scopes mean |
|---|---|---|
| The `WWW-Authenticate` challenge, on the 401 | 6750 | What *this request* needed, right now |
| Protected resource, at the MCP server | 9728 | What to ask for to reach *this resource* |
| Authorization server, at the issuer | 8414 | Everything the issuer can grant, for every resource behind it and every client registered by any means |

They are tried in that order, which is the order of how much each one knows
about the request that was refused. The challenge wins because it is the server
answering the question that was actually asked; the MCP authorization spec makes
it the first thing a client should ask for, and asking for less than it named is
a second 401 on the same call. The resource's list is next, and the
authorization server's is the fallback for a resource that says nothing, which
is most of them — it is last because it is the only one of the three that is not
about this server at all.

Nothing is read out of a challenge that did not put a `scope` in one, which is
every vendor on the list today: their challenges name the metadata and nothing
else.

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

## A server the operator added

The catalog is six servers somebody checked. This is the seventh onward, and it
is the same feature with the checking left out.

An operator gives two things, which are the two things a catalog entry was
giving: a **name** and an **address**. After those two strings there is no
second code path. The same era probe, the same sign-in, the same `tools/list`,
the same `PluginAccess` per agent and per tool, the same refusals in the same
order, the same grant that reaches no prompt, transcript, event or sandbox.
`PluginKind::Custom` is a variant beside the other six and every consumer of a
plugin treats it as one.

### The name is the only name

It is the prefix its tools are called by, so `home_assistant` offers
`home_assistant__turn_on`. There is no display name beside it.

A second name would be a second string that can drift from the one an agent
types, and there is nowhere for that drift to surface except inside a turn that
cannot find a tool it was told it had. So what an operator types is normalized —
lowercased, everything that is not a letter or a digit becoming one underscore,
runs collapsing — and the result is what the panel draws, what the prompt says
and what the model calls. The webview does not predict it: it draws what came
back, because the rule lives in Rust and a copy of it in the front end is a
second place for it to be wrong.

Collapsing runs is also what makes a `__` impossible in a name, which matters
more than it looks: two underscores are what separate a plugin from its tool, so
a name carrying a pair would split a call in the wrong place and route it to a
plugin that does not exist. The name is capped at 32 characters for the other
half of the same reason — a provider refuses a function name past 64, and the
tool needs the rest.

`plugins_kind_unique` was one row per vendor per crew and is now also what stops
two added servers in one crew sharing a name. Two tool lists behind one prefix
would make which one a call landed on depend on row order.

### The address is stored on the row, and a vendor's is not

`plugins.endpoint` is empty for the six and set for the rest, and the asymmetry
is deliberate in both directions.

Where a vendor's server lives is a decision the *build* makes and re-makes on
every release. A stored copy would freeze it, and the next time a vendor moved,
every crew connected before the move would keep dialling the old host. That is
the failure migration 26 exists to clean up after, and it is not worth
reintroducing for a string the build already has.

An added server has nowhere else to keep it. That gives `PluginKind::from_row`
one clean rule: a catalog slug resolves whatever is beside it, a slug with an
address needs no build knowledge at all, and a slug with neither is a row this
build cannot dial — which is exactly what a newer build's plugin looks like
after a downgrade, and is skipped rather than raised, for the reason every
unreadable plugin row is skipped.

The address is canonicalized on the way in, because it is two things at once:
the URL a POST goes to and the RFC 8707 resource indicator the sign-in is
scoped to. The trailing slash goes; a fragment is refused rather than trimmed,
since a fragment is never sent to a server and a token issued for one is a token
nothing will present.

**HTTPS, or loopback.** Everything else carries a crew's grant, so plain HTTP is
refused here rather than in a packet capture. Loopback is the exception because
a server on this machine is the commonest thing an operator wrote themselves and
nothing on that connection leaves the machine. It is decided by parsing the host
and asking whether the address is loopback, not by matching three spellings:
`127.0.0.0/8` is all loopback, an IPv6 host is bracketed so the port cannot be
split off at the first colon, and `127.evil.com` is not a loopback address in
any reading.

### A pasted key is a grant with nothing to renew it

The catalog never needs this. A server somebody wrote very often has no
authorization server behind it at all, just a token minted by hand, and asking
one of those to discover a sign-in is a round trip whose only outcome is a 401
with nothing useful in it.

So `Credential::Key` skips discovery and sends the key as a bearer. It is stored
in `access_token`, spent by the same code, hidden from the model by the same
absence of a field, and dropped on disconnect by the same delete. What it has
none of is the machinery for renewing itself — no refresh token, no stated
expiry, no client, no token endpoint — and each of those absences is the truth
about a key somebody minted by hand. A server that stops accepting it says so
once, and the operator pastes another. The one retry the OAuth path gets is not
attempted, because a refresh against an empty token endpoint is a request to
nowhere.

Leaving the key out is not a lesser option: the server is then asked what it
wants exactly as a vendor's is, and a server that publishes protected-resource
metadata signs in through the full flow.

### Its name only resolves against a crew that has it

This is the one place a custom server differs from a catalog one, and it is at
parse time rather than at call time.

`neon__run_sql` parses whether or not anybody connected Neon, which is what
makes "Neon is not connected. Ask the operator" reachable — a refusal that names
the way forward, instead of "unknown tool", which names nothing. A name this
build has never heard of cannot do that: there is nowhere for it to come from
but the crew's own rows, which is also the only place its address exists.

So `split_plugin_tool` tries the catalog first and this agent's own connected
plugins second. A prefix that is neither is not a plugin call at all, which is
what keeps a model composing `use_screen__click` from being reported as a server
nobody has ever heard of.

### It speaks the transport that was replaced, and a vendor does not

MCP has had two HTTP transports. Streamable HTTP is one POST per request with
the reply in its response, and it is what everything in this file assumes. The
one it replaced — revision `2024-11-05`, still commonly called the SSE transport
— is a GET that stays open, an `endpoint` event naming a second URL, every
request POSTed to that URL, and every reply arriving back down the stream. It
has been deprecated since `2025-03-26`.

Guaca speaks it, for an added server only. The asymmetry is the catalog's own
argument rather than an inconsistency in it: a vendor Guaca vouches for is a
vendor Guaca can hold to the current transport, and refusing one of the six over
it would be a message somebody at that vendor would read. A box in an operator's
own network is not a vendor. Refusing that is not a migration incentive, it is a
plugin that does not work, on a server the operator can see working in a browser,
with nothing on screen saying why. `Dial::legacy_transport` is the switch and
`plugins::dial` is the only thing that sets it.

Which transport a server wants is probed rather than configured, exactly as the
era is, and out of the same failure: an MCP endpoint on the older transport
answers a POST with `405` or `404`, which is a refusal with no JSON-RPC in it,
which is already the shape that says "this is not a modern server". So the rule
grows one clause: for an added server, that shape is a reason to try a GET
before concluding anything. A stream that opens identifies the transport; one
that does not leaves the original refusal exactly as it was.

**Whose refusal the operator sees turns on how far the second attempt got.** A
GET answered with something that is not an event stream, or not answered at all,
says nothing the first attempt did not, so the first one stands. Anything past
that came off the server's own stream and is the more specific of the two — a
401 that says to sign in, a revision nothing shares, a session redirected onto
another host. Reporting the `405` there sends an operator to look at a transport
that was working.

**A session on that transport belongs to a stream, and the stream is per
request.** `initialize` establishes a session that is scoped to the event stream
it was sent on, so a session kept between turns is a socket per plugin per crew,
held open through sleep and reconnect, for a handshake that costs one round trip.
Every exchange opens a stream, shakes hands on it, sends one request and drops
it. That is the same "a session per call" rule this file argues for on the
current transport, arrived at from the other side.

**The message endpoint has to be on the same origin the stream was.** It is a
redirect invented by the far end after the connection was made, and following it
puts a crew's credential and every argument of every tool call on a host the
operator never named. Relative is the common form, absolute is legal, and an
absolute one naming anywhere else is refused.

Two older revisions came onto the list of ones this build accepts along with it,
`2025-03-26` and `2024-11-05`, because a server of that vintage negotiates down
to one of them and would otherwise be refused for sharing no revision. The cost
is nothing: `tools/list` and `tools/call` are the only methods this client has
ever sent and both are unchanged across all five.

### Headers, which are how the request arrives rather than who is asking

A server somebody runs can want a third thing, after an address and a token, and
it is the one the catalog never needs because nothing can discover it: an
`X-API-Key` because that is where its framework looks, a pair of
`Cf-Access-Client-*` because it sits behind Cloudflare Access, a tenant id
because one deployment serves several.

These are not a third credential and they are not a `Credential` at all. They
describe how a request *reaches* the server, so they go on every request
whichever of the other things paid for it — the unauthenticated probe, the
handshake, the tool list, every call, and the GET that opens an event stream. A
client that put them only on the requests it thinks of as "the call" never opens
the stream at all.

Composing rather than choosing is what makes the awkward case work rather than
needing a case of its own. A server behind Access that also signs in: the
headers get the request past the gate, the 401 from behind the gate starts the
browser dance exactly as it would anywhere, and the grant that comes back is
spent with the headers still on every request. Nothing about that path is
written down twice.

**A sign-in reaches more hosts than the server, so the headers stop at the
resource's origin.** `oauth::Gate` is that rule and it is load-bearing in both
directions. Without it the composition fails before it starts: the gate refuses
the RFC 9728 metadata document a sign-in has to read first, and the operator
gets a `403` out of discovery with nothing to act on. Sending them to every host
in the dance instead would hand the operator's gate credential to a vendor's
authorization server, which is a credential leak dressed up as a convenience.

The origin rule covers both shapes without a special case. A self-hosted server
that is its own issuer has registration and the token endpoint behind the same
gate, and they get the headers. A resource whose authorization server is
somebody else's host does not, and nothing of the operator's crosses over. The
refresh is included, and it is the one that would otherwise fail a day after
everything looked fine: the token endpoint is on that host too.

**Every value is a secret and is treated as one.** Stored in a column read only
by the code that puts it on the wire, never in a prompt, a transcript, an event
or a sandbox. What crosses IPC on the way back is the *names*: a panel has to be
able to say that `x-api-key` is on the request, because that is the question an
operator debugging their own server is asking, and it must not be a place to
read back what the key is worth. Same boundary `connector_env` draws, same
reason.

**A header Guaca builds itself is refused rather than overwritten.** Anything
starting `mcp-`, and `accept`, `content-type`, `content-length`, `host`,
`connection`. A modern server compares the `mcp-*` headers against the request
body and refuses a request where they disagree, so one written here would fail
every call with an error the operator reads as the server rejecting their work.
Names are lowercased on the way in, because two spellings of one field name are
a duplicate rather than two headers and sending both leaves which one wins to
insertion order. A value carrying a line break is refused: that is header
injection, and the way it reaches a box like this is a key pasted with the
newline still attached.

**`authorization` is allowed, and a key beside it is refused.** It is the only
way to send `Basic`, or a scheme a vendor invented, so refusing it would make
those unreachable. But the key box sends `Authorization: Bearer` and there is
one such header on a request: both given, one would silently overwrite the
other. `commands::presented` refuses the pair in a sentence that says they are
one slot rather than two.

### Changing the address is a reconnection

`readdress_plugin`, not an edit, and for the reason `set_plugin_connection` is
its own command: a server at a new address is not the one that published the old
tool list, so the list has to be re-read. What survives is what survives any
reconnection — the row's id, who may spend it, which of its tools are whose —
because the operator is fixing an address, not deciding what the crew may do. A
local server changing port should not cost anybody their permissions.

Headers go the other way from the key on this path, and the difference is what
can be shown. Sending a set replaces it whole; sending none leaves the stored
one alone. That is the rule a group's API key already has, for the same reason:
a value that cannot be read back is one the panel cannot re-send, so "absent"
has to mean keep or every reconnection would quietly drop it. Removing them is
an empty set, which is a thing the operator did rather than a thing they forgot.
The key keeps its own older rule — absent means "ask the server" — because a
server that stopped needing one would otherwise be unreachable from this panel.

### Testing it is the whole path, minus the browser

An address, a key and a set of headers are four ways to be wrong and one button.
Before this, finding out which meant pressing Add: a failure replaced nothing
but told the operator only whichever sentence the first failing layer produced,
and a success on the wrong address connected a crew to it.

`probe_server` runs the real path — the same probe, both transports, the
handshake, `tools/list` — with whatever has been typed and not yet saved, and
writes nothing. What comes back is not a sentence but the things underneath it
separately, because the failures are separate: which transport answered, which
revision was settled on, whether there was a handshake, what the server calls
itself, what it published, and how long it took. A `405` on the current
transport and a working event stream are the same sentence to a person and
opposite instructions.

It stops one step short of connecting in exactly one place: **a server that
wants a sign-in is reported as wanting one rather than sent to a browser.** The
question being asked is whether this is the right address, and answering it with
a consent screen is a question nobody asked — and a diagnostic that opens one is
a diagnostic nobody runs twice.

The two 401s are the reason the answer is four states rather than a boolean.
"Nothing was presented and it refused" and "something was presented and it
refused" are one status code and opposite problems: an operator who cannot tell
them apart re-pastes a key at a server that never wanted one, or goes hunting
for a sign-in that is really a typo.

`check_plugin` is the same question asked of a plugin the crew already has, and
it is a separate command because the credential is separate: it spends the grant
in the store, which is the only way to answer "is our sign-in still good". That
is the failure a crew notices as an agent reporting a tool it cannot call, and
the only other way to ask it is to connect again, which replaces the tool list
and opens a browser. A stale grant is renewed first, because the next real call
would renew it too and a check that skipped it would report a working plugin as
broken. It is offered on every connected plugin rather than only the added ones:
the question is the same for a vendor.

### What the operator is told

That nobody checked it, in the row, and that it gets everything a shipped server
gets. That is the whole of the difference and it belongs on screen rather than
in this file: an added server is offered to agents, spends the operator's
account, and acts on the real world exactly as Stripe does.

The model is told where it is. A model knows what Neon is and has never heard of
`home_assistant`, so the prompt line for an added server names its host. For the
six that would be noise — the name says it, and the address is the same on every
install.

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

## Two protocol eras, and how a server's is decided

Revision `2026-07-28` deleted the handshake.

Before it, a session was established with `initialize`, the agreed protocol
version came back in the reply, and the server could mint a session id that
every later request had to carry. After it there is no session at all: every
POST stands alone and declares its own version, in `_meta` in the body and in
the `MCP-Protocol-Version` header beside it, along with `Mcp-Method` and — on a
`tools/call` — `Mcp-Name`.

Both are in the field and will be for years. Every vendor on the list shakes
hands today; a server written this year may not. A client that speaks one era
cannot reach half of them, so this one speaks both.

**Which era a server is, is decided by asking it something only a modern server
answers.** `server/discover` is mandatory for a modern server, so its answer is
the probe, and the refusals carry as much information as the answer:

- A result: modern, at the revision that was asked for.
- `UnsupportedProtocolVersionError` (`-32022`): modern, at revisions it names.
  The spec says to retry with a mutually supported one rather than fall back,
  because a modern error can only come from a modern server.
- `HeaderMismatch` (`-32020`): also modern, and a bug here. Raised rather than
  fallen back from, since falling back would hide it behind a handshake that
  happens to work.
- A 401, or a transport failure: neither says anything about the era. The first
  is the sign-in and the second is a server that is not answering.
- Anything else — a `400`, a `404`, an unknown-method JSON-RPC error, a body
  that is not MCP: a server that has never heard of `server/discover`.

That last case is why the rule is written on the *body* rather than on the
status code. A real legacy server answers an unknown method with `200` and a
JSON-RPC error, which is neither a `400` nor a modern error shape, and a
status-only reading would decide it was something else entirely.

**A version list that names only handshake-era revisions is a fallback, not a
retry.** A dual-era server asked for something it lacks may answer `-32022`
listing `2025-11-25`. That is not an invitation to ask again in the modern
shape — it is the server saying to shake hands. Retrying modernly there is
refused all over again and the plugin never connects. `mcp::modern` is the
comparison that decides, and it is a comparison rather than a second list
because a revision is a date and a date in `YYYY-MM-DD` sorts as a string
exactly as it sorts as a date.

**The era is remembered per endpoint, for the life of the process.** That is not
the session cache the next section argues against. An era is a property of the
deployed server rather than of a grant, it cannot expire, and a remembered one
that turns out to be wrong fails once and re-probes — which is what a server
upgraded underneath a running Guaca looks like. Without it, every plugin call on
a legacy server would pay for a probe whose answer is already known, which is an
internet round trip in front of every tool call in the crew.

**The negotiated version is what later requests declare, not the one that was
asked for.** A legacy server that only knows `2025-06-18` answers `initialize`
with that, and every request after it carries it. Sending the constant instead
is a header that contradicts the handshake.

### `x-mcp-header`

A modern server may ask for some of a call's arguments to be mirrored into HTTP
headers, so an intermediary can route on them without parsing a body. It is
optional for a server and mandatory for a client, and a server validates the
header against the body: a client that skips it has its call refused as a header
mismatch, on that tool, forever.

So the schema comes back from `Store::plugin_reach` beside the grant — not for
the model, which has had it since the turn was built, but for the transport.
Annotations are read at `tools/list` too, and a tool whose annotation cannot be
honored is **dropped rather than offered**: one reachable only through `items`,
`oneOf`, or a `$ref` has no single value in a call to mirror, so a call to it
would be refused every time for a reason no model can act on. One tool goes,
not the server.

Only on a modern session. On a legacy one the annotation means nothing, and
dropping the tool would take a working capability away over a field nobody
reads.

### What is deliberately not implemented

The 2024-11-05 HTTP+SSE transport, deprecated since `2025-03-26` and scheduled
for removal: a client that falls back to it keeps servers alive that should be
migrating. Resources, prompts, `subscriptions/listen`, and the server-to-client
input requests a modern server can embed in a result: an agent here is offered
tools and nothing else, and a half-implemented capability is worse than an
absent one.

Client ID Metadata Documents are the other one, and the only one with a date on
it. The `2026-07-28` authorization spec makes them a **SHOULD** and demotes
dynamic client registration to a **MAY**, retained for servers that do not
support CIMD. Guaca registers dynamically, which works at every vendor on the
list today and is what *Why dynamic client registration decides who is shipped*
argues for. CIMD would mean a document hosted at a URL Guaca controls, naming
redirect URIs that are loopback ports chosen at bind time — so it is a change to
`guaca.bot` as much as to this repo, and it is the next thing to do here rather
than something ruled out.

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

Every tool call opens a fresh session. On a legacy server that is `initialize`
and then the call; on a modern one it is the call, because there is no session
to open. Keeping a legacy session would save a round trip and is not done,
because a cached session is a second thing that can go stale — the server can
expire it, the token under it can be refreshed, the crew can disconnect the
plugin — and each of those surfaces as a tool call that fails for no reason a
model can act on.

The handshake is tens of milliseconds against a call that is already crossing the
internet to run somebody's SQL. This is the place to come back to if a
measurement ever says otherwise.

The remembered era is not an exception to this. It is not a session, it holds
nothing that can expire, and it is what keeps the era probe from being a second
round trip in front of the handshake it replaced.

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

**No revocation at the vendor on disconnect.** The grant is dropped locally. Not
every authorization server publishes a revocation endpoint, and an operator who
wants the authorization itself withdrawn has to do that where they granted it.

## Testing

Three layers, and each catches something the others cannot.

- **Unit**, in `mcp.rs`, `oauth.rs`, `domain/plugin.rs` and `llm/tools.rs`: the
  two body shapes a streamable-HTTP server may answer with, which revisions are
  modern, header value encoding, `x-mcp-header` reachability, the RFC 7636 PKCE
  example, the RFC 8414 well-known path insertion, scope priority across the
  three lists, the name and address an added server is allowed, and the
  tool-name split.
- **Scripted**, in `tests/plugins.rs`: the real `oauth`, `mcp`, `plugins` and
  store code against a server that publishes all four metadata documents and
  answers MCP as an event stream. Includes a full runtime turn, so the tool
  definitions, the dispatch and the grant being spent are proved to meet, and
  one turn per axis where a model calls something it was not offered, plus one
  where two agents on one plugin are given different halves of it. The scripted
  server speaks either era, and the modern one validates its own headers against
  its own body the way the spec requires of it — so a client that skipped a
  mirrored header, sent the wrong version, or fell back when it should not have
  fails here rather than at a vendor.
- **Live**, `./scripts/plugins.sh`: whether the five vendors still publish what
  this build expects. It runs `oauth::discover` — the same call a sign-in makes
  — rather than rebuilding the metadata URLs beside it, because a test with its
  own copy of RFC 8414 passes on a server this build cannot reach. Reaches the
  internet, authorizes nothing, spends nothing.

The live one is the one that matters over time. Everything offline is a stub
agreeing with what this app believes MCP authorization is, and the failure worth
catching is that belief going stale.
