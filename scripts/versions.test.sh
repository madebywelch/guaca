#!/usr/bin/env bash
#
# The version floors, tested where the installer cannot run.
#
# `scripts/install.sh` builds a .app and refuses to start on anything but macOS
# in its third check, so the arithmetic that decides whether somebody is told
# now or by a wall of errors later would otherwise never be tested anywhere. The
# checks live in `scripts/versions.sh` for exactly this reason, and this sources
# it. No harness, no dependencies: bash and an exit code, so the gate runs it on
# Linux and on a Mac alike.
#
# Deliberately not `set -e`: a failing case must be counted and reported rather
# than ending the file at the first one, which is also what makes the output
# worth reading when it does fail.

set -uo pipefail

cd "$(dirname "$0")/.."
# shellcheck source=scripts/versions.sh
. scripts/versions.sh

failures=0
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

check() {
  local what="$1" got="$2" want="$3"
  if [ "$got" = "$want" ]; then
    printf '  ok   %s\n' "$what"
  else
    printf '  FAIL %s\n       got:  %s\n       want: %s\n' "$what" "$got" "$want"
    failures=$((failures + 1))
  fi
}

say() { printf '\n\033[1m%s\033[0m\n' "$1"; }

say "at_least"
check "equal is enough"              "$(at_least 10.33.0 10.33.0 && echo yes)" "yes"
check "a patch above the floor"      "$(at_least 10.33.1 10.33.0 && echo yes)" "yes"
check "a patch below the floor"      "$(at_least 10.32.9 10.33.0 || echo no)"  "no"
check "a minor above the floor"      "$(at_least 10.34.0 10.33.0 && echo yes)" "yes"
check "a major above the floor"      "$(at_least 11.0.0 10.33.0 && echo yes)"  "yes"
check "a major below the floor"      "$(at_least 9.99.99 10.33.0 || echo no)"  "no"
check "a short version reads as .0"  "$(at_least 10.33 10.33.0 && echo yes)"   "yes"

say "pnpm_pin"
printf '{"name":"x","packageManager":"pnpm@10.33.0"}' > "$scratch/pinned.json"
printf '{"name":"x"}' > "$scratch/unpinned.json"
printf '{"name":"x","packageManager":"yarn@4.1.0"}' > "$scratch/yarn.json"
check "reads the version corepack reads" "$(pnpm_pin "$scratch/pinned.json")" "10.33.0"
check "no field is no pin"               "$(pnpm_pin "$scratch/unpinned.json")" ""
check "another manager is no pnpm pin"   "$(pnpm_pin "$scratch/yarn.json")" ""

# The repository's own pin, read the way the installer reads it. A sed that
# stopped matching would leave the check silent rather than wrong, which is the
# failure this catches.
pin="$(pnpm_pin)"
check "this repository declares a pin" "$([ -n "$pin" ] && echo pinned)" "pinned"

say "pnpm_verdict"
# Word for word, because this is a sentence somebody reads instead of a build
# failing four hundred crates in. What to run is the whole of what it is for.
check "an old pnpm is told exactly what to run" "$(pnpm_verdict 8.15.4 10.33.0)" \
  'pnpm 8.15.4 is older than the 10.33.0 this repository pins; run `corepack enable pnpm` (corepack ships with Node), or `npm install -g pnpm@10.33.0`, and run this again.'
check "the pinned version passes"   "$(pnpm_verdict 10.33.0 10.33.0)" ""
check "a newer pnpm passes"         "$(pnpm_verdict 10.34.1 10.33.0)" ""
check "no pin says nothing"         "$(pnpm_verdict 8.15.4 "")" ""
check "no version says nothing"     "$(pnpm_verdict "" 10.33.0)" ""

if [ "$failures" -ne 0 ]; then
  printf '\n\033[1m%d version check(s) failed\033[0m\n' "$failures" >&2
  exit 1
fi
printf '\nAll version checks passed\n'
