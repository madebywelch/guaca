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

## A browser hands a document over as bytes

The desktop's `stage_files` takes a path, because Tauri hands the window the
path of a dropped file and the Rust side reads it: a document never enters
the renderer. A browser is the renderer, and it has bytes rather than a path,
so `POST /v1/upload?name=…` takes one file per request and puts it in the
same store by the same digest. `stageUploads` on the frontend loops and
collects refusals exactly as `stage_files` does, one line per file in the
store's own words, so the composer sees one answer shape from either door.
The drop is DOM events when hosted and Tauri's when not, and `onFileDrop`
hides which; a browser also gets an attach button, because a drop is not the
only way a person has a file.

The body limit on the route is four times the store's, on purpose. Under it a
file the store refuses is refused with the store's sentence, which names the
file, its size and the limit; over it is the framework's bare 413, which
nobody reaches by dropping the wrong thing.

The desktop app pointed at a box is the third case. The drop is still a path
on this disk, and the box has never seen this disk, so `forward_files` reads
the path in this process and posts the bytes to the box's route. Same answer
shape, same sentences.

## A sign-in comes back through the origin the browser used

Every OAuth flow in the app lands its redirect on a loopback port bound
before the redirect is named, which is the argument `oauth.rs` opens with. A
box has no browser at the machine and no port a remote browser could reach,
so on a server the redirect is a route, `/v1/oauth/callback`, on the origin
the operator's browser reached the workspace at.

`Landing` is the seam. `Loopback` binds the port; `Served` files the flow
under its `state` in a map the daemon holds, and names the route on the
origin. Everything after the landing (PKCE, the state, the issuer check, the
exchange) is one path, because `read_answer` is one function and both
waiters call it. The route hands what arrives to the flow waiting on that
state and waits for the flow to say which page to show, so a browser is told
"Connected" by the code that checked the state and the issuer, never by a
route that could only guess. A callback nobody is waiting for is a stale tab
or a guess, and gets a 404.

The origin is the one thing the daemon cannot know at boot: a box is behind
a tunnel whose name is the operator's. `Reach` remembers the origin of the
last call that arrived, read off `X-Forwarded-*` first and `Host` second,
because a proxy terminates TLS and the browser saw the proxy's name.
`GUACA_ORIGIN` overrides it for a box that is called by more than one name.
A sign-in started before either is known is refused with the sentence that
says which to do.

The browser is asked to open the page. `open_url` on a server used to refuse,
which was honest and useless; now it emits `OpenUrl` on the event socket, the
page opens a tab, and draws the same URL as a link in a banner, because a
window opened from an event rather than a click is one a browser may refuse
and a sign-in that opened nothing looks exactly like one that hung. The banner
goes when the flow behind it ends, either way.

Two things this does not settle. A vendor that only accepts loopback or
`https` redirect URIs for a self-registered client refuses a box reached over
plain `http`; behind a tunnel the redirect is `https` and this does not
arise. And the guaca.bot account registered its client with a loopback
redirect: the served redirect has to be registered there too before an
account sign-in from a box completes. `tests/account.rs` drives the flow
against a scripted server that accepts it; the live service is a change on
the other side of that wire.

## The desktop app can show a box, and the menu bar follows

The desktop app is a window over exactly one workspace at a time. Pointed at
a box from Settings, Workspace, it becomes a client of it over the same HTTP
a browser uses: `transport.ts` reads the box's address and token from
storage once, at load, and every call and the event socket go there. Which
host the page is in was decided at import time on purpose, so a change is a
reload rather than a state, and nothing can be half-switched. The address is
probed before it is stored: `/health` for whether it is a Guaca workspace and
which build, `capabilities` for whether the token opens it.

This machine's runtime keeps running underneath, and its crew keeps working,
exactly as it does with the window closed. The pane says so. It is not a
footgun to be designed away: a laptop's crew working while the operator
looks at a box's is the same arrangement as a laptop's crew working while the
operator looks at their email.

The strip follows the window. `tray.rs` reads this machine's runtime, which
is the wrong runtime while the window shows a box, so the window hands the
tray the box's presence instead: `presenceOf` projects the store into the
same `Presence` Rust reads locally, coalesced and only when it would draw
differently, and `report_presence` puts it on the tray. A click on a row
drawn that way belongs to the box, and the window is what holds a connection
to it, so the tray sends the click back down `guac://menubar` and the window
does what the row said. A window showing this machine again tells the tray
so once at boot, because the process outlives the page and the last page may
have left it fed.

The webview's content policy allows `https:` and `wss:` for calls and files,
and loopback `http:` for a container on this machine. Scripts are still
`'self'` only; what this widens is where the page may fetch from, which is
what a client of a box is.

## One image, and the build it says it is

`Dockerfile` builds the daemon without Tauri and the page with the same
`pnpm build` CI runs, and puts both in a `debian:bookworm-slim` with TLS roots
and `curl` for the health check, running as an unprivileged user with
`/var/lib/guaca` as the one volume. `docker-compose.yml` publishes it to
`127.0.0.1` only, and `deploy/guacad.service` is the same daemon under systemd
for a box without a container runtime, with `DynamicUser` and one
`StateDirectory`. The desktop app is not in the image and cannot be, for the
reason `docs/ARCHITECTURE.md` gives under *Why there is no Docker image for the
app*; the daemon is a service, and this is its container.

The image binds every interface *inside the container*, because that is the
only way a published port reaches it, and the container's network namespace
is the boundary the loopback default draws on bare metal. The invitation the
daemon prints knows this: an unspecified bind is printed as `localhost`,
which is right for the operator who published the port to their own machine,
and the line beside it says to substitute a tunnel's address for a box.

The build context has no `.git` in it, on purpose, so the commit is an
argument. `GUACA_COMMIT` reaches `vite.config.ts` for the page's About and
`option_env!` in `server/mod.rs` for `/health`, and `scripts/image.sh` asserts
the container reports the build it was asked for. That string is the
answer to the first question about any bug that reproduces on one machine
and not another: a box and a laptop that disagree about it are running
different code. The desktop reads its commit from the repository it was built
in, so the two hosts answer the same question the same way.

`scripts/image.sh` is the gate. It builds the image, starts it on a random
port, waits for `/health`, and checks the four things a box fails silently:
the build string, a 401 without the token, the server's capabilities with it,
and the page at `/`. Then it stops the container and checks that it stopped.
It needs Docker and nothing else, spends nothing, and is not in `ci.sh`
because `ci.sh` runs inside a container that has no daemon to hand.

```sh
./scripts/image.sh              # build, run, check, remove
KEEP=1 ./scripts/image.sh       # and leave it on http://127.0.0.1:8787
```

## What is not built

Said here rather than discovered. A hosted workspace today is the whole
runtime, every command, the transcript, the desk, the flow board, the
schedule, the compost and the plugins that were signed in before the move.
What it does not have yet:

- **Repositories on a box.** A clone from a remote with a credential, and only
  `pi` to write in it. A different feature, argued above.
- **The account sign-in from a box.** The flow is built and tested against a
  scripted server; guaca.bot has to register the served redirect before the
  live one completes.
- **Pages an agent wrote, and the computer's live screen, on a hosted page.**
  Both are served from loopback ports on the runtime's machine, which in a
  browser is the wrong machine. Each needs a route on the daemon.
- **The page that says how to put a tunnel in front of a box.** The image
  and the unit file exist; what to put in front of them on a rented machine
  is written nowhere yet. `GUACA_BIND` defaults to loopback on purpose: a
  default that binds every interface is one operator's firewall away from a
  public workspace.
