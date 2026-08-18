#!/usr/bin/env bash
#
# The Apple Container spike: everything this project believes about a runtime it
# was written without, checked against the runtime.
#
#   ./scripts/spike-apple.sh
#
# It builds the desktop image, runs the conformance suite in
# `src-tauri/tests/apple.rs`, and then prints the handful of things a test
# cannot assert because nobody knows yet what the answer looks like — the raw
# text of `container --version`, where labels land in `container ls --format
# json`, what the runtime says when a name is taken. Those are printed for a
# person to read against the comments in `src-tauri/src/computer/apple.rs`,
# which is where the guesses are marked.
#
# Nothing here runs in CI. It needs Apple Container, which installs on nothing
# but macOS 26 on Apple silicon, and it makes real VMs.
#
# Before running, so the network measurements mean something:
#
#   nc -l 8765            # in another terminal: a service on Mac loopback
#   export GUAC_SPIKE_LAN=<an address on your network>
#
set -euo pipefail

cd "$(dirname "$0")/.."

# Finder-launched apps do not inherit Homebrew's directory and neither does
# every shell; the signed package puts the binary here.
if ! command -v container >/dev/null 2>&1; then
  if [ -x /usr/local/bin/container ]; then
    PATH="/usr/local/bin:$PATH"
    export PATH
  else
    cat >&2 <<'MISSING'
spike-apple.sh: no `container` binary at /usr/local/bin/container or on PATH.

Apple Container needs macOS 26 on Apple silicon. Install the signed package
from github.com/apple/container/releases, then run this again.
MISSING
    exit 1
  fi
fi

step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

# The name every probe resource shares. Not the shape the app uses
# (`guac-<8 hex>`), so nothing here can collide with a real machine.
PROBE=guac-spike-probe

cleanup() {
  container delete --force "$PROBE" >/dev/null 2>&1 || true
  container volume delete "$PROBE" >/dev/null 2>&1 || true
  container network delete "$PROBE" >/dev/null 2>&1 || true
}
trap cleanup EXIT

step "Starting the runtime"
# The flag is not optional: without it a first run asks for permission to
# install a kernel, and a child spawned with no terminal waits on that prompt.
container system start --enable-kernel-install

step "The desktop image"
if [ -n "${GUAC_COMPUTER_IMAGE:-}" ]; then
  echo "using the image already named by GUAC_COMPUTER_IMAGE: $GUAC_COMPUTER_IMAGE"
else
  computer-image/build.sh --check
  echo "building guaca-computer:spike (this is a desktop; the first build is not quick)"
  container build --file computer-image/Dockerfile --tag guaca-computer:spike .
  GUAC_COMPUTER_IMAGE=guaca-computer:spike
fi
export GUAC_COMPUTER_IMAGE

step "The conformance suite"
cat <<'MAPPING'
Each test is one item of "Provider smoke tests" in docs/LOCAL_COMPUTERS.md:

  1  a_computer_is_made_as_a_container_a_volume_and_a_network
  2  a_command_keeps_its_two_streams_and_its_exit_code
  3  a_credential_reaches_one_command_and_nothing_else
  4  a_binary_file_arrives_on_a_machine_byte_for_byte
  5  the_desktop_is_watchable_through_the_loopback_viewer
  6  the_browser_is_driven_over_its_remote_interface_and_the_screen_photographed
  7  a_home_file_and_the_browser_profile_survive_a_sleep
  8  a_machine_nobody_is_using_stops_itself_and_keeps_its_disk
  9  destroying_a_computer_takes_all_three_of_its_resources
 10  one_agent_cannot_reach_another_agents_desktop

MAPPING

# One at a time: each test makes a VM with four vCPUs and a desktop in it, and
# ten of those at once is a Mac that stops responding rather than a faster run.
set +e
cargo test --manifest-path src-tauri/Cargo.toml --test apple -- \
  --ignored --nocapture --test-threads=1
suite=$?
set -e

step "CONFIRM THESE AGAINST computer/apple.rs COMMENTS"
cat <<'WHY'
Everything below is a guess in apple.rs marked as one. The tests cannot assert
them because the assertion would be the guess. Read each against the file.
WHY

probe() {
  local what="$1"
  shift
  printf '\n\033[1m--- %s\033[0m\n$ container %s\n' "$what" "$*"
  set +e
  local out status
  out="$(container "$@" 2>&1)"
  status=$?
  set -e
  printf '%s\n(exit %d)\n' "$(printf '%s' "$out" | head -c 4000)" "$status"
}

# `parse_version` takes the first three dotted numbers in whatever this prints,
# and `supported()` refuses anything below 1.2.2 or at 2.0.0 and above.
probe "the exact wording of --version, which parse_version reads" --version

# `probe_runtime` treats a zero exit as "the service is running" and anything
# else as stopped.
probe "what a running service says, and with which exit code" system status

# `already_exists()` matches "exists" or "already in use", and `name_taken`
# refuses to adopt anything that is not labelled as this computer's.
probe "creating a network with labels" \
  network create --label guac=true --label guac.installation=spike \
  --label guac.computer=00000000-0000-0000-0000-000000000000 "$PROBE"
probe "creating the same network again: the wording already_exists() matches" \
  network create --label guac=true "$PROBE"
probe "where a network's labels land, which left_by() reads" network inspect "$PROBE"

# `volume create -s` is the only moment the home quota can be enforced.
probe "creating a volume with a quota and labels" \
  volume create -s 20G --label guac=true --label guac.installation=spike \
  --label guac.computer=00000000-0000-0000-0000-000000000000 "$PROBE"
probe "whether volume inspect answers with an array or an object" volume inspect "$PROBE"

# `missing()` matches "not found" or "no such", and reading a real error as an
# absence is what would throw away a disk.
probe "the wording missing() matches, on something that was never made" \
  inspect guac-spike-nothing-here

# **`configuration.id` must be the container's name.** `read_owned` returns this
# field and the sweep deletes what it does not recognise; if this is a generated
# identifier rather than `guac-…`, every live machine looks unclaimed.
probe "creating a labelled container, to read its ls entry" \
  create --name "$PROBE" --network "$PROBE" \
  --mount "type=volume,source=$PROBE,target=/home/user" \
  --cpus 2 --memory 2G --shm-size 1G \
  --label guac=true --label guac.installation=spike \
  --label guac.computer=00000000-0000-0000-0000-000000000000 \
  --env GUAC_IDLE_SECONDS=900 "$GUAC_COMPUTER_IMAGE"
probe "configuration.id and configuration.labels, which read_owned() reads" \
  ls --all --format json

# `read_state` maps the words here onto running/asleep and refuses anything it
# does not know; `read_address` reads networks[0].address for the viewer.
probe "the status word and the address of a started container" start "$PROBE"
probe "inspect on a running container: status, networks[0].address" inspect "$PROBE"
probe "stop with a grace period, which is the browser's chance to save" \
  stop --time 10 "$PROBE"
probe "inspect on a stopped container: the word read_state must know" inspect "$PROBE"

step "Record these in docs/LOCAL_COMPUTERS.md under \"Spike results\""
cat <<'CHECKLIST'
The ten smoke items, each pass or fail with what it said:

  1. Create from the pinned image: container, volume and network all made, and
     `container ls --format json` names the container by the name create chose.
  2. Exec keeps stdout, stderr and the exit code apart, runs as `user` in
     /home/user, and finds ~/.local/bin ahead of /usr/bin.
  3. A sentinel reaches one command and is absent from the next, from
     /proc/1/environ, and from `container inspect`.
  4. A 300 KiB binary placed in chunks reads back byte for byte.
  5. noVNC loads through the loopback viewer proxy.
  6. Chromium is driven over CDP by browser.py, the screen photographs at
     1280x800, and xdotool moves the pointer.
  7. A home file and the Chrome profile survive stop/start, and the browser
     opens again afterwards (SingletonLock was cleared).
  8. With GUAC_IDLE_SECONDS=20 and nothing touching the heartbeat, the machine
     stops itself and keeps its volume.
  9. Destroy removes container, volume and network, and the viewer says the
     machine is gone.
 10. A second agent cannot reach the first's 6080.

And the measurements test 10 printed, which the document has to state honestly:

  - another agent's guest address and noVNC port  (release blocker: must fail)
  - a service bound only to Mac loopback          (record what happened)
  - a service bound to the Mac's LAN address      (record what happened)
  - another LAN address                           (record what happened)
  - public HTTP, HTTPS, DNS, arbitrary TCP        (expected reachable)

Also worth recording, because constants in the code depend on them:

  - whether 4 GiB of memory was needed or 3 GiB would do (docs say the spike
    must measure this before the constants are committed);
  - how long a first boot takes, and a wake from stopped;
  - what the image weighs, pulled and on disk.

What this spike has already settled about the builder itself, kept in
computer-image/README.md because each was found the hard way:

  - `container build` does read computer-image/Dockerfile.dockerignore: the
    context transfer drops to 45 B.
  - It refuses `*` plus `!` re-inclusions, failing with
    `changes out of order: "computer-image/google-chrome" ""` before any
    instruction runs. Hence a plain exclude list.
  - It refuses that file at all above about 1.9 KB, failing with
    `Error: unavailable: "Stream unexpectedly closed."`, which says nothing
    about ignore files. Bisected and content-independent: 1938 B builds, 2230 B
    does not. Hence entries and a pointer, with the prose in the README.
  - A stopped container keeps its writable layer: a file written to /tmp
    survives a stop and a start, and /tmp is not a separate mount. So a stale
    /tmp/.X0-lock stopped Xvfb on the second boot and the desktop never came
    back. PID 1 clears it and the socket directory beside it on every boot, and
    the provider mounts a tmpfs at /tmp as well.
CHECKLIST

if [ "$suite" -ne 0 ]; then
  echo
  echo "the conformance suite failed (exit $suite); the probes above still ran" >&2
fi
exit "$suite"
