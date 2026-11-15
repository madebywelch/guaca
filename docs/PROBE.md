# The probe

Most of what is Mac-only in Guaca happens where the test suites cannot reach:
whether the app takes the keyboard when it starts, whether macOS asks about
notifications and what it does with a banner, where the menu bar panel stands
and who has the keyboard when it closes. The gate runs on Linux and never
compiles that code. The probe is how a machine checks it on a Mac, without a
person and, for everything but the system's own dialogs, without the screen.

It has three parts:

- **A control channel in a test build.** The `probe` Cargo feature adds an
  owner-only Unix socket to the app (`src-tauri/src/probe.rs` and
  `probe/channel.rs`). Through it a script reads state the app already has and
  acts in a window the way a person would.
- **A page bridge** (`src/lib/probe.ts`), which is the page's end of that
  channel: it finds elements by accessible role and name, presses them, types
  into fields and reads what is shown.
- **A command**, `scripts/probe.py`, which builds the test build, runs it
  against a scratch Host, asks macOS what happened, and prints one JSON result.

## Running it

```sh
scripts/probe.py --list                  # the scenarios
scripts/probe.py                         # all of them, three times each
scripts/probe.py launch panel            # some of them
scripts/probe.py --runs 1 permission     # once
scripts/probe.py --no-build notification # reuse the last test build
scripts/probe.py --keep panel            # leave the scratch directories
```

It needs a Mac with the toolchain `install.sh` needs (Rust, pnpm) and the
Xcode command line tools, for `swiftc`. Progress goes to stderr; stdout is one
JSON document with every step of every run, `pass` overall, and `identical`
per scenario, which says whether the runs came out step for step the same. The
exit status is 1 if any step failed.

## What a run does

Every run of every scenario starts from nothing and leaves nothing.

1. **Build or reuse the test build.** `pnpm tauri build --debug --bundles app
   --features probe --config src-tauri/tauri.probe.conf.json` makes
   `Guaca Probe.app` with the bundle identifier `com.madebywelch.guac.probe`,
   and `cargo build --no-default-features --features server --bin guacad`
   makes the daemon. The probe's frontend is built into
   `src-tauri/target/probe/dist`, so `dist/` stays a normal build.
2. **Start a scratch Host.** `guacad` on a free loopback port, with its own
   data directory and a token the probe generates. It is started once to make
   its schema, stopped, seeded with the group `.claude/skills/run-guaca/seed.py`
   writes (four agents and a transcript), optionally given one notification
   waiting for a window, and started again. No Docker.
3. **Copy and sign the app.** The copy is signed ad hoc as a whole, under its
   own bundle identifier: the probe's, or a new one per run
   (`com.madebywelch.guac.probe.fresh…`) when a scenario needs a Mac that has
   never been asked about notifications. The copy lives under
   `src-tauri/target/probe/apps/`, because macOS refuses notification clients
   that run from a temporary directory: it logs "Failed to find or validate
   client" and answers every request with an error.
4. **Launch it in the background** with `open -g -j`, with `HOME` and
   `CFFIXED_USER_HOME` pointed at a scratch directory, so its WebKit storage,
   preferences, caches and application support all land there and nothing is
   written under `~/Library`. The launch names the socket in
   `GUACA_PROBE_SOCKET` and sets `GUACA_PROBE_QUIET`, which stops the app
   activating itself when it finishes launching (see *launch* below for why).
5. **Point the window at the Host** through the bridge, and wait until it draws
   the seeded group.
6. **Run the scenario's steps**, each a wait on a condition with a timeout.
   Nothing sleeps to let something happen; the only fixed windows are the ones
   an absence is measured over ("nothing is claimed for three seconds").
7. **Tear down**: quit the app through the channel, stop the Host, remove the
   copy from Launch Services with `lsregister -u`, delete every scratch
   directory.

## The channel

A request is one line of JSON with an `id` and an `op`; the answer is one line
with the same `id` and either `value` or `error`.

| op | What it does |
|---|---|
| `hello` | The process id and the bundle identifier the build was configured with. |
| `state` | Every window (visible, focused, minimized), the notification permission as `notifications.rs` reads it, and the menu bar: the glyph, title and tooltip as drawn, whether the panel has its host, and whether it is showing. |
| `page` | A call into a window's bridge, below. |
| `clickIcon` | A left click on the menu bar icon, through the same `Tray::toggle` a click runs, at the icon's own place. |
| `close` | A window's close button: for the main window, hiding it. |
| `hide` | The app hidden, which hands the keyboard to whatever is next. |
| `quit` | The app quits. |

The bridge answers `ready`, `view` (which conversation is open and the agents
the store holds), `attach` (save a Host and reload, which is what the host
chooser does), `find`, `click`, `type`, `key` and `text`. Elements are named
by role and accessible name, as a screen reader would read them:
`{"do": "click", "role": "button", "name": "Send a test notification"}`. A name
that matches nothing is answered with the names that are shown; one that
matches more than one says how many. Nothing new is added to the markup for the
probe: it finds what the app already labels for assistive technology, so a
scenario breaks when what a person would see changes and not when a class
name does.

### Why it cannot ship

A control socket in a shipped app would let any process running as the
operator drive it. Four things keep it out:

- The feature is off by default, and `no_feature_but_probe_itself_turns_the_channel_on`
  in `probe.rs` asks Cargo whether any other feature reaches it.
- `probe/channel.rs` does not compile with optimizations on: a release build
  with `--features probe` stops with "the probe feature is for test builds
  only".
- The bridge is imported only when Vite builds in `probe` mode, a constant the
  bundler folds away otherwise, and `./scripts/ci.sh web` fails if the built
  `dist/` contains it.
- Even in a test build the socket opens only when the launch names one, in a
  directory only its owner can enter, readable only by its owner, and every
  connection is asked for its account.

## Observers

Each answers a yes-or-no question about what macOS did, independently of what
the app says it did.

- **The unified log**, streamed from before the step to after it and filtered
  to the test bundle's identifier. The lines the scenarios read are
  usernoted's: "Sending request for permission for *bundle*" (the app asked),
  "Presenting `<NotificationRecord app:"com.apple.notificationcenter.askpermissions" … req:"*bundle*">` as alert"
  (macOS put its question on screen), "Removing displayed …" (it took it down),
  and "Presenting `<NotificationRecord app:"*bundle*" …>` as banner".
- **The front app**, from `NSWorkspace`'s activation notification, so even a
  moment in front is seen (`scripts/probe/system.swift watch-front`).
- **Windows on screen**, from the window server, by process.
- **The signature**, from `codesign`.
- **The Host's outbox**, read only, for whether a delivery was claimed.

`scripts/probe/system.swift` is compiled on first use into
`src-tauri/target/probe/system`.

## Scenarios

| Scenario | What it asserts |
|---|---|
| `launch` | Launched with `open -g` (asked to stay in the background), a new bundle does not become frontmost, takes no keyboard focus and is still running after ten seconds. Also recorded: the same launch with the app's launch-time activation turned off, and a normal foreground launch. |
| `permission` | On a bundle macOS has never seen, the app reports "not asked"; with a delivery waiting, connecting asks macOS, macOS shows its question, nothing is claimed while it is unanswered, and macOS takes the question down when the app quits. The probe never answers it. |
| `notification` | Whatever this Mac allows the probe's own bundle, a waiting delivery and the Settings test button do what the Notifications pane says: claimed and presented as a banner when allowed, asked about when never asked, nothing claimed and nothing presented otherwise. Asserted on the log and the outbox, not on the app's word. |
| `panel` | With the window closed, a click on the icon opens the panel, the panel lists every agent in the seeded group, a second click closes it, and the app that had the keyboard before has it after. |
| `allow`, `deny` | Screen: on a fresh bundle, press Allow (or Don't Allow) on macOS's question; then the waiting delivery is claimed and presented as a banner (or the pane says notifications are off and nothing is claimed). |
| `click` | Screen: a delivery about one conversation is shown as a banner; clicking it opens that conversation. |
| `capture` | Screen: one image of each of the test build's own windows, the panel open, and nothing else. |

The screen scenarios press system UI found by accessibility role and name, only
inside the group whose text names the test build, and only while nobody has
touched the keyboard or pointer for three seconds; otherwise the step is
reported as deferred rather than forced. They need Accessibility (and, for
`capture`, Screen Recording) for the app that runs the command, and fail with
that reason until it has them. The probe never asks for either.

## What it cannot reach

- **A person's click is not the channel's click.** `clickIcon` runs the same
  code as a click on the icon, but macOS decides whether an app may come to the
  front, and it allows that in answer to a person's input. Opened from the
  channel with the window closed, the panel does not take the keyboard, so the
  hand-back the `panel` scenario asserts holds without having been tested
  under load. A real click needs the screen layer.
- **What anything looks like.** The probe reads text, roles and state, never
  pixels. `capture` saves images for a person to look at.
- **macOS's own wording.** The labels `allow` and `deny` press are the ones
  macOS shows in English.
- **How many times a test bundle is listed.** Each fresh bundle that asks
  about notifications can appear in System Settings, Notifications, until macOS
  forgets the copy.

## What shows on screen

A run asks the app to stay hidden, but macOS's own question about
notifications is drawn by Notification Center: `permission` puts it on screen
once per run, for the few seconds until the app quits and macOS takes it down,
and so do `allow`, `deny` and `click` until they answer it. `launch`
brings the test build to the front for a moment in each run, and hides it again
straight away. No banner is raised unless macOS allows the test bundle to show
one. The delivery the probe seeds says it is a test, and the Settings test
button's banner carries the test build's name, Guaca Probe.
