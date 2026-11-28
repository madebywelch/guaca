#!/usr/bin/env python3
from __future__ import annotations

import base64
import fcntl
import json
import os
import re
import select
import subprocess
import sys
import tempfile
import termios
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
SCRIPT = HERE / "space.sh"
FLY_TOKEN = "fly-test-token-" + "9" * 24
GOOD = "ghcr.io/madebywelch/guaca/guacad@sha256:" + "a" * 64
NEWER = "ghcr.io/madebywelch/guaca/guacad@sha256:" + "b" * 64
BROKEN = "ghcr.io/madebywelch/guaca/guacad@sha256:" + "d" * 64
BUILDS = {GOOD: ("0.5.1", "1111111"), NEWER: ("0.5.2", "2222222"), BROKEN: ("0.5.2", "3333333")}
HEX64 = re.compile(r"(?<!sha256:)\b[0-9a-f]{64}\b")


class Fly:

    def __init__(self) -> None:
        self.orgs = ["personal"]
        self.apps: dict[str, dict] = {}
        self.foreign: set[str] = set()
        self.calls: list[dict] = []
        self.refuse: dict[tuple[str, str], tuple[int, str]] = {}
        self.ids = 0
        fly = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *_: object) -> None:
                pass

            def _answer(self, method: str) -> None:
                length = int(self.headers.get("content-length") or 0)
                raw = self.rfile.read(length) if length else b""
                body = json.loads(raw) if raw else None
                url = urlparse(self.path)
                fly.calls.append({"method": method, "path": url.path, "query": parse_qs(url.query), "body": body,
                                  "auth": self.headers.get("authorization", "")})
                status, answer = fly.route(method, url.path, parse_qs(url.query), body,
                                           self.headers.get("authorization", ""))
                data = json.dumps(answer).encode() if answer is not None else b""
                self.send_response(status)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self) -> None:
                self._answer("GET")

            def do_POST(self) -> None:
                self._answer("POST")

            def do_DELETE(self) -> None:
                self._answer("DELETE")

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.server.handle_error = lambda *_: None
        self.base = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def next_id(self, prefix: str) -> str:
        self.ids += 1
        return f"{prefix}{self.ids:04d}"

    def route(self, method: str, path: str, query: dict, body: dict | None, auth: str) -> tuple[int, object]:
        if path.startswith("/space/"):
            return self.health(path.split("/")[2], path)
        if auth != f"Bearer {FLY_TOKEN}":
            return 401, {"error": "token invalid"}
        if (method, path) in self.refuse:
            status, said = self.refuse[(method, path)]
            return status, {"error": said}
        parts = path.strip("/").split("/")
        if path == "/v1/tokens/current":
            return 200, {"tokens": [{"org_slug": org, "token_id": f"t{n}"} for n, org in enumerate(self.orgs)]}
        if path == "/graphql":
            return 200, {"data": {"organizations": {"nodes": [{"slug": org} for org in self.orgs]}}}
        if path == "/v1/apps" and method == "GET":
            org = query.get("org_slug", [""])[0]
            return 200, {"apps": [{"name": n, "network": a["network"]} for n, a in self.apps.items() if a["org"] == org]}
        if len(parts) > 2 and parts[1] == "apps" and parts[2] in self.foreign:
            return 403, {"error": "unauthorized"}
        if path == "/v1/apps" and method == "POST":
            if body["name"] in self.apps or body["name"] in self.foreign:
                return 422, {"error": "Name has already been taken"}
            self.apps[body["name"]] = {"org": body["org_slug"], "network": body.get("network", ""), "ips": [],
                                       "secrets": {}, "secret_writes": 0, "volumes": {}, "machines": {}}
            return 201, {"id": "app1", "created_at": 0}
        app = self.apps.get(parts[2]) if len(parts) > 2 else None
        if app is None:
            return 404, {"error": "app not found"}
        name = parts[2]
        rest = parts[3:]
        if not rest:
            if method == "DELETE":
                del self.apps[name]
                return 202, None
            return 200, {"name": name, "organization": {"slug": app["org"]}, "status": "deployed"}
        if rest == ["ip_assignments"]:
            if method == "POST":
                ip = {"ip": "66.241.125.1" if body["type"] == "shared_v4" else "2a09:8280:1::1",
                      "shared": body["type"] == "shared_v4", "egress": False}
                app["ips"].append(ip)
                return 200, ip
            return 200, {"ips": app["ips"]}
        if rest == ["secrets"]:
            if method == "POST":
                app["secrets"].update(body["values"])
                app["secret_writes"] += 1
                return 200, {"version": app["secret_writes"], "secrets": [{"name": k} for k in body["values"]]}
            return 200, {"secrets": [{"name": k, "digest": "x"} for k in app["secrets"]]}
        if rest[0] == "volumes":
            if len(rest) == 1 and method == "POST":
                vid = self.next_id("vol_")
                app["volumes"][vid] = {"id": vid, "name": body["name"], "region": body["region"],
                                       "size_gb": body["size_gb"], "state": "created", "type": "local",
                                       "from": body.get("snapshot_id"), "snapshots": [],
                                       "blocks": 2560000, "blocks_free": 2500000, "block_size": 4096}
                return 200, self.volume(app, vid)
            if len(rest) == 1:
                return 200, [self.volume(app, v) for v in app["volumes"]]
            volume = app["volumes"].get(rest[1])
            if volume is None:
                return 404, {"error": "volume not found"}
            if rest[2:] == ["snapshots"]:
                if method == "POST":
                    volume["snapshots"].append({"id": self.next_id("vs_"), "status": "created",
                                                "created_at": f"2026-11-27T00:00:{self.ids:02d}Z", "size": 1})
                    return 200, None
                return 200, volume["snapshots"]
            return 200, self.volume(app, rest[1])
        if rest[0] == "machines":
            if len(rest) == 1 and method == "POST":
                mid = self.next_id("m")
                app["machines"][mid] = {"id": mid, "name": body.get("name"), "region": body.get("region"),
                                        "config": body["config"], "state": "started", "version": "v1",
                                        "min_secrets_version": body.get("min_secrets_version")}
                return 200, self.machine(app, mid)
            if len(rest) == 1:
                return 200, [self.machine(app, m) for m in app["machines"]]
            machine = app["machines"].get(rest[1])
            if machine is None:
                return 404, {"error": "machine not found"}
            action = rest[2] if len(rest) > 2 else ""
            if method == "DELETE":
                del app["machines"][rest[1]]
                return 200, {"ok": True}
            if action == "stop":
                machine["state"] = "stopped"
                return 200, {"ok": True}
            if action == "start":
                if machine["state"] == "replacing":
                    return 412, {"error": "machine is replacing"}
                machine["state"] = "started"
                return 200, {"ok": True}
            if action == "wait":
                want = query.get("state", ["started"])[0]
                return (200, {"ok": True}) if machine["state"] == want else (408, {"error": "timeout"})
            if action == "exec":
                script = body["command"][-1]
                if "curl" in script:
                    return 200, {"stdout": "200", "exit_code": 0}
                return 200, {"stdout": app["secrets"].get("GUACA_TOKEN", "") + "\n", "exit_code": 0}
            if method == "POST" and not action:
                if body.get("current_version") not in (None, machine["version"]):
                    return 412, {"error": "machine version changed"}
                machine["config"] = body["config"]
                if machine["state"] == "stopped":
                    machine["state"] = "replacing"
                machine["version"] = "v" + str(int(machine["version"][1:]) + 1)
                return 200, self.machine(app, rest[1])
            answer = self.machine(app, rest[1])
            if machine["state"] == "replacing":
                machine["state"] = "stopped"
            return 200, answer
        return 404, {"error": f"no route {method} {path}"}

    def volume(self, app: dict, vid: str) -> dict:
        volume = {k: v for k, v in app["volumes"][vid].items() if k not in ("snapshots", "from")}
        volume["attached_machine_id"] = next((m["id"] for m in app["machines"].values()
                                              if m["config"]["mounts"][0]["volume"] == vid), None)
        return volume

    def machine(self, app: dict, mid: str) -> dict:
        machine = app["machines"][mid]
        return {"id": mid, "state": machine["state"], "region": machine["region"], "config": machine["config"],
                "version": machine["version"], "image_ref": {"digest": machine["config"]["image"].split("@")[1]},
                "checks": [{"name": "servicecheck-00-http-8787", "status": "passing"}]}

    def health(self, name: str, path: str) -> tuple[int, object]:
        app = self.apps.get(name)
        running = [m for m in (app or {}).get("machines", {}).values() if m["state"] == "started"]
        if not running or running[0]["config"]["image"] == BROKEN:
            return 503, {"error": "no machine answers"}
        if path.endswith("/v1/call"):
            return 401, {"err": {"kind": "unauthorized"}}
        version, build = BUILDS[running[0]["config"]["image"]]
        return 200, {"status": "ok", "service": "guacad", "version": version, "build": build}

    def app_calls(self, method: str, suffix: str) -> list[dict]:
        return [c for c in self.calls if c["method"] == method and c["path"].endswith(suffix)]


def signed_feed(scratch: Path, image: str, version: str, commit: str, sign: bool = True) -> tuple[Path, Path]:
    key = scratch / "main.key"
    if not key.exists():
        subprocess.run(["openssl", "genpkey", "-algorithm", "ed25519", "-out", str(key)], check=True)
        der = subprocess.run(["openssl", "pkey", "-in", str(key), "-pubout", "-outform", "DER"],
                             check=True, capture_output=True).stdout
        (scratch / "main-keys.pub").write_text(base64.b64encode(der[-32:]).decode() + "\n")
    manifest = json.dumps({"schema": 1, "channel": "main", "version": version, "commit": commit + "0" * 33,
                           "image": image, "apiGeneration": 1}).encode()
    (scratch / "manifest.json").write_bytes(manifest)
    signature = subprocess.run(["openssl", "pkeyutl", "-sign", "-inkey", str(key), "-rawin", "-in",
                                str(scratch / "manifest.json")], check=True, capture_output=True).stdout
    if not sign:
        signature = bytes(64)
    feed = scratch / f"feed-{commit}.json"
    feed.write_text(json.dumps({"manifest": base64.b64encode(manifest).decode(),
                                "signature": base64.b64encode(signature).decode()}))
    return feed, scratch / "main-keys.pub"


class Run:

    def __init__(self, fly: Fly, scratch: Path) -> None:
        self.fly = fly
        self.scratch = scratch
        self.feed, self.keys = signed_feed(scratch, GOOD, "0.5.1", "1111111")

    def env(self, name: str, **over: str) -> dict:
        env = {k: v for k, v in os.environ.items() if not k.startswith(("FLY_", "GUACA_"))}
        env.update({
            "FLY_API_TOKEN": FLY_TOKEN,
            "FLY_API_HOSTNAME": self.fly.base,
            "FLY_GRAPHQL_URL": self.fly.base + "/graphql",
            "GUACA_SPACE_ORIGIN": f"{self.fly.base}/space/{name}",
            "GUACA_MAIN_FEED": self.feed.as_uri(),
            "GUACA_MAIN_KEYS": str(self.keys),
            "GUACA_SPACE_WAIT": "4",
        })
        env.update(over)
        return {k: v for k, v in env.items() if v != ""}

    def __call__(self, *args: str, name: str = "test-space", **over: str) -> subprocess.CompletedProcess:
        return subprocess.run(["bash", str(SCRIPT), *args], env=self.env(name, **over), capture_output=True,
                              text=True, timeout=120, stdin=subprocess.DEVNULL, start_new_session=True)

    def on_terminal(self, *args: str, name: str = "test-space", **over: str) -> tuple[int, str, str]:
        leader, follower = os.openpty()

        def own_terminal() -> None:
            os.setsid()
            fcntl.ioctl(follower, termios.TIOCSCTTY, 0)

        proc = subprocess.Popen(["bash", str(SCRIPT), *args], env=self.env(name, **over), stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                                preexec_fn=own_terminal, pass_fds=(follower,))
        os.close(follower)
        seen = b""
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            ready, _, _ = select.select([leader], [], [], 0.2)
            if ready:
                try:
                    chunk = os.read(leader, 4096)
                except OSError:
                    break
                if not chunk:
                    break
                seen += chunk
            elif proc.poll() is not None:
                break
        out, err = proc.communicate(timeout=30)
        os.close(leader)
        return proc.returncode, out + err, seen.decode(errors="replace")


FAILED: list[str] = []


def check(what: str, holds: bool, detail: object = "") -> None:
    print(f"{'PASS' if holds else 'FAIL'}  {what}" + (f"  ({detail})" if detail and not holds else ""), flush=True)
    if not holds:
        FAILED.append(what)


def dry_body(text: str, call: str) -> dict:
    lines = text.splitlines()
    start = lines.index(call) + 2
    end = next((n for n in range(start, len(lines)) if not lines[n].startswith("  ")), len(lines))
    return json.loads("\n".join(line[2:] for line in lines[start:end]))


def dry_runs(run: Run) -> None:
    closed = "http://127.0.0.1:9"
    for command in (["create", "dry-space"], ["status", "dry-space"], ["update", "dry-space"],
                    ["invite", "dry-space"], ["destroy", "dry-space", "--confirm", "dry-space"]):
        done = run(*command, "--dry-run", name="dry-space", FLY_API_TOKEN="", FLY_API_HOSTNAME=closed)
        text = done.stdout + done.stderr
        check(f"{command[0]} --dry-run needs no credential and reaches no Fly", done.returncode == 0,
              text[-400:])
        check(f"{command[0]} --dry-run names its Fly calls", f"{closed}/v1/apps/dry-space" in text, text[:300])
        check(f"{command[0]} --dry-run shows no token or secret", not HEX64.search(text) and "****" in text,
              HEX64.findall(text))
    create = run("create", "dry-space", "--dry-run", name="dry-space", FLY_API_TOKEN="", FLY_API_HOSTNAME=closed)
    text = create.stdout
    body = dry_body(text, f"POST {closed}/v1/apps/dry-space/machines")
    config = body["config"]
    check("create pins the image by digest, from the signed feed", config["image"] == GOOD, config["image"])
    check("create asks for its own private network",
          '"network": "dry-space-net"' in text, text[:600])
    check("create makes one 10 GB volume", '"size_gb": 10' in text)
    service = config["services"][0]
    check("the machine is always on, behind HTTPS, with a health check on /health",
          service["autostop"] == "off" and service["min_machines_running"] == 1
          and {"port": 443, "handlers": ["tls", "http"]} in service["ports"]
          and service["checks"][0]["path"] == "/health" and service["internal_port"] == 8787, service)
    check("the machine has the measured size and keeps the zone and the space marker",
          config["guest"]["memory_mb"] == 1024 and config["env"]["TZ"] == "America/New_York"
          and config["env"]["GUACA_SPACE"] == "1" and "GUACA_TOKEN" not in config["env"], config["env"])
    zoned = run("create", "dry-space", "--dry-run", "--tz", "Europe/Paris", "--region", "cdg", name="dry-space",
                FLY_API_TOKEN="", FLY_API_HOSTNAME=closed)
    check("--tz and --region reach the machine", '"TZ": "Europe/Paris"' in zoned.stdout
          and '"region": "cdg"' in zoned.stdout, zoned.stdout[-300:])
    unsigned, _ = signed_feed(run.scratch, GOOD, "0.5.1", "4444444", sign=False)
    refused = run("create", "dry-space", "--dry-run", name="dry-space", FLY_API_TOKEN="",
                  GUACA_MAIN_FEED=unsigned.as_uri())
    check("a build that is not signed with Guaca's key is refused",
          refused.returncode != 0 and "not signed" in refused.stderr, refused.stderr)
    first = run("--dry-run", "--region", "cdg", "create", "dry-space", name="dry-space", FLY_API_TOKEN="",
                FLY_API_HOSTNAME=closed)
    check("options may come before the command", first.returncode == 0 and '"region": "cdg"' in first.stdout,
          first.stdout[-300:] + first.stderr)
    tagged = run("update", "dry-space", "ghcr.io/madebywelch/guaca/guacad:latest", "--dry-run", name="dry-space",
                 FLY_API_TOKEN="")
    check("an image named by tag is refused", tagged.returncode != 0 and "digest" in tagged.stderr, tagged.stderr)


def against_fly(run: Run) -> None:
    fly = run.fly
    missing = run("create", "test-space", FLY_API_TOKEN="")
    check("without FLY_API_TOKEN, create says how to get one",
          missing.returncode != 0 and "fly tokens create org" in missing.stderr, missing.stderr)

    fly.orgs = ["personal", "work"]
    several = run("create", "test-space")
    check("a token that reaches several organizations is refused, naming them",
          several.returncode != 0 and "personal, work" in several.stderr and "FLY_ORG" in several.stderr,
          several.stderr)
    check("and nothing was made", not fly.apps)
    chosen = run("create", "test-space", FLY_ORG="work", GUACA_SPACE_WAIT="1")
    check("FLY_ORG picks one of them", chosen.returncode == 0 and fly.apps["test-space"]["org"] == "work",
          chosen.stdout + chosen.stderr)
    del fly.apps["test-space"]
    fly.orgs = ["personal"]

    fly.calls.clear()
    first = run("create", "test-space")
    out = first.stdout + first.stderr
    app = fly.apps.get("test-space", {})
    token = app.get("secrets", {}).get("GUACA_TOKEN", "")
    check("create makes the app, a volume and one machine, and the space answers", first.returncode == 0
          and len(app.get("volumes", {})) == 1 and len(app.get("machines", {})) == 1, out)
    check("the app is on a network of its own", app.get("network") == "test-space-net", app.get("network"))
    check("the app gets a shared IPv4 and an IPv6 address", len(app.get("ips", [])) == 2, app.get("ips"))
    check("the token is strong and is a Fly secret, not machine config",
          re.fullmatch(r"[0-9a-f]{64}", token) is not None
          and token not in json.dumps([m["config"] for m in app["machines"].values()]), len(token))
    machine = next(iter(app["machines"].values()))
    check("the machine waits for the secret it was given", machine["min_secrets_version"] == 1, machine)
    check("the token is not printed where no terminal is, and create says how to see it",
          token not in out and "space.sh invite test-space" in out, out[-300:])
    check("every Fly call carried the token, and only as a header",
          all(c["auth"] == f"Bearer {FLY_TOKEN}" for c in fly.calls if not c["path"].startswith("/space/")))

    again = run("create", "test-space")
    out = again.stdout + again.stderr
    check("create again succeeds", again.returncode == 0, out)
    check("create again makes no second machine or volume",
          len(app["volumes"]) == 1 and len(app["machines"]) == 1, (app["volumes"].keys(), app["machines"].keys()))
    check("create again keeps the token", app["secrets"]["GUACA_TOKEN"] == token and app["secret_writes"] == 1)
    check("create again makes no second address", len(app["ips"]) == 2, app["ips"])
    check("create again prints no token", token not in out)

    code, said, screen = run.on_terminal("invite", "test-space")
    check("invite shows the invitation on the terminal", code == 0
          and f"/space/test-space/#token={token}" in screen, (code, said[-200:]))
    check("and nowhere else", token not in said)

    status = run("status", "test-space")
    out = status.stdout
    check("status names the machine state, image digest, health and volume fill", status.returncode == 0
          and "started" in out and "a" * 64 in out and "ok, Guaca 0.5.1 at 1111111" in out
          and "10 GB" in out and "% full" in out and "a wrong token is refused" in out
          and "own token admits a call" in out, out)
    check("status prints no token", token not in out + status.stderr)

    login = run.scratch / "flyctl-login"
    login.mkdir()
    (login / "fly").write_text(f"#!/bin/sh\n[ \"$*\" = 'auth token' ] && echo {FLY_TOKEN}\n")
    (login / "fly").chmod(0o755)
    path = f"{login}:{os.environ['PATH']}"
    code, said, screen = run.on_terminal("invite", "test-space", FLY_API_TOKEN="", PATH=path)
    check("without FLY_API_TOKEN, invite uses the flyctl login on this machine", code == 0
          and f"/space/test-space/#token={token}" in screen and token not in said, (code, said[-200:]))
    (login / "fly").write_text("#!/bin/sh\necho 'Error: no access token available' >&2; exit 1\n")
    out = run("invite", "test-space", FLY_API_TOKEN="", PATH=path)
    check("a flyctl that is not logged in is said plainly", out.returncode != 0
          and "fly auth login" in out.stderr, out.stderr)

    fly.refuse[("POST", "/v1/apps/other-space/volumes")] = (422, "insufficient capacity in iad")
    refused = run("create", "other-space")
    check("a Fly error is said in plain words, with Fly's reason",
          refused.returncode != 0 and "insufficient capacity in iad" in refused.stderr
          and "Fly refused to make the space's volume" in refused.stderr, refused.stderr)
    fly.refuse.clear()

    fly.foreign.add("their-space")
    fly.calls.clear()
    theirs = run("create", "their-space")
    check("a name another organization owns, which Fly answers 403, is said to be taken, not a token fault",
          theirs.returncode != 0 and "taken on Fly by another organization" in theirs.stderr
          and not fly.app_calls("POST", "/v1/apps"), theirs.stderr)
    status = run("status", "their-space")
    check("status of a name Fly refuses to show says Fly refused, and goes no further",
          status.returncode != 0 and "HTTP 403" in status.stderr and not fly.app_calls("GET", "/their-space/machines"),
          status.stdout + status.stderr)

    taken = run("create", "test-space", FLY_ORG="work")
    check("an app of the same name in another organization is refused",
          taken.returncode != 0 and "already exists in personal" in taken.stderr, taken.stderr)

    run.feed, _ = signed_feed(run.scratch, NEWER, "0.5.2", "2222222")
    updated = run("update", "test-space")
    out = updated.stdout + updated.stderr
    machine = next(iter(app["machines"].values()))
    snapshots = next(iter(app["volumes"].values()))["snapshots"]
    stop = fly.app_calls("POST", f"/machines/{machine['id']}/stop")
    check("update stops the machine, snapshots the volume and replaces the image",
          updated.returncode == 0 and machine["config"]["image"] == NEWER and len(snapshots) == 1 and stop, out)
    check("update keeps the machine, its volume and the rest of its config",
          len(app["machines"]) == 1 and len(app["volumes"]) == 1 and machine["config"]["env"]["GUACA_SPACE"] == "1")
    order = [c["path"].rsplit("/", 1)[-1] for c in fly.calls if c["method"] == "POST" and "/test-space/" in c["path"]]
    check("the snapshot is taken after the stop and before the new image",
          order.index("stop") < order.index("snapshots") < order.index(machine["id"]), order)
    same = run("update", "test-space")
    check("updating to what runs already changes nothing", same.returncode == 0 and "Nothing to do" in same.stdout,
          same.stdout + same.stderr)

    before = len(snapshots)
    forced = run("update", "test-space", "--force")
    check("update --force snapshots and swaps the image it already runs", forced.returncode == 0
          and len(snapshots) == before + 1 and machine["config"]["image"] == NEWER
          and "now runs" in forced.stdout, forced.stdout + forced.stderr)

    run.feed, _ = signed_feed(run.scratch, GOOD, "0.5.1", "1111111")
    older = run("update", "test-space")
    check("update refuses an older Guaca than the one running, and says why", older.returncode != 0
          and "older" in older.stderr and machine["config"]["image"] == NEWER, older.stderr)

    fly.calls.clear()
    old_volume = next(iter(app["volumes"]))
    broken = run("update", "test-space", BROKEN)
    out = broken.stdout + broken.stderr
    restored = [v for v in app["volumes"].values() if v["from"]]
    now = next(iter(app["machines"].values()))
    check("a build that does not answer is undone, and update says so", broken.returncode != 0
          and "previous version is back" in out, out)
    check("the old image runs again only on a copy of the snapshot taken before the update",
          now["config"]["image"] == NEWER and len(restored) == 1
          and now["config"]["mounts"][0]["volume"] == restored[0]["id"]
          and restored[0]["from"] == app["volumes"][old_volume]["snapshots"][-1]["id"], (now, restored))
    check("the workspace the failed build left is kept", old_volume in app["volumes"] and old_volume in out)
    status = run("status", "test-space")
    check("status names the kept volume as left by a failed update", "left by a failed update" in status.stdout,
          status.stdout)

    no = run("destroy", "test-space")
    check("destroy without --confirm deletes nothing and says how", no.returncode != 0
          and "--confirm test-space" in no.stderr and "test-space" in fly.apps, no.stderr)
    wrong = run("destroy", "test-space", "--confirm", "other")
    check("destroy with the wrong name deletes nothing", wrong.returncode != 0 and "test-space" in fly.apps)
    gone = run("destroy", "test-space", "--confirm", "test-space")
    check("destroy deletes the app and says what went with it", gone.returncode == 0
          and "test-space" not in fly.apps and "the machine" in gone.stdout and "the volume" in gone.stdout
          and old_volume in gone.stdout, gone.stdout + gone.stderr)


def help_lists_every_setting() -> None:
    text = SCRIPT.read_text()
    body = re.sub(r"cat <<'EOF'.*?\nEOF\n", "", text, flags=re.S)
    read = sorted(set(re.findall(r"\$\{?((?:FLY|GUACA)_[A-Z_]+)", body)) - {"GUACA_TOKEN"})
    shown = subprocess.run(["bash", str(SCRIPT), "--help"], capture_output=True, text=True).stdout
    missing = [name for name in read if not re.search(rf"^  {name}( |$)", shown, re.M)]
    check("--help lists every setting the script reads", not missing, missing)


def main() -> int:
    for tool in ("curl", "jq", "openssl"):
        if subprocess.run(["sh", "-c", f"command -v {tool}"], capture_output=True).returncode:
            print(f"{tool} is not installed; this test needs it", file=sys.stderr)
            return 2
    help_lists_every_setting()
    with tempfile.TemporaryDirectory() as scratch:
        run = Run(Fly(), Path(scratch))
        dry_runs(run)
        against_fly(run)
    print(f"\n{'all passed' if not FAILED else f'{len(FAILED)} failed'}")
    return 1 if FAILED else 0


if __name__ == "__main__":
    sys.exit(main())
