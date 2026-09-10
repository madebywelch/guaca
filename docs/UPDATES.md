# Host updates

Design and longer-term direction. The first implementation now provides the
shared status UI, API compatibility checks, advisory release manifest, local
backup/replacement progress journal, and self-hosted update instructions.
[Hosting](HOSTING.md#updating-a-self-hosted-backend) describes shipped behavior.
Automatic idle scheduling, a VPS controller, in-app backup restore/deletion,
and installation of arbitrary feed-selected releases remain future work.
The local manager still interrupts work by stopping the backend; it does not
implement a separate runtime maintenance mode.

The remaining sections describe the full target experience. The existing manual
local-container replacement is described in [Hosting](HOSTING.md#updating-the-local-host).

Every client should answer three separate questions: which host it is connected
to, whether that host has a newer published release, and whether this client
can use the host's API. A browser served by an old backend has an equally old
frontend. Comparing their commits cannot tell it that an update exists.

## What the operator sees

Use **host** in the interface, as the existing Workspace settings do. An update
belongs to the whole workspace, not a crew or agent, and does not enter For You.

On connection, check compatibility before mounting the workspace. Once connected,
show a quiet, dismissible notice above the conversation when a newer compatible
release is available: **Host update available** with **Review update**. Dismissal
is scoped to that host and target release; it does not hide later releases.
Keep the status available in Settings > Workspace. Do not move the transcript,
discard composer text, or send an operating-system notification for this.

The review panel names the host, shows installed and available release versions,
links release notes, reports when the release check last succeeded, and displays
the action available for that particular host. Commit and image identifiers live
under Details. Illustrative version numbers in mockups are not release claims.

| State | Message and action |
|---|---|
| Compatible, newer release available | Host update available; Review update |
| Compatible, current release | Host is up to date; Check for updates |
| Frontend too old for host | Update Guaca on desktop; Reload Guaca in a browser if the served bundle is compatible |
| Host too old for frontend | Update this host; retain access to host settings and recovery instructions |
| Release lookup failed | Could not check for updates; last successful check and Retry |
| Legacy or custom build without comparable metadata | Version could not be verified; show known build and update instructions |
| Update underway | The actual current stage, with reconnect/recovery status |

Compatibility failures use the connection screen before ordinary workspace API
calls start. Missing legacy metadata is **unverified**, not evidence that the
host is current or definitely incompatible. Support explicitly tested legacy
responses; otherwise explain that the host needs an update to verify compatibility.
A failed latest-release lookup must not block a connection whose compatibility
is already known.

## One source of release information

Publish a small versioned release manifest alongside the existing GitHub release.
Use the repository's public release infrastructure; no Guaca account or new
dependency on guaca.bot is required. Publish the manifest only after both the
desktop download and backend image have passed release verification.

The manifest identifies the channel, semantic release version, full source
commit, immutable multi-platform backend image digest, desktop download, release
notes, and client/backend API compatibility. It must be machine-validated by
the release process. Do not derive a download or executable command from prose
in release notes. Sign the manifest before using it to authorize installation;
the update manager verifies it against a bundled release key and restricts image
repositories and download origins. The status UI only renders validated data.

Add release version and API generation to `/health`, preserving the existing
service/build fields for older clients. Each frontend embeds its release version
and supported backend API generations. An API generation changes for an
incompatible contract; optional additive features use capabilities. Different
commits do not imply incompatible APIs. Semantic version comparison distinguishes
older releases from newer ones; development builds and different channels are
not silently treated as upgrades or downgrades. Match published releases to their
commit as well as their version so locally modified builds remain identifiable.

The backend checks a fixed public release source and exposes a small authenticated
update-status response to either client. Cache successful checks for six hours;
support an explicit refresh with rate limiting and a bounded request timeout.
Return the checked-at time and a stale/error state instead of changing a cached
result to "up to date" on a network failure. No workspace content or credentials
go to the release source. Make automatic release checks configurable for offline
installations. Status reads remain useful even when automatic checks are disabled.

Keep `/health` and the status envelope compatible across releases so an older
page can explain a newer host. Recheck health on reconnect and when a page returns
to the foreground. An already open browser may need a reload after a backend
update; the websocket reconnect alone cannot load new JavaScript. Preserve its
unsent draft before offering that reload. A future release can only be detected
by clients that already contain this checking code; existing older installations
need one manual update to acquire it.

## Updating a host managed by the desktop

Keep the existing backup-before-replacement behavior and improve its visibility.
The update target must be compatible with the installed desktop. If the available
host release needs a newer app, lead with **Download Guaca** and then update the
host after the new app is installed. Do not turn the existing image-reference
inequality check into a downgrade operation when an older desktop reconnects.

**Review update** opens the current and target versions, affected workspace, and
current work. Before **Back up and update**, state that running work will be
interrupted and will need review after restart. Show active conversations and
coding jobs from the backend, including other clients' work. Offer **Later**.
Do not promise "update when idle" in the first version: a correct idle update
requires stopping admission of new runs and scheduled work atomically, not
watching a client-side activity count fall to zero.

The update manager owns one serialized, durable operation per host:

1. Validate the target and acquire the host update lock. Confirm the selected
   connection is the container being managed, not merely a loopback address.
2. Download and verify the target image before interrupting work.
3. Enter maintenance mode to refuse new runs and routine firings, then stop the
   backend. Existing work interruption is explicit; it is not checkpointing.
4. Copy the stopped volume to a distinct backup. Record the old image, volume,
   port, operation ID and backup reference before removing the old container.
5. Replace the container using the preserved workspace and connection settings.
   Let the backend apply its forward-only migrations on startup.
6. Verify the expected release, API compatibility, authenticated workspace
   access, and readiness. Reconnect and show **Host updated** only after these
   checks succeed. Link interrupted work to the existing recovery notices.

Render real stages: **Downloading**, **Stopping host**, **Backing up**,
**Starting updated host**, **Reconnecting**, **Complete** or **Recovery needed**.
Use indeterminate progress unless the manager has actual byte counts. Never
animate a guessed percentage. Persist enough state outside the replaced
container to explain a crash or reconnect midway through the operation. Closing
the panel does not cancel it. Prevent app exit during the critical replacement
phase until a separate long-lived manager owns the operation.

If download fails, the old host keeps running. If backup fails, cancel and
restart the old host, reporting restart failure separately. Once the new binary
can have migrated the live volume, never start the old binary on that volume.
Recovery restores the backup to a separate volume and runs the recorded old
image there, preserving the failed volume for diagnosis. Show when the backup
was made and that restoring it loses subsequent changes. Require an explicit
restore decision. Backups must be discoverable and removable in host details;
never delete the only recovery backup as an automatic cleanup step.

## Updating a remote or externally managed host

The same status and review panel appears in a browser and on desktop. The action
depends on the manager, not geographic location: a Compose container on this Mac
is externally managed, and a VPS reached over a loopback SSH tunnel is remote.

For the first release, show **View update instructions** for self-hosted VPS,
Compose, and systemd deployments. Instructions identify the verified target,
explain stopping and backing up the workspace, recreating the service with the
same volume/configuration, verifying readiness, and restoring the backup on
failure. Tailor executable commands only when the deployment configuration is
known. The current Compose file builds from source; generic `compose pull`
instructions would not update this checkout. After an operator updates the host,
the client reconnects and verifies it; an open browser offers a draft-preserving
reload when its bundle changed.

Container image replacement requires stopping and recreating the container.
[Docker Compose documents this behavior](https://docs.docker.com/reference/cli/docker/compose/up/).
The backend cannot reliably own an operation that removes its own process.
For one-click VPS updates, introduce an optional host controller outside the
container, installed explicitly by the host administrator. It manages one named
deployment and exposes only release-specific update/status/recovery operations.
Use a distinct administrative authorization boundary, not ordinary agent tools
or possession of a conversation credential. Do not give guacad the Docker
socket, SSH keys, arbitrary shell execution, or arbitrary image selection.

The controller must survive backend replacement, serialize concurrent requests,
persist progress, back up and restore volumes, preserve deployment configuration,
and report status while guacad is unavailable. The UI only offers **Back up and
update** when that controller is installed, reachable and authorized. Until then,
show the working manual path. Detection does not require installing a controller.

## Delivery and verification

Ship in this order:

1. Release manifest and consistent build/API metadata, validated against both
   artifacts. Use the same metadata in `/health`, browser bundle and desktop.
2. Shared status UI for desktop and browser, including legacy, offline, stale
   page, incompatible API, dismissal and self-hosted instructions.
3. Local update hardening: compatible targets, real durable progress, backup
   inventory, operation recovery, and verified completion.
4. Optional VPS controller and one-click remote updates as a separate change.

Required checks include a browser whose frontend and backend are both one
release behind; a newer desktop with an older host; an older desktop with a
newer host; custom/dirty builds; prerelease channels; missing or invalid release
metadata; offline release checks; two clients racing an update; and a target
changing between review and execution. The confirmed target stays pinned.

Run local-container integration tests against disposable workspaces, injecting
download, stop, backup, migration and readiness failures. Kill the manager at
each persisted stage and prove it can explain and recover the outcome. Verify
groups, files, credentials, port and access token survive a successful update,
and that no interrupted tool action is automatically replayed. Use equivalent
controller tests before offering remote updates. No verification should update
the operator's actual host.
