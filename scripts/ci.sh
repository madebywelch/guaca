#!/usr/bin/env bash
#
# The full verification gate. Runs identically on a laptop and in a container,
# so "it passed locally" and "it passed in CI" mean the same thing.
#
#   ./scripts/ci.sh          everything
#   ./scripts/ci.sh assets   just the icons and tray glyphs against their scripts
#   ./scripts/ci.sh scripts  installer checks, probe unit tests, disk image layout
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

run_grants() {
  step "No comment, suppression or prose file without the owner's grant"
  node --test scripts/check-grants.test.mjs
  node scripts/check-grants.mjs
}

run_assets() {
  # The icons are compiled in, so a stale one is invisible until the app is
  # opened on a Mac. The check is a regeneration into a temp directory, which
  # is also the only way to catch a mark edited without running its script.
  step "Icons and tray glyphs"
  python3 scripts/check-icons.py
}

run_scripts() {
  # The version floors `install.sh` checks. That script builds a .app and stops
  # on anything that is not a Mac, so the part of it that decides whether
  # somebody is told now or by a compiler four hundred crates in is kept in
  # `scripts/versions.sh` and held here, on Linux, where this gate runs too.
  step "Shell checks"
  bash scripts/versions.test.sh
  node --test scripts/install-image.test.mjs
  bash scripts/install-host.test.sh
  bash deploy/box/install.test.sh
  python3 scripts/check-docker-deps.py

  step "Probe evidence (no app or screen)"
  python3 scripts/probe.test.py

  # Making the image needs hdiutil, so only its Finder layout is held here;
  # opening a packaged image is a release step in docs/RELEASING.md.
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

  # The probe's page bridge belongs only in a `vite build --mode probe`
  # bundle (src/lib/probe.ts). The bundler folds it out of this one; checked on
  # the output, because a guard rewritten into something the bundler cannot
  # fold would ship it without a word.
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
  # The daemon is a second host over the same library, and it is built without
  # Tauri on purpose: `app.rs` expands a macro that reads dist/, so a server
  # that needed the desktop feature could not be built in a container that has
  # no frontend. Nothing above catches a break here, because every target above
  # is compiled with the desktop feature on.
  step "Daemon (no Tauri, no frontend bundle)"
  cargo clippy --manifest-path src-tauri/Cargo.toml \
    --no-default-features --features server --all-targets -- -D warnings

  # A promise to an operator rather than a check of the code, so it gets a line
  # of its own: a self-hosted workspace runs with its account service absent.
  # The blanket run below covers it too, which is what keeps a new target from
  # being forgotten here; this line is what tells a reader which promise broke.
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
  all)
    run_grants
    run_assets
    run_scripts
    run_web
    run_rust
    run_connector
    run_formal
    ;;
  *)
    echo "usage: $0 [all|grants|assets|scripts|web|rust|desktop|daemon|connector|formal]" >&2
    exit 2
    ;;
esac

step "All checks passed"
