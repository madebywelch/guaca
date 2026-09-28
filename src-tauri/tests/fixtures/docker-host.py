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
def main(name):
    """Whether `name` is the one container this fixture models in full."""
    return "name" not in state or name == state["name"]
def mount(destination):
    """The volume a --mount argument puts at `destination`."""
    spec = next(arg for arg in args if f"dst={destination}" in arg)
    return dict(part.split("=", 1) for part in spec.split(",") if "=" in part)["src"]

if args[0] == "context":
    print("unix:///fixture/docker.sock")
elif args[0] == "info":
    print("linux")
elif args[:2] == ["container", "ls"]:
    name = args[args.index("--filter") + 1].removeprefix("name=^/").removesuffix("$")
    if name in state.get("others", {}): print("other-id")
    elif main(name) and state["exists"]: print("container-id")
elif args[:2] == ["container", "inspect"]:
    if args[2] in state.get("others", {}):
        print(json.dumps([state["others"][args[2]]]))
    else:
        if not state["exists"]: fail()
        print(json.dumps([state["container"]]))
elif args[:2] == ["image", "inspect"]:
    if mode == "download": fail()
    print(json.dumps([{"Config": {"Labels": {
        "org.opencontainers.image.revision": "abcdef1",
        "org.opencontainers.image.version": state.get("image_version", ""),
    }}}]))
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
    if args[-1] in state.get("others", {}): del state["others"][args[-1]]
    else: state["exists"] = False
    save()
elif args[0] == "rename":
    others = state.get("others", {})
    if args[1] not in others: fail()
    others[args[2]] = others.pop(args[1])
    save()
elif args[0] == "run" and "sh" in args:
    state["restored_from"] = mount("/backup")
    save()
elif args[0] == "run" and "--detach" not in args:
    copy = mount("/backup")
    if mode in ("backup", "backup-restart") and "-backup-" in copy: fail()
    if mode == "preserve" and "-failed-" in copy: fail()
    state.setdefault("volumes", {})[copy] = mount("/source")
    if "-backup-" in copy: state["backup"] = copy
    save()
elif args[0] == "run":
    if mode == "create-always" or (mode == "create" and args[-1] == "fixture:new"): fail()
    if args[-1] == state.get("refuse"): fail()
    name = args[args.index("--name") + 1]
    if not main(name):
        labels = dict(args[i + 1].split("=", 1) for i, a in enumerate(args) if a == "--label")
        state.setdefault("others", {})[name] = {"Config": {"Image": args[-1], "Labels": labels}}
        save()
        sys.exit(0)
    state["exists"] = True
    state["container"]["State"]["Running"] = True
    state["container"]["Config"]["Image"] = args[-1]
    state["created"] = args
    save()
elif args[:2] == ["volume", "ls"]:
    for name in state.get("volumes", {}): print(name)
elif args[:2] == ["volume", "rm"]:
    if mode == "prune": fail()
    state.get("volumes", {}).pop(args[2], None)
    save()
elif args[:2] == ["image", "ls"]:
    for image in state.get("images", []): print(f"{image['id']} {image['ref']}")
elif args[:2] == ["image", "rm"]:
    if args[2] in state.get("in_use", []): fail()
    state["images"] = [image for image in state.get("images", []) if image["id"] != args[2]]
    save()
elif args[0] == "exec":
    print("fixture-token")
else:
    fail()
