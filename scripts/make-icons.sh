#!/usr/bin/env bash
#
# Rebuilds every committed icon from the two scripts that draw them: the app
# icon, and the three menu bar glyphs. `scripts/make-icon.py` draws one 1024px
# PNG and `tauri icon` derives the platform set from it; `scripts/make-tray.py`
# draws the glyphs at 36px. `scripts/check-icons.py` runs this into a temp
# directory and fails the gate when the committed files are not what comes out,
# so this is the command to run when it does.
#
#   ./scripts/make-icons.sh              rebuild the repo's icons in place
#   ./scripts/make-icons.sh /tmp/icons   rebuild them under another root, which
#                                        is how the check compares
#
# Editing a mark is editing the species in `make-icon.py`: the tray imports it,
# so the dock and the menu bar move together.

set -euo pipefail

cd "$(dirname "$0")/.."
root="${1:-.}"

command -v python3 >/dev/null || { echo "python3 not found" >&2; exit 1; }
[ -x node_modules/.bin/tauri ] || { echo "tauri CLI not found; run pnpm install" >&2; exit 1; }

python3 scripts/make-icon.py "$root/scripts/icon-source.png"
node_modules/.bin/tauri icon "$root/scripts/icon-source.png" -o "$root/src-tauri/icons"
python3 scripts/make-tray.py "$root/src-tauri/icons"

# `tauri icon` packs icon.icns by walking a map, so its chunks land in a
# different order on every run and every rebuild would be a diff even when no
# pixel moved. A reader finds a chunk by its type rather than by where it
# stands, so sorting them costs nothing and makes the file reproducible.
python3 - "$root/src-tauri/icons/icon.icns" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
data = path.read_bytes()
chunks, at = [], 8
while at < len(data):
    chunks.append(data[at : at + int.from_bytes(data[at + 4 : at + 8], "big")])
    at += len(chunks[-1])
body = b"".join(sorted(chunks))
path.write_bytes(b"icns" + (8 + len(body)).to_bytes(4, "big") + body)
PY
