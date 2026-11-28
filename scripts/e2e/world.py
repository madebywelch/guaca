
from __future__ import annotations

import json
import os
import re
import secrets
import shutil
import signal
import socket
import sqlite3
import subprocess
import threading
import time
import urllib.error
import urllib.request
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

from .browser import Browser, BrowserError

ROOT = Path(__file__).resolve().parents[2]
TARGET = ROOT / "src-tauri/target"
VERIFY = TARGET / "verify"
SCRATCH = VERIFY / "scratch"
EVIDENCE = VERIFY / "evidence"
WEB = ROOT / "dist"
GUACAD = VERIFY / "bin/guacad"
GUACAD_CLOCK = VERIFY / "bin/guacad-clock"

STAND_IN_KEY = "verify-stand-in"
STAND_IN_MODEL = "verify/stand-in"


class Failed(Exception):
    pass


def admitted(browser: Browser) -> bool:
    return bool(browser.find("button", "App settings") or browser.find("region", "Get started with Guaca"))


def until(what: str, check: Callable[[], Any], timeout: float = 60, every: float = 0.1) -> Any:
    deadline = time.monotonic() + timeout
    last = ""
    while True:
        try:
            value = check()
        except Failed:
            raise
        except Exception as error:
            value, last = None, f"{type(error).__name__}: {error}"
        if value:
            return value
        if time.monotonic() >= deadline:
            raise Failed(f"{what}: not within {timeout:g}s" + (f" (last: {last})" if last else ""))
        time.sleep(every)


def redact(text: str, secrets_: list[str]) -> str:
    for secret in secrets_:
        if secret:
            text = text.replace(secret, "<redacted>")
    return re.sub(r"#token=[^\s\"']+", "#token=<redacted>", text)


def scrub(directory: Path, secrets_: list[str]) -> None:
    for path in directory.rglob("*"):
        if path.is_file() and path.suffix != ".png":
            raw = path.read_bytes().decode("utf-8", "surrogateescape")
            clean = redact(raw, secrets_)
            if clean != raw:
                path.write_bytes(clean.encode("utf-8", "surrogateescape"))


@dataclass
class Call:

    index: int
    body: dict
    key: str = ""

    @property
    def messages(self) -> list[dict]:
        return self.body.get("messages") or []

    @property
    def system(self) -> str:
        first = self.messages[0] if self.messages else {}
        return text_of(first) if first.get("role") == "system" else ""

    @property
    def speaker(self) -> str:
        found = re.match(r"You are ([^,.\n]+)", self.system)
        return found.group(1).strip() if found else ""

    @property
    def tools(self) -> list[str]:
        return [tool.get("function", {}).get("name", "") for tool in self.body.get("tools") or []]

    @property
    def last(self) -> dict:
        return self.messages[-1] if self.messages else {}

    @property
    def last_user(self) -> str:
        for message in reversed(self.messages):
            if message.get("role") == "user":
                return text_of(message)
        return ""

    def said(self, needle: str) -> bool:
        return any(needle in text_of(m) for m in self.messages if m.get("role") != "system")

    def tool_results(self) -> list[str]:
        results: list[str] = []
        for message in self.messages:
            if message.get("role") == "user":
                results = []
            elif message.get("role") == "tool":
                results.append(text_of(message))
        return results


def text_of(message: dict) -> str:
    content = message.get("content") or ""
    if isinstance(content, list):
        return " ".join(part.get("text", "") for part in content if isinstance(part, dict))
    return str(content)


@dataclass
class Reply:

    text: str = ""
    calls: list[tuple[str, dict]] = field(default_factory=list)
    hold: threading.Event | None = None
    status: int = 200

    def summary(self) -> dict:
        out: dict = {}
        if self.status != 200:
            out["status"] = self.status
        if self.text:
            out["say"] = self.text
        if self.calls:
            out["call"] = [{"name": name, "args": args} for name, args in self.calls]
        return out


def say(text: str) -> Reply:
    return Reply(text=text)


def call(name: str, **args: Any) -> Reply:
    return Reply(calls=[(name, args)])


def refuse(status: int, text: str) -> Reply:
    return Reply(text=text, status=status)


def held(gate: threading.Event, reply: Reply) -> Reply:
    reply.hold = gate
    return reply


UNSCRIPTED = "This reply was not scripted. The scenario did not expect this call."

USAGE = {"prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120}


class Model:

    def __init__(self, evidence: Path) -> None:
        from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

        self.script: Callable[[Call], Reply | None] = lambda _: None
        self.calls: list[dict] = []
        self.unscripted: list[dict] = []
        self.dropped = 0
        self.received = threading.Condition()
        self._lock = threading.Lock()
        self._log = evidence / "model.jsonl"
        self._bodies = evidence / "model"
        self._bodies.mkdir(parents=True, exist_ok=True)
        model = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *_: Any) -> None:
                pass

            def do_GET(self) -> None:
                body = json.dumps({"object": "list", "data": [{"id": STAND_IN_MODEL, "object": "model"}]}).encode()
                self.send_response(200)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_POST(self) -> None:
                length = int(self.headers.get("content-length", 0))
                body = json.loads(self.rfile.read(length) or b"{}")
                bearer = self.headers.get("authorization", "")
                reply = model.answer(body, bearer.removeprefix("Bearer ").strip())
                if reply.hold is not None:
                    reply.hold.wait()
                if reply.status != 200:
                    error = json.dumps({"error": {"message": reply.text, "code": reply.status}}).encode()
                    self.send_response(reply.status)
                    self.send_header("content-type", "application/json")
                    self.send_header("content-length", str(len(error)))
                    self.end_headers()
                    self.wfile.write(error)
                    return
                if body.get("stream"):
                    self.send_response(200)
                    self.send_header("content-type", "text/event-stream")
                    self.send_header("connection", "close")
                    self.end_headers()
                    self.close_connection = True
                    try:
                        for frame in stream(reply):
                            self.wfile.write(f"data: {json.dumps(frame)}\n\n".encode())
                            self.wfile.flush()
                        self.wfile.write(b"data: [DONE]\n\n")
                    except (BrokenPipeError, ConnectionResetError):
                        model.dropped += 1
                    return
                whole = json.dumps(complete(reply)).encode()
                self.send_response(200)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(whole)))
                self.end_headers()
                self.wfile.write(whole)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.base = f"http://127.0.0.1:{self.server.server_address[1]}/v1"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def answer(self, body: dict, key: str = "") -> Reply:
        with self._lock:
            index = len(self.calls) + 1
            request = Call(index, body, key)
            try:
                reply = self.script(request)
            except Exception as error:
                reply = None
                body = {**body, "_scriptError": repr(error)}
            entry = {"call": index, "speaker": request.speaker, "asked": request.last_user[-400:],
                     "last": request.last.get("role"), "toolResults": [r[:400] for r in request.tool_results()]}
            if reply is None:
                reply = say(UNSCRIPTED)
                entry["unscripted"] = True
                self.unscripted.append(entry)
            entry["answer"] = reply.summary()
            self.calls.append(entry)
            (self._bodies / f"{index:03}.json").write_text(json.dumps(body, indent=1))
            with self._log.open("a") as log:
                log.write(json.dumps(entry) + "\n")
        with self.received:
            self.received.notify_all()
        return reply

    def by(self, speaker: str) -> list[dict]:
        return [entry for entry in self.calls if entry["speaker"] == speaker]

    def stop(self) -> None:
        self.server.shutdown()
        self.server.server_close()


def stream(reply: Reply) -> list[dict]:
    frames: list[dict] = []
    for start in range(0, len(reply.text), 12):
        frames.append({"choices": [{"index": 0, "delta": {"content": reply.text[start:start + 12]}}]})
    for index, (name, args) in enumerate(reply.calls):
        frames.append({"choices": [{"index": 0, "delta": {"tool_calls": [{
            "index": index, "id": f"call_{index}_{name}", "type": "function",
            "function": {"name": name, "arguments": json.dumps(args)}}]}}]})
    finish = "tool_calls" if reply.calls else "stop"
    frames.append({"choices": [{"index": 0, "delta": {}, "finish_reason": finish}], "usage": USAGE})
    return frames


def complete(reply: Reply) -> dict:
    message: dict = {"role": "assistant", "content": reply.text or None}
    if reply.calls:
        message["tool_calls"] = [{"id": f"call_{i}_{name}", "type": "function",
                                  "function": {"name": name, "arguments": json.dumps(args)}}
                                 for i, (name, args) in enumerate(reply.calls)]
    return {"object": "chat.completion", "model": STAND_IN_MODEL, "usage": USAGE,
            "choices": [{"index": 0, "message": message, "finish_reason": "tool_calls" if reply.calls else "stop"}]}


class Egress:

    def __init__(self, log: Path) -> None:
        self.attempts: list[str] = []
        self._log = log
        self.sock = socket.socket()
        self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.listen(16)
        self.address = f"http://127.0.0.1:{self.sock.getsockname()[1]}"
        self._open = True
        threading.Thread(target=self._serve, daemon=True).start()

    def _serve(self) -> None:
        while self._open:
            try:
                conn, _ = self.sock.accept()
            except OSError:
                return
            with conn:
                conn.settimeout(2)
                try:
                    head = conn.recv(4096).split(b"\r\n", 1)[0].decode(errors="replace")
                except OSError:
                    head = ""
                target = " ".join(head.split(" ")[:2])
                self.attempts.append(target)
                with self._log.open("a") as log:
                    log.write(json.dumps({"refused": target}) + "\n")
                try:
                    conn.sendall(b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                except OSError:
                    pass

    def stop(self) -> None:
        self._open = False
        self.sock.close()


UpdaterScript = Callable[[dict], "dict | None"]


class Updater:

    def __init__(self, path: Path, evidence: Path) -> None:
        self.path = path
        self.asked: list[dict] = []
        self.script: UpdaterScript = lambda ask: None
        self._log = evidence / "updater.jsonl"
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.bind(str(path))
        self.sock.listen(16)
        self._open = True
        threading.Thread(target=self._serve, daemon=True).start()

    def _serve(self) -> None:
        while self._open:
            try:
                conn, _ = self.sock.accept()
            except OSError:
                return
            with conn:
                conn.settimeout(5)
                try:
                    ask = json.loads(conn.makefile("rb").readline())
                except (OSError, ValueError):
                    continue
                self.asked.append(ask)
                reply = self.script(ask)
                with self._log.open("a") as log:
                    log.write(json.dumps({"asked": ask, "answered": reply}) + "\n")
                if reply is None:
                    continue
                try:
                    conn.sendall((json.dumps(reply) + "\n").encode())
                except OSError:
                    pass

    def updates(self) -> list[dict]:
        return [ask for ask in self.asked if ask.get("ask") == "update"]

    def stop(self) -> None:
        self._open = False
        self.sock.close()


class Host:

    def __init__(self, scratch: Path, evidence: Path, egress: Egress, binary: Path,
                 minute_ms: int | None = None, updater: Path | None = None) -> None:
        self.root = scratch / "host"
        self.home = scratch / "home"
        for directory in (self.root, self.home):
            directory.mkdir(parents=True, exist_ok=True)
        self.binary = binary
        self.minute_ms = minute_ms
        self.updater = updater
        self.egress = egress
        self.port = 0
        self.token = secrets.token_hex(32)
        self.db = self.root / "data/guac.db"
        self.log = evidence / "host.log"
        self.proc: subprocess.Popen | None = None
        self.starts = 0

    @property
    def origin(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    @property
    def invitation(self) -> str:
        return f"{self.origin}/#token={self.token}"

    def start(self) -> None:
        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": str(self.home),
            "TZ": "UTC",
            "LANG": "C.UTF-8",
            "GUACA_ROOT": str(self.root),
            "GUACA_BIND": f"127.0.0.1:{self.port}",
            "GUACA_TOKEN": self.token,
            "GUACA_WEB": str(WEB),
            "GUAC_LOG": "guac=info,warn",
            "GUACA_ACCOUNT_ORIGIN": self.egress.address,
            "HTTP_PROXY": self.egress.address,
            "HTTPS_PROXY": self.egress.address,
            "ALL_PROXY": self.egress.address,
            "NO_PROXY": "127.0.0.1,localhost",
        }
        if self.minute_ms:
            env["GUACA_PROBE_MINUTE_MS"] = str(self.minute_ms)
        if self.updater:
            env["GUACA_UPDATER_SOCKET"] = str(self.updater)
        self.starts += 1
        with self.log.open("ab") as log:
            log.write(f"---- start {self.starts} ----\n".encode())
            log.flush()
            self.proc = subprocess.Popen([str(self.binary)], env=env, stdout=log,
                                         stderr=subprocess.STDOUT, start_new_session=True)
        until("the Host answers /health", self._healthy, 60)

    def _healthy(self) -> bool:
        assert self.proc
        if self.proc.poll() is not None:
            raise Failed(f"the Host exited with status {self.proc.returncode}; see host.log")
        if not self.port:
            ready = re.search(r"guacad ready addr=127\.0\.0\.1:(\d+)",
                              self.log_text().split(f"---- start {self.starts} ----")[-1])
            if not ready:
                return False
            self.port = int(ready.group(1))
        with urllib.request.urlopen(f"{self.origin}/health", timeout=1) as answer:
            return answer.status == 200

    def stop(self) -> int | None:
        if not self.proc or self.proc.poll() is not None:
            return self.proc.returncode if self.proc else None
        self.proc.send_signal(signal.SIGTERM)
        try:
            return self.proc.wait(timeout=60)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
            raise Failed("the Host did not stop within 60s of SIGTERM")

    def log_text(self) -> str:
        return re.sub(r"\x1b\[[0-9;]*m", "", self.log.read_text(errors="replace"))

    def kill(self) -> None:
        if self.proc and self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait()

    def call(self, name: str, **args: Any) -> Any:
        request = urllib.request.Request(
            f"{self.origin}/v1/call",
            data=json.dumps({"name": name, "args": args}).encode(),
            headers={"content-type": "application/json", "authorization": f"Bearer {self.token}"},
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

    def post(self, path: str, body: Any, headers: dict | None = None) -> tuple[int, str]:
        request = urllib.request.Request(f"{self.origin}{path}", data=json.dumps(body).encode(),
                                         headers={"content-type": "application/json", **(headers or {})},
                                         method="POST")
        try:
            with urllib.request.urlopen(request, timeout=30) as answer:
                return answer.status, answer.read().decode(errors="replace")
        except urllib.error.HTTPError as error:
            return error.code, error.read().decode(errors="replace")

    def rows(self, sql: str, *params: Any) -> list[dict]:
        conn = sqlite3.connect(f"file:{self.db}?mode=ro", uri=True, timeout=5)
        try:
            conn.row_factory = sqlite3.Row
            return [dict(row) for row in conn.execute(sql, params)]
        finally:
            conn.close()

    def scalar(self, sql: str, *params: Any) -> Any:
        found = self.rows(sql, *params)
        return next(iter(found[0].values())) if found else None


class World:

    def __init__(self, scenario: str, run: int, evidence_root: Path, clock_minute_ms: int | None = None,
                 binary: Path | None = None, updater: bool = False) -> None:
        self.scenario = scenario
        self.run = run
        self.scratch = SCRATCH / uuid.uuid4().hex[:12]
        self.scratch.mkdir(parents=True)
        self.evidence = evidence_root / scenario / f"run-{run}"
        if self.evidence.exists():
            shutil.rmtree(self.evidence)
        self.evidence.mkdir(parents=True)
        self.steps: list[dict] = []
        self.secrets: list[str] = []
        self.shots = 0
        self.page_errors: list[str] = []
        self.events: list[dict] = []
        self.egress = Egress(self.evidence / "egress.jsonl")
        self.browser_egress = Egress(self.evidence / "browser-egress.jsonl")
        self.model = Model(self.evidence)
        self.updater = Updater(self.scratch / "updater.sock", self.evidence) if updater else None
        self.host = Host(self.scratch, self.evidence, self.egress,
                         binary or (GUACAD_CLOCK if clock_minute_ms else GUACAD), clock_minute_ms,
                         self.updater.path if self.updater else None)
        self.browser: Browser | None = None


    def start(self, browser: bool = True) -> None:
        self.host.start()
        if browser:
            self.open_browser()

    def open_browser(self) -> Browser:
        if self.browser:
            self.browser.close()
        self.browser = Browser(self.scratch / f"browser-{self.host.starts}/profile",
                               proxy=self.browser_egress.address)
        self.browser.listen(self._on_page)
        self.browser.open(self.host.invitation)
        until("the page loads and the token is exchanged",
              lambda: self.browser and "#token=" not in (self.browser.evaluate("location.href") or "#token=")
              and admitted(self.browser), 60)
        return self.browser

    @property
    def ui(self) -> Browser:
        assert self.browser, "the scenario has not opened a browser"
        return self.browser

    def _on_page(self, message: dict) -> None:
        method = message.get("method")
        params = message.get("params") or {}
        if method == "Network.webSocketFrameReceived":
            payload = (params.get("response") or {}).get("payloadData") or ""
            try:
                event = json.loads(payload)
            except ValueError:
                return
            self.events.append(event)
            with (self.evidence / "events.jsonl").open("a") as log:
                log.write(redact(json.dumps(event), self.hidden) + "\n")
        elif method == "Runtime.exceptionThrown":
            details = params.get("exceptionDetails") or {}
            text = (details.get("exception") or {}).get("description") or details.get("text") or "exception"
            self.page_errors.append(text.splitlines()[0][:300])

    def close(self) -> None:
        for stop in (lambda: self.browser and self.browser.close(), self.host.stop,
                     self.model.stop, self.egress.stop, self.browser_egress.stop,
                     lambda: self.updater and self.updater.stop()):
            try:
                stop()
            except Exception:
                try:
                    self.host.kill()
                except Exception:
                    pass
        browser_log = self.scratch / f"browser-{self.host.starts}/chromium.log"
        if browser_log.exists():
            shutil.copy(browser_log, self.evidence / "chromium.log")
        scrub(self.evidence, self.hidden)
        shutil.rmtree(self.scratch, ignore_errors=True)


    def check(self, step: str, holds: Any, observed: Any = None, detail: Any = None,
              shot: bool = False, stop: bool = True) -> bool:
        entry: dict = {"step": step, "pass": bool(holds)}
        if observed is not None:
            entry["observed"] = observed
        if detail is not None:
            entry["detail"] = detail
        if shot and self.browser:
            entry["evidence"] = self.capture(step)
        self.steps.append(json.loads(redact(json.dumps(entry), self.hidden)))
        if stop and not holds:
            raise Failed(step)
        return bool(holds)

    def capture(self, label: str) -> list[str]:
        self.shots += 1
        slug = re.sub(r"[^a-z0-9]+", "-", label.lower()).strip("-")[:48]
        stem = f"{self.shots:02}-{slug}"
        try:
            if any(secret in self.ui.text() for secret in self.hidden):
                (self.evidence / f"{stem}.png.withheld").write_text("a secret was on screen\n")
            else:
                self.ui.screenshot(self.evidence / f"{stem}.png")
            (self.evidence / f"{stem}.ax.txt").write_text(redact(self.ui.snapshot(), self.hidden))
        except BrowserError as error:
            return [f"capture failed: {error}"]
        return [f"{stem}.png", f"{stem}.ax.txt"]

    @property
    def hidden(self) -> list[str]:
        return [self.host.token, *self.secrets]

    def wait(self, what: str, check: Callable[[], Any], timeout: float = 60) -> Any:
        return until(what, check, timeout)
