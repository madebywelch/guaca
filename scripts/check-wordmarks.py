#!/usr/bin/env python3

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
MAKE = ROOT / "design" / "wordmarks" / "make.sh"


def written(root: Path) -> set[Path]:
    marks = {p.relative_to(root) for p in (root / "design" / "wordmarks").glob("*.svg")}
    page = root / "design" / "wordmarks.html"
    return marks | ({page.relative_to(root)} if page.exists() else set())


def compare(drawn: Path) -> list[str]:
    problems = []
    drawn_files, kept_files = written(drawn), written(ROOT)
    for name in sorted(kept_files - drawn_files):
        problems.append(f"  {name}: no script draws it any more")
    for name in sorted(drawn_files - kept_files):
        problems.append(f"  {name}: missing")
    for name in sorted(drawn_files & kept_files):
        if (drawn / name).read_bytes() != (ROOT / name).read_bytes():
            problems.append(f"  {name}: differs")
    return problems


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="guaca-wordmarks-") as temp:
        drawn = Path(temp)
        run = subprocess.run([str(MAKE), str(drawn)], cwd=ROOT, capture_output=True, text=True)
        if run.returncode != 0:
            print("design/wordmarks/make.sh did not run, so nothing was compared:\n", file=sys.stderr)
            print((run.stdout + run.stderr).strip(), file=sys.stderr)
            return 2
        problems = compare(drawn)

    if problems:
        print("The committed wordmarks are not what design/wordmarks/wordmarks.py draws "
              "from the tokens in src/styles.css:\n", file=sys.stderr)
        print("\n".join(problems), file=sys.stderr)
        print("\nRun ./design/wordmarks/make.sh and commit what it writes.", file=sys.stderr)
        return 1

    print(f"wordmarks: {len(written(ROOT))} files match design/wordmarks/wordmarks.py and src/styles.css")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
