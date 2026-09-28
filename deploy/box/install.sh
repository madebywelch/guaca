#!/bin/sh
# Installs Guaca on a Linux box with Docker, as a host its operator can update
# from the app.
#
#   curl -fsSL https://raw.githubusercontent.com/madebywelch/guaca/main/deploy/box/install.sh | sudo sh
#
# It makes two containers. `guaca-updater` holds the Docker socket and runs
# nothing else; `guacad`, the host, is made and replaced by it and never sees
# the socket. The host is published to 127.0.0.1:$GUACA_PORT only: put a tunnel
# in front of it (Tailscale Serve, or a TLS reverse proxy) and give people that
# address. Running this again reconfigures the box and keeps its workspace.
#
#   GUACA_ENV     settings handed to the host, one NAME=value per line
#                 (GUACA_TOKEN, GUACA_ORIGIN, ANTHROPIC_API_KEY, GH_TOKEN; the
#                 full list is HOST_ENV in src-tauri/src/updater.rs).
#                 Default /etc/guaca/guaca.env, if it exists. Keep it 0600.
#   GUACA_IMAGE   the image to install. Default: the latest release.
#   GUACA_PORT    the loopback port. Default 8787.
#   GUACA_VOLUME  the workspace volume. Default guacad-data. Name an existing
#                 one to adopt its workspace, e.g. a Compose host's.
set -eu

ENV_FILE="${GUACA_ENV:-/etc/guaca/guaca.env}"
IMAGE="${GUACA_IMAGE:-}"
PORT="${GUACA_PORT:-8787}"

fail() {
  echo "$1" >&2
  exit 1
}

command -v docker >/dev/null 2>&1 || fail "Docker is not installed. Install Docker Engine, then run this again."
docker info >/dev/null 2>&1 || fail "Docker is not running, or this user cannot use it. Start Docker, or run this with sudo."

if [ -z "$IMAGE" ]; then
  manifest=$(curl -fsSL https://github.com/madebywelch/guaca/releases/latest/download/guaca-release.json) ||
    fail "Could not read the latest Guaca release. Check this box's connection, then run this again."
  IMAGE=$(printf '%s' "$manifest" | sed -n 's/.*"image": *"\(ghcr\.io\/madebywelch\/guaca\/guacad@sha256:[0-9a-f]\{64\}\)".*/\1/p')
  [ -n "$IMAGE" ] || fail "The latest Guaca release names no host image. Try again later."
fi

# A container by either name that guaca-updater did not make is somebody
# else's. Said here rather than discovered as a name conflict halfway through.
for pair in guacad:bot.guaca.box guaca-updater:bot.guaca.updater; do
  name=${pair%%:*}
  label=${pair#*:}
  if docker container inspect "$name" >/dev/null 2>&1; then
    owner=$(docker container inspect --format "{{index .Config.Labels \"$label\"}}" "$name")
    [ "$owner" = "$name" ] || fail "A container named $name already exists and was not made by this installer. Remove it (docker rm -f $name; its volumes are kept), then run this again."
  fi
done

docker image inspect "$IMAGE" >/dev/null 2>&1 || docker pull "$IMAGE"

set -- --rm --user 0 \
  --mount type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock \
  --entrypoint /usr/local/bin/guaca-updater
if [ -f "$ENV_FILE" ]; then
  set -- "$@" --env-file "$ENV_FILE"
fi
if [ -n "${GUACA_PORT:-}" ]; then
  set -- "$@" --env GUACA_PORT="$GUACA_PORT"
fi
if [ -n "${GUACA_VOLUME:-}" ]; then
  set -- "$@" --env GUACA_VOLUME="$GUACA_VOLUME"
fi
docker run "$@" "$IMAGE" install "$IMAGE"

# The updater makes the host once it is serving. Wait for the host to answer.
tries=0
until curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; do
  tries=$((tries + 1))
  [ "$tries" -lt 180 ] || fail "The host did not start. 'docker logs guaca-updater' says why."
  sleep 1
done

echo "Guaca is running on 127.0.0.1:$PORT."
if token=$(docker exec guacad cat /var/lib/guaca/config/token 2>/dev/null) && [ -n "$token" ]; then
  echo "Access key: $token"
else
  echo "Access key: the GUACA_TOKEN in $ENV_FILE."
fi
echo "Put a tunnel in front of that port, then connect from the Guaca app with the tunnel's address and this key."
