#!/usr/bin/env python3
import json
import pathlib
import sys
import time

root = pathlib.Path(__file__).parent
path = root / "engines-state.json"
state = json.loads(path.read_text())
args = sys.argv[1:]
endpoint = state["current"]
if args[:1] == ["--host"]:
    endpoint, args = args[1], args[2:]
with (root / "engines-calls.jsonl").open("a") as output:
    output.write(json.dumps([endpoint] + args) + "\n")
engine = state["engines"].get(endpoint)
def save():
    path.write_text(json.dumps(state))
def refuse(said):
    print(said, file=sys.stderr)
    sys.exit(1)

if args[:2] == ["context", "ls"]:
    for context in state["contexts"]:
        print(json.dumps({"Name": context[0], "DockerEndpoint": context[1]}))
    sys.exit(0)
if args[:2] == ["context", "inspect"]:
    print(state["current"])
    sys.exit(0)
if args[:2] == ["context", "show"]:
    print(next(c[0] for c in state["contexts"] if c[1] == state["current"]))
    sys.exit(0)
if engine is None or not engine["up"]:
    refuse(f"Cannot connect to the Docker daemon at {endpoint}. Is the docker daemon running?")
containers = engine.setdefault("containers", {})
images = engine.setdefault("images", [])
def image(ref):
    return next((i for i in images if i["id"] == ref or ref in i["refs"]), None)

if args[0] == "info":
    print(f"{engine['id']} linux" if "{{.ID}}" in args[-1] else "linux")
elif args[:2] == ["container", "ls"]:
    name = args[args.index("--filter") + 1].removeprefix("name=^/").removesuffix("$")
    if name in containers:
        print(f"id-{name}")
elif args[:2] == ["container", "inspect"]:
    print(json.dumps([containers[args[2]]]))
elif args[0] == "start":
    containers[args[1]]["State"]["Running"] = True
    save()
elif args[:2] == ["image", "inspect"]:
    found = [image(ref) for ref in args[2:]]
    if None in found:
        refuse(f"Error: No such image: {args[2]}")
    print(json.dumps([{"Id": i["id"], "Config": {"Labels": i["labels"]}} for i in found]))
elif args[:2] == ["image", "ls"]:
    for i in images:
        if "org.opencontainers.image.revision" in i["labels"]:
            print(i["id"])
elif args[0] == "tag":
    image(args[1])["refs"].append(args[2])
    save()
elif args[0] == "pull":
    for _ in range(1000):
        if not state.get("holdPull") or (root / "pull-gate").exists():
            break
        time.sleep(0.02)
    if args[1] not in state.get("registry", {}):
        refuse(f"Error response from daemon: pull access denied for {args[1]}, repository does not exist or may require 'docker login': denied")
    images.append({"id": state["registry"][args[1]], "refs": [args[1]], "labels": {}})
    save()
elif args[:2] == ["volume", "ls"]:
    for name in engine.setdefault("volumes", []):
        print(name)
elif args[0] == "run" and "--detach" in args:
    name = args[args.index("--name") + 1]
    volume = next(a for a in args if a.startswith("type=volume,src=")).split(",")[1].removeprefix("src=")
    if volume not in engine.setdefault("volumes", []):
        engine["volumes"].append(volume)
    containers[name] = {"Config": {"Image": args[-1], "Labels": {state["owner"]: name}}, "State": {"Running": True}, "NetworkSettings": {"Ports": {"8787/tcp": [{"HostIp": "127.0.0.1", "HostPort": str(state["port"])}]}}}
    save()
elif args[0] == "exec":
    print("fixture-token")
else:
    refuse("Injected Docker failure")
