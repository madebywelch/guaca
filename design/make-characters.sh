#!/usr/bin/env bash
#
# Builds design/characters.html: the cast as it ships beside three redesigns,
# every creature live. The baseline is drawn by src/avatars itself; the three
# directions are prototypes in design/characters/.
#
#   ./design/make-characters.sh
#
# The page is self-contained (script, styles and the app's three fonts are
# inlined), so it opens from anywhere with no server and no network.

set -euo pipefail
cd "$(dirname "$0")/.."

ESBUILD=node_modules/.pnpm/node_modules/.bin/esbuild
[ -x "$ESBUILD" ] || { echo "esbuild not found; run pnpm install" >&2; exit 1; }

mkdir -p node_modules/.cache
"$ESBUILD" design/characters/page.ts \
  --bundle --platform=browser --format=iife --target=es2022 --log-level=warning \
  --outfile=node_modules/.cache/characters.js

node - <<'JS'
const fs = require("node:fs");
const js = fs.readFileSync("node_modules/.cache/characters.js", "utf8");
if (js.includes("</script")) throw new Error("bundle would close its own script tag");
const css = fs.readFileSync("design/characters/page.css", "utf8");
const font = (family, pkg, file) => {
  const data = fs.readFileSync(`node_modules/@fontsource-variable/${pkg}/files/${file}`).toString("base64");
  return `@font-face{font-family:"${family}";font-style:normal;font-display:swap;font-weight:100 900;src:url(data:font/woff2;base64,${data}) format("woff2");}`;
};
const fonts = [
  font("Instrument Sans Variable", "instrument-sans", "instrument-sans-latin-wght-normal.woff2"),
  font("Inter Variable", "inter", "inter-latin-wght-normal.woff2"),
  font("JetBrains Mono Variable", "jetbrains-mono", "jetbrains-mono-latin-wght-normal.woff2"),
].join("\n");
const html = [
  "<!doctype html>",
  '<html lang="en">',
  "<head>",
  '<meta charset="utf-8">',
  '<meta name="viewport" content="width=device-width">',
  "<title>Guaca characters: three redesigns</title>",
  `<style>\n${fonts}\n${css}</style>`,
  "</head>",
  "<body>",
  "<script>",
  js,
  "</script>",
  "</body>",
  "</html>",
  "",
].join("\n");
fs.writeFileSync("design/characters.html", html);
JS
echo "wrote design/characters.html"
