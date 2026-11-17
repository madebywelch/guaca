#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

help=$(sh deploy/box/install.sh --help)
read=$(sed "/<<'EOF'/,/^EOF$/d" deploy/box/install.sh | grep -oE '\$\{?GUACA_[A-Z_]+' | tr -d '${' | sort -u)

[ -n "$read" ] || { echo "found no GUACA_ setting the installer reads" >&2; exit 1; }
missing=0
for name in $read; do
  if ! printf '%s\n' "$help" | grep -qE "^  $name( |$)"; then
    echo "deploy/box/install.sh reads $name, and its --help does not list it" >&2
    missing=1
  fi
done
exit "$missing"
