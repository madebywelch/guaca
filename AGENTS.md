# Guaca

Local desktop app. You talk to LLM agents; the agents talk to each other. Tauri
v2, React + TypeScript front, Rust back.

This file is the map and the routing table. What is left in it applies wherever
you are working. The reasoning lives in `docs/`, one file per subsystem, and the
table below says which one to open before changing something.

## Where things are

```
src/                  React + TypeScript. A view over the runtime, nothing more.
  avatars/            An agent's character. Five shapes, drawn from numbers.
    silhouette.ts     The five, as one radius function each, and what sizes them.
    form.ts           The body, as a function of a character and a mood.
    eyes.ts           One stroke, four numbers, and where it is looking.
    catalog.ts        The cast, and every key an older build wrote.
    moods.ts          Ten expressions, and the one place a signal becomes one.
    clock.ts          One frame loop for every creature on screen.
  lib/transcript.ts   What a channel shows, and what it collapses. Read first.
  lib/rail.ts         What order the rail draws agents in, and where a drop lands.
  lib/presence.ts     A crew, in the two marks its circle can carry.
  lib/orb.ts          How a crew stands inside its circle, and when it counts.
  lib/reach.ts        How close the pointer comes before the crews slide out.
  lib/search.ts       One ranking over hits from SQLite and from the store.
  lib/trail.ts        A turn's own tool calls: what folds into one chip.
  lib/callout.ts      The one part of a reply that is for you, in a box.
  lib/figure.ts       A fenced block the transcript draws instead of printing.
  lib/chart.ts        A chart spec, and where every mark goes. No DOM.
  lib/palette.ts      Eight hues in one order, and why that order and not another.
  lib/diff.ts         Two versions of a page, as the lines between them.
  lib/reasoning.ts    A turn's own thinking: how much is held, what is drawn.
  lib/cafeteria.ts    Preset agents, waiting to be hired. Content, not runtime.
  lib/compost.ts      Where a deleted agent waits, and how long it has left.
  lib/roles.ts        What an agent is for, in OpenRouter's twelve words.
  lib/plugins.ts      A plugin's mark and color. Everything else is Rust's.
  lib/ipc.ts          Every call into Rust.
  lib/transport.ts    How a call travels: Tauri's bridge in a window, HTTP and
                      a socket in a browser or in a window pointed at a box.
                      Which one is read at runtime.
  lib/menubar.ts      The strip's view of the store, for a window showing a
                      box. One projection, and the test beside it is the gate.
  lib/prefs.ts        What the operator sets and the runtime never reads.
  lib/appearance.ts   Scale and surface, as one write to the root element.
  lib/follow.ts       Whether a transcript may move under the operator.
  lib/notify.ts       When an interruption is warranted. Mostly when it is not.
  lib/announce.ts     What that interruption would say. One event in, one line out.
  lib/keybinds.ts     Every key the app answers to, in one list.
  lib/limits.ts       The five bounds a conversation runs inside, in words.
  components/         One file per surface.
    Memory.tsx        What an agent remembers, and what happens when both of
                      you write it at once.
    WorkingNotes.tsx  What it is in the middle of, which is the other store
                      and the one that expires.
    TokenEntry.tsx    The one screen a browser sees before the workspace, and
                      only a browser.
src-tauri/src/
  domain/             AgentCard, Envelope, Routine, Connector, Signin, Approval,
                      Search, ids. No I/O.
    repository.rs     A directory an agent may write code in, which of two
                      programs writes it, and whether it asks before pushing.
    worknote.rs       A line about work in flight, and why it is not memory.
    promise.rs        A closing sentence that says the work is still coming,
                      in a turn that is over. One rule, two readers.
    approval.rs       The two things an agent stops to ask a person, and why
                      only one of them may draw the model's own words.
    escalation.rs     The third thing, which stops nothing: work an agent
                      cannot move and only the operator can.
    group.rs          A crew's wall, and the settings its agents run on.
    plugin.rs         The servers a crew can sign in to, what it got, and
                      which of its agents may spend it.
    deployment.rs     Desktop or server, and the five things that decides.
                      Nothing about who paid for the box.
  runtime/
    guard.rs          The loop guard. Read this one first.
    mod.rs            Agent actors and the message bus.
    prompt.rs         Prompt assembly, including the trust boundary.
    events.rs         Events pushed to the UI.
  llm/                OpenAI-compatible client, SSE decoding, tool definitions.
    codex.rs          The other protocol: where a ChatGPT subscription is spent.
    claude.rs         The third, which is a program rather than a protocol:
                      where a Claude subscription is spent, and why it has to be.
    catalog.rs        Which models OpenRouter sees doing which kind of work.
    modality.rs       Whether a picture reaches the model that is answering.
  subscription.rs     Signing in to that subscription. A credential, not a wire.
  account.rs          The optional Guaca account. Nothing else depends on it.
  mcp.rs              The client end of MCP, in both of its protocol eras.
  oauth.rs            Signing a crew in to a plugin's server. PKCE, no client id.
  plugins.rs          Where those two meet the store, and a turn spends a grant.
  repo.rs             Whether a directory is one an agent may be given. Runs git.
                      Also the work tree each agent gets of its own, and when
                      that one may be put back on the default branch.
  shell.rs            One line in that directory, run and answered in the turn
                      that asked. The small door; `coding/` is the big one.
  coding/             Starting something that writes code, and reading it back.
    mod.rs            One process, one ceiling, one prompt. Read this one first.
    pi.rs             `pi`'s argument vector and its stream.
    claude_code.rs    Claude Code's, which are not the same and cannot be.
    bridge.rs         The other end of a job that is still running: what an
                      operator can say to one, and what it may not do alone.
  db/                 SQLite. Plain SQL, numbered migrations.
  e2b.rs              Computers: the machines agents look at and point at.
  proxy.rs            Loopback viewer for those machines.
  artifact.rs         The other loopback origin: where a page an agent wrote
                      is allowed to run, and everything it may not reach.
  sessions.py         Reports what a machine's Chrome is signed in to.
  kernel.rs           Browsers: a hosted Chrome, which is where the web belongs.
  cdp.rs              The DevTools protocol. Asks a page instead of looking.
  workspace.rs        Per-agent memory: one markdown file the agent rewrites.
                      Its counterpart is `domain/worknote.rs` plus one table.
  files.rs            Attachments, addressed by the SHA-256 of their contents.
  eval.rs             Reads a run and says whether it communicated sensibly.
  trajectory.rs       Reads a run's events and says whether the machinery did.
  config.rs           Operator settings, and the API key the webview never sees.
  programs.rs         The PATH the four programs this app runs are found on,
                      which is not the one a double-clicked app is started with.
  commands.rs         The entire IPC surface. Knows no host.
  ipc.rs              That surface written once: one macro makes the Tauri
                      wrappers, the HTTP dispatch and the list the contract
                      test reads.
  boot.rs             Opening a workspace, which is the same act in both hosts.
  server/mod.rs       The second host: the same runtime over HTTP and a
                      socket, behind one token.
  bin/guacad.rs       The daemon that starts it, configured from the
                      environment because systemd starts it, not a person.
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
| A message that arrives while an agent is already working, and the coding job whose answer could not reach the turn that started it | *A working turn reads its inbox, and only what it can answer where it stands* in `docs/ARCHITECTURE.md`, then `Runtime::take_in` and `Intake` |
| What a turn is told it was asked for: `expects_reply`, `intent`, `ReplyMode` | *Cascades terminate because of one asymmetry*, and `runtime/prompt.rs`, which has to agree with it |
| A turn that ends on "checking now" and does nothing, the round it is given back, the fault that counts them | *A turn ends when the model stops calling tools, so a closing promise is silence* in `docs/gotchas/runtime.md`, then `src-tauri/src/domain/promise.rs` and the `## Your reply` block in `runtime/prompt.rs` |
| Streaming, retries, the budget, when a run settles | *A failed model call is retried*, *A thought is shown and never kept*, *The budget counts model calls* |
| What a turn shows of itself while it runs: the thinking, the calls, the line above the composer | *A thought is shown and never kept* and *A turn's own work is watched while it happens*, then `src/lib/reasoning.ts` |
| How a turn is paid for: providers, the ChatGPT sign-in, the Responses API | *A subscription is a second provider, not a second endpoint*, then `llm/codex.rs` and `subscription.rs` |
| Turns paid for by a Claude plan, the `claude` program, the structured answer it is held to | *What the restriction leaves open, which is the program* in `docs/PROTOCOL.md`, then `llm/claude.rs`, and run the live half of `tests/claude.rs` |
| A sign-in that stopped working, refreshing, expiry, signing out | *A token's `exp` is a floor on its life, not a ceiling* in `docs/ARCHITECTURE.md`, then `Subscription::renew` and the 401 path in `codex::stream` |
| What a group decides for itself: provider, models, timeout, limits | *A group chooses its own provider*, *Nothing about who pays is inferred* and *A run is measured against the limits of the group it happens in*, then `domain/group.rs` |
| Stopping a conversation: what a stop marks, wakes, and must never release | *A stop marks the run and releases nothing*, then `Runtime::stop_run` |
| Permission prompts, parked turns, acting in the operator's name | *A protected action parks the turn that asked for it* |
| An agent writing code at all: the repository, the grant, the `code` tool, the job | `docs/CODING.md`, then `domain/repository.rs` and `Runtime::start_job` |
| An agent running one command in its repository, and which of the two doors a piece of work goes through | *A repository has two doors, and the small one is `shell`* in `docs/CODING.md`, then `src-tauri/src/shell.rs` and `Runtime::run_in_repository` |
| Which directory a job actually runs in, worktrees, resetting a tree between jobs, two agents working in one codebase at once | *Each agent gets a work tree of its own, and Guaca resets it* in `docs/CODING.md`, then `domain::repository::Bench`, `repo::prepare` and `Footing::resettable` |
| Which program writes the code, a spent plan, a harness that will not start | *There are two harnesses because a subscription is spent by one program* in `docs/CODING.md`, then `domain::repository::Harness` and `coding/mod.rs` |
| What branch a coding job starts on, and what it is told about the tree | *A job is told where it is standing before it is told what to do* in `docs/CODING.md`, then `repo::footing` and the brief assembled in `Runtime::start_job` |
| An argument either harness is started with, or how its stream is read | *One process lifecycle, two of what genuinely differs* in `docs/CODING.md`, then `coding/pi.rs` and `coding/claude_code.rs`, and run the live half of `tests/coding.rs` |
| Reaching a job that is already running, stopping one, or anything a hook does | *A job can be reached while it runs* in `docs/CODING.md`, then `coding/bridge.rs`, and run the live half of `tests/coding.rs`, which is the only thing that can check any of it |
| Whether a job stops before it pushes, and what counts as outward-facing | *The gate is a decision the operator takes per repository* in `docs/CODING.md`, then `bridge::outward` and `Runtime::park_with` |
| Whether a `shell` line stops before it pushes, and which asker the card names | *The gate is asked from the same function* in `docs/CODING.md`, then `Runtime::ask_about_push` and `Asker`, which have to answer for both doors |
| A push kept in a script, what the gate follows and what it will not | *The line is read, and then what the line runs is read* and *And what it deliberately does not read* in `docs/CODING.md`, then `bridge::outward` and `bridge::Reach` |
| An operator asked the same thing twice, and how long a no lasts | *One no settles the question for the rest of the run* in `docs/CODING.md`, then `Runs::refused` and `Runtime::ask_about_push` |
| What a job inherits from the operator's own Claude Code, and what that costs | *A job inherits the operator's own Claude Code setup* in `docs/CODING.md`, which has the measurement and the one hazard in it |
| A program that is installed and reported missing: `claude`, `pi`, `git`, `gh` | *A double-clicked app does not have the operator's `PATH`* below, then `src-tauri/src/programs.rs` |
| Anything an agent stops to ask a person: the desk, the queue, the two kinds of request, `ask_operator` | `docs/ATTENTION.md`, then `domain/approval.rs` and `Runtime::park` |
| An agent that cannot go on at all, what reaches the operator without parking a turn, `escalate` | *Three things an agent can do about a person* and *What an escalation is* in `docs/ATTENTION.md`, then `domain/escalation.rs` and `Store::raise_escalation` |
| The crews' column, its badges, how a crew names itself, which crew the rail is inside | *A group is a place you can be inside* in `docs/WORKSPACE.md`, then `src/lib/presence.ts`, `src/components/GroupRail.tsx` and `src/components/OrbTag.tsx` |
| When the crews' column comes out, how close is close enough, what holds it out | *The column does not stand open* in `docs/WORKSPACE.md`, then `src/lib/reach.ts` and the three boxes in `.grail` |
| Who spoke to whom, what a run cost, where that board lives | *The flow board is analysis, so it is in a crew's settings* in `docs/WORKSPACE.md`, then `src/components/GroupActivity.tsx` and `Store::conversation_flow` |
| What an agent may do with a page it has just read | *A page that was read this turn cannot quietly press a button* |
| Screenshots, coordinates, what a screen action answers with | *A computer is looked at, never asked* in `docs/MACHINES.md` |
| Attachments, previews, drops, handing a document to the operator | *Files are references, and what a model gets depends on what they are* |
| SQLite, the pool, migrations | *Storage*, and the two comments in `Store::open` |
| Schedules, triggers, what a firing looks like | `docs/ROUTINES.md` |
| Whether a firing lands on an agent that is already working | *A firing can be skipped, which is not the same as deferred* and *A skipped firing is in the history* in `docs/ROUTINES.md`, then `Activity::is_working` and `Runtime::sweep_schedule` |
| What an agent knows about its own schedule, and how it changes one | *An agent reads its own schedule before it decides to write another one* and *Changing a routine is `update`* in `docs/ROUTINES.md` |
| Whether an agent may have a computer or a browser at all | *A computer is given to one agent, not to the workspace* in `docs/MACHINES.md`, then `Runtime::surfaces_for` |
| Sandboxes, the desktop, the screen, sign-ins on it | `docs/MACHINES.md` |
| Hosted browsers, CDP, `browse`, live view, browser profiles | `docs/BROWSERS.md` |
| Which of the two a piece of work belongs on, and credentials | *Connectors* in `docs/PROTOCOL.md`, then both files above |
| Plugins: what is on the list, signing one in, calling its tools | `docs/PLUGINS.md`, then `oauth.rs` and `mcp.rs` |
| A server the operator added: its name, its address, a pasted key | *A server the operator added* in `docs/PLUGINS.md`, then `PluginKind::custom` and `PluginKind::from_row` |
| Headers an operator gave a server: what is refused, where they go | *Headers, which are how the request arrives* in `docs/PLUGINS.md`, then `domain::plugin::Headers`, the loops in `mcp.rs` that apply them, and `oauth::Gate`, which is why they stop at one origin |
| Testing an address or a connected plugin without connecting it | *Testing it is the whole path, minus the browser* in `docs/PLUGINS.md`, then `plugins::inspect` and `plugins::check` |
| Anything about the wire: protocol versions, the handshake, headers | *Two protocol eras* in `docs/PLUGINS.md`, then `mcp.rs`, whose era probe is the one thing no offline test of a single server can check |
| Running Guaca somewhere other than the operator's machine: the daemon, the token, a browser as the client | `docs/HOSTING.md`, then `server/mod.rs` and `src/lib/transport.ts`, and run `tests/server.rs` under `--no-default-features --features server` |
| What a hosted workspace refuses, and why each refusal is a fact about a machine rather than a missing feature | *Five capabilities, and none of them is a feature nobody finished* in `docs/HOSTING.md`, then `domain/deployment.rs` and every reader of `capabilities` in `src/` |
| A command that works at a desk and fails on a box, or a new command at all | *One list, three readers* in `docs/HOSTING.md`, then the `surface!` block in `src-tauri/src/ipc.rs` and `ipc.contract.test.ts` |
| An invitation, a token that stopped working, the screen a browser sees before the app | *A browser is admitted by a token, and the token arrives by fragment* in `docs/HOSTING.md`, then `src/components/TokenEntry.tsx` and `adoptInvitation` |
| A file dropped or picked in a browser, or dropped on a window that is showing a box | *A browser hands a document over as bytes* in `docs/HOSTING.md`, then `onFileDrop` in `src/lib/ipc.ts`, the upload route in `server/mod.rs` and `forward_files` |
| A plugin or account sign-in from a box: where the redirect lands, the origin it names, the page the browser is shown | *A sign-in comes back through the origin the browser used* in `docs/HOSTING.md`, then `oauth::Landing`, `commands::Reach` and the callback route in `server/mod.rs` |
| The desktop app showing a box, the Workspace pane, and what the menu bar draws while it does | *The desktop app can show a box, and the menu bar follows* in `docs/HOSTING.md`, then `attached` in `src/lib/transport.ts`, `src/lib/menubar.ts` and `Tray::feed` |
| Which transport a server is spoken to over, and who gets the older one | *It speaks the transport that was replaced* in `docs/PLUGINS.md`, then `mcp::probe` and `sse_exchange` |
| Which agents in a crew get a plugin | *Signing in is one decision, and handing it out is another* in `docs/PLUGINS.md`, then `domain/plugin.rs` and `Store::plugin_tools`, which has to agree with `Store::plugin_reach` |
| Which of a plugin's tools which agents may call | *And which of its tools, for which of them, which is a third decision* in `docs/PLUGINS.md`, then `Store::set_plugin_tool` and both readers of `plugin_tool_access` |
| The guaca.bot account: signing in, what it is for, why it is optional | `docs/ACCOUNT.md`, then `account.rs` |
| Channels, the rail, search: what the operator sees | `docs/WORKSPACE.md`, then `src/lib/transcript.ts` |
| An agent's character: the drawing, a new one, or why there are no eyebrows | `docs/CHARACTERS.md`, then `src/avatars/form.ts` |
| A sixth shape, a shape that lost its corners, or why the drop is not bigger | *Five shapes, one weight* in `docs/CHARACTERS.md`, then `src/avatars/silhouette.ts` and the suite beside it, which is the gate |
| A new expression, or what the app reads one from | *Moods* in `docs/CHARACTERS.md`, then `moods.ts`, whose table and `moodFor` are the whole of it |
| Anything that moves on a creature: a look, a blink, a message landing | *The gaze moves the body* in `docs/CHARACTERS.md`, then `AgentAvatar.tsx`, which is the only place the three meet |
| Charts, tables, a page an agent wrote: what a reply can be drawn as | *A reply can be a figure* in `docs/WORKSPACE.md`, then `src/lib/figure.ts` and `src/lib/chart.ts` |
| A box round the part of a reply that needs the operator, and which marker draws which one | *A reply can mark the one part that needs a person* in `docs/WORKSPACE.md`, then `src/lib/callout.ts` |
| A chart's colors, or how many series one may carry | *A chart's colors are the output of a check* in `docs/WORKSPACE.md`, then `src/lib/palette.ts` and the test beside it, which is the gate |
| Running a model's own HTML, or anything about that origin | *A page an agent wrote runs somewhere else* in `docs/WORKSPACE.md`, then `src-tauri/src/artifact.rs` |
| A page the operator can work, and what it may hand back | *A page can hand one value back* in `docs/WORKSPACE.md`, then `BRIDGE` in `src-tauri/src/artifact.rs` and `Answering` in `src/components/HtmlArtifact.tsx` |
| A turn's tool calls in a channel: what folds, what a chip says, what opens | *A turn's own work is chips* in `docs/WORKSPACE.md`, then `src/lib/trail.ts` |
| What an agent changed about its own memory, and where the version before it came from | *A memory rewrite opens as a diff* in `docs/WORKSPACE.md`, then `Workspace::write` and `src/lib/diff.ts` |
| What an agent currently remembers, and editing it by hand | *An agent's memory is in the panel* in `docs/WORKSPACE.md`, then `src/components/Memory.tsx` and `src-tauri/src/workspace.rs` |
| Which of the two stores something belongs in, what `note_progress` is for, why one is a file and the other a table | *An agent's memory is what it knows, and its working notes are what it is doing* in `docs/WORKSPACE.md`, then `src-tauri/src/domain/worknote.rs`, whose header is the argument |
| An `@` that names an agent: what resolves, and what it draws in either place | *A mention is one thing, in the box and in the message* in `docs/WORKSPACE.md`, then `src/lib/mentions.ts` and the layer under `Composer`'s textarea |
| A size, a space, a radius, a duration or a shadow, anywhere in the app | *Every length is named* below, then the token block at the top of `src/styles.css`, and the closed-set suite in `styles.test.ts`, which is the gate |
| What color a column is, a surface a panel is drawn on, anything about light or dark | *The page is the only white thing, and both edges are the same off-white* in `docs/WORKSPACE.md`, then the two token blocks in `src/styles.css` and the columns suite in `styles.test.ts` |
| Anything announced to a screen reader, or a live region | *A transcript is a log, and says one thing out loud* in `docs/WORKSPACE.md` |
| Scrolling a transcript, following the newest line, when the view may move | *A transcript follows the end for whoever is at the end, and nobody else* in `docs/WORKSPACE.md`, then `src/lib/follow.ts` |
| The menu bar: the glyph, the count, what the menu offers, closing the window | *The menu bar is Guaca with the window shut* in `docs/WORKSPACE.md`, then `src-tauri/src/menubar.rs` |
| The rail's order, dragging a row, groups as places you go inside | *The rail is arranged by hand*, *A drop is one call* and *A group is a place you can be inside* in `docs/WORKSPACE.md`, then `src/lib/rail.ts` and `src/lib/orb.ts` |
| Deleting an agent, putting one back, what the thirty days hold | *Deleting an agent is a thirty-day hold* in `docs/WORKSPACE.md`, then `Runtime::discard_agent` and `Runtime::purge_agent`, which are the two halves of what used to be one act |
| Deleting a group, and why a disband does not use the compost | *Deleting a group deletes the crew, and the machines they were renting* in `docs/WORKSPACE.md`, then `disband_group` in `src-tauri/src/commands.rs` |
| Preset agents, hiring a crew | *The cafeteria is a copy machine* in `docs/WORKSPACE.md`, then `src/lib/cafeteria.ts` |
| Settings, the surface, the scale, what may interrupt the operator | *Settings is nine places*, *The page is the only white thing, and both edges are the same off-white* and *An interruption has to earn it* in `docs/WORKSPACE.md` |
| What About says this build is, and where that string comes from | *About says which commit it is* in `docs/WORKSPACE.md`, then `src/lib/build.ts` and the `define` in `vite.config.ts` |
| The group editor: what a crew overrides and what it inherits | *A group's settings are the app's, with the crew's answer on top* in `docs/WORKSPACE.md`, then `src/components/GroupEditor.tsx` |
| What model an agent is offered, and how its job is guessed at | *The model field suggests three, and is still a text box* in `docs/WORKSPACE.md`, then `src/lib/roles.ts` and `llm/catalog.rs`, whose twelve use cases have to agree |
| Whether a model can be shown a picture: an attachment, a screen, what an agent is told it is | *What a model can be sent is asked of the endpoint, not assumed* in `docs/ARCHITECTURE.md`, then `llm/modality.rs` and the four places `Modalities` is spent from `Runtime::run_turn` |
| A prompt, or anything that changes how much a crew talks | *Three test suites, asking different questions*, then run the live evals |
| What a real crew of eight does with one directive, and what is different when you ask twice | *A crew is watched rather than asserted*, then `src-tauri/tests/crew.rs` |

Unqualified section names are headings in `docs/ARCHITECTURE.md`. Every row
also has a shorter companion under `docs/gotchas/`, indexed below: the doc
argues the design, the gotchas file lists what has already been broken by
changing it.

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

**A coding harness is a second program, and there are two of them because a
subscription is spent by the program it was issued to.** `pi` holding an
Anthropic OAuth credential and dialling the Messages API is refused with *You're
out of extra usage* while `claude` on the same machine and the same account runs
the work off the plan. So an operator whose one plan is spent needs the other
*program*, and no amount of configuration on the first reaches it. The choice
lives on the repository, beside the note, because it is the same shape of fact:
how work happens in this directory. Everything inside a harness (the model, the
thinking level, the sign-in) belongs to the harness and is never passed from
here. `docs/CODING.md`.

**A Claude subscription pays for a turn by being the program, never by holding
its token.** Anthropic restricts consumer OAuth tokens to Claude Code and
Claude.ai, enforced server-side since January 2026 and explicit in its terms
since February 2026, so an Anthropic sign-in in this app is not implemented and
will not be: there is no field for one and nothing in `llm/claude.rs` that could
carry it. What the restriction leaves open is the program. `Provider::Claude`
runs `claude` once per model call and reads its stdout, so the credential never
leaves the program it was issued to, and the operator signs in where they already
did. Same sentence the coding harness is built on, one level up: there it decides
who writes the code, here it decides who answers the turn. Dates and sources:
`docs/PROTOCOL.md`.

Guaca keeps its own round loop on that provider, and that is the load-bearing
half. The program is an agent harness and would happily run its own rounds, which
would move `max_tool_rounds`, `reserve_step` and every stop check inside a process
this app does not control. So it is given no tools at all and asked for one
structured answer per call through `--json-schema`, built from the turn's own
`ToolSpec`s as a discriminated union: what comes back is a thing to say and a list
of calls, the runtime dispatches them exactly as it does for the other two, and
`runtime/mod.rs` never learns there was a third. A coding job can afford to hand
its loop over because it is a different unit of work with its own budget. A turn
cannot: it is the unit the five limits are written in.

**The runtime runs in two hosts, and neither it nor `commands.rs` knows
which.** A window on the operator's machine and a daemon on a box that stays
awake are `app.rs` and `server/mod.rs`, over one library. `boot.rs` opens the
workspace for both, `ipc.rs` writes the command surface once, and
`domain/deployment.rs` is the only place the two are told apart: five
capabilities, each something physically on the operator's machine, refused in
the command before anything is spent and drawn on the row before the field is
filled in. A managed box and an operator's own box are the same variant, and a
third would mean something below that line had started caring who paid.
`docs/HOSTING.md`.

**A creature is a shape, not a drawing.** Every agent is cut from one of five
silhouettes (circle, octagon, square, water drop, cloud) and recomputed every
frame from that shape, a character (a row of numbers) and a mood (another row).
No path is ever drawn by hand and no transform is ever put on the drawing: a
character that slides around inside its own box reads as a sprite being moved,
and one whose outline changes reads as a thing that is alive. The body only
breathes, leans and settles, because a body that acts as hard as a face is a
body nobody can read a face on; what acts is two eyes, each one stroke with four
numbers on it. Four casts of hand-drawn characters preceded this and every one
of them failed the same way. `docs/CHARACTERS.md`.

Nothing below `silhouette.ts` knows how many shapes there are: a shape is the
first term of the resting radius and a mood is what gets added to it, so a cloud
kneads, sags and settles through the code a circle does. The two numbers that
size them are the whole of the design. Every silhouette encloses the circle's
area, computed at load rather than balanced by eye, and none rests past `CREST`,
because the moods already spend nearly all the room between `FORM.radius` and
`FORM.reach` on the swell that follows a look. A shape with a point or a flat
underside gives area back rather than taking that room.

**Every length is named, not spelled.** A size, a space, a radius, a duration,
an easing and a shadow are each spelled from a closed set of tokens at the top
of `src/styles.css`, and `styles.test.ts` fails the build on a literal. The rule
is not that 13px is the right size for a second line: it is that the decision is
*named*, so changing your mind means editing one token and watching every rule
that shares the decision move with it. Spelled at the point of use a decision is
invisible, and this file reached 41 font sizes with 38 of them inside ten
pixels, 41 padding lengths, 22 radii and 5 easing curves with a considered color
system sitting at the top of it the whole time. Nobody chose that, and no review
would have caught it. The exceptions are named in the suite with their reasons;
add one there rather than working around it.

**One arrival, and two registers of motion in CSS.** Every surface that appears
runs `@keyframes pop` at `--tempo-enter` on `--ease`, listed in one rule, and a
new surface joins by adding its selector to it. What tells a menu from a dialog
is `--pop-origin`, which a menu's own component sets to the corner nearest the
button that opened it, *after* measuring where it actually fit: a menu pulled
back off a window edge and grown from the corner it wanted slides across the
screen on the way in. The two curves are not preferences. `--ease` is a surface
arriving and `--ease-loop` is anything that loops or travels. A third has to
argue it is a register.

There was a spring, for the one register that is no longer CSS: a character
reacting. An agent is a shape recomputed every frame now, so its overshoot is a
spring in the geometry and its recoil is a displacement of its own outline.
`docs/CHARACTERS.md`.

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

Every entry is a change that looked obviously right, the failure it caused, and
what the design does instead. There are too many to carry everywhere, so they
are filed by category under `docs/gotchas/`: read the one you are about to work
in, on the way in. The long doc beside each says why the design is what it is;
the gotchas file says what it already cost somebody to change it.

| Working on | Read |
|---|---|
| Turns, the inbox, replies, stopping a run, what a run is billed | `docs/gotchas/runtime.md` |
| A ChatGPT sign-in, the `claude` program, either of their wires | `docs/gotchas/providers.md` |
| Model suggestions, and whether a model can be shown a picture | `docs/gotchas/models.md` |
| Repositories, the two doors, the gate, either harness, the bridge | `docs/gotchas/coding.md` |
| Plugins, MCP, and the OAuth they and the account share | `docs/gotchas/plugins.md` |
| The daemon, a browser as a client, the boot both hosts share | `docs/gotchas/hosting.md` |
| Computers, browsers, sandboxes, sign-ins found on them | `docs/gotchas/machines.md` |
| Schedules, triggers, firings | `docs/gotchas/routines.md` |
| Approvals, questions, escalations, the desk | `docs/gotchas/attention.md` |
| An agent's memory, its working notes, and the panels for both | `docs/gotchas/memory.md` |
| A turn drawn while it runs: the bubble, the trail, the thinking | `docs/gotchas/transcript.md` |
| Charts, callouts, and a page an agent wrote | `docs/gotchas/figures.md` |
| Attachments, previews, a file in a reply | `docs/gotchas/files.md` |
| Anything in `src/styles.css` | `docs/gotchas/styles.md` |
| The rail, the crews' column, deleting an agent | `docs/gotchas/workspace.md` |
| The menu bar, the tray, closing the window | `docs/gotchas/menubar.md` |
| A test, a stub, or which suite would catch this | `docs/TESTING.md` |

## Conventions

- Match the surrounding code. Comments explain why, never what.
- Every guard refusal and every error the operator can hit says what happened
  and what to do about it. Both are read by a model or a human under pressure.
- Errors an agent reads mid-turn need a way forward, not just a reason. A
  refusal that only says no gets reworded and retried.
- New behavior needs a test that would fail without it. Failure paths first.
- No dead code, no speculative API surface. The contract test fails on a command
  nothing calls.
- **American spelling, everywhere this repo writes its own words.** Prose,
  comments, test names, identifiers, schema, UI copy, commit messages. `color`,
  `behavior`, `authorize`, `recognize`, `center`, `catalog`. This started as
  British by accident in the first commit and drifted for a year: the account
  subsystem ended up American while the plugin subsystem beside it stayed
  British, and `oauth.rs` disagreed with itself inside one file. There are
  exactly two things the rule does not reach, and both are somebody else's
  spelling rather than a choice: a token named by a language, a spec or a vendor
  keeps whatever they called it (`color-mix`, `grayscale()`, the `Authorization`
  header, `authorization_endpoint`, `TransactionBehavior`), and a list matched
  against what an operator typed carries both spellings, which is why `WORDS` in
  `roles.ts` still holds `localisation`.

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
./scripts/image.sh       # the daemon's image: build it, run it, prove it answers
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

The other suites, one per subsystem with something live behind it, are in
`docs/TESTING.md`: the crew recording, the subscription, the account, the
plugins, the two coding harnesses, the two machine providers, and the two reads
off OpenRouter's catalog. Run the one you touched.
