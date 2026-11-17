#!/usr/bin/env bash

set -euo pipefail

cd "$(dirname "$0")/.."

step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

ensure_dist() {
  if [ ! -f dist/index.html ]; then
    step "Building frontend (required before the Rust crate compiles)"
    pnpm build
  fi
}

run_grants() {
  step "No comment, suppression or prose file without the owner's grant"
  node --test scripts/check-grants.test.mjs
  node scripts/check-grants.mjs
}

run_assets() {
  step "Icons and tray glyphs"
  python3 scripts/check-icons.py
}

run_scripts() {
  step "Shell checks"
  bash scripts/versions.test.sh
  node --test scripts/install-image.test.mjs
  bash scripts/install-host.test.sh
  bash deploy/box/install.test.sh
  python3 scripts/check-docker-deps.py

  step "Probe evidence (no app or screen)"
  python3 scripts/probe.test.py

  step "Release disk image layout"
  python3 scripts/package-dmg.test.py

  step "CI runs every part of this gate"
  python3 scripts/check-ci-parts.test.py
  python3 scripts/check-ci-parts.py
}

run_formal() {
  step "Conversation recovery model"
  bash scripts/formal.sh
}

run_e2e() {
  step "End-to-end harness"
  python3 scripts/verify.test.py

  step "End-to-end scenarios"
  python3 scripts/verify.py --build
}

run_connector() {
  step "Self-hosted Google connector"
  (cd connectors/google && pnpm install --frozen-lockfile && pnpm typecheck && pnpm test)
}

run_web() {
  step "Operator copy"
  node --test scripts/check-operator-copy.test.mjs
  node scripts/check-operator-copy.mjs

  step "Lint and format"
  pnpm check

  step "Typecheck and build"
  pnpm build

  step "No probe bridge in the shipped bundle"
  if grep -rl "__guacaProbe" dist; then
    echo "dist/ contains the probe bridge; only a --mode probe build may" >&2
    exit 1
  fi

  step "Release metadata contract"
  node --test scripts/release-manifest.test.mjs

  step "Frontend tests"
  pnpm test
}

run_desktop() {
  ensure_dist

  step "Rust format"
  cargo fmt --manifest-path src-tauri/Cargo.toml -- --check

  step "Clippy"
  cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings

  step "Rust tests"
  cargo test --manifest-path src-tauri/Cargo.toml

  if [ "$(uname -s)" = Darwin ]; then
    step "Native notification action ordering and shutdown"
    python3 scripts/test-notification-actions.py
  fi
}

run_daemon() {
  step "Daemon (no Tauri, no frontend bundle)"
  cargo clippy --manifest-path src-tauri/Cargo.toml \
    --no-default-features --features server --all-targets -- -D warnings

  step "A workspace whose account service is not there"
  cargo test --manifest-path src-tauri/Cargo.toml \
    --no-default-features --features server --test unreachable

  cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features server
}

run_rust() {
  run_desktop
  run_daemon
}

case "${1:-all}" in
  grants) run_grants ;;
  assets) run_assets ;;
  scripts) run_scripts ;;
  web) run_web ;;
  rust) run_rust ;;
  desktop) run_desktop ;;
  daemon) run_daemon ;;
  connector) run_connector ;;
  formal) run_formal ;;
  e2e) run_e2e ;;
  all)
    run_grants
    run_assets
    run_scripts
    run_web
    run_rust
    run_e2e
    run_connector
    run_formal
    ;;
  *)
    echo "usage: $0 [all|grants|assets|scripts|web|rust|desktop|daemon|connector|formal|e2e]" >&2
    exit 2
    ;;
esac

step "All checks passed"
