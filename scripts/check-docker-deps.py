#!/usr/bin/env python3
"""Hold each image's cached dependency step to the paths Cargo.toml points at.

The Dockerfiles copy the Rust manifests on their own and run Cargo before the
sources arrive, so a dependency change rebuilds once and a source change does
not. Cargo resolves every `path =` dependency and every `[patch]` entry on
every platform, so a path the manifest names and the stage has not copied yet
stops the image build at that step. Nothing else in the gate builds an image,
so this checks the copies instead: every Cargo command in a Dockerfile must
come after a COPY that brings in each path the manifest names.

  python3 scripts/check-docker-deps.py [Dockerfile ...]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "src-tauri" / "Cargo.toml"


def manifest_paths() -> list[str]:
    """Paths of dependencies and patches, not of the crate's own targets."""
    paths, section = set(), ""
    for line in MANIFEST.read_text().splitlines():
        header = re.match(r"\s*\[+([^\]]+)\]+", line)
        if header:
            section = header[1].strip()
            continue
        dependency = section.startswith("patch") or "dependencies" in section
        for path in re.findall(r'\bpath\s*=\s*"([^"]+)"', line):
            if dependency:
                paths.add(str(Path("src-tauri", path)))
    return sorted(paths)


def instructions(text: str) -> list[str]:
    joined = re.sub(r"\\\n", " ", text)
    return [line.strip() for line in joined.splitlines() if line.strip() and not line.lstrip().startswith("#")]


def covers(source: str, path: str) -> bool:
    source = source.rstrip("/").removeprefix("./")
    return source in ("", ".") or path == source or path.startswith(source + "/")


def check(dockerfile: Path, required: list[str]) -> list[str]:
    problems: list[str] = []
    copied: list[str] = []
    for line in instructions(dockerfile.read_text()):
        word, _, rest = line.partition(" ")
        word = word.upper()
        if word == "FROM":
            copied = []
        elif word in ("COPY", "ADD"):
            args = [a for a in rest.split() if not a.startswith("--")]
            if "--from" not in rest:
                copied.extend(args[:-1])
        elif word == "RUN" and re.search(r"\bcargo\b", rest):
            for path in required:
                if not any(covers(source, path) for source in copied):
                    problems.append(
                        f"  {dockerfile.name}: a cargo step runs before {path} is copied;"
                        f" copy it next to src-tauri/Cargo.toml"
                    )
    return sorted(set(problems))


def main(argv: list[str]) -> int:
    files = [Path(a) for a in argv] or [ROOT / "Dockerfile", ROOT / "Dockerfile.ci"]
    required = manifest_paths()
    problems = [p for f in files for p in check(f, required)]
    if problems:
        print("An image build would fail to resolve the Rust dependencies:", file=sys.stderr)
        print("\n".join(problems), file=sys.stderr)
        return 1
    print(f"Every cargo step in {len(files)} Dockerfiles has {', '.join(required) or 'no path dependencies'}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
