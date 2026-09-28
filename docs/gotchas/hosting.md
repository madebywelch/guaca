# Hosting

The daemon, a browser as a client, and the boot both hosts share.
`docs/HOSTING.md`, then `server/mod.rs`, `boot.rs`, `ipc.rs` and
`src/lib/transport.ts`.

- **Which host the page is in is read once, on import, and the test setup
  answers "a window".** `src/test-setup.ts` puts `__TAURI_INTERNALS__` on
  `globalThis` so that every suite draws the desktop, which is right for every
  suite but the two about the other host. Those delete the bridge *before*
  the dynamic import of the module under test; a static import runs first and
  reads the bridge that is still there. A hosted test that passes with a
  static import is testing the desktop.
- **Tauri is an optional feature because of one macro, and only one CI step
  compiles without it.** `generate_context!` reads `dist/` at compile time.
  Every target but the daemon step in `ci.sh` is built with the desktop on, so
  a `use` of something Tauri-only outside `app.rs` and `tray.rs` passes clippy,
  passes every test, and breaks the daemon build alone. Run the daemon step
  before calling a Rust change done.
- **A new command is one line in `surface!`, and its arguments have to be
  spelled the way `commands.rs` spells them.** The macro calls
  `commands::$name(&state, $args)`, so a renamed or added argument fails to
  compile in the macro rather than at runtime, which is the point: one rebase
  was caught there three times. Adding the function
  and forgetting the line is caught by `ipc.contract.test.ts` instead.
- **`is_loopback` matched any hostname beginning `127.`** before it parsed the
  octets, so `127.example.com` was refused as a local endpoint. The test in
  `deployment.rs` holds every spelling a model server's console prints and
  the lookalikes that are not loopback.
- **axum 0.7 routes with `:name`, and `{name}` compiles.** The braces are a
  literal path segment under matchit 0.7, so the file route registered, matched
  nothing, and every preview drew nothing. `a_stored_file_is_reachable_by_its_digest`
  is what catches it.
- **The invitation carries the token in the fragment, and the socket carries
  it in the query string, and those are not interchangeable.** The fragment
  never leaves the browser. The query string reaches the daemon and any proxy
  in front of it, and is there only because a WebSocket handshake cannot carry
  a header. Moving the invitation to a query string for symmetry would put the
  token in every access log on the way in.
- **`unauthorized` is one event on the window, not a branch in each caller.**
  A token rotated on the box turns every call away at once. The transport
  raises `UNAUTHORIZED_EVENT` and `TokenEntry` unmounts the app; a caller that
  catches the refusal itself and draws a banner draws forty of them.
- **A refused token is forgotten, not kept.** `TokenEntry` clears storage
  when `capabilities()` refuses the paste. Kept, the next reload would admit
  the page on the strength of a stored token, and the app's own reads would
  fail forty times before the form appeared.
- **Withheld rather than hidden, everywhere a capability is drawn.** The
  Claude row, the loopback presets and the Claude Code harness all stay on
  screen on a server and say why they cannot be chosen. A control that
  vanishes is a pane that disagrees with the operator's laptop and explains
  nothing. The harness reason comes from the box (`withheld` on
  `coding_harnesses`), and the panel used to ignore that field and offer an
  install command for a program the box would not run.
- **A refusal's alternative has to exist.** `Absent::LocalDirectories` once
  said "link the repository by its remote instead" while nothing linked a
  repository by its remote. The build gate checks that a
  refusal offers a way forward; it cannot check that the way forward is
  built. Read the sentence against the feature list before shipping it.
- **The store's default capabilities are a desktop's, on purpose.** Nothing
  draws before `ready`, and a hosted page that read "everything" for one
  frame would offer nothing a desktop does not. Defaulting to "nothing" would
  make every desktop panel flash its refusals on launch.
- **A container has to bind every interface, and the image says so; the
  daemon's own default stays loopback.** `GUACA_BIND=127.0.0.1:8787` inside a
  container is a port nothing outside it can reach, and the symptom is a
  published port that connects and hangs. The image sets `0.0.0.0:8787` and
  the compose file publishes to `127.0.0.1` on the host; the unit file leaves
  the default, because on bare metal loopback is the boundary.
- **The health check spells the port a second time.** `HEALTHCHECK` probes
  `127.0.0.1:8787` because it cannot read `GUACA_BIND`; an override that moves
  the port leaves a container that works and reports unhealthy, or the
  reverse. Move both.
- **`.git` is not in the build context, so the commit is passed in.** Left to
  `builtOn()` alone the page in the image says its version is a dash and
  `/health` says `""`, which is two hosts that cannot be told apart.
  `GUACA_COMMIT` is the argument, `scripts/image.sh` supplies it and asserts
  the container answers with it.
- **A drop hands the composer a promise now, not a list of paths.** Three
  doors (Tauri's paths, DOM files, a path forwarded to a box) end in one
  `Staged`, and `onFileDrop` is where the door is chosen. A caller that reads
  paths off the drop is back to one host.
- **The served landing files a flow by its state and the guard drops it.**
  `Filed` takes the entry out of the map when the flow ends however it ends,
  including a timeout; without it every abandoned sign-in leaves a sender in
  the map and a stale tab can wake a flow that no longer exists.
- **The callback route waits for the flow to name the page.** A route that
  answered "Connected" on its own would say it to a mix-up. The flow runs
  `read_answer`, which is the same function the loopback listener runs, and
  sends the page back over the `Answer`'s reply channel.
- **The origin a sign-in comes back to is the last one seen, unless told.**
  Read off `X-Forwarded-Host` and `X-Forwarded-Proto` before `Host`, because
  a tunnel rewrites both and the browser saw the tunnel's name. A box called
  by two names gets `GUACA_ORIGIN`.
- **`hosted` is always true, so it tells nothing apart.** Every window has
  called a host since the runtime left it. A window and a browser are told
  apart by `desktop`, and a window before setup by `attached()`. The drop
  branched on `hosted` and sent every window down the browser's DOM
  listeners, where Tauri's own drop events arrive and nothing listened; a
  file dropped on the desktop went nowhere. The reveal channel, the drop and
  the menu bar feed are the places a window has something a browser does not.
- **The tray keeps what it was fed across page loads.** The process outlives
  the page, so the icon draws the last report until the next one replaces it,
  and the panel keeps showing the last host until the window reports another.
  The host is compared rather than replaced, so a window reloading onto the
  same host does not reload the panel under somebody typing in it.
- **The contract test counts `invokeLocal` as a caller.** The desktop-only
  commands are reached through it, and a test that only recognizes `invoke`
  reports them as surface nobody uses.
- **A screen's credential is in the path, and the artifact's is in the
  query, and swapping either breaks something.** noVNC resolves `app/ui.js`
  and its socket relative to the page, so a query string is lost by the
  second request; a ticket in the path survives. The artifact is one request
  with no relative loads, and a token in its path would put the workspace
  token where `frame-ancestors` and the address bar can see it.
- **The screen relay strips `referer` and `cookie` before the viewer.** The
  viewer forwards every header it does not rewrite to the machine on the far
  side, and a referer carrying the ticket would hand it to E2B.
- **`screened` rewrites only an address that begins with the viewer's own.**
  A computer with no screen up has no address and is left alone; a desktop
  has no secret and is left alone. The relative address is the page's to
  resolve, because only the page knows which origin it reached.
- **A clone's token lives beside the settings, and only the helper line
  lives in the clone.** `.git/config` is inside a directory a job is pointed
  at and an agent reads; the credential-store file is not. Moving the token
  into the clone's config for convenience hands it to every job.
- **The remote draft has no path, so `path` is `#[serde(default)]`.** Without
  it a remote-only draft is refused as a build mismatch ("missing field
  `path`"), which reads as a version skew rather than the missing attribute
  it is.
- **The clone-removal check canonicalizes the repos directory first.** The
  stored path is canonical because git agreed to it; the configured directory
  can be spelled through a symlink, which on macOS every temporary directory
  is, and comparing the two as spelled leaves every unlinked clone on disk.
- **`ANTHROPIC_API_KEY` is read where the daemon starts, not where the test
  runs.** `Settings.claude_key` is passed rather than read from the
  environment inside the server, so a machine that exports the key does not
  quietly un-withhold the harness in a suite asserting it is withheld.
- **`OnDisk::under` is the one place the three directories are arranged.**
  `boot.rs` used to spell two of them itself and the third arrived on `main`
  as a separate argument; a host that built `Workspace` and `FileStore` by
  hand would be pointing part of the runtime at a directory nobody chose.
- **Every settings write goes through `Runtime::change_config`.** It holds one
  lock from the read to the broadcast. Two clients that each read, patched and
  saved on their own put each other's field back, and the suite only saw it on
  a multi-threaded runtime: on one thread there is no await between the read
  and the save, so nothing can interleave. `settingsChanged` carries the
  redacted settings to every client, the one that made the change included.
- **Save sends what changed, not the form.** The settings pane follows a
  change made elsewhere into every field its operator has not touched, and
  sends only fields that differ from the settings as they stand. The form's
  full `patch` is for a connection test, which has to test what is on screen.
  Sending the whole form saved each window's stale copy over the other's.
- **`streamLagged` is not dead code.** The current host closes a lagging
  socket itself and never sends it, but older hosts do, and a page newer than
  its host has to resynchronize on it. The transport test is named for that.
- **The page id travels in a task-local, not an argument.** A sign-in opens
  its browser tab from deep inside `oauth::authorize`, many calls below the
  command. `CALLER` is scoped around `ipc::dispatch` in the call route, and
  `page_opener` reads it; a command that spawned the sign-in onto another task
  would lose it and fall back to every window.
- **A source build run with `pnpm app` offers a host update it cannot
  download.** With no `GUACA_BACKEND_IMAGE`, `host::IMAGE` names the versioned
  GHCR tag, and a container that `scripts/install.sh` created runs a local
  `guacad:<commit>` image, so the two differ and the update is offered. The
  registry answers the pull with `denied` until that tag is published. Every
  pull failure used to say "check your connection"; `pull_failure` now names
  the refusal and the image. `pnpm app` now builds this checkout's image and
  compiles its name in, which is `scripts/app.sh`.
- **A development build under the installed app's identifier manages the
  installed app's host.** `LocalHost` names its container, its data volume and
  its update journal from the bundle identifier, and `tauri dev` used the one
  in `tauri.conf.json`. On this Mac in a dev window found, started and offered
  to update the operator's own container, and on a Docker context without it,
  would have created one over the operator's old data volume. `pnpm app` passes
  `tauri.dev.conf.json`, whose identifier is `com.madebywelch.guac.dev`.
- **`pnpm app` could not start at all after `guacad` became a second binary.**
  `tauri dev` runs a bare `cargo run --no-default-features` and adds back only
  the default features that do not enable `tauri/custom-protocol`. That was
  inside `desktop`, so `desktop` went with it, and two `[[bin]]` targets with
  no `default-run` gave `cargo run` nothing to choose. `custom-protocol` is its
  own default feature and `default-run` is `guac`.
- **The Docker card in host setup is a stack, not a `.preset`.** It was drawn
  with the provider picker's classes, which lay one line out as a flex row, and
  its status, update prompt, update log and buttons stood side by side in
  columns a word wide. A finished update is not drawn at all; only one with an
  `error` is, and Docker's own buttons appear only while Docker is unusable.
- **A box's host gets its settings from a list, by name.** The updater makes
  `guacad` itself, so a variable the host starts reading reaches a box only if
  it is in `HOST_ENV` in `updater.rs`. `the_compose_host_and_a_box_read_the_same_settings`
  fails when the Compose file hands the host a variable the list does not.
- **The updater reaches the host at its bridge address, not its port.** The
  port is published to the box's own loopback, which a container cannot see.
  Probed through the published port, the updater waits a minute and reports a
  host that never started while the host is answering the tunnel.
- **The socket volume is read-only in the host, and connecting still works.**
  `connect(2)` needs write permission on the socket file, not a writable mount;
  the 0666 mode `bind` sets is what admits the host's unprivileged user.
  `scripts/box.sh` is the only check that runs it on a real kernel.
- **The updater's container runs with `--no-healthcheck`.** The image's check
  probes port 8787, which the updater does not serve, and would mark every
  box's updater unhealthy forever.
- **An update request is answered before the update runs.** The host it came
  through is about to stop. Not answering during an update is the host
  restarting, and the panel draws it that way; drawn as an error, every
  successful update shows a red line for a minute.
- **An answer from before the click also says "not updating."** The panel
  finishes only on an updater read that began after the box accepted
  (`managerAt`), and the monitor never lets an older read overwrite a newer one.
  Without both, a poll in flight when the button was pressed reports the update
  finished before it started.
- **A finished stage in the journal while an update runs is the last update's.**
  The manager records a new operation only once it has checked the target, and
  a box answers the click before that, so for the first second of every update
  the journal said **Host updated** and the panel said so too. `hostProgress`
  takes an unfinished stage as this update's, and **Host updated** only when
  the journal's target is this update's.
- **The progress poll is not a check the operator asked for.** The panel
  refreshes every 1.5 seconds while an update runs, and every refresh set
  **Check for updates** to **Checking…**, so the button flickered for the whole
  update and was the only sign anything was happening. Nor is a host that stops
  answering during an update on this Mac an error: it is stopped for the
  backup, and the red line it drew is kept for when nothing is running.
- **The Docker CLI in the image is 28 on purpose.** It negotiates down to the
  Docker a long-term-support distribution ships and speaks an API above the
  floor the newest daemons require. The newest CLI narrows the first half.

- **A source build and the release it is numbered after are the same
  version.** The version moves only in a release commit, so an app built from
  `main` says 0.2.0 against a 0.2.0 host that lacks every command added since.
  The Skills pane said "different versions; update whichever is older" while
  the Host pane showed 0.2.0 twice and "Compatible", because the message, the
  pane and the notice compared versions only, and the API generation moves
  only for incompatible changes. Each call now names its build as well, and a
  tie in version is broken by commit on both sides: `otherBuild` in
  `releases.ts` and `different_builds` in `ipc.rs`, one rule written twice.
  A host older than the field still gives the generic sentence; the pane
  is the client's and says the truth either way.
- **Main's feed is one file on purpose.** The release manifest and its
  signature are two files because a release never changes. `guaca-main.json`
  is replaced on every push, so a manifest and a signature read as a pair could
  come from two builds and a correct build would be refused as unsigned. The
  envelope is what `feed` reads; do not split it to match the release layout.
- **Main's feed is a branch because releases here are immutable.** A rolling
  prerelease was the first design, and GitHub answered the first upload with
  `Cannot upload assets to an immutable release`. Turning immutability off to
  fix that would unprotect every real release's signed manifest. The feed is
  `main-feed`, appended to by CI and never force-pushed.
- **The key CI holds is not a release key.** `main-keys.pub` and
  `release-keys.pub` are separate lists and `updates.rs` fails the build if
  they share a key. Adding the main key to the release list would let anyone
  who can push a workflow sign a release for every box.
- **A box on `main` is behind by commit, never by version.** Every build of
  `main` says the same version, so `available` compares `health.build` with
  the feed's commit on that channel, and the panel matches a finished update by
  image. Matched by version, an update that never began is reported as the one
  before it.
- **`GUACA_CHANNEL` is the updater's, and the host asks for it.** It is in
  `OWN_ENV`, not `HOST_ENV`. In `HOST_ENV` the host learned it only when its
  container was made, and reinstalling replaces the updater and not the host,
  so a box moved onto `main` kept reporting releases while its updater
  refused every update that named no commit, including the one that would have
  remade the host. `/v1/updates` asks the updater which channel it follows on
  every check. An update request that named a channel would let anything in
  the host move a release box onto `main`.
- **The app rebuild runs in its own process group.** `install.sh` quits the
  app once the bundle is built, and launchd ends whatever is left in a job's
  process group when the job exits (`AbandonProcessGroup` in
  `launchd.plist(5)`). In the app's group the script would be ended by the
  quit it asked for, leaving the old app closed and the new one never installed.
  Only failure is reported: success is the app closing.
- **`install.sh` looks for the app with `pgrep -a`.** macOS `pgrep` and
  `pkill` leave out their own ancestors unless given `-a`, and when the app
  starts the rebuild the app is one. Without it the script found nothing to
  quit, replaced and deleted the bundle under the running app, and `open`
  brought that old process forward instead of the new build. The first file
  picker opened in it afterward aborted the process from inside AppKit.
  From a terminal the app is never an ancestor, which is why the line had
  worked since the script was written.
- **Updates used to keep every backup.** Each is a full copy of the
  workspace, and on a box following `main` the disk would fill in a few
  updates, after which every update fails at its backup step. One is kept now, the
  latest; earlier ones go only after the new one is recorded. Moving that
  removal before the copy would leave a window with no backup at all.
- **A replaced updater is removed with `--volumes`.** The image declares
  `VOLUME /var/lib/guaca`, which the updater never mounts, so each updater
  got an anonymous volume that outlived it. `--volumes` removes only anonymous
  ones; the updater's named state and socket volumes stay.

