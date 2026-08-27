#!/usr/bin/env bash
#
# Build the latest Guaca and put it in /Applications, replacing whatever is
# already there.
#
#   ./scripts/install.sh              latest main, built here, installed
#   ./scripts/install.sh --no-pull    whatever is checked out right now
#   ./scripts/install.sh --launch     and open it once it lands
#
#   GUACA_DEST             where the app goes            (/Applications)
#   GUACA_SIGN_IDENTITY    what signs it                 (-, ad-hoc)
#
# There is nothing to download. Latest means the tip of `main`, compiled on
# this machine, so this script is a fetch, a build and a swap.
#
# The swap is the part worth reading. It is two renames on one volume rather
# than a copy over the top: `cp -R` onto an existing bundle merges into it, and
# what survives is every file the previous build shipped and this one does not,
# inside an app that looks new. Staging the incoming copy beside the
# destination first means the app on disk is either the old one or the new one
# and never a mixture of both, and anything that goes wrong before the new one
# has been checked puts the old one back rather than leaving nothing.
#
# It quits a running instance first, and that is not politeness. Closing the
# window hides it and the menu bar keeps it alive, so "I closed it" is not the
# same as "it is not running", and replacing a bundle out from under a live
# process is how you get an app that dies partway through somebody's turn.
#
# It re-signs. Tauri leaves the linker's ad-hoc signature, which covers the
# executable and seals nothing else, so `codesign --verify` on the installed
# bundle answers a narrower question than the one being asked. Signing it as a
# bundle is what makes the check at the end mean something. Name a certificate
# in GUACA_SIGN_IDENTITY to sign with one instead.
#
# Nothing under ~/Library/Application Support/com.madebywelch.guac is touched.
# The workspace, the database, the sign-ins and the settings are the operator's,
# and reinstalling is not resetting.
#
# This builds; it does not verify. ./scripts/ci.sh is the gate.

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
      sed -n '3,11p' "$0" | sed 's/^# \{0,1\}//'
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

# Everything that can be known before a five minute build is checked before it,
# including whether the answer can be written where it is going: a permission
# error after the compile is the same error, discovered at the worst moment.
step "Checking what this needs"

[ "$(uname -s)" = "Darwin" ] || fail "macOS only; this builds a .app bundle."

for tool in git pnpm cargo codesign ditto; do
  command -v "$tool" >/dev/null 2>&1 || fail "$tool is not on PATH; install it and run this again."
done

[ -d "$DEST" ] || fail "$DEST does not exist; set GUACA_DEST to where the app should go."
[ -w "$DEST" ] || fail "$DEST is not writable by $(id -un); set GUACA_DEST, or fix the permissions."

note "destination  $DEST"
note "signature    $([ "$IDENTITY" = "-" ] && echo "ad-hoc" || echo "$IDENTITY")"

if [ "$PULL" -eq 1 ]; then
  step "Getting the latest"

  # A dirty tree is somebody in the middle of something. Building it is the
  # useful answer and moving it is not, so the pull is skipped and said out
  # loud rather than the whole run refused.
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
      # Fast-forward only. An install script that merges or rebases is one that
      # can hand back a tree with conflicts in it, which is a worse problem
      # than the old app.
      git merge --ff-only "origin/$BRANCH" \
        || fail "$BRANCH has diverged from origin/$BRANCH; sort that out first, or use --no-pull."
    fi
  fi
fi

COMMIT="$(git rev-parse --short HEAD)"

step "Building"
note "from $COMMIT"

pnpm install --frozen-lockfile
pnpm tauri build --bundles app

[ -d "$BUILT" ] || fail "the build produced no bundle at $BUILT."

VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$BUILT/Contents/Info.plist")"
BUNDLE_ID="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$BUILT/Contents/Info.plist")"

step "Signing"

# --identifier is what keeps an ad-hoc signature from being named after the
# hash of the binary, which changes on every build.
codesign --force --sign "$IDENTITY" --identifier "$BUNDLE_ID" "$BUILT"
note "$(codesign -dv "$BUILT" 2>&1 | sed -n 's/^Signature=//p')"

INSTALLED="$DEST/Guaca.app"
EXEC="$INSTALLED/Contents/MacOS/guac"

if pgrep -f "^$EXEC" >/dev/null 2>&1; then
  step "Quitting the running Guaca"

  osascript -e "quit app id \"$BUNDLE_ID\"" >/dev/null 2>&1 || true

  waited=0
  while pgrep -f "^$EXEC" >/dev/null 2>&1 && [ "$waited" -lt 40 ]; do
    sleep 0.25
    waited=$((waited + 1))
  done

  # A window with an unsaved dialog, an agent mid-turn, or an app that never
  # got the event. The turn is lost either way; a half-copied bundle is worse.
  if pgrep -f "^$EXEC" >/dev/null 2>&1; then
    note "it did not quit on its own; ending it"
    pkill -f "^$EXEC" || true
    sleep 1
  fi
fi

step "Installing"

STAGE="$DEST/.Guaca.app.incoming.$$"
BACKUP="$DEST/.Guaca.app.previous.$$"
COMMITTED=0

# The old app is held one rename away until the new one is in place and has
# been checked, and whatever ends the script decides which of them stays: a
# failure, a Ctrl-C in the half second the destination is empty, or the end of
# a run that worked. One function, one flag, and no call site that has to
# remember to clean up after the one failure somebody thought of.
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

# Built here, so there is no quarantine flag to clear in the ordinary case.
# There is one if the bundle ever came off a disk image, and its symptom is a
# dialog the operator cannot argue with.
xattr -dr com.apple.quarantine "$INSTALLED" 2>/dev/null || true

# Otherwise Finder and Spotlight can spend a while believing in the app that
# was there a minute ago, icon included.
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister"
[ -x "$LSREGISTER" ] && "$LSREGISTER" -f "$INSTALLED" >/dev/null 2>&1 || true

step "Checking what landed"

# The copy that was verified is the one that runs. Checking the build and
# trusting the copy is checking the wrong bundle.
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
