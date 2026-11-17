---
name: verify
description: Prove a Guaca change end to end, the way a person uses it, with no API key and no network. scripts/verify.py starts the real daemon (guacad) and its web bundle from an empty workspace, answers the model from a script on loopback, drives the page in headless Chromium by accessible role and name, and records evidence for every step. Use before calling a user-visible change done, when a scenario fails in CI, and when adding the scenario a new feature ships with.
---

# Verifying Guaca

One command drives the product the way a person does and says whether each
journey that decides a release still holds:

```sh
scripts/verify.py doctor          # is this checkout ready to verify?
scripts/verify.py --build         # build, then run every scenario once
scripts/verify.py reply lapse     # some of them
scripts/verify.py --runs 20 peers # the same scenario twenty times in a row
scripts/verify.py --list          # what each scenario proves
./scripts/ci.sh e2e               # what CI runs: the harness tests, then every scenario
```

Every run starts from nothing: an empty data directory, a free loopback port,
a fresh browser profile. Runs never share state, so a failure is about the
commit and not about the run before it. The result is one JSON document on
stdout; exit status 1 means a run failed, 2 means the checkout is not ready.

`scripts/verify.py --list` is the map: each scenario, what it proves, and the
claims a run must pass to count. A run that skips a claim fails, so the list
cannot drift from what runs. The handles and the traps live in
`scripts/e2e/scenarios.py`, in the helpers each scenario calls.

When you drive or add a scenario:

- Find elements by role and accessible name with `w.ui`.
- Wait on a row, an element or an event, never on a fixed delay.
- Script the model by what was said to it, never by call count.
- Read rows only through `w.host.rows`, which opens the database read only.
- A step the product cannot satisfy fails with the names it did show. Never
  weaken the step to make it pass.

## Launch

```sh
scripts/verify.py build
```

Builds three things, which `doctor` then checks:

| What | Built by | Lives at |
|---|---|---|
| The web bundle the image serves | `pnpm build` | `dist/` |
| The daemon, with the features the image ships | `cargo build --no-default-features --features server --bin guacad` | `src-tauri/target/verify/bin/guacad` |
| The same daemon with the `probe-host` clock | the same, with `server,probe-host` | `src-tauri/target/verify/bin/guacad-clock` |

The daemon is a debug build of the image's source and features. The image
itself is a release build (`scripts/image.sh` proves that one starts). The clock
build differs only in `src-tauri/src/probe_clock.rs`: with
`GUACA_PROBE_MINUTE_MS` set, the ten-minute request window and the outbox's
waits run on a shorter minute. It refuses to compile with optimizations on,
so no shipped Host has it. Only `lapse` uses it.

A run is ready when the Host answers `/health` and the page has exchanged the
invitation's token for a session and drawn the App settings button. That is
usable, not just alive: the schema is migrated and the page is admitted.

Before the first scenario starts its clock, `verify.py` starts Chromium once on
a throwaway profile and waits, for at most three minutes, until it has opened a
tab. A cold browser on a loaded runner can take longer than a scenario's
30-second answer to its first tab, and that start belongs to the run, not to
whichever scenario happens to go first. The summary reports it as
`browserStartSeconds`. A browser that never starts runs no scenario and exits 2,
naming the browser under `missing`.

To drive the product by hand against the same stand-in model:

```sh
scripts/verify.py launch
```

It prints the invitation link, the endpoint, the model and the placeholder key
to type into Connect AI (API key, Custom endpoint). Ctrl-C stops it.

## Doctor

```sh
scripts/verify.py doctor
```

Read only. It reports the commit, whether `dist/` exists and is not a probe-mode
bundle, whether each daemon exists and is newer than every file under
`src-tauri/src`, which Chromium it will use, and the Python version. Anything
not ok names its fix. Run it first whenever a result looks wrong: a stale
daemon is the usual cause of a scenario that fails on code you just fixed.
Set `GUACA_VERIFY_CHROMIUM` to choose a browser binary.

## Drive

A scenario is a function in `scripts/e2e/scenarios.py` taking a `World`
(`scripts/e2e/world.py`). The world holds four things:

- `w.host`: guacad. `start()`, `stop()` (SIGTERM, the way a service manager
  stops it), `kill()` (a crash), `call(name, **args)` (a command over `/v1/call`,
  the route the page uses), `post(path, body, headers)`, and `rows(sql)` /
  `scalar(sql)`, which open the database read only.
- `w.model`: the OpenAI-compatible endpoint on loopback. Set `w.model.script` to
  a function from a `Call` to a `Reply`: `say(text)`, `call(tool, **args)`,
  `refuse(status, text)`, or `held(gate, reply)` for a call that stays in the air
  until the scenario sets the gate. A `Call` reads `speaker`, `last_user`,
  `tool_results()`, `said(text)` and `key`. Returning `None` answers with a fixed
  sentence and records the call as unscripted, which fails the run.
- `w.ui`: the page (`scripts/e2e/browser.py`). `find(role, name)`, `click`,
  `fill` (select all, then type), `choose` (a select, by keyboard), `key`,
  `value`, `names(role)`, `text()`, `snapshot()`. Elements are found in Chromium's
  accessibility tree by role and accessible name, the way a screen reader
  finds them, and pressed with real mouse and keyboard events. A name that
  matches nothing fails listing the names that are shown.
- `w.check(step, holds, observed=..., detail=..., shot=True)`: one recorded step.

Shared journeys in `scenarios.py`: `first_run` (onboarding from an empty
workspace to a first agent that has answered), `write(w, to, text)` (the field),
`new_agent`, `open_agent`, `conversation`, `said`, `between`, `outcomes`,
`settled(w, n)`.

Rules that keep a result a function of the commit alone:

- **Wait on something observable, never on time.** `w.wait(what, check)`
  polls a condition the product exposes: a row, an element, an event. The
  timeout only bounds how long a failure takes to report.
- **A script is a pure function of the request.** Branch on what was said
  (`last_user`, `said`, `tool_results`), never on the system prompt and never on
  a count of earlier calls: two agents run at once, and their calls arrive in
  either order.
- **`observed` holds only what is the same on every run.** No ids, times or
  ports. Those go in `detail`. `identical` in the summary compares
  `observed` across runs, so a value that varies there is a finding.
- **Nothing leaves the machine.** The Host and the browser are pointed at a
  proxy on loopback that refuses every connection and records it. The Host's
  release check to github.com is refused there on every run.
- **Randomness is seeded.** The page's `Math.random` is a seeded generator, and
  the browser runs in UTC with the en-US locale.

### Adding a scenario for a new feature

1. Write the journey as a person would do it, then the failure path: the
   refusal, the denial, the crash, the retry.
2. Find the handles with `scripts/verify.py --keep <scenario>` or by reading the
   `.ax.txt` the failing step captured. Use the names the app already gives
   assistive technology. If an element has none, add an accessible name to the
   component; that is a fix for a screen reader too.
3. Script the model for exactly the calls the journey makes.
4. Assert on what a person sees (`w.ui`), what changed underneath (rows,
   files, messages) and what the page was told (`w.events`).
5. Add it to `SCENARIOS` with one sentence of what it proves, naming the
   failure path after a semicolon, and list every `w.check` it makes as its
   claims.
6. Break the product on purpose and watch the scenario fail for the right
   reason. Then run it `--runs 20` and require 20 passes and `identical: true`.

## Evidence

Each run writes to `src-tauri/target/verify/evidence/<scenario>/run-<n>/`
(`--evidence DIR` moves it):

| File | What it holds |
|---|---|
| `steps.json` | every step: what was checked, whether it held, what was seen |
| `NN-<step>.png`, `NN-<step>.ax.txt` | the screen and the accessibility tree at each step that drew something, and at a failure |
| `model.jsonl`, `model/NNN.json` | each call to the model: who asked, what, the tool results it carried, what it was answered, and the full request |
| `events.jsonl` | every event the Host sent the page over its socket |
| `egress.jsonl`, `browser-egress.jsonl` | every connection the Host or Chromium tried to make off the machine |
| `host.log`, `chromium.log` | the daemon's log across every start, and the browser's |
| `harness-error.txt` | the trace, when the harness itself broke |

The workspace token and any secret a scenario reads off the page (a routine's
webhook secret) are redacted from every file. Screenshots are taken only on
screens that do not show a secret. A proof is the action, the state it left
and the side effect, read from a second place: a reply is in the transcript
the page shows and in the `messages` table, and the model saw the message.

## Cleanup

Each run removes its own scratch directory (`src-tauri/target/verify/scratch/<id>/`)
and stops what it started: the browser, the Host, the stand-in model and the
proxy, by process handle, never by name. The evidence directory is never
touched by teardown; a new run of the same scenario and number replaces it.
A run stopped with Ctrl-C or SIGTERM still tears down. To clear everything:

```sh
rm -rf src-tauri/target/verify/scratch src-tauri/target/verify/evidence
```

## Helpers

| Command | What it does |
|---|---|
| `scripts/verify.py` | build, doctor, run, list, launch, and `mac` |
| `python3 scripts/verify.test.py` | the harness's own tests: no browser, no Host |
| `scripts/verify.py --keep <scenario>` | leaves the world running at its last step, printing the invitation link to open by hand |

## What only a Mac can show

The Linux run covers the daemon and the page, which is every workspace's
runtime and every client's interface. The native shell is not on Linux: the
Tauri window, macOS notification banners and the permission question, and
the menu bar panel. That half is `scripts/probe.py` (`docs/PROBE.md`), reached
through the same tool on a Mac:

```sh
scripts/verify.py mac --list
scripts/verify.py mac --runs 1 panel
```

It builds a test app with a control channel, starts the same daemon on
loopback, and asserts on what macOS logged. It does not run in pull request
CI, which has no Mac. Treat a change to `src-tauri/src/app.rs`, `tray.rs`,
`menubar.rs`, `notifications.rs` or `src/components/MenubarPanel.tsx` as
unverified until the probe has run on a Mac.
