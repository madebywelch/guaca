#!/usr/bin/env bash

set -euo pipefail
cd "$(dirname "$0")/../.."

[ -d node_modules/@fontsource-variable ] || { echo "fonts not found; run pnpm install" >&2; exit 1; }
if command -v uv >/dev/null; then
  uv=(uv)
elif command -v pipx >/dev/null; then
  uv=(pipx run uv)
else
  echo "neither uv nor pipx found; see https://docs.astral.sh/uv/" >&2
  exit 1
fi

mkdir -p node_modules/.cache
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

"${uv[@]}" run --quiet --python ">=3.11" --with "fonttools>=4.60" --with "brotli>=1.1" design/wordmarks/wordmarks.py "$@"
