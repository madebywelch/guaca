#!/usr/bin/env bash
#
# Builds design/wordmarks.html and the SVGs beside this script: five wordmark
# prototypes, each large and at rail size on both surfaces, and all five in a
# row beside the one that ships. What each one is, and why, is the docstring
# at the top of wordmarks.py.
#
#   ./design/wordmarks/make.sh
#
# The page is self-contained (fonts inlined), so it opens from anywhere with no
# server and no network. Needs node_modules, for the app's fonts and for node
# to sample the cast's silhouettes, and uv, which brings the script's own
# fontTools.

set -euo pipefail
cd "$(dirname "$0")/../.."

[ -d node_modules/@fontsource-variable ] || { echo "fonts not found; run pnpm install" >&2; exit 1; }
command -v uv >/dev/null || { echo "uv not found; see https://docs.astral.sh/uv/" >&2; exit 1; }

mkdir -p node_modules/.cache
# 96 steps is a multiple of 16, so every one of the octagon's corners is a
# sample rather than something cut off between two of them.
node --experimental-strip-types --no-warnings --input-type=module -e '
import { writeFileSync } from "node:fs";
import { SILHOUETTES } from "./src/avatars/silhouette.ts";
const STEPS = 96;
const out = {};
for (const [name, radius] of Object.entries(SILHOUETTES)) {
  out[name] = Array.from({ length: STEPS }, (_, i) => {
    const a = (i / STEPS) * Math.PI * 2;
    const r = radius(a) * 50;
    return [+(50 + r * Math.cos(a)).toFixed(1), +(50 + r * Math.sin(a)).toFixed(1)];
  });
}
writeFileSync("node_modules/.cache/silhouettes.json", JSON.stringify(out));
'

uv run --quiet design/wordmarks/wordmarks.py
