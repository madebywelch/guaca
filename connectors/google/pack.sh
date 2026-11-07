#!/bin/sh
set -eu
cd "$(dirname "$0")"
pnpm exec esbuild src/account/document.tsx src/account/pages.tsx src/components/chrome.tsx src/content.ts --outdir=src --outbase=src --format=esm --jsx=automatic
pnpm exec tsc --declaration --emitDeclarationOnly --outDir src --rootDir src --jsx react-jsx --module esnext --moduleResolution bundler --target es2022 --skipLibCheck src/account/document.tsx src/account/pages.tsx src/components/chrome.tsx src/content.ts
pnpm pack --pack-destination "${1:-.}"
