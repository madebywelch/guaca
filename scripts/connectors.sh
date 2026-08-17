#!/usr/bin/env bash
#
# The live connector tests: whether a credential the operator pasted in actually
# reaches the service it is for, from a real machine.
#
# CI cannot answer this. The scripted suites prove the secret goes into a
# sandbox's environment and nowhere else; only the real API can show that an
# agent handed the key can do the work the key is for.
#
#   ./scripts/connectors.sh          every connector scenario
#   ./scripts/connectors.sh mistral  just the ones whose name matches
#
# The keys come from the app's own settings and database, so what is measured is
# what the operator is actually running. A sandbox is created and released; the
# API calls cost a few cents.

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

# --nocapture so the extracted text is printed whether or not it passed: a pass
# with unreadable output is still worth looking at, and a failure is
# unactionable without seeing what came back.
cargo test --manifest-path src-tauri/Cargo.toml --test connectors \
  "${1:-}" -- --ignored --nocapture --test-threads=1
