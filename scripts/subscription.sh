#!/usr/bin/env bash
#
# The live subscription test: whether a real ChatGPT plan still answers a real
# turn through this app.
#
# CI cannot answer this. Everything in the Rust suite is a stub agreeing with
# the shape this app believes the Responses protocol has, and the failure worth
# catching is that belief going stale: OpenAI can rename a stream event, require
# a field or retire a model slug, and every offline test keeps passing while no
# agent can speak.
#
#   ./scripts/subscription.sh
#
# It reads the sign-in from the app's own config directory, so what is measured
# is what the operator is actually running. Point GUAC_SUBSCRIPTION_JSON at a
# file to test a different one. It spends a few hundred tokens of plan quota.

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

# The Tauri build macro reads dist/ at compile time.
[ -f dist/index.html ] || pnpm build

GUAC_SUBSCRIPTION_JSON="$CREDENTIALS" \
  cargo test --manifest-path src-tauri/Cargo.toml --test subscription \
  -- --ignored --nocapture
