#!/usr/bin/env bash

set -euo pipefail

cd "$(dirname "$0")/.."

CONFIG="$HOME/Library/Application Support/com.madebywelch.guac/config.json"

if [ ! -f "$CONFIG" ]; then
  echo "no settings at $CONFIG; open Guaca and set an endpoint and key first" >&2
  exit 1
fi

read_model='
import json, os, sys
i = json.load(open(sys.argv[1]))["inference"]
chatgpt = i.get("provider") == "chatgpt"
model = i.get("subscriptionModel") if chatgpt else i["defaultModel"]
if i.get("provider", "compatible") == "compatible":
    model = os.environ.get("GUACA_TEST_MODEL", "").strip() or model
print(model, end="")
print(" (ChatGPT subscription)" if chatgpt else "")
'
model=$(python3 -c "$read_model" "$CONFIG")
printf '\033[1m==> Live evals against %s\033[0m\n' "$model"
echo "These make real model calls and cost real money. Ctrl-C now if that is a surprise."
echo

cargo test --manifest-path src-tauri/Cargo.toml --test evals \
  "${1:-live::}" -- --ignored --nocapture --test-threads=1
