#!/usr/bin/env python3
"""Hold the committed icons to the scripts that draw them.

`src-tauri/icons/` is the one generated directory in the repo that nothing
checked. `scripts/make-icon.py` draws the app icon, `tauri icon` derives the
platform set from it (both inside `scripts/make-icons.sh`), and
`scripts/make-tray.py` draws the three menu bar glyphs. Edit the species and
nothing fails; the dock keeps the old drawing until somebody opens the app on a
Mac and notices. So this runs the whole pipeline into a temp directory and
compares it with what is committed.

Every file is a byte comparison, and the pipeline is reproducible: both
generators are arithmetic and PNG encoding with nothing time-, path- or
order-dependent in them, and `make-icons.sh` sorts icon.icns's chunks, which
`tauri icon` otherwise packs in a different order on every run.

  python3 scripts/check-icons.py
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
ICONS = ROOT / "src-tauri" / "icons"

# Finder and Explorer leave these behind; they are nobody's icon.
SKIP = {".DS_Store", "Thumbs.db"}


def files_under(root: Path) -> set[Path]:
    return {p.relative_to(root) for p in root.rglob("*") if p.is_file() and p.name not in SKIP}


def compare(drawn: Path, kept: Path) -> list[str]:
    """Every difference between what the scripts draw and what is committed."""
    problems = []

    def shown(name: Path) -> str:
        return str((kept / name).relative_to(ROOT))

    drawn_files, kept_files = files_under(drawn), files_under(kept)
    for name in sorted(kept_files - drawn_files):
        problems.append(f"  {shown(name)}: no script draws it any more")
    for name in sorted(drawn_files - kept_files):
        problems.append(f"  {shown(name)}: missing")
    for name in sorted(drawn_files & kept_files):
        if (drawn / name).read_bytes() != (kept / name).read_bytes():
            problems.append(f"  {shown(name)}: differs")
    return problems


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="guaca-icons-") as temp:
        drawn = Path(temp)
        run = subprocess.run(
            [str(HERE / "make-icons.sh"), str(drawn)], cwd=ROOT, capture_output=True, text=True
        )
        if run.returncode != 0:
            print("The icon scripts did not run, so nothing was compared:\n", file=sys.stderr)
            print((run.stdout + run.stderr).strip(), file=sys.stderr)
            return 2
        problems = compare(drawn / "src-tauri" / "icons", ICONS)

    if problems:
        print("The committed icons are not what scripts/make-icon.py and "
              "scripts/make-tray.py draw:\n", file=sys.stderr)
        print("\n".join(problems), file=sys.stderr)
        print("\nRun ./scripts/make-icons.sh and commit what it writes.", file=sys.stderr)
        return 1

    print(f"icons: {len(files_under(ICONS))} files match scripts/make-icon.py and scripts/make-tray.py")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
