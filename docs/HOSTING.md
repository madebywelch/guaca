# Hosting

Guaca on a machine that stays awake. The same runtime, reached over HTTP and a
socket instead of over Tauri's IPC, so a crew keeps working with the operator's
laptop shut. `src-tauri/src/server/mod.rs` is the host, `src-tauri/src/bin/guacad.rs`
starts it, `src/lib/transport.ts` is the other end, and `domain/deployment.rs`
is the line between what a box can do and what only a desk can.

## The runtime runs in two hosts, and neither knows which

The runtime never knew it was inside a window. `docs/ARCHITECTURE.md` opens
with the reason: agents are `tokio` tasks, the webview is a view over them, and
the API key never crosses into it. What did know was `commands.rs`, which took
Tauri's `State` in every signature and wore `#[tauri::command]` on every
function. Hosting is the act of taking that knowledge out.

So `commands.rs` takes `&AppState` and nothing else, `boot.rs` opens a
workspace for whichever host asks, and there are two hosts over the top:
`app.rs`, which puts the runtime behind a window, and `server/mod.rs`, which
puts it behind a socket. Everything between them is one library. That is the
design rather than a coincidence, and the whole suite is the evidence: every
Rust suite and every frontend test passed unmodified when the second host was
added, because nothing they exercise learned which host it was in.

Tauri is an optional feature of the crate for a reason that is mechanical
rather than tidy. `app.rs` expands `tauri::generate_context!`, which reads
`dist/` at compile time, so a daemon that needed the desktop feature could not
be built in a container that has no frontend. `--no-default-features
--features server` is what `ci.sh` builds and tests the daemon with, and it is
the only thing that does: every other target is compiled with the desktop on.

## Two, not three

An operator chooses between three things: run it here, run it on a box they
rent, or let guaca.ai hand them one. The runtime only ever sees two, because
the difference between the last two is who pressed the button at the provider.
`Deployment` is `Desktop` or `Server`, and there is no third variant.

That is what makes bring-your-own-box free rather than a second product. A
managed box and an operator's own box run the same binary, hold the same state
and refuse the same things, and nothing below `Deployment` can tell them apart
or has any business trying. Whoever provisioned it is a fact about the bill,
and the bill is not the runtime's subject. If a change needs a third variant,
something below that line has started caring who paid, and that is the thing
to undo.

## One list, three readers

The command surface is 102 names, and three things have to agree on it: the
Tauri wrappers, the HTTP dispatch and the list `ipc.contract.test.ts` compares
against the TypeScript side. Three hand-kept lists drift, and the drift is a
panel that works at a desk and fails on a box with nothing on screen saying
which half is gone.

`ipc.rs` writes the list once. The `surface!` macro takes each name with its
arguments and its declared return type, and generates all three readers from
it. The dispatch annotates the return type, so the list cannot claim a shape
the implementation does not have: a command whose signature changes fails to
compile in the macro rather than at runtime in a browser. That is what caught
the rebase onto per-agent worktrees, which had added an argument to
`update_repository` and renamed two others.

## Opening a workspace is one act

`boot.rs` opens the database, expires the approvals nothing can answer any
more, loads the settings, builds the runtime, starts every agent, the
scheduler, the sign-in sweep and the compost, and brings up the viewer and the
artifact origin. Both hosts call it and neither has a copy of its own.

Two copies would drift in the worst way there is: a loop started in one host
and forgotten in the other is a workspace where routines never fire and
nothing reports it. On a server that is a crew that has been silent for a week
before anybody notices, because nobody was at the desk to notice.

The permission expiry deserves a sentence. A parked turn is answered by a task
holding the line for it, and nothing holds a line across a restart, so a
request still pending at boot is waiting on an agent that no longer exists and
is closed rather than left drawing live buttons. On a desktop that is the rare
case. On a server it stops being rare: a container is recycled, a host is
drained, a deploy happens, and every one of those lands here.

## Five capabilities, and none of them is a feature nobody finished

A hosted workspace refuses five things, and every one of them is something
that is *on the operator's machine* rather than something Guaca chose not to
implement:

- **A directory the operator picked.** A repository is a directory, and the
  box's directories are not the operator's. `docs/CODING.md` is explicit that
  seeing the branch you are on and the change you have not committed is the
  point of the local version, and that does not survive the move.
- **A model server on loopback.** LM Studio and Ollama are two clicks on a
  desktop. `localhost` typed into a hosted workspace is the box talking to
  itself, and a tunnel that let it reach the laptop would put "is your laptop
  awake" back in front of exactly the turns that use it.
- **A Claude plan as the provider.** `Provider::Claude` works by being the
  program: `claude` runs where the operator signed in, so the credential never
  leaves the program it was issued to. There is no version of that which ships
  the credential to a box. `docs/PROTOCOL.md`.
- **Claude Code as the harness.** The same fact one level down.
- **A path on the operator's disk.** Attachments named by path, and a saved
  copy landing in the downloads folder. On a server both become the browser's
  own upload and download; the capability is gone and the ability to hand a
  document over is not.

The list is meant to stay this short. A sixth flag has to be something that is
physically on the operator's machine, not a thing somebody has not got round
to. And each refusal names an alternative, because a refusal that only says no
gets reworded and retried by a model and reported as a bug by a person:
`every_refusal_says_what_to_do_instead` in `deployment.rs` fails the build on
one that does not. The alternative has to be something that exists. The
directories refusal once promised linking by remote, which is not built, and
an operator sent to look for it found nothing.

`Capabilities` is a struct of flags rather than `matches!` calls scattered
through the panels, and it is read in two places. The Rust side refuses in the
command, before anything is spent: `save_file`, `create_repository`, and the
settings patch that would store a loopback endpoint each call `require`. The
frontend reads the same struct once at boot, into the store, and draws the
refusal *on the row, before the field is filled in*. Withheld rather than
hidden: a preset that vanishes on a server is a pane that disagrees with the
operator's laptop and explains nothing, so LM Studio stays on the list saying
"Not from a server", the Claude row says "Not on a server", and Claude Code's
harness row carries the reason it is withheld instead of an install command
that leads nowhere. `coding_harnesses` reports that reason from the box, which
is the one capability read at a moment other than boot, because the row it
lands on already asks the machine what is installed.

## A browser is admitted by a token, and the token arrives by fragment

A hosted workspace holds inference keys, plugin refresh tokens and every
transcript a crew has written, so there is no anonymous mode and no read-only
mode: a caller is the operator or it is nobody. One bearer token per
workspace, compared in constant time on every route but `/health`, generated
on the first run and written beside the settings with mode 0600 so a first run
needs nothing prepared.

The token is not in `config.json`. That file is rewritten wholesale whenever
the operator presses Save, and a credential in it would be one a settings
change could drop. The same argument `subscription.rs` makes.

How a browser gets it is the seam most operators never see. The daemon prints
an invitation on every start, `http://addr:port/#token=…`, and `TokenEntry`
takes the token out of the fragment before the first render decides anything,
stores it, and puts the address back without it. A fragment because it is the
one part of a URL a browser never sends: not to the daemon, not to a proxy in
front of it, and not to whatever logs either keeps. The socket takes the token
in a query string because a WebSocket handshake cannot carry a header, and
that is a real cost (a query string reaches proxy logs), which is why the token
is per-workspace and rotatable rather than derived from anything longer-lived.

The form is for the other cases: a token rotated on the box, a browser whose
storage was cleared, an address typed by hand. It checks a pasted token with
`capabilities()`, the cheapest call there is and the one the app reads first
anyway, so a wrong token is refused on that screen rather than forty times by
the reads the app would make. A token the box stops accepting mid-session is
one event on the window, raised by the transport the moment any call comes
back `unauthorized`; `TokenEntry` unmounts the app and asks again, because
every read the app would make is the same refusal.

On a desktop `TokenEntry` renders its children and nothing else. The runtime
is inside the process asking, and there is no token to have.

## One bundle, and the host is read at runtime

The daemon serves the same `dist/` the desktop app embeds, and which host the
page is in is read from `window` when the module loads: Tauri puts
`__TAURI_INTERNALS__` there before any of this code runs, and its absence means
a browser, which means a daemon on the other end of the origin the page came
from. A build-time flag would mean two bundles, and the second would be the
one nobody runs the suite against.

The shapes are the same because Tauri's IPC already was "a name, some named
arguments, and a value or a structured error" plus one event channel. Those
are a POST and a WebSocket. Nothing above `ipc::dispatch` learns which
arrived, and `ipc.contract.test.ts` fails the build if the two transports ever
answer to different sets of names.

The socket reconnects, with backoff to a ceiling, because a box is reached
across a network and a network drops. Reconnecting is not resynchronizing:
events missed while it was down are gone, exactly as they are while the
desktop app is closed, and the answer is the same in both. What the UI draws
it refetches, which is what `onReconnect` is for. Losing events is also the
correct failure on the other end: `SocketSink` drops what no client is
attached to read, because a runtime that blocked on a slow socket would turn
one bad connection into a crew that stops thinking.

## What is not built

Said here rather than discovered. A hosted workspace today is the whole
runtime, every command, the transcript, the desk, the flow board, the
schedule, the compost and the plugins that were signed in before the move.
What it does not have yet:

- **Attachments by upload.** The read side exists (`fileUrl` fetches by
  digest over the route) and the write side does not: a browser has bytes to
  hand over rather than a path, which is a different command.
- **Repositories on a box.** A clone from a remote with a credential, and only
  `pi` to write in it. A different feature, argued above.
- **Plugin sign-in from a box.** OAuth redirects to loopback today; on a
  server the redirect has to come back through the served origin.
- **The desktop app as a client of a remote box.** A browser is already one.
- **Packaging.** A container image and a unit file for `guacad`, and the page
  that says how to put a tunnel in front of it. `GUACA_BIND` defaults to
  loopback on purpose: a default that binds every interface is one operator's
  firewall away from a public workspace.
