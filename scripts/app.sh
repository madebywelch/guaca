#!/usr/bin/env bash
#
# Run the desktop app from this checkout, beside an installed Guaca rather
# than inside it.
#
#   pnpm app                        this script
#   GUAC_LOG=guac=debug pnpm app    with the lines you launched it to read
#
# Two things make it a separate app, and both are here because the default was
# the installed one.
#
# The identity. `tauri.dev.conf.json` gives this build its own bundle
# identifier. The local host's container, its data volume and its update
# journal are all named from the identifier, so under the installed app's, a
# development build's "On this Mac" starts, migrates and updates the operator's
# own workspace.
#
# The host image. Unset, `host::IMAGE` names the versioned GHCR tag, which a
# source build may not be allowed to pull and is not this checkout's daemon
# when it can. This builds the checkout's own and compiles its name in. The tag
# is the image's ID, so any change to what goes into the image is a new tag,
# the app offers it as a host update, and that update is the path an operator
# takes. Nothing changed is every layer cached and the same tag.
#
# Without Docker it still runs: "On this Mac" is unavailable, and "Remote host"
# works as it does in the installed app.

set -euo pipefail

cd "$(dirname "$0")/.."

if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  # The same qualifier vite.config.ts gives a dev server's About, so the page
  # and the daemon's /health name one build.
  COMMIT="$(git rev-parse --short=7 HEAD)"
  [ -z "$(git status --porcelain --untracked-files=no)" ] || COMMIT="$COMMIT-dirty"
  printf '==> Building this checkout'"'"'s host image on Docker context %s\n' "$(docker context show)"
  IID="$(mktemp)"
  trap 'rm -f "$IID"' EXIT
  docker build \
    --build-arg "GUACA_VERSION=$(node -p 'require("./package.json").version')" \
    --build-arg "GUACA_COMMIT=$COMMIT" \
    --iidfile "$IID" .
  ID="$(sed 's/^sha256://' "$IID")"
  export GUACA_BACKEND_IMAGE="guacad:dev-${ID:0:12}"
  docker tag "$ID" "$GUACA_BACKEND_IMAGE"
  printf '==> Host image %s\n' "$GUACA_BACKEND_IMAGE"
else
  printf '==> Docker is not ready: "On this Mac" is unavailable in this run. "Remote host" works.\n' >&2
fi

exec pnpm tauri dev --config src-tauri/tauri.dev.conf.json "$@"
