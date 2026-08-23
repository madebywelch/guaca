#!/usr/bin/env bash
#
# The live plugin test: whether the three vendors still publish what this build
# expects to find.
#
# CI cannot answer this. Everything in the offline suite is a scripted server
# agreeing with what this app believes MCP authorisation looks like, and the
# failure worth catching is that belief going stale: a vendor can move an
# endpoint, stop offering dynamic client registration, or start asking for a
# token on a server that used to be open, and every offline test keeps passing
# while no operator can connect.
#
#   ./scripts/plugins.sh
#
# It reaches the real internet and spends nothing: no account is authorised, no
# browser is opened and no tool is called.

set -euo pipefail

cd "$(dirname "$0")/.."

printf '\033[1m==> Live plugin test\033[0m\n'
echo "Reads public metadata from Neon, Cloudflare and Clerk. No sign-in, no spend."
echo

# The Tauri build macro reads dist/ at compile time.
[ -f dist/index.html ] || pnpm build

cargo test --manifest-path src-tauri/Cargo.toml --test plugins -- --ignored --nocapture
