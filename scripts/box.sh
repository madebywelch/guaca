#!/usr/bin/env bash

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

reinstall() {
  env -u GUACA_CHANNEL "$@" GUACA_IMAGE="$IMAGE" GUACA_ENV="$SETTINGS" GUACA_PORT="$PORT" GUACA_VOLUME="$WORKSPACE" \
    sh deploy/box/install.sh >/dev/null
  for _ in $(seq 1 30); do
    if ! docker container inspect guaca-updater-retiring >/dev/null 2>&1 &&
      curl -fsS -H "authorization: Bearer ${TOKEN}" "${BASE}/v1/host" 2>/dev/null | grep -q '"managed":true'; then
      return 0
    fi
    sleep 1
  done
  fail "the updater did not come back after reinstalling with $*"
}
followed() {
  curl -fsS -H "authorization: Bearer ${TOKEN}" "${BASE}/$1" |
    python3 -c 'import json, sys; print(json.load(sys.stdin).get("channel", "absent"))'
}

step "Refusing a channel nobody can read, before anything changes"
updater="$(docker container inspect --format '{{.Id}}' guaca-updater)"
if GUACA_CHANNEL=nightly GUACA_IMAGE="$IMAGE" GUACA_ENV="$SETTINGS" GUACA_PORT="$PORT" GUACA_VOLUME="$WORKSPACE" \
  sh deploy/box/install.sh >/dev/null 2>&1; then
  fail "the installer accepted a channel nobody can read"
fi
if said="$(docker run --rm --env GUACA_CHANNEL=nightly --entrypoint /usr/local/bin/guaca-updater "$IMAGE" install "$IMAGE" 2>&1)"; then
  fail "the updater would replace itself on a channel nobody can read"
fi
grep -q "GUACA_CHANNEL is" <<<"$said" || fail "the updater refused a channel nobody can read without saying why: $said"
[ "$(docker container inspect --format '{{.Id}}' guaca-updater)" = "$updater" ] || fail "a refused channel replaced the updater"

step "Moving the box onto main, which the host learns from its updater"
reinstall GUACA_CHANNEL=main
[ "$(followed v1/host)" = "main" ] || fail "the updater does not say it follows main"
[ "$(followed v1/updates)" = "main" ] || fail "the host still reports releases after its updater moved to main"
[ "$(docker container inspect --format '{{.Id}}' guacad)" = "$before" ] || fail "moving channels replaced the host"
answer="$(curl -s -w '\n%{http_code}' -X POST -H "authorization: Bearer ${TOKEN}" \
  -H 'content-type: application/json' -d '{"version":"0.0.0"}' "${BASE}/v1/host/update")"
echo "answer: ${answer}"
[ "$(tail -1 <<<"$answer")" = "409" ] || fail "a box on main accepted an update that named no build"
grep -q "names the build" <<<"$answer" || fail "a box on main refused an unnamed build without saying why"

step "Moving it back"
reinstall
[ "$(followed v1/host)" = "release" ] || fail "reinstalling without a channel left the box on main"
[ "$(followed v1/updates)" = "release" ] || fail "the host still reports main after its updater moved back"

step "Box check passed"
