#!/usr/bin/env bash
#
# Builds scripts/caricature-review.html: the characters as they are at a git
# ref beside the characters as they are in the working tree, drawn live by
# both versions of src/avatars.
#
#   ./scripts/make-caricature-review.sh [ref]     # ref defaults to HEAD
#
# The page is self-contained, so it can be opened from anywhere and sent to
# anyone. Nothing on it is a sketch: both halves are the app's own geometry.

set -euo pipefail
cd "$(dirname "$0")/.."

ESBUILD=node_modules/.pnpm/node_modules/.bin/esbuild
[ -x "$ESBUILD" ] || { echo "esbuild not found; run pnpm install" >&2; exit 1; }

REF="${1:-HEAD}"
BEFORE=node_modules/.cache/avatars-before
mkdir -p "$BEFORE"
for f in silhouette form eyes moods catalog; do
  git show "$REF:src/avatars/$f.ts" > "$BEFORE/$f.ts"
done

# Bundled rather than run through a loader: pnpm's store is not a flat
# node_modules, so a bundle left to resolve anything at runtime cannot find it.
"$ESBUILD" scripts/caricature-review.ts \
  --bundle --platform=browser --format=iife --log-level=warning \
  --alias:before="./$BEFORE" \
  --outfile=node_modules/.cache/caricature-review.js

node - <<'JS'
const fs = require("node:fs");
const js = fs.readFileSync("node_modules/.cache/caricature-review.js", "utf8");
if (js.includes("</script")) throw new Error("bundle would close its own script tag");
const html = [
  "<!doctype html>",
  '<html lang="en">',
  "<head>",
  '<meta charset="utf-8">',
  '<meta name="viewport" content="width=device-width">',
  "<title>Guaca characters: the caricature pass</title>",
  "</head>",
  "<body>",
  "<script>",
  js,
  "</script>",
  "</body>",
  "</html>",
  "",
].join("\n");
fs.writeFileSync("scripts/caricature-review.html", html);
JS
echo "wrote scripts/caricature-review.html"
