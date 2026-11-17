#!/usr/bin/env bash

set -euo pipefail

cd "$(dirname "$0")/.."

CONFIG="$HOME/Library/Application Support/com.madebywelch.guac/config.json"

if [ ! -f "$CONFIG" ]; then
  echo "no settings at $CONFIG; open Guaca and set an endpoint and key first" >&2
  exit 1
fi

printf '\033[1m==> A live crew, against your own key\033[0m\n'
echo "These make real model calls and cost real money. Ctrl-C now if that is a surprise."
echo

cargo test --manifest-path src-tauri/Cargo.toml --test crew \
  -- --ignored --nocapture --test-threads=1
