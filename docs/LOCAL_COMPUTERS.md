# Local agent computers

Status: reviewed against the code on 2026-08-17; PR A (provider extraction)
open; Apple spike pending

Date: 2026-08-17

Scope: add local computer providers without changing the agent-computer product
contract

The words **must**, **should**, and **may** in this document are requirements,
recommendations, and permitted choices respectively.

## Decision

Guaca will support three computer providers behind one Rust-owned interface:

1. **Apple Container**, the preferred local provider on Apple silicon running
   macOS 26 or newer. Each agent container runs in its own lightweight Linux VM.
2. **Docker**, the compatibility provider for Macs that cannot use Apple
   Container or whose operator already uses a Docker-compatible engine.
3. **E2B**, the existing hosted provider and the option with an off-device
   network boundary.

Apple Container and Docker will use the same Guaca-owned OCI desktop image, and
both are driven through their command-line tools, spawned by argument vector
with no host shell in between. Apple's `container` CLI was designed to mirror
`docker`'s, so the two providers are thin argument builders over one shared
"spawn, capture, parse JSON" helper rather than two protocol clients. A direct
implementation on Virtualization.framework is out of scope: Apple's
Containerization project already owns the VM, OCI, storage, networking, and
lifecycle layers Guaca would otherwise have to build and maintain.

Provider choice is app-wide for newly created computers. Every computer is
pinned to the provider that created it until the operator destroys it. Changing
the setting must never silently migrate, replace, or destroy an existing disk.

Local mode is not a claim that arbitrary agent code is harmless. It improves
kernel and filesystem isolation, especially with Apple Container, but a local
guest may still reach services exposed by the Mac or its LAN. The UI must say
so. E2B remains the stronger choice when the machine must also be physically
off the operator's network.

## Goals

- Let an operator use agent computers without an E2B account or API key.
- Preserve the current computer contract: one persistent Linux machine per
  agent, a shell, internet access, a browser, a visible desktop, sleep/wake, and
  explicit destruction.
- Keep the agent runtime and all provider logic in Rust. The webview renders
  state and forwards intent only.
- Keep browser profiles and agent-created files across sleep and app restarts.
- Preserve the rule that a group credential reaches only the environment of a
  sandbox command. It must not enter a prompt, transcript, frontend IPC,
  command-line argument, log, or persistent sandbox file.
- Isolate agents from one another by compute instance, network, and storage.
- Never expose noVNC beyond loopback or give its provider credentials to the
  webview.
- Recover safely from crashes between provisioning a resource and recording
  it, and remove resources that Guaca owns but no live agent claims.
- Keep E2B behavior working while the provider boundary is introduced.

## Non-goals

- Containerizing the Guaca Tauri application itself.
- Moving an existing computer's disk between providers.
- Running macOS guests, GPU workloads, Kubernetes, or nested container engines.
- Silently installing a privileged runtime or accepting an installer prompt on
  the operator's behalf.
- Building a general-purpose VM manager on Virtualization.framework.
- Claiming that the first local release blocks every route to the host or LAN.
- Replacing Guaca's protected-action and prompt-injection rules. Local execution
  makes those rules more important but does not redefine them.
- Automatically rebuilding existing computers when the OCI image changes.

## Product behavior

### Settings

Settings gains a **Computer provider** selector with these values:

- `Automatic (recommended)`
- `Apple Container — local`
- `Docker — local`
- `E2B — hosted`

Under the selector, Guaca shows a current status for each provider:
`ready`, `not installed`, `not running`, `unsupported`, or `error`. Status text
must explain the next action. Examples:

- “Apple Container requires Apple silicon and macOS 26 or newer.”
- “Apple Container is installed but stopped. Starting a computer will start
  its service.”
- “Docker is installed but its engine is not running. Open Docker and try
  again.”
- “E2B needs an API key.”

Guaca may start an installed Apple Container service when the operator or an
agent explicitly asks for a computer. Guaca must not install Apple Container,
install Docker, start Docker Desktop, request administrator privileges, or
change system networking without a distinct operator action.

The E2B key remains a redacted setting. The idle-minutes setting moves from the
E2B section to the provider-neutral computer section.

When a local provider is selected, Settings displays this disclosure:

> Local computers run untrusted agent commands on this Mac. They cannot see
> host files unless shared, but they may reach services exposed by this Mac or
> its local network. Use E2B when you need an off-device network boundary.

Changing the provider displays: “Existing computers keep their current
provider until you destroy them.”

### Automatic selection

For a newly created computer, `automatic` resolves once in this order:

1. Apple Container, when supported and either running or startable by Guaca.
2. Docker, when its engine is reachable.
3. E2B, when a key is configured.
4. Unconfigured, when none is ready.

The resolved provider is written to the computer record. It is never resolved
again for that computer. A temporary provider failure must return an actionable
error, not create a replacement on another provider.

Config migration preserves intent: an existing installation with an E2B key is
set explicitly to `e2b`; an existing installation without one is set to
`automatic`. A fresh installation defaults to `automatic`.

### Agent and pane behavior

- A computer-capable prompt and the `run_command`, `browse`,
  `open_on_desktop`, and `use_screen` tools are offered only when the selected
  provider can create a computer or the agent already owns a reachable one.
- When no provider is usable, the prompt must not claim that the agent has a
  computer. This prevents a predictable tool call from becoming a “no E2B API
  key is set” failure.
- The computer pane uses provider-neutral state. It shows `running`, `asleep`,
  or no computer, just as it does today.
- A provider setup failure is shown inline in Settings or the computer pane,
  never as a modal alert.
- “Sleep” stops compute but keeps the disk. “Destroy” removes the compute
  instance, its private network, and its persistent volume.
- An existing computer remains visible even when the currently selected
  default provider differs from its provider.

## Architecture

The target dependency direction is:

```text
agent tools / Tauri commands
            |
            v
     ComputerManager
       |          |
       |          +-- shared desktop, browser, files, sign-in detection
       v
 ComputerProvider
       |-- Apple Container
       |-- Docker Engine
       `-- E2B
```

`ComputerManager` owns provisioning, per-agent serialization, provider
selection, credentials, idle accounting, crash recovery, and conversion to the
public IPC type. Providers own only external resource operations.

The desktop and browser behavior moves out of `e2b.rs` into shared computer
code. `start_desktop`, `open_on_desktop`, `browse`, screenshots, mouse/keyboard
actions, `browser.py`, `sessions.py`, attachment placement, and attachment
reading must use the provider's `exec` primitive and nothing else. A provider
must not reimplement browser semantics, and there is no copy primitive: files
already travel as base64 in chunked `exec` commands (`runtime/mod.rs`, the
`PLACE_CHUNK` loop) and come back through `base64 -w0`, so every provider gets
placement for free and the trait stays at one operation for "do something on
the machine".

### Provider interface

The exact Rust spelling may follow the surrounding code, but the interface must
represent these operations:

```rust
trait ComputerProvider: Send + Sync {
    async fn probe(&self) -> ProviderStatus;
    async fn create(&self, request: CreateComputer) -> Result<ProviderHandle>;
    async fn find(&self, computer_id: ComputerId) -> Result<Option<ProviderHandle>>;
    async fn inspect(&self, handle: &ProviderHandle) -> Result<ProviderState>;
    async fn start(&self, handle: &ProviderHandle) -> Result<ProviderHandle>;
    async fn stop(&self, handle: &ProviderHandle) -> Result<()>;
    async fn delete(&self, handle: &ProviderHandle) -> Result<()>;
    async fn exec(&self, handle: &ProviderHandle, request: ExecRequest)
        -> Result<Output>;
    async fn viewer_target(&self, handle: &ProviderHandle)
        -> Result<ViewerTarget>;
    async fn list_owned(&self, installation: InstallationId)
        -> Result<Vec<ProviderHandle>>;
}
```

Requirements on the interface:

- `exec` accepts an argument vector, environment map, timeout, and working
  directory. Host-side shells must never be used to invoke a local runtime: a
  local provider spawns its CLI by argument vector, and a timeout kills that
  child. The guest process may outlive the kill, exactly as an E2B command
  outlives its HTTP timeout today; the next command finds it or does not.
- Agent-authored shell text is passed as one argument to `/bin/bash -lc` inside
  the guest. It must never be interpolated into a host command.
- `ProviderHandle` and `ViewerTarget` are backend-only types and are never
  serializable over Tauri IPC.
- A provider reports `running`, `asleep`, or `gone`. Unknown external states
  fail closed as an error; they are not interpreted as permission to replace a
  disk.
- Provider errors map to common categories: `unsupported`, `unconfigured`,
  `unavailable`, `resourceGone`, `image`, `timeout`, and `operation`. Every
  operator-facing message says what failed and what to do next.

### Public IPC

The existing `sandboxId` terminology becomes provider-neutral:

```ts
interface Computer {
  id: string;
  provider: "appleContainer" | "docker" | "e2b";
  state: "running" | "asleep";
  vncUrl: string | null;
}

interface AgentCard {
  // existing fields
  computerId: string | null;
}

interface ComputerProviderStatus {
  provider: "appleContainer" | "docker" | "e2b";
  state: "ready" | "notInstalled" | "notRunning" | "unsupported" | "error";
  canStart: boolean;
  detail: string;
}
```

`Settings` adds `computerProvider`, and `SettingsPatch` accepts the same field.
The existing `computerIdleMinutes`, `e2bKeySet`, `e2bKeyHint`, and redacted-key
behavior remain. Provider probes are cached briefly by `ComputerManager` and
invalidated after a settings change or provider operation; prompt construction
must not launch a new CLI process for every model call.

All fields and enum values crossing IPC remain camelCase. Add one
`computer_provider_statuses` command and call it from Settings; the command must
be included on both sides of the IPC contract test. No provider secret, host
path, guest IP, published port, or external resource ID crosses IPC.

## Persistence and migration

Append database migration 16. Do not edit migrations 6–8.

Create a provider-neutral `computers` table with this logical shape:

```text
id                 stable Guaca computer UUID, primary key
agent_id           unique foreign key to agents
provider           appleContainer | docker | e2b
provider_id        provider resource identifier, nullable while provisioning
control_secret     provider command token, empty for local providers
viewer_secret      provider viewer token, empty for local providers
image_ref          pinned local image digest, empty for E2B
record_state       provisioning | ready | deletePending
last_used_at       milliseconds since epoch
created_at         milliseconds since epoch
updated_at         milliseconds since epoch
```

The record type containing the two secret columns must not implement
`Serialize`. Public queries project into a separate redacted type.

Migration 16 must:

1. Copy every complete legacy E2B tuple (`sandbox_id`, envd token, traffic
   token) into `computers` with provider `e2b` and state `ready`.
2. Treat a partial legacy tuple as absent, matching current behavior.
3. `ALTER TABLE agents DROP COLUMN` each of the three E2B-specific columns.
   The bundled SQLite is 3.46 and the columns are neither indexed nor
   referenced, so this is legal, and it is preferred to rebuilding `agents`:
   a rebuild is where a forward-only migration goes wrong on a real database,
   and here it would buy nothing.
4. Include a migration test that starts at schema version 15 with running,
   partial, and absent legacy computer rows, and asserts every agent row is
   otherwise untouched.

`AgentCard.computer_id` is populated through a left join. Provider credentials
are loaded only by the computer store methods.

App configuration moves to version 2 and gains:

```text
computer.provider       automatic | appleContainer | docker | e2b
computer.idleMinutes    1..1440
computer.installationId stable UUID used to scope external resource labels
e2b.apiKey              unchanged secret
```

No runtime path may continue reading or writing the legacy agent columns after
migration 16.

## Provisioning and lifecycle

All operations for one agent pass through a per-agent single-flight lock. An
operator click and an agent tool call must not create two machines.

Provisioning follows this order:

1. Resolve the provider and allocate a Guaca computer UUID.
2. Insert a `provisioning` row before making the external resource.
3. Create resources with a deterministic name and labels containing the
   installation ID, computer ID, and agent ID.
4. Record the returned provider ID, provider secrets, and image digest, then
   mark the row `ready`.
5. If creation or recording fails, delete all resources created so far. If
   cleanup also fails, keep a `deletePending` row for startup recovery.

Because the external name and labels contain the computer UUID, startup can
recover a resource created just before a crash even when `provider_id` was not
written yet.

Lifecycle rules:

- `ensure` inspects a recorded computer without waking it first.
- `running` refreshes `last_used_at` and returns the existing machine.
- `asleep` starts the same machine and keeps its disk.
- `gone` removes the stale row and creates a replacement only for the action
  that explicitly required a computer.
- A provider error preserves the row and disk reference.
- Explicit destroy clears the row only after provider deletion succeeds.
- Deleting an agent tries to destroy its computer first. On failure, the agent
  is still soft-deleted and the computer is marked `deletePending` for retry.
- Provider changes affect only computers created afterwards.

E2B retains its server-side idle timeout. Local providers need two layers:

- The app updates `last_used_at` on every exec, browser action, desktop
  action, or active viewer session and stops idle local computers.
- The image's PID 1 is a shell loop that exits when a heartbeat file goes
  stale, and the host's idle ticker touches that file on every running local
  computer. If Guaca crashes or is force-quit, the guest exits after the
  configured idle period, which leaves the container stopped with its disk
  intact. That is the whole watchdog: a loop and a file, not an init
  framework.

A graceful app shutdown also stops every running local computer. It does not
stop hosted E2B computers early; their existing timeout remains authoritative.

## Local OCI desktop image

Add a versioned `computer-image/` directory. CI builds both `linux/arm64` and
`linux/amd64`, publishes a multi-platform image, and records the platform digest
in the app release. Runtime pulls must use that digest, never a mutable tag.

Publication is a maintainer decision, not something this work can settle from
a fork: the repository has no hosted CI today, and the digest has to live under
a registry namespace the maintainer owns (`ghcr.io/madebywelch/…`). Until it
is published, nothing here is testable, so the image reference the app uses is
a single constant with one documented development override,
`GUAC_COMPUTER_IMAGE`, read at startup and shown in Settings when set. The
override exists so a reviewer can build the image locally and try the feature;
it is not a user setting.

The initial image should use a pinned Debian stable base and contain:

- an unprivileged `user` account (uid 1000) with home `/home/user` and
  passwordless `sudo`, because agents `apt-get install` and the E2B desktop
  lets them;
- Xvfb, a minimal XFCE session, x11vnc, and noVNC at `/opt/noVNC`, the path
  `start_desktop` already uses, so the shared desktop code does not fork per
  provider;
- native Chromium plus a `google-chrome` compatibility wrapper on `PATH`, so
  `browser.py`, `chrome_flags`, and the shim all keep working unchanged;
- Firefox ESR, a file manager, and a text editor;
- Python 3 with `websocket-client` already installed (E2B installs it on
  first browse; the image should not need the network for that), `curl`, Git,
  CA certificates, `scrot`, FFmpeg, and `xdotool`;
- Guaca's `browser.py` and `sessions.py`, the Chrome wrapper, and the desktop
  entry;
- one Chrome profile at `/home/user/.guac/chrome`, regardless of whether the
  browser was opened from a tool, desktop icon, or file association.

It must not contain an SSH server, Docker socket, host credential, baked API
key, or provider token. noVNC listens inside the guest; it is not published to
all host interfaces.

Each agent gets one private named volume mounted at `/home/user`. This preserves
browser cookies, inbox files, and work products across stop/start and permits a
future explicit image rebuild without exposing the Mac home directory. Destroy
deletes this volume. Sleep never does. On every boot, PID 1 copies the image's
home skeleton into the volume without overwriting anything already there, and
removes Chrome's `SingletonLock`, which a stopped container leaves behind and
which would otherwise make the next Chrome refuse the profile as "in use".

Local containers are never privileged. They receive no host PID, network, IPC,
or user namespace; no device or host bind mounts; and no container-engine
socket. Drop at least `NET_ADMIN`, `NET_RAW`, `SYS_ADMIN`, `SYS_MODULE`, and
`SYS_PTRACE`. The image may allow package installation inside the guest, but
guest root cannot gain capabilities removed from the container's bounding set.

Initial resource limits are 4 vCPUs, 4 GiB RAM, and 1 GiB shared memory. The
Apple spike must measure whether 3 GiB is sufficient before these constants are
committed. The target persistent-home limit is 20 GiB. Apple should enforce it
when creating the volume; the Docker spike must record whether its selected
Desktop storage driver can enforce the same quota and, if not, how usage is
reported before the shared Docker disk fills. Resource controls are not
exposed as user settings in the first release.

Image upgrades are explicit. A newer app may report that an image is stale, but
must not replace a container or discard its writable layer automatically.

## Apple Container provider

Target Apple Container 1.2.x, with 1.2.2 as the first tested minimum. Accept
compatible `>=1.2.2,<2.0.0` releases and fail with a version-specific message
outside that range.

- Discover the signed `container` executable without invoking a shell. Finder
  launches with a restricted `PATH`, so check the documented install location
  (`/usr/local/bin/container`) as well as inherited search paths.
- Use machine-readable command output (`--format json` on `list`, the JSON
  `inspect` already emits). Never parse presentation tables.
- Use ordinary `container create/run`, not `container machine`. The latter can
  mount the Mac home directory read/write by default.
- Create one standard container, one named volume (`volume create -s 20G`,
  which enforces the quota at creation), and one isolated network per agent.
  Never attach agent containers to the shared default network.
- Address resources by deterministic ID and Guaca ownership labels, not by the
  agent's editable name.
- The viewer proxy connects to the guest IP on port 6080, read from
  `inspect`'s `networks[0].address`. The host routes to that address directly
  on macOS 26 (Apple's own how-to shows the gateway `192.168.64.1` reaching a
  container), so noVNC is not published.
- Pass secret values in the child process environment and invoke
  `container exec --env VARIABLE ...`, where the argument contains only the
  variable name. Confirmed in the 1.2.2 source: `exec` and `run` share
  `Parser.env`, which reads a bare name from `ProcessInfo.processInfo.environment`
  (`Sources/Services/ContainerAPIService/Client/Parser.swift`). The spawned
  process gets an allowlisted host environment plus the secrets, and neither
  arguments nor tracing fields may include values.
- `exec` propagates the guest exit code as its own
  (`ContainerExec.swift`, `throw ArgumentParser.ExitCode(exitCode)`) and keeps
  stdout and stderr separate when no TTY is requested.
- Start the service with `container system start --enable-kernel-install`;
  without the flag it prompts on a first run and a spawned child would hang on
  that prompt.
- Stop/start the existing standard container for sleep/wake. Do not use
  `--rm`.
- On deletion, remove the container, its volume, and its network in that order,
  tolerating “not found” for idempotency.

Apple's separate networks isolate agents from containers on other networks.
They do not establish that host or LAN services are unreachable. Guaca must not
create a host-service DNS mapping, but the product disclosure remains required.

## Docker provider

The Docker provider drives the `docker` CLI the same way the Apple provider
drives `container`: spawned by argument vector, never through a host shell,
JSON output only. An earlier draft required the Engine API over its Unix
socket; that was dropped because reqwest cannot speak Unix sockets, so it
meant either a new dependency (`bollard`) or a hand-rolled HTTP client, and it
bought nothing the CLI does not already give: `docker exec -e NAME` inherits
the value from the client environment exactly as `container exec` does,
`inspect` and `ps --format json` are stable JSON, and Docker contexts are
resolved for us.

- Support Docker Desktop first. Other engines are compatible only when their
  `docker` CLI passes the same smoke tests.
- Discover the CLI at `/usr/local/bin/docker` and
  `/Applications/Docker.app/Contents/Resources/bin/docker` as well as
  inherited search paths, for the same Finder reason as above.
- Create one container, named volume, and user-defined bridge network per
  agent.
- Publish noVNC to an engine-assigned port bound explicitly to `127.0.0.1`
  (`-p 127.0.0.1:0:6080`), read back from `inspect`. The Guaca viewer proxy is
  the only URL given to the webview.
- Apply the same labels, resource limits, capabilities, image digest, command
  environment, and lifecycle rules as Apple Container.
- Never mount `/var/run/docker.sock` inside an agent container.
- Report clearly that Docker containers share the Docker Linux VM's kernel and
  therefore do not provide Apple Container's VM-per-agent boundary.

Guaca does not install or license Docker Desktop. It detects an unavailable
engine and tells the operator to start or install their chosen runtime.

## E2B provider

The first provider-abstraction change must wrap current E2B behavior without
changing it:

- secure sandbox creation and both access tokens;
- server-side idle timeout, pause without memory, and token refresh on resume;
- current desktop template and internet access;
- envd command execution;
- private public traffic through the loopback viewer proxy;
- orphan listing and deletion;
- per-command group credential environments.

E2B's external sandbox ID and tokens move to the `computers` table. No E2B type
may remain in `Runtime::ensure_computer`, Tauri commands, domain `AgentCard`, or
frontend types.

## Viewer proxy

Generalize `proxy.rs` rather than bypassing it. The webview always receives a
URL shaped like:

```text
http://127.0.0.1:<viewer-port>/<computer-id>/6080/vnc.html?...
```

The manager resolves that opaque computer ID into a backend-only
`ViewerTarget` containing scheme, host, port, path prefix, and optional
upstream headers. E2B targets use TLS and the traffic token. Apple targets use
the guest IP. Docker targets use the loopback published port.

Targets are cached only while the resource is running and invalidated on
sleep, wake, deletion, or provider failure. A target may be re-inspected once
on a cache miss; the proxy must not execute a provider CLI separately for every
noVNC asset or WebSocket frame.

The proxy remains bound to loopback, strips forged forwarding and provider
headers, supports WebSocket upgrade, and never returns upstream credentials to
the browser. Existing proxy tests become provider-neutral and retain an E2B
header test.

## Credentials and command safety

`ComputerManager`, not a provider call site, loads the group's connector
environment and attaches it to every agent command. Shared desktop-maintenance
commands run without group credentials unless they are executing an
agent-requested action that needs them.

The following are release-blocking requirements:

- Secret values never implement `Debug` or `Serialize` in a provider request.
- Logs may contain variable names, provider, computer ID, exit code, duration,
  and byte counts, but never environment values.
- Local runtime arguments contain `--env NAME`, never `--env NAME=value`, for
  both CLIs. The test that spawns a fake CLI captures its argv and its
  environment separately and asserts the sentinel is in the second only.
- The environment exists only for the guest process. It is not placed in the
  container definition, image configuration, heartbeat, shell profile, or
  filesystem.
- Command output is still model-visible. The prompt continues telling agents
  not to print credentials; Guaca cannot prevent an explicitly malicious
  command from printing an environment value it was authorized to use.
- Tests use a sentinel secret and search captured arguments, logs, IPC values,
  container inspection, and guest files for that sentinel.

## Network and threat boundary

Every local agent receives a distinct provider network. No two agents share a
network or volume, and Guaca creates no route or DNS name specifically for one
agent to reach another.

The first release allows ordinary outbound internet access because browsing and
package installation are core computer behavior. It must not claim enforced
host/LAN denial. In particular, services bound by the operator beyond loopback
may be reachable through the virtualization network.

Before release, the Apple and Docker smoke suites must record whether a guest
can reach:

- another agent's guest address and noVNC port;
- a service bound only to Mac loopback;
- a service bound to the Mac's LAN address;
- another LAN address;
- public HTTP, HTTPS, DNS, and arbitrary TCP destinations.

Agent-to-agent connectivity is a release blocker. Host/LAN reachability is a
documented local-mode limitation unless a host-enforced control is found that
does not require giving the guest `NET_ADMIN` or routing all traffic through a
breakable guest rule.

A later hardened mode may use an internal network plus a controlled egress
gateway or domain allowlist. It is a separate design because an HTTP-only proxy
would break arbitrary package managers and network tools, and guest-root
firewall rules are not a security boundary.

## Orphans and ownership

Every newly created external resource carries all three labels:

```text
guac=true
guac.installation=<installation UUID>
guac.computer=<computer UUID>
```

The agent ID may be an additional diagnostic label but must not be the ownership
key. Renaming an agent changes no resource name or label.

At startup, and after a failed deletion, Guaca lists only resources carrying
its exact installation label. It reconciles them with `computers`:

- a `provisioning` record plus matching resource is recovered and completed;
- an unclaimed or `deletePending` resource is deleted;
- a ready record whose provider resource is gone is cleared;
- an unknown provider or ambiguous ownership is reported and preserved, never
  guessed at or deleted.

Volumes and networks are swept with the same installation and computer labels.
Legacy E2B resources already recorded in the database remain manageable by
their exact IDs. An unclaimed resource carrying only the old `guac=true` label
must not be deleted after installation scoping lands, because two installations
using the same E2B account cannot be distinguished; log it for manual cleanup
instead.

## Failure behavior

- A failed create releases partial resources and tells the caller which stage
  failed and how to retry.
- A failed wake preserves the computer record and reports that the disk was not
  replaced.
- A missing external resource clears the stale record only after the provider
  has positively reported `not found`.
- Timeouts do not imply `gone`.
- A failed stop leaves the state as last inspected and offers retry.
- A failed explicit destroy retains the record and offers retry.
- A failed cleanup during agent deletion is logged and persisted as
  `deletePending`; soft deletion and run-settlement cleanup still complete.
- Image pull/build output is summarized in the UI. Raw provider output is
  available only in debug logs after redaction.
- Existing local computers remain usable without a network connection when
  their pinned image and runtime data are already present.

## Verification

### Unit and contract tests

- Configuration v1-to-v2 migration and automatic-provider resolution.
- Database migration 15-to-16, including complete, partial, and absent E2B
  records.
- Provider enum parsing fails closed on unknown values.
- Per-agent single-flight provisioning under concurrent operator and agent
  requests.
- Create rollback, crash recovery, delete-pending retry, and installation-scoped
  orphan cleanup.
- State mapping for running, asleep, gone, timeout, and unknown provider state.
- Provider switch never changes an existing record.
- No computer prompt or tools when no provider is available.
- Existing-computer tools remain offered when the default provider changes.
- Credential sentinel absent from argv, logs, IPC, inspection, and files.
- Provider-neutral viewer rewriting, WebSocket upgrade, forged-header removal,
  and E2B header injection.
- Sign-in scans inspect only running computers and never wake a sleeping one.
- IPC contract remains camelCase and contains every called command exactly
  once.
- Computer pane and Settings states for ready, unavailable, asleep, gone, and
  provider mismatch.

### Provider smoke tests

Run the same provider conformance suite against Apple Container and Docker:

1. Create a computer from the pinned image.
2. Execute a command and preserve stdout, stderr, and exit code.
3. Inject a sentinel environment value for one command and prove it is absent
   from the next command and provider inspection.
4. Place a binary attachment and read it back through the shared chunked
   `exec` path, byte for byte.
5. Start the desktop and load noVNC through Guaca's loopback proxy.
6. Launch Chromium, use CDP through `browser.py`, take a screenshot, and drive
   input with `xdotool`.
7. Write a home-directory file, stop, start, and verify the file and Chrome
   profile survive.
8. Let the heartbeat expire and verify the resource becomes asleep without
   deleting its volume.
9. Destroy and verify container, volume, network, viewer target, and DB record
   are gone.
10. Perform the network-boundary measurements listed above with two agents.

The E2B cascade suite remains green through the abstraction. Run:

```sh
./scripts/ci.sh
cargo test --manifest-path src-tauri/Cargo.toml --test trajectory
```

Because provider readiness changes the system prompt and tool catalogue, run
the live evaluation suite before release:

```sh
./scripts/evals.sh
```

### Acceptance criteria

The feature is complete when:

- A fresh supported Mac can select Apple Container, create an agent computer,
  browse, use the desktop, sleep, wake, and destroy it without an E2B key.
- Docker passes the same product-level conformance suite on its supported Macs.
- Existing E2B installations migrate without losing or replacing their
  computers.
- Existing computer UI and tools contain no E2B-specific names except the
  provider label in Settings.
- No provider secret or group credential crosses frontend IPC or appears in
  captured argv, logs, inspection, or persisted guest files.
- Two local agents cannot reach each other's guest services or storage.
- The local network limitation is visible before local mode is used.
- A crash during provisioning, deletion, or normal running does not leave an
  untracked Guaca resource.
- All CI, trajectory, provider conformance, and live prompt evaluations pass.

## Delivery sequence

This lands as a stack of pull requests, not one. A single change of this size
to a repository with one maintainer would not get reviewed, and the split
below is also what makes a failure attributable: the extraction lands with no
intended behavior change, so anything that breaks after it is the abstraction,
and anything that breaks after a provider lands is that provider.

1. **Apple spike** (no PR): build the ARM64 image; validate exec, desktop, CDP,
   persistence, secret inheritance, resource limits, networking, and watchdog
   against Apple Container 1.2.2. Record results in this document before
   product code depends on them.
2. **PR A, provider extraction:** `ComputerManager`, the provider interface,
   the `computers` table and migration 16, a provider-neutral viewer proxy,
   and an E2B adapter with no intended behavior change. Desktop, browser,
   files, screenshots, and sign-in scanning move above the interface in the
   same PR, because they cannot be tested against a second provider until they
   have.
3. **PR B, Apple Container:** the `computer-image/` directory, local
   lifecycle, image acquisition, viewer target, volume/network cleanup, idle
   watchdog, provider selection and status in Settings, config migration, the
   disclosure, tool/prompt gating, pane states, and the conformance suite.
   This is the first PR an operator can click on.
4. **PR C, Docker:** the second CLI provider and the same conformance suite.
5. **Release hardening:** orphan/crash exercises, credential sentinel audit,
   network measurements, CI, trajectory, and live evals; the image publication
   workflow, which needs the maintainer's registry.

Two open PRs touch the same files. #7 (maintainer, "Chrome stops asking for a
keyring") edits `e2b.rs`, `tools.rs`, and `prompt.rs`; PR A moves most of
`e2b.rs`, so it is rebased after #7 lands rather than raced against it. #11
gates computer tools on an E2B key; PR B gates them on provider readiness, so
#11 is superseded when B lands and should be closed then.

## Review decisions

The recommended answers below make this spec executable as written. Change
them before implementation if the product direction differs.

1. **Supported Macs:** ship Docker in the same feature if Guaca must support
   Intel or macOS before 26. Recommended: yes, retain that compatibility.
2. **Image delivery:** pull a public, digest-pinned multi-platform image on
   first use rather than adding hundreds of megabytes to every app bundle.
   Recommended: pull on demand and show progress.
3. **Network claim:** disclose possible host/LAN access in local mode and keep
   E2B for an off-device boundary. Recommended: ship this honest baseline, then
   design hardened egress separately.
4. **Automatic priority:** Apple Container, Docker, then E2B for fresh
   computers, while migrating configured E2B users to explicit E2B.
   Recommended: accept.
5. **Image replacement:** never rebuild an existing computer automatically.
   Recommended: accept; add an explicit preserve-home rebuild only when an
   incompatible image actually requires it.
6. **Docker transport:** drive the `docker` CLI by argument vector rather
   than the Engine socket, sharing one spawn/parse helper with the Apple
   provider. Recommended: accept; see the Docker section for why.
7. **Legacy columns:** `DROP COLUMN` the three E2B columns on `agents` rather
   than rebuilding the table. Recommended: accept.
8. **Trait surface:** `exec` only; no copy primitives, since placement is
   already chunked `exec`. Recommended: accept.

## Primary references

- [Apple Container repository and platform requirements](https://github.com/apple/container)
- [Apple Container 1.2.2 technical overview](https://github.com/apple/container/blob/1.2.2/docs/technical-overview.md)
- [Apple Container 1.2.2 command reference](https://github.com/apple/container/blob/1.2.2/docs/command-reference.md)
- [Apple Container networking how-to](https://github.com/apple/container/blob/1.2.2/docs/how-to.md)
- [Apple Container machine behavior](https://github.com/apple/container/blob/1.2.2/docs/container-machine.md)
- [Apple Container environment parsing](https://github.com/apple/container/blob/1.2.2/Sources/Services/ContainerAPIService/Client/Parser.swift)
- [Apple Virtualization framework](https://developer.apple.com/documentation/virtualization)
- [Docker Engine API](https://docs.docker.com/reference/api/engine/)
- [Docker Desktop container isolation](https://docs.docker.com/security/faqs/containers/)
- [Docker port publishing](https://docs.docker.com/engine/network/port-publishing/)
- [E2B infrastructure architecture](https://github.com/e2b-dev/infra/blob/main/docs/ARCHITECTURE.md)
