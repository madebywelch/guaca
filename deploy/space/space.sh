#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Puts a private Guaca space on Fly.io, and keeps it.

A space is one Fly Machine running the published guacad image, pinned by
digest, with one 10 GB volume at /var/lib/guaca. It is a Fly app of its own
on a private network of its own, so it can reach no other app in the
organization and none can reach it. It stays on, and answers at
https://<name>.fly.dev.

  deploy/space/space.sh create  <name> [image@sha256:...]
                                         make the space, or finish making it
  deploy/space/space.sh invite  <name>   show its invitation again
  deploy/space/space.sh status  <name>   machine, image, health, volume
  deploy/space/space.sh update  <name> [image@sha256:...]
                                         snapshot, replace the image, check,
                                         and put the old one back on failure
  deploy/space/space.sh destroy <name> --confirm <name>
                                         delete the space and everything in it

Options:

  --dry-run          print every Fly call it would make, and make none. Needs
                     no credential; the token is shown as ****.
  --release VERSION  create or update with a signed release (0.5.1, or
                     latest) instead of the newest build of main.
  --region CODE      create in this Fly region. Default iad.
  --tz ZONE          the zone routines and reminders keep, by its IANA name.
                     Default America/New_York.
  --confirm NAME     destroy only: the space's name again.
  --force            update only: snapshot, swap and check even when the space
                     already runs that image.

The invitation, https://<name>.fly.dev/#token=..., is the space's password.
It is shown on this terminal and nowhere else: not in a file, not in Fly's
logs. With no terminal, create says so and invite shows it later.

Settings, read from the environment:

  FLY_API_TOKEN      a Fly token for the organization, from
                     `fly tokens create org`. Left out, the token of the
                     flyctl login on this machine. Not needed with --dry-run.
  FLY_ORG            the organization's slug. Left out, the one organization
                     the token reaches; a token that reaches several stops
                     here and asks for this.
  FLY_API_HOSTNAME   the Machines API. Default https://api.machines.dev.
  FLY_GRAPHQL_URL    where organizations are listed for a token that is not
                     scoped to one. Default https://api.fly.io/graphql.
  GUACA_SPACE_ORIGIN the address a space answers at. Default
                     https://<name>.fly.dev.
  GUACA_SPACE_WAIT   seconds a new image has to answer its health check
                     before it counts as failed. Default 90.
  GUACA_MAIN_FEED    the signed feed of main. Default the main-feed branch.
  GUACA_RELEASES     where signed releases are downloaded from. Default
                     https://github.com/madebywelch/guaca/releases.
  GUACA_MAIN_KEYS    the keys a build of main must be signed with. Default
                     main-keys.pub beside this repository.
  GUACA_RELEASE_KEYS the keys a release must be signed with. Default
                     release-keys.pub beside this repository.
EOF
}

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"

MEMORY_MB=1024
SWAP_MB=512
VOLUME_GB=10
VOLUME=guaca_data
PORT=8787
DATA=/var/lib/guaca
STOP_TIMEOUT=30s
STARTED_WITHIN=300
HEALTHY_WITHIN="${GUACA_SPACE_WAIT:-90}"
SNAPSHOT_WITHIN=600
TOKEN_FROM_HOST='for p in /proc/[0-9]*; do if [ "$(cat "$p/comm" 2>/dev/null)" = guacad ]; then tr "\000" "\n" < "$p/environ" | sed -n "s/^GUACA_TOKEN=//p"; exit 0; fi; done; exit 1'
ADMITS_FROM_HOST='token=$(for p in /proc/[0-9]*; do if [ "$(cat "$p/comm" 2>/dev/null)" = guacad ]; then tr "\000" "\n" < "$p/environ" | sed -n "s/^GUACA_TOKEN=//p"; break; fi; done); curl -s -o /dev/null -w "%{http_code}" -X POST -H "content-type: application/json" -H "authorization: Bearer $token" -d "{\"name\":\"capabilities\"}" http://127.0.0.1:8787/v1/call'

API="${FLY_API_HOSTNAME:-https://api.machines.dev}"
GRAPHQL="${FLY_GRAPHQL_URL:-https://api.fly.io/graphql}"
MAIN_FEED="${GUACA_MAIN_FEED:-https://raw.githubusercontent.com/madebywelch/guaca/refs/heads/main-feed/guaca-main.json}"
RELEASES="${GUACA_RELEASES:-https://github.com/madebywelch/guaca/releases}"
MAIN_KEYS="${GUACA_MAIN_KEYS:-$REPO/main-keys.pub}"
RELEASE_KEYS="${GUACA_RELEASE_KEYS:-$REPO/release-keys.pub}"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

say() { printf '%s\n' "$*"; }

COMMAND=""
value=0
for arg in "$@"; do
  if [ "$value" = 1 ]; then value=0; continue; fi
  case "$arg" in
    -h | --help | help) usage; exit 0 ;;
    --release | --region | --tz | --confirm) value=1 ;;
    -*) ;;
    *) COMMAND="$arg"; break ;;
  esac
done
case "$COMMAND" in
  "") usage; exit 0 ;;
  create | invite | status | update | destroy) ;;
  *) fail "There is no command '$COMMAND'. The commands are create, invite, status, update and destroy; --help says what each does." ;;
esac

NAME=""
IMAGE=""
RELEASE=""
REGION=iad
TZONE=America/New_York
CONFIRM=""
DRY=0
FORCE=0
SEEN_COMMAND=0
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY=1 ;;
    --force) FORCE=1 ;;
    --release) RELEASE="${2:?--release needs a version, such as 0.5.1 or latest}"; shift ;;
    --region) REGION="${2:?--region needs a Fly region code, such as iad}"; shift ;;
    --tz) TZONE="${2:?--tz needs a zone, such as America/New_York}"; shift ;;
    --confirm) CONFIRM="${2:?--confirm needs the space name again}"; shift ;;
    -h | --help) usage; exit 0 ;;
    -*) fail "There is no option '$1'. --help lists them." ;;
    *)
      if [ "$SEEN_COMMAND" = 0 ]; then SEEN_COMMAND=1
      elif [ -z "$NAME" ]; then NAME="$1"
      elif { [ "$COMMAND" = update ] || [ "$COMMAND" = create ]; } && [ -z "$IMAGE" ]; then IMAGE="$1"
      else fail "'$1' is one word too many. --help shows what $COMMAND takes."
      fi
      ;;
  esac
  shift
done

[ -n "$NAME" ] || fail "Name the space: deploy/space/space.sh $COMMAND <name>."
printf '%s' "$NAME" | grep -Eq '^[a-z0-9][a-z0-9-]{1,61}[a-z0-9]$' ||
  fail "'$NAME' cannot be a Fly app name. Use 3 to 63 lowercase letters, digits and dashes, starting and ending with a letter or digit."
printf '%s' "$REGION" | grep -Eq '^[a-z]{3}$' || fail "'$REGION' is not a Fly region code. Use three letters, such as iad."
printf '%s' "$TZONE" | grep -Eq '^[A-Za-z_]+(/[A-Za-z0-9_+-]+)*$' || fail "'$TZONE' is not a zone name. Use one such as America/New_York."
if [ -n "$IMAGE" ]; then
  printf '%s' "$IMAGE" | grep -Eq '^[a-z0-9.-]+(:[0-9]+)?/[a-z0-9._/-]+@sha256:[0-9a-f]{64}$' ||
    fail "'$IMAGE' is not pinned by digest. Name the image as repository@sha256:<64 hex digits>."
  [ -z "$RELEASE" ] || fail "Name an image or a release, not both."
fi
if [ "$COMMAND" != create ] && [ "$COMMAND" != update ] && [ -n "$RELEASE" ]; then
  fail "--release is for create and update."
fi

[ "$FORCE" = 0 ] || [ "$COMMAND" = update ] || fail "--force is for update."

if [ "$COMMAND" = destroy ] && [ "$CONFIRM" != "$NAME" ]; then
  fail "Destroying $NAME deletes its machine, its volume, every snapshot and the workspace in it, for good. Run it again with --confirm $NAME to go ahead."
fi

printf '%s' "$HEALTHY_WITHIN" | grep -Eq '^[0-9]+$' || fail "GUACA_SPACE_WAIT is '$HEALTHY_WITHIN'. Set it to a number of seconds, or leave it out for 90."

APP="$NAME"
NETWORK="$NAME-net"
ORIGIN="${GUACA_SPACE_ORIGIN:-https://$NAME.fly.dev}"
ORIGIN="${ORIGIN%/}"

for tool in curl jq; do
  command -v "$tool" >/dev/null 2>&1 || fail "This needs $tool, which is not installed. Install it, then run this again."
done

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

AUTH=""
if [ "$DRY" = 0 ]; then
  token="${FLY_API_TOKEN:-}"
  if [ -z "$token" ]; then
    flyctl="$(type -P flyctl || type -P fly || true)"
    if [ -n "$flyctl" ]; then
      token="$("$flyctl" auth token 2>/dev/null)" ||
        fail "FLY_API_TOKEN is not set, and flyctl is not logged in. Run 'fly auth login', or export a token from 'fly tokens create org', then run this again."
    fi
  fi
  token="$(printf '%s' "$token" | tr -d '\r\n')"
  [ -n "$token" ] || fail "FLY_API_TOKEN is not set. Make one with 'fly tokens create org' and export it, or log in with 'fly auth login', or add --dry-run to see what this would do."
  case "$token" in
    FlyV1\ *) AUTH="Authorization: $token" ;;
    *) AUTH="Authorization: Bearer $token" ;;
  esac
  unset token
fi

mask() {
  sed -E 's/"GUACA_TOKEN": *"[^"]*"/"GUACA_TOKEN":"****"/g'
}

dry() {
  local method="$1" path="$2" body="${3:-}"
  printf '%s %s%s\n' "$method" "$API" "$path"
  printf '  Authorization: Bearer ****\n'
  if [ -n "$body" ]; then
    printf '%s\n' "$body" | mask | sed 's/^/  /'
  fi
}

STATUS=""
BODY=""
call() {
  local method="$1" path="$2" body="${3:-}" url
  case "$path" in
    http*) url="$path" ;;
    *) url="$API$path" ;;
  esac
  local out="$SCRATCH/body"
  local args=(-sS -o "$out" -w '%{http_code}' -X "$method" -H "Accept: application/json" --max-time 120)
  if [ -n "$body" ]; then
    args+=(-H "Content-Type: application/json" --data-binary @-)
  fi
  if ! STATUS="$(printf '%s' "$body" | curl "${args[@]}" -H @<(printf '%s\n' "$AUTH") "$url" 2>"$SCRATCH/curl")"; then
    fail "Could not reach Fly at $url: $(sed 's/^curl: ([0-9]*) //' "$SCRATCH/curl" | head -1). Check this machine's connection, then run this again."
  fi
  BODY="$(cat "$out" 2>/dev/null || true)"
}

flyerror() {
  local what="$1" said
  said="$(printf '%s' "$BODY" | jq -r '.error // .message // empty' 2>/dev/null || true)"
  [ -n "$said" ] || said="$(printf '%s' "$BODY" | head -c 300)"
  case "$STATUS" in
    401 | 403) fail "Fly refused to $what: FLY_API_TOKEN is not allowed to (HTTP $STATUS: $said). Make a token for this organization with 'fly tokens create org'." ;;
    *) fail "Fly refused to $what (HTTP $STATUS): ${said:-no reason given}." ;;
  esac
}

fly() {
  local what="$1" method="$2" path="$3" body="${4:-}"
  call "$method" "$path" "$body"
  case "$STATUS" in
    2??) ;;
    *) flyerror "$what" ;;
  esac
}

org() {
  if [ -n "${FLY_ORG:-}" ]; then
    printf '%s' "$FLY_ORG"
    return
  fi
  fly "read which organizations FLY_API_TOKEN reaches" GET /v1/tokens/current
  local slugs
  slugs="$(printf '%s' "$BODY" | jq -r '[.tokens[]?.org_slug // empty] | unique | .[]')"
  if [ -z "$slugs" ]; then
    call POST "$GRAPHQL" '{"query":"{ organizations { nodes { slug } } }"}'
    case "$STATUS" in 2??) ;; *) flyerror "list the organizations FLY_API_TOKEN reaches" ;; esac
    slugs="$(printf '%s' "$BODY" | jq -r '[.data.organizations.nodes[]?.slug // empty] | unique | .[]')"
  fi
  local count
  count="$(printf '%s\n' "$slugs" | grep -c . || true)"
  [ "$count" -gt 0 ] || fail "FLY_API_TOKEN reaches no organization. Make a token with 'fly tokens create org'."
  if [ "$count" -gt 1 ]; then
    fail "FLY_API_TOKEN reaches $count organizations ($(printf '%s' "$slugs" | paste -sd, - | sed 's/,/, /g')). Set FLY_ORG to the one this space belongs in, then run this again."
  fi
  printf '%s' "$slugs"
}

verify() {
  local manifest="$1" signature="$2" keys="$3"
  [ -f "$keys" ] || fail "There are no keys to check the signature with at $keys. Run this from a checkout of the Guaca repository."
  if ! openssl pkeyutl -help 2>&1 | grep -q rawin && ! command -v node >/dev/null 2>&1; then
    fail "This machine cannot check an Ed25519 signature: it needs OpenSSL 3 or Node.js. Install one (brew install openssl@3 on a Mac), then run this again."
  fi
  printf '%s' "$signature" | tr -d ' \r\n' | base64 -d >"$SCRATCH/sig" 2>/dev/null || return 1
  local line n=0
  while IFS= read -r line || [ -n "$line" ]; do
    line="$(printf '%s' "$line" | tr -d ' \r')"
    case "$line" in "" | \#*) continue ;; esac
    n=$((n + 1))
    { printf '\060\052\060\005\006\003\053\145\160\003\041\000'; printf '%s' "$line" | base64 -d 2>/dev/null; } >"$SCRATCH/key$n.der"
    if openssl pkeyutl -verify -pubin -keyform DER -inkey "$SCRATCH/key$n.der" -rawin -in "$manifest" -sigfile "$SCRATCH/sig" >/dev/null 2>&1; then
      return 0
    fi
    if command -v node >/dev/null 2>&1 &&
      node -e 'const c=require("crypto"),f=require("fs");process.exit(c.verify(null,f.readFileSync(process.argv[1]),{key:f.readFileSync(process.argv[2]),format:"der",type:"spki"},f.readFileSync(process.argv[3]))?0:1)' "$manifest" "$SCRATCH/key$n.der" "$SCRATCH/sig" 2>/dev/null; then
      return 0
    fi
  done <"$keys"
  return 1
}

TARGET_VERSION=""
TARGET_COMMIT=""
TARGET_NAME=""
resolve() {
  if [ -n "$IMAGE" ]; then
    TARGET_NAME="$IMAGE"
    return
  fi
  local manifest="$SCRATCH/manifest.json" signature keys
  if [ -n "$RELEASE" ]; then
    local base
    if [ "$RELEASE" = latest ]; then base="$RELEASES/latest/download"; else base="$RELEASES/download/v${RELEASE#v}"; fi
    curl -fsSL --max-time 60 "$base/guaca-release.json" -o "$manifest" 2>/dev/null ||
      fail "Could not read Guaca release $RELEASE from $base. Check the version and this machine's connection."
    signature="$(curl -fsSL --max-time 60 "$base/guaca-release.json.sig" 2>/dev/null)" ||
      fail "Guaca release $RELEASE has no signature, so it is not installed."
    keys="$RELEASE_KEYS"
  else
    local feed
    feed="$(curl -fsSL --max-time 60 "$MAIN_FEED" 2>/dev/null)" ||
      fail "Could not read the latest build of main from $MAIN_FEED. Check this machine's connection, or name a release with --release."
    printf '%s' "$feed" | jq -r '.manifest // empty' 2>/dev/null | base64 -d >"$manifest" 2>/dev/null ||
      fail "The latest build of main is unreadable. Try again after the next push."
    signature="$(printf '%s' "$feed" | jq -r '.signature // empty')"
    keys="$MAIN_KEYS"
  fi
  verify "$manifest" "$signature" "$keys" ||
    fail "That build is not signed with Guaca's key, so it is not installed."
  IMAGE="$(jq -r '.image // empty' "$manifest")"
  TARGET_VERSION="$(jq -r '.version // empty' "$manifest")"
  TARGET_COMMIT="$(jq -r '.commit // empty' "$manifest")"
  printf '%s' "$IMAGE" | grep -Eq '^ghcr\.io/madebywelch/guaca/guacad@sha256:[0-9a-f]{64}$' ||
    fail "That build names no image pinned by digest, so it is not installed."
  if [ -n "$RELEASE" ]; then TARGET_NAME="Guaca $TARGET_VERSION"; else TARGET_NAME="main at ${TARGET_COMMIT:0:7}"; fi
}

machine_config() {
  local image="$1" volume="$2"
  jq -n --arg image "$image" --arg volume "$volume" --arg tz "$TZONE" --arg origin "$ORIGIN" \
    --arg data "$DATA" --arg stop "$STOP_TIMEOUT" --arg name "$NAME" \
    --argjson memory "$MEMORY_MB" --argjson swap "$SWAP_MB" --argjson port "$PORT" '{
    image: $image,
    guest: { cpu_kind: "shared", cpus: 1, memory_mb: $memory },
    init: { swap_size_mb: $swap },
    env: { TZ: $tz, GUACA_ORIGIN: $origin, GUACA_SPACE: "1" },
    processes: [{
      name: "guacad",
      user: "root",
      exec: ["/bin/sh", "-c", ("chown guaca:guaca " + $data + " && exec setpriv --reuid=guaca --regid=guaca --init-groups /usr/local/bin/guacad")]
    }],
    mounts: [{ volume: $volume, path: $data }],
    restart: { policy: "always" },
    stop_config: { signal: "SIGTERM", timeout: $stop },
    services: [{
      protocol: "tcp",
      internal_port: $port,
      autostop: "off",
      autostart: true,
      min_machines_running: 1,
      ports: [
        { port: 443, handlers: ["tls", "http"] },
        { port: 80, handlers: ["http"], force_https: true }
      ],
      checks: [{ type: "http", method: "GET", path: "/health", protocol: "http", interval: "15s", timeout: "5s", grace_period: "10s" }]
    }],
    metadata: { guaca_space: $name }
  }'
}

healthy() {
  local deadline=$((SECONDS + HEALTHY_WITHIN)) answer
  while [ "$SECONDS" -lt "$deadline" ]; do
    if answer="$(curl -fsS --max-time 5 "$ORIGIN/health" 2>/dev/null)" && [ "$(printf '%s' "$answer" | jq -r '.status // empty' 2>/dev/null)" = ok ]; then
      if [ -z "${1:-}" ] || printf '%s' "$answer" | jq -e --arg want "$1" '(.build // "") as $b | ($b != "" and ($want | startswith($b[0:7])))' >/dev/null 2>&1; then
        HEALTH="$answer"
        return 0
      fi
    fi
    sleep 2
  done
  return 1
}

started() {
  local id="$1" deadline=$((SECONDS + STARTED_WITHIN))
  while [ "$SECONDS" -lt "$deadline" ]; do
    call GET "/v1/apps/$APP/machines/$id/wait?state=started&timeout=60"
    case "$STATUS" in
      2??) return 0 ;;
      408) ;;
      *)
        call GET "/v1/apps/$APP/machines/$id"
        case "$(printf '%s' "$BODY" | jq -r '.state // empty' 2>/dev/null)" in
          started) return 0 ;;
          failed | destroyed) return 1 ;;
        esac
        sleep 2
        ;;
    esac
  done
  return 1
}

launch() {
  local id="$1" deadline=$((SECONDS + STARTED_WITHIN))
  while [ "$SECONDS" -lt "$deadline" ]; do
    call GET "/v1/apps/$APP/machines/$id"
    case "$(printf '%s' "$BODY" | jq -r '.state // empty' 2>/dev/null)" in
      started) return 0 ;;
      stopped) call POST "/v1/apps/$APP/machines/$id/start" ;;
    esac
    sleep 2
  done
}

stopped() {
  local id="$1" deadline=$((SECONDS + STARTED_WITHIN))
  while [ "$SECONDS" -lt "$deadline" ]; do
    call GET "/v1/apps/$APP/machines/$id/wait?state=stopped&timeout=60"
    case "$STATUS" in
      2??) return 0 ;;
      408) ;;
      *)
        call GET "/v1/apps/$APP/machines/$id"
        [ "$(printf '%s' "$BODY" | jq -r '.state // empty' 2>/dev/null)" = stopped ] && return 0
        sleep 2
        ;;
    esac
  done
  return 1
}

show_invitation() {
  local token="$1"
  if { : >/dev/tty; } 2>/dev/null; then
    {
      printf '\nThe invitation to %s. It is the space'"'"'s password: open it, keep it, and do not paste it anywhere else.\n\n' "$NAME"
      printf '  %s/#token=%s\n\n' "$ORIGIN" "$token"
    } >/dev/tty
  else
    say "There is no terminal here to show the invitation on, so it was not shown. Run 'deploy/space/space.sh invite $NAME' in a terminal to see it."
  fi
}

host_token() {
  local id="$1" body
  body="$(jq -n --arg script "$TOKEN_FROM_HOST" '{command: ["setpriv", "--reuid=guaca", "--regid=guaca", "--init-groups", "/bin/sh", "-c", $script], timeout: 30}')"
  fly "read the space's token from its machine" POST "/v1/apps/$APP/machines/$id/exec" "$body"
  printf '%s' "$BODY" | jq -r '.stdout // empty' | tr -d '\r\n'
}

the_machine() {
  fly "list the space's machines" GET "/v1/apps/$APP/machines"
  local count
  count="$(printf '%s' "$BODY" | jq '[.[] | select(.state != "destroyed")] | length')"
  [ "$count" -le 1 ] || fail "$APP has $count machines, and a space has one. Remove the extra ones in the Fly dashboard, then run this again."
  MACHINE_JSON="$(printf '%s' "$BODY" | jq -c '[.[] | select(.state != "destroyed")][0] // empty')"
  MACHINE="$(printf '%s' "$MACHINE_JSON" | jq -r '.id // empty')"
}

app_exists() {
  call GET "/v1/apps/$APP"
  case "$STATUS" in
    2??) return 0 ;;
    404) return 1 ;;
    403)
      [ "${1:-}" != owned ] || fail "The name $APP is taken on Fly by another organization, so this space cannot have it. Pick another name."
      flyerror "look up $APP"
      ;;
    *) flyerror "look up $APP" ;;
  esac
}

cmd_create() {
  resolve
  local slug
  if [ "$DRY" = 1 ]; then
    slug="${FLY_ORG:-<the one organization the token reaches>}"
    say "Would create the space $NAME in $slug, region $REGION, from $TARGET_NAME:"
    say ""
    [ -n "${FLY_ORG:-}" ] || dry GET /v1/tokens/current
    dry GET "/v1/apps/$APP"
    dry POST /v1/apps "$(jq -n --arg n "$APP" --arg o "$slug" --arg net "$NETWORK" '{name: $n, org_slug: $o, network: $net}')"
    dry GET "/v1/apps/$APP/ip_assignments"
    dry POST "/v1/apps/$APP/ip_assignments" '{"type":"shared_v4"}'
    dry POST "/v1/apps/$APP/ip_assignments" '{"type":"v6"}'
    dry GET "/v1/apps/$APP/secrets"
    dry POST "/v1/apps/$APP/secrets" '{"values":{"GUACA_TOKEN":"****"}}'
    dry GET "/v1/apps/$APP/volumes"
    dry POST "/v1/apps/$APP/volumes" "$(jq -n --arg n "$VOLUME" --arg r "$REGION" --argjson gb "$VOLUME_GB" --argjson m "$MEMORY_MB" \
      '{name: $n, region: $r, size_gb: $gb, encrypted: true, auto_backup_enabled: true, compute: {cpu_kind: "shared", cpus: 1, memory_mb: $m}}')"
    dry GET "/v1/apps/$APP/machines"
    dry POST "/v1/apps/$APP/machines" "$(jq -n --arg r "$REGION" --argjson c "$(machine_config "$IMAGE" "<the volume's id>")" \
      '{name: "guacad", region: $r, config: $c, min_secrets_version: "<the version the secrets call returned>"}')"
    dry GET "/v1/apps/$APP/machines/<the machine's id>/wait?state=started&timeout=60"
    say "GET $ORIGIN/health"
    say ""
    say "Running create again finds each of these and makes none of them twice. The token is generated here and never replaced."
    return
  fi

  slug="$(org)"
  if app_exists owned; then
    local owner
    owner="$(printf '%s' "$BODY" | jq -r '.organization.slug // empty')"
    [ -z "$owner" ] || [ "$owner" = "$slug" ] ||
      fail "A Fly app named $APP already exists in $owner, not $slug. Pick another name, or set FLY_ORG=$owner."
    fly "list the apps in $slug" GET "/v1/apps?org_slug=$slug"
    local net
    net="$(printf '%s' "$BODY" | jq -r --arg a "$APP" '.apps[]? | select(.name == $a) | .network // empty')"
    [ -z "$net" ] || [ "$net" = "$NETWORK" ] ||
      fail "$APP is on the private network '$net', not one of its own, so it could reach other apps. Destroy it and create it again, or pick another name."
    say "The app $APP exists."
  else
    call POST /v1/apps "$(jq -n --arg n "$APP" --arg o "$slug" --arg net "$NETWORK" '{name: $n, org_slug: $o, network: $net}')"
    case "$STATUS" in
      2??) ;;
      409 | 422)
        printf '%s' "$BODY" | grep -qi 'taken\|exists' && fail "The name $APP is taken on Fly by someone else. Pick another name."
        flyerror "create the app $APP"
        ;;
      *) flyerror "create the app $APP" ;;
    esac
    say "Created the app $APP on its own private network, $NETWORK."
  fi

  fly "list $APP's addresses" GET "/v1/apps/$APP/ip_assignments"
  local ips="$BODY" kind
  for kind in shared_v4 v6; do
    if ! printf '%s' "$ips" | jq -e --arg k "$kind" '(.ips // .) | map(select(if $k == "shared_v4" then (.shared == true) else ((.ip // "") | test(":")) and ((.ip // "") | startswith("fdaa") | not) end)) | length > 0' >/dev/null 2>&1; then
      fly "give $APP a public address" POST "/v1/apps/$APP/ip_assignments" "{\"type\":\"$kind\"}"
      say "Gave $APP a public $kind address."
    fi
  done

  fly "list $APP's secrets" GET "/v1/apps/$APP/secrets"
  local secrets_version="" token=""
  if printf '%s' "$BODY" | jq -e '(.secrets // .) | map(select(.name == "GUACA_TOKEN")) | length > 0' >/dev/null 2>&1; then
    say "The space's token is already set, and is kept."
  else
    token="$(od -An -tx1 -N32 /dev/urandom | tr -d ' \n')"
    [ "${#token}" = 64 ] || fail "Could not generate a token on this machine."
    fly "save the space's token" POST "/v1/apps/$APP/secrets" "$(jq -n --arg t "$token" '{values: {GUACA_TOKEN: $t}}')"
    secrets_version="$(printf '%s' "$BODY" | jq -r '.version // empty')"
    say "Generated the space's token and saved it as a Fly secret."
  fi

  fly "list $APP's volumes" GET "/v1/apps/$APP/volumes"
  local volumes volume
  volumes="$(printf '%s' "$BODY" | jq -c --arg n "$VOLUME" '[.[] | select(.name == $n and ((.state // "") | test("destroy") | not))]')"
  the_machine
  if [ -n "$MACHINE" ]; then
    volume="$(printf '%s' "$MACHINE_JSON" | jq -r '.config.mounts[0].volume // empty')"
    say "The machine $MACHINE exists, on the volume $volume; nothing new is made."
  else
    case "$(printf '%s' "$volumes" | jq length)" in
      0)
        fly "make the space's volume" POST "/v1/apps/$APP/volumes" "$(jq -n --arg n "$VOLUME" --arg r "$REGION" --argjson gb "$VOLUME_GB" --argjson m "$MEMORY_MB" \
          '{name: $n, region: $r, size_gb: $gb, encrypted: true, auto_backup_enabled: true, compute: {cpu_kind: "shared", cpus: 1, memory_mb: $m}}')"
        volume="$(printf '%s' "$BODY" | jq -r '.id')"
        say "Made the $VOLUME_GB GB volume $volume in $REGION."
        ;;
      1)
        volume="$(printf '%s' "$volumes" | jq -r '.[0].id')"
        say "The volume $volume exists, and is used."
        ;;
      *)
        fail "$APP has $(printf '%s' "$volumes" | jq length) volumes named $VOLUME and no machine, so it is not clear which holds the workspace. Keep the one you want and delete the others in the Fly dashboard, then run this again."
        ;;
    esac
    local request
    request="$(jq -n --arg r "$REGION" --argjson c "$(machine_config "$IMAGE" "$volume")" --arg v "$secrets_version" \
      '{name: "guacad", region: $r, config: $c} + (if $v == "" then {} else {min_secrets_version: ($v | tonumber? // $v)} end)')"
    fly "start the space's machine" POST "/v1/apps/$APP/machines" "$request"
    MACHINE="$(printf '%s' "$BODY" | jq -r '.id')"
    say "Started the machine $MACHINE from $TARGET_NAME."
  fi

  started "$MACHINE" || fail "The machine $MACHINE did not start within ${STARTED_WITHIN}s. 'deploy/space/space.sh status $NAME' shows its state, and the Fly dashboard its logs."
  healthy || fail "The machine started, but $ORIGIN/health did not answer within ${HEALTHY_WITHIN}s. 'deploy/space/space.sh status $NAME' shows what Fly sees."
  say "$ORIGIN answers its health check."
  [ -n "$token" ] || token="$(host_token "$MACHINE")"
  [ -n "$token" ] || fail "The space is up, but its token could not be read back from the machine. 'deploy/space/space.sh invite $NAME' tries again."
  show_invitation "$token"
}

cmd_invite() {
  if [ "$DRY" = 1 ]; then
    dry GET "/v1/apps/$APP/machines"
    dry POST "/v1/apps/$APP/machines/<the machine's id>/exec" "$(jq -n '{command: ["/bin/sh", "-c", "<read GUACA_TOKEN from the guacad process>"], timeout: 30}')"
    say "The invitation is then shown on this terminal only."
    return
  fi
  app_exists || fail "There is no space named $NAME. 'deploy/space/space.sh create $NAME' makes it."
  the_machine
  [ -n "$MACHINE" ] || fail "$NAME has no machine. 'deploy/space/space.sh create $NAME' finishes making it."
  local token
  token="$(host_token "$MACHINE")"
  [ -n "$token" ] || fail "The token could not be read from the machine $MACHINE. Is it running? 'deploy/space/space.sh status $NAME' says."
  show_invitation "$token"
}

cmd_status() {
  if [ "$DRY" = 1 ]; then
    dry GET "/v1/apps/$APP"
    dry GET "/v1/apps/$APP/machines"
    dry GET "/v1/apps/$APP/volumes"
    dry GET "/v1/apps/$APP/volumes/<the volume's id>/snapshots"
    dry POST "/v1/apps/$APP/machines/<the machine's id>/exec" "$(jq -n '{command: ["/bin/sh", "-c", "<ask the host whether its own token admits a call>"], timeout: 30}')"
    say "GET $ORIGIN/health"
    say "POST $ORIGIN/v1/call with a wrong token"
    return
  fi
  app_exists || fail "There is no space named $NAME. 'deploy/space/space.sh create $NAME' makes it."
  say "Space    $NAME ($ORIGIN)"
  the_machine
  if [ -z "$MACHINE" ]; then
    say "Machine  none. 'deploy/space/space.sh create $NAME' finishes making it."
  else
    say "Machine  $MACHINE, $(printf '%s' "$MACHINE_JSON" | jq -r '.state'), in $(printf '%s' "$MACHINE_JSON" | jq -r '.region'), $(printf '%s' "$MACHINE_JSON" | jq -r '.config.guest.memory_mb') MB"
    say "Image    $(printf '%s' "$MACHINE_JSON" | jq -r '.config.image')"
    local digest
    digest="$(printf '%s' "$MACHINE_JSON" | jq -r '.image_ref.digest // empty')"
    [ -z "$digest" ] || say "Digest   $digest"
    say "Checks   $(printf '%s' "$MACHINE_JSON" | jq -r 'if (.checks // []) | length == 0 then "none reported yet" else [.checks[] | "\(.name) \(.status)"] | join(", ") end')"
  fi
  local health
  if health="$(curl -fsS --max-time 10 "$ORIGIN/health" 2>/dev/null)"; then
    say "Health   $(printf '%s' "$health" | jq -r '"\(.status), Guaca \(.version // "unknown") at \(.build // "an unknown build")"' 2>/dev/null || printf 'answered')"
  else
    say "Health   $ORIGIN/health does not answer"
  fi
  local wrong
  wrong="$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 -X POST -H 'content-type: application/json' \
    -H @<(printf 'authorization: Bearer %s\n' "not-$(od -An -tx1 -N8 /dev/urandom | tr -d ' \n')") \
    -d '{"name":"capabilities"}' "$ORIGIN/v1/call" 2>/dev/null || true)"
  case "$wrong" in
    401) say "Access   a wrong token is refused" ;;
    *) say "Access   a wrong token was answered $wrong, not 401" ;;
  esac
  if [ -n "$MACHINE" ] && [ "$(printf '%s' "$MACHINE_JSON" | jq -r '.state')" = started ]; then
    local admits
    call POST "/v1/apps/$APP/machines/$MACHINE/exec" "$(jq -n --arg s "$ADMITS_FROM_HOST" '{command: ["setpriv", "--reuid=guaca", "--regid=guaca", "--init-groups", "/bin/sh", "-c", $s], timeout: 30}')"
    admits="$(printf '%s' "$BODY" | jq -r '.stdout // empty' 2>/dev/null | tr -d '\r\n')"
    case "$STATUS:$admits" in
      2??:200) say "Token    the space's own token admits a call" ;;
      2??:*) say "Token    the space's own token was answered ${admits:-nothing}" ;;
      *) say "Token    could not be checked from inside the machine (HTTP $STATUS)" ;;
    esac
  fi
  fly "list $APP's volumes" GET "/v1/apps/$APP/volumes"
  local volumes="$BODY" attached
  attached="$(printf '%s' "$MACHINE_JSON" | jq -r '.config.mounts[0].volume // empty' 2>/dev/null)"
  printf '%s' "$volumes" | jq -r --arg a "$attached" '.[] | select((.state // "") | test("destroy") | not) |
    (if (.blocks // 0) > 0 then "\(((.blocks - .blocks_free) * .block_size / 1073741824 * 10 | floor) / 10) GB used of \(.size_gb) GB (\(((.blocks - .blocks_free) * 100 / .blocks) | floor)% full)" else "\(.size_gb) GB, fill not reported yet" end) as $fill |
    if .id == $a then "Volume   \(.id), \($fill)" else "Kept     \(.id), \($fill), not in use: left by a failed update; delete it once you no longer need it" end'
  if [ -n "$attached" ]; then
    fly "list the volume's snapshots" GET "/v1/apps/$APP/volumes/$attached/snapshots"
    say "Snaps    $(printf '%s' "$BODY" | jq -r 'if length == 0 then "none yet" else "\(length), the latest \(map(select(.status == "created")) | sort_by(.created_at) | last | .created_at // "in progress")" end')"
  fi
}

cmd_update() {
  if [ "$DRY" = 1 ]; then
    resolve
    say "Would update the space $NAME to $TARGET_NAME:"
    say ""
    say "GET $ORIGIN/health"
    dry GET "/v1/apps/$APP/machines"
    dry POST "/v1/apps/$APP/machines/<the machine's id>/stop" '{"signal":"SIGTERM","timeout":"'"$STOP_TIMEOUT"'"}'
    dry GET "/v1/apps/$APP/machines/<the machine's id>/wait?state=stopped&timeout=60"
    dry POST "/v1/apps/$APP/volumes/<the volume's id>/snapshots"
    dry GET "/v1/apps/$APP/volumes/<the volume's id>/snapshots"
    dry POST "/v1/apps/$APP/machines/<the machine's id>" "$(jq -n --argjson c "$(machine_config "$IMAGE" "<the volume's id>")" '{config: $c, current_version: "<the machine version read above>"}')"
    dry POST "/v1/apps/$APP/machines/<the machine's id>/start"
    dry GET "/v1/apps/$APP/machines/<the machine's id>/wait?state=started&timeout=60"
    say "GET $ORIGIN/health, until it reports the new build"
    say ""
    say "If the new build does not answer, the workspace it may have migrated is kept, and the old image runs again only on a copy of the snapshot taken before it:"
    say ""
    dry POST "/v1/apps/$APP/volumes" "$(jq -n --arg n "$VOLUME" --arg r "<the machine's region>" '{name: $n, region: $r, snapshot_id: "<the snapshot taken above>", size_gb: "<the volume'"'"'s size>"}')"
    dry DELETE "/v1/apps/$APP/machines/<the machine's id>?force=true"
    dry POST "/v1/apps/$APP/machines" "$(jq -n '{name: "guacad", region: "<the machine'"'"'s region>", config: "<the previous config, on the restored volume>"}')"
    return
  fi

  app_exists || fail "There is no space named $NAME. 'deploy/space/space.sh create $NAME' makes it."
  the_machine
  [ -n "$MACHINE" ] || fail "$NAME has no machine to update. 'deploy/space/space.sh create $NAME' finishes making it."
  resolve
  local current version region config volume
  current="$(printf '%s' "$MACHINE_JSON" | jq -r '.config.image')"
  version="$(printf '%s' "$MACHINE_JSON" | jq -r '.version // .instance_id // empty')"
  region="$(printf '%s' "$MACHINE_JSON" | jq -r '.region')"
  config="$(printf '%s' "$MACHINE_JSON" | jq -c '.config')"
  volume="$(printf '%s' "$config" | jq -r '.mounts[0].volume // empty')"
  [ -n "$volume" ] || fail "The machine $MACHINE mounts no volume, so there is no workspace to keep safe. Nothing was changed."
  if [ "${current##*@}" = "${IMAGE##*@}" ] && [ "$FORCE" = 0 ]; then
    say "$NAME already runs $TARGET_NAME ($IMAGE). Nothing to do; --force snapshots and swaps it anyway."
    return
  fi

  local running=""
  running="$(curl -fsS --max-time 10 "$ORIGIN/health" 2>/dev/null | jq -r '.version // empty' 2>/dev/null || true)"
  if [ -n "$running" ] && [ -n "$TARGET_VERSION" ]; then
    local older
    older="$(printf '%s\n%s\n' "$TARGET_VERSION" "$running" | sort -t. -k1,1n -k2,2n -k3,3n | head -1)"
    if [ "$older" = "$TARGET_VERSION" ] && [ "$TARGET_VERSION" != "$running" ]; then
      fail "$NAME runs Guaca $running, and $TARGET_NAME is older. An older Guaca must never run on a workspace a newer one may have migrated, so nothing was changed. To go back, restore a snapshot from before the newer version into a new volume."
    fi
  fi

  local vsize
  fly "read the volume $volume" GET "/v1/apps/$APP/volumes/$volume"
  vsize="$(printf '%s' "$BODY" | jq -r '.size_gb')"
  [ "$(printf '%s' "$BODY" | jq -r '.type // "local"')" != cache ] ||
    fail "The volume $volume cannot be snapshotted, so a failed update could not be undone safely. Nothing was changed."

  say "Stopping $MACHINE; agents put their work down and carry on after the update."
  fly "stop the machine $MACHINE" POST "/v1/apps/$APP/machines/$MACHINE/stop" "{\"signal\":\"SIGTERM\",\"timeout\":\"$STOP_TIMEOUT\"}"
  stopped "$MACHINE" || fail "The machine $MACHINE did not stop within ${STARTED_WITHIN}s, so nothing was changed. 'deploy/space/space.sh status $NAME' shows its state."

  local before snapshot=""
  fly "list the volume's snapshots" GET "/v1/apps/$APP/volumes/$volume/snapshots"
  before="$(printf '%s' "$BODY" | jq -c '[.[].id]')"
  call POST "/v1/apps/$APP/volumes/$volume/snapshots"
  case "$STATUS" in
    2??) ;;
    *)
      call POST "/v1/apps/$APP/machines/$MACHINE/start"
      flyerror "snapshot the volume $volume, so nothing was changed and $MACHINE was started again on its old image"
      ;;
  esac
  local deadline=$((SECONDS + SNAPSHOT_WITHIN))
  while [ "$SECONDS" -lt "$deadline" ]; do
    fly "list the volume's snapshots" GET "/v1/apps/$APP/volumes/$volume/snapshots"
    snapshot="$(printf '%s' "$BODY" | jq -r --argjson before "$before" '[.[] | select(.status == "created" and ((.id as $i | $before | index($i)) | not))] | sort_by(.created_at) | last | .id // empty')"
    [ -z "$snapshot" ] || break
    sleep 3
  done
  if [ -z "$snapshot" ]; then
    call POST "/v1/apps/$APP/machines/$MACHINE/start"
    fail "The snapshot of $volume did not finish within ${SNAPSHOT_WITHIN}s, so the update could not be undone safely. Nothing was changed, and $MACHINE was started again on its old image."
  fi
  say "Took the snapshot $snapshot of $volume."

  local next
  next="$(printf '%s' "$config" | jq -c --arg image "$IMAGE" '.image = $image')"
  call POST "/v1/apps/$APP/machines/$MACHINE" "$(jq -n --argjson c "$next" --arg v "$version" '{config: $c} + (if $v == "" then {} else {current_version: $v} end)')"
  case "$STATUS" in
    2??) ;;
    *)
      local refused="$STATUS"
      call POST "/v1/apps/$APP/machines/$MACHINE/start"
      fail "Fly refused to replace the image (HTTP $refused), so the old image was started again on the untouched volume. Snapshot $snapshot is kept."
      ;;
  esac
  launch "$MACHINE"
  say "Replaced the image with $TARGET_NAME; waiting for it to answer."
  if started "$MACHINE" && healthy "$TARGET_COMMIT"; then
    say "$NAME now runs $TARGET_NAME and answers its health check. The snapshot $snapshot is what a restore would go back to."
    return
  fi

  say "$TARGET_NAME did not answer within ${HEALTHY_WITHIN}s, so the previous version is being put back."
  local restored
  call POST "/v1/apps/$APP/volumes" "$(jq -n --arg n "$VOLUME" --arg r "$region" --arg s "$snapshot" --argjson gb "$vsize" '{name: $n, region: $r, snapshot_id: $s, size_gb: $gb}')"
  case "$STATUS" in
    2??) restored="$(printf '%s' "$BODY" | jq -r '.id')" ;;
    *) fail "Recovery needed: the snapshot $snapshot could not be restored (HTTP $STATUS). The machine $MACHINE still has $TARGET_NAME on the volume $volume, which it may have migrated, so the old image was not started on it. Restore $snapshot into a new volume and run $current on that." ;;
  esac
  call DELETE "/v1/apps/$APP/machines/$MACHINE?force=true"
  case "$STATUS" in
    2??) ;;
    *) fail "Recovery needed: the failed machine $MACHINE could not be removed (HTTP $STATUS). The restored copy of the workspace is the volume $restored; run $current on it." ;;
  esac
  local back
  back="$(printf '%s' "$config" | jq -c --arg v "$restored" '.mounts[0].volume = $v')"
  call POST "/v1/apps/$APP/machines" "$(jq -n --arg r "$region" --argjson c "$back" '{name: "guacad", region: $r, config: $c}')"
  case "$STATUS" in
    2??) MACHINE="$(printf '%s' "$BODY" | jq -r '.id')" ;;
    *) fail "Recovery needed: the previous version could not be started on the restored volume $restored (HTTP $STATUS). Start $current on it by hand." ;;
  esac
  if started "$MACHINE" && healthy; then
    fail "The update to $TARGET_NAME did not finish, and the previous version is back on $restored, with the workspace as it was when the update began. The workspace the failed version left is kept as $volume."
  fi
  fail "Recovery needed: the previous version was started on the restored volume $restored as $MACHINE, but does not answer. The workspace the failed version left is kept as $volume, and the snapshot as $snapshot."
}

cmd_destroy() {
  if [ "$DRY" = 1 ]; then
    dry GET "/v1/apps/$APP/machines"
    dry GET "/v1/apps/$APP/volumes"
    dry DELETE "/v1/apps/$APP?force=true"
    return
  fi
  app_exists || fail "There is no space named $NAME, so nothing was deleted."
  fly "list the space's machines" GET "/v1/apps/$APP/machines"
  local machines="$BODY"
  fly "list $APP's volumes" GET "/v1/apps/$APP/volumes"
  local volumes="$BODY"
  fly "delete the app $APP" DELETE "/v1/apps/$APP?force=true"
  say "Deleted the space $NAME: the Fly app $APP and with it"
  printf '%s' "$machines" | jq -r '.[] | select(.state != "destroyed") | "  the machine \(.id) (\(.config.image))"'
  printf '%s' "$volumes" | jq -r '.[] | select((.state // "") | test("destroy") | not) | "  the volume \(.id) (\(.size_gb) GB) and its snapshots"'
  say "  its addresses and its token."
}

"cmd_$COMMAND"
