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
- **`OnDisk::under` is the one place the three directories are arranged.**
  `boot.rs` used to spell two of them itself and the third arrived on `main`
  as a separate argument; a host that built `Workspace` and `FileStore` by
  hand would be pointing part of the runtime at a directory nobody chose.
