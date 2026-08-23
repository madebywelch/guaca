# Guaca

Local desktop app. You talk to LLM agents; the agents talk to each other. Tauri
v2, React + TypeScript front, Rust back.

This file is the map and the routing table. What is left in it applies wherever
you are working. The reasoning lives in `docs/`, one file per subsystem, and the
table below says which one to open before changing something.

## Where things are

```
src/                  React + TypeScript. A view over the runtime, nothing more.
  lib/transcript.ts   What a channel shows, and what it collapses. Read first.
  lib/rail.ts         What order the rail draws agents in, and where a drop lands.
  lib/orb.ts          How a crew stands inside its circle, and when it counts.
  lib/search.ts       One ranking over hits from SQLite and from the store.
  lib/trail.ts        A turn's own tool calls: what folds into one chip.
  lib/figure.ts       A fenced block the transcript draws instead of printing.
  lib/chart.ts        A chart spec, and where every mark goes. No DOM.
  lib/palette.ts      Eight hues in one order, and why that order and not another.
  lib/diff.ts         Two versions of a page, as the lines between them.
  lib/reasoning.ts    A turn's own thinking: how much is held, what is drawn.
  lib/cafeteria.ts    Preset agents, waiting to be hired. Content, not runtime.
  lib/roles.ts        What an agent is for, in OpenRouter's twelve words.
  lib/plugins.ts      A plugin's mark and colour. Everything else is Rust's.
  lib/ipc.ts          Every call into Rust.
  lib/prefs.ts        What the operator sets and the runtime never reads.
  lib/appearance.ts   Scale and surface, as one write to the root element.
  lib/follow.ts       Whether a transcript may move under the operator.
  lib/notify.ts       When an interruption is warranted. Mostly when it is not.
  lib/announce.ts     What that interruption would say. One event in, one line out.
  lib/keybinds.ts     Every key the app answers to, in one list.
  lib/limits.ts       The five bounds a conversation runs inside, in words.
  components/         One file per surface.
src-tauri/src/
  domain/             AgentCard, Envelope, Routine, Connector, Signin, Approval,
                      Search, ids. No I/O.
    group.rs          A crew's wall, and the settings its agents run on.
    plugin.rs         The servers a crew can sign in to, what it got, and
                      which of its agents may spend it.
  runtime/
    guard.rs          The loop guard. Read this one first.
    mod.rs            Agent actors and the message bus.
    prompt.rs         Prompt assembly, including the trust boundary.
    events.rs         Events pushed to the UI.
  llm/                OpenAI-compatible client, SSE decoding, tool definitions.
    codex.rs          The other protocol: where a ChatGPT subscription is spent.
    catalogue.rs      Which models OpenRouter sees doing which kind of work.
  subscription.rs     Signing in to that subscription. A credential, not a wire.
  account.rs          The optional Guaca account. Nothing else depends on it.
  mcp.rs              The client end of MCP. Three methods, one POST each.
  oauth.rs            Signing a crew in to a plugin's server. PKCE, no client id.
  plugins.rs          Where those two meet the store, and a turn spends a grant.
  db/                 SQLite. Plain SQL, numbered migrations.
  e2b.rs              Computers: the machines agents look at and point at.
  proxy.rs            Loopback viewer for those machines.
  artifact.rs         The other loopback origin: where a page an agent wrote
                      is allowed to run, and everything it may not reach.
  sessions.py         Reports what a machine's Chrome is signed in to.
  kernel.rs           Browsers: a hosted Chrome, which is where the web belongs.
  cdp.rs              The DevTools protocol. Asks a page instead of looking.
  workspace.rs        Per-agent memory: one markdown file the agent rewrites.
  files.rs            Attachments, addressed by the SHA-256 of their contents.
  eval.rs             Reads a run and says whether it communicated sensibly.
  trajectory.rs       Reads a run's events and says whether the machinery did.
  config.rs           Operator settings, and the API key the webview never sees.
  commands.rs         The entire IPC surface.
  menubar.rs          What the menu bar says. No Tauri, no menu, no drawing.
  tray.rs             Drawing that, and turning a click back into a decision.
  app.rs              Where Tauri is wired up. It and `tray.rs` are the only
                      two files that know Tauri exists.
```

The agent runtime lives in Rust, not the webview. Each agent is a `tokio` task
with its own inbox, so sending is enqueue-and-return and N agents genuinely run
concurrently. It also means your API key never crosses into the webview. If you
find yourself adding agent logic to `src/`, you are in the wrong half of the
repo: the frontend renders state and forwards intent.

## Read before you change

| Changing | Read |
|---|---|
| Messaging, replies, cascades, hop limits, the guard | `runtime/guard.rs`, then *Cascades terminate because of one asymmetry* and *The five limits* in `docs/ARCHITECTURE.md` |
| What a turn is told it was asked for: `expects_reply`, `intent`, `ReplyMode` | *Cascades terminate because of one asymmetry*, and `runtime/prompt.rs`, which has to agree with it |
| Streaming, retries, the budget, when a run settles | *A failed model call is retried*, *A thought is shown and never kept*, *The budget counts model calls* |
| What a turn shows of itself while it runs: the thinking, the calls, the line above the composer | *A thought is shown and never kept* and *A turn's own work is watched while it happens*, then `src/lib/reasoning.ts` |
| How a turn is paid for: providers, the ChatGPT sign-in, the Responses API | *A subscription is a second provider, not a second endpoint*, then `llm/codex.rs` and `subscription.rs` |
| What a group decides for itself: provider, models, timeout, limits | *A group chooses its own provider*, *Nothing about who pays is inferred* and *A run is measured against the limits of the group it happens in*, then `domain/group.rs` |
| Stopping a conversation: what a stop marks, wakes, and must never release | *A stop marks the run and releases nothing*, then `Runtime::stop_run` |
| Permission prompts, parked turns, acting in the operator's name | *A protected action parks the turn that asked for it* |
| What an agent may do with a page it has just read | *A page that was read this turn cannot quietly press a button* |
| Screenshots, coordinates, what a screen action answers with | *A computer is looked at, never asked* in `docs/MACHINES.md` |
| Attachments, previews, drops, handing a document to the operator | *Files are references, and what a model gets depends on what they are* |
| SQLite, the pool, migrations | *Storage*, and the two comments in `Store::open` |
| Schedules, triggers, what a firing looks like | `docs/ROUTINES.md` |
| What an agent knows about its own schedule, and how it changes one | *An agent reads its own schedule before it decides to write another one* and *Changing a routine is `update`* in `docs/ROUTINES.md` |
| Whether an agent may have a computer or a browser at all | *A computer is given to one agent, not to the workspace* in `docs/MACHINES.md`, then `Runtime::surfaces_for` |
| Sandboxes, the desktop, the screen, sign-ins on it | `docs/MACHINES.md` |
| Hosted browsers, CDP, `browse`, live view, browser profiles | `docs/BROWSERS.md` |
| Which of the two a piece of work belongs on, and credentials | *Connectors* in `docs/PROTOCOL.md`, then both files above |
| Plugins: what is on the list, signing one in, calling its tools | `docs/PLUGINS.md`, then `oauth.rs` and `mcp.rs` |
| Which agents in a crew get a plugin | *Signing in is one decision, and handing it out is another* in `docs/PLUGINS.md`, then `domain/plugin.rs` and `Store::plugin_tools`, which has to agree with `Store::plugin_reach` |
| Which of a plugin's tools which agents may call | *And which of its tools, for which of them, which is a third decision* in `docs/PLUGINS.md`, then `Store::set_plugin_tool` and both readers of `plugin_tool_access` |
| The guaca.bot account: signing in, what it is for, why it is optional | `docs/ACCOUNT.md`, then `account.rs` |
| Channels, the rail, search: what the operator sees | `docs/WORKSPACE.md`, then `src/lib/transcript.ts` |
| Charts, tables, a page an agent wrote: what a reply can be drawn as | *A reply can be a figure* in `docs/WORKSPACE.md`, then `src/lib/figure.ts` and `src/lib/chart.ts` |
| A chart's colours, or how many series one may carry | *A chart's colours are the output of a check* in `docs/WORKSPACE.md`, then `src/lib/palette.ts` and the test beside it, which is the gate |
| Running a model's own HTML, or anything about that origin | *A page an agent wrote runs somewhere else* in `docs/WORKSPACE.md`, then `src-tauri/src/artifact.rs` |
| A turn's tool calls in a channel: what folds, what a chip says, what opens | *A turn's own work is chips* in `docs/WORKSPACE.md`, then `src/lib/trail.ts` |
| What an agent changed about its own memory, and where the version before it came from | *A memory rewrite opens as a diff* in `docs/WORKSPACE.md`, then `Workspace::write` and `src/lib/diff.ts` |
| Anything announced to a screen reader, or a live region | *A transcript is a log, and says one thing out loud* in `docs/WORKSPACE.md` |
| Scrolling a transcript, following the newest line, when the view may move | *A transcript follows the end for whoever is at the end, and nobody else* in `docs/WORKSPACE.md`, then `src/lib/follow.ts` |
| The menu bar: the glyph, the count, what the menu offers, closing the window | *The menu bar is Guaca with the window shut* in `docs/WORKSPACE.md`, then `src-tauri/src/menubar.rs` |
| The rail's order, dragging a row, groups as places you go inside | *The rail is arranged by hand*, *A drop is one call* and *A group is a place you can be inside* in `docs/WORKSPACE.md`, then `src/lib/rail.ts` and `src/lib/orb.ts` |
| Deleting a group, deleting an agent, what goes with either | *Deleting a group deletes the crew, and the machines they were renting* in `docs/WORKSPACE.md`, then `retire_agent` in `src-tauri/src/commands.rs` |
| Preset agents, hiring a crew | *The cafeteria is a copy machine* in `docs/WORKSPACE.md`, then `src/lib/cafeteria.ts` |
| Settings, the surface, the scale, what may interrupt the operator | *Settings is nine places*, *The reading column has two surfaces* and *An interruption has to earn it* in `docs/WORKSPACE.md` |
| The group editor: what a crew overrides and what it inherits | *A group's settings are the app's, with the crew's answer on top* in `docs/WORKSPACE.md`, then `src/components/GroupEditor.tsx` |
| What model an agent is offered, and how its job is guessed at | *The model field suggests three, and is still a text box* in `docs/WORKSPACE.md`, then `src/lib/roles.ts` and `llm/catalogue.rs`, whose twelve use cases have to agree |
| A prompt, or anything that changes how much a crew talks | *Three test suites, asking different questions*, then run the live evals |

Unqualified section names are headings in `docs/ARCHITECTURE.md`.

## True everywhere

**Anything crossing IPC is camelCase.** `rename_all` on a tagged enum renames
variants, not fields; you also need `rename_all_fields`. `ipc.contract.test.ts`
compares the Rust and TypeScript command lists directly, so a rename that only
lands on one side fails the build rather than at runtime.

**Migrations are forward-only and numbered.** One has already run against a real
database by the time you think of an improvement, and editing it leaves that
database at the same `user_version` with a different schema. Add another. They
run with foreign key enforcement off, which is what SQLite's own table-rebuild
procedure wants and what a migration cannot arrange for itself: the pragma is a
no-op inside a transaction. See `migrations::run`.

**A subscription is a second provider, not a second endpoint.** An operator pays
for a turn with a pasted key or with a ChatGPT sign-in, and the two share almost
nothing: different host, different wire protocol (Responses, not chat
completions), different auth header, models that are not the operator's to
choose, and no price on the answer. The two meet in exactly one function,
`LlmClient::stream_chat`, and `llm/codex.rs` translates. Anything above that line
sees one shape of request. Keep it that way: a provider branch in the runtime, the
prompt or the guard is the wrong half of the repo.

**An account is optional, and an install that never signs in never contacts
it.** `guaca.bot` holds one thing Guaca cannot: an OAuth client, for the
services that will only issue programmatic access to a registered application.
Signing in is authorization code with PKCE on a loopback port bound before the
redirect is named, which is `oauth.rs`'s argument pointed at one known server;
the device grant that was there first is gone rather than kept beside it,
because two doors to one account means the weaker one decides what the account
is worth. `docs/ACCOUNT.md`.

One thing does depend on it, and only one: the Google plugin, whose server is
that account. `Runtime::account_token` is read on a turn that calls one of its
tools and nowhere else, so a machine with no account is a machine where that
plugin refuses to connect and every other part of the app is unchanged. Keep it
that way: the account is a credential for one plugin, not a thing the runtime,
the prompt or the guard may consult.

**A Claude subscription cannot pay for a turn, and this is not an oversight.**
Anthropic restricts consumer OAuth tokens to Claude Code and Claude.ai, enforced
server-side since January 2026 and explicit in its terms since February 2026. The
flow would fail at the server and put the operator's account at risk, so it is
not implemented. Claude models arrive through an API key or OpenRouter, which is
still the default. Dates and sources: `docs/PROTOCOL.md`.

**A secret never reaches a model.** A credential's value and a cookie's value do
not enter a prompt, a transcript, an event or the webview, and there is no field
on the types that cross those boundaries for one to arrive in. Keep it that way:
`docs/MACHINES.md`.

**A computer and a browser are two places, not two views of one.** A computer is
an E2B machine with a screen, worked by looking and pointing (`use_screen`). A
browser is a hosted Chrome, worked by asking the page (`browse`). Different
providers, unrelated cookie jars, separate sign-ins, either configurable without
the other. Anything that describes one to a model has to disclaim the other, or
the model takes a screenshot to see what `browse` did.

## What looks like a simplification and is not

- **A group's settings are two blocks, and each is all-or-nothing.** A draft
  that mentions `inference` or `limits` replaces every override in that block,
  and one that leaves it out changes none of them. Per-field "absent means leave
  alone" would let a caller half-write a crew's settings, and would make a UI
  that renders what it read back clear a field by forgetting to mention it. The
  API key is the one exception and has the opposite rule (absent keeps the
  stored key), because it is the one setting that cannot be read back.
- **Every setting a group can hold is `None` when it is inherited, all the way
  down.** In the draft, in the column, and in the resolved overrides. An empty
  string is a field an operator blanked, which also means inherit, and it is
  normalised to `None` on the way in so the two can never disagree.
- **A thread between two agents is `pair_messages`.** A send is filed under the
  recipient and the answer under the sender, so a thread assembled from one
  channel's rows is missing messages nobody can account for.
- **`FileCard` hands the frame a `blob:` URL on purpose.** WebKit refuses a
  custom scheme in a frame and says nothing. A direct `guacfile:` `src` passes
  every test in this repo and draws an empty rectangle.
- **Every `guacfile:` answer carries `access-control-allow-origin`, refusals
  included.** A custom scheme is cross-origin to the page that asked, so without
  it a `fetch` rejects with `TypeError: Load failed` and never sees the status.
  An `img` is exempt and is the only preview that is not a `fetch`, so dropping
  the header shows as pictures drawing and every document, log and PDF failing,
  each with the one error message that cannot say which failure it was.
- **The full file view resets `overflow`, not just the height cap and the mask.**
  A clipping flex item has an automatic minimum size of zero, so a document left
  clipping shrinks to fit and eats the rest of itself, nothing overflows the body
  and the reading view has no scrollbar. The body is the only thing in that
  dialog that scrolls, so nothing inside it may clip.
- **`.dialog.dialog--file` is doubled because `.dialog` is declared after it.**
  A one-class modifier above the base rule loses every property they share on
  source order, which is invisible in a diff: the reading view opened at the
  ordinary 38rem for that reason. `styles.test.ts` walks the modifiers.
- **What a memory rewrite replaced is absent, empty or a page, and the three
  mean different things.** Absent is a call that replaced nothing, which is
  every other tool and every write recorded before the field existed: there is
  nothing to compare and the content is drawn as it always was. Empty is an
  agent's first memory, which replaced nothing because there was nothing, and
  draws as a page that is all new. A truthiness check collapses the first two
  and loses the only write where the whole page is the news. `Part::ToolCall`
  in `domain/envelope.rs`, then `trailStep`.
- **A figure is a fence in the reply, not a tool call and not a new part.** An
  agent has the numbers in hand at the moment it writes the sentence about them,
  so a chart behind a tool call is a round trip spent sending back something it
  had already finished computing. A fence also needs no runtime change at all:
  `as_plain_text` returns the text of a message, so the record, the prompt, the
  dedup fingerprint, search and a peer's copy keep working untouched, and the
  agent can read back what it drew. `Part::Json` is still unused and is still
  not this.
- **A chart spec that has not finished arriving is neither drawn nor refused.**
  A reply lands a token at a time, so a chart spends most of its life on screen
  as half a JSON object, and calling that an error puts a red box under every
  figure for a second, which teaches an operator the feature is broken.
  `looksComplete` counts braces outside strings, since a category legitimately
  named `}` must not end the document early, and until they balance the figure
  says it is still drawing. Once they balance, a spec that still will not parse is
  wrong rather than late, and says so.
- **A refused chart is drawn as its own source with the reason under it.** Both
  halves are load-bearing. The operator needs to see what their agent thought it
  was showing them, and the agent needs a sentence it can act on next turn,
  which is why every refusal in `readChart` names the field and the fix. "Invalid
  chart" costs a whole turn and teaches nothing.
- **The eight series colours are the output of a check, and the *order* is the
  check.** Neighbouring slots are what touch in a stack and cross in a line
  chart, so neighbours are the pairs that decide whether a chart is readable to a
  colourblind operator, and nobody can verify that by looking. The order came out
  of enumerating all 40,320 and keeping the 160 that pass on this app's own two
  surfaces. `palette.test.ts` recomputes every figure in `palette.ts`'s comment
  from the hexes themselves, so a hex nudged because a screenshot looked slightly
  off fails the suite. A ninth hue is refused rather than generated: a generated
  one is indistinguishable from one of the eight under colourblindness.
- **Nothing inside a chart's drawing is focusable, and the Figures table is
  why.** The `svg` is one `role="img"` with a sentence on it, which makes its
  subtree invisible to a screen reader by definition, so a label on a band
  would be announced to nobody, and tab stops would put twelve invisible
  rectangles between one message and the next for a readout the table already
  holds in a form somebody can read. The table is also the relief that lets
  three light-mode hues sit under 3:1 against the surface, and
  `palette.test.ts` asserts that debt so it cannot be dropped quietly.
- **A page an agent wrote is framed from `artifact.rs`, never from `srcdoc`.**
  A frame pointed at `srcdoc:`, `blob:` or `about:blank` inherits the framing
  document's content policy, and this app's forbids script. The page would draw
  and its script would silently never run: an empty rectangle that passes every
  test, which is the same failure `FileCard` has a note about. So it gets an
  origin of its own, and the round trip through `frame_artifact` is what buys it.
- **`allow-scripts` and `allow-same-origin` must never appear together.** On the
  frame or in `ARTIFACT_CSP`. Together they let the page remove its own sandbox
  attribute and reload out of the box, which is the whole lock. `default-src
  'none'` is the other half and is not decoration: `<img src="https://…/?data=">`
  is the cheapest exfiltration there is, and a model's page is content written by
  something that may have read a hostile web page earlier in the same turn.
- **The height reporter is prepended to a model's page, not appended.** A
  model's page is exactly where an unclosed tag lives, and an unclosed tag
  swallows everything after it. Ahead of the doctype it is still parsed and run.
  The parent trusts the message by the window that sent it and by nothing else:
  an opaque origin reports itself as `"null"`, so an origin check would either
  reject every real message or accept every forged one.
- **A peer is not told any of this.** The figure section is in the prompt for
  every reply mode but `ToPeer`. A peer is a model and wants the numbers, so a
  chart spec on that path is tokens spent drawing something nobody will look at.
- **`emit_reply` delivers a reply that carries a file and no text.** Handing over
  a document with nothing typed is normal, and judging the reply empty by its
  text alone drops the thing the turn was spent producing.
- **`body_with_files` names a file on an agent's own turns too, not just on
  incoming ones.** An agent that reads its last turn back without the file it
  attached has no record of handing anything over, so it attaches the document
  again and reports it as the first time.
- **Only the component drawing the live bubbles subscribes to `streams`.** One
  level higher, a single token re-renders every message in the transcript. The
  same split is why the line above the composer, the turn's chips and the open
  thinking are three components: they sit next to each other and change at
  wildly different rates, and written as one every token re-rendered every chip.
- **A turn's thinking is held whole and drawn one line at a time.** Those are
  two decisions, and holding 240 characters made them one: the tail was all
  there was, which is fine for a wait of thirty seconds and no use for one of
  ten minutes. Nothing about holding it widens what "never kept" means. It is
  the same slice, dropped by the same event, and it reaches no channel, no
  prompt and no hash.
- **The line drawn is the last sentence that *finished*, under the model's own
  heading.** Not the tail. A tail replaced every sixteen milliseconds is a
  flicker that says a turn is alive, which is what the pulse already said, and
  nobody can read a sentence as it is typed. Waiting for the full stop costs a
  second of staleness and is the difference between a line and a blur.
- **The live trail and the thinking have one lifetime, because they have one
  mechanism.** `ToolStarted` and `ToolFinished` are addressed to the placeholder
  exactly as `ReasoningDelta` is, so a retry that reopens under a new id starts
  both again. What was done is not lost by that: it is in the message that lands
  at the end of the turn, which is the record. These are only what that record
  looks like before it exists.
- **`ToolStarted` is emitted before the call, and that ordering is the
  feature.** A `run_command` can sit for a minute. A call reported only once it
  comes back is silence for exactly as long as the wait it was meant to explain,
  which is the state that reads as a hang.
- **`ToolFinished` carries the whole `Part::ToolCall` and not its outcome.** The
  chip a turn draws while it runs and the chip the transcript draws afterwards
  are then one function over one value, rather than two that agree on the day
  they were written. The outcome alone was enough until `replaced` arrived, at
  which point a live memory rewrite silently stopped opening as a diff while the
  recorded one still did.
- **A transcript decides where the operator is by comparing the offset, not by
  listening for a scroll event.** The event is delivered after the fact and a
  token committing in between arrives first, so anything that waits to be told
  has already put the view back on the floor: under streaming text a trackpad
  could not climb out of a channel at all. `lib/follow.ts` remembers the offset
  it wrote and checks the box is still there before writing again, which is why
  one pixel is enough and no threshold is. Its listener is bound by a ref
  callback for the same reason: the node is replaced whenever the pane shows a
  pair thread or the activity board, and an effect cannot re-bind on that. The
  size observer beside it is bound there for that reason too, and it is not
  decoration: everything under a transcript takes height from it without
  anything arriving or scrolling, so a composer growing a line or the working
  panel opening put the newest message under the fold and left it there.
- **What a plugin's sign-in asks for is the resource's list, not its
  authorisation server's.** They are two documents and two lists: RFC 9728 names
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
  with `signed_in` false rather than claiming a sign-in that never happened.
- **The loopback port is bound before the client is registered.** That ordering
  is the whole reason a redirect is acceptable here at all, and it is the
  difference between this flow and the one `subscription.rs` argues against:
  `docs/PLUGINS.md`.
- **The sign-in tests carry real cookie names.** A cookie's presence is not a
  login. Do not loosen them without a fresh capture from a live machine.
- **All three conditions in `needs_consent` are load-bearing.** Each one alone
  refuses honest work. Read the doc comment before narrowing or widening any.
  The fourth clause is not one of them: a yes is remembered against that site
  for the rest of the turn, because asking per press produced four dialogs in a
  row for one account and a question in that shape is one an operator clicks
  through. It is not a standing yes. It lives on the turn's `Reading`, reaches
  no table, and `Reading::took_in` drops it the moment the turn takes in a page
  from anywhere else.
- **An envelope booked against a run is released by whatever consumes it.** A
  path that takes one without turning it into a turn leaves the run outstanding
  for the life of the process.
- **A key in settings says what the workspace can hand out; the card says who
  was given it.** Both have to be true, and `Surfaces::given_to` is the only
  place they meet. Deciding from the key alone is what this replaced: every
  agent was offered `run_command` and `browse`, and the first one to think of it
  rented a machine mid-turn. Deciding from what an agent is holding instead
  would be worse than either, because a machine is reclaimed on the provider's
  clock: the tools would vanish from a working agent the moment its sandbox
  slept.
- **The gate is in `ensure_computer` and `ensure_browser`, not at the tool call
  sites.** Those two functions are the only places a machine or a browser is
  made, and tools are not the only route to them: a file arriving for an agent
  is placed on its machine, and a text file too long to inline is placed there
  too. A gate at the call sites would rent a machine for an agent the operator
  deliberately did not give one. `Runtime::not_given` sits in front of the
  dispatcher as well, and that is not a duplicate: it is what turns a model
  calling a tool it was never offered into a refusal it can act on rather than
  an error that reads like a broken machine.
- **What an agent is *given* comes from the turn's card; what it *holds* comes
  from the row.** `run_turn` reads one `AgentCard` and passes it through every
  round, so a machine or a browser provisioned by the first tool call is on the
  row and not on that snapshot. Read from the snapshot, the second call of a
  turn provisions again: a duplicate sandbox that bills until the sweep finds
  it, and a second browser Kernel refuses by name, which is a `browse` tool that
  fails for the rest of the turn after the first page loads. `Runtime::held` is
  the read, and the card stays the authority on `has_computer` and `has_browser`.
- **A 409 from Kernel's create is an orphan to adopt, not a failure to
  report.** The name is one per agent, so a conflict is this agent's own
  browser, alive and unrecorded: a crash between creating one and writing it
  down. It is found by its `guac-agent` tag rather than by the name the conflict
  was about, and asked for by id, because a list row is not documented to carry
  a socket and a session without one names a browser nothing can talk to.
- **Taking a computer back leaves `sandbox_id` where it is.** The machine sleeps
  and its disk stays, because that disk is where the operator's sign-ins live.
  A revoke that destroyed it would make giving the computer back mean signing
  everything in again, which is the one thing an agent cannot do for itself.
  Taking a browser back closes it instead, for the same reason from the other
  end: closing is what writes the cookies to the profile.
- **Every `use_screen` action answers with a picture, and only the newest one
  stays.** The first is what stops a model acting on a screen two actions old;
  the second is what keeps that affordable. Removing either breaks the other.
- **A machine's Chrome opens no debugging port.** Two ways to use the web on one
  screen disagreed about which window was in front, and each fix moved the
  disagreement. `docs/BROWSERS.md` has the history before you add one back.
- **A sign-in is stored against the surface it was found on.** Both are scanned
  independently, so a replace that took the agent's whole set would erase the
  other's findings on every scan.
- **A fired routine carries `Part::Routine`, not text.** The instruction reaches
  the model either way, because `as_plain_text` returns it. The part is what
  keeps the transcript from drawing a schedule firing as Guaca talking to the
  operator: `docs/ROUTINES.md`.
- **`Routine::next_run_at` is an `Option` because some triggers have no next
  run.** A sentinel date would be shown to the operator, and it is one bad
  comparison away from firing something that was waiting on a connector.
- **An agent's standing routines are in its prompt, and that is not a duplicate
  of `schedule` with `list`.** A list behind a tool call arrives after the model
  has decided what to do, so an agent asked to change something it already keeps
  wrote a second routine beside the first and reported the change as made. Both
  fired. `docs/ROUTINES.md` before you take the section out, and note that
  `update` is what the ids in it are for.
- **A stop marks a run and releases nothing.** `track_inflight` reads a negative
  delta against a run it is no longer counting as that run reaching zero, and
  emits a second `RunSettled` for it. So a stop that helpfully released the
  envelopes it was ending would report the run finished twice, which fails the
  trajectory suite and double-counts its spend. Marking is the whole mechanism:
  each of the three boundaries releases through `finish_turn`, exactly as an
  ordinary turn does.
- **The stop check sits before `reserve_step`, not after it.** One line later and
  a run reports a model call that a stop prevented, for the rest of its life.
- **The checks inside the round loop do not cover the way out of it.** A turn
  whose last call returns text and no tool calls leaves by the break at the
  bottom, so there is a fourth check after the loop and before the reply is
  decided. Without it a stop that landed during a single-round reply turn — the
  whole turn, in that shape — still wrote to the peer that was waiting.
- **An actor only ever examines the envelope it is holding.** A paused agent
  parked on one run therefore cannot see that another run's work is queued behind
  it, which is why the pause park drains the queue whenever anything is stopped.
  Survivors go to a holding queue and stay counted in `depth`.
- **`--flesh` and `--flesh-soft` are pinned on `.rail`.** The rail is dark in
  both surfaces and reads both tokens, so a dark value for either would repaint
  it and no test would notice. Pinning them on the one element every rail rule
  descends from makes the rail a colour scope rather than a naming convention.
- **`data-surface` is only ever `light` or `dark`.** `system` is resolved before
  it reaches the document. A stylesheet rule keyed on `system` would have to
  duplicate the one keyed on `dark`, and CSS has no way to share them.
- **The menu bar's presence is read, not accumulated.** Every number on it but
  the session total is a fresh read of the roster, the activity map, the pending
  requests and the usage table. One assembled by adding up events drifts the
  moment one is missed, and what drifts is the number the operator is using to
  decide whether to go and look.
- **`menubar::plan` exists so an open menu is not replaced under the operator.**
  Same row shapes in the same order is the same menu saying different numbers,
  which is a text edit; anything else is a rebuild. The spend on that menu moves
  every few seconds while a crew works, so a strip that rebuilt on every change
  would close itself exactly when it was worth reading.
- **The attention glyph is the one tray image that is not a template.** macOS
  tints a template image to match the menu bar, so a template glyph cannot have
  a colour. Giving up the tint buys the one state that must not be missed, and
  the count beside the icon says the same thing in text.
- **An ampersand in a menu item has to be doubled.** Every platform's menu reads
  `&` as a mnemonic marker and eats it, so an agent called `R&D` draws as `RD`.
  `menubar::escape_mnemonic` is applied on the way into an item and nowhere
  earlier, so the rows a test reads are the words a person would.
- **A model suggestion is ranked by capability inside a use case, never by
  OpenRouter's default order.** That order is tokens routed, which is bulk
  traffic: the same cheap high-throughput model tops eleven of the twelve use
  cases, so three suggestions built on it are the same three under every agent
  with a different sentence above them each time. The category picks the pool
  and `sort=intelligence-high-to-low` picks the order inside it. The price on
  each row is not decoration either: capability ordering ignores price, so
  without it the button is a one-click way to make every turn forty times
  dearer.
- **An unknown category is refused in `catalogue.rs`, not by OpenRouter.**
  OpenRouter answers one with 200 and an empty list, so a slug it has renamed is
  indistinguishable from a use case nobody sends work to, and the dialog would
  draw nothing for exactly the agents it was built for. `ipc.contract.test.ts`
  compares the twelve in `CATEGORIES` against the twelve in `ROLES`, and the
  `#[ignore]`d test in `catalogue.rs` asks the live service whether it still
  ranks all of them, which is the failure no offline suite can see.
- **`roleFor` returning nothing is the common answer, and a tie returns
  nothing too.** Most agents are a Manager or an Inbox and OpenRouter has no
  category for either. A scorer that always names its best guess puts a legal
  model under a scheduling agent, and one bad suggestion is what teaches an
  operator to ignore the good ones. Sales is the single deliberate bend: nothing
  ranks it, so its vocabulary scores into marketing.
- **Closing the window hides it, and only while the tray exists.** Tauri exits
  when the last window closes, which for this app means a routine set for every
  morning stops firing the first time somebody tidies their screen. A hidden
  window is not a closed one, so preventing the close is the whole mechanism.
  The condition is not caution: an app with no window and no menu bar icon is
  one the operator cannot see, cannot reach and cannot stop.

## Conventions

- Match the surrounding code. Comments explain why, never what.
- Every guard refusal and every error the operator can hit says what happened
  and what to do about it. Both are read by a model or a human under pressure.
- Errors an agent reads mid-turn need a way forward, not just a reason. A
  refusal that only says no gets reworded and retried.
- New behaviour needs a test that would fail without it. Failure paths first.
- No dead code, no speculative API surface. The contract test fails on a command
  nothing calls.

## Ownership

**A person owns every commit, and the tool that helped write it is not a
co-author.** Whoever commits answers for the change: in review, in the incident,
and a year later when the reason matters more than the diff. That accountability
does not divide and does not transfer, so the record must not suggest it did.
Use whatever tools you like and sign your own work.

No machine signature anywhere. No `Co-authored-by` trailer naming a model, no
"Generated with" footer, no session or tool link, in commits, PR titles and
bodies, issues, comments or code.

**A trailer written on a contributor's branch still reaches `main`.** GitHub's
squash-merge collects `Co-authored-by` lines out of the commits it squashes and
appends them to the squash message, so a trailer nobody chose arrives on the
default branch through the merge box. Two commits reached `main` that way before
anyone noticed, and taking them back out meant rewriting published history. Read
the merge box before confirming it. Claude Code emits the trailer by default and
stops when `includeCoAuthoredBy` is `false` in its settings, which is the fix at
the source rather than at the end.

## Verify

```sh
./scripts/ci.sh          # lint, typecheck, build, every test suite
./scripts/ci.sh rust     # Rust only
GUAC_LOG=guac=debug pnpm app
```

Getting a change on screen has its own file: `.claude/skills/run-guaca`. It
holds the harness that draws a component in seconds without a Rust build, a key
or any spend; what to do when the app refuses to start; and the rule about the
operator's own workspace, which every workspace on the machine shares.

Three suites ask three different questions, and a change can pass one while
failing the next. *Three test suites, asking different questions* in
`docs/ARCHITECTURE.md` is the long version.

- **Cascade tests**, inside the Rust suite, ask whether the runtime did as it
  was told. They drive the real runtime against a scripted OpenAI-compatible
  server. If you change messaging, they are the ones that will catch you.
- **Evals** ask whether the resulting traffic is something an operator would
  want to watch. Every cascade defect this app has had passed the first suite
  and failed this one. CI cannot see a prompt that makes agents chattier, so if
  you change a prompt, run the live half: `./scripts/evals.sh`, which costs
  money.
- **Trajectory** asks whether the machinery behaved: every placeholder closed,
  every parked turn released, every model call on the run's bill, nothing filed
  against a run already reported finished. If you touch streaming, settle
  detection, retries or the budget, run it.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test trajectory
```

A fourth, narrower one exists for the subscription. `tests/subscription.rs` runs
the real runtime against a scripted *Responses* server, which is a protocol the
other three never touch, and it holds one `#[ignore]`d live test. Run that after
changing `llm/codex.rs`, or when a sign-in that worked stops working: everything
offline is a stub agreeing with what this app believes the protocol is, and the
failure worth catching is that belief going stale.

```sh
./scripts/subscription.sh    # a real call against your own ChatGPT plan
```

`tests/account.rs` is the same shape again for the guaca.bot sign-in: a scripted
authorization server, and the real `Account` driven through discovery, the
loopback listener, the PKCE exchange and the first call the token is spent on.
Its stub checks that the verifier presented at the token endpoint actually
hashes to the challenge that was sent, because a sign-in that stops proving that
still works. Its `#[ignore]`d half asks whether the live service still publishes
what this build reads, and `GUACA_ACCOUNT_ORIGIN` points it at a Worker on this
machine instead. It authorizes nothing and stores nothing.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test account -- --ignored
```

A fifth, `tests/plugins.rs`, does the same job for MCP: a scripted server that
publishes the four metadata documents an OAuth sign-in needs, and one runtime
turn that calls a plugin tool end to end. Its live half runs `oauth::discover`
against every vendor on the list and asks whether each still publishes what this
build expects, which is the failure no offline test can see. It reaches the internet, authorises nothing and spends nothing.

```sh
./scripts/plugins.sh
```

A sixth, `tests/machines.rs`, is the same shape for the two providers: scripted
control planes for Kernel and E2B, and the real `Runtime` provisioning against
them. It is entirely offline and costs nothing. Nothing else in the build
reaches a provider, so without it every suite passes with a turn renting a
machine on every tool call. Run it after touching `ensure_computer`,
`ensure_browser` or either client.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test machines
```

The model suggestions beside an agent's model field have the same shape again,
without a script because it is one test. It asks the live OpenRouter whether it
still ranks models for all twelve of the use cases this build believes in, which
is the one failure the offline suite cannot see: a category renamed there answers
200 with an empty list. It reaches the internet, authorises nothing and spends
nothing.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib \
  llm::catalogue::tests::every_use_case -- --ignored --nocapture
```
