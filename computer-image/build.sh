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
  if command -v shellcheck >/dev/null 2>&1; then
    # `guaca-init` runs as PID 1 with `set -u` and no `set -e`, so unchecked
    # commands are deliberate; SC2317 fires on the signal handler, which is
    # called by `trap` rather than by name.
    shellcheck --shell=sh --exclude=SC2317 \
      "$IMAGE_DIR/guaca-init" "$IMAGE_DIR/google-chrome" "$IMAGE_DIR/novnc_proxy" \
      || fail "shellcheck refused one of the image's scripts"
    shellcheck "${BASH_SOURCE[0]}" "$ROOT/scripts/spike-apple.sh" \
      || fail "shellcheck refused one of the build scripts"
  else
    echo "  (shellcheck is not installed; syntax was checked with sh -n only)"
  fi
}

# The launcher `desktop.rs` calls, run against a stub. What matters is not that
# it starts noVNC — it cannot, here — but that the flags the call site passes
# come out as the argument order websockify expects, which is web, then listen,
# then the VNC server.
check_novnc_launcher() {
  local stub want with_flags bare
  stub="$(mktemp -d "${TMPDIR:-/tmp}/guaca-check.XXXXXX")"
  printf '#!/bin/sh\necho "$@"\n' > "$stub/websockify"
  chmod +x "$stub/websockify"

  want="--web /opt/noVNC 6080 localhost:5900"
  with_flags="$(PATH="$stub:$PATH" sh "$IMAGE_DIR/novnc_proxy" \
    --vnc localhost:5900 --listen 6080 --web /opt/noVNC)"
  # The same answer with no arguments at all, because the defaults in the
  # launcher are the call site's and not noVNC's.
  bare="$(PATH="$stub:$PATH" sh "$IMAGE_DIR/novnc_proxy")"
  rm -rf "$stub"

  [ "$with_flags" = "$want" ] || fail "novnc_proxy turned the call site's flags into: $with_flags"
  [ "$bare" = "$want" ] || fail "novnc_proxy with no arguments produced: $bare"
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
  echo "==> checking what the image and desktop.rs both claim"
  check_chrome_agreement
  check_desktop_paths
  echo "==> checking the pinned base"
  check_base_pin
  echo "    all checks passed"
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
