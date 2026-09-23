#!/usr/bin/env python3
"""A process-boundary Docker fixture. All state stays beside this copied script."""
import json
import pathlib
import sys

root = pathlib.Path(__file__).parent
path = root / "docker-state.json"
state = json.loads(path.read_text())
args = sys.argv[1:]
with (root / "docker-calls.jsonl").open("a") as output:
    output.write(json.dumps(args) + "\n")
mode = state.get("failure", "")
def fail():
    print("Injected Docker failure", file=sys.stderr)
    sys.exit(1)
def save():
    path.write_text(json.dumps(state))

if args[0] == "context":
    print("unix:///fixture/docker.sock")
elif args[0] == "info":
    print("linux")
elif args[:2] == ["container", "ls"]:
    if state["exists"]: print("container-id")
elif args[:2] == ["container", "inspect"]:
    if not state["exists"]: fail()
    print(json.dumps([state["container"]]))
elif args[:2] == ["image", "inspect"]:
    if mode == "download": fail()
    print(json.dumps([{"Config": {"Labels": {"org.opencontainers.image.revision": "abcdef1"}}}]))
elif args[0] == "pull":
    if mode == "download": fail()
elif args[0] == "stop":
    if mode == "stop": fail()
    state["container"]["State"]["Running"] = False
    save()
elif args[0] == "start":
    if mode == "backup-restart": fail()
    state["container"]["State"]["Running"] = True
    save()
elif args[0] == "rm":
    state["exists"] = False
    save()
elif args[0] == "run" and "--detach" not in args:
    if mode in ("backup", "backup-restart"): fail()
    state["backup"] = next(arg for arg in args if "dst=/backup" in arg)
    save()
elif args[0] == "run":
    if mode == "create": fail()
    state["exists"] = True
    state["container"]["State"]["Running"] = True
    state["container"]["Config"]["Image"] = args[-1]
    save()
elif args[0] == "exec":
    print("fixture-token")
else:
    fail()
