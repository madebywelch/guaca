#!/usr/bin/env bash
#
# Redraws docs/img/crew.svg from the app's own drawing code.
#
# Run it after changing the avatars, so the front page cannot quietly drift
# away from what the app actually looks like. It did once: the strip was still
# showing a cast of vegetables three casts after they were deleted.

set -euo pipefail
cd "$(dirname "$0")/.."

ESBUILD=node_modules/.pnpm/node_modules/.bin/esbuild
[ -x "$ESBUILD" ] || { echo "esbuild not found; run pnpm install" >&2; exit 1; }

# Bundled rather than run through a loader: pnpm's store is not a flat
# node_modules, so a bundle left to resolve anything at runtime cannot find it.
# Written inside the project for the same reason.
mkdir -p docs/img
"$ESBUILD" scripts/make-crew.ts \
  --bundle --platform=node --format=cjs --log-level=warning \
  --outfile=node_modules/.cache/make-crew.cjs

# A failed render must leave the last good illustration intact.
node node_modules/.cache/make-crew.cjs > node_modules/.cache/crew.svg
mv node_modules/.cache/crew.svg docs/img/crew.svg
echo "wrote docs/img/crew.svg"
