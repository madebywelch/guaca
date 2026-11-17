#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

readonly TLA_VERSION=1.7.4
readonly TLA_SHA256=936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88
readonly MODEL_DIR=formal/recovery
readonly CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/guaca"
readonly JAR="$CACHE/tla2tools-$TLA_VERSION.jar"

readonly CODE="MarkBeforeRead=TRUE AcceptOneCommit=TRUE RecoverUntouched=TRUE RecoverClears=TRUE"
readonly CALM="OperatorStops=FALSE StartCanFail=FALSE"
readonly STOPS="OperatorStops=TRUE StartCanFail=FALSE"
readonly FAILS="OperatorStops=FALSE StartCanFail=TRUE"
readonly ALL="NoBlindRepeat NothingLost OnlyStartedReported ReportedOnce StoppedStaysStopped"

CHECKS=(
  "holds|code, no stops, no failed commit|$CODE $CALM|$ALL"
  "holds|code, operator stops and a failed start mark|$CODE OperatorStops=TRUE StartCanFail=TRUE|OnlyStartedReported ReportedOnce"
  "finding|a stop of the conversation a turn took a message from settles that message unanswered|$CODE $STOPS|NothingLost"
  "finding|a stop during a drain, or of a message taken into a turn, is undone by the next host|$CODE $STOPS|StoppedStaysStopped"
  "finding|a stopped conversation handed on is later delivered blind|$CODE $STOPS|NoBlindRepeat"
  "finding|a failed start mark settles the conversation unanswered and unreported|$CODE $FAILS|NothingLost"
  "broken|model read before the start mark|MarkBeforeRead=FALSE AcceptOneCommit=TRUE RecoverUntouched=TRUE RecoverClears=TRUE $CALM|NoBlindRepeat"
  "broken|pending_runs row in its own commit|MarkBeforeRead=TRUE AcceptOneCommit=FALSE RecoverUntouched=TRUE RecoverClears=TRUE $CALM|NothingLost"
  "broken|recover reports every unfinished conversation|MarkBeforeRead=TRUE AcceptOneCommit=TRUE RecoverUntouched=FALSE RecoverClears=TRUE $CALM|OnlyStartedReported"
  "broken|recover keeps the rows it reported|MarkBeforeRead=TRUE AcceptOneCommit=TRUE RecoverUntouched=TRUE RecoverClears=FALSE $CALM|ReportedOnce"
)

fetch_tools() {
  command -v java >/dev/null || { echo "formal: java 11 or newer is required" >&2; exit 1; }
  if [ ! -f "$JAR" ]; then
    mkdir -p "$CACHE"
    curl -fsSL -o "$JAR.part" "https://github.com/tlaplus/tlaplus/releases/download/v$TLA_VERSION/tla2tools.jar"
    mv "$JAR.part" "$JAR"
  fi
  echo "$TLA_SHA256  $JAR" | sha256sum -c --quiet - || { rm -f "$JAR"; echo "formal: tla2tools.jar does not match its pinned checksum" >&2; exit 1; }
}

run_tlc() {
  local constants=$1 invariants=$2 work=$3
  {
    echo "SPECIFICATION Spec"
    echo "CONSTANTS"
    echo "    MaxEnds = ${MAX_ENDS:-2}"
    for pair in $constants; do echo "    ${pair%%=*} = ${pair#*=}"; done
    echo "INVARIANTS"
    echo "    TypeOK"
    for name in $invariants; do echo "    $name"; done
  } >"$work/Recovery.cfg"
  cp "$MODEL_DIR/Recovery.tla" "$work/"
  (cd "$work" && java -XX:+UseParallelGC -cp "$JAR" tlc2.TLC -workers 1 -deadlock \
    -metadir "$work/states" -config Recovery.cfg Recovery.tla) >"$work/tlc.out" 2>&1 || true
}

check() {
  local expect=$1 what=$2 constants=$3 invariants=$4
  local work
  work=$(mktemp -d)
  run_tlc "$constants" "$invariants" "$work"
  local out="$work/tlc.out" violated=""
  if grep -q "Model checking completed. No error has been found." "$out"; then
    violated=none
  else
    violated=$(sed -n 's/^Error: Invariant \([A-Za-z]*\) is violated\.$/\1/p' "$out" | head -1)
  fi
  local states
  states=$(sed -n 's/.* \([0-9]*\) distinct states found.*/\1/p' "$out" | tail -1)
  case "$expect:$violated" in
    holds:none)
      printf 'ok    holds     %-22s %s (%s states)\n' "$invariants" "$what" "$states" | sed 's/NoBlindRepeat NothingLost OnlyStartedReported ReportedOnce StoppedStaysStopped/all five/'
      rm -rf "$work"; return 0 ;;
    finding:"$invariants"|broken:"$invariants")
      printf 'ok    %-9s %-22s %s\n' "$expect" "$invariants" "$what"
      rm -rf "$work"; return 0 ;;
    finding:none)
      printf 'FAIL  %s now holds: %s. Move it to the holds rows in scripts/formal.sh.\n' "$invariants" "$what" >&2 ;;
    broken:none)
      printf 'FAIL  the broken variant "%s" no longer breaks %s: the model has lost the step that guards it.\n' "$what" "$invariants" >&2 ;;
    *)
      printf 'FAIL  %s (%s): expected %s, TLC reported %s. Output: %s\n' "$what" "$invariants" "$expect" "${violated:-an error}" "$out" >&2 ;;
  esac
  [ "${violated:-}" = "" ] && tail -20 "$out" >&2
  return 1
}

fetch_tools

if [ "${1:-}" = "--trace" ]; then
  shift
  work=$(mktemp -d)
  run_tlc "$CODE $1" "$2" "$work"
  cat "$work/tlc.out"
  exit 0
fi

failed=0
for row in "${CHECKS[@]}"; do
  IFS='|' read -r expect what constants invariants <<<"$row"
  check "$expect" "$what" "$constants" "$invariants" || failed=1
done
exit $failed
