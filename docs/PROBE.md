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
scripts/probe.py --patience 600 click    # give up on a busy Mac after ten minutes
```

It needs a Mac with the toolchain `install.sh` needs (Rust, pnpm) and the
Xcode command line tools, for `swiftc`. Progress goes to stderr; stdout is one
JSON document with every step of every run, `pass` overall, and `identical`
per scenario, which says whether the runs came out step for step the same. The
exit status is 1 if any step failed.

A run that would take the front or act on system UI while someone is using the
Mac is put off, not forced: it is tried again once nobody has touched the
keyboard or pointer for five seconds, for up to `--patience` seconds (half an
hour by default), and `deferrals` in the JSON lists every run that was put
off. Input from a Screen Sharing session counts as someone at the Mac.

While it runs, the command holds one `caffeinate -u -d -i` from before the
first screen step until it exits, so a banner, the question and a capture are
drawn on a lit screen. That turns the display on, declares the user active,
and keeps the display and the Mac awake. Nothing in the Mac's own energy
settings changes.

A display that goes to sleep raises the lock screen's shield about ten seconds
later, even with the password after sleep turned off, and the session then
reads as locked. Waking the display lowers the shield without a password. So a
screen step that finds the session locked first wakes the display and waits up
to ten seconds, and fails with "the screen is locked" only if the session is
still locked after that. Waking the display also resets the count macOS keeps
of any event, so someone at the Mac is read from the keyboard, the pointer and
the scroll wheel alone, and the probe's own wake never puts a run off.

## What a run does

Every run starts from a fresh workspace and removes it afterward. A failed run
keeps only the diagnostics listed below.

1. **Build or reuse the test build.** `pnpm tauri build --debug --bundles app
   --features probe --config src-tauri/tauri.probe.conf.json` makes
   `Guaca Probe.app` with the bundle identifier `com.madebywelch.guac.probe`,
   and `cargo build --no-default-features --features server,probe-host --bin guacad`
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
   `GUACA_PROBE_SOCKET`. It is a background launch (`open -g`), which the app
   leaves in the background (see *launch* below).
5. **Point the window at the Host** through the bridge, and wait until it draws
   the seeded group.
6. **Run the scenario's steps**, each a wait on a condition with a timeout.
   Nothing sleeps to let something happen; the only fixed windows are the ones
   an absence is measured over ("nothing is claimed for three seconds").
7. **Tear down**: capture bounded diagnostics before quitting the app through
   the channel, stop the Host, remove the copy from Launch Services with
   `lsregister -u`, delete every scratch directory. Failed attempts retain only
   the evidence described below.

## Failure evidence

A failed attempt writes `src-tauri/target/probe/failures/<unique-attempt>/evidence.json`
before deleting its scratch profile. Its JSON result includes the directory in
`evidence`, or `null` if evidence could not be written. Successful and deferred
attempts do not retain evidence. `--keep` still leaves the scratch directories.
Retained evidence directories are owner-only, with mode `0700`; files use
`0600`. Each attempt creates a unique directory and never overwrites a file.

Each app quit first captures sanitized state and the main page's
`{"do":"diagnostic"}` response. This includes quits in a scenario's `finally`,
before the attempt catches the failure. The last 16 snapshots stay in memory
until the attempt ends. Each diagnostic request uses a separate connection with
a one-second deadline and a 256 KiB response limit. A missing or stalled channel
does not prevent normal teardown or replace the original failure.

Each launch sets `GUACA_PROBE_TRACE` to its own file in `<scratch.channel>`:
`native.jsonl` for the first launch, then `native-2.jsonl` and so on. The app
opens the file exclusively at startup, so a relaunch could not reuse the first
launch's file, and writes a `trace.opened` row first. Its `hello` reply
carries `trace`, whether that writer is open and has not failed. After
stopping the app, the attempt reads at most 1 MiB and 4096 lines from each
launch's trace and joins the rows in launch order. If no trace is readable,
`native.readable` is false and the row list is empty. A malformed row is skipped.

`traces` gives each launch's `health`. It is `healthy` when the app reported
an open writer and the file begins with `trace.opened`, and `writer failed`
otherwise. `events` counts the rows after the marker. A healthy trace with no
events means the app ran and traced nothing.

Every banner the probe presses is noted with the trace position at the press.
When a press does not do what its step expects, `presses` attributes it from
the rows after that press: `no native response arrived`, `a native response
arrived and no click was emitted`, or `a click was emitted and the page did not
act`. If that launch's trace is not healthy, the press is not attributed. This
adds evidence only, and no step's pass condition or timeout depends on it.

`notification.response` is written for every response the native delegate
hands over, before any filtering. Its `id` is the click number when the
response is for one of this session's banners, and 0 otherwise. Its `value`
holds bit 1 for a default action and bit 2 for this session. A press on a
banner sent without a click identifier, such as the Settings test banner,
leaves a row with `value` 1.

Evidence has a strict allowlist. Native rows retain only `phase`, numeric `id`,
numeric or boolean `value`, numeric `us`, and boolean `main`. Page rows retain
only `phase`, numeric `id`, numeric or boolean `value`, and numeric `ms`, plus
the page's numeric `ms` clock. IDs are opaque nonnegative integers. Numbers
must be finite and no larger in magnitude than an unsigned 64-bit integer.
Native evidence keeps at most 4096 rows; each page snapshot keeps at most 256.
Only these phases are accepted:

- Native: `trace.opened`, `notification.response`, `notification.default`,
  `notification.clicked`, `window.open`, `window.reveal`, `toggle.enter`,
  `toggle.return`, `app.did_hide`, and `toggle.completed`.
- Page: `notification.clicked`, `notification.missing`,
  `notification.remembered`, `reveal.agent`, `select.requested`,
  `select.invalid`, and `select.committed`.

State retains only boolean visibility, focus, and minimization fields for the
fixed `main` and `menubar` windows, and the menu bar's boolean `attached`,
`panel`, and `panelVisible` fields. Evidence never copies app or Host logs,
configuration, databases, session files, credentials, messages, titles,
exception text, or arbitrary payloads.

`python3 scripts/probe.test.py` checks retention, sanitization, and teardown
without launching the app or using the screen. `./scripts/ci.sh scripts`
includes these tests.

## The channel

A request is one line of JSON with an `id` and an `op`; the answer is one line
with the same `id` and either `value` or `error`.

| op | What it does |
|---|---|
| `hello` | The process id and the bundle identifier the build was configured with. |
| `state` | Every window (visible, focused, minimized), the notification permission as `notifications.rs` reads it, and the menu bar: the glyph, title and tooltip as drawn, whether the panel has its host, and whether it is showing. |
| `page` | A call into a window's bridge, below. |
| `clickIcon` | Dispatches `Tray::toggle` on the main thread, at the icon's own place. When closing a visible panel with no visible main window while the app is not already hidden, acknowledgment waits for `NSApplicationDidHideNotification`. Completion is bounded at five seconds, below the existing channel and scenario limits. A queued click whose receiver has closed is canceled before it runs. This is a channel action, not a physical click. |
| `close` | A window's close button: for the main window, hiding it. |
| `hide` | The app hidden, which hands the keyboard to whatever is next. |
| `quit` | The app quits. |
| `refuseNextBanner` | The next banner the window hands to macOS is refused as macOS refuses one, so the window gives back what it claimed. Answered at once. |
| `busy` | The main thread held in 20 ms slices for `ms` milliseconds, answered at once. The window keeps answering between slices, but its run loop is almost never idle, as when it draws a long reply. |

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

### A shorter minute for the scratch Host

A person counts as away two minutes after a window last had focus, and
finished work is gathered for ten minutes before it is offered. A scenario
about being away would wait twelve minutes a run. The scratch Host is built
with a second test feature, `probe-host`, and a scenario that needs it starts
the Host with `GUACA_PROBE_MINUTE_MS`, the length of a minute in milliseconds
(between 1000 and 60000). On that Host both waits run on the shorter minute,
and so does the ten minutes a request is held before it lapses into For you,
which is how `scripts/verify.py lapse` watches a lapse in ten seconds;
nothing else changes, and nothing an agent reads says a different length.

It is held to the same promise as the channel. `probe_clock::scaled` is the
identity in every build without the feature, `probe-host` does not compile
with optimizations on ("the probe-host feature is for test builds only"), and
the unit test in `probe.rs` that asks Cargo whether any other feature turns on
`probe` asks the same of `probe-host`.

## Observers

Each answers a yes-or-no question about what macOS did, independently of what
the app says it did.

- **The unified log**, streamed from before the step to after it and filtered
  to the test bundle's identifier. The lines the scenarios read are
  usernoted's: "Sending request for permission for *bundle*" (the app asked),
  "Presenting `<NotificationRecord app:"com.apple.notificationcenter.askpermissions" … req:"*bundle*">` as alert"
  (macOS put its question on screen), "Removing displayed …" (it took it down),
  and "Presenting `<NotificationRecord app:"*bundle*" …>` as banner".
  That last line is written whether or not anything is drawn, so a banner also
  has to pass usernoted's Do Not Disturb resolution for the bundle: a line with
  "interruptionSuppression: silence" and a `resolutionReason` (a Focus, or
  "display shared" while the screen is shared or mirrored) means macOS kept it
  off the screen, and the step fails with that reason.
- **The front app**, from `NSWorkspace`'s activation notification, so even a
  moment in front is seen (`scripts/probe/system.swift watch-front`).
- **Windows on screen**, from the window server, by process.
- **The signature**, from `codesign`.
- **The Host's outbox**, read only, for whether a delivery was claimed.

`log stream` drops lines when the daemons it watches are busy: usernoted,
sweeping the bundles earlier runs registered, has lost the very line a step
waited for while the persisted log still held it. So the stream answers "seen"
quickly, and only `log show` over the same window answers "not there": a wait
that runs out asks the persisted log before it fails, a count or an absence is
asserted on both, and a log that cannot be read back makes the step unknown,
never absent. A line only the persisted log had is listed in the run's
`streamMissed`, so a run says how often its live observer was wrong.

`scripts/probe/system.swift` is compiled on first use into
`src-tauri/target/probe/system`.

### macOS 26 and 27

The scenarios were run on Apple silicon under macOS 26 and 27. The log lines
above match on both. macOS 27 adds `category:"permission"` and
`dismissAfterDefaultAction:true` inside the record, appends the presented
options to the line ("as banner (["badge", "sound", "alert"])"), and names the
app's path in "Sending request for permission for *bundle* with path …"; the
patterns allow for all three. Two things differ for the screen layer. macOS 27
draws the question's Allow and Don't Allow only under the pointer, and offers
them to assistive technology as named actions on the question's group instead
of as buttons, so `press` takes a button with that name if there is one and
the group's action with that name otherwise. And macOS 27's Notification Center can list
application elements among its own children that lead back to itself, so the
helper does not follow an application or the menu bar below the root, which
keeps a walk of the tree to a fraction of a second.

## Scenarios

| Scenario | What it asserts |
|---|---|
| `launch` | Two background launches of a new bundle with `open -g`, first and second, as every other scenario launches: neither becomes frontmost or takes the keyboard, both are still running after ten seconds, and the app in front stays in front. A normal launch takes the front, and hiding it hands the front back. Before the app stopped asking to be activated when it finished launching, macOS 27 brought the first background launch of a new bundle to the front and focused its window. |
| `permission` | On a bundle macOS has never seen, the app reports "not asked"; with a delivery waiting, connecting asks macOS, macOS shows its question, nothing is claimed while it is unanswered, and macOS takes the question down when the app quits. The probe never answers it. |
| `notification` | Whatever this Mac allows the probe's own bundle, a waiting delivery and the Settings test button do what the Notifications pane says: claimed and presented as a banner when allowed, asked about when never asked, nothing claimed and nothing presented otherwise. Asserted on the log and the outbox, not on the app's word. |
| `panel` | With the window closed, a click on the icon opens the panel, the panel lists every agent in the seeded group, a second click closes it, and the app that had the keyboard before has it after. |
| `panelstable` | With the main window hidden, a channel `clickIcon` closes the visible panel, the probe waits until it is hidden, and another `clickIcon` immediately reopens it. Once reopened with focus, the panel must stay visible and focused for three seconds. This supplements `draft` without changing its assertions or schedule. |
| `allow` | Screen: on a fresh bundle, press Allow on macOS's question; the waiting delivery is claimed and drawn as a banner, the app reports allowed, and the pane says so. |
| `button` | Screen: on a bundle just allowed, the Settings test button raises a banner. |
| `busy` | Screen: with the window's main thread kept busy from before Allow is pressed, the waiting delivery is still drawn as a banner, and a click on it opens its conversation. Before the app handed banners to macOS itself, the plugin dropped a banner whenever the main run loop was not idle at the instant of the hand-off: this scenario failed every run, and `button` two runs in three. |
| `deny` | Screen: on a fresh bundle, press Don't Allow; the app reports denied, the pane says macOS has notifications turned off, nothing is claimed and no banner is presented. |
| `click` | Screen: a delivery about one conversation is shown as a banner; clicking it opens that conversation. |
| `away` | Screen: four deliveries waiting when the window connects, then Allow; all four are claimed and macOS presents exactly one banner, the "While you were away" fold. |
| `capture` | Screen: one image of each of the test build's own windows, the panel open, and nothing else. |
| `longaway` | Screen: at the window once, then Guaca hidden for six real minutes, long enough for WebKit to suspend a page nobody can see; a question and a finished run, on a Host with a six-second minute, are each claimed and drawn as a banner. On a build whose main window lets WebKit suspend its page, both stay unclaimed. |
| `question` | Screen: a run on the loopback stand-in model that asks the operator a question is claimed and drawn as a banner at once, and a click on it opens the conversation. |
| `hidden` | Screen: at the window once, then Guaca hidden; a run that finishes is drawn as a banner, and its click opens the conversation. The Host's minute is 30 seconds here, so the two minutes before away outlast the window's 45 second heartbeat: a window that kept saying it was there would hold the run back as seen. |
| `switched` | Screen: the same, with Finder brought to the front instead of Guaca hidden. |
| `fold` | Screen: two runs that finish while the operator is away are gathered into one banner, and its click opens For you. |
| `testclick` | Screen: a click on the Settings test banner brings Guaca forward. |
| `push` | Every window closed: a question and a finished run each reach a push address on loopback once, with what they are about. |
| `hiddenpush` | Screen: at the window once, then Guaca hidden; a run that finishes reaches the push address once. |
| `refused` | Screen: a question whose banner the system refuses is given back by the window, pushed once although the window is present, and not taken again by that window. |
| `escape` | Screen: with the window closed and Finder in front, a real click on the icon, through the window server, opens the panel and gives it the keyboard; a real Escape closes it, and Finder is in front again. The hand-back is asserted only once the panel had the keyboard. |
| `draft` | Screen: a draft typed to one agent in the panel is not in another agent's box and is still there after going back, and after the panel closes and opens; read through Accessibility, as assistive technology reads it. |

The screen scenarios press system UI found by accessibility role and name, only
inside the group whose text names the test build, and only while nobody has
touched the keyboard or pointer for five seconds; otherwise the step is
reported as deferred rather than forced. They need Accessibility (and, for
`capture`, Screen Recording) for the app that runs the command, and fail with
that reason until it has them. macOS grants both to the responsible process:
the terminal a person runs the command in, or the agent or service that
started it, which is what has to be listed in System Settings, Privacy &
Security. The probe never asks for either. Banners are not drawn while the
display is shared, so the banner scenarios (`allow`, `button`, `busy`, `click`
and `away`) need Screen Sharing disconnected, or "Allow notifications when
mirroring or sharing the display" turned on. And nothing is drawn behind the
lock screen, so every screen step first checks that the session is unlocked,
wakes the display if it is not, and fails saying so if it is still locked: a
Mac that runs the probe unattended needs a logged-in session that asks for no
password when its display sleeps.

## What it cannot reach

- **A person's click is not the channel's click.** `clickIcon` dispatches the
  toggle on the main thread and waits for the hide lifecycle when closing the
  panel would hide the app. It does not synthesize a physical mouse event or
  prove how macOS handles a person's input. `panelstable` checks visibility and
  focus after channel close and reopen. A real click still needs the screen
  layer, which is what `escape` makes.
- **What anything looks like.** The probe reads text, roles and state, never
  pixels. `capture` saves images for a person to look at.
- **macOS's own wording.** The labels `allow` and `deny` press are the ones
  macOS shows in English, "Allow" and "Don’t Allow" with a curly apostrophe.
- **How many times a test bundle is listed.** Each fresh bundle that asks
  about notifications can appear in System Settings, Notifications, until macOS
  forgets the copy. Before every run the probe unregisters each copy Launch
  Services still lists from a probe's folder of copies once it is gone from
  disk, a run killed before its teardown included, and the summary says how
  many (`forgotten`). What usernoted remembers about a bundle and the line
  System Settings shows for it belong to the system; they cannot be removed
  without an administrator, and the probe does not try.

## What shows on screen

A run asks the app to stay hidden, but macOS's own question about
notifications is drawn by Notification Center: `permission` puts it on screen
once per run, for the few seconds until the app quits and macOS takes it down,
and so do `allow`, `deny` and `click` until they answer it. `launch`
brings the test build to the front for a moment in each run, and hides it again
straight away. Every launch, even a background one, puts the test build's icon in
the Dock for the length of the run. No banner is raised unless macOS allows the
test bundle to show one. The delivery the probe seeds says it is a test, and the Settings test
button's banner carries the test build's name, Guaca Probe.
