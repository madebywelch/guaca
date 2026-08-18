#!/usr/bin/env bash
# Builds the desktop image an agent's computer boots, and checks the things
# about it that can be checked without one.
#
#   computer-image/build.sh              # check, then build for this Mac
#   computer-image/build.sh --check      # only the checks; needs no runtime
#   computer-image/build.sh --push       # both platforms, to a registry
#   computer-image/build.sh --digest     # move the pinned base forward
#
# The checks are the point of the `--check` mode: three of the files in here are
# copies of strings that also live in `src-tauri/src/computer/desktop.rs`, and
# the two disagreeing is not a build failure. It is a browser that opens on a
# profile no agent can read, on a machine that reports no error at all.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE_DIR="$ROOT/computer-image"
DESKTOP_RS="$ROOT/src-tauri/src/computer/desktop.rs"
BASE_TAG="debian:bookworm-slim"

# What a locally built image is called when nobody says otherwise. Not the
# published reference: that one is in `IMAGE_REF`, is written by the publishing
# workflow, and is the thing this exists to stand in for until it is real.
REF="${GUAC_COMPUTER_IMAGE:-guaca-computer:dev}"

mode=build
# Whether shellcheck actually ran, which decides what the last line claims.
linted=no
while [ $# -gt 0 ]; do
  case "$1" in
    --check) mode=check ;;
    --push) mode=push ;;
    --digest) mode=digest ;;
    -t | --tag) REF="$2"; shift ;;
    -h | --help) sed -n '2,9p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "build.sh: unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done

fail() {
  echo "build.sh: $*" >&2
  exit 1
}

# Every shell file in the image is checked for syntax here, because two of them
# are read by nothing else until a machine boots: a typo in `guaca-init` is a
# computer that will not start, reported to the operator as a container that
# exited immediately.
check_shell() {
  for script in guaca-init google-chrome novnc_proxy; do
    sh -n "$IMAGE_DIR/$script" || fail "$script is not valid POSIX sh"
  done
  if ! command -v shellcheck >/dev/null 2>&1; then
    linted=no
    return
  fi
  # Errors only, and deliberately so. This runs in two places that do not agree
  # about what is installed: a maintainer's Mac usually has no shellcheck, and
  # the runner that publishes the image always does. A gate that fires on style
  # would therefore be discovered by the first publish, having never been seen
  # by whoever wrote the script. An error is a bug in any shell; a warning here
  # is mostly a deliberate choice, since `guaca-init` runs with `set -u` and no
  # `set -e` and its signal handler is called by `trap` rather than by name.
  shellcheck -S error --shell=sh \
    "$IMAGE_DIR/guaca-init" "$IMAGE_DIR/google-chrome" "$IMAGE_DIR/novnc_proxy" \
    || fail "shellcheck found an error in one of the image's scripts"
  shellcheck -S error "${BASH_SOURCE[0]}" "$ROOT/scripts/spike-apple.sh" \
    || fail "shellcheck found an error in one of the build scripts"
  linted=yes
}

# The launcher `desktop.rs` calls, run against a stub. What matters is not that
# it starts noVNC — it cannot, here — but that the flags the call site passes
# come out as the argument order websockify expects, which is web, then listen,
# then the VNC server.
check_novnc_launcher() {
  local stub want with_flags bare refused status
  stub="$(mktemp -d "${TMPDIR:-/tmp}/guaca-check.XXXXXX")"
  printf '#!/bin/sh\necho "$@"\n' > "$stub/websockify"
  chmod +x "$stub/websockify"

  want="--web /opt/noVNC 6080 localhost:5900"
  with_flags="$(PATH="$stub:$PATH" sh "$IMAGE_DIR/novnc_proxy" \
    --vnc localhost:5900 --listen 6080 --web /opt/noVNC)"
  # The same answer with no arguments at all, because the defaults in the
  # launcher are the call site's and not noVNC's.
  bare="$(PATH="$stub:$PATH" sh "$IMAGE_DIR/novnc_proxy")"

  # And a flag it does not know refuses rather than proceeding. A dropped flag
  # takes its value with it, the value is then read as another flag, and what
  # serves on 6080 is a desktop pointed somewhere nobody asked for.
  set +e
  refused="$(PATH="$stub:$PATH" sh "$IMAGE_DIR/novnc_proxy" --cert /tmp/x 2>&1)"
  status=$?
  set -e
  rm -rf "$stub"

  [ "$with_flags" = "$want" ] || fail "novnc_proxy turned the call site's flags into: $with_flags"
  [ "$bare" = "$want" ] || fail "novnc_proxy with no arguments produced: $bare"
  [ "$status" = "2" ] || fail "novnc_proxy accepted a flag it does not understand (exit $status)"
  case "$refused" in
    *--cert*) ;;
    *) fail "novnc_proxy refused a flag without naming it: $refused" ;;
  esac
}

# One field of one file, spelled for whichever `stat` this machine has. The
# guest is Debian and `guaca-init` is written for GNU stat; a Mac has BSD's,
# which takes the same letters after a different flag.
stat_field() {
  stat -c "%$1" "$2" 2>/dev/null || stat -f "%$1" "$2"
}

# `guaca-init` actually run, against a scratch home, with the two things that
# broke a live machine asserted at the end.
#
# The first: `cp -Rpn "$SKELETON/." "$HOME_DIR/"` applies the *source
# directory's* own attributes to the destination directory, so a skeleton owned
# by root hands the volume's mount point back to root — undoing a chown made
# before the copy, with every command in the boot reporting success.
# /home/user stayed root's on a real machine and three conformance tests failed
# on `Permission denied`, none of them anywhere near the cause.
#
# Ownership needs root to demonstrate, and this rarely runs as root, so the
# check uses the *group* instead: the same `-p` copies it by the same rule, and
# an account can change a file's group to any group it belongs to. A machine
# whose account has only one group cannot show it either, and says so rather
# than passing quietly.
#
# The second: a stopped container keeps its writable layer, so the X server's
# lock file outlives a stop. The second boot found it, Xvfb refused the display,
# and the browser died on a missing one. Those paths are variables in
# `guaca-init` precisely so that this can point them at the scratch directory —
# a check that removed the real `/tmp/.X11-unix` would take a running display
# with it.
check_boot() {
  local work home skel script other group

  other=""
  for group in $(id -G); do
    if [ "$group" != "$(id -g)" ]; then
      other="$group"
      break
    fi
  done

  work="$(mktemp -d "${TMPDIR:-/tmp}/guaca-boot.XXXXXX")"
  home="$work/home"
  skel="$work/skeleton"
  mkdir -p "$skel/.local/bin" "$skel/.guac/chrome" "$home/.guac/chrome" "$work/run" \
    "$work/tmp/.X11-unix"
  printf '%s\n' 'PATH="$HOME/.local/bin:$PATH"' > "$skel/.profile"
  printf '#!/bin/sh\nexit 0\n' > "$skel/.local/bin/google-chrome"
  chmod 0755 "$skel/.local/bin/google-chrome"
  # What an agent edited on an earlier boot, and what a stopped container left:
  # a browser lock, a display lock, and the socket beside it.
  printf '%s\n' 'edited by the agent' > "$home/.profile"
  touch "$home/.guac/chrome/SingletonLock"
  touch "$work/tmp/.X0-lock" "$work/tmp/.X11-unix/X0"
  [ -n "$other" ] && chgrp "$other" "$skel" 2>/dev/null

  # The real file, with the guest's absolute paths pointed at the scratch
  # directory and this account's ids standing in for the guest's. The loop's
  # sleep is shortened so the boot ends in about a second instead of thirty.
  script="$work/guaca-init"
  sed -e "s#^HOME_DIR=/home/user#HOME_DIR=$home#" \
    -e "s#^SKELETON=/opt/guaca/home#SKELETON=$skel#" \
    -e "s#^BEAT_DIR=/run/guaca#BEAT_DIR=$work/run#" \
    -e "s#^X_LOCK=/tmp/.X0-lock#X_LOCK=$work/tmp/.X0-lock#" \
    -e "s#^X_SOCKETS=/tmp/.X11-unix#X_SOCKETS=$work/tmp/.X11-unix#" \
    -e "s#^GUEST_UID=1000#GUEST_UID=$(id -u)#" \
    -e "s#^GUEST_GID=1000#GUEST_GID=$(id -g)#" \
    -e 's#sleep 30 &#sleep 1 \&#' \
    "$IMAGE_DIR/guaca-init" > "$script"
  # BSD's `stat` takes the same format letters after a different flag, except
  # for the modification time, which GNU spells `%Y` and BSD spells `%m`.
  if ! stat -c %u . >/dev/null 2>&1; then
    sed -i.bak -e 's#stat -c %Y#stat -f %m#' -e 's#stat -c #stat -f #' "$script"
  fi

  GUAC_IDLE_SECONDS=1 sh "$script" >/dev/null 2>&1

  [ -x "$home/.local/bin/google-chrome" ] \
    || fail "the boot did not put an executable Chrome wrapper in the home"
  grep -q 'edited by the agent' "$home/.profile" \
    || fail "the boot overwrote a file the agent had already edited"
  [ ! -e "$home/.guac/chrome/SingletonLock" ] \
    || fail "the boot left Chrome's SingletonLock behind; the next browser refuses the profile"
  [ ! -e "$work/tmp/.X0-lock" ] \
    || fail "the boot left the X display lock behind; Xvfb refuses a display that is already \
locked, so a machine that has been stopped and started never gets its desktop back"
  [ ! -e "$work/tmp/.X11-unix" ] \
    || fail "the boot left the X socket directory behind; a socket in it is the display lock by \
another name"

  if [ -n "$other" ] && [ "$(stat_field g "$skel")" = "$other" ]; then
    group="$(stat_field g "$home")"
    [ "$group" = "$(id -g)" ] || fail "the boot left the home belonging to the skeleton's group \
($group) rather than the account it hands the home to: the copy applied the skeleton directory's \
attributes to the mount point, which is what the chown after it exists to undo"
  else
    echo "  (this account has one group, so the mount-point handover was not exercised;" \
      "src-tauri/tests/apple.rs asserts it on a real machine)"
  fi

  rm -rf "$work"
}

# The strings this image and the running app both hold. Each of these was a real
# failure once: two Chrome profiles on one machine, and a desktop entry pointing
# at a wrapper that was not there.
check_chrome_agreement() {
  local flag
  for flag in --no-sandbox --no-first-run --password-store=basic \
    --user-data-dir=/home/user/.guac/chrome --remote-debugging-port=9222; do
    grep -q -- "$flag" "$IMAGE_DIR/google-chrome" \
      || fail "the image's Chrome wrapper does not pass $flag"
  done
  grep -q -- '--no-sandbox' "$DESKTOP_RS" || fail "desktop.rs no longer passes --no-sandbox"
  grep -q -- '--password-store=basic' "$DESKTOP_RS" \
    || fail "desktop.rs no longer passes --password-store=basic; the image would write a cookie jar it cannot read"
  grep -q '"/home/user/.guac/chrome"' "$DESKTOP_RS" \
    || fail "desktop.rs no longer keeps the profile at /home/user/.guac/chrome"
  grep -q 'CDP_PORT: u16 = 9222' "$DESKTOP_RS" \
    || fail "desktop.rs no longer drives Chrome on 9222; the image's wrapper opens the wrong port"
  grep -q 'Exec=/home/user/.local/bin/google-chrome %U' "$DESKTOP_RS" \
    || fail "desktop.rs writes a different desktop entry than the image ships"
  grep -q 'Exec=/home/user/.local/bin/google-chrome %U' "$IMAGE_DIR/google-chrome.desktop" \
    || fail "the image's desktop entry does not launch the shim"
}

# Every command `start_desktop` runs names a path, and a path is a thing this
# image either has or does not. The Dockerfile asserts them again at build time
# on the real filesystem; this catches the case where the two files drifted
# apart without anyone building anything.
check_desktop_paths() {
  local path
  for path in /opt/noVNC/utils/novnc_proxy /opt/noVNC; do
    grep -q -- "$path" "$DESKTOP_RS" || continue
    grep -q -- "$path" "$IMAGE_DIR/Dockerfile" \
      || fail "desktop.rs uses $path and the image does not create it"
  done
  grep -q 'src-tauri/src/computer/browser.py' "$IMAGE_DIR/Dockerfile" \
    || fail "the image no longer ships the browser driver"
  [ -f "$ROOT/src-tauri/src/computer/browser.py" ] || fail "browser.py is not where the Dockerfile copies it from"
  [ -f "$ROOT/src-tauri/src/computer/sessions.py" ] || fail "sessions.py is not where the Dockerfile copies it from"
}

# Nothing the Dockerfile copies may be under an excluded path, the heavy
# directories must stay excluded, and the file must remain an exclude list.
#
# All three have cost a build. A `COPY` of something excluded fails on a missing
# file, minutes into a transfer. A context that quietly grows back to the whole
# checkout is 6.5 GB across a virtio mount. And the pattern style is not a
# preference: Apple Container 1.2.2 refuses `*` plus `!` re-inclusions during
# context transfer, naming a file rather than the pattern, before any
# instruction runs.
check_context() {
  local ignore copied excluded path rule size
  ignore="$IMAGE_DIR/Dockerfile.dockerignore"
  [ -f "$ignore" ] || fail "the image has no $ignore, so its context is the whole repository"

  if grep -qE '^[[:space:]]*[!*]' "$ignore"; then
    fail "$ignore must be a plain exclude list: Apple Container 1.2.2 refuses \`*\` and \`!\` here \
with \"changes out of order\" during context transfer"
  fi

  # Its size is a build failure too, and a mystifying one: Apple Container 1.2.2
  # ends the build immediately with `Error: unavailable: "Stream unexpectedly
  # closed."` when this file is larger than about 1.9 KB. Bisected on a live
  # runtime, content-independent: 1938 bytes builds, 2230 bytes does not. So the
  # reasoning about this file lives in the README and the file stays a list.
  size="$(wc -c < "$ignore" | tr -d '[:space:]')"
  [ "$size" -lt 1500 ] || fail "$ignore is $size bytes. Apple Container 1.2.2 fails with \
\"Stream unexpectedly closed.\" above about 1.9 KB, so this file holds entries and a pointer, and \
the prose belongs in $IMAGE_DIR/README.md"

  # Comments and blank lines out; what is left is what the builder will skip.
  excluded="$(grep -vE '^[[:space:]]*(#|$)' "$ignore")"
  # The source of each COPY, which for this Dockerfile is always its first
  # argument and always a path in the repository.
  copied="$(awk '/^COPY /{ print $2 }' "$IMAGE_DIR/Dockerfile")"

  for path in $copied; do
    for rule in $excluded; do
      case "$path" in
        "$rule" | "$rule"/*)
          fail "the Dockerfile copies $path and $ignore excludes $rule, so the build would fail \
on a file the context never carried"
          ;;
      esac
    done
  done

  # The two that matter by size. Named individually because losing either is
  # not a failure — it is a build that still works and takes minutes longer,
  # which is exactly the kind of regression nobody reports.
  for rule in src-tauri/target node_modules; do
    printf '%s\n' "$excluded" | grep -qx -- "$rule" \
      || fail "$ignore no longer keeps $rule out of the build context"
  done
}

# One pinned base, named in two files. The Dockerfile's default is what a bare
# `docker build` uses and `BASE_DIGEST` is what the publishing workflow passes
# in, so a difference between them is an image built from something other than
# what was published.
check_base_pin() {
  local pinned declared
  pinned="$(tr -d '[:space:]' < "$IMAGE_DIR/BASE_DIGEST")"
  case "$pinned" in
    "$BASE_TAG"@sha256:*) ;;
    *) fail "BASE_DIGEST should read $BASE_TAG@sha256:…, and reads: $pinned" ;;
  esac
  declared="$(sed -n 's/^ARG BASE_IMAGE=//p' "$IMAGE_DIR/Dockerfile" | tr -d '[:space:]')"
  [ "$declared" = "$pinned" ] \
    || fail "the Dockerfile pins $declared and BASE_DIGEST pins $pinned; run build.sh --digest"

  # `IMAGE_REF` is included verbatim into the binary, so a second line or a
  # comment in it becomes part of an image reference nobody typed.
  [ "$(wc -l < "$IMAGE_DIR/IMAGE_REF" | tr -d ' ')" = "1" ] \
    || fail "IMAGE_REF must hold exactly one line"
}

checks() {
  echo "==> checking the image's scripts"
  check_shell
  check_novnc_launcher
  check_boot
  echo "==> checking what the image and desktop.rs both claim"
  check_chrome_agreement
  check_desktop_paths
  echo "==> checking the build context and the pinned base"
  check_context
  check_base_pin
  # Said as what actually ran. On a machine with no linter installed, "all
  # checks passed" reads as a lint that approved these scripts, when the truth
  # is that the only place it has run is the runner that publishes the image.
  #
  # (A comment line here must not begin with the linter's own name: it reads
  # `#<name>` as a directive and refuses the file. Found by running it.)
  if [ "$linted" = yes ]; then
    echo "    all checks passed, shellcheck included"
  else
    echo "    checks passed; shellcheck: not installed, skipped (brew install shellcheck)"
  fi
}

# The digest the registry currently gives for the base tag. Read from the
# registry rather than from a local daemon so that this answers the same on a
# Mac with nothing installed, which is the machine most likely to be running it.
resolve_base_digest() {
  local token digest
  token="$(curl -fsS \
    "https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/debian:pull" \
    | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')"
  [ -n "$token" ] || fail "Docker Hub would not issue a pull token for library/debian"
  digest="$(curl -fsSI \
    -H "Authorization: Bearer $token" \
    -H 'Accept: application/vnd.oci.image.index.v1+json,application/vnd.docker.distribution.manifest.list.v2+json' \
    "https://registry-1.docker.io/v2/library/debian/manifests/${BASE_TAG#*:}" \
    | tr -d '\r' | sed -n 's/^[Dd]ocker-[Cc]ontent-[Dd]igest: //p')"
  case "$digest" in
    sha256:*) printf '%s@%s\n' "$BASE_TAG" "$digest" ;;
    *) fail "the registry did not answer with a digest for $BASE_TAG" ;;
  esac
}

repin_base() {
  local resolved current
  resolved="$(resolve_base_digest)"
  current="$(tr -d '[:space:]' < "$IMAGE_DIR/BASE_DIGEST")"
  if [ "$resolved" = "$current" ]; then
    echo "the pinned base is already the current $BASE_TAG: $resolved"
    return
  fi
  printf '%s\n' "$resolved" > "$IMAGE_DIR/BASE_DIGEST"
  # In place, so the two never have to be edited separately. The Dockerfile's
  # default is what somebody building by hand gets.
  sed -i.bak "s|^ARG BASE_IMAGE=.*|ARG BASE_IMAGE=$resolved|" "$IMAGE_DIR/Dockerfile"
  rm -f "$IMAGE_DIR/Dockerfile.bak"
  echo "pinned $resolved"
  echo "was    $current"
  echo "Commit both files, and say in the message which Debian point release this is."
}

# Apple's builder when it is there, Docker's when it is not. Apple Container
# builds for the machine it runs on, which is the only platform the spike
# needs; both platforms are the publishing workflow's business.
build_here() {
  if command -v container >/dev/null 2>&1; then
    echo "==> container build -t $REF"
    (cd "$ROOT" && container build --file computer-image/Dockerfile --tag "$REF" .)
  elif command -v docker >/dev/null 2>&1; then
    echo "==> docker buildx build -t $REF (this machine's platform)"
    (cd "$ROOT" && docker buildx build --load \
      --file computer-image/Dockerfile --tag "$REF" .)
  else
    fail "neither \`container\` nor \`docker\` is on PATH; one of them has to build this"
  fi
  echo
  echo "Point the app at it with:"
  echo "  export GUAC_COMPUTER_IMAGE=$REF"
}

# Both platforms, straight to a registry. A multi-platform image cannot be
# loaded into a local daemon at all — there is nowhere to put two of them — so
# this pushes or it does nothing.
build_and_push() {
  command -v docker >/dev/null 2>&1 || fail "--push needs docker buildx"
  case "$REF" in
    */*) ;;
    *) fail "--push needs a registry reference, not $REF (try -t ghcr.io/<owner>/guaca-computer:0.1.0)" ;;
  esac
  echo "==> docker buildx build --platform linux/amd64,linux/arm64 --push -t $REF"
  (cd "$ROOT" && docker buildx build --push \
    --platform linux/amd64,linux/arm64 \
    --build-arg "BASE_IMAGE=$(tr -d '[:space:]' < "$IMAGE_DIR/BASE_DIGEST")" \
    --file computer-image/Dockerfile --tag "$REF" .)
}

case "$mode" in
  check) checks ;;
  digest) repin_base ;;
  push) checks; build_and_push ;;
  build) checks; build_here ;;
esac
