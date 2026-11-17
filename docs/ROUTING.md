# Routing

Which document to read before changing a subsystem, and which gotchas file
lists what has already been broken by changing it.

## Read before you change

| Changing | Read |
|---|---|
| Messaging, replies, cascades, hop limits, the guard | `runtime/guard.rs`, then *Cascades terminate because of one asymmetry* and *The five limits* in `docs/ARCHITECTURE.md` |
| A message that arrives while an agent is already working, and the answer that could not reach the turn that started the work | *A working turn reads its inbox, and only what it can answer where it stands* in `docs/ARCHITECTURE.md`, then `Runtime::take_in` and `Intake` |
| What a turn is told it was asked for: `expects_reply`, `intent`, `ReplyMode` | *Cascades terminate because of one asymmetry*, and `runtime/prompt.rs`, which has to agree with it |
| A turn that ends on "checking now" and does nothing, the round it is given back, the fault that counts them | *A turn ends when the model stops calling tools, so a closing promise is silence* in `docs/gotchas/runtime.md`, then `src-tauri/src/domain/promise.rs` and the `## Your reply` block in `runtime/prompt.rs` |
| Streaming, retries, the budget, when a run settles | *A failed model call is retried*, *A thought is shown and never kept*, *The budget counts model calls* |
| What a finished run ended in: the `outcomes` table, its headline and status, who owns a run, when one is seen, `listOutcomes` and `markOutcomesSeen` | *A run ends in an outcome* in `docs/ARCHITECTURE.md`, then `domain/outcome.rs`, `db/outcomes.rs` and `Store::settle_run_as`, and run `tests/outcomes.rs` |
| Errands: the `errand` tool, who may send one, what an errand may do, what it spends | *An errand is the agent's model again, inside the turn that sent it* in `docs/ARCHITECTURE.md`, then `domain/errand.rs` and `runtime/errand.rs`, and run `tests/errands.rs` |
| What a turn shows of itself while it runs: the thinking, the calls, the line above the composer | *A thought is shown and never kept* and *A turn's own work is watched while it happens*, then `src/lib/reasoning.ts` |
| How a turn is paid for: providers, the ChatGPT sign-in, the Responses API | *A subscription is a second provider, not a second endpoint*, then `llm/codex.rs` and `subscription.rs` |
| Turns paid for by a Claude plan, the `claude` program, the structured answer it is held to | *What the restriction leaves open, which is the program* in `docs/PROTOCOL.md`, then `llm/claude.rs`, and run the live half of `tests/claude.rs` |
| A sign-in that stopped working, refreshing, expiry, signing out | *A token's `exp` is a floor on its life, not a ceiling* in `docs/ARCHITECTURE.md`, then `Subscription::renew` and the 401 path in `codex::stream` |
| What a group decides for itself: provider, models, timeout, limits, and an agent's own model over it | *A group chooses its own provider*, *Nothing about who pays is inferred* and *A run is measured against the limits of the group it happens in*, then `domain/group.rs` and `AgentCard::own_model` |
| Stopping a conversation: what a stop marks, wakes, and must never release | *A stop marks the run and releases nothing*, then `Runtime::stop_run` |
| A host stopping or restarting: the drain, what is put down, what the next host picks up and what it reports interrupted | *A stopping host puts its work down, and the next one picks it up* in `docs/HOSTING.md`, then `runtime/drain.rs` and `db/put_down.rs`, and run the scenarios under *a host that stops* in `tests/cascade.rs` |
| The status bar, a page pinned to it, its condensed view, quick actions, the places at its end | *The status bar holds pages the groups keep, and a click on one is the bar's* in `docs/WORKSPACE.md`, then `domain/widget.rs`, `src/lib/widget.ts` and `src/components/StatusBar.tsx`, and `widget.test.ts`, which holds the widths the `artifact` tool states |
| An agent reading the settings or asking to change them, the operator's current view | *An agent can read the settings, and what the operator is looking at* in `docs/WORKSPACE.md` and *A settings change is a permission with its own diff* in `docs/ATTENTION.md`, then `config::agent_patch` and `Runtime::use_settings` |
| What interrupts the operator: the outbox, its rule, presence, the dispatcher, `pendingDeliveries`, `claimDelivery` and `releaseDelivery`, the `attention` settings | *What interrupts the operator* and *Reminders* in `docs/ATTENTION.md`, then `domain/outbox.rs`, `runtime/outbox.rs` and `db/outbox.rs`, and run `tests/outbox.rs` and `tests/outbox_endpoint.rs` |
| A push to the operator's phone or a webhook: the `delivery` settings, what a push says, ntfy and JSON, retries, `pushReport` and `testPush`, keeping the address and the token secret | *Reaching the operator with every window closed* in `docs/ATTENTION.md`, then `src-tauri/src/push.rs`, the endpoint half of `runtime/outbox.rs` and `db/outbox.rs`, and `DeliveryConfig` in `config.rs`, and run `tests/outbox_endpoint.rs`, which never leaves loopback |
| Permission prompts, parked turns, acting in the operator's name | *A protected action parks the turn that asked for it* |
| Approval allow and block rules | `docs/ATTENTION.md`, then `domain/policy.rs` and `db/policy.rs` |
| A program that is installed and reported missing: `claude` | `src-tauri/src/programs.rs`, whose header says why a double-clicked app does not have the operator's `PATH` |
| A sign-in this Mac already has, the first-run offer, why Codex's token is not imported | *The first run may offer a sign-in that is already there* in `docs/PROTOCOL.md`, then `src-tauri/src/pickup.rs`, `existing_signins` in `commands.rs` and the offer in `OnboardingProvider.tsx` |
| Anything an agent stops to ask a person: the desk, the queue, the two kinds of request, `ask_operator` | `docs/ATTENTION.md`, then `domain/approval.rs` and `Runtime::park` |
| A request nobody answered in ten minutes: the decision it becomes, what the turn is told, a stop or restart that ends it first, the one-time grant an Allow writes, migration 74 | *The ten minutes* in `docs/ATTENTION.md`, then `Runtime::park`, `Store::lapse_approval` and `Store::take_grant`, and run the lapse scenarios in `tests/cascade.rs` and `tests/policies.rs` |
| An agent that cannot go on at all, what reaches the operator without parking a turn, `escalate` | *Three things an agent can do about a person* and *What an escalation is* in `docs/ATTENTION.md`, then `domain/escalation.rs` and `Store::raise_escalation` |
| The box you write in, its recipient, and what will answer | *The composer is bounded and names its recipient* in `docs/WORKSPACE.md`, then `src/components/Composer.tsx` and its styles |
| The app icon, the menu bar glyphs, the drawn wordmark, the README's header | *The icon is a paper tile, and the wordmark is the icon, drawn* in `docs/WORKSPACE.md`, then `scripts/make-icon.py`, `scripts/make-tray.py` and `scripts/make-icons.sh`, which rewrites them, and `scripts/check-icons.py`, which the gate runs |
| The window's rows, the top bar, what the window opens on, where it goes when the open agent is deleted | *The window is a timeline over one column* in `docs/WORKSPACE.md`, then `src/App.tsx`, `src/components/TopBar.tsx` and `bootstrap` and `refreshAgents` in `src/lib/store.ts` |
| The timeline: where a face sits, the pile, the band since you last looked, the now line, upcoming runs and dates, a request that stops waiting, the keys along it | *The timeline puts each agent at a moment of today* in `docs/WORKSPACE.md`, then `src/lib/ribbon.ts`, `src/components/Ribbon.tsx` and `src/components/Timeline.tsx`, and `ribbon.test.ts`, which is the gate |
| The field: who it writes to, Tab answering the waiting question, an `@` at the start, the search row, the recipient picker | *The field writes to someone and leaves you where you are* in `docs/WORKSPACE.md`, then `src/lib/field.ts` and the suite beside it, and `src/components/Field.tsx` |
| Today: what the opening column shows, in what order, and what it says when nothing needs you | *Today is what needs you, then what finished* in `docs/WORKSPACE.md`, then `src/components/Home.tsx` |
| An agent's page: its tabs, which tab opening lands on, the details page under them | *An agent's page is tabs, and opens where the agent is* in `docs/WORKSPACE.md`, then `src/components/ChannelView.tsx`, `src/components/AgentPanel.tsx` and the `agentView` effect in `src/App.tsx` |
| The Groups sheet: what opens it, what it holds, where groups are switched and managed | *The Groups sheet is where groups are switched and managed* in `docs/WORKSPACE.md`, then `src/components/GroupsSheet.tsx` and `src/components/Sidebar.tsx` |
| A phone or a touch screen: the window at touch sizes, choosing a group there | *A phone gets the same single column* in `docs/WORKSPACE.md`, then the touch block in `src/styles.css` and `src/lib/viewport.ts` |
| A keyboard shortcut, the four that work anywhere, the Shortcuts pane | *Every key the app answers to is in one list* in `docs/WORKSPACE.md`, then `src/lib/keybinds.ts` and the handler in `src/App.tsx` |
| The Groups sheet's list as an index: the mark, the name, the state, the group's count, the places at its foot | *The sheet's list is an index, and its foot is the back matter* in `docs/WORKSPACE.md`, then `src/components/Sidebar.tsx` and the rail's rules in `src/styles.css` |
| The groups' column, its badges, how a group names itself, which group the rail is inside | *A group is a place you can be inside* in `docs/WORKSPACE.md`, then `src/lib/presence.ts`, `src/components/GroupRail.tsx` and `src/components/OrbTag.tsx` |
| When the groups' column comes out, how close is close enough, what holds it out | *The column does not stand open* in `docs/WORKSPACE.md`, then `src/lib/reach.ts` and the three boxes in `.grail` |
| Who spoke to whom, what a run cost, where that board lives | *The flow board is analysis, so it is in a group's settings* in `docs/WORKSPACE.md`, then `src/components/GroupActivity.tsx` and `Store::conversation_flow` |
| What an agent may do with a page it has just read, and whether it is asked about it at all | *A page that was read this turn cannot quietly press a button*, then `Consent` in `domain/agent.rs` and `needs_consent` |
| Screenshots, coordinates, what a screen action answers with | *A computer is looked at, never asked* in `docs/MACHINES.md` |
| Attachments, previews, drops, handing a document to the operator | *Files are references, and what a model gets depends on what they are* |
| SQLite, the pool, migrations | *Storage*, and the two comments in `Store::open` |
| Schedules, triggers, what a firing looks like | `docs/ROUTINES.md` |
| The group's calendar, the `calendar` tool, what an occasion is for | `docs/CALENDAR.md`, then `domain/occasion.rs` and `Runtime::keep_calendar` |
| Whether one group can reach another group's dates, and what a refusal may say | *A group's calendar, and one group cannot touch another's* in `docs/CALENDAR.md`, then the WHERE clauses in `Store::update_occasion` and `Store::delete_occasion` |
| A deadline with no time on it, a meeting with a length, one `starts_at` column | *One instant column, two kinds of date* in `docs/CALENDAR.md`, then `occasion::Clean::new`, which is the only place the invariant lives |
| A date an agent wrote, what it is told back, what the prompt says the day is | *A model has no clock* and *Every write is answered with the date that was stored* in `docs/CALENDAR.md`, then `parse_when` and `Occasion::describe` |
| What the calendar panel draws, the group filter, the month it opens on | *What the operator sees is every group, a month at a time* in `docs/CALENDAR.md`, then `src/lib/calendar.ts` and the suite beside it, which is the gate |
| Whether a firing lands on an agent that is already working | *A firing can be skipped, which is not the same as deferred* and *A skipped firing is in the history* in `docs/ROUTINES.md`, then `Activity::is_working` and `Runtime::sweep_schedule` |
| An event that fires a routine: the route, the secret, what the sender is answered, what the body is to the model | *The secret is in a header, and the answer says what became of the event* in `docs/ROUTINES.md`, then `server::events` in `src-tauri/src/server/mod.rs`, `src-tauri/src/webhook.rs` and `Runtime::deliver_event`, and the event route's tests in `tests/cascade.rs`, which run in the server build |
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
| Testing what only a Mac does: the probe, its control channel and page bridge, its scenarios and observers, or the `probe` feature | `docs/PROBE.md`, then `src-tauri/src/probe.rs` and `src/lib/probe.ts`, and run `scripts/probe.py` on a Mac |
| Native notification acceptance, permission readings and clicks | `docs/NOTIFICATION_ACTIONS.md`, then `src-tauri/src/notifications.rs` and `src/lib/transport.ts`, and `docs/gotchas/menubar.md` |
| Running Guaca somewhere other than the operator's machine: the daemon, the token, a browser as the client | `docs/HOSTING.md`, then `server/mod.rs` and `src/lib/transport.ts`, and run `tests/server.rs` under `--no-default-features --features server` |
| Updating a host from the app: the button, the updater on a box, the signed manifest, the restore after a failed update | *Updating a remote or externally managed host* in `docs/UPDATES.md`, then `host.rs` and `updater.rs`, and run `scripts/image.sh` and `scripts/box.sh` |
| What an update shows while it runs: the bar, its steps, a box's host going silent, a rebuild's log | *Updating a host managed by the desktop* in `docs/UPDATES.md`, then `src/lib/updating.ts` and the suite beside it, which reads the step names from `host.rs` and `install.sh` |
| A box that follows `main`, the image CI builds for every push, its feed and key, and the app rebuilding itself after the host | *The main channel* in `docs/UPDATES.md`, then `.github/workflows/main-channel.yml`, `updates::feed` and `rebuild.rs` |
| What a hosted workspace refuses, and why each refusal is a fact about a machine rather than a missing feature | *Resources belong to the backend* in `docs/HOSTING.md`, then `domain/deployment.rs` and every reader of `capabilities` in `src/` |
| A command that works at a desk and fails on a box, or a new command at all | *One list, three readers* in `docs/HOSTING.md`, then the `surface!` block in `src-tauri/src/ipc.rs` and `ipc.contract.test.ts` |
| An invitation, a token that stopped working, the screen a browser sees before the app | *A browser is admitted by a token, and the token arrives by fragment* in `docs/HOSTING.md`, then `src/components/TokenEntry.tsx` and `adoptInvitation` |
| A file dropped or picked in a browser, or dropped on a window that is showing a box | *A browser hands a document over as bytes* in `docs/HOSTING.md`, then `onFileDrop` in `src/lib/ipc.ts`, the upload route in `server/mod.rs` and `forward_files` |
| A plugin or account sign-in from a box: where the redirect lands, the origin it names, the page the browser is shown | *A sign-in comes back through the origin the browser used* in `docs/HOSTING.md`, then `oauth::Landing`, `commands::Reach` and the callback route in `server/mod.rs` |
| A page an agent wrote or a computer's screen, drawn in a browser or a window pointed at a box | *Two loopback origins reach a browser through the daemon* in `docs/HOSTING.md`, then the `artifact` and `screen` routes in `server/mod.rs`, `AppState::screened` and `screenUrl` in `src/lib/files.ts` |
| The window noticing that its host has gone: the banner, the stale transcript, the last-heard line | *A window whose host has gone is not live, and says so* in `docs/WORKSPACE.md`, then `src/lib/link.ts` and the close in `src/lib/transport.ts` |
| The desktop app showing a box, the Host pane, and what the menu bar draws while it does | *The desktop app can show a box, and the menu bar follows* in `docs/HOSTING.md`, then `attached` in `src/lib/transport.ts`, `src/lib/menubar.ts` and `Tray::feed` |
| Which transport a server is spoken to over, and who gets the older one | *It speaks the transport that was replaced* in `docs/PLUGINS.md`, then `mcp::probe` and `sse_exchange` |
| Which agents in a group get a plugin | *Signing in is one decision, and handing it out is another* in `docs/PLUGINS.md`, then `domain/plugin.rs` and `Store::plugin_tools`, which has to agree with `Store::plugin_reach` |
| Which of a plugin's tools which agents may call | *And which of its tools, for which of them, which is a third decision* in `docs/PLUGINS.md`, then `Store::set_plugin_tool` and both readers of `plugin_tool_access` |
| The guaca.bot account: signing in, what it is for, why it is optional | `docs/ACCOUNT.md`, then `account.rs` |
| Channels, the window, search: what the operator sees | `docs/WORKSPACE.md`, then `src/lib/transcript.ts` |
| An agent's character: the drawing, a new one, or why the cut cast has no eyebrows | `docs/CHARACTERS.md`, then `src/avatars/form.ts` |
| A sixth shape, a shape that lost its corners, or why the drop is not bigger | *Five shapes, one weight* in `docs/CHARACTERS.md`, then `src/avatars/silhouette.ts` and the suite beside it, which is the gate |
| A new expression, or what the app reads one from | *Moods* in `docs/CHARACTERS.md`, then `moods.ts`, whose table and `moodFor` are the whole of it |
| Anything that moves on a creature: a look, a blink, a message landing | *The gaze moves the body* in `docs/CHARACTERS.md`, then `AgentAvatar.tsx`, which is the only place the three meet |
| How hard the body follows a look, when it moves against the eyes, an idle rail that will not sit still, or a frame past `FORM.reach` | *Together, not after*, *The body answers a stare, not a glance* and *The outline is bounded whatever it is handed* in `docs/CHARACTERS.md`, then `settle` in `src/avatars/eyes.ts`, `PULL` and `grip` in `form.ts`, and the sixteen-direction suite in `form.test.ts`, which is the gate |
| A lid, a pupil, a shut eye, one lid lower than the other or one eye smaller | *The eye is a ball, a pupil and two lids* and *A mirror can be calm, cross or afraid, but never doubtful* in `docs/CHARACTERS.md`, then `Eye`, `skew` and `PEEK` in `src/avatars/eyes.ts`, and the containment suite in `form.test.ts`, which is the gate |
| The drawn cast: the brush line, drawing on twos, a brow, a mouth, the mark on a head, the operator's choice of cast | *Two casts* in `docs/CHARACTERS.md`, then `src/avatars/drawn.ts` and the suite beside it, which holds every face inside every body, and `Cast` in `src/lib/prefs.ts` |
| The shape a look pulls, or an idle body that moves | *A look pulls a pear, not a bulge* and *An idle body is still* in `docs/CHARACTERS.md`, then the pear block in `bodyPoints` |
| An agent aimed at a peer: how far a look carries, what it does to the eye, why up and down are not the same size | *An aimed look* in `docs/CHARACTERS.md`, then `AIM` and the lids that ride the pupil in `eyesAt` in `src/avatars/eyes.ts`, and the containment suite in `form.test.ts`, which is the gate |
| A group that moves as one animal, a tempo, a phase, `--gait` | *No two of them keep time together* in `docs/CHARACTERS.md`, then `gaitOf` in `src/avatars/clock.ts` and the suite beside it |
| Charts, tables, a page an agent wrote: what a reply can be drawn as | *A reply can be a figure* in `docs/WORKSPACE.md`, then `src/lib/figure.ts` and `src/lib/chart.ts` |
| A box round the part of a reply that needs the operator, and which marker draws which one | *A reply can mark the one part that needs a person* in `docs/WORKSPACE.md`, then `src/lib/callout.ts` |
| A chart's colors, or how many series one may carry | *A chart's colors are the output of a check* in `docs/WORKSPACE.md`, then `src/lib/palette.ts` and the test beside it, which is the gate |
| Running a model's own HTML, or anything about that origin | *A page an agent wrote runs somewhere else* in `docs/WORKSPACE.md`, then `src-tauri/src/artifact.rs` |
| A page a group keeps: the `artifact` tool, owners, versions, the Artifacts dialog, the card in a transcript | *An artifact is a page a group keeps* in `docs/WORKSPACE.md`, then `domain/artifact.rs`, `db/artifacts.rs` and `Runtime::keep_artifact` |
| A kept page's live data, what the operator allows, a click that reaches the owner | *Live data is declared, allowed once, and read with no model* and *A click on a kept page reaches its owner* in `docs/WORKSPACE.md`, then `Runtime::read_artifact`, `Runtime::call_connector`, and `onSend` in `src/components/HtmlArtifact.tsx` |
| A page the operator can work, and what it may hand back | *A page can hand one value back* in `docs/WORKSPACE.md`, then `BRIDGE` in `src-tauri/src/artifact.rs` and `Answering` in `src/components/HtmlArtifact.tsx` |
| A turn's tool calls in a channel: what folds, what a footnote says, what opens | *A turn's own work is a footnote* in `docs/WORKSPACE.md`, then `src/lib/trail.ts` |
| A refusal the runtime wrote for a model, drawn in a transcript, and the words an operator reads instead | *A refusal is read in the operator's words* in `docs/WORKSPACE.md`, then `src/lib/refusal.ts`, whose table `refusal.test.ts` holds against the Rust sources |
| A turn on the page: the margin, the running head, your own turns | *The transcript is a page, and its margin carries when and who* in `docs/WORKSPACE.md`, then `src/components/MessageItem.tsx` and the transcript section of `src/styles.css` |
| What an agent changed about its own memory, and where the version before it came from | *A memory rewrite opens as a diff* in `docs/WORKSPACE.md`, then `Workspace::write` and `src/lib/diff.ts` |
| What an agent currently remembers, and editing it by hand | *An agent's memory is in the panel* in `docs/WORKSPACE.md`, then `src/components/Memory.tsx` and `src-tauri/src/workspace.rs` |
| The details page under an agent's tabs: its head, what a section is framed in, a mark drawn in a control | *The details page is the agent's, and starts with its profile* in `docs/WORKSPACE.md`, then `src/components/AgentPanel.tsx`, `src/lib/profile.ts` and `src/components/Icon.tsx` |
| Skills: what one is, which group can read or write it, the `skill` tool, the index in the prompt | `docs/SKILLS.md`, then `domain/skill.rs` and `skills.rs` |
| The six skills a workspace starts with, and why a deleted one stays deleted | *A workspace starts with six, and is given them once* in `docs/SKILLS.md`, then `skills::STARTERS` and `Skills::offer` |
| Browsing and adding from skills.sh, what an added skill carries, what an agent may read of it | *skills.sh is where the rest come from, and only the operator adds* and *A skill carries files* in `docs/SKILLS.md`, then `skills_sh.rs` and `src/components/SkillDirectory.tsx`, and run the live half of `tests/skills_sh.rs` |
| What agents are told about Guaca itself, and the manual they read | *The manual is a skill, and a test keeps it true* in `docs/SKILLS.md`, then `src-tauri/skills/guaca/SKILL.md` and `src/lib/manual.test.ts`, which is the gate |
| An agent's notebook: what it keeps beyond memory, the `notebook` tool, the index in the prompt | *An agent's notebook is everything else it keeps* in `docs/WORKSPACE.md`, then `src-tauri/src/notebook.rs` |
| Which of the two stores something belongs in, what `note_progress` is for, why one is a file and the other a table | *An agent's memory is what it knows, and its working notes are what it is doing* in `docs/WORKSPACE.md`, then `src-tauri/src/domain/worknote.rs`, whose header is the argument |
| An `@` that names an agent: what resolves, and what it draws in either place | *A mention is one thing, in the box and in the message* in `docs/WORKSPACE.md`, then `src/lib/mentions.ts` and the layer under `Composer`'s textarea |
| A size, a space, a radius, a duration or a shadow, anywhere in the app | *Every length is named* in `docs/INVARIANTS.md`, then the token block at the top of `src/styles.css`, and the closed-set suite in `styles.test.ts`, which is the gate |
| What color a column is, a surface a panel is drawn on, anything about light or dark | *The workspace follows the selected surface* in `docs/WORKSPACE.md`, then the two token blocks in `src/styles.css` and the columns suite in `styles.test.ts` |
| Any color the interface draws in, a theme, the grays, the attention hue, contrast, what may be drawn in amber | *A theme is four choices, and every color in it is solved* in `docs/WORKSPACE.md`, then `src/lib/theme.ts` and `theme.test.ts`, which is the gate. The hexes in `styles.css` are its output, not a place to edit |
| A typeface, the reading or display face, the reading size, a label's case, the smallest size on screen | *Type is one interface face, one reading face, one display face and a reading size* in `docs/WORKSPACE.md`, then the type ladder and the `data-typeface` blocks in `src/styles.css` |
| Anything announced to a screen reader, or a live region | *A transcript is a log, and says one thing out loud* in `docs/WORKSPACE.md` |
| Scrolling a transcript, following the newest line, when the view may move | *A transcript follows the end for whoever is at the end, and nobody else* in `docs/WORKSPACE.md`, then `src/lib/follow.ts` |
| The menu bar: the glyph, the count, the panel under the icon, closing the window | *The menu bar is Guaca with the window shut* in `docs/WORKSPACE.md`, then `src-tauri/src/menubar.rs`, `src-tauri/src/tray.rs` and `src/components/MenubarPanel.tsx`, and the `Presence` check in `ipc.contract.test.ts`, which is the gate between the window and the icon |
| The Groups sheet's corner: the icon's pair, the name that folds away, the one thing it reports | *The corner is the icon's pair* in `docs/WORKSPACE.md`, then `src/lib/brand.ts` and the suite beside it, which is the gate, and `gaze` in `AgentAvatar` |
| The order in the Groups sheet, dragging a row or a group's circle, groups as places you go inside | *The rail is arranged by hand*, *A drop is one call* and *A group is a place you can be inside* in `docs/WORKSPACE.md`, then `src/lib/rail.ts`, `src/lib/orb.ts` and `Store::move_group` |
| The places on the status bar and at the foot of the Groups sheet, and what goes behind the plus | *The places are at the end of the status bar, and at the foot of the sheet* and *Making something is one plus, in the top bar* in `docs/WORKSPACE.md`, then the places in `src/components/StatusBar.tsx`, the foot in `src/components/Sidebar.tsx`, `forYouCount` in `src/lib/store.ts` and `src/components/NewMenu.tsx` |
| Deleting an agent, putting one back, what the thirty days hold | *Deleting an agent is a thirty-day hold* in `docs/WORKSPACE.md`, then `Runtime::discard_agent` and `Runtime::purge_agent`, which are the two halves of what used to be one act |
| Deleting a group, and why a disband does not use the compost | *Deleting a group deletes the agents in it, and the machines they were renting* in `docs/WORKSPACE.md`, then `disband_group` in `src-tauri/src/commands.rs` |
| Preset agents, hiring a group | *The cafeteria is a copy machine* in `docs/WORKSPACE.md`, then `src/lib/cafeteria.ts` |
| Settings, the surface, the scale, the theme, what may interrupt the operator | *Settings is a nav and a pane*, *The workspace follows the selected surface*, *A theme is four choices, and every color in it is solved* and *An interruption has to earn it* in `docs/WORKSPACE.md` |
| What About says this build is, and where that string comes from | *About says which commit it is* in `docs/WORKSPACE.md`, then `src/lib/build.ts` and the `define` in `vite.config.ts` |
| Building, signing, notarizing or publishing a macOS release, and checking its download | `docs/RELEASING.md`, then `.claude/skills/guaca-release/SKILL.md`, which has to agree with it |
| The group editor: what a group overrides and what it inherits | *A group's settings are the app's, with its own answer on top* in `docs/WORKSPACE.md`, then `src/components/GroupEditor.tsx` |
| What model an agent is offered, and how its job is guessed at | *The model field suggests three, and is still a text box* in `docs/WORKSPACE.md`, then `src/lib/roles.ts` and `llm/catalog.rs`, whose twelve use cases have to agree |
| Whether a model can be shown a picture: an attachment, a screen, what an agent is told it is | *What a model can be sent is asked of the endpoint, not assumed* in `docs/ARCHITECTURE.md`, then `llm/modality.rs` and the four places `Modalities` is spent from `Runtime::run_turn` |
| A prompt, or anything that changes how much a group talks | *Three test suites, asking different questions*, then run the live evals |
| What a real group of eight does with one directive, and what is different when you ask twice | *A group is watched rather than asserted*, then `src-tauri/tests/crew.rs` |

Unqualified section names are headings in `docs/ARCHITECTURE.md`. Every row
also has a shorter companion under `docs/gotchas/`, indexed below: the doc
argues the design, the gotchas file lists what has already been broken by
changing it.

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
| Connectors (plugins in the code), MCP, and the OAuth they and the account share | `docs/gotchas/plugins.md` |
| The daemon, the updater beside it on a box, a browser as a client, the boot that opens a workspace | `docs/gotchas/hosting.md` |
| Computers, browsers, sandboxes, sign-ins found on them | `docs/gotchas/machines.md` |
| Schedules, triggers, firings | `docs/gotchas/routines.md` |
| A group's calendar, an occasion, the wall between two groups' dates | `docs/gotchas/calendar.md` |
| Approvals, questions, escalations, the desk | `docs/gotchas/attention.md` |
| An agent's memory, its working notes, and the panels for both | `docs/gotchas/memory.md` |
| Skills, the `skill` tool, the starters, skills.sh, the `guaca` manual | `docs/gotchas/skills.md` |
| A turn drawn while it runs: the bubble, the trail, the thinking | `docs/gotchas/transcript.md` |
| Charts, callouts, and a page an agent wrote | `docs/gotchas/figures.md` |
| A page a group keeps, its owner and its history | `docs/gotchas/artifacts.md` |
| Attachments, previews, a file in a reply | `docs/gotchas/files.md` |
| Anything in `src/styles.css` | `docs/gotchas/styles.md` |
| The Groups sheet, the groups' column, deleting an agent | `docs/gotchas/workspace.md` |
| The menu bar, its panel, the tray, closing the window | `docs/gotchas/menubar.md` |
| A test, a stub, or which suite would catch this | `docs/TESTING.md` |
