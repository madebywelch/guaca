#!/usr/bin/env bash

set -euo pipefail

cd "$(dirname "$0")/.."

CREDENTIALS="${GUAC_SUBSCRIPTION_JSON:-$HOME/Library/Application Support/com.madebywelch.guac/subscription.json}"

if [ ! -f "$CREDENTIALS" ]; then
  echo "no ChatGPT sign-in at $CREDENTIALS" >&2
  echo "Open Guaca, go to Settings -> Provider, and press Sign in." >&2
  exit 1
fi

printf '\033[1m==> Live subscription test\033[0m\n'
echo "This makes a real model call against your ChatGPT plan."
echo

[ -f dist/index.html ] || pnpm build

GUAC_SUBSCRIPTION_JSON="$CREDENTIALS" \
  cargo test --manifest-path src-tauri/Cargo.toml --test subscription \
  -- --ignored --nocapture
