# Plugins, MCP and OAuth

Signing a crew in to a server, who may spend that sign-in, which tools they may
call, and the two protocol eras underneath all of it. `docs/PLUGINS.md`, then
`mcp.rs`, `oauth.rs`, `plugins.rs` and `domain/plugin.rs`. The first entry is
the guaca.bot account, which signs in through the same file: `docs/ACCOUNT.md`
and `account.rs`.

- **The issuer a redirect is checked against is the one the service published,
  not the origin the document was fetched from.** Those are the same string only
  for an authorization server at the root of an origin, and `guaca.bot` mounts
  its own at `/api/auth`. Substituting the origin opened a browser, reached the
  consent screen, took a code and refused it on the way back, every time; the
  offline suite agreed with the substitution, because its stub published its own
  origin as the issuer and sent no `iss` at all. An absent `issuer` still means
  the origin, because that is what the root well-known address implies, and both
  are checked to be on the configured origin: an issuer nobody checked is one a
  metadata document could point at a third party whose codes Guaca would then
  accept. `ServerMetadata::issuer`, then *The issuer is read, never assumed* in
  `docs/ACCOUNT.md`.
- **What a plugin's sign-in asks for is the resource's list, not its
  authorization server's.** They are two documents and two lists: RFC 9728 names
  the scopes for *that resource*, RFC 8414 names everything the server can issue
  behind it. AgentMail's MCP server wants three and the Clerk instance behind it
  lists seven, four of which it refuses a registered client, as an
  `invalid_scope` in the operator's browser. So the resource's list wins, the
  server's is the fallback, `*` is dropped and `offline_access` is added when the
  server names it: without a refresh token a plugin asks to be signed in again
  every hour. Where neither publishes a list, Guaca sends no `scope` at all and
  the server applies its own default, which is what Cloudflare's consent screen
  is choosing between. `oauth::requested_scope`, then `docs/PLUGINS.md`.
- **Cloudflare is `mcp.cloudflare.com`, not one of the fifteen
  `*.mcp.cloudflare.com`.** Each subdomain is a single product area, so one of
  them is a crew that can make a Worker and cannot read a DNS record, and
  several of them is a hundred tool definitions on every turn. The apex host is
  the whole API behind `search` and `execute`: the model writes JavaScript
  against the OpenAPI document and Cloudflare runs it, so 2,500 endpoints cost
  about a thousand tokens instead of a million.
- **A plugin's sign-in belongs to the group, and who may spend it does not.**
  `PluginAccess` is `Everyone` or a named list, and the empty list is why it is
  not one list with a sentinel: everyone covers agents nobody has hired yet, and
  a list that meant everyone when it was empty would hand a plugin back to the
  crew at the moment the operator unticked the last name. Filtering the tool
  definitions is not the enforcement either. A model names tools it was never
  offered, so `Store::plugin_reach` asks the same question again on the call
  path, from the same SQL fragment, and its two refusals are different
  sentences: "nobody connected this" is the operator's to fix, "connected, but
  not for you" is a peer's to do.
- **A tool takes the same answer a plugin does, and the two compose.** Who may
  spend the sign-in is about the account; who may call `gmail_send` is about the
  capability, and an agent has to pass both. That is what lets one crew put the
  agent that triages an inbox beside the agent that answers it, on one sign-in,
  with different halves of it each. A single answer per plugin cannot say that,
  and neither could the crew-wide tool switch this replaced.
- **A tool nobody has narrowed is on, and inside a narrowed one only the named
  agents are.** The two defaults point opposite ways on purpose. An unseen
  *tool* is one the vendor shipped after the operator last looked, and an
  allow-list over tools would switch it off with nothing on screen saying a
  decision had been taken. An unseen *agent* is one hired next week, and it must
  not inherit the capability the operator went out of their way to fence off.
  `PLUGIN_TOOL_REACHED_BY_AGENT` is both rules at once.
- **A tool switched off for the crew is `Chosen` with an empty list.** Not a
  third state and not a second table: it is the same empty list `PluginAccess`
  already argues for at the plugin level, and one click still gets there.
  Migration 31 rewrites every `plugin_denied_tools` row as exactly that.
- **The wider refusal is given before the narrower one.** More than one is true
  at once. A tool narrowed to nobody is off for everybody, so `ToolDenied` is
  said before `NotChosen`; being off a plugin covers every tool on it, so
  `NotChosen` is said before `ToolNotChosen`. Two of the four send an agent to a
  peer and two do not, and an agent told to ask around about a tool nobody has
  spends a turn proving it, as does the peer.
- **The tool half of that rule is Rust and the agent half is SQL, and that is
  not an oversight.** `PLUGIN_REACHED_BY_AGENT` is one fragment pasted into two
  queries; the tools cannot be, because the tool list is a JSON column and
  there is nothing for SQL to filter without taking it apart inside the
  database. `Store::plugin_tools` partitions it in Rust, `Store::plugin_reach`
  asks in SQL, both compare the server's own unprefixed name, and store tests
  drive both refusals through both.
- **A name on a tool the plugin itself does not reach grants nothing, and is
  kept anyway.** The two controls are set in either order, so ticking an agent
  on a tool before widening the plugin to them is a state to pass through, not
  one to refuse. `plugin_reach` takes the intersection and the panel says which
  name is not counting yet: a permission panel naming an agent that would be
  refused is the one thing it must not do.
- **A tool an agent cannot call is named in its prompt anyway, under one of two
  headings.** The name only: no description, no schema, and never a definition.
  An agent that is simply not shown `create_refund` answers "we cannot do
  refunds" to the one person who could switch it back on. Which heading decides
  where the turn goes next, so `withheld` and `elsewhere` are two lists and two
  sentences: nobody has the first, and a peer has the second.
- **The roster names a peer per tool, not just per plugin.** An agent that has
  Stripe and cannot refund is exactly the case `reaches` exists for, and the
  plugin-level line is silent about it because this agent has Stripe. Only what
  this agent lacks and that peer can actually call: naming a peer who would be
  refused in turn is the failure the roster exists to prevent, not one to
  commit.
- **A plugin's tool list is read once and kept.** `tools/list` on every turn is
  a network round trip in front of every model call, paid by every agent in the
  crew, to re-learn something that changes when a vendor ships rather than when
  an agent thinks. The stored list is what the turn is built from; connecting
  again is what refreshes it.
- **A plugin tool a provider would refuse is dropped, not renamed.** Providers
  validate a function name against `[A-Za-z0-9_-]{1,64}`. Renaming to fit needs
  a mapping back at call time, and a mapping nothing can see is how a call lands
  on the wrong tool.
- **`plugins::connect` opens the server with no token first.** It is the only
  honest way to find out whether the server wants one, and its refusal is where
  the address of the sign-in comes from: the `WWW-Authenticate` challenge names
  the vendor's own protected-resource metadata, which beats any well-known path
  Guaca guessed at. Every server on the list asks today; a public one connects
  with `signed_in` false rather than claiming a sign-in that never happened. A
  pasted key is the one credential that skips the question, because a server
  that takes one has no authorization server to discover.
- **A server the operator added has one name, and it is the tool prefix.** Not a
  display name beside a slug: a second name drifts from the one an agent types,
  and the only place that surfaces is a turn that cannot find a tool it was told
  it had. What was typed is normalized in Rust and the webview draws what came
  back rather than predicting it. Collapsing runs of punctuation to one
  underscore is also what makes a `__` impossible in a name, which is what
  `split_plugin_tool` splits on.
- **Its address is on the row, and a catalog kind's is not.** Where a vendor's
  server lives is a decision the build makes and re-makes every release, so a
  stored copy would keep a crew dialling the old host after the vendor moved:
  that is what migration 26 exists to clean up after. A row with neither a
  catalog slug nor an address is one nothing can dial, which is a newer build's
  plugin after a downgrade, and is skipped. `PluginKind::from_row`.
- **A header the operator wrote is not a credential, and that is why it
  composes.** It describes how a request *reaches* the server rather than who is
  asking, so it goes on every one — the unauthenticated probe, the handshake,
  the tool list, every call, and the GET that opens an event stream — whichever
  of the other things paid for it. That is what makes a server behind Cloudflare
  Access that also signs in work without a case of its own: the headers get past
  the gate, and the 401 behind it starts the browser dance unchanged. A client
  that put them only on what it thinks of as "the call" never opens the stream.
- **A sign-in reaches more hosts than the server, so the operator's headers stop
  at the resource's origin.** `oauth::Gate`. Both directions are load-bearing:
  without them the gate refuses the metadata document a sign-in reads first and
  discovery dies on a `403`, and with them everywhere the operator's gate
  credential reaches a vendor's authorization server. The rule covers a
  self-hosted server that is its own issuer — registration, token and refresh
  all behind the same gate — without a case of its own. The refresh is the one
  that would otherwise fail a day after everything looked fine.
- **A header this client builds itself is refused rather than overwritten, and
  `authorization` is not one of them.** Anything `mcp-*` disagreeing with the
  body is refused by a modern server with an error that reads as the server
  rejecting the operator's work. `authorization` is allowed because it is the
  only way to send `Basic` or a scheme a vendor invented — and a key beside it
  is refused, because the key box writes the same header and one would silently
  win. `Headers::parse` does the first, `commands::presented` does the second.
- **Header *names* cross IPC and values never do.** A panel has to be able to
  say `x-api-key` is on the request, because that is the question an operator
  debugging their own server is asking, and it must not be a place to read back
  what the key is worth. Same boundary `connector_env` draws.
- **Sending headers to `readdress_plugin` replaces the set and sending none
  keeps it.** The rule a group's API key has, for its reason: a value that
  cannot be read back is one the panel cannot re-send, so absent has to mean
  keep. An empty list removes them, which is a thing the operator did. The key
  on that command is the other way round and stays that way: it is this
  command's own older rule, and a server that stopped needing a key would
  otherwise be unreachable from the panel.
- **The older transport is offered to a server the operator added and to no
  vendor.** A vendor Guaca vouches for is one it can hold to streamable HTTP,
  and refusing one of the six over it is a message somebody at that vendor
  reads. A box in an operator's own network is not a vendor: refusing it is not
  a migration incentive, it is a plugin that does not work on a server they can
  see working in a browser. `Dial::legacy_transport`, set only in
  `plugins::dial`.
- **Whose refusal the operator sees after a fallback turns on how far the
  second attempt got.** A GET that was not answered with a stream, or not
  answered at all, says nothing the POST did not, so the POST's stands.
  Anything past that came off the server's own stream and is the more specific
  of the two. Reporting the `405` there sends an operator to look at a
  transport that was working, which is why *not an event stream* is its own
  error variant rather than a sentence inside `Malformed`.
- **A message endpoint on another origin is refused rather than followed.** It
  is a redirect invented by the far end after the connection was made, and
  following it puts a crew's credential and every tool argument on a host the
  operator never named.
- **Testing reports a server that wants a sign-in; it does not run one.** That
  is the single step `probe_server` stops short of `add_plugin` at. The question
  is whether this is the right address, and answering it with a consent screen
  is a question nobody asked — and a diagnostic that opens one is a diagnostic
  nobody runs twice. It is also why "nothing presented and refused" and
  "something presented and refused" are two states: one status code, opposite
  problems, and told apart wrongly an operator re-pastes a key at a server that
  never wanted one.
- **A name only resolves against a crew that has it, and a catalog name always
  resolves.** `neon__run_sql` parses whether or not Neon is connected, which is
  what makes "Neon is not connected, ask the operator" reachable instead of
  "unknown tool". A name this build has never heard of cannot do that, because
  the crew's rows are the only place it or its address could come from — which
  is also what keeps a model composing `use_screen__click` from being reported
  as a plugin nobody has.
- **This client speaks two protocol eras, and the probe is what decides.**
  `2026-07-28` deleted the handshake. `server/discover` is mandatory for a
  modern server, so its answer — or a refusal in one of the two shapes only a
  modern server produces — identifies the era, and anything else is a server
  that wants `initialize`. The rule is written on the *body*, not the status
  code: a real legacy server answers an unknown method with `200` and a
  JSON-RPC error. A `-32022` naming only handshake-era revisions is a fallback
  rather than a retry, because that is a dual-era server saying to shake hands
  in the only vocabulary a modern request gave it.
- **The era is remembered per endpoint and a session is not.** An era belongs to
  the deployed server rather than to a grant, cannot expire, and re-probes on
  the one failure it causes. Without it every plugin call on a legacy server
  pays for a probe whose answer is known, in front of the handshake it replaced.
- **What later requests declare is what was negotiated, not what was asked
  for.** A legacy server that only knows `2025-06-18` says so in its handshake
  reply, and a header carrying the constant instead contradicts it.
- **A tool whose `x-mcp-header` cannot be honored is dropped, not offered.** A
  modern server validates the mirrored header against the body, so a call built
  without it is refused every time for a reason no model can act on. An
  annotation reachable only through `items` or a `oneOf` has no single value in
  a call to mirror, so that tool goes and the rest of the server stays. Only on
  a modern session: on a legacy one the field means nothing and dropping the
  tool would take a working capability away over something nobody reads.
- **The loopback port is bound before the client is registered.** That ordering
  is the whole reason a redirect is acceptable here at all, and it is the
  difference between this flow and the one `subscription.rs` argues against:
  `docs/PLUGINS.md`.
