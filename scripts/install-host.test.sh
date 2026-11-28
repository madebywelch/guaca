#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/install-host.sh
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
cat > "$scratch/docker" <<'EOF'
#!/usr/bin/env bash
printf '<%s>' "$@" >> "$CALLS"
printf '\n' >> "$CALLS"
case "$1" in
  info) printf '%s\n' "$MEMORY" ;;
  pull) exit "$PULL_RESULT" ;;
  build) exit "$BUILD_RESULT" ;;
esac
EOF
chmod +x "$scratch/docker"
export PATH="$scratch:$PATH" CALLS="$scratch/calls" MEMORY=4131463168 PULL_RESULT=0 BUILD_RESULT=0
note() { :; }
git() { if [ "$1" = status ]; then printf '%s' "$DIRTY"; else echo abc123def456; fi; }
node() { if [ "$1" = -p ]; then echo 0.2.0; else printf '%s' "$IMAGE"; return "$LOOKUP"; fi; }
COMMIT=abc123d DIRTY= IMAGE= LOOKUP=0
tag=guacad:abc123def456
image="ghcr.io/madebywelch/guaca/guacad@sha256:$(printf 'b%.0s' {1..64})"
assert_calls() { diff -u <(printf '%s\n' "$1") "$CALLS"; : > "$CALLS"; }
install_host
assert_calls "<info><--format><{{.MemTotal}}>
<build><--build-arg><GUACA_VERSION=0.2.0><--build-arg><GUACA_COMMIT=abc123d><-t><$tag><.>"
[ "$GUACA_BACKEND_IMAGE" = "$tag" ]
MEMORY=3758096383
if install_host 2>"$scratch/error"; then echo "low memory accepted" >&2; exit 1; fi
assert_calls '<info><--format><{{.MemTotal}}>'
printf '%s\n' 'Docker needs at least 4 GiB of memory to build the local host; run `colima start --memory 4` or increase memory in Docker Desktop Resources, then try again.' | diff -u - "$scratch/error"
MEMORY=3758096384
install_host
assert_calls "<info><--format><{{.MemTotal}}>
<build><--build-arg><GUACA_VERSION=0.2.0><--build-arg><GUACA_COMMIT=abc123d><-t><$tag><.>"
IMAGE="$image" MEMORY=0
install_host
assert_calls "<pull><$image>"
[ "$GUACA_BACKEND_IMAGE" = "$image" ]
PULL_RESULT=1
if install_host 2>/dev/null; then echo "pull failure accepted" >&2; exit 1; fi
assert_calls "<pull><$image>"
PULL_RESULT=0 LOOKUP=1
if install_host 2>/dev/null; then echo "lookup failure accepted" >&2; exit 1; fi
[ ! -s "$CALLS" ]
DIRTY='?? local' MEMORY=4131463168
install_host
assert_calls "<info><--format><{{.MemTotal}}>
<build><--build-arg><GUACA_VERSION=0.2.0><--build-arg><GUACA_COMMIT=abc123d><-t><$tag><.>"
browser="ghcr.io/madebywelch/guaca/browser@sha256:$(printf 'c%.0s' {1..64})"
browser_build="<build><--build-arg><GUACA_VERSION=0.2.0><--build-arg><GUACA_COMMIT=abc123d><-t><guaca-browser:abc123def456><deploy/browser>"
GUACA_BROWSER_IMAGE=guaca-browser:dev-1
if install_browser 2>"$scratch/error"; then echo "a browser not pinned by digest was accepted" >&2; exit 1; fi
[ ! -s "$CALLS" ]
printf '%s\n' 'GUACA_BROWSER_IMAGE must name the published browser by digest, as the release run does; unset it to build the browser here.' | diff -u - "$scratch/error"
GUACA_BROWSER_IMAGE="$browser" PULL_RESULT=1
if install_browser 2>/dev/null; then echo "browser pull failure accepted" >&2; exit 1; fi
assert_calls "<pull><$browser>"
PULL_RESULT=0
install_browser
assert_calls "<pull><$browser>"
[ "$GUACA_BROWSER_IMAGE" = "$browser" ]
GUACA_BROWSER_IMAGE= BUILD_RESULT=1
if install_browser 2>/dev/null; then echo "browser build failure accepted" >&2; exit 1; fi
assert_calls "$browser_build"
BUILD_RESULT=0 GUACA_BROWSER_IMAGE=
install_browser
assert_calls "$browser_build"
[ "$GUACA_BROWSER_IMAGE" = guaca-browser:abc123def456 ]
host_at="$(grep -n '^  install_host || exit 1$' scripts/install.sh | cut -d: -f1)"
browser_at="$(grep -n '^  install_browser || exit 1$' scripts/install.sh | cut -d: -f1)"
build_at="$(grep -n '^pnpm tauri build' scripts/install.sh | cut -d: -f1)"
{ [ -n "$host_at" ] && [ -n "$browser_at" ] && [ -n "$build_at" ] && [ "$host_at" -lt "$browser_at" ] && [ "$browser_at" -lt "$build_at" ]; } \
  || { echo "install.sh does not set up the browser after the host and before the build" >&2; exit 1; }
echo "Installer host checks passed"
