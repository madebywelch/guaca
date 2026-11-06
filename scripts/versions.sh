#!/usr/bin/env bash
#
# The version floors this repository builds inside.
#
# One job: turn the version a machine has and the version this repository needs
# into the sentence somebody should read, or into nothing. It lives here rather
# than in `install.sh` because that script builds a .app and refuses to start
# anywhere but macOS, and the arithmetic is the part that has to be right before
# four hundred crates are compiled. `scripts/versions.test.sh` sources this file
# and the gate runs that on Linux; it could not source the installer.
#
# Sourced, not run. Nothing here has an entry point.

# Is $1 at least $2, for dotted version numbers? Written out because `sort -V`
# is GNU and macOS ships a BSD sort without it, and because the answer decides
# whether somebody is told now or told by a wall of errors later.
at_least() {
  local a b i
  IFS=. read -r -a a <<<"$1"
  IFS=. read -r -a b <<<"$2"
  for i in 0 1 2; do
    [ "${a[i]:-0}" -gt "${b[i]:-0}" ] && return 0
    [ "${a[i]:-0}" -lt "${b[i]:-0}" ] && return 1
  done
  return 0
}

# The pnpm this repository pins, read out of `packageManager` in package.json.
#
# That field is corepack's: a machine that has run `corepack enable pnpm` gets
# exactly this version, which is also the one the lockfile was written with.
# Empty where the field is absent or names a different manager, which is a
# repository without a pin rather than a broken one.
pnpm_pin() {
  sed -n 's/.*"packageManager": *"pnpm@\([0-9][0-9.]*\)".*/\1/p' "${1:-package.json}"
}

# What is wrong with the pnpm in hand, as the sentence to say, or nothing.
#
# The version arrives as an argument rather than being asked of `pnpm
# --version` here, which is what makes this testable: no gate has a pnpm of
# every age, and the check is the comparison and the advice rather than the
# call. A newer pnpm is fine — it reads an older lockfile — so the floor is a
# floor and not an equality.
pnpm_verdict() {
  local got="${1:-}"
  local pin="${2:-}"
  [ -n "$pin" ] || return 0
  [ -n "$got" ] || return 0
  if at_least "$got" "$pin"; then
    return 0
  fi
  printf 'pnpm %s is older than the %s this repository pins; run `corepack enable pnpm` (corepack ships with Node), or `npm install -g pnpm@%s`, and run this again.' \
    "$got" "$pin" "$pin"
}
