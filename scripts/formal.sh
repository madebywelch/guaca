#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

readonly TLA_VERSION=1.7.4
readonly TLA_SHA256=936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88
readonly CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/guaca"
readonly JAR="$CACHE/tla2tools-$TLA_VERSION.jar"

readonly CODE="MarkBeforeRead=TRUE AcceptOneCommit=TRUE RecoverUntouched=TRUE RecoverClears=TRUE KeepLink=TRUE SkipStartNote=TRUE"
readonly CALM="OperatorStops=FALSE StartCanFail=FALSE TurnsCanFail=FALSE"
readonly STOPS="OperatorStops=TRUE StartCanFail=FALSE TurnsCanFail=FALSE"
readonly FAILS="OperatorStops=FALSE StartCanFail=TRUE TurnsCanFail=FALSE"
readonly ALL="NoBlindRepeat NothingLost OnlyStartedReported ReportedOnce StoppedStaysStopped"
readonly RELAY="AckAfterCommit=TRUE FindFirst=TRUE CatchUpKeyed=TRUE PlaceRunOneCommit=TRUE AmbiguousHeld=TRUE OrderedByCommit=TRUE PressChecked=TRUE Relinks=TRUE RelinkRetires=FALSE TellFailUnheard=TRUE Loops={1,2} RoundLock=TRUE ConditionalClaim=TRUE AnswerChecksPending=TRUE"
readonly PROPERTIES="EventuallyTerminal"
readonly RELAY_ALL="HeardOnce HeardOrToldWhy ShownOnce NothingDropped InOrder DecidedOnce PressedByOwnerAtAsk NoSilentDrop"

CHECKS=(
  "recovery|holds|code, no stops, no failed commit|$CODE $CALM|$ALL"
  "recovery|holds|code, operator stops, a failed start mark, and turns that end on a failed model call or an exhausted budget|$CODE OperatorStops=TRUE StartCanFail=TRUE TurnsCanFail=TRUE|$ALL"
  "recovery|broken|model read before the start mark|MarkBeforeRead=FALSE AcceptOneCommit=TRUE RecoverUntouched=TRUE RecoverClears=TRUE KeepLink=TRUE SkipStartNote=TRUE $CALM|NoBlindRepeat"
  "recovery|broken|pending_runs row in its own commit|MarkBeforeRead=TRUE AcceptOneCommit=FALSE RecoverUntouched=TRUE RecoverClears=TRUE KeepLink=TRUE SkipStartNote=TRUE $CALM|NothingLost"
  "recovery|broken|recover reports every unfinished conversation|MarkBeforeRead=TRUE AcceptOneCommit=TRUE RecoverUntouched=FALSE RecoverClears=TRUE KeepLink=TRUE SkipStartNote=TRUE $CALM|OnlyStartedReported"
  "recovery|broken|recover keeps the rows it reported|MarkBeforeRead=TRUE AcceptOneCommit=TRUE RecoverUntouched=TRUE RecoverClears=FALSE KeepLink=TRUE SkipStartNote=TRUE $CALM|ReportedOnce"
  "recovery|broken|a message the turn acted on is handed on without its carrier|MarkBeforeRead=TRUE AcceptOneCommit=TRUE RecoverUntouched=TRUE RecoverClears=TRUE KeepLink=FALSE SkipStartNote=TRUE $STOPS|NoBlindRepeat"
  "recovery|broken|recover counts the notice that a start could not be recorded as a touch|MarkBeforeRead=TRUE AcceptOneCommit=TRUE RecoverUntouched=TRUE RecoverClears=TRUE KeepLink=TRUE SkipStartNote=FALSE OperatorStops=FALSE StartCanFail=TRUE TurnsCanFail=FALSE|OnlyStartedReported"

  "relay|holds|code, a provider that redelivers, catch-up, two host ends, two relay loops, presses and typed answers racing the lapse, a relink, failing tells, ambiguous posts|$RELAY|$RELAY_ALL"
  "relay|holds|every committed item reaches sent, failed or doubt, with one relay loop and no relink|$RELAY Loops={1} Relinks=FALSE|EventuallyTerminal"
  "relay|broken|acknowledge before the commit|$RELAY AckAfterCommit=FALSE|HeardOrToldWhy"
  "relay|broken|post again after a crash without asking find|$RELAY FindFirst=FALSE|ShownOnce"
  "relay|broken|catch-up that skips the idempotency key|$RELAY CatchUpKeyed=FALSE|HeardOnce"
  "relay|broken|place_runs written in a second commit|$RELAY PlaceRunOneCommit=FALSE|NothingDropped"
  "relay|broken|an ambiguous post queued to send again|$RELAY AmbiguousHeld=FALSE|ShownOnce"
  "relay|broken|rows picked without the commit order|$RELAY OrderedByCommit=FALSE|InOrder"
  "relay|broken|a press taken from anyone, anywhere|$RELAY PressChecked=FALSE|PressedByOwnerAtAsk"
  "relay|broken|a relink that retires the old conversation's runs|$RELAY RelinkRetires=TRUE|NoSilentDrop"
  "relay|broken|a private line that failed is acknowledged anyway|$RELAY TellFailUnheard=FALSE|HeardOrToldWhy"
  "relay|broken|two relay loops with neither the round lock nor the conditional claim|$RELAY RoundLock=FALSE ConditionalClaim=FALSE|ShownOnce"
  "relay|broken|a typed answer taken without checking the ask is still pending|$RELAY AnswerChecksPending=FALSE|DecidedOnce"
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
  local model=$1 constants=$2 invariants=$3 work=$4
  local module="${model^}"
  {
    echo "SPECIFICATION Spec"
    echo "CONSTANTS"
    echo "    MaxEnds = ${MAX_ENDS:-2}"
    local -A value=()
    local names=() pair
    for pair in $constants; do
      [[ -v "value[${pair%%=*}]" ]] || names+=("${pair%%=*}")
      value[${pair%%=*}]=${pair#*=}
    done
    for name in "${names[@]}"; do echo "    $name = ${value[$name]}"; done
    echo "INVARIANTS"
    echo "    TypeOK"
    for name in $invariants; do [[ " $PROPERTIES " == *" $name "* ]] || echo "    $name"; done
    for name in $invariants; do [[ " $PROPERTIES " != *" $name "* ]] || printf 'PROPERTIES\n    %s\n' "$name"; done
  } >"$work/$module.cfg"
  cp "formal/$model/$module.tla" "$work/"
  (cd "$work" && java -XX:+UseParallelGC -cp "$JAR" tlc2.TLC -workers auto -deadlock -lncheck final \
    -metadir "$work/states" -config "$module.cfg" "$module.tla") >"$work/tlc.out" 2>&1 || true
}

check() {
  local model=$1 expect=$2 what=$3 constants=$4 invariants=$5
  local work
  work=$(mktemp -d)
  run_tlc "$model" "$constants" "$invariants" "$work"
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
      printf 'ok    holds     %-22s %s (%s states)\n' "$invariants" "$what" "$states" | sed 's/NoBlindRepeat NothingLost OnlyStartedReported ReportedOnce StoppedStaysStopped/all five/; s/HeardOnce HeardOrToldWhy ShownOnce NothingDropped InOrder DecidedOnce PressedByOwnerAtAsk NoSilentDrop/all eight/'
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
  scope=$1
  [[ $scope == *TurnsCanFail* ]] || scope="$scope TurnsCanFail=FALSE"
  run_tlc recovery "$CODE $scope" "$2" "$work"
  cat "$work/tlc.out"
  exit 0
fi

failed=0
for row in "${CHECKS[@]}"; do
  IFS='|' read -r model expect what constants invariants <<<"$row"
  check "$model" "$expect" "$what" "$constants" "$invariants" || failed=1
done
exit $failed
