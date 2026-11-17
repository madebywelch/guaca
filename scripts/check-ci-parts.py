#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path

root = Path(__file__).resolve().parent.parent
gate = Path(sys.argv[1]) if len(sys.argv) > 1 else root / "scripts" / "ci.sh"
workflow = Path(sys.argv[2]) if len(sys.argv) > 2 else root / ".github" / "workflows" / "ci.yml"

script = gate.read_text()
bodies = {
    name: re.findall(r"^\s*(run_\w+)\s*$", body, re.M)
    for name, body in re.findall(r"^(run_\w+)\(\) \{\n(.*?)^\}", script, re.M | re.S)
}
arguments = {function: argument for argument, function in re.findall(r"^\s*([a-z]+)\) (run_\w+) ;;", script, re.M)}
everything = re.search(r"^\s*all\)\n(.*?);;", script, re.M | re.S)
if not everything:
    sys.exit(f"{gate} has no all) case")


def leaves(function: str) -> list[str]:
    calls = bodies.get(function, [])
    return [leaf for call in calls for leaf in leaves(call)] if calls else [function]


wanted = {arguments[leaf] for call in re.findall(r"run_\w+", everything.group(1)) for leaf in leaves(call)}
ran = set(re.findall(r"\./scripts/ci\.sh (\w+)", workflow.read_text()))
missing = sorted(wanted - ran)
if missing:
    sys.exit(f"{workflow} never runs ./scripts/ci.sh {' or '.join(missing)}, which ./scripts/ci.sh runs")
print(f"ci: {workflow.name} runs every part of the gate ({', '.join(sorted(wanted))})")
