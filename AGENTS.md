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
  lib/search.ts       One ranking over hits from SQLite and from the store.
  lib/trail.ts        A turn's own tool calls: what folds into one chip.
  lib/cafeteria.ts    Preset agents, waiting to be hired. Content, not runtime.
  lib/ipc.ts          Every call into Rust.
  lib/prefs.ts        What the operator sets and the runtime never reads.
  lib/appearance.ts   Scale and surface, as one write to the root element.
  lib/notify.ts       When an interruption is warranted. Mostly when it is not.
  lib/announce.ts     What that interruption would say. One event in, one line out.
  lib/keybinds.ts     Every key the app answers to, in one list.
  components/         One file per surface.
src-tauri/src/
  domain/             AgentCard, Envelope, Routine, Connector, Signin, Approval,
                      Search, ids. No I/O.
  runtime/
    guard.rs          The loop guard. Read this one first.
    mod.rs            Agent actors and the message bus.
    prompt.rs         Prompt assembly, including the trust boundary.
    events.rs         Events pushed to the UI.
  llm/                OpenAI-compatible client, SSE decoding, tool definitions.
  db/                 SQLite. Plain SQL, numbered migrations.
  e2b.rs              Computers: the machines agents look at and point at.
  proxy.rs            Loopback viewer for those machines.
  sessions.py         Reports what a machine's Chrome is signed in to.
  kernel.rs           Browsers: a hosted Chrome, which is where the web belongs.
  cdp.rs              The DevTools protocol. Asks a page instead of looking.
  workspace.rs        Per-agent memory: one markdown file the agent rewrites.
  files.rs            Attachments, addressed by the SHA-256 of their contents.
  eval.rs             Reads a run and says whether it communicated sensibly.
  trajectory.rs       Reads a run's events and says whether the machinery did.
  config.rs           Operator settings, and the API key the webview never sees.
  commands.rs         The entire IPC surface.
  app.rs              The only file that knows Tauri exists.
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
| Stopping a conversation: what a stop marks, wakes, and must never release | *A stop marks the run and releases nothing*, then `Runtime::stop_run` |
| Permission prompts, parked turns, acting in the operator's name | *A protected action parks the turn that asked for it* |
| What an agent may do with a page it has just read | *A page that was read this turn cannot quietly press a button* |
| Screenshots, coordinates, what a screen action answers with | *A computer is looked at, never asked* in `docs/MACHINES.md` |
| Attachments, previews, drops, handing a document to the operator | *Files are references, and what a model gets depends on what they are* |
| SQLite, the pool, migrations | *Storage*, and the two comments in `Store::open` |
| Schedules, triggers, what a firing looks like | `docs/ROUTINES.md` |
| Sandboxes, the desktop, the screen, sign-ins on it | `docs/MACHINES.md` |
| Hosted browsers, CDP, `browse`, live view, browser profiles | `docs/BROWSERS.md` |
| Which of the two a piece of work belongs on, and credentials | *Connectors* in `docs/PROTOCOL.md`, then both files above |
| Channels, the rail, search: what the operator sees | `docs/WORKSPACE.md`, then `src/lib/transcript.ts` |
| A turn's tool calls in a channel: what folds, what a chip says, what opens | *A turn's own work is chips* in `docs/WORKSPACE.md`, then `src/lib/trail.ts` |
| Anything announced to a screen reader, or a live region | *A transcript is a log, and says one thing out loud* in `docs/WORKSPACE.md` |
| The rail's order, dragging a row, groups as places you go inside | *The rail is arranged by hand*, *A drop is one call* and *A group is a place you can be inside* in `docs/WORKSPACE.md`, then `src/lib/rail.ts` |
| Preset agents, hiring a crew | *The cafeteria is a copy machine* in `docs/WORKSPACE.md`, then `src/lib/cafeteria.ts` |
| Settings, the surface, the scale, what may interrupt the operator | *Settings is eight places*, *The reading column has two surfaces* and *An interruption has to earn it* in `docs/WORKSPACE.md` |
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
- **`emit_reply` delivers a reply that carries a file and no text.** Handing over
  a document with nothing typed is normal, and judging the reply empty by its
  text alone drops the thing the turn was spent producing.
- **`body_with_files` names a file on an agent's own turns too, not just on
  incoming ones.** An agent that reads its last turn back without the file it
  attached has no record of handing anything over, so it attaches the document
  again and reports it as the first time.
- **Only the component drawing the live bubbles subscribes to `streams`.** One
  level higher, a single token re-renders every message in the transcript.
- **The sign-in tests carry real cookie names.** A cookie's presence is not a
  login. Do not loosen them without a fresh capture from a live machine.
- **All three conditions in `needs_consent` are load-bearing.** Each one alone
  refuses honest work. Read the doc comment before narrowing or widening any.
- **An envelope booked against a run is released by whatever consumes it.** A
  path that takes one without turning it into a turn leaves the run outstanding
  for the life of the process.
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
