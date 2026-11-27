#!/usr/bin/env python3
from __future__ import annotations

USAGE = """Run a guacad image the way deploy/space/space.sh configures a Fly Machine, and prove it works as a space.

    IMAGE=guacad:abc1234 deploy/space/prove.py

The machine config is read from `space.sh create --dry-run`, not restated: the
same environment, entry point and user, memory and swap, volume path and port,
and no Docker socket. The volume starts empty and owned by root, as a fresh Fly
volume does. A Caddy container in front terminates TLS for space-proof.fly.dev
and forwards the way Fly's proxy does, and headless Chromium resolves that name
to it. The model is scripted on this machine and reached through a relay container
on the space's network, so nothing is spent and no firewall rule is needed.

It proves, in order: /health within HEALTHY_WITHIN seconds; a wrong token is
refused at the front door; the invitation opens the main window over HTTPS with
a Secure session cookie and a live event socket; the Mac app's calls and event
socket from tauri://localhost are admitted with the token and refused without
it; a first task finishes and is
listed under For you; the token is never in the log; the Host panel says the
space is updated for you and offers no update; there is no built-in browser,
and an agent that tries one is told so and its turn ends; the token can be read
and checked from inside the machine the way space.sh does it; and after the
container is replaced on the same volume, as update does, the work is still
there. Evidence goes to src-tauri/target/verify/evidence/space/.
"""

import json
import os
import secrets
import shlex
import socket
import ssl
import subprocess
import sys
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(REPO / "scripts"))

from e2e.browser import Browser
from e2e.scenarios import FIRST_ROW_SHOWN, conversation, menu_section, scenario_firstplan, settled
from e2e.world import EVIDENCE, Failed, World, call, redact, say, until

NAME = "space-proof"
HOSTNAME = f"{NAME}.fly.dev"
ORIGIN = f"https://{HOSTNAME}"
STAND_IN = "ghcr.io/madebywelch/guaca/guacad@sha256:" + "0" * 64
HEALTHY_WITHIN = 5
CADDY = os.environ.get("CADDY_IMAGE", "caddy:2.10")
RUN = f"guaca-space-proof-{os.getpid()}"
SCRIPTED_PAGE = "https://example.com/"


def docker(*args: str, check: bool = True, env: dict | None = None, stdin: str | None = None) -> str:
    done = subprocess.run(["docker", *args], capture_output=True, text=True, env=env, input=stdin)
    if check and done.returncode != 0:
        raise Failed(f"docker {' '.join(args[:2])}: {done.stderr.strip()[:400]}")
    return done.stdout.strip()


def machine_config() -> dict:
    done = subprocess.run(["bash", str(HERE / "space.sh"), "create", NAME, STAND_IN, "--dry-run"],
                          capture_output=True, text=True,
                          env={**{k: v for k, v in os.environ.items() if not k.startswith(("FLY_", "GUACA_"))},
                               "FLY_API_HOSTNAME": "http://127.0.0.1:9"})
    if done.returncode:
        raise Failed(f"space.sh create --dry-run failed: {done.stderr.strip()}")
    lines = done.stdout.splitlines()
    start = lines.index(f"POST http://127.0.0.1:9/v1/apps/{NAME}/machines") + 2
    end = next(n for n in range(start, len(lines)) if not lines[n].startswith("  "))
    return json.loads("\n".join(line[2:] for line in lines[start:end]))["config"]


DOOR = """
import queue, socket, threading
waiting = queue.Queue()
def listen(port):
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("0.0.0.0", port)); s.listen(64); return s
def copy(a, b):
    try:
        while True:
            chunk = a.recv(65536)
            if not chunk: break
            b.sendall(chunk)
    except OSError: pass
    for end in (a, b):
        try: end.shutdown(socket.SHUT_RDWR)
        except OSError: pass
def pair(guest):
    host = waiting.get()
    threading.Thread(target=copy, args=(guest, host), daemon=True).start(); copy(host, guest)
def hosts():
    s = listen(9000)
    while True: waiting.put(s.accept()[0])
threading.Thread(target=hosts, daemon=True).start()
s = listen(8000)
while True: threading.Thread(target=pair, args=(s.accept()[0],), daemon=True).start()
"""


class ModelDoor:

    def __init__(self, model_base: str, network: str) -> None:
        self.model = int(model_base.rsplit(":", 1)[1].split("/")[0])
        self.network = network
        self.container = f"{RUN}-model"
        self.port = 0

    def open(self, image: str) -> str:
        docker("run", "-d", "--name", self.container, "--network", self.network, "--network-alias", "model-door",
               "-p", "127.0.0.1::9000", "--entrypoint", "python3", image, "-c", DOOR)
        self.port = int(docker("port", self.container, "9000/tcp").splitlines()[0].rsplit(":", 1)[1])
        for _ in range(8):
            self._wait_one()
        return "http://model-door:8000/v1"

    def _wait_one(self) -> None:
        threading.Thread(target=self._serve_one, daemon=True).start()

    def _serve_one(self) -> None:
        deadline = time.monotonic() + 30
        while True:
            try:
                door = socket.create_connection(("127.0.0.1", self.port), timeout=5)
                break
            except OSError:
                if time.monotonic() > deadline:
                    return
                time.sleep(0.2)
        door.settimeout(None)
        try:
            first = door.recv(65536)
        except OSError:
            return
        self._wait_one()
        if not first:
            return
        model = socket.create_connection(("127.0.0.1", self.model), timeout=10)
        model.settimeout(None)
        model.sendall(first)

        def copy(src: socket.socket, dst: socket.socket) -> None:
            try:
                while chunk := src.recv(65536):
                    dst.sendall(chunk)
            except OSError:
                pass
            for end in (src, dst):
                try:
                    end.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass

        threading.Thread(target=copy, args=(door, model), daemon=True).start()
        copy(model, door)


class SpaceHost:

    def __init__(self, image: str, config: dict, evidence: Path) -> None:
        self.image = image
        self.config = config
        self.token = secrets.token_hex(32)
        self.container = f"{RUN}-guacad"
        self.volume = f"{RUN}-data"
        self.network = f"{RUN}-net"
        self.log = evidence / "host.log"
        self.port = 0
        self.starts = 0
        self.healthy_after = 0.0
        self.origin_direct = ""

    @property
    def origin(self) -> str:
        return ORIGIN

    @property
    def invitation(self) -> str:
        return f"{ORIGIN}/#token={self.token}"

    def run_args(self) -> list[str]:
        config = self.config
        process = config["processes"][0]
        memory = config["guest"]["memory_mb"]
        swap = config.get("init", {}).get("swap_size_mb", 0)
        mount = config["mounts"][0]
        service = config["services"][0]
        args = ["run", "-d", "--name", self.container, "--network", self.network, "--network-alias", "guacad",
                "--memory", f"{memory}m", "--memory-swap", f"{memory + swap}m",
                "--user", process["user"], "--entrypoint", process["exec"][0],
                "--stop-signal", config["stop_config"]["signal"],
                "--stop-timeout", config["stop_config"]["timeout"].rstrip("s"),
                "--mount", f"type=volume,src={self.volume},dst={mount['path']},volume-nocopy",
                "-p", f"127.0.0.1::{service['internal_port']}",
                "--env", "GUACA_TOKEN"]
        for key, value in config["env"].items():
            args += ["--env", f"{key}={value}"]
        return args + [self.image, *process["exec"][1:]]

    def start(self) -> None:
        self.starts += 1
        began = time.monotonic()
        docker(*self.run_args(), env={**os.environ, "GUACA_TOKEN": self.token})
        port = self.config["services"][0]["internal_port"]
        self.port = int(docker("port", self.container, f"{port}/tcp").splitlines()[0].rsplit(":", 1)[1])
        self.origin_direct = f"http://127.0.0.1:{self.port}"

        def answers() -> bool:
            if docker("inspect", "-f", "{{.State.Running}}", self.container) != "true":
                raise Failed("the container exited: " + redact(docker("logs", self.container, check=False), [self.token])[-600:])
            done = subprocess.run(["curl", "-fsS", "--max-time", "1", f"{self.origin_direct}/health"],
                                  capture_output=True, text=True)
            return done.returncode == 0 and '"status":"ok"' in done.stdout

        until("the space answers /health", answers, 60, every=0.05)
        self.healthy_after = time.monotonic() - began

    def stop(self) -> int | None:
        if docker("ps", "-aq", "-f", f"name=^{self.container}$", check=False):
            with self.log.open("a") as log:
                log.write(f"---- container {self.starts} ----\n")
                log.write(self.logs())
            docker("stop", self.container, check=False)
            docker("rm", "-f", self.container, check=False)
        return 0

    def kill(self) -> None:
        docker("rm", "-f", self.container, check=False)

    def logs(self) -> str:
        done = subprocess.run(["docker", "logs", self.container], capture_output=True, text=True)
        return done.stdout + done.stderr

    def log_text(self) -> str:
        return self.log.read_text(errors="replace") if self.log.exists() else ""

    def call(self, name: str, **args: object) -> object:
        body = json.dumps({"name": name, "args": args})
        done = subprocess.run(["curl", "-sS", "--max-time", "30", "-X", "POST", f"{self.origin_direct}/v1/call",
                               "-H", "content-type: application/json", "-H", "@-", "--data-binary", body],
                              input=f"authorization: Bearer {self.token}\n", capture_output=True, text=True)
        reply = json.loads(done.stdout or "{}")
        if "ok" not in reply:
            raise Failed(f"{name}: {redact(json.dumps(reply), [self.token])[:300]}")
        return reply["ok"]

    def post(self, path: str, body: object, headers: dict | None = None) -> tuple[int, str]:
        extra: list[str] = []
        for key, value in (headers or {}).items():
            extra += ["-H", f"{key}: {value}"]
        done = subprocess.run(["curl", "-sS", "-o", "-", "-w", "\n%{http_code}", "-X", "POST",
                               f"{self.origin_direct}{path}", "-H", "content-type: application/json", *extra,
                               "--data-binary", json.dumps(body)], capture_output=True, text=True)
        text, _, code = done.stdout.rpartition("\n")
        return int(code or 0), text

    def rows(self, sql: str, *params: object) -> list[dict]:
        query = ("import json, sqlite3, sys; q = json.load(sys.stdin); "
                 "c = sqlite3.connect('file:/var/lib/guaca/data/guac.db?mode=ro', uri=True, timeout=5); "
                 "c.row_factory = sqlite3.Row; print(json.dumps([dict(r) for r in c.execute(q['sql'], q['params'])]))")
        out = docker("exec", "-i", self.container, "python3", "-c", query,
                     stdin=json.dumps({"sql": sql, "params": list(params)}))
        return json.loads(out)

    def scalar(self, sql: str, *params: object) -> object:
        found = self.rows(sql, *params)
        return next(iter(found[0].values())) if found else None


class SpaceWorld(World):

    def __init__(self, image: str, config: dict) -> None:
        super().__init__("space", 1, EVIDENCE)
        self.host = SpaceHost(image, config, self.evidence)
        self.door = ModelDoor(self.model.base, self.host.network)
        self.front = 0

    def open_browser(self) -> Browser:
        if self.browser:
            self.browser.close()
        self.browser = Browser(self.scratch / f"browser-{self.host.starts}/profile", flags=[
            f"--host-resolver-rules=MAP {HOSTNAME}:443 127.0.0.1:{self.front}",
            "--ignore-certificate-errors",
        ])
        self.browser.listen(self._on_page)
        self.browser.open(self.host.invitation)
        until("the invitation opens the main window and the token is exchanged",
              lambda: self.browser and "#token=" not in (self.browser.evaluate("location.href") or "#token=")
              and (self.browser.find("button", "App settings")
                   or self.browser.find("region", "Get started with Guaca")
                   or self.browser.find("button", "Plan a project Turn an idea into a manageable first step.")), 60)
        return self.browser


def front_door(world: SpaceWorld, path: str, token: str, body: str | None = None) -> tuple[int, str]:
    args = ["curl", "-sk", "--max-time", "15", "--connect-to", f"{HOSTNAME}:443:127.0.0.1:{world.front}",
            "-o", "-", "-w", "\n%{http_code}", "-H", "@-"]
    if body is not None:
        args += ["-X", "POST", "-H", "content-type: application/json", "--data-binary", body]
    done = subprocess.run([*args, f"{ORIGIN}{path}"], input=f"authorization: Bearer {token}\n",
                          capture_output=True, text=True)
    text, _, code = done.stdout.rpartition("\n")
    return int(code or 0), text


def raw_front(world: SpaceWorld, request: str) -> str:
    context = ssl.create_default_context()
    context.check_hostname = False
    context.verify_mode = ssl.CERT_NONE
    with socket.create_connection(("127.0.0.1", world.front), timeout=10) as plain:
        with context.wrap_socket(plain, server_hostname=HOSTNAME) as tls:
            tls.sendall(request.encode())
            head = b""
            while b"\r\n\r\n" not in head:
                chunk = tls.recv(4096)
                if not chunk:
                    break
                head += chunk
    return head.split(b"\r\n\r\n", 1)[0].decode(errors="replace")


def mac_app(world: SpaceWorld) -> dict:
    token = world.host.token
    app = "tauri://localhost"
    preflight = raw_front(world, f"OPTIONS /v1/call HTTP/1.1\r\nHost: {HOSTNAME}\r\nOrigin: {app}\r\n"
                                 "Access-Control-Request-Method: POST\r\n"
                                 "Access-Control-Request-Headers: authorization, content-type\r\n"
                                 "Connection: close\r\n\r\n")
    body = json.dumps({"name": "capabilities"})
    called = raw_front(world, f"POST /v1/call HTTP/1.1\r\nHost: {HOSTNAME}\r\nOrigin: {app}\r\n"
                              f"Authorization: Bearer {token}\r\nContent-Type: application/json\r\n"
                              f"Content-Length: {len(body)}\r\nConnection: close\r\n\r\n{body}")
    socket_head = raw_front(world, f"GET /v1/events?token={token} HTTP/1.1\r\nHost: {HOSTNAME}\r\n"
                                   f"Origin: {app}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                                   "Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n")
    wrong = raw_front(world, f"GET /v1/events?token=not-{secrets.token_hex(8)} HTTP/1.1\r\nHost: {HOSTNAME}\r\n"
                             f"Origin: {app}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                             "Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n")
    status = lambda head: head.split("\r\n", 1)[0]
    return {
        "preflight": status(preflight),
        "allowsApp": f"access-control-allow-origin: {app}" in preflight.lower(),
        "call": status(called),
        "socket": status(socket_head),
        "wrongTokenSocket": status(wrong),
    }


def prove(world: SpaceWorld, config: dict) -> None:
    host = world.host
    w = world
    docker("network", "create", host.network)
    docker("volume", "create", host.volume)
    owner = docker("run", "--rm", "--user", "0", "--entrypoint", "stat",
                   "--mount", f"type=volume,src={host.volume},dst=/v,volume-nocopy", host.image, "-c", "%U", "/v")
    w.check("the volume starts owned by root, as a fresh Fly volume does", owner == "root", owner)
    docker("run", "-d", "--name", f"{RUN}-front", "--network", host.network, "-p", "127.0.0.1::443", CADDY,
           "caddy", "reverse-proxy", "--from", ORIGIN, "--to", "guacad:8787", "--internal-certs")
    world.front = int(docker("port", f"{RUN}-front", "443/tcp").splitlines()[0].rsplit(":", 1)[1])
    w.model.base = world.door.open(host.image)

    scenario_firstplan(w)

    w.check(f"the space answered /health within {HEALTHY_WITHIN}s of starting", host.healthy_after <= HEALTHY_WITHIN,
            observed=round(host.healthy_after, 2))
    user = docker("exec", host.container, "sh", "-c",
                  "for p in /proc/[0-9]*; do [ \"$(cat $p/comm 2>/dev/null)\" = guacad ] && stat -c %U $p && break; done")
    data_owner = docker("exec", host.container, "stat", "-c", "%U", config["mounts"][0]["path"])
    w.check("guacad runs as guaca, and owns the volume it was given", user == "guaca" and data_owner == "guaca",
            observed={"process": user, "volume": data_owner})
    sock = docker("exec", host.container, "sh", "-c", "test -S /var/run/docker.sock && echo yes || echo no")
    w.check("there is no Docker socket in the space", sock == "no", sock)

    code, _ = front_door(world, "/v1/call", "not-" + secrets.token_hex(8), json.dumps({"name": "capabilities"}))
    session, _ = front_door(world, "/v1/auth/session", "not-" + secrets.token_hex(8), "{}")
    w.check("a wrong token is refused at the front door", code == 401 and session in (401, 403),
            observed={"call": code, "session": session})
    code, said = front_door(world, "/health", "", None)
    w.check("/health answers at the space's https address", code == 200 and '"ok"' in said, observed=code)

    cookies = w.ui.send("Network.getCookies", {"urls": [ORIGIN + "/"]})["cookies"]
    session_cookie = [c for c in cookies if c["name"].startswith("guaca_session_v2")]
    w.check("the invitation leaves a Secure, HttpOnly, SameSite=Strict session cookie for the fly.dev address",
            len(session_cookie) == 1 and session_cookie[0]["secure"] and session_cookie[0]["httpOnly"]
            and session_cookie[0]["sameSite"] == "Strict" and session_cookie[0]["domain"] == HOSTNAME,
            observed=[{k: c.get(k) for k in ("name", "domain", "secure", "httpOnly", "sameSite")} for c in cookies])
    w.check("the page's event socket is live over wss at that address", len(w.events) > 0
            and w.ui.evaluate("location.origin") == ORIGIN, observed={"events": len(w.events)})
    w.check("the token is out of the address bar", "#token" not in (w.ui.evaluate("location.href") or ""))

    seen_by_app = mac_app(world)
    w.check("the Mac app's remote-host calls from tauri://localhost are admitted at the fly.dev address, and only with the token",
            seen_by_app["allowsApp"] and seen_by_app["preflight"].split(" ")[1] in ("200", "204")
            and " 200" in seen_by_app["call"]
            and " 101" in seen_by_app["socket"] and " 101" not in seen_by_app["wrongTokenSocket"],
            observed=seen_by_app)

    row = until("For you lists the first result", lambda: (
        w.ui.click("button", "For you") if not w.ui.find("dialog", "For you", exact=False) else None,
        w.ui.evaluate(FIRST_ROW_SHOWN))[1], 30)
    w.check("the first task's result is listed under For you", row and "Book the park" in (row.get("headline") or ""),
            observed=row, shot=True)
    w.ui.key("Escape")

    w.check("the token is never written to the space's log", host.token not in host.logs(),
            observed=[line for line in host.logs().splitlines() if "invitation" in line][:1])

    status = json.loads(front_door(world, "/v1/host", host.token)[1] or "{}")
    w.check("the host says it is a space with no updater", status == {"managed": False, "space": True},
            observed=status)
    menu_section(w, "Host")
    w.wait("the Host panel says the space is updated for you",
           lambda: "This host is a Guaca space, and it is updated for you." in w.ui.text())
    w.check("the Host panel offers no update and no instructions",
            not w.ui.find("button", "Back up and update host") and not w.ui.find("button", "View update instructions"),
            observed=w.ui.names("button")[:40], shot=True)

    capabilities = host.call("capabilities")
    w.check("the space offers no browser of its own", capabilities.get("localBrowser") is False,
            observed=capabilities)
    agent = host.scalar("SELECT id FROM agents WHERE name = ?", "Product Manager")
    try:
        host.call("give_agent_browser", id=agent)
        given = "given"
    except Failed as error:
        given = str(error)
    seen: list[str] = []

    def script(c):
        if c.last_user == "ping":
            return say("pong")
        if c.last.get("role") == "tool":
            seen.append(c.tool_results()[-1] if c.tool_results() else "")
            return say("I could not open a browser here.")
        if "Open the page" in c.last_user:
            return call("browse", action="open", url=SCRIPTED_PAGE)
        return None

    w.model.script = script
    began = time.monotonic()
    host.call("send_message", agentId=agent, text=f"Open the page {SCRIPTED_PAGE} and tell me its title.",
              idempotencyKey=secrets.token_hex(8))
    settled(w, 2, "the turn that asked for a browser settles")
    took = time.monotonic() - began
    tool_said = seen[-1] if seen else ""
    w.check("an agent that asks for a browser is told in plain words there is none, and its turn ends promptly",
            ("no browser" in tool_said or "Kernel" in tool_said or "browser" in given.lower()) and took < 30,
            observed={"giveBrowser": given[:200], "toolResult": tool_said[:300], "seconds": round(took, 1)})

    as_guaca = ["setpriv", "--reuid=guaca", "--regid=guaca", "--init-groups", "/bin/sh", "-c"]
    exec_token = docker("exec", host.container, *as_guaca, shell_snippet("TOKEN_FROM_HOST"))
    admits = docker("exec", host.container, *as_guaca, shell_snippet("ADMITS_FROM_HOST"))
    w.check("the token can be read and checked from inside the machine, the way space.sh does it",
            exec_token == host.token and admits == "200", observed={"matches": exec_token == host.token,
                                                                     "admits": admits})

    agents_before = host.scalar("SELECT count(*) FROM agents")
    host.stop()
    host.start()
    w.ui.evaluate("location.reload()")
    w.wait("the page is back after the container is replaced",
           lambda: w.ui.find("log", "Conversation with", exact=False) or w.ui.find("button", "App settings"))
    agents_after = host.scalar("SELECT count(*) FROM agents")
    still = host.rows("SELECT count(*) AS n FROM outcomes")[0]["n"]
    w.check("after the container is replaced on the same volume, the work and the session are still there",
            agents_after == agents_before and still >= 2 and "#token" not in (w.ui.evaluate("location.href") or ""),
            observed={"agentsBefore": agents_before, "agentsAfter": agents_after, "outcomes": still}, shot=True)
    w.check(f"the replaced container answered /health within {HEALTHY_WITHIN}s",
            host.healthy_after <= HEALTHY_WITHIN, observed=round(host.healthy_after, 2))
    w.check("the replaced container's log has no token either", host.token not in host.logs())


def shell_snippet(name: str) -> str:
    text = (HERE / "space.sh").read_text()
    line = next(line for line in text.splitlines() if line.startswith(f"{name}="))
    return shlex.split(line.split("=", 1)[1])[0]


def cleanup(world: SpaceWorld | None) -> None:
    for name in (f"{RUN}-guacad", f"{RUN}-front", f"{RUN}-model"):
        docker("rm", "-f", "-v", name, check=False)
    docker("volume", "rm", "-f", f"{RUN}-data", check=False)
    docker("network", "rm", f"{RUN}-net", check=False)


def main() -> int:
    if "-h" in sys.argv or "--help" in sys.argv:
        print(USAGE)
        return 0
    image = os.environ.get("IMAGE", "")
    if not image:
        print("Set IMAGE to the guacad image to prove, such as IMAGE=guacad:abc1234.", file=sys.stderr)
        return 2
    config = machine_config()
    world = SpaceWorld(image, config)
    failed = None
    try:
        prove(world, config)
    except Exception as error:
        failed = error
    finally:
        try:
            world.close()
        finally:
            cleanup(world)
    report = {"image": image, "steps": world.steps, "evidence": str(world.evidence),
              "healthySeconds": round(world.host.healthy_after, 2)}
    (world.evidence / "steps.json").write_text(json.dumps(world.steps, indent=1))
    print(redact(json.dumps(report, indent=1), [world.host.token]))
    if failed:
        print(redact(f"FAILED: {failed}", [world.host.token]), file=sys.stderr)
        return 1
    return 0 if all(step["pass"] for step in world.steps) else 1


if __name__ == "__main__":
    sys.exit(main())
