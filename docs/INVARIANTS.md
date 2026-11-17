# Invariants

What holds wherever you are working in this repository, whichever subsystem
you are in. Rules that belong to one subsystem are in its own file, found
through `docs/ROUTING.md`.

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

**A Claude subscription pays for a turn by being the program, never by holding
its token.** Anthropic restricts consumer OAuth tokens to Claude Code and
Claude.ai, enforced server-side since January 2026 and explicit in its terms
since February 2026, so an Anthropic sign-in in this app is not implemented and
will not be: there is no field for one and nothing in `llm/claude.rs` that could
carry it. What the restriction leaves open is the program. `Provider::Claude`
runs `claude` once per model call and reads its stdout, so the credential never
leaves the program it was issued to, and the operator signs in where they already
did. Dates and sources: `docs/PROTOCOL.md`.

Guaca keeps its own round loop on that provider, and that is the load-bearing
half. The program is an agent harness and would happily run its own rounds, which
would move `max_tool_rounds`, `reserve_step` and every stop check inside a process
this app does not control. So it is given no tools at all and asked for one
structured answer per call through `--json-schema`, built from the turn's own
`ToolSpec`s as a discriminated union: what comes back is a thing to say and a list
of calls, the runtime dispatches them exactly as it does for the other two, and
`runtime/mod.rs` never learns there was a third. A turn cannot hand its loop
over: it is the unit the five limits are written in.

**Connector, tool, skill, secret: four words, and the code's older name for
two of them.** A connector is an MCP server a group signs in to; the code calls
it a plugin (`domain/plugin.rs`, `mcp.rs`, the `*_plugin*` commands, the
`plugins` table), because those names are the wire contract with a desktop on
an older release. A tool is one function an agent calls, a connector's or
Guaca's own. A skill is a markdown document of instructions (`docs/SKILLS.md`).
A secret is a pasted credential handed to chosen agents as an environment
variable; the code calls it a `Connector` (`domain/connector.rs`), which
predates the other meaning. Say connector, tool, skill and secret wherever a
person or a model reads it.

**A calendar records and a routine fires, and they are two tables because of
that one word.** Both are lists an agent keeps and a model reads them as near
synonyms, so the split is enforced in the design rather than in wording alone: a
`routines` row owns a moment the scheduler sweeps and reaches the agent as a
message, and an `occasions` row owns a moment nothing sweeps. Give the calendar
a trigger and every note about a customer's schedule becomes an agent running at
3am; drop the calendar and every deadline an agent learns becomes a wake-up.
`docs/CALENDAR.md`.

It is also not the Google plugin, which reads the operator's real calendar and
can genuinely book things. Nothing here leaves the machine, and the word
"calendar" carries the opposite assumption strongly enough that *books nothing,
invites nobody* is said in the tool description, the prompt section and the
panel's own footer. A model wrote an occasion and told the operator the call was
booked, which is what all three are for.

**There is one runtime, and it runs in the host.** Every workspace is a
`guacad` daemon, in a container on the operator's Mac or on a box that stays
awake, and `server/mod.rs` is the only place a `Runtime` is built. The desktop
window, its menu bar panel and a browser are clients of that host over the same
HTTP commands and event socket. `app.rs` holds only what is about the
operator's own machine (the window, the menu bar, notifications, the local
host's container, reading a dropped file) and never runs an agent: agent logic
written there is in the wrong half. `boot.rs` opens the workspace, `ipc.rs`
writes the command surface once, and `domain/deployment.rs` is the only place
what a host can do is decided: four capabilities, refused in the command before
anything is spent and drawn on the row before the field is filled in. A managed box and an operator's own box are the same variant, and a
third would mean something below that line had started caring who paid.
`docs/HOSTING.md`.

**A creature is a shape, not a drawing.** Every agent is cut from one of five
silhouettes (circle, octagon, square, water drop, cloud) and recomputed every
frame from that shape, a character (a row of numbers) and a mood (another row).
No path is ever drawn by hand and no transform is ever put on the drawing: a
character that slides around inside its own box reads as a sprite being moved,
and one whose outline changes reads as a thing that is alive. The body only
breathes, leans and settles, because a body that acts as hard as a face is a
body nobody can read a face on; what acts is two eyes, each a ball with a pupil
and two lids. Four casts of hand-drawn characters preceded this and every one of
them failed the same way. `docs/CHARACTERS.md`.

There are two casts of these creatures, cut from paper or drawn with a brush,
and the operator picks one. Both are drawn from the same body, the same moods
and the same gaze, decided once a frame in `AgentAvatar`; what differs is only
how that is put on screen. Keep it that way: a decision made in one cast and not
the other is a mood that means two things.

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
not enter a prompt, a transcript or an event, and there is no field on the types
that cross those boundaries for one to arrive in: a credential is held in
`secrets::Secret`, which cannot be serialized or displayed and leaves only
through `expose()`. Five values reach the page on purpose: last-four hints, the
local daemon token, the menu bar host token, the subscription poll handle, and
the webhook secret on request. The webhook secret crosses only through
`reveal_webhook_secret`, when the operator asks to see it, and never in a state
payload or an event; `tests/server.rs` holds both that and this list. Keep it
that way: `docs/MACHINES.md`.

**A computer and a browser are two places, not two views of one.** A computer is
an E2B machine with a screen, worked by looking and pointing (`use_screen`). A
browser is a hosted Chrome, worked by asking the page (`browse`). Different
providers, unrelated cookie jars, separate sign-ins, either configurable without
the other. Anything that describes one to a model has to disclaim the other, or
the model takes a screenshot to see what `browse` did.

**A comment, a suppression or a new prose file needs the owner's grant.** A
comment that excuses a workaround, a document that records a gotcha instead of
making it impossible, and a suppression that silences a check are the same
move, so `scripts/check-grants.mjs` refuses all three in what a change adds or
modifies: every kind of comment, docstrings included; `#[allow]`, `#[expect]`,
`#[ignore]`, `@ts-ignore`, `@ts-expect-error`, `biome-ignore`, lint-disable
lines, `noqa`, `type: ignore`, `.skip`, `.only` and `.todo`; a sleep standing in
for a check in a test or script; and any new prose file (`.md`, `.mdx`,
`.markdown`, `.rst`, `.adoc`, `.txt`, `.org`, an HTML page or a license). A
comment fails wherever it is in the tree, touched or not, except in a prose file,
where an old one stays until its line is touched, and in a shipped migration the
ledger lists under `migrations`, which must stay byte-identical to the commit it
names because a migration is never edited. A suppression already in the tree
stays until its line is touched. `node scripts/check-grants.mjs strip` removes
every comment the lint finds and formats what it leaves with rustfmt and Biome,
for a branch that brought old comments along. The lint has to read every
file a change touches, so a text file that is neither code it has a reader for,
prose, nor data on the lint's own list fails until it gets a reader or a place on
that list with its reason. A poll is not a sleep: a loop whose bound the lint can
see (a counted `for`, or one that reads the clock) may wait between attempts when
it re-checks its condition each attempt, through its own non-constant condition
or an exit under a condition (an `if`, a `match` arm, a `case`, a `catch` or an
`&&`); an exit at the top of the loop body checks nothing and does not count.
Lines a tool reads and that silence nothing pass by rule
(shebangs, `# syntax=`, `/// <reference>`, `@vitest-environment`, SPDX
identifiers, `# shellcheck source=` and the version beside a pinned action); a
`SAFETY:` comment is a comment like any other. Everything else needs the
owner, and only the owner, recorded in `grants.json` by path and a hash of the
exact text, so an unrelated edit keeps a grant and a reworded comment loses it,
and a grant that matches nothing fails. A change to the ledger fails the check
until the owner approves that exact ledger: the approval is the pull request
label `grants-approved-` followed by the first 12 hex digits of the SHA-256 of
what `grants.json` says (parsed, with every object's keys in sorted order and
no whitespace), which lives outside the diff, so any later change to what the
ledger says needs a new one, a change to its formatting alone does not, and the
author never approves their own. `node scripts/check-grants.mjs hash` prints
the label. A push to `main`
prints the change and passes, since `main` only takes a pull request whose
gate was green. A pull request is judged a second time by `main`'s copy of the
lint, because one that edits the lint or its workflow would otherwise be judged
by its own edit: `.github/workflows/judge.yml` runs on `pull_request_target`,
so the workflow is the one on `main`, checks the head out beside `main` as
data, and runs only `main`'s scripts, `node scripts/check-grants.mjs judge`,
with a token that can only read and no secret. The judge also fails a pull
request that changes the judge (the lint, `scripts/grants/` or anything under
`.github/workflows`) until the owner approves that judge with the label
`judge-approved-` followed by the first 12 hex digits of the SHA-256 of
`git ls-tree -r <head> -- scripts/check-grants.mjs scripts/grants .github/workflows`. The error says what to do instead: encode a constraint as a
type, an assertion or a test, put the why in the commit message, fix the cause
of a workaround, and put a document's knowledge where it is enforced.

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
