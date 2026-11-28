#!/usr/bin/env bash

set -euo pipefail

cd "$(dirname "$0")/.."

DEST="${GUACA_DEST:-/Applications}"
IDENTITY="${GUACA_SIGN_IDENTITY:--}"
BUILT="src-tauri/target/release/bundle/macos/Guaca.app"
PULL=1
LAUNCH=0

while [ $# -gt 0 ]; do
  case "$1" in
    --no-pull) PULL=0 ;;
    --launch)  LAUNCH=1 ;;
    -h|--help)
      cat <<'EOF'
Build the latest Guaca and put it in /Applications, replacing whatever is
already there.

  ./scripts/install.sh              latest main, built here, installed
  ./scripts/install.sh --no-pull    whatever is checked out right now
  ./scripts/install.sh --launch     and open it once it lands

  GUACA_DEST             where the app goes            (/Applications)
  GUACA_SIGN_IDENTITY    what signs it                 (-, ad-hoc)
EOF
      exit 0
      ;;
    *)
      echo "usage: $0 [--no-pull] [--launch]" >&2
      exit 2
      ;;
  esac
  shift
done

step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }
note() { printf '    %s\n' "$1"; }
fail() { printf '\033[1m%s\033[0m\n' "$1" >&2; exit 1; }

# shellcheck source=scripts/versions.sh
. scripts/versions.sh
. scripts/install-host.sh

step "Checking what this needs"

[ "$(uname -s)" = "Darwin" ] || fail "macOS only; this builds a .app bundle."

xcrun --find clang >/dev/null 2>&1 \
  || fail "the Xcode Command Line Tools are missing; run \`xcode-select --install\`, finish that installer, and run this again."

command -v node >/dev/null 2>&1 \
  || fail "node is not on PATH; install Node.js from nodejs.org and run this again."

for tool in git cargo codesign ditto; do
  command -v "$tool" >/dev/null 2>&1 || fail "$tool is not on PATH; install it and run this again."
done

command -v pnpm >/dev/null 2>&1 \
  || fail "pnpm is not on PATH; run \`corepack enable pnpm\` (corepack ships with Node), and run this again."
PINNED="$(pnpm_pin)"
VERDICT="$(pnpm_verdict "$(pnpm --version 2>/dev/null || true)" "$PINNED")"
[ -z "$VERDICT" ] || fail "$VERDICT"

FLOOR="$(sed -n 's/^rust-version *= *"\([0-9.]*\)".*/\1/p' src-tauri/Cargo.toml)"
GOT="$(rustc --version | sed -n 's/^rustc \([0-9][0-9.]*\).*/\1/p')"
if [ -n "$FLOOR" ] && [ -n "$GOT" ] && ! at_least "$GOT" "$FLOOR"; then
  fail "rustc $GOT is older than the $FLOOR this app builds with; run \`rustup update stable\` and run this again."
fi

[ -d "$DEST" ] || fail "$DEST does not exist; set GUACA_DEST to where the app should go."
[ -w "$DEST" ] || fail "$DEST is not writable by $(id -un); set GUACA_DEST, or fix the permissions."

note "destination  $DEST"
note "signature    $([ "$IDENTITY" = "-" ] && echo "ad-hoc" || echo "$IDENTITY")"

if [ "$PULL" -eq 1 ]; then
  step "Getting the latest"

  if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    note "working tree has uncommitted changes; building those, not origin's"
  else
    BRANCH="$(git rev-parse --abbrev-ref HEAD)"
    git fetch --quiet origin

    if ! git rev-parse --verify --quiet "origin/$BRANCH" >/dev/null; then
      note "no origin/$BRANCH; building what is checked out"
    elif [ "$(git rev-parse HEAD)" = "$(git rev-parse "origin/$BRANCH")" ]; then
      note "already at origin/$BRANCH"
    else
      git merge --ff-only "origin/$BRANCH" \
        || fail "$BRANCH has diverged from origin/$BRANCH; sort that out first, or use --no-pull."
    fi
  fi
fi

COMMIT="$(git rev-parse --short HEAD)"

step "Building"
note "from $COMMIT"

pnpm install --frozen-lockfile
if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  step "Building the local host"
  install_host || exit 1
  install_browser || exit 1
else
  note "Docker is not ready; installing the desktop client for a remote host."
  note "For a local source build, start Docker and run this installer again."
fi
pnpm tauri build --bundles app

[ -d "$BUILT" ] || fail "the build produced no bundle at $BUILT."

VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$BUILT/Contents/Info.plist")"
BUNDLE_ID="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$BUILT/Contents/Info.plist")"

step "Signing"

codesign --force --sign "$IDENTITY" --identifier "$BUNDLE_ID" "$BUILT"
note "$(codesign -dv "$BUILT" 2>&1 | sed -n 's/^Signature=//p')"

INSTALLED="$DEST/Guaca.app"
EXEC="$INSTALLED/Contents/MacOS/guac"

if pgrep -a -f "^$EXEC" >/dev/null 2>&1; then
  step "Quitting the running Guaca"

  osascript -e "quit app id \"$BUNDLE_ID\"" >/dev/null 2>&1 || true

  waited=0
  while pgrep -a -f "^$EXEC" >/dev/null 2>&1 && [ "$waited" -lt 40 ]; do
    sleep 0.25
    waited=$((waited + 1))
  done

  if pgrep -a -f "^$EXEC" >/dev/null 2>&1; then
    note "it did not quit on its own; ending it"
    pkill -a -f "^$EXEC" || true
    sleep 1
  fi
fi

step "Installing"

STAGE="$DEST/.Guaca.app.incoming.$$"
BACKUP="$DEST/.Guaca.app.previous.$$"
COMMITTED=0

restore() {
  rm -rf "$STAGE"

  [ -e "$BACKUP" ] || return 0

  if [ "$COMMITTED" -eq 1 ]; then
    rm -rf "$BACKUP"
  else
    rm -rf "$INSTALLED"
    mv "$BACKUP" "$INSTALLED"
    printf '    %s\n' "put the previous app back at $INSTALLED" >&2
  fi
}

trap restore EXIT
trap 'exit 130' INT TERM

ditto "$BUILT" "$STAGE"

if [ -e "$INSTALLED" ]; then
  mv "$INSTALLED" "$BACKUP"
fi

mv "$STAGE" "$INSTALLED" \
  || fail "could not put the new app at $INSTALLED."

xattr -dr com.apple.quarantine "$INSTALLED" 2>/dev/null || true

LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister"
[ -x "$LSREGISTER" ] && "$LSREGISTER" -f "$INSTALLED" >/dev/null 2>&1 || true

step "Checking what landed"

codesign --verify --strict "$INSTALLED" \
  || fail "the installed bundle does not verify."

[ -x "$EXEC" ] || fail "$EXEC is missing or not executable."

COMMITTED=1

note "Guaca $VERSION ($COMMIT)"
note "$INSTALLED"
note "settings and workspace left where they were"

if [ "$LAUNCH" -eq 1 ]; then
  step "Opening it"
  open "$INSTALLED"
fi
