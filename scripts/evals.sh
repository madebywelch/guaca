#!/usr/bin/env bash
#
# The live evals: whether a real model, reading the real prompts, communicates
# like something an operator would want to watch.
#
# CI cannot answer this. The scripted evals check that the runtime contains a
# bad habit; only a real model can show whether the prompts encourage one. So
# these run by hand, after a prompt change, and they cost a few cents.
#
#   ./scripts/evals.sh              every live scenario
#   ./scripts/evals.sh delegation   just the ones whose name matches
#
# Defaults come from the app's settings. GUACA_TEST_MODEL and, for OpenRouter,
# OPENROUTER_API_KEY override this test process without editing those settings.

set -euo pipefail

cd "$(dirname "$0")/.."

CONFIG="$HOME/Library/Application Support/com.madebywelch.guac/config.json"

if [ ! -f "$CONFIG" ]; then
  echo "no settings at $CONFIG; open Guaca and set an endpoint and key first" >&2
  exit 1
fi

# The model belonging to whichever provider is chosen. Printing the endpoint's
# model while a subscription is paying names one the run will never call.
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

# --nocapture so the conversation each scenario produced is printed whether or
# not it passed: a passing eval with an ugly transcript is still worth reading,
# and a failing one is unactionable without it.
#
# One thread, because several crews thinking at once against the same endpoint
# makes the timings meaningless and the output interleave.
cargo test --manifest-path src-tauri/Cargo.toml --test evals \
  "${1:-live::}" -- --ignored --nocapture --test-threads=1
