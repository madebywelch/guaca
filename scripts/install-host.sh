#!/usr/bin/env bash

install_host() {
  local image="" memory status tag
  tag="guacad:$(git rev-parse --short=12 HEAD)" || return 1
  status="$(git status --porcelain)" || return 1
  if [ -z "$status" ]; then
    image="$(node scripts/install-image.mjs)" || return 1
  else
    note "working tree has uncommitted or untracked files; building the local host"
  fi
  if [ -n "$image" ]; then
    note "pulling the published host for this commit"
    docker pull "$image" || { echo "Could not pull the published host; check Docker and try again." >&2; return 1; }
    docker tag "$image" "$tag" || { echo "Could not tag the published host; check Docker and try again." >&2; return 1; }
    export GUACA_BACKEND_IMAGE="$tag"
    return
  fi
  memory="$(docker info --format '{{.MemTotal}}')" || return 1
  if ! [[ "$memory" =~ ^[0-9]+$ ]] || [ "$memory" -lt 3758096384 ]; then
    echo 'Docker needs at least 4 GiB of memory to build the local host; run `colima start --memory 4` or increase memory in Docker Desktop Resources, then try again.' >&2
    return 1
  fi
  export GUACA_BACKEND_IMAGE="$tag"
  docker build --build-arg "GUACA_VERSION=$(node -p 'require("./package.json").version')" --build-arg "GUACA_COMMIT=$COMMIT" -t "$GUACA_BACKEND_IMAGE" .
}
