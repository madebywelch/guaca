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
  compile in the macro rather than at runtime, which is the point: the rebase
  onto per-agent worktrees was caught there three times. Adding the function
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
- **A refusal's alternative has to exist.** `Absent::LocalDirectories` said
  "link the repository by its remote instead" for as long as the flag has,
  and nothing links a repository by its remote. The build gate checks that a
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
- **`hosted` is true in a window pointed at a box, and `attached()` is how
  the two hosted cases are told apart.** The reveal channel, the drop and the
  menu bar feed are the three places a window still has something a browser
  does not, and each checks `attached()` rather than `hosted`. A fourth that
  checks `hosted` alone is a desktop feature that vanishes when the window
  shows a box.
- **The tray keeps what it was fed across page loads.** The process outlives
  the page. A window that comes back showing this machine sends
  `report_presence(null)` once at boot; without it the strip keeps drawing a
  box nobody is looking at.
- **The contract test counts `invokeLocal` as a caller.** The two desktop-only
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
