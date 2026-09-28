#!/usr/bin/env bash
#
# Builds design/brandmark.html: three ways the name folds away and leaves the
# cast in the title bar, every creature live and drawn by src/avatars itself.
#
#   ./design/brandmark/make.sh
#
# The page is self-contained (script, the app's stylesheet and its three fonts
# are inlined), so it opens from anywhere with no server and no network.

set -euo pipefail
cd "$(dirname "$0")/../.."

ESBUILD=node_modules/.pnpm/node_modules/.bin/esbuild
[ -x "$ESBUILD" ] || { echo "esbuild not found; run pnpm install" >&2; exit 1; }

mkdir -p node_modules/.cache
"$ESBUILD" design/brandmark/page.tsx \
  --bundle --platform=browser --format=iife --target=es2022 --log-level=warning \
  --jsx=automatic --define:process.env.NODE_ENV='"production"' --minify \
  --outfile=node_modules/.cache/brandmark.js

node - <<'JS'
const fs = require("node:fs");
const js = fs.readFileSync("node_modules/.cache/brandmark.js", "utf8");
if (js.includes("</script")) throw new Error("bundle would close its own script tag");
/* The app's stylesheet, less the one line that needs Tailwind's compiler:
   nothing a creature is drawn with is a utility class. */
const app = fs.readFileSync("src/styles.css", "utf8").replace(/^@import "tailwindcss";\n/, "");
if (app.includes('@import "tailwindcss"')) throw new Error("styles.css no longer starts with the Tailwind import");
const page = fs.readFileSync("design/brandmark/page.css", "utf8");
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
  "<title>Guaca: the brand mark, folding into the cast</title>",
  `<style>\n${fonts}\n${app}\n${page}</style>`,
  "</head>",
  "<body>",
  '<div id="root"></div>',
  "<script>",
  js,
  "</script>",
  "</body>",
  "</html>",
  "",
].join("\n");
fs.writeFileSync("design/brandmark.html", html);
JS
echo "wrote design/brandmark.html"
