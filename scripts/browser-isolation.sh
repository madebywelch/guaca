#!/usr/bin/env bash

set -euo pipefail
cd "$(dirname "$0")/.."

BROWSER_IMAGE="${BROWSER_IMAGE:?name the browser image to check}"
STANDIN_IMAGE="${STANDIN_IMAGE:?name an image with node, curl and python3 to stand in for guacad}"
RUN="guaca-isolation-$$"
NET="${RUN}-net"
SIDECAR="${RUN}-browser"
GUACAD="${RUN}-guacad"
LAN="${RUN}-lan"

SIDECAR_ARGS=(
  --init
  --read-only
  --tmpfs /tmp
  --tmpfs /run
  --cap-drop ALL
  --cap-add NET_ADMIN
  --cap-add SETUID
  --cap-add SETGID
  --cap-add SETPCAP
  --cap-add CHOWN
  --security-opt no-new-privileges
  --security-opt "seccomp=deploy/browser/seccomp.json"
  --memory 2g
  --cpus 2
  --pids-limit 512
)
DOCKER_DESKTOP_NAMES=(--add-host host.docker.internal:host-gateway)

passed=0
failed=0
pass() { printf 'PASS  %s%s\n' "$1" "${2:+  ($2)}"; passed=$((passed + 1)); }
miss() { printf 'FAIL  %s%s\n' "$1" "${2:+  ($2)}"; failed=$((failed + 1)); }
note() { printf 'NOTE  %s\n' "$1"; }
field() { node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{const v=JSON.parse(s)[process.argv[1]];console.log(typeof v==="string"?v:JSON.stringify(v??""))})' "$1"; }

cleanup() {
  docker rm -f -v "${SIDECAR}" "${GUACAD}" "${LAN}" >/dev/null 2>&1 || true
  docker network rm "${NET}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker network create "${NET}" >/dev/null
docker run -d --name "${GUACAD}" --network "${NET}" "${STANDIN_IMAGE}" >/dev/null
docker run -d --name "${SIDECAR}" --network "${NET}" "${DOCKER_DESKTOP_NAMES[@]}" "${SIDECAR_ARGS[@]}" "${BROWSER_IMAGE}" >/dev/null
docker cp scripts/browser-isolation.mjs "${GUACAD}:/tmp/browser-isolation.mjs" >/dev/null

SIDECAR_IP="$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "${SIDECAR}")"
GUACAD_IP="$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "${GUACAD}")"
GATEWAY="$(docker network inspect -f '{{range .IPAM.Config}}{{.Gateway}}{{end}}' "${NET}")"
CDP="http://${SIDECAR_IP}:9223"
drive() { docker exec "${GUACAD}" node /tmp/browser-isolation.mjs "$1" "${CDP}" "${@:2}" || true; }

for _ in $(seq 1 60); do
  if docker exec "${GUACAD}" curl -fsS -m 2 "${CDP}/json/version" >/dev/null 2>&1; then break; fi
  sleep 1
done
docker exec "${GUACAD}" curl -fsS -m 2 "${CDP}/json/version" >/dev/null || { docker logs "${SIDECAR}" 2>&1 | tail -40; echo "the browser never answered" >&2; exit 1; }
for _ in $(seq 1 60); do
  if docker exec "${GUACAD}" curl -fsS -m 2 http://127.0.0.1:8787/health >/dev/null 2>&1; then break; fi
  sleep 1
done

echo "$(docker exec "${SIDECAR}" cat /etc/chromium-version) on $(docker image inspect -f '{{.Os}}/{{.Architecture}}' "${BROWSER_IMAGE}"), image $(( $(docker image inspect -f '{{.Size}}' "${BROWSER_IMAGE}") / 1000000 )) MB as Docker reports it"

inspect() { docker inspect -f "$1" "${SIDECAR}"; }

ports="$(inspect '{{json .NetworkSettings.Ports}}')"
bindings="$(inspect '{{json .HostConfig.PortBindings}}')"
if [ "${bindings}" = "{}" ] && ! grep -q HostPort <<<"${ports}"; then pass "CDP is published to no host port"; else miss "CDP is published to no host port" "${ports}"; fi

mounts="$(inspect '{{len .Mounts}}')"
if [ "${mounts}" = "0" ]; then pass "the sidecar mounts no volume"; else miss "the sidecar mounts no volume" "${mounts} mounts"; fi

if ! inspect '{{json .Config.Env}}' | grep -qi 'GUACA\|TOKEN' && ! docker exec "${SIDECAR}" test -e /var/lib/guaca; then
  pass "the sidecar holds no Guaca token"
else
  miss "the sidecar holds no Guaca token"
fi

caps="memory=$(inspect '{{.HostConfig.Memory}}') nanoCpus=$(inspect '{{.HostConfig.NanoCpus}}') pids=$(inspect '{{.HostConfig.PidsLimit}}')"
if [ "${caps}" = "memory=2147483648 nanoCpus=2000000000 pids=512" ]; then pass "CPU, memory and PIDs are capped" "${caps}"; else miss "CPU, memory and PIDs are capped" "${caps}"; fi

security="$(inspect '{{json .HostConfig.SecurityOpt}}')"
pid1_seccomp="$(docker exec "${SIDECAR}" grep Seccomp: /proc/1/status | tr -s '\t ' ' ')"
if grep -q '"seccomp=' <<<"${security}" && ! grep -q 'unconfined' <<<"${security}" && [ "${pid1_seccomp}" = "Seccomp: 2" ]; then
  pass "a seccomp profile is applied rather than unconfined"
else
  miss "a seccomp profile is applied rather than unconfined" "${pid1_seccomp}"
fi

if docker exec "${SIDECAR}" sh -c 'cat /proc/[0-9]*/cmdline 2>/dev/null | tr "\0" " "' | grep -q -- '--no-sandbox'; then
  miss "Chromium runs without --no-sandbox"
else
  pass "Chromium runs without --no-sandbox"
fi

renderers="$(docker exec "${SIDECAR}" sh -c 'for p in $(pgrep -f -- --type=renderer); do grep Seccomp: /proc/$p/status; done' | tr -s '\t ' ' ' | tr '\n' ' ')"
if [ -n "${renderers}" ] && ! grep -qv '^Seccomp: 2$' < <(tr -s ' ' '\n' <<<"${renderers}" | paste -d' ' - -); then
  pass "every renderer runs under Chromium's seccomp filter" "${renderers% }"
else
  miss "every renderer runs under Chromium's seccomp filter" "${renderers:-no renderer}"
fi

named="$(docker exec "${GUACAD}" curl -s -o /dev/null -w '%{http_code}' -m 5 "http://${SIDECAR}:9223/json/version" || true)"
if [ "${named}" = "500" ]; then pass "Chrome refuses a CDP request addressed by name" "status ${named}"; else miss "Chrome refuses a CDP request addressed by name" "status ${named}"; fi

socket="$(drive version | field url)"
case "${socket}" in
  "ws://${SIDECAR_IP}:9223/devtools/browser/"*) pass "the relay hands back a socket at the address it was asked on" "${socket}" ;;
  *) miss "the relay hands back a socket at the address it was asked on" "${socket}" ;;
esac

if docker run --rm --entrypoint curl "${STANDIN_IMAGE}" -fsS -m 5 "${CDP}/json/version" >/dev/null 2>&1; then
  miss "a container off the browser network cannot reach CDP"
else
  pass "a container off the browser network cannot reach CDP"
fi

if docker run --rm --network "${NET}" --entrypoint curl "${STANDIN_IMAGE}" -fsS -m 5 "http://${GUACAD}:8787/health" >/dev/null 2>&1; then
  pass "control: a plain container on the network reaches the stand-in guacad"
else
  miss "control: a plain container on the network reaches the stand-in guacad"
fi

docker run -d --name "${LAN}" -p "${GATEWAY}::8000" --entrypoint python3 "${STANDIN_IMAGE}" -m http.server 8000 >/dev/null
LAN_PORT="$(docker port "${LAN}" 8000/tcp | head -1 | sed -E 's/.*:([0-9]+)$/\1/')"
for _ in $(seq 1 20); do
  if curl -fsS -m 2 "http://${GATEWAY}:${LAN_PORT}/" >/dev/null 2>&1; then break; fi
  sleep 1
done
if docker run --rm --network "${NET}" --entrypoint curl "${STANDIN_IMAGE}" -fsS -m 5 "http://${GATEWAY}:${LAN_PORT}/" >/dev/null 2>&1; then
  pass "control: a plain container on the network reaches the LAN stand-in at ${GATEWAY}:${LAN_PORT}"
  LAN_PROVEN=1
else
  note "control: a plain container on the network cannot reach the LAN stand-in at ${GATEWAY}:${LAN_PORT} (this machine's firewall stops it, so the LAN check below proves less here)"
  LAN_PROVEN=0
fi

refused() {
  local what="$1" url="$2" said error
  said="$(drive visit "${url}")"
  error="$(field error <<<"${said}")"
  case "${error}" in
    net::ERR_CONNECTION_REFUSED | net::ERR_ADDRESS_UNREACHABLE | net::ERR_NETWORK_ACCESS_DENIED | net::ERR_BLOCKED_BY_ADMINISTRATOR)
      pass "the browser cannot reach ${what}" "${error}" ;;
    *) miss "the browser cannot reach ${what}" "${said}" ;;
  esac
}

refused "guacad by name (DNS to a private address)" "http://${GUACAD}:8787/health"
refused "guacad by address" "http://${GUACAD_IP}:8787/health"
if [ "${LAN_PROVEN}" = 1 ]; then
  refused "the LAN (a service on the network gateway)" "http://${GATEWAY}:${LAN_PORT}/"
else
  refused "the LAN (the network gateway)" "http://${GATEWAY}:${LAN_PORT}/"
fi
refused "host.docker.internal" "http://host.docker.internal:${LAN_PORT}/"
refused "cloud metadata, 169.254.169.254" "http://169.254.169.254/latest/meta-data/"
refused "its own CDP port on loopback" "http://127.0.0.1:9222/json/version"
refused "localhost" "http://localhost:9223/json/version"
refused "IPv6 loopback" "http://[::1]:9222/json/version"
refused "a public name that resolves to loopback" "http://127.0.0.1.nip.io:9222/json/version"
refused "file://" "file:///etc/passwd"

public="$(drive visit https://example.com/)"
if [ "$(field title <<<"${public}")" = "Example Domain" ]; then pass "the browser loads a public page" "Example Domain"; else miss "the browser loads a public page" "${public}"; fi

webrtc="$(drive webrtc https://example.com/)"
udp="$(field udp <<<"${webrtc}")"
if [ "${udp}" = "[]" ]; then pass "WebRTC gathers no UDP candidate" "none"; else miss "WebRTC gathers no UDP candidate" "${webrtc}"; fi

download="$(drive download /tmp/chromium/downloads)"
landed=""
for _ in $(seq 1 5); do
  landed="$(docker exec "${SIDECAR}" sh -c 'ls /tmp/chromium/downloads 2>/dev/null' || true)"
  if [ -n "${landed}" ]; then break; fi
  sleep 1
done
if [ -z "${landed}" ] && [ "$(field began <<<"${download}")" = "[]" ]; then pass "a download is refused"; else miss "a download is refused" "${download} ${landed}"; fi

sandbox="$(drive visit chrome://sandbox)"
if [ "$(field title <<<"${sandbox}")" = "Sandbox Status" ] && grep -q "adequately sandboxed" <<<"$(field text <<<"${sandbox}")"; then
  pass "Chromium reports itself sandboxed" "Sandbox Status"
else
  miss "Chromium reports itself sandboxed" "${sandbox:0:300}"
fi

printf '\n%d passed, %d failed\n' "${passed}" "${failed}"
[ "${failed}" = 0 ]
