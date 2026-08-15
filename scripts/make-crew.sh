#!/usr/bin/env bash
#
# Redraws docs/img/crew.svg from the app's own character catalog.
#
# Run it after changing the avatars, so the front page cannot quietly drift
# away from what the app actually looks like.

set -euo pipefail
cd "$(dirname "$0")/.."

ESBUILD=node_modules/.pnpm/node_modules/.bin/esbuild
[ -x "$ESBUILD" ] || { echo "esbuild not found; run pnpm install" >&2; exit 1; }

# Bundled to one file, react included: pnpm's store is not a flat node_modules,
# so a bundle left to resolve react at runtime cannot find it. Written inside
# the project for the same reason.
mkdir -p docs/img
# CommonJS, because react-dom's server build reaches for `require` at load time
# and an ESM bundle cannot give it one.
"$ESBUILD" scripts/make-crew.tsx \
  --bundle --platform=node --format=cjs --jsx=automatic --log-level=warning \
  --outfile=node_modules/.cache/make-crew.cjs

node node_modules/.cache/make-crew.cjs > docs/img/crew.svg
echo "wrote docs/img/crew.svg"
