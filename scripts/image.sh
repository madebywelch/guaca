#!/usr/bin/env bash
#
# Builds the daemon's image and proves it answers.
#
#   ./scripts/image.sh          build, run, check, remove
#   KEEP=1 ./scripts/image.sh   leave the container running afterward
#
# The checks are the ones a box fails silently: the process is up, the token
# gate holds, the page is served, and the build the container reports is the
# build that was asked for. Nothing here needs a key or spends anything.

set -euo pipefail
cd "$(dirname "$0")/.."

step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }
fail() { printf '\033[31mFAIL:\033[0m %s\n' "$1" >&2; exit 1; }

COMMIT="$(git rev-parse --short=7 HEAD 2>/dev/null || echo unknown)"
if [ -n "$(git status --porcelain 2>/dev/null)" ]; then COMMIT="${COMMIT}-dirty"; fi
IMAGE="${IMAGE:-guacad:${COMMIT}}"
NAME="guacad-check-$$"

step "Building ${IMAGE}"
docker build --build-arg "GUACA_COMMIT=${COMMIT}" -t "${IMAGE}" .

step "Starting"
docker run -d --rm --name "${NAME}" -p 127.0.0.1::8787 "${IMAGE}" >/dev/null
cleanup() {
  if [ -z "${KEEP:-}" ]; then docker rm -f "${NAME}" >/dev/null 2>&1 || true; fi
}
trap cleanup EXIT

PORT="$(docker port "${NAME}" 8787/tcp | head -1 | sed -E 's/.*:([0-9]+)$/\1/')"
BASE="http://127.0.0.1:${PORT}"
for _ in $(seq 1 60); do
  if curl -fsS "${BASE}/health" >/dev/null 2>&1; then break; fi
  sleep 1
done
curl -fsS "${BASE}/health" >/dev/null || { docker logs "${NAME}"; fail "never became healthy"; }

step "Checking"
health="$(curl -fsS "${BASE}/health")"
echo "health: ${health}"
case "${health}" in
  *"\"build\":\"${COMMIT}\""*) ;;
  *) fail "health reports a build other than ${COMMIT}" ;;
esac

code="$(curl -s -o /dev/null -w '%{http_code}' -X POST "${BASE}/v1/call" \
  -H 'content-type: application/json' -d '{"name":"capabilities"}')"
[ "${code}" = "401" ] || fail "a call without the token answered ${code}, not 401"

token="$(docker exec "${NAME}" cat /var/lib/guaca/config/token)"
[ -n "${token}" ] || fail "no token was generated"
docker logs "${NAME}" 2>&1 | grep -q 'open this in a browser' || fail "no invitation was printed"

caps="$(curl -fsS -X POST "${BASE}/v1/call" \
  -H "authorization: Bearer ${token}" -H 'content-type: application/json' \
  -d '{"name":"capabilities"}')"
echo "capabilities: ${caps}"
case "${caps}" in
  *'"localDirectories":false'*) ;;
  *) fail "a container reported a desktop's capabilities" ;;
esac

curl -fsS "${BASE}/" | grep -q '<div id="root">' || fail "the page is not served at /"

# What a remote-linked repository runs on. Asked inside the container, because
# an image that lost one of these fails only when somebody links a repository.
docker exec "${NAME}" git --version >/dev/null || fail "git is not in the image"
docker exec "${NAME}" gh --version >/dev/null || fail "gh is not in the image"
docker exec "${NAME}" claude --version >/dev/null || fail "claude is not in the image"

step "Stopping"
docker stop "${NAME}" >/dev/null
# `--rm` took the container; the trap's remove is then a no-op.
docker logs "${NAME}" >/dev/null 2>&1 && fail "the container did not stop"

step "Image ${IMAGE} answers"
if [ -n "${KEEP:-}" ]; then
  docker run -d --name "${NAME}" -p 127.0.0.1:8787:8787 "${IMAGE}" >/dev/null
  echo "left running as ${NAME} on http://127.0.0.1:8787"
fi
