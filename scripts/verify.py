#!/usr/bin/env python3
from __future__ import annotations

USAGE = """Verify Guaca end to end, the way a person uses it.

    scripts/verify.py                     every scenario, once each
    scripts/verify.py reply peers         some of them
    scripts/verify.py --runs 50 reply     the same scenario fifty times in a row
    scripts/verify.py --list              what there is, and what each proves
    scripts/verify.py doctor              is this checkout ready to verify?
    scripts/verify.py build               the web bundle and both Hosts
    scripts/verify.py launch              a scratch Host and stand-in model to drive by hand
    scripts/verify.py mac [probe args]    the Mac-only half: scripts/probe.py, on a Mac

Each run starts the daemon (guacad) from an empty data directory on a free
loopback port, serving the web bundle a browser loads, with an OpenAI-compatible
model on loopback that answers from the scenario's script. Headless Chromium
opens the invitation link and the scenario works the page by accessible role
and name. Nothing leaves the machine: the Host and the browser are pointed at
a proxy that refuses every connection and writes down where it was going.

Every run writes its evidence to src-tauri/target/verify/evidence/<scenario>/run-<n>/:
steps.json (each step, whether it held, what was seen), a screenshot and the
accessibility tree for each step that drew something, model.jsonl and model/
(every call to the model and what it was answered), events.jsonl (every event
the page was sent), egress.jsonl, host.log and chromium.log. Teardown removes
the scratch directory and never the evidence.

stdout is one JSON document; the exit status is 1 if any run failed, 2 if the
checkout is not ready to verify. .claude/skills/verify/SKILL.md is the guide.
"""

import argparse
import json
import shutil
import signal
import subprocess
import sys
import time
import traceback
from pathlib import Path

from e2e.browser import BrowserError, find_chromium
from e2e.scenarios import SCENARIOS, unproved
from e2e.world import (EVIDENCE, GUACAD, GUACAD_CLOCK, ROOT, SCRATCH, STAND_IN_KEY, STAND_IN_MODEL,
                       WEB, Failed, World, say)

MANIFEST = ROOT / "src-tauri/Cargo.toml"


def build() -> None:
    print("==> pnpm build", file=sys.stderr)
    subprocess.run(["pnpm", "build"], cwd=ROOT, check=True)
    GUACAD.parent.mkdir(parents=True, exist_ok=True)
    for features, binary in (("server", GUACAD), ("server,probe-host", GUACAD_CLOCK)):
        step = ["cargo", "build", "--manifest-path", str(MANIFEST), "--no-default-features",
                "--features", features, "--bin", "guacad"]
        print(f"==> {' '.join(step)}", file=sys.stderr)
        subprocess.run(step, cwd=ROOT, check=True)
        shutil.copy2(ROOT / "src-tauri/target/debug/guacad", binary)


def doctor() -> dict:
    def built(path: Path) -> dict:
        if not path.exists():
            return {"ok": False, "path": str(path.relative_to(ROOT)), "fix": "scripts/verify.py build"}
        return {"ok": True, "path": str(path.relative_to(ROOT)),
                "built": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(path.stat().st_mtime))}

    head = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True)
    dirty = subprocess.run(["git", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT,
                           capture_output=True, text=True)
    web = built(WEB / "index.html")
    if web["ok"] and any("__guacaProbe" in p.read_text(errors="ignore") for p in (WEB / "assets").glob("*.js")):
        web = {**web, "ok": False, "fix": "dist/ is a probe-mode bundle; run pnpm build"}
    chromium = find_chromium()
    checks = {
        "commit": head.stdout.strip() + ("-dirty" if dirty.stdout.strip() else ""),
        "web": web,
        "guacad": built(GUACAD),
        "guacadClock": built(GUACAD_CLOCK),
        "chromium": {"ok": bool(chromium), "path": chromium,
                     **({} if chromium else {"fix": "install Chromium or set GUACA_VERIFY_CHROMIUM"})},
        "python": {"ok": sys.version_info >= (3, 10), "version": sys.version.split()[0]},
    }
    newest = max((p.stat().st_mtime for p in (ROOT / "src-tauri/src").rglob("*.rs")), default=0)
    for key in ("guacad", "guacadClock"):
        path = GUACAD if key == "guacad" else GUACAD_CLOCK
        if checks[key]["ok"] and path.stat().st_mtime < newest:
            checks[key] = {**checks[key], "ok": False, "fix": "older than src-tauri/src; scripts/verify.py build"}
    checks["ok"] = all(v["ok"] for v in checks.values() if isinstance(v, dict))
    return checks


def attempt(name: str, run: int, evidence: Path, keep: bool) -> dict:
    world = World(name, run, evidence, SCENARIOS[name].minute_ms)
    started = time.monotonic()
    error = None
    try:
        SCENARIOS[name].run(world)
        if not world.steps:
            world.check("the scenario recorded a step", False, stop=False)
        missing = unproved(SCENARIOS[name].claims, world.steps)
        if missing:
            world.check("the scenario proved every claim it makes", False, observed=missing, stop=False)
        if world.page_errors:
            world.check("the page threw nothing", False, observed=world.page_errors, stop=False)
    except Failed as why:
        if not world.steps or world.steps[-1]["pass"]:
            world.steps.append({"step": str(why), "pass": False})
            world.capture(f"failed {why}") if world.browser else None
    except BrowserError as why:
        world.steps.append({"step": f"the page did not allow it: {why}", "pass": False})
        world.capture("failed") if world.browser else None
    except Exception as why:
        error = traceback.format_exc()
        world.steps.append({"step": f"the harness failed: {why!r}", "pass": False})
    finally:
        if keep:
            print(f"keeping the world for {name} run {run}: {world.host.invitation}; press Ctrl-C to end",
                  file=sys.stderr)
            try:
                signal.pause()
            except KeyboardInterrupt:
                pass
        world.close()
    passed = bool(world.steps) and all(step["pass"] for step in world.steps)
    result = {"scenario": name, "run": run, "pass": passed, "seconds": round(time.monotonic() - started, 1),
              "evidence": str(world.evidence.relative_to(ROOT) if world.evidence.is_relative_to(ROOT) else world.evidence), "steps": world.steps,
              "modelCalls": len(world.model.calls), "egress": world.egress.attempts}
    if error:
        (world.evidence / "harness-error.txt").write_text(error)
    (world.evidence / "steps.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def launch() -> int:
    world = World("launch", 1, EVIDENCE)
    world.model.script = lambda c: say("pong" if c.last_user == "ping" else "A reply from the stand-in model.")
    world.start(browser=False)
    print(json.dumps({"open": world.host.invitation, "endpoint": world.model.base, "apiKey": STAND_IN_KEY,
                      "model": STAND_IN_MODEL, "evidence": str(world.evidence)}, indent=2))
    print("Connect AI: API key, Custom endpoint, with the endpoint, model and key above. Ctrl-C ends it.",
          file=sys.stderr)
    try:
        signal.pause()
    except KeyboardInterrupt:
        pass
    world.close()
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=USAGE, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("scenarios", nargs="*", help="which to run, or doctor, build, launch, mac")
    parser.add_argument("--runs", type=int, default=1, help="times in a row to run each (default 1)")
    parser.add_argument("--list", action="store_true", help="list the scenarios and exit")
    parser.add_argument("--keep", action="store_true", help="leave each world running until Ctrl-C")
    parser.add_argument("--evidence", type=Path, default=EVIDENCE, help="where evidence goes")
    parser.add_argument("--build", action="store_true", help="build before running")
    args, rest = parser.parse_known_args()

    if args.scenarios[:1] == ["mac"]:
        probe = ROOT / "scripts/probe.py"
        return subprocess.call([sys.executable, str(probe), *args.scenarios[1:], *rest])
    if rest:
        parser.error(f"unrecognized arguments: {' '.join(rest)}")
    if args.list:
        for name, scenario in SCENARIOS.items():
            clock = f" (probe-host Host, {scenario.minute_ms} ms minute)" if scenario.minute_ms else ""
            print(f"{name:12} {scenario.proves}{clock}")
            for claim in scenario.claims:
                print(f"{'':12}   {claim}")
        return 0
    if args.scenarios == ["doctor"]:
        checks = doctor()
        print(json.dumps(checks, indent=2))
        return 0 if checks["ok"] else 2
    if args.scenarios == ["build"]:
        build()
        return 0
    if args.scenarios == ["launch"]:
        return launch()

    unknown = [name for name in args.scenarios if name not in SCENARIOS]
    if unknown:
        print(f"no such scenario: {', '.join(unknown)} (see --list)", file=sys.stderr)
        return 2
    if args.build:
        build()
    checks = doctor()
    names = args.scenarios or list(SCENARIOS)
    needed = ["web", "guacad", "chromium"] + (["guacadClock"] if any(SCENARIOS[n].minute_ms for n in names) else [])
    missing = {key: checks[key] for key in needed if not checks[key]["ok"]}
    if missing:
        print(json.dumps({"ready": False, "missing": missing}, indent=2))
        return 2
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    SCRATCH.mkdir(parents=True, exist_ok=True)
    results = []
    for name in names:
        for run in range(1, args.runs + 1):
            result = attempt(name, run, args.evidence, args.keep)
            results.append(result)
            print(f"{name} run {run}: {'pass' if result['pass'] else 'FAIL'} ({result['seconds']}s)",
                  file=sys.stderr)
            if not result["pass"]:
                failed = next(s for s in result["steps"] if not s["pass"])
                print(f"  {failed['step']}\n  evidence: {result['evidence']}", file=sys.stderr)

    by_scenario: dict[str, list] = {}
    for result in results:
        by_scenario.setdefault(result["scenario"], []).append(
            [(s["step"], s["pass"], json.dumps(s.get("observed"), sort_keys=True)) for s in result["steps"]])
    summary = {
        "pass": all(r["pass"] for r in results),
        "commit": checks["commit"],
        "counts": {name: {"runs": len(runs), "passed": sum(1 for r in results if r["scenario"] == name and r["pass"])}
                   for name, runs in by_scenario.items()},
        "identical": {name: all(r == runs[0] for r in runs) for name, runs in by_scenario.items()},
        "runs": results,
    }
    print(json.dumps(summary, indent=2))
    return 0 if summary["pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
