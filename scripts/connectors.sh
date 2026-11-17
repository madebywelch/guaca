#!/usr/bin/env bash

set -euo pipefail

cd "$(dirname "$0")/.."

CONFIG="$HOME/Library/Application Support/com.madebywelch.guac/config.json"

if [ ! -f "$CONFIG" ]; then
  echo "no settings at $CONFIG; open Guaca and add an E2B key first" >&2
  exit 1
fi

printf '\033[1m==> Live connector tests\033[0m\n'
echo "These start a real machine and call real APIs. Ctrl-C now if that is a surprise."
echo

cargo test --manifest-path src-tauri/Cargo.toml --test connectors \
  "${1:-}" -- --ignored --nocapture --test-threads=1
