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

## Resources belong to the backend

A hosted workspace resolves repository paths and model addresses on the
backend. It can clone a remote, use a mounted working directory, or call a
model service reachable from its own network. The repository form names the
choice explicitly. `localhost` means the backend; for a model on a Docker
host, use `host.docker.internal` (the compose file supplies the Linux mapping).
A model running on a sleeping laptop will still stop answering, even when
Guaca itself runs on a VPS.

The backend may run the official Codex and Claude CLIs under its own user.
Guaca does not import the laptop's credentials or provide a Claude.ai login
flow. Configure the CLI on the backend, as described under **Coding inside
the container** below. This applies to an operator-controlled workspace;
offering consumer subscription routing as a managed service is a separate
product and policy question.

This client-local capability remains unavailable:

- **A path on the operator's disk.** Attachments named by path, and a saved
  copy landing in the downloads folder. On a server both become the browser's
  own upload and download; the capability is gone and the ability to hand a
  document over is not.

Each refusal names an alternative, because a refusal that only says no
gets reworded and retried by a model and reported as a bug by a person:
`every_refusal_says_what_to_do_instead` in `deployment.rs` fails the build on
one that does not.

`Capabilities` is read by the command boundary and by the frontend. Repository
paths and local model endpoints are available in both hosts, interpreted on
the backend. File paths on the client and desktop subscription credentials
remain unavailable on the server. The Claude provider row explains that
limitation. `coding_harnesses` separately checks installed programs and offers
Claude Code on a server with `ANTHROPIC_API_KEY`; its row explains the required
credential when absent.

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

## A repository arrives on a box as a clone of a remote

A desktop repository is a directory the operator picked: their own checkout,
their branch, their uncommitted change, which `docs/CODING.md` says is the
point of the local version. A box has no directory anybody could pick, so a
repository is linked by its remote instead, and the workspace clones it into
a directory of its own under `data/repos/`. Everything downstream is the
code that already existed: the same worktrees, the same `shell` and `code`
doors, the same push gate, against a clone whose work comes back as branches
and pushes rather than as a tree the operator is sitting in.

The clone carries three local config lines, each for a failure that would
otherwise be silent. A `credential.helper` pointing at a file beside the
settings, so a fetch or a push from any process standing in the tree (a
job's harness included) finds the token without the token ever entering
`.git/config` or a URL; the file is git's own credential-store format, mode
0600, named for the clone's directory, and it goes when the repository does.
And an identity, `guaca <guaca@localhost>`, because a box has no
operator-level git config and a harness that cannot commit reports a broken
repository rather than a missing name.

A token goes with an https remote and is refused for an ssh one, which is
reached with a key the box holds. Unlinking a clone removes it, clone and
credential both: they were the workspace's, not the operator's, and the
check is the clone living under `repos/` rather than the row's say-so.

A repository offers Codex, Claude Code and pi on either host. Availability is
reported by the installed program, not inferred from an API key. The image
ships all three plus `git` and `gh`.

## Two loopback origins reach a browser through the daemon

The desktop serves two things from loopback ports of their own: a page an
agent wrote (`artifact.rs`, an origin so the page's script may run under a
policy of its own) and a computer's live screen (`proxy.rs`, a relay that
attaches the sandbox token the webview must never hold). In a browser, or in
a window pointed at a box, `127.0.0.1` is the wrong machine, and both drew
a blank rectangle that passed every test. Each is now also a route on the
daemon, in front of the same loopback server.

A page is the simpler one. `/v1/artifact/{id}` answers from `page_for`,
which the loopback server answers from too, with every header
`artifact.rs` argues for. The token is in the query, the trade the file
route makes, because a frame cannot carry a header. `frame-ancestors 'self'`
now names the daemon's origin, which is what frames it, and the frame's
`sandbox` keeps the page's origin opaque exactly as before.

The screen is a relay one hop out from the relay. `/v1/screen/{ticket}/
{sandbox}/{port}/…` rewrites the head and copies everything after it, the
shape `proxy.rs` has; an upgrade is taken over from hyper once the handshake
is answered and spliced with the viewer's socket, which is how noVNC's RFB
transport gets through, and an ordinary request is read to the end and
answered whole. The credential is a path segment rather than a query,
because noVNC resolves its own scripts and its socket relative to the page
it was served from and a query string does not survive that. It is a ticket
for that one sandbox, `sha256(token ":" sandbox)`, checkable without being
stored and worth nothing but the right to watch that screen through this
daemon; `referer` and `cookie` are dropped on the way in so it never reaches
the machine on the far side. `AppState::screened` rewrites a computer's
address to the route on a server and leaves it alone on a desktop, and the
page resolves the relative address against the origin it reached.

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

- **The account sign-in from a box.** The flow is built and tested against a
  scripted server; guaca.bot has to register the served redirect before the
  live one completes.
- **The page that says how to put a tunnel in front of a box.** The image
  and the unit file exist; what to put in front of them on a rented machine
  is written nowhere yet. `GUACA_BIND` defaults to loopback on purpose: a
  default that binds every interface is one operator's firewall away from a
  public workspace.

## Browser isolation and desktop access

The daemon admits cross-origin requests from the packaged Tauri origins and
local development origins on port 1420. The workspace token is still required;
opaque origins and unrelated sites receive no CORS access to workspace APIs.

Screen documents carry `sandbox allow-scripts` in their response policy as
well as on the hosted iframe. This keeps sandbox-served code away from the
workspace's DOM and storage even when its URL is opened directly. Screen
assets allow CORS so noVNC's JavaScript modules load from that opaque origin.
The screen ticket authorizes those assets and its socket, never workspace APIs.
The deliberate difference is that viewer settings cannot persist in browser
storage. The remote computer's files, sign-ins and clipboard remain its own.

HTML attachments download when opened directly and remain readable as text in
the transcript. File responses prohibit script, disable MIME sniffing and carry
no referrer. Explicit downloads request attachment disposition on the server,
so a desktop window connected across origins also receives a download.

An artifact URL carries a ticket for that document, not the workspace token:
script can read its own URL even with an opaque origin. Artifact policies admit
the desktop origins as ancestors, while retaining their opaque sandbox.

## Reconnecting is a snapshot followed by events

Each socket begins with the current activity, unfinished reply text and coding
jobs. Capturing that snapshot and subscribing to subsequent events share a
lock with emission, so a delta cannot be omitted or applied twice. A slow
client whose feed overflows reconnects for another snapshot. Heartbeats retire
connections that disappeared without a close frame.

On every connection, including the first, the page refreshes the roster,
settings, decisions, usage and selected transcript. It keeps the window and
composer mounted. A transcript refresh merges messages arriving during the
read. A new snapshot removes obsolete streams and restores Stop for live runs.
Thinking and past live tool chips are not replayed; completed tool records
remain in the transcript. Live reply snapshots hold at most 512 KiB per turn;
the completed message remains authoritative for larger replies.

## A restart preserves work and asks before repeating it

Closing or disconnecting a client never stops the daemon's actors, schedules,
or coding jobs. Restarting the daemon is different: process state and an
external tool's unrecorded result cannot be recovered reliably.

Every accepted conversation now records its first delivery in `pending_runs`,
in the same SQLite transaction as the message. Settlement removes that entry;
an operator stop also removes it without releasing the in-memory bookings.
Startup converts remaining entries into durable interruption notices, once,
before starting actors. Each notice links the original message to **Try again**.
Completed messages, attachments, memories, working notes and repository files
remain on the volume. Pending approvals expire because their waiting turns no
longer exist. No interrupted tool action or approval is automatically replayed.

This is a deliberate recovery policy: an external action can succeed just
before the process dies, so automatic replay could send or push twice. Review
the conversation before retrying. Retry starts a new run with the original
request and the normal limits. This journal does not checkpoint a model's
thinking or resume a coding subprocess. Back up the volume before deploying;
SQLite migrations are forward-only.

A workspace also holds a process lock for its runtime's lifetime. A second
host pointed at the same data directory refuses to start, rather than running
a second scheduler or treating the first process's work as interrupted.


## Coding inside the container

The image includes pinned releases of Codex, `pi` and Claude Code, plus Git,
GitHub CLI, Node 22, pnpm, Python 3, a C/C++ build toolchain and ripgrep.
`docker-compose.yml` explicitly forwards the optional `ANTHROPIC_API_KEY`,
`CODEX_API_KEY`, `OPENAI_API_KEY`, `OPENROUTER_API_KEY` and `GH_TOKEN` environment variables.
These configure the coding tools; the inference key entered in Guaca Settings
is separate and is never silently reused by a harness.

For a personally operated backend, sign in as the container's `guaca` user
(the default), after starting it:

```sh
docker compose exec guacad codex login --device-auth
docker compose exec guacad codex login status
docker compose exec guacad claude auth login
docker compose exec guacad claude auth status
```

Codex prints the verification link and device code. Claude's CLI can print a
login URL and accept the code when its callback cannot reach the container.
These operations belong to the official programs. Their home is
`/var/lib/guaca`, on the persistent volume; never sign in as root and expect the
daemon user to find that session. Reopen the repository panel to refresh the
CLI status. Codex coding authentication is separate from Guaca's ChatGPT
provider sign-in. The model settings for coding also belong to each CLI.

Claude's noninteractive CLI prioritizes `ANTHROPIC_API_KEY` over subscription
OAuth. Leave that variable unset when intending to use a CLI-owned subscription;
otherwise jobs can incur API charges. `claude setup-token` also supports the
CLI's `CLAUDE_CODE_OAUTH_TOKEN` environment variable for personally operated
headless scripts; it is not a Guaca OAuth client or a token field in Settings.
The compose file does not import a laptop's OAuth credentials automatically.
See [Codex authentication](https://developers.openai.com/codex/auth),
[Claude authentication](https://code.claude.com/docs/en/authentication) and
[Claude's usage terms](https://code.claude.com/docs/en/legal-and-compliance).
A managed service must use a permitted API/provider authentication arrangement;
CLI availability alone is not authorization to route users' consumer plans.

In a group's repository panel, **Git access** is independent of either CLI
sign-in. For HTTPS, save a repository-scoped access token and the username your
Git service requires. GitHub's token creation link is included; select the
repository and grant Contents read/write (workflow edits need their own
permission). SSH uses keys configured under the backend user. The token is
stored outside the checkout, mode 0600, replaced atomically, and scoped to the
origin's host **and path**. Linked agent worktrees share the repository's helper.
Existing tokens keep their configured path; saving again applies the tighter
scope. **Remove saved token** removes Guaca's local copy; revoke it at the Git
service too if it must become unusable elsewhere.

**Check read and push access** runs `ls-remote` and a push dry run. It changes
no remote refs and distinguishes read failure from push failure. Branch
protection and server hooks can only decide an actual update. A separate push
URL is shown explicitly; an origin token does not grant access to that other
address. Git access does not sign in `gh` for pull-request API operations;
configure `gh auth login` or `GH_TOKEN` separately if jobs need those.

Codex jobs support worktrees, streamed progress, completion, cancellation,
acknowledged corrections and Guaca push approvals through the official
app-server interface (Codex 0.153 or newer). Changing harness preserves the
repository's path, engineer assignments and gate setting. Corrections guide the
next model decision; they do not reverse commands already executed. See
[the coding contract](CODING.md#codex-runs-through-its-official-cli) for approval
policy and command-rule behavior.

A container does not inherit the host's installed dependencies, login sessions,
or unmounted files. For a working tree on the host, add a volume such as
`./project:/workspace/project` and choose **Directory on backend** with
`/workspace/project`. The directory must be writable by UID 1000. Agent
worktrees live in Guaca's persistent volume. Do not mount the Docker socket.
For projects requiring other toolchains (for example Rust, Go or Java), derive
an image from `guacad` and install the versions the project needs. Keep those
versions in the derived Dockerfile so the environment can be rebuilt.

The compose service runs with an init process to reap child processes and uses
a named volume for settings, credentials, SQLite, attachments, memories and
repositories. A container on your laptop still sleeps with that laptop. Use
this same image on an always-on host to keep agents working while it is shut.

## Verifying the host boundary

Run `./scripts/ci.sh` for desktop and daemon lint, builds and offline suites.
`./scripts/image.sh` builds a container, checks both harnesses and the pinned
package manager, then verifies settings and the token survive a restart on
its temporary volume. The script removes its container and volume afterward.
Rust 1.89 or newer is required; the container builds with 1.95.

For a real browser check, build the frontend and daemon, then run:

```sh
pnpm build
cargo build --manifest-path src-tauri/Cargo.toml --no-default-features --features server --bin guacad
GUACAD=src-tauri/target/debug/guacad CHROME_BIN=/path/to/chrome node scripts/hosting-browser.mjs
```

The browser check uses a fresh Chrome profile, a temporary workspace and an
offline streaming model. It verifies partial-reply recovery, completion with
no client, artifact isolation and answer delivery, and crash recovery without
automatic replay. No account or provider credential is used. Chrome's macOS
installation is the default when `CHROME_BIN` is absent. Live provider sign-ins
and a real E2B desktop remain separate integration checks requiring accounts.

## Moving an existing workspace

Stop the original backend before taking a backup. Copy its data directory
(the database together with any SQLite WAL files, attachments, memories and
worktrees) into the new root's `data/`, and its configuration directory into
`config/`. Keep an untouched backup: an older executable cannot open a database
migrated by a newer one. For Docker, restore that root into the named volume
and make it writable by UID 1000 before starting the service.

Repository paths name the old machine until their directories are mounted at
the same paths or linked again on the backend. Hosted provider credentials
must be configured for the new environment; moving Guaca's files does not move
the desktop keychain or a coding tool's sign-in. Keep the original workspace
stopped after migration so its schedules do not run alongside the copy. The
process lock prevents two hosts sharing one directory, not two independent
copies. A browser connected to the new daemon starts no local backend.

The Git integration suite (`tests/repository_auth.rs`) runs a local authenticated
smart-HTTP server and exercises clone, pull, a real push from a linked worktree,
credential rotation, revocation and repository-path scoping. It uses dummy
credentials and no external Git service. Claude tests remain offline unless
explicitly selected. An optional live Codex contract test pins a small model:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features server --test codex_live -- --ignored
```

It uses `gpt-5.4-mini` at low reasoning in a disposable repository and never
uses the operator's configured default model. It steers an active turn, checks
and commits the revised file, then denies a push to a disposable local bare
repository and verifies that no remote ref changed. No external Git service
is contacted.
