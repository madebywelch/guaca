# Hosting

Guaca is a native desktop client connected to a `guacad` host. The host runs
agents, schedules, coding jobs and plugins. It can run in Docker on the same
Mac or on an always-on remote machine. Closing the app stops no backend work.
A container on a sleeping laptop still sleeps; unattended work needs a host
that stays awake.

## The runtime runs in two hosts, and neither knows which

The native client no longer constructs `Runtime`, opens the live database or
starts schedulers. `app.rs` wires the window, notifications, menu bar, local
file forwarding, Docker setup and read-only exports from the previous desktop
workspace. `server/mod.rs` and `boot.rs` own runtime startup.

The desktop and browser use the same HTTP commands and WebSocket events.
Tauri remains an optional Cargo feature. Build the backend with
`--no-default-features --features server`; it requires no native webview.
`Deployment::Desktop` remains for library compatibility and tests, not as an
alternative startup mode in the downloadable application.

## Portable hosts now, managed compute spaces next

This change delivers the native desktop client, local Docker setup, explicit
remote-host connections, and group export/import. Both local Docker and a VPS
run the same server runtime. `Deployment::Desktop` remains for compatibility;
managed hosting does not need another runtime variant.

The next phase is a managed Guaca product with persistent compute spaces.
The user signs in, creates or selects a space, and the app discovers its
connection. Provisioning, ownership, billing, and workspace discovery belong
to the managed service. They are not implemented by this change.

`guaca.bot` remains the central identity and integration service for providers
such as Google, serving Gmail, Calendar, and Drive tools. Its account and
provider grants must be separate from the lifecycle and address of a compute
space. The intended authorization boundary is described in
[The account](ACCOUNT.md#managed-compute-is-the-next-phase).

Self-hosting remains supported without a Guaca account unless an integration
requires it. Explicit host addresses are part of that option. SSH tunnels and
Tailscale can support testing or private deployments; a particular Tailnet
hostname is not part of the managed product's sign-in contract. Managed users
should not configure host addresses, Docker, or callback URLs.

## One list, three readers

`ipc.rs` lists the backend commands, generates their HTTP dispatcher and
exports the names checked by `ipc.contract.test.ts`. Both desktop and browser
call that surface. Native commands are a separate, small list in `app.rs` for
operations on the client machine. No Tauri wrapper holds `AppState`.

The contract suite compares frontend calls to both lists and prevents runtime
construction from returning to desktop startup.

## Opening a workspace is one act

`boot.rs` opens the database, expires stale approvals, loads settings and
starts the runtime, scheduler, sign-in sweep, compost and viewers. Only the
backend invokes it. Opening, closing or switching desktop windows does not
start another copy of these loops. Pending turns cannot survive a backend
restart; recovery notices explain interrupted work without replaying it.

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
the backend. File paths and credentials on the client do not transfer to the server.
`coding_harnesses` checks installed programs and their own sign-in status.
Claude Code can use its backend CLI sign-in or a configured API credential.

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

The daemon serves the same frontend bundle that Tauri embeds. `HostSetup`
runs before the workspace mounts. A saved local connection starts the managed
Docker container and retrieves its token automatically; a saved remote
connection is probed before the UI opens. Failure returns to host setup.

`Settings → Workspace` exposes the same On this Mac / Remote host choices.
Changing hosts probes the new connection, saves it and reloads the UI. Groups
stay on their own host; switching is not migration. Remote addresses require
HTTPS, except loopback connections. Credentials are never sent to a nonlocal
plain HTTP address entered through this form.

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

Account sign-in and OAuth-enabled MCP plugins use authorization code with
PKCE. The original embedded runtime bound a loopback port before naming the
redirect. The server runtime uses `/v1/oauth/callback` on the origin through
which the operator reaches the workspace. ChatGPT and GitHub device sign-ins
do not redirect to this route.

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

Plugin sign-in registers its callback dynamically, subject to the vendor's
redirect rules. HTTPS reverse proxies provide an HTTPS callback; an SSH port
forward provides the local HTTP callback described below.

Account sign-in uses the fixed `guaca-desktop` client at guaca.bot. Its current
registrations allow the loopback paths, not arbitrary remote hostnames. A
reachable `/v1/oauth/callback` route does not itself authorize that URL with
guaca.bot. The service must accept it before sign-in can complete.
`tests/account.rs` checks the served flow against a scripted service; it does
not establish acceptance of a production hostname.

## The desktop app can show a box, and the menu bar follows

The desktop shows exactly one host at a time. Its menu bar reads the presence
fed by the connected frontend, and sends actions back through that connection.
It has no local runtime to fall back to. Closing the window hides it; quitting
the app ends the client and leaves the host running.

The webview policy admits HTTPS/WSS and loopback HTTP/WS for Docker. Scripts
remain restricted to the bundled application. Host setup and Docker errors
appear before any workspace-dependent screen mounts.

## A repository arrives on a box as a clone of a remote

A desktop repository is a directory the operator picked: their own checkout,
their branch, their uncommitted change, which `docs/CODING.md` says is the
point of the local version. A box has no directory anybody could pick, so a
repository is linked by its remote instead, and the workspace clones it into
a directory of its own under `data/repos/`. Everything downstream is the
code that already existed: the same worktrees, the same `shell` and `code`
doors, the same push gate, against a clone whose work comes back as branches
and pushes rather than as a tree the operator is sitting in.

The clone carries explicit Git configuration. A `credential.helper` points at a file beside the
settings, so a fetch or a push from any process standing in the tree (a
job's harness included) finds the token without the token ever entering
`.git/config` or a URL; the file is git's own credential-store format, mode
0600, named for the clone's directory, and it goes when the repository does.
The operator supplies their commit name and email when linking a remote, or
under **Git access** afterward. Leaving both blank inherits the backend's Git
configuration. `user.useConfigOnly=true` prevents Git from inventing a container
identity when that configuration is absent. Existing directories keep their
identity; older clones using `guaca <guaca@localhost>` can update it under Git
access. Changes apply to future commits and do not rewrite history.

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

- **Account sign-in at an unregistered remote hostname.** Settings → Account
  cannot complete sign-in when guaca.bot rejects the host's callback, typically
  with `invalid_redirect`. This also prevents connecting account-backed Google
  tools there; conversations, coding, and device-code sign-ins remain usable.
  The exact HTTPS callback must be accepted by the account service. Local
  Docker callbacks use
  `http://127.0.0.1:<port>/v1/oauth/callback`; guaca-bot migration 0005 registers
  that path with native loopback port matching. A browser using `localhost`
  returns to `127.0.0.1` for OAuth. A VPS reached through a local SSH forward
  can use that same registered pattern while the tunnel is open, provided
  `GUACA_ORIGIN` does not override it with another origin. VPS hosting alone
  does not make the callback fail.
- **Managed compute spaces.** Provisioning, billing, ownership, automatic
  connection discovery, and account-to-workspace authorization are next-phase
  work. Registering individual VPS or Tailnet callbacks is not the planned
  managed onboarding flow. Google provider callbacks stay at guaca.bot;
  changing a compute host must not require changing those registrations.

## Connecting a desktop to a VPS over SSH

A VPS can be tested without a domain or a public HTTP port. Install Docker,
Compose and Buildx on the server, check out the desired Guaca revision, and
run `GUACA_COMMIT=$(git rev-parse --short=7 HEAD) docker compose up -d --build --wait`.
The supplied Compose file publishes only `127.0.0.1:8787`. Docker restarts the
service after a host reboot; the named volume holds the workspace. Keep the
same Compose project name when updating so it reuses that volume.

On the Mac, open a tunnel, substituting the SSH key and host:

```sh
ssh -N -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 \
  -o ServerAliveCountMax=3 -o IdentitiesOnly=yes -i ~/.ssh/your_key \
  -L 127.0.0.1:18787:127.0.0.1:8787 root@your-host
```

In **Settings → Workspace → Remote host**, enter `http://127.0.0.1:18787`.
Retrieve the access key on the server with
`docker compose exec -T guacad cat /var/lib/guaca/config/token`, then paste it
into the app's Access key field. Treat that value as a password. HTTP here
crosses only loopback interfaces; SSH encrypts the network connection. The
backend runs on the VPS even though the client uses a loopback address.

Closing the tunnel disconnects the interface and prevents browser callbacks
from arriving. It does not stop the VPS, its schedules or coding jobs. Reopen
the tunnel at the same local port and the app reconnects. This is an operator
setup for testing or private access; the desktop does not manage SSH tunnels.
A normal HTTPS deployment uses a TLS reverse proxy on the server and the
host's HTTPS address in the app. Register its exact account callback before
using the optional Guaca-account sign-in. WebSocket proxying must remain
enabled, and proxy logs must omit query strings because the event socket
carries an access token there.

The GitHub App broker uses the same optional Compose overlay as a local host.
Create new broker state on the VPS and authorize the user there. Mount the
App private key only into the broker. Provider CLI sign-ins also belong to the
backend user on this server, independently of sign-ins on the Mac. See
[GitHub App access](GITHUB.md) and **Coding inside the container** below.

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

Alternatively, [connect a GitHub App](GITHUB.md). Its credential service
authenticates both Git and `gh`, renews installation tokens automatically, and
keeps the PEM out of the coding container. Self-hosters supply their own App;
no Guaca account is required.

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

Use **Import / export** in a group’s settings to export that group. Import is
also available under **Settings → Workspace**. The desktop can list and export
groups left by the previous native version without starting those agents or
migrating the old database. Quit the old app before exporting so memory files
and repository work stop changing while the snapshot is taken.

The versioned `.guaca.json` format includes the group’s nonsecret model and
limit settings, agents, instructions, memory, transcripts, attachments, working
notes, usage history, calendar entries, routines and firing history. Database
reads use one transaction. Files are checked by their SHA-256 content address.
The current limits are 64 MB per archive and 32 MB of attachment bytes.

Imports create a new group and new IDs. Routines arrive paused. Running jobs,
pending approvals, remote computers, browser sessions, repository directories,
API keys and stored sign-in credentials are not copied. Repository/harness
settings and service names remain in a reconnection checklist under the
imported group’s Import / export pane. Relink repositories, assign engineers,
connect plugins and configure providers before resuming work. Never resume the
same schedule in both copies unless duplicate execution is intended.

Exports contain conversation content and memories, which may themselves hold
private information. Native exports are saved to Downloads with mode 0600.
The archive is not encrypted. Import validates its format, field allowlists,
identifiers and group boundaries before inserting. A failed import rolls back
the database and removes new memories; already installed content-addressed
attachment bytes may remain unreferenced. No import executes code or contacts
a service. Raw volume backup remains the operator’s disaster-recovery option,
not the migration flow in the app.

## Installing after the hosting changes

`./scripts/install.sh` builds and installs the native macOS application. It
fast-forwards the current branch when possible; `--no-pull` uses the checkout
as it stands. It preserves the original workspace. When Docker is ready, it
also builds a backend image tagged with the source commit and embeds that
image reference in the app. It does not start a backend during installation.
Without Docker, installation still produces a client for remote hosts.

On first launch choose **On this Mac** or **Remote host**. Local setup detects
Docker and offers installation, Open Docker and retry actions. Guaca creates
an unprivileged container, binds a free loopback port, generates an access key
and connects without exposing the key. The named volume survives app and
container restarts. Existing containers are reused rather than silently
replaced while jobs are running. Preview bundle IDs use separate containers
and volumes. A remote Docker context is refused for On this Mac.

For downloadable releases, `GUACA_BACKEND_IMAGE` must name the published image
for that build, ideally by digest. The fallback is the versioned GHCR image;
that image must be published and publicly pullable before releasing the app.
Source development can override it with an already built local image. App and
backend publishing are separate from merging this branch.

## Upgrading databases from the hosting branch

Main used migrations 45–47 for browser consent, the calendar and the author
message index. Earlier hosting builds used 45 for repository remotes and 46
for the interrupted-run journal. The combined build retains main's history and
assigns the hosting additions to 48 and 49. It recognizes the old hosting
schema, fills the missing main additions under the migration write lock, and
preserves existing URLs and pending runs while advancing their new slots.
Tests cover fresh databases and upgrades from both histories, including a
second startup. Back up before upgrading; never run either older branch on an
upgraded database.

Main's calendar and browser-consent commands are available on both transports.
Hosted event routines receive POSTs at `/events/service/topic` on the backend's
public origin. The routine panel prints that address and its dedicated webhook
secret. That secret cannot call the workspace API, and the workspace token
cannot post a routine event. Desktop installs retain the existing loopback
receiver.


The repository form checks whether the connected backend has a GitHub App.
For a GitHub URL it selects that connection by default, before cloning. Token
access remains an explicit alternative for other accounts or Git services.
GitHub user authorization follows the clone under Git access; it supplies human
commit and pull-request attribution. Optional names, instructions and manual
commit metadata are under More options.

## Updating the local host

The desktop compares the managed container's image reference with the one
built into the application. It offers **Back up and update host** when they
differ. Updating is explicit because it interrupts jobs. The manager downloads
the image first, stops the container, copies the whole volume to a new backup
volume, and only then replaces the container. The running port and token are
preserved. A failed backup cancels the upgrade. A failure after the new binary
has touched the database leaves the backup available; it never starts an old
binary against a potentially migrated database. Backup volume names begin
with the container name followed by `-backup-` and are kept until the operator
removes them. This is a recovery backup, not an automatic migration between
hosts.

`./scripts/release-candidate.sh` runs the gates, builds and exercises a local
image, and builds its matching native candidate without publishing anything.
For distribution, first publish a multi-architecture backend image and verify
it can be pulled without registry credentials. Run the candidate script with
`GUACA_BACKEND_IMAGE=...@sha256:...` to pin the app to that image. macOS signing
and notarization remain release prerequisites for a public download. Test on a
clean Mac and on the remote host before merging or publishing the release.
