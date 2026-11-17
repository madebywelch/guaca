#!/usr/bin/env bash

set -euo pipefail
cd "$(dirname "$0")/.."

ESBUILD=node_modules/.pnpm/node_modules/.bin/esbuild
[ -x "$ESBUILD" ] || { echo "esbuild not found; run pnpm install" >&2; exit 1; }

mkdir -p docs/img
"$ESBUILD" scripts/make-crew.ts \
  --bundle --platform=node --format=cjs --log-level=warning \
  --outfile=node_modules/.cache/make-crew.cjs

node node_modules/.cache/make-crew.cjs > node_modules/.cache/crew.svg
mv node_modules/.cache/crew.svg docs/img/crew.svg
echo "wrote docs/img/crew.svg"
