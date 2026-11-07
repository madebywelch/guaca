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
  tag) exit "$TAG_RESULT" ;;
esac
EOF
chmod +x "$scratch/docker"
export PATH="$scratch:$PATH" CALLS="$scratch/calls" MEMORY=4131463168 PULL_RESULT=0 TAG_RESULT=0
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
assert_calls "<pull><$image>
<tag><$image><$tag>"
[ "$GUACA_BACKEND_IMAGE" = "$tag" ]
PULL_RESULT=1
if install_host 2>/dev/null; then echo "pull failure accepted" >&2; exit 1; fi
assert_calls "<pull><$image>"
PULL_RESULT=0 TAG_RESULT=1
if install_host 2>/dev/null; then echo "tag failure accepted" >&2; exit 1; fi
assert_calls "<pull><$image>
<tag><$image><$tag>"
TAG_RESULT=0 LOOKUP=1
if install_host 2>/dev/null; then echo "lookup failure accepted" >&2; exit 1; fi
[ ! -s "$CALLS" ]
DIRTY='?? local' MEMORY=4131463168
install_host
assert_calls "<info><--format><{{.MemTotal}}>
<build><--build-arg><GUACA_VERSION=0.2.0><--build-arg><GUACA_COMMIT=abc123d><-t><$tag><.>"
echo "Installer host checks passed"
