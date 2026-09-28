#!/usr/bin/env bash
#
# Installs a box on this machine's Docker the way deploy/box/install.sh does on
# a server, and proves the parts no fake Docker can: that the updater, inside
# its own container, can make the host, reach it, share a socket with it, and
# replace itself; and that the host relays it behind the workspace's token.
#
#   IMAGE=guacad:abc1234 ./scripts/box.sh     an image scripts/image.sh built
#
# It does not install an update. The updater installs only a release this
# build's key signed, from GitHub, so what is checked instead is that the
# release there now is refused before anything stops. The sequence itself is
# the desktop's, and `host::tests` and the ignored Docker test in host.rs run it.
#
# The names are a box's own, so it refuses to run beside a box that exists.

set -euo pipefail
cd "$(dirname "$0")/.."

step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }
fail() { printf '\033[31mFAIL:\033[0m %s\n' "$1" >&2; exit 1; }

: "${IMAGE:?Set IMAGE to a host image, e.g. the one ./scripts/image.sh built}"
for name in guacad guaca-updater guaca-updater-retiring; do
  if docker container inspect "$name" >/dev/null 2>&1; then
    fail "a container named $name exists; this check will not run beside a box"
  fi
done
for volume in guaca-updater guaca-updater-socket; do
  if docker volume inspect "$volume" >/dev/null 2>&1; then
    fail "a volume named $volume exists; this check will not run beside a box"
  fi
done

WORKSPACE="box-check-$$-data"
PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')"
TOKEN="box-check-token-$$"
SETTINGS="$(mktemp)"
printf 'GUACA_TOKEN=%s\nGUACA_UPDATE_CHECKS=off\n' "$TOKEN" > "$SETTINGS"
BASE="http://127.0.0.1:${PORT}"

cleanup() {
  docker rm -f guacad guaca-updater guaca-updater-retiring >/dev/null 2>&1 || true
  docker volume rm "$WORKSPACE" guaca-updater guaca-updater-socket >/dev/null 2>&1 || true
  docker volume ls --format '{{.Name}}' | grep -E '^guacad-(backup|failed)-' | xargs -r docker volume rm >/dev/null 2>&1 || true
  rm -f "$SETTINGS"
}
trap cleanup EXIT

step "Installing a box from ${IMAGE}"
said="$(GUACA_IMAGE="$IMAGE" GUACA_ENV="$SETTINGS" GUACA_PORT="$PORT" GUACA_VOLUME="$WORKSPACE" \
  sh deploy/box/install.sh)" || { docker logs guaca-updater 2>&1 | tail -20; fail "the installer failed"; }
echo "$said"
grep -q "Access key: the GUACA_TOKEN" <<<"$said" || fail "the installer did not report the key from the settings file"

step "Checking the host the updater made"
host="$(docker container inspect guacad)"
python3 - "$host" "$IMAGE" "$WORKSPACE" "$TOKEN" <<'PY' || fail "the host is not made the way a box's host must be"
import json, sys
host, image, workspace, token = json.loads(sys.argv[1])[0], *sys.argv[2:]
assert host["Config"]["Labels"]["bot.guaca.box"] == "guacad", "not labeled as the box's"
assert host["Config"]["Image"] == image, host["Config"]["Image"]
env = host["Config"]["Env"]
assert f"GUACA_TOKEN={token}" in env, "the settings file did not reach the host"
assert "GUACA_UPDATER_SOCKET=/run/guaca-updater/updater.sock" in env
mounts = {m["Destination"]: m for m in host["Mounts"]}
assert mounts["/var/lib/guaca"]["Name"] == workspace
assert mounts["/run/guaca-updater"]["RW"] is False, "the host can write the updater's socket volume"
assert "/var/run/docker.sock" not in mounts, "the host was given the Docker socket"
for binding in host["HostConfig"]["PortBindings"]["8787/tcp"]:
    assert binding["HostIp"] == "127.0.0.1", binding
PY

step "Asking the updater through the host"
code="$(curl -s -o /dev/null -w '%{http_code}' "${BASE}/v1/host")"
[ "$code" = "401" ] || fail "the updater answered without the token (${code})"
report="$(curl -fsS -H "authorization: Bearer ${TOKEN}" "${BASE}/v1/host")"
echo "report: ${report}"
python3 - "$report" "$IMAGE" <<'PY' || fail "the host did not relay the updater"
import json, sys
report, image = json.loads(sys.argv[1]), sys.argv[2]
assert report["managed"] is True and report["updating"] is False, report
assert report["running"]["image"] == image, report
assert report["error"] is None, report
PY

step "Asking for a release this build's key did not sign"
before="$(docker container inspect --format '{{.Id}}' guacad)"
answer="$(curl -s -w '\n%{http_code}' -X POST -H "authorization: Bearer ${TOKEN}" \
  -H 'content-type: application/json' -d '{"version":"0.0.0"}' "${BASE}/v1/host/update")"
echo "answer: ${answer}"
[ "$(tail -1 <<<"$answer")" = "409" ] || fail "an update nobody reviewed was accepted"
[ "$(docker container inspect --format '{{.Id}}' guacad)" = "$before" ] || fail "the host was touched"
[ "$(docker container inspect --format '{{.State.Running}}' guacad)" = "true" ] || fail "the host stopped"

step "Installing again, which is how an updater replaces itself"
updater="$(docker container inspect --format '{{.Id}}' guaca-updater)"
GUACA_IMAGE="$IMAGE" GUACA_ENV="$SETTINGS" GUACA_PORT="$PORT" GUACA_VOLUME="$WORKSPACE" \
  sh deploy/box/install.sh >/dev/null
for _ in $(seq 1 30); do
  if ! docker container inspect guaca-updater-retiring >/dev/null 2>&1; then break; fi
  sleep 1
done
docker container inspect guaca-updater-retiring >/dev/null 2>&1 && fail "the replaced updater was left running"
[ "$(docker container inspect --format '{{.Id}}' guaca-updater)" != "$updater" ] || fail "the updater was not replaced"
[ "$(docker container inspect --format '{{.Id}}' guacad)" = "$before" ] || fail "replacing the updater replaced the host"
for _ in $(seq 1 30); do
  if curl -fsS -H "authorization: Bearer ${TOKEN}" "${BASE}/v1/host" 2>/dev/null | grep -q '"managed":true'; then break; fi
  sleep 1
done
curl -fsS -H "authorization: Bearer ${TOKEN}" "${BASE}/v1/host" | grep -q '"managed":true' \
  || fail "the host lost its updater when the updater was replaced"

step "Box check passed"
