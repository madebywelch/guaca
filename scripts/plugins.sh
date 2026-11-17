#!/usr/bin/env bash

set -euo pipefail

cd "$(dirname "$0")/.."

printf '\033[1m==> Live plugin test\033[0m\n'
echo "Reads public metadata from every server on the list. No sign-in, no spend."
echo

[ -f dist/index.html ] || pnpm build

cargo test --manifest-path src-tauri/Cargo.toml --test plugins -- --ignored --nocapture
