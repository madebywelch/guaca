#!/usr/bin/env bash
# Build a desktop candidate pinned to its container image. No publishing.
# Set GUACA_BACKEND_IMAGE to a published digest for a distributable app.
set -euo pipefail
cd "$(dirname "$0")/.."
./scripts/ci.sh
COMMIT="$(git rev-parse --short=12 HEAD)"
if [ -z "${GUACA_BACKEND_IMAGE:-}" ]; then
  export GUACA_BACKEND_IMAGE="guacad:$COMMIT"
  docker build --build-arg "GUACA_COMMIT=$COMMIT" -t "$GUACA_BACKEND_IMAGE" .
  GUACA_TEST_IMAGE="$GUACA_BACKEND_IMAGE" cargo test --manifest-path src-tauri/Cargo.toml \
    --no-default-features --features server --lib \
    host::tests::docker_host_survives_client_and_container_restarts -- --ignored
else
  case "$GUACA_BACKEND_IMAGE" in
    *@sha256:*) ;;
    *) echo 'A distributable candidate needs GUACA_BACKEND_IMAGE pinned by digest.' >&2; exit 1 ;;
  esac
fi
pnpm tauri build --bundles app
printf '\nCandidate: src-tauri/target/release/bundle/macos/Guaca.app\nBackend: %s\n' "$GUACA_BACKEND_IMAGE"
