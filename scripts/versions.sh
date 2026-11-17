#!/usr/bin/env bash

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

pnpm_pin() {
  sed -n 's/.*"packageManager": *"pnpm@\([0-9][0-9.]*\)".*/\1/p' "${1:-package.json}"
}

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
