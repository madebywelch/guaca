#!/usr/bin/env bash
#
# The full verification gate. Runs identically on a laptop and in a container,
# so "it passed locally" and "it passed in CI" mean the same thing.
#
#   ./scripts/ci.sh          everything
#   ./scripts/ci.sh rust     just the Rust suite
#   ./scripts/ci.sh web      just the frontend

set -euo pipefail

cd "$(dirname "$0")/.."

step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

# The Tauri build macro reads dist/ at compile time, so the frontend has to
# exist before the Rust crate will even typecheck.
ensure_dist() {
  if [ ! -f dist/index.html ]; then
    step "Building frontend (required before the Rust crate compiles)"
    pnpm build
  fi
}

run_web() {
  step "Lint and format"
  pnpm check

  step "Typecheck and build"
  pnpm build

  step "Frontend tests"
  pnpm test
}

run_rust() {
  ensure_dist

  step "Rust format"
  cargo fmt --manifest-path src-tauri/Cargo.toml -- --check

  step "Clippy"
  cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings

  step "Rust tests"
  cargo test --manifest-path src-tauri/Cargo.toml

  # The daemon is a second host over the same library, and it is built without
  # Tauri on purpose: `app.rs` expands a macro that reads dist/, so a server
  # that needed the desktop feature could not be built in a container that has
  # no frontend. Nothing above catches a break here, because every target above
  # is compiled with the desktop feature on.
  step "Daemon (no Tauri, no frontend bundle)"
  cargo clippy --manifest-path src-tauri/Cargo.toml \
    --no-default-features --features server --all-targets -- -D warnings
  cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features server
}

case "${1:-all}" in
  web) run_web ;;
  rust) run_rust ;;
  all)
    run_web
    run_rust
    ;;
  *)
    echo "usage: $0 [all|web|rust]" >&2
    exit 2
    ;;
esac

step "All checks passed"
