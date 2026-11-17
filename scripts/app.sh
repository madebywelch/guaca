#!/usr/bin/env bash

set -euo pipefail

cd "$(dirname "$0")/.."

if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
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
