#!/usr/bin/env python3

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
ICONS = ROOT / "src-tauri" / "icons"

SKIP = {".DS_Store", "Thumbs.db"}


def files_under(root: Path) -> set[Path]:
    return {p.relative_to(root) for p in root.rglob("*") if p.is_file() and p.name not in SKIP}


def compare(drawn: Path, kept: Path) -> list[str]:
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
