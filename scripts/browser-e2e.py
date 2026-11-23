#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import re
import secrets
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
from e2e.world import Reply, call, complete, say, stream, text_of

RUN = f"guaca-e2e-{os.getpid()}"
NET = f"{RUN}-net"
GUACAD = f"{RUN}-guacad"
SIDECAR = f"{RUN}-browser"
MODEL = f"{RUN}-model"
MODEL_PORT = 8000
TOKEN = secrets.token_hex(32)
PAGE = "https://example.com/"
WEB_CONTENT = "[WEB CONTENT"


class Failed(Exception):
    pass


def docker(*args: str, check: bool = True, env: dict | None = None) -> str:
    done = subprocess.run(["docker", *args], capture_output=True, text=True, env=env)
    if check and done.returncode != 0:
        raise Failed(f"docker {' '.join(args[:3])}: {done.stderr.strip()}")
    return done.stdout.strip()


def sidecar_args() -> list[str]:
    script = (ROOT / "scripts/browser-isolation.sh").read_text()
    block = script[script.index("SIDECAR_ARGS=(") :]
    block = block[block.index("\n") + 1 : block.index("\n)")]
    return [word.strip('"') for line in block.splitlines() for word in line.split()]


def until(what: str, check, seconds: float):
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        try:
            last = check()
        except (Failed, OSError, urllib.error.URLError) as error:
            last = error
        else:
            if last:
                return last
        time.sleep(0.5)
    raise Failed(f"{what} did not happen within {seconds:.0f}s (last: {last!r})")


class Host:
    def __init__(self, origin: str) -> None:
        self.origin = origin

    def call(self, name: str, **args):
        request = urllib.request.Request(
            f"{self.origin}/v1/call",
            data=json.dumps({"name": name, "args": args}).encode(),
            headers={"content-type": "application/json", "authorization": f"Bearer {TOKEN}"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as answer:
                reply = json.loads(answer.read())
        except urllib.error.HTTPError as error:
            raise Failed(f"{name}: HTTP {error.code}: {error.read()[:300].decode(errors='replace')}")
        if "ok" not in reply:
            raise Failed(f"{name}: {json.dumps(reply)[:300]}")
        return reply["ok"]


class Model:
    def __init__(self) -> None:
        self.seen: list[str] = []
        model = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *_) -> None:
                pass

            def do_GET(self) -> None:
                whole = json.dumps(model.seen).encode()
                self.send_response(200)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(whole)))
                self.end_headers()
                self.wfile.write(whole)

            def do_POST(self) -> None:
                length = int(self.headers.get("content-length", 0))
                body = json.loads(self.rfile.read(length) or b"{}")
                reply = model.answer(body)
                if body.get("stream"):
                    self.send_response(200)
                    self.send_header("content-type", "text/event-stream")
                    self.send_header("connection", "close")
                    self.end_headers()
                    self.close_connection = True
                    for frame in stream(reply):
                        self.wfile.write(f"data: {json.dumps(frame)}\n\n".encode())
                    self.wfile.write(b"data: [DONE]\n\n")
                    return
                whole = json.dumps(complete(reply)).encode()
                self.send_response(200)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(whole)))
                self.end_headers()
                self.wfile.write(whole)

        self.server = ThreadingHTTPServer(("0.0.0.0", MODEL_PORT), Handler)
        self.server.daemon_threads = True

    def answer(self, body: dict) -> Reply:
        messages = body.get("messages") or []
        last = messages[-1] if messages else {}
        if last.get("role") != "tool":
            return call("browse", action="open", url=PAGE)
        result = text_of(last)
        self.seen.append(result)
        if "being set up" in result:
            return say("The browser on this Mac is still being set up, so I will look again shortly.")
        title = next((line for line in result.splitlines()[1:] if line.strip()), "")
        return say(f'I opened it. The page is titled "{title.strip()}".')


def said_in(envelope: dict) -> str:
    return " ".join(part["text"] for part in envelope.get("parts", []) if part.get("text"))


def reply_after(host: Host, agent: str, sent: str) -> dict:
    def found():
        listed = host.call("channel_messages", channelId=agent, limit=20)
        after = False
        for envelope in listed:
            if envelope["id"] == sent:
                after = True
            elif after and envelope["from"]["kind"] == "agent" and said_in(envelope):
                return envelope
        return None

    return until("the agent's reply", found, 120)


def main() -> int:
    passed = failed = 0

    def check(what: str, ok: bool, detail: str = "") -> None:
        nonlocal passed, failed
        if ok:
            passed += 1
        else:
            failed += 1
        print(f"{'PASS' if ok else 'FAIL'}  {what}{f'  ({detail})' if detail else ''}", flush=True)

    guacad_image = os.environ["GUACAD_IMAGE"]
    browser_image = os.environ["BROWSER_IMAGE"]
    docker("network", "create", NET)
    docker(
        "run", "-d", "--name", MODEL, "--network", NET, "-p", f"127.0.0.1::{MODEL_PORT}",
        "-v", f"{ROOT / 'scripts'}:/scripts:ro", "--entrypoint", "python3", guacad_image,
        "/scripts/browser-e2e.py", "model",
    )
    model_port = docker("port", MODEL, f"{MODEL_PORT}/tcp").splitlines()[0].rsplit(":", 1)[1]

    def seen() -> list[str]:
        with urllib.request.urlopen(f"http://127.0.0.1:{model_port}/seen", timeout=5) as answer:
            return json.loads(answer.read())

    until("the stand-in model answers", lambda: seen() == [], 60)
    docker(
        "run", "-d", "--name", GUACAD, "--network", NET, "-p", "127.0.0.1::8787",
        "--env", "GUACA_TOKEN", "--env", f"GUACA_BROWSER={SIDECAR}:9223", guacad_image,
        env={**os.environ, "GUACA_TOKEN": TOKEN},
    )
    port = docker("port", GUACAD, "8787/tcp").splitlines()[0].rsplit(":", 1)[1]
    host = Host(f"http://127.0.0.1:{port}")
    until("guacad answers /health", lambda: urllib.request.urlopen(f"{host.origin}/health", timeout=2).status == 200, 90)

    host.call("update_settings", patch={
        "provider": "compatible", "baseUrl": f"http://{MODEL}:{MODEL_PORT}/v1", "apiKey": "sk-stand-in", "defaultModel": "verify/stand-in",
    })
    capabilities = host.call("capabilities")
    settings = host.call("get_settings")
    print(f"capabilities: {json.dumps(capabilities, sort_keys=True)}")
    print(f"kernelKeySet: {json.dumps(settings['kernelKeySet'])}")
    check("the host offers a browser of its own with no Kernel key",
          capabilities.get("localBrowser") is True and settings["kernelKeySet"] is False)

    agent = host.call("create_agent", draft={
        "name": "Scout", "avatar": "avocado", "color": "#7fb069", "model": "", "systemPrompt": "", "skills": [],
    })["id"]
    state = host.call("local_browser")
    check("nothing is set up while nobody has a browser", state == "idle", state)

    host.call("give_agent_browser", id=agent)
    state = host.call("local_browser")
    check("giving an agent a browser asks for one to be set up", state == "settingUp", state)

    sent = host.call("send_message", agentId=agent, text=f"Open {PAGE} and tell me its title.", idempotencyKey=secrets.token_hex(8))["messageId"]
    early = reply_after(host, agent, sent)
    said = said_in(early)
    check("a browse before the browser is ready is told it is being set up, and the turn ends",
          any("being set up" in result for result in seen()), said[:120])

    docker("run", "-d", "--name", SIDECAR, "--network", NET, *sidecar_args(), browser_image)
    state = until("the host sees its browser", lambda: host.call("local_browser") == "ready" and "ready", 90)
    check("the host sees the browser once it is up", state == "ready", state)

    sent = host.call("send_message", agentId=agent, text=f"Open {PAGE} and tell me its title.", idempotencyKey=secrets.token_hex(8))["messageId"]
    reply = reply_after(host, agent, sent)
    page = seen()[-1]
    print("tool result the model saw:")
    print(page[:600])
    said = said_in(reply)
    print(f"agent reply: {said}")
    check("the model saw the page under the web-content label", page.startswith(WEB_CONTENT) and PAGE in page)
    check("the agent replied with the page's title", "Example Domain" in said, said[:120])
    browser = host.call("agent_browser", id=agent)
    print(f"agent browser: {json.dumps(browser, sort_keys=True)}")
    check("the agent's browser is running with no live view",
          bool(browser) and browser["state"] == "running" and browser["liveViewUrl"] is None)

    print(f"\n{passed} passed, {failed} failed")
    return 0 if failed == 0 else 1


def cleanup() -> None:
    for name in (SIDECAR, GUACAD, MODEL):
        docker("rm", "-f", "-v", name, check=False)
    gone = subprocess.run(["docker", "network", "rm", NET], capture_output=True, text=True)
    if gone.returncode != 0 and not re.search(r"not found|No such network", gone.stderr):
        print(f"could not remove {NET}: {gone.stderr.strip()}", file=sys.stderr)


if __name__ == "__main__" and sys.argv[1:] == ["model"]:
    Model().server.serve_forever()
elif __name__ == "__main__":
    try:
        code = main()
    except Failed as error:
        print(f"FAIL  {error}")
        subprocess.run(["docker", "logs", "--tail", "40", GUACAD])
        code = 1
    finally:
        cleanup()
    sys.exit(code)
