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

    FLY_API_TOKEN=... deploy/space/prove.py --live <name>

proves a space running on Fly instead, through its public address and the
Machines API: the certificate, /health through Fly's proxy and Fly's own check,
http sent to https, guacad running as guaca, a wrong token refused and the
space's own admitted, the event socket over wss, the volume mounted and owned
by guaca, data kept through a restart and an update --force to the same image,
no token in Fly's logs, and no reach to or from another app in the
organization, tested with a throwaway app it destroys at once (--skip-probe
leaves that out). It reads the
machine's memory from Fly's metrics, and leaves the workspace as it found it.
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


LIVE_MARK_DIR = "/var/lib/guaca"
AS_GUACA = ["setpriv", "--reuid=guaca", "--regid=guaca", "--init-groups", "/bin/sh", "-c"]
CONNECTS = ("import socket, sys\n"
            "try:\n"
            "    socket.create_connection((sys.argv[1], int(sys.argv[2])), timeout=5).close(); print('open')\n"
            "except Exception as e:\n"
            "    print('closed', type(e).__name__)\n")
RESOLVES = ("import socket, sys\n"
            "try:\n"
            "    print('resolves', socket.getaddrinfo(sys.argv[1], 80)[0][4][0])\n"
            "except Exception as e:\n"
            "    print('unknown', type(e).__name__)\n")


class LiveFly:

    def __init__(self, app: str) -> None:
        self.app = app
        self.api = os.environ.get("FLY_API_HOSTNAME", "https://api.machines.dev").rstrip("/")
        token = os.environ.get("FLY_API_TOKEN", "").strip()
        if not token:
            flyctl = next((t for t in ("flyctl", "fly") if subprocess.run(["sh", "-c", f"command -v {t}"],
                                                                           capture_output=True).returncode == 0), "")
            if flyctl:
                token = subprocess.run([flyctl, "auth", "token"], capture_output=True, text=True).stdout.strip()
        if not token:
            raise Failed("Set FLY_API_TOKEN to a token for the space's organization, or log in with 'fly auth login'.")
        self.auth = token if token.startswith("FlyV1 ") else f"Bearer {token}"
        self.secrets = [token, *token.removeprefix("FlyV1 ").split(",")]

    def request(self, method: str, url: str, body: object = None, timeout: float = 60) -> tuple[int, object]:
        import urllib.error
        import urllib.request
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(url if url.startswith("http") else self.api + url, data=data, method=method,
                                     headers={"Authorization": self.auth, "Accept": "application/json",
                                              **({"Content-Type": "application/json"} if data else {})})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as answer:
                raw = answer.read()
                status = answer.status
        except urllib.error.HTTPError as error:
            raw, status = error.read(), error.code
        except (OSError, ValueError) as error:
            raise Failed(f"could not reach Fly for {method} {url.split('?')[0]}: {type(error).__name__}") from None
        try:
            return status, json.loads(raw) if raw else None
        except ValueError:
            return status, raw.decode(errors="replace")[:300]

    def ok(self, method: str, url: str, body: object = None, timeout: float = 60) -> object:
        status, answer = self.request(method, url, body, timeout)
        if not 200 <= status < 300:
            raise Failed(f"Fly answered {method} {url.split('?')[0]} with {status}: {str(answer)[:200]}")
        return answer

    def machines(self, app: str | None = None) -> list[dict]:
        return [m for m in self.ok("GET", f"/v1/apps/{app or self.app}/machines") if m.get("state") != "destroyed"]

    def exec(self, machine: str, command: list[str], app: str | None = None) -> str:
        answer = self.ok("POST", f"/v1/apps/{app or self.app}/machines/{machine}/exec",
                         {"command": command, "timeout": 30}, timeout=60)
        return (answer.get("stdout") or "").strip()


def tls_answer(hostname: str, request: str) -> tuple[str, dict]:
    context = ssl.create_default_context()
    with socket.create_connection((hostname, 443), timeout=15) as plain:
        with context.wrap_socket(plain, server_hostname=hostname) as tls:
            cert = tls.getpeercert()
            tls.sendall(request.encode())
            head = b""
            while b"\r\n\r\n" not in head:
                chunk = tls.recv(4096)
                if not chunk:
                    break
                head += chunk
    return head.split(b"\r\n\r\n", 1)[0].decode(errors="replace"), cert


def https(hostname: str, method: str, path: str, token: str = "", body: str = "") -> tuple[int, str, dict]:
    import urllib.error
    import urllib.request
    headers = {"content-type": "application/json"}
    if token:
        headers["authorization"] = f"Bearer {token}"
    req = urllib.request.Request(f"https://{hostname}{path}", data=body.encode() if body else None, method=method,
                                 headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=15) as answer:
            return answer.status, answer.read().decode(errors="replace"), dict(answer.headers)
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode(errors="replace"), dict(error.headers)


def live_healthy(hostname: str, within: float = 120) -> float:
    began = time.monotonic()

    def answers() -> bool:
        code, said, _ = https(hostname, "GET", "/health")
        return code == 200 and '"status":"ok"' in said

    until(f"https://{hostname}/health answers", answers, within, every=1)
    return time.monotonic() - began


def live_started(fly: LiveFly, machine: str, app: str | None = None, within: float = 180) -> None:
    until(f"the machine {machine} is started",
          lambda: fly.ok("GET", f"/v1/apps/{app or fly.app}/machines/{machine}").get("state") == "started",
          within, every=2)


def live_network(fly: LiveFly, org: str, region: str | None, image: str, mid: str, check) -> None:
    probe = f"{fly.app}-probe-{secrets.token_hex(3)}"
    try:
        fly.ok("POST", "/v1/apps", {"name": probe, "org_slug": org})
        made = fly.ok("POST", f"/v1/apps/{probe}/machines", {"name": "probe", "region": region, "config": {
            "image": image, "guest": {"cpu_kind": "shared", "cpus": 1, "memory_mb": 256}, "auto_destroy": True,
            "restart": {"policy": "no"},
            "processes": [{"name": "probe", "exec": ["python3", "-m", "http.server", "--bind", "::", "8080"]}]}})
        live_started(fly, made["id"], probe)
        probe_ip = fly.ok("GET", f"/v1/apps/{probe}/machines/{made['id']}")["private_ip"]
        space_ip = fly.ok("GET", f"/v1/apps/{fly.app}/machines/{mid}")["private_ip"]
        listening = until("the probe listens on its private address", lambda: "open" in fly.exec(
            made["id"], ["python3", "-c", CONNECTS, probe_ip, "8080"], probe), 30, every=1)
        own = fly.exec(mid, ["python3", "-c", CONNECTS, space_ip, "22"])
        to_probe = fly.exec(mid, ["python3", "-c", CONNECTS, probe_ip, "8080"])
        by_name = fly.exec(mid, ["python3", "-c", RESOLVES, f"{probe}.internal"])
        to_space = fly.exec(made["id"], ["python3", "-c", CONNECTS, space_ip, "22"], probe)
        check("from inside the space, another app in the organization cannot be reached, nor reach it",
              bool(listening) and own.startswith("open") and to_probe.startswith("closed")
              and by_name.startswith("unknown") and to_space.startswith("closed"),
              {"otherAppToItself": listening, "spaceToItself": own, "spaceToOtherApp": to_probe,
               "otherAppByName": by_name, "otherAppToSpace": to_space})
    finally:
        fly.request("DELETE", f"/v1/apps/{probe}?force=true")
        gone = fly.request("GET", f"/v1/apps/{probe}")[0]
        check(f"the throwaway app {probe} was destroyed", gone == 404, gone)


def prove_live(name: str, probe_network: bool = True) -> tuple[list[dict], list[str]]:
    fly = LiveFly(name)
    hostname = f"{name}.fly.dev"
    steps: list[dict] = []
    hidden = list(fly.secrets)

    def check(what: str, holds: bool, observed: object = "") -> None:
        steps.append({"step": what, "pass": bool(holds), "observed": observed})
        print(redact(f"{'PASS' if holds else 'FAIL'}  {what}  {json.dumps(observed)}", hidden), flush=True)

    status, app = fly.request("GET", f"/v1/apps/{name}")
    if status != 200:
        raise Failed(f"there is no space named {name} this token can see (HTTP {status})")
    org = app.get("organization", {}).get("slug", "")
    machines = fly.machines()
    check("the space has one machine, and it is started", len(machines) == 1 and machines[0]["state"] == "started",
          [{"id": m["id"], "state": m["state"], "region": m.get("region")} for m in machines])
    machine = machines[0]
    mid = machine["id"]
    image = machine["config"]["image"]

    head, cert = tls_answer(hostname, f"HEAD /health HTTP/1.1\r\nHost: {hostname}\r\nConnection: close\r\n\r\n")
    expires = ssl.cert_time_to_seconds(cert["notAfter"])
    issuer = dict(x[0] for x in cert.get("issuer", ()))
    check("the HTTPS certificate is valid for the address, and not about to expire",
          expires - time.time() > 7 * 86400,
          {"issuer": issuer.get("organizationName"), "notAfter": cert["notAfter"]})
    code, said, headers = https(hostname, "GET", "/health")
    check("/health passes through Fly's proxy", code == 200 and '"status":"ok"' in said
          and "fly-request-id" in {k.lower() for k in headers}, {"status": code, "health": said[:200]})
    checks = machine.get("checks") or []
    check("Fly's own health check passes", bool(checks) and all(c.get("status") == "passing" for c in checks),
          [{k: c.get(k) for k in ("name", "status")} for c in checks])
    redirect = subprocess.run(["curl", "-sS", "-o", "/dev/null", "-w", "%{http_code} %{redirect_url}",
                               "--max-time", "15", f"http://{hostname}/health"], capture_output=True, text=True).stdout
    check("plain http is sent to https", redirect.split(" ")[0] in ("301", "302", "307", "308")
          and f"https://{hostname}" in redirect, redirect)

    who = fly.exec(mid, ["/bin/sh", "-c", "for p in /proc/[0-9]*; do [ \"$(cat $p/comm 2>/dev/null)\" = guacad ] "
                                          "&& stat -c %U $p && break; done"])
    check("guacad runs as guaca, the way processes[].user and exec say", who == "guaca", who)

    token = fly.exec(mid, [*AS_GUACA, shell_snippet("TOKEN_FROM_HOST")])
    hidden.append(token)
    check("the space's token can be read from inside the machine, and is strong",
          len(token) == 64 and all(c in "0123456789abcdef" for c in token), {"length": len(token)})
    capabilities = json.dumps({"name": "capabilities"})
    wrong, _, _ = https(hostname, "POST", "/v1/call", "not-" + secrets.token_hex(8), capabilities)
    session, _, _ = https(hostname, "POST", "/v1/auth/session", "not-" + secrets.token_hex(8), "{}")
    check("a wrong token is refused through Fly's proxy", wrong == 401 and session in (401, 403),
          {"call": wrong, "session": session})
    right, said, _ = https(hostname, "POST", "/v1/call", token, capabilities)
    check("the space's own token is admitted through Fly's proxy", right == 200 and '"ok"' in said, right)
    shaking = ("GET /v1/events?token={} HTTP/1.1\r\nHost: " + hostname + "\r\nOrigin: {}\r\nUpgrade: websocket\r\n"
               "Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n")
    page = tls_answer(hostname, shaking.format(token, f"https://{hostname}"))[0].split("\r\n", 1)[0]
    app_socket = tls_answer(hostname, shaking.format(token, "tauri://localhost"))[0].split("\r\n", 1)[0]
    refused = tls_answer(hostname, shaking.format("not-" + secrets.token_hex(8), f"https://{hostname}"))[0]
    check("the event WebSocket connects over wss, from the page and from the Mac app, and not with a wrong token",
          " 101" in page and " 101" in app_socket and " 101" not in refused.split("\r\n", 1)[0],
          {"page": page, "macApp": app_socket, "wrongToken": refused.split("\r\n", 1)[0]})

    volume = machine["config"]["mounts"][0]
    mounted = fly.exec(mid, ["/bin/sh", "-c", f"grep ' {volume['path']} ' /proc/mounts | cut -d' ' -f1,3; "
                                              f"stat -c '%U:%G' {volume['path']} {volume['path']}/data"])
    check("the volume is mounted at its path and owned by guaca",
          mounted.count("guaca:guaca") == 2 and "ext4" in mounted, {"volume": volume["volume"], "seen": mounted})

    mark = f"{LIVE_MARK_DIR}/.prove-{secrets.token_hex(4)}"
    stamp = secrets.token_hex(8)
    fly.exec(mid, [*AS_GUACA, f"printf %s {stamp} > {mark}"])

    def current() -> str:
        return fly.machines()[0]["id"]

    def marked() -> str:
        return fly.exec(current(), ["/bin/sh", "-c", f"cat {mark} 2>/dev/null; true"])

    try:
        def uptime() -> float:
            try:
                return float(fly.exec(mid, ["cat", "/proc/uptime"]).split()[0])
            except (Failed, ValueError, IndexError):
                return float("inf")

        before = uptime()
        fly.ok("POST", f"/v1/apps/{name}/machines/{mid}/restart", timeout=120)
        until("the machine has booted again", lambda: uptime() < before, 180, every=2)
        live_started(fly, mid)
        took = live_healthy(hostname)
        check("data on the volume survives a machine restart, and the space answers again",
              marked() == stamp, {"upBefore": round(before), "upAfter": round(uptime()),
                                  "healthyAfterSeconds": round(took, 1)})

        snapshots_before = len(fly.ok("GET", f"/v1/apps/{name}/volumes/{volume['volume']}/snapshots") or [])
        done = subprocess.run(["bash", str(HERE / "space.sh"), "update", name, image, "--force"],
                              capture_output=True, text=True, stdin=subprocess.DEVNULL, start_new_session=True,
                              env={**os.environ, "FLY_API_TOKEN": fly.secrets[0]}, timeout=1200)
        said = redact(done.stdout + done.stderr, hidden)
        after = fly.machines()
        snapshots_after = len(fly.ok("GET", f"/v1/apps/{name}/volumes/{volume['volume']}/snapshots") or [])
        check("update to the same digest snapshots, swaps and passes its health check, on the same machine and volume",
              done.returncode == 0 and "now runs" in said and len(after) == 1 and after[0]["id"] == mid
              and after[0]["config"]["mounts"][0]["volume"] == volume["volume"]
              and snapshots_after > snapshots_before,
              {"said": said.strip().splitlines()[-1:], "snapshots": [snapshots_before, snapshots_after]})
        check("data on the volume survives the update", marked() == stamp)
    finally:
        fly.exec(current(), ["/bin/sh", "-c", f"rm -f {mark}"])
    check("the proof's mark is gone from the volume", marked() == "")
    mid = current()

    lines: list[str] = []
    cursor = ""
    for _ in range(30):
        _, page_ = fly.request("GET", f"https://api.fly.io/api/v1/apps/{name}/logs"
                                      + (f"?next_token={cursor}" if cursor else ""))
        if not isinstance(page_, dict):
            break
        batch = [str((entry.get("attributes") or {}).get("message", "")) for entry in page_.get("data") or []]
        lines += batch
        following = (page_.get("meta") or {}).get("next_token") or ""
        if not batch or following == cursor:
            break
        cursor = following
    text = "\n".join(lines)
    check("Fly's logs for the app hold no token and no invitation", len(lines) > 0 and token not in text
          and "#token=" not in text and not any(part and part in text for part in fly.secrets),
          {"linesRead": len(lines), "guacadStarts": text.count("keeping local time")})

    if probe_network:
        live_network(fly, org, machine.get("region"), image, mid, check)
    else:
        print("SKIP  the private network, as --skip-probe asks", flush=True)

    query = "https://api.fly.io/prometheus/" + org + "/api/v1/query?query="
    import urllib.parse

    def metric(expr: str) -> float:
        answer = fly.ok("GET", query + urllib.parse.quote(expr))
        found = (answer.get("data") or {}).get("result") or []
        return float(found[0]["value"][1]) if found else 0.0

    total = metric(f'fly_instance_memory_mem_total{{app="{name}"}}')
    available = metric(f'avg_over_time(fly_instance_memory_mem_available{{app="{name}"}}[10m])')
    rss = fly.exec(mid, ["/bin/sh", "-c", "for p in /proc/[0-9]*; do [ \"$(cat $p/comm 2>/dev/null)\" = guacad ] "
                                          "&& sed -n 's/^VmRSS: *//p' $p/status && break; done"])
    check("memory in use, read from Fly's metrics", total > 0,
          {"totalMiB": round(total / 2**20), "usedMiB": round((total - available) / 2**20),
           "guacadResident": rss})
    steps.append({"step": "a first task with the scripted model", "pass": True,
                  "observed": "skipped: the scripted model runs on this machine, which a space cannot reach, and a "
                              "task would hire agents into the owner's workspace"})
    print("SKIP  a first task with the scripted model", flush=True)
    return steps, hidden


def live_main(name: str, probe_network: bool = True) -> int:
    try:
        steps, hidden = prove_live(name, probe_network)
    except Failed as error:
        print(f"FAILED: {error}", file=sys.stderr)
        return 1
    print(f"\n{sum(not s['pass'] for s in steps)} failed of {len(steps)}")
    return 0 if all(step["pass"] for step in steps) else 1


def main() -> int:
    if "-h" in sys.argv or "--help" in sys.argv:
        print(USAGE)
        return 0
    if "--live" in sys.argv:
        at = sys.argv.index("--live") + 1
        if at >= len(sys.argv):
            print("Name the space: deploy/space/prove.py --live <name>.", file=sys.stderr)
            return 2
        return live_main(sys.argv[at], "--skip-probe" not in sys.argv)
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
