#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import math
import os
import re
import secrets
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path
from typing import Any, Callable

USAGE = """Run Guaca's Mac-only behavior as scenarios a machine can check.

    scripts/probe.py                       every scenario, three times each
    scripts/probe.py launch panel          some of them
    scripts/probe.py --runs 1 permission   once
    scripts/probe.py --no-build ...        reuse the last test build
    scripts/probe.py --list                what there is

Each scenario starts a scratch Host (the daemon, on loopback, with its own
data directory and a seeded workspace) and a test build of the app (its own
bundle identifier, a scratch home, launched in the background), drives it
through the probe's control channel, asks macOS what it did, and tears all of
it down. One JSON document comes out on stdout; the exit status is 1 if any
step failed. docs/PROBE.md says what each scenario proves and what it cannot.

Screen steps (pressing Allow on a permission prompt, clicking a banner,
capturing the test windows) need Accessibility and Screen Recording for the
app that runs this command. Without them those scenarios fail with that
reason; nothing here asks for either.
"""

ROOT = Path(__file__).resolve().parent.parent
TAURI = ROOT / "src-tauri"
BUILT_APP = TAURI / "target/debug/bundle/macos/Guaca Probe.app"
GUACAD = TAURI / "target/debug/guacad"
HELPER_SOURCE = ROOT / "scripts/probe/system.swift"
HELPER = TAURI / "target/probe/system"
APPS = TAURI / "target/probe/apps"
SEED = ROOT / ".claude/skills/run-guaca/seed.py"
LSREGISTER = (
    "/System/Library/Frameworks/CoreServices.framework/Frameworks/"
    "LaunchServices.framework/Support/lsregister"
)

IDENTIFIER = "com.madebywelch.guac.probe"
APP_NAME = "Guaca Probe"
AGENTS = ["Manager", "Researcher", "Critic", "Scribe"]
GROUP = "00000000-0000-4000-8000-000000000001"

DIALOG = ROOT / "src/components/SettingsPage.tsx"


def settings_copy() -> tuple[dict[str, str], str]:
    source = DIALOG.read_text()
    access = re.search(r"^const MAC_ACCESS: Record<NotificationAccess, string> = \{\n(.*?)^\};$", source, re.M | re.S)
    banner = re.search(r'^const TEST_NOTIFICATION = "([^"]+)";$', source, re.M)
    if access is None or banner is None:
        raise SystemExit(f"{DIALOG.relative_to(ROOT)} has no MAC_ACCESS or TEST_NOTIFICATION, which are the words"
                         " the probe expects the Notifications pane and the test banner to say")
    return dict(re.findall(r'^\s*(\w+):\s*"([^"]+)",$', access.group(1), re.M)), banner.group(1)


PANE, TEST_BODY = settings_copy()
SENT = "Sent. It should be on your screen now."

DELIVERY_TITLE = "Guaca probe: test notification"
DELIVERY_BODY = "Raised by scripts/probe.py while testing a build. Nothing needs you."

FAILURES = TAURI / "target/probe/failures"
DIAGNOSTIC_TIMEOUT = 1.0
EVIDENCE_BYTES = 256 * 1024
NATIVE_BYTES = 1024 * 1024
NATIVE_ROWS = 4096
PAGE_ROWS = 256
EVIDENCE_SNAPSHOTS = 16
NATIVE_PHASES = frozenset({
    "trace.opened", "notification.response",
    "notification.default", "notification.clicked", "window.open", "window.reveal",
    "toggle.enter", "toggle.return", "app.did_hide", "toggle.completed",
})
PAGE_PHASES = frozenset({
    "notification.clicked", "notification.missing", "notification.remembered", "reveal.agent",
    "select.requested", "select.invalid", "select.committed",
})


class Failed(Exception):
    pass


class Refused(Exception):
    pass


class Deferred(Exception):
    pass


def redact(text: str, secrets_: list[str]) -> str:
    for secret in secrets_:
        if secret:
            text = text.replace(secret, "<redacted>")
    return re.sub(r"#token=[^\s\"']+", "#token=<redacted>", text)


def wait_until(what: str, check: Callable[[], Any], timeout: float, every: float = 0.1) -> Any:
    deadline = time.monotonic() + timeout
    last = ""
    while True:
        try:
            value = check()
        except Failed:
            raise
        except Exception as error:
            value, last = None, str(error)
        if value:
            return value
        if time.monotonic() >= deadline:
            raise Failed(f"{what}: not within {timeout:g}s" + (f" (last: {last})" if last else ""))
        time.sleep(every)


def run(args: list[str], **kwargs: Any) -> subprocess.CompletedProcess:
    return subprocess.run(args, capture_output=True, text=True, **kwargs)



def helper(*args: str) -> Any:
    if not HELPER.exists() or HELPER.stat().st_mtime < HELPER_SOURCE.stat().st_mtime:
        HELPER.parent.mkdir(parents=True, exist_ok=True)
        built = run(["swiftc", "-O", str(HELPER_SOURCE), "-o", str(HELPER)])
        if built.returncode != 0:
            raise Failed(f"could not build the system helper: {built.stderr.strip()[-400:]}")
    answer = run([str(HELPER), *args], timeout=30)
    if args[0] in REAL_INPUT:
        OWN_INPUT["at"] = time.monotonic()
    value = json.loads(answer.stdout.strip().splitlines()[-1]) if answer.stdout.strip() else None
    if answer.returncode != 0:
        raise Failed((value or {}).get("error") or answer.stderr.strip() or "the system helper failed")
    return value


def front() -> dict:
    return helper("front")


class FrontWatch:

    def __init__(self) -> None:
        helper("front")
        self.events: list[dict] = []
        self.proc = subprocess.Popen(
            [str(HELPER), "watch-front"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True
        )
        assert self.proc.stdout
        first = json.loads(self.proc.stdout.readline())
        self.before = first["front"]
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _read(self) -> None:
        assert self.proc.stdout
        for line in self.proc.stdout:
            self.events.append(json.loads(line))

    def activated(self, pid: int) -> bool:
        return any(event.get("pid") == pid for event in self.events)

    def stop(self) -> None:
        if self.proc.poll() is None:
            assert self.proc.stdin
            self.proc.stdin.close()
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()


class UnifiedLog:

    def __init__(self, path: Path, predicate: str) -> None:
        self.path = path
        self.predicate = predicate
        self.since = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(time.time() - 1))
        self.missed: list[str] = []
        self.out = open(path, "w")
        self.proc = subprocess.Popen(
            ["log", "stream", "--style", "ndjson", "--level", "debug", "--predicate", predicate],
            stdout=self.out,
            stderr=subprocess.STDOUT,
        )
        wait_until("the log stream starts", lambda: path.stat().st_size > 0, 15)

    @staticmethod
    def _messages(text: str) -> list[str]:
        found = []
        for line in text.splitlines():
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(event, dict) and "eventMessage" in event:
                found.append(event["eventMessage"])
        return found

    def messages(self) -> list[str]:
        return self._messages(self.path.read_text(errors="replace"))

    def persisted(self) -> list[str]:
        shown = run(["log", "show", "--style", "ndjson", "--info", "--debug",
                     "--start", self.since, "--predicate", self.predicate], timeout=60)
        if shown.returncode != 0:
            raise Failed(f"unknown: the log could not be read back ({shown.stderr.strip()[-200:]})")
        return self._messages(shown.stdout)

    def matching(self, pattern: str) -> list[str]:
        rx = re.compile(pattern)
        return [m for m in self.messages() if rx.search(m)]

    def settled(self, pattern: str) -> list[str]:
        rx = re.compile(pattern)
        streamed = [m for m in self.messages() if rx.search(m)]
        kept = [m for m in self.persisted() if rx.search(m)]
        extra = [m for m in kept if m not in streamed]
        self.missed += [m for m in extra if m not in self.missed]
        return streamed + extra

    def wait_for(self, what: str, pattern: str, timeout: float) -> str:
        try:
            return wait_until(what, lambda: (self.matching(pattern) or [None])[0], timeout, every=0.25)
        except Failed:
            found = self.settled(pattern)
            if found:
                return found[0]
            raise

    def reaches(self, what: str, pattern: str, count: int, timeout: float) -> bool:
        try:
            wait_until(what, lambda: len(self.matching(pattern)) >= count, timeout, every=0.25)
            return True
        except Failed:
            return len(self.settled(pattern)) >= count

    def stop(self) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        if not self.out.closed:
            self.out.close()


def notification_log(scratch: Scratch, bundle: str) -> UnifiedLog:
    predicate = (
        f'(eventMessage CONTAINS "{bundle}") '
        'AND (process == "usernoted" OR process == "NotificationCenter" OR process == "guac" '
        'OR subsystem BEGINSWITH "com.apple.UserNotifications")'
    )
    log = UnifiedLog(scratch.root / "unified.log", predicate)
    scratch.track(log.stop)
    scratch.logs.append(log)
    return log


def codesign(app: Path) -> dict:
    verify = run(["codesign", "--verify", "--strict", str(app)])
    shown = run(["codesign", "-dv", str(app)])
    identifier = re.search(r"^Identifier=(.+)$", shown.stderr, re.M)
    return {
        "valid": verify.returncode == 0,
        "identifier": identifier.group(1) if identifier else None,
        "why": verify.stderr.strip() or None,
    }


AWAY = 5.0

INPUT_SLACK = 0.5

REAL_INPUT = frozenset({"click-icon", "escape"})

OWN_INPUT = {"at": -math.inf}


def idle_seconds() -> float:
    return float(helper("idle")["seconds"])


def person_away(what: str) -> None:
    idle = idle_seconds()
    since_own = time.monotonic() - OWN_INPUT["at"]
    if idle < AWAY and idle < since_own - INPUT_SLACK:
        raise Deferred(f"{what}: someone is using the Mac; try again later")


def screen_step(what: str) -> None:
    access = helper("access")
    if not access["accessibility"]:
        raise Failed(
            f"{what}: Accessibility is not allowed for the app that runs this command "
            "(System Settings, Privacy & Security, Accessibility)"
        )
    if access["locked"]:
        run(["caffeinate", "-u", "-t", "2"])
        try:
            wait_until("the lock screen lowers once the display wakes", lambda: not helper("access")["locked"], 10, every=0.5)
        except Failed:
            raise Failed(f"{what}: the screen is locked, and only a person can unlock it") from None
    person_away(what)



def evidence_number(value: Any) -> bool:
    return type(value) in (int, float) and abs(value) <= 2**64 - 1 and math.isfinite(value)


def evidence_rows(rows: Any, phases: frozenset[str], clock: str) -> list[dict]:
    if not isinstance(rows, list):
        return []
    kept = []
    for row in rows[:NATIVE_ROWS if clock == "us" else PAGE_ROWS]:
        if not isinstance(row, dict) or type(row.get("phase")) is not str or row["phase"] not in phases:
            continue
        if type(row.get("id")) is not int or not 0 <= row["id"] <= 2**64 - 1:
            continue
        if not (type(row.get("value")) is bool or evidence_number(row.get("value"))):
            continue
        if not evidence_number(row.get(clock)) or row[clock] < 0:
            continue
        clean = {key: row[key] for key in ("phase", "id", "value", clock)}
        if clock == "us":
            if type(row.get("main")) is not bool:
                continue
            clean["main"] = row["main"]
        kept.append(clean)
    return kept


def diagnostic(path: Path, **request: Any) -> Any:
    try:
        deadline = time.monotonic() + DIAGNOSTIC_TIMEOUT
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
            sock.settimeout(DIAGNOSTIC_TIMEOUT)
            sock.connect(str(path))
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return None
            sock.settimeout(remaining)
            sock.sendall(json.dumps({"id": 1, **request}).encode() + b"\n")
            data = bytearray()
            while b"\n" not in data and len(data) < EVIDENCE_BYTES:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    return None
                sock.settimeout(remaining)
                chunk = sock.recv(min(4096, EVIDENCE_BYTES - len(data)))
                if not chunk:
                    return None
                data.extend(chunk)
            if b"\n" in data:
                reply = json.loads(data.split(b"\n", 1)[0])
                if isinstance(reply, dict) and reply.get("id") == 1 and reply.get("ok") is True:
                    return reply.get("value")
    except Exception:
        pass
    return None


def diagnostic_snapshot(path: Path) -> dict:
    snapshot: dict = {}
    state = diagnostic(path, op="state")
    if isinstance(state, dict):
        windows = state.get("windows")
        clean: dict = {"windows": {}}
        if isinstance(windows, list):
            for window in windows:
                if isinstance(window, dict) and window.get("label") in ("main", "menubar"):
                    clean["windows"][window["label"]] = {
                        key: window[key] for key in ("visible", "focused", "minimized")
                        if type(window.get(key)) is bool
                    }
        menubar = state.get("menubar")
        if isinstance(menubar, dict):
            clean["menubar"] = {key: menubar[key] for key in ("attached", "panel", "panelVisible")
                                if type(menubar.get(key)) is bool}
        snapshot["state"] = clean
    page = diagnostic(path, op="page", window="main", call={"do": "diagnostic"})
    if isinstance(page, dict):
        snapshot["page"] = {"rows": evidence_rows(page.get("rows"), PAGE_PHASES, "ms")}
        if evidence_number(page.get("ms")) and page["ms"] >= 0:
            snapshot["page"]["ms"] = page["ms"]
    return snapshot


class Scratch:

    def __init__(self) -> None:
        self.root = Path(tempfile.mkdtemp(prefix="guaca-probe-"))
        self.home = self.root / "home"
        self.channel = self.root / "run"
        self.host = self.root / "host"
        for dir_ in (self.home, self.channel, self.host):
            dir_.mkdir(mode=0o700)
        self.apps = APPS / self.root.name
        self.apps.mkdir(parents=True)
        self.started: list[Callable[[], None]] = []
        self.logs: list[UnifiedLog] = []
        self.snapshots: list[dict] = []
        self.launches: list[bool | None] = []
        self.presses: list[dict] = []

    def trace_path(self, launch: int) -> Path:
        return self.channel / ("native.jsonl" if launch == 1 else f"native-{launch}.jsonl")

    def traces(self) -> list[tuple[bool, list[dict]]]:
        traces = []
        for launch in range(1, max(len(self.launches), 1) + 1):
            trace = self.trace_path(launch)
            try:
                if not trace.is_file():
                    traces.append((False, []))
                    continue
                with trace.open("rb") as source:
                    data = source.read(NATIVE_BYTES)
            except OSError:
                traces.append((False, []))
                continue
            rows = []
            for line in data.splitlines()[:NATIVE_ROWS]:
                try:
                    rows.append(json.loads(line))
                except (ValueError, UnicodeError, RecursionError):
                    pass
            traces.append((True, evidence_rows(rows, NATIVE_PHASES, "us")))
        return traces

    def press(self, what: str) -> dict:
        entry = {"press": what, "launch": len(self.launches),
                 "after": sum(len(rows) for _, rows in self.traces()), "missed": False}
        self.presses.append(entry)
        return entry

    def retain_failure(self) -> Path | None:
        directory = FAILURES / uuid.uuid4().hex
        try:
            traces = self.traces()
            native = {"readable": any(readable for readable, _ in traces),
                      "rows": [row for _, rows in traces for row in rows][:NATIVE_ROWS]}
            healthy = [reported is not False and readable and bool(rows) and rows[0]["phase"] == "trace.opened"
                       for reported, (readable, rows) in zip(self.launches, traces)]
            health = [{"launch": launch, "health": "healthy" if ok else "writer failed",
                       "events": sum(row["phase"] != "trace.opened" for row in rows)}
                      for launch, (ok, (_, rows)) in enumerate(zip(healthy, traces), 1)]
            presses = []
            for index, press in enumerate(self.presses):
                if not press["missed"]:
                    continue
                end = self.presses[index + 1]["after"] if index + 1 < len(self.presses) else None
                phases = {row["phase"] for row in native["rows"][press["after"]:end]}
                if not (0 < press["launch"] <= len(healthy) and healthy[press["launch"] - 1]):
                    attribution = "the native trace writer failed, so the press cannot be attributed"
                elif "notification.response" not in phases:
                    attribution = "no native response arrived"
                elif "notification.clicked" not in phases:
                    attribution = "a native response arrived and no click was emitted"
                else:
                    attribution = "a click was emitted and the page did not act"
                presses.append({"press": press["press"], "attribution": attribution})
            FAILURES.mkdir(parents=True, exist_ok=True, mode=0o700)
            FAILURES.chmod(0o700)
            directory.mkdir(mode=0o700)
            fd = os.open(directory / "evidence.json", os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, "w") as target:
                target.write(json.dumps({"native": native, "traces": health, "presses": presses,
                                         "snapshots": self.snapshots},
                                        indent=2, allow_nan=False) + "\n")
            return directory
        except Exception:
            return None

    def track(self, stop: Callable[[], None]) -> None:
        self.started.append(stop)

    def stop_all(self) -> None:
        while self.started:
            try:
                self.started.pop()()
            except Exception as error:
                print(f"while cleaning up: {error!r}", file=sys.stderr)

    def close(self) -> None:
        self.stop_all()
        shutil.rmtree(self.root, ignore_errors=True)
        shutil.rmtree(self.apps, ignore_errors=True)


def free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


class StandIn:

    ASK = "PROBE-ASK"
    REPLY = "Probe test run finished.\nNothing here needs you; this is a scripted reply."
    QUESTION = "Probe test question: which one?"
    ANSWER = "First"

    def __init__(self, scratch: Scratch) -> None:
        from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

        stand_in = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_: Any) -> None:
                pass

            def do_POST(self) -> None:
                body = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))) or b"{}")
                frames = stand_in.answer(body.get("messages", []))
                self.send_response(200)
                self.send_header("content-type", "text/event-stream")
                self.end_headers()
                for frame in frames:
                    self.wfile.write(f"data: {json.dumps(frame)}\n\n".encode())
                self.wfile.write(b"data: [DONE]\n\n")

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.base = f"http://127.0.0.1:{self.server.server_address[1]}/v1"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        scratch.track(self.server.shutdown)

    def answer(self, messages: list[dict]) -> list[dict]:
        asked = next((m for m in reversed(messages) if m.get("role") == "user"), {})
        text = asked.get("content") or ""
        if isinstance(text, list):
            text = " ".join(part.get("text", "") for part in text if isinstance(part, dict))
        answered = any(m.get("role") == "tool" for m in messages[messages.index(asked) + 1:]) if asked else False
        usage = {"prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120}
        if self.ASK in text and not answered:
            call = {"index": 0, "id": "call_ask", "type": "function",
                    "function": {"name": "ask_operator",
                                 "arguments": json.dumps({"question": self.QUESTION, "options": [self.ANSWER, "Second"]})}}
            return [{"choices": [{"index": 0, "delta": {"tool_calls": [call]}}]},
                    {"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}], "usage": usage}]
        return [{"choices": [{"index": 0, "delta": {"content": self.REPLY}}]},
                {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "usage": usage}]


class Host:

    def __init__(self, scratch: Scratch, minute_ms: int | None = None) -> None:
        self.scratch = scratch
        self.minute_ms = minute_ms
        self.port = free_port()
        self.origin = f"http://127.0.0.1:{self.port}"
        self.token = secrets.token_hex(32)
        self.db = scratch.host / "data/guac.db"
        self.proc: subprocess.Popen | None = None
        self.waiting: list[str] = []

    def start(self) -> None:
        env = {
            "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
            "HOME": str(self.scratch.home),
            "GUACA_ROOT": str(self.scratch.host),
            "GUACA_BIND": f"127.0.0.1:{self.port}",
            "GUACA_TOKEN": self.token,
            "GUAC_LOG": "guac=info,warn",
        }
        if self.minute_ms:
            env["GUACA_PROBE_MINUTE_MS"] = str(self.minute_ms)
        log = open(self.scratch.root / "host.log", "ab")
        self.proc = subprocess.Popen(
            [str(GUACAD)], env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True
        )
        self.scratch.track(self.stop)
        wait_until("the scratch Host answers", self._healthy, 60)

    def _healthy(self) -> bool:
        assert self.proc
        if self.proc.poll() is not None:
            raise Failed(f"the scratch Host exited with status {self.proc.returncode}")
        with urllib.request.urlopen(f"{self.origin}/health", timeout=1) as answer:
            return answer.status == 200

    def stop(self) -> None:
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=20)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()

    def seed(self, delivery: dict | None = None, count: int = 1) -> str | None:
        self.start()
        self.stop()
        seeded = run([sys.executable, str(SEED), "--db", str(self.db)])
        if seeded.returncode != 0:
            raise Failed(f"could not seed the workspace: {seeded.stderr.strip()}")
        self.waiting = []
        for _ in range(count if delivery is not None else 0):
            assert delivery is not None
            delivery_id = str(uuid.uuid4())
            self.waiting.append(delivery_id)
            now = int(time.time() * 1000)
            with sqlite3.connect(self.db) as conn:
                agent = None
                target: dict = {"kind": "forYou"}
                if delivery.get("agent"):
                    agent = conn.execute(
                        "SELECT id FROM agents WHERE name=?", (delivery["agent"],)
                    ).fetchone()[0]
                    target = {"kind": "channel", "agentId": agent}
                conn.execute(
                    "INSERT INTO outbox (id,kind,destination,dedup_key,group_id,agent_id,title,body,"
                    "target,next_attempt_at,created_at) VALUES (?,'needs_you','clients',?,?,?,?,?,?,?,?)",
                    (delivery_id, f"probe:{delivery_id}", GROUP, agent, DELIVERY_TITLE,
                     DELIVERY_BODY, json.dumps(target), now, now),
                )
        self.start()
        return self.waiting[0] if self.waiting else None

    def delivery(self, delivery_id: str) -> dict:
        with sqlite3.connect(f"file:{self.db}?mode=ro", uri=True) as conn:
            status, offered, delivered = conn.execute(
                "SELECT status, offered_at, delivered_at FROM outbox WHERE id=?", (delivery_id,)
            ).fetchone()
        return {"status": status, "offered": offered is not None, "delivered": delivered is not None}

    def agent_id(self, name: str) -> str:
        with sqlite3.connect(f"file:{self.db}?mode=ro", uri=True) as conn:
            return conn.execute("SELECT id FROM agents WHERE name=?", (name,)).fetchone()[0]

    def call(self, name: str, **args: Any) -> Any:
        request = urllib.request.Request(
            f"{self.origin}/v1/call",
            data=json.dumps({"name": name, "args": args}).encode(),
            headers={"content-type": "application/json", "authorization": f"Bearer {self.token}"},
            method="POST",
        )
        with urllib.request.urlopen(request, timeout=30) as answer:
            return json.loads(answer.read())["ok"]

    def use(self, model: StandIn) -> None:
        self.call("update_settings", patch={"provider": "compatible", "baseUrl": model.base,
                                            "apiKey": "probe-stand-in", "defaultModel": "probe/stand-in"})

    def send(self, agent: str, text: str) -> None:
        self.call("send_message", agentId=self.agent_id(agent), text=text)

    def rows(self, kind: str | None = None) -> list[dict]:
        with sqlite3.connect(f"file:{self.db}?mode=ro", uri=True) as conn:
            conn.row_factory = sqlite3.Row
            found = [dict(row) for row in conn.execute(
                "SELECT id, kind, destination, status, title, body, target, last_error, next_attempt_at "
                "FROM outbox ORDER BY created_at")]
        return [row for row in found if kind is None or row["kind"] == kind]


class Client:

    def __init__(self, path: Path) -> None:
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(20)
        self.sock.connect(str(path))
        self.file = self.sock.makefile("rw")
        self.next = 1

    def ask(self, op: str, **fields: Any) -> Any:
        request = {"id": self.next, "op": op, **fields}
        self.next += 1
        self.file.write(json.dumps(request) + "\n")
        self.file.flush()
        line = self.file.readline()
        if not line:
            raise Failed(f"the app closed the channel during {op}")
        reply = json.loads(line)
        if not reply["ok"]:
            raise Refused(f"{op}: {reply['error']}")
        return reply.get("value")

    def page(self, window: str, **call: Any) -> Any:
        return self.ask("page", window=window, call=call)

    def close(self) -> None:
        try:
            self.sock.close()
        except OSError:
            pass


class App:

    def __init__(self, scratch: Scratch, bundle: str = IDENTIFIER, folder: str = "") -> None:
        self.scratch = scratch
        self.bundle = bundle
        self.path = scratch.apps / folder / f"{APP_NAME}.app"
        shutil.copytree(BUILT_APP, self.path, symlinks=True)
        plist = self.path / "Contents/Info.plist"
        if bundle != IDENTIFIER:
            run(["plutil", "-replace", "CFBundleIdentifier", "-string", bundle, str(plist)], check=True)
        signed = run(["codesign", "--force", "--deep", "--sign", "-", "--identifier", bundle, str(self.path)])
        if signed.returncode != 0:
            raise Failed(f"could not sign the test build: {signed.stderr.strip()}")
        self.socket = scratch.channel / "probe.sock"
        self.client: Client | None = None
        self.pid: int | None = None
        scratch.track(self.forget)
        scratch.track(self.quit)

    def launch(self, background: bool = True, hidden: bool = True) -> Client:
        if self.socket.exists():
            self.socket.unlink()
        args = ["open", "-n"]
        if background:
            args.append("-g")
        if hidden:
            args.append("-j")
        env = {
            "HOME": str(self.scratch.home),
            "CFFIXED_USER_HOME": str(self.scratch.home),
            "GUACA_PROBE_SOCKET": str(self.socket),
            "GUACA_PROBE_TRACE": str(self.scratch.trace_path(len(self.scratch.launches) + 1)),
            "GUAC_LOG": "guac=info,warn",
        }
        for key, value in env.items():
            args += ["--env", f"{key}={value}"]
        log = str(self.scratch.root / "app.log")
        args += ["--stdout", log, "--stderr", log, "-a", str(self.path)]
        self.scratch.launches.append(None)
        opened = run(args)
        if opened.returncode != 0:
            raise Failed(f"could not launch the test build: {opened.stderr.strip()}")
        wait_until("the probe channel opens", self.socket.exists, 30)
        self.client = Client(self.socket)
        hello = self.client.ask("hello")
        self.scratch.launches[-1] = hello.get("trace") if type(hello.get("trace")) is bool else None
        self.pid = hello["pid"]
        return self.client

    def alive(self) -> bool:
        if self.pid is None:
            return False
        try:
            os.kill(self.pid, 0)
        except ProcessLookupError:
            return False
        return True

    def state(self) -> dict:
        assert self.client
        return self.client.ask("state")

    def quit(self) -> None:
        if self.client:
            try:
                snapshot = diagnostic_snapshot(self.socket)
                self.scratch.snapshots.append(snapshot)
                self.scratch.snapshots = self.scratch.snapshots[-EVIDENCE_SNAPSHOTS:]
            except Exception:
                pass
        if self.client and self.alive():
            try:
                self.client.ask("quit")
            except (Failed, OSError):
                pass
        if self.client:
            self.client.close()
            self.client = None
        try:
            wait_until("the test build quits", lambda: not self.alive(), 15)
        except Failed:
            if self.pid:
                os.kill(self.pid, 9)
        self.pid = None
        run(["pkill", "-9", "-f", str(self.path / "Contents/MacOS/")])

    def forget(self) -> None:
        run([LSREGISTER, "-u", str(self.path)])



def connect(app: App, host: Host) -> None:
    assert app.client
    client = app.client
    wait_until("the window's page answers", lambda: client.page("main", do="ready")["loaded"] == "complete", 30)
    client.page("main", do="attach", origin=host.origin, token=host.token)
    wait_until(
        "the window shows the seeded workspace",
        lambda: sorted(client.page("main", do="view")["agents"]) == sorted(AGENTS),
        60,
        every=0.25,
    )


def open_notifications(client: Client) -> str:
    client.page("main", do="click", role="button", name="App settings")
    wait_until(
        "Settings opens",
        lambda: client.page("main", do="find", role="tab", name="Notifications")["count"] == 1,
        10,
    )
    client.page("main", do="click", role="tab", name="Notifications")
    return wait_until(
        "the Notifications section shows the test button",
        lambda: client.page("main", do="find", role="button", name="Send a test notification")["count"],
        10,
    )


def pane_text(client: Client) -> str:
    return client.page("main", do="text")


def macos_line(text: str) -> str | None:
    found = re.search(r"(macOS [^.]*\.|This copy of Guaca is not running[^.]*\.)", text)
    return found.group(1) if found else None


class Record:

    def __init__(self, scenario: str, index: int, secrets_: list[str]) -> None:
        self.scenario = scenario
        self.index = index
        self.steps: list[dict] = []
        self.secrets = secrets_
        self.deferred = False

    def check(self, step: str, holds: bool, detail: Any = None, stop: bool = False) -> bool:
        entry: dict = {"step": step, "pass": bool(holds)}
        if detail is not None:
            entry["detail"] = json.loads(redact(json.dumps(detail), self.secrets))
        self.steps.append(entry)
        if stop and not holds:
            raise Failed(step)
        return bool(holds)

    def observe(self, step: str, value: Any) -> None:
        self.steps.append({"step": step, "observed": json.loads(redact(json.dumps(value), self.secrets))})

    @property
    def passed(self) -> bool:
        return not self.deferred and all(step.get("pass", True) for step in self.steps)

    @property
    def outcome(self) -> str:
        return "not run" if self.deferred else "pass" if self.passed else "fail"

    def result(self) -> dict:
        return {"scenario": self.scenario, "run": self.index, "pass": self.passed,
                "deferred": self.deferred, "outcome": self.outcome, "steps": self.steps}



def background_launch(record: Record, app: App, label: str) -> None:
    person_away("launching where the app may take the front")
    watch = FrontWatch()
    try:
        app.launch(background=True, hidden=False)
        try:
            wait_until("the test build exits", lambda: not app.alive(), 10)
            exited = True
        except Failed:
            exited = False
        took = watch.activated(app.pid or -1)
        focused = [w["label"] for w in app.state()["windows"] if w["focused"]]
        hand_back(app, watch.before)
        record.check(f"{label}: still running after ten seconds", not exited)
        record.check(f"{label}: did not become the frontmost app", not took,
                     {"activations": [e for e in watch.events if e.get("pid") == app.pid]})
        record.check(f"{label}: no window took the keyboard", not focused, {"focused": focused})
        record.check(f"{label}: the app that was in front is in front again",
                     front().get("pid") == watch.before.get("pid"))
    finally:
        watch.stop()
        app.quit()


def scenario_launch(record: Record, scratch: Scratch) -> None:
    app = App(scratch, fresh_bundle(), "plain")
    signed = codesign(app.path)
    record.check("the test build is signed as its own bundle", signed["valid"], signed, stop=True)
    try:
        background_launch(record, app, "first background launch of a new bundle")
        background_launch(record, app, "second background launch of the same bundle")

        person_away("launching in the foreground")
        watch = FrontWatch()
        try:
            app.launch(background=False, hidden=False)
            try:
                wait_until("the test build comes to the front", lambda: watch.activated(app.pid or -1), 10)
                took = True
            except Failed:
                took = False
            hand_back(app, watch.before)
            record.check("a normal launch takes the front", took)
            after = front()
            record.check("a normal launch: the app that was in front is in front again",
                         after.get("pid") == watch.before.get("pid"),
                         {"before": watch.before.get("bundle"), "after": after.get("bundle"),
                          "activations": [e.get("bundle") for e in watch.events]})
        finally:
            watch.stop()
            app.quit()
    finally:
        app.forget()


def hand_back(app: App, before: dict) -> None:
    if app.client and front().get("pid") == app.pid:
        app.client.ask("hide")
        wait_until("the app that was in front is in front again",
                   lambda: front().get("pid") != app.pid, 10)


def holds_for(what: str, check: Callable[[], Any], seconds: float) -> bool:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if not check():
            return False
        time.sleep(0.25)
    return True


def asked(bundle: str) -> str:
    return rf"Sending request for permission for {re.escape(bundle)} "


def prompt(bundle: str, verb: str = "Presenting") -> str:
    return (rf'{verb} <NotificationRecord app:"com\.apple\.notificationcenter\.askpermissions"'
            rf'[^>]*req:"{re.escape(bundle)}"')


def banner(bundle: str) -> str:
    return rf'Presenting <NotificationRecord app:"{re.escape(bundle)}"[^>]*> as banner'


def held_back(log: UnifiedLog, bundle: str) -> str | None:
    pattern = (rf"interruptionSuppression: silence;.*resolutionReason: ([^;]+);"
               rf".*bundleIdentifier: {re.escape(bundle)};")
    for message in log.settled(pattern):
        found = re.search(pattern, message)
        if found:
            return found.group(1)
    return None


def drawn(record: Record, log: UnifiedLog, bundle: str) -> None:
    reason = held_back(log, bundle)
    record.check("macOS does not hold the banner back", reason is None,
                 {"reason": reason} if reason else None, stop=True)


def fresh_bundle() -> str:
    return f"{IDENTIFIER}.fresh{uuid.uuid4().hex[:8]}"


def notification_center() -> int:
    found = run(["pgrep", "-x", "NotificationCenter"]).stdout.split()
    if not found:
        raise Failed("Notification Center is not running")
    return int(found[0])


def press_when_shown(what: str, *press: str) -> Any:
    center = str(notification_center())
    last = ""

    def attempt() -> Any:
        nonlocal last
        try:
            return helper(press[0], center, *press[1:])
        except Failed as why:
            last = str(why)
            return None

    try:
        return wait_until(what, attempt, 5, every=0.2)
    except Failed:
        raise Failed(f"{what}: {last}; what names {APP_NAME}: {helper('describe', center, APP_NAME)}") from None


def scenario_permission(record: Record, scratch: Scratch) -> None:
    bundle = fresh_bundle()
    host = Host(scratch)
    record.secrets.append(host.token)
    delivery = host.seed({})
    assert delivery
    app = App(scratch, bundle)
    signed = codesign(app.path)
    record.check("signed as a bundle macOS has never seen", signed["identifier"] == bundle, signed, stop=True)
    log = notification_log(scratch, bundle)
    try:
        app.launch()
        reported = app.state()["notifications"]
        record.check("the app reports that macOS has not asked", reported == "notAsked", reported, stop=True)
        wait_until("the Host offers the delivery", lambda: host.delivery(delivery)["offered"], 15)
        record.check("a delivery is waiting for a window", host.delivery(delivery)["status"] == "pending")
        connect(app, host)
        log.wait_for("the app asks macOS", asked(bundle), 20)
        record.check("connecting with a delivery waiting asks macOS", True)
        log.wait_for("macOS shows its question", prompt(bundle), 10)
        record.check("macOS shows its question", True)
        record.check(
            "nothing is claimed while the question is unanswered",
            holds_for("unclaimed", lambda: host.delivery(delivery)["status"] == "pending", 3),
            host.delivery(delivery),
        )
        record.check("no banner is presented", not log.settled(banner(bundle)))
        record.observe("what the app reports while macOS is asking", app.state()["notifications"])
    finally:
        app.quit()
        try:
            log.wait_for("the question goes when the app quits", prompt(bundle, "Removing displayed"), 10)
            gone = True
        except Failed:
            gone = False
        record.check("macOS takes its question down when the app quits", gone)
        log.stop()
        host.stop()
        app.forget()


BRANCHES = {
    "allowed": "claimed and presented as a banner",
    "notAsked": "macOS asked, nothing claimed",
}


def scenario_notification(record: Record, scratch: Scratch) -> None:
    host = Host(scratch)
    record.secrets.append(host.token)
    delivery = host.seed({})
    assert delivery
    app = App(scratch)
    log = notification_log(scratch, IDENTIFIER)
    try:
        app.launch()
        assert app.client
        client = app.client
        permission = app.state()["notifications"]
        record.observe("what macOS allows the test bundle", permission)
        branch = BRANCHES.get(permission, "nothing claimed, nothing presented")
        record.observe("branch exercised", branch)
        print(f"notification: macOS says {permission}, so this run checks: {branch}", file=sys.stderr)
        connect(app, host)
        if permission == "allowed":
            wait_until("the window claims the delivery", lambda: host.delivery(delivery)["delivered"], 15)
            record.check("the waiting delivery is claimed", True)
            log.wait_for("macOS presents it", banner(IDENTIFIER), 10)
            record.check("macOS presents it as a banner", True)
            drawn(record, log, IDENTIFIER)
        elif permission == "notAsked":
            log.wait_for("the app asks macOS", asked(IDENTIFIER), 20)
            record.check("the app asks macOS before claiming", True)
            record.check("nothing is claimed while unanswered",
                         holds_for("unclaimed", lambda: host.delivery(delivery)["status"] == "pending", 3))
        else:
            record.check("nothing is claimed",
                         holds_for("unclaimed", lambda: host.delivery(delivery)["status"] == "pending", 5))
            record.check("macOS is not asked again", not log.settled(asked(IDENTIFIER)))

        open_notifications(client)
        shown = app.state()["notifications"]
        line = PANE.get(shown, "")
        try:
            wait_until("the pane reads macOS's answer", lambda: line and line in pane_text(client), 5)
            says = True
        except Failed:
            says = False
        record.check("the pane says what macOS says", says,
                     {"permission": shown, "pane": macos_line(pane_text(client))})
        banners = len(log.settled(banner(IDENTIFIER)))
        client.page("main", do="click", role="button", name="Send a test notification")
        if shown == "allowed":
            wait_until("the pane says it was sent", lambda: SENT in pane_text(client), 10)
            log.reaches("macOS presents the test", banner(IDENTIFIER), banners + 1, 10)
            record.check("the test is presented as a banner and the pane says so",
                         len(log.settled(banner(IDENTIFIER))) == banners + 1)
        else:
            record.check("the pane never says it was sent",
                         holds_for("not sent", lambda: SENT not in pane_text(client), 3))
            record.check("macOS presents nothing", len(log.settled(banner(IDENTIFIER))) == banners)
        record.observe("banners presented", len(log.settled(banner(IDENTIFIER))))
    finally:
        app.quit()
        log.stop()
        host.stop()
        app.forget()


def scenario_panel(record: Record, scratch: Scratch) -> None:
    host = Host(scratch)
    record.secrets.append(host.token)
    host.seed()
    app = App(scratch)
    try:
        client = app.launch()
        connect(app, host)
        wait_until("the panel is made for the window's host", lambda: app.state()["menubar"]["attached"], 20)
        record.check("the panel knows the window's host", True)
        client.ask("close", window="main")
        wait_until("the window closes", lambda: not visible(app, "main"), 10)
        record.check("closing the window hides it and keeps the app running", app.alive())
        person_away("opening the panel")
        before = front()
        client.ask("clickIcon")
        wait_until("the panel opens", lambda: app.state()["menubar"]["panelVisible"], 10)
        record.check("a click on the icon opens the panel", True)
        listed = wait_until(
            "the panel lists the group",
            lambda: all(name in client.page("menubar", do="text") for name in AGENTS),
            10,
        )
        record.check("the panel lists every agent in the group", listed, {"agents": AGENTS})
        try:
            wait_until("the test build comes forward", lambda: front().get("pid") == app.pid, 3)
            forward = True
        except Failed:
            forward = False
        record.observe("opening the panel brings the test build forward", forward)
        client.ask("clickIcon")
        wait_until("the panel closes", lambda: not app.state()["menubar"]["panelVisible"], 10)
        record.check("a second click closes it", True)
        try:
            wait_until("the keyboard goes back", lambda: front().get("pid") == before.get("pid"), 5)
            back = True
        except Failed:
            back = False
        record.check("the app that had the keyboard has it again", back,
                     {"before": before.get("bundle"), "after": front().get("bundle")})
    finally:
        app.quit()
        host.stop()
        app.forget()


def visible(app: App, label: str) -> bool:
    return any(w["label"] == label and w["visible"] for w in app.state()["windows"])


class Prompted:

    def __init__(self, record: Record, scratch: Scratch, button: str, count: int = 1, busy: int = 0,
                 minute_ms: int | None = None) -> None:
        self.record = record
        self.minute_ms = minute_ms
        self.scratch = scratch
        self.button = button
        self.count = count
        self.busy = busy
        self.app: App | None = None
        self.host: Host | None = None
        self.log: UnifiedLog | None = None
        self.delivery = ""

    def __enter__(self) -> "Prompted":
        try:
            self._answer()
        except BaseException:
            self.__exit__(None, None, None)
            raise
        return self

    def _answer(self) -> None:
        screen_step(f"pressing {self.button}")
        bundle = fresh_bundle()
        self.host = Host(self.scratch, self.minute_ms)
        self.record.secrets.append(self.host.token)
        delivery = self.host.seed({"agent": "Critic"}, self.count)
        assert delivery
        self.delivery = delivery
        self.app = App(self.scratch, bundle)
        self.log = notification_log(self.scratch, bundle)
        self.app.launch()
        connect(self.app, self.host)
        self.log.wait_for("macOS shows its question", prompt(bundle), 20)
        self.record.check("macOS asks about the fresh bundle", True)
        person_away(f"pressing {self.button}")
        if self.busy:
            assert self.app.client
            self.app.client.ask("busy", ms=self.busy)
        pressed = press_when_shown(f"pressing {self.button}", "press", "AXButton", self.button, APP_NAME)
        self.record.check(f"pressed {self.button} on the question about {APP_NAME}", True, pressed)

    def __exit__(self, *_: Any) -> None:
        if self.app:
            self.app.quit()
        if self.log:
            self.log.stop()
        if self.host:
            self.host.stop()
        if self.app:
            self.app.forget()
        self.app = self.log = self.host = None


def scenario_allow(record: Record, scratch: Scratch) -> None:
    with Prompted(record, scratch, "Allow") as it:
        assert it.app and it.host and it.log
        host, app = it.host, it.app
        wait_until("the window claims the delivery", lambda: host.delivery(it.delivery)["delivered"], 15)
        record.check("the waiting delivery is claimed once the answer is yes", True)
        it.log.wait_for("macOS presents it", banner(app.bundle), 10)
        record.check("macOS presents it as a banner", True)
        drawn(record, it.log, app.bundle)
        record.check("the app reports allowed", app.state()["notifications"] == "allowed")
        assert app.client
        client = app.client
        open_notifications(client)
        wait_until("the pane says macOS allows it", lambda: PANE["allowed"] in pane_text(client), 5)
        record.check("the pane says macOS allows notifications", True)


def scenario_button(record: Record, scratch: Scratch) -> None:
    with Prompted(record, scratch, "Allow") as it:
        assert it.app and it.log and it.app.client
        app, client = it.app, it.app.client
        it.log.wait_for("macOS presents the waiting delivery", banner(app.bundle), 15)
        drawn(record, it.log, app.bundle)
        open_notifications(client)
        wait_until("the pane says macOS allows it", lambda: PANE["allowed"] in pane_text(client), 5)
        client.page("main", do="click", role="button", name="Send a test notification")
        shown = it.log.reaches("macOS presents the test", banner(app.bundle), 2, 10)
        record.check("the test button's banner is presented", shown,
                     None if shown else {"pane says sent": SENT in pane_text(client)})


def scenario_busy(record: Record, scratch: Scratch) -> None:
    with Prompted(record, scratch, "Allow", busy=10000) as it:
        assert it.app and it.host and it.log and it.app.client
        client, bundle = it.app.client, it.app.bundle
        critic = it.host.agent_id("Critic")
        it.log.wait_for("macOS presents the delivery", banner(bundle), 15)
        drawn(record, it.log, bundle)
        record.check("a delivery handed over while the main thread is busy is drawn", True)
        record.check("another conversation is open first", client.page("main", do="view")["selected"] != critic)
        screen_step("clicking the banner")
        pressed = scratch.press("clicking the banner")
        press_when_shown("clicking the banner", "press-group", DELIVERY_TITLE)
        try:
            wait_until("the window opens the conversation",
                       lambda: client.page("main", do="view")["selected"] == critic, 15)
        except Failed:
            pressed["missed"] = True
            raise
        record.check("its click opens the conversation", True)


def scenario_deny(record: Record, scratch: Scratch) -> None:
    with Prompted(record, scratch, "Don\u2019t Allow") as it:
        assert it.app and it.host and it.log and it.app.client
        host, app = it.host, it.app
        wait_until("the app reads the answer", lambda: app.state()["notifications"] == "denied", 10)
        record.check("the app reports denied", True)
        client = app.client
        open_notifications(client)
        try:
            wait_until("the pane reads macOS's answer", lambda: PANE["denied"] in pane_text(client), 5)
            says = True
        except Failed:
            says = False
        record.check("the pane says macOS has notifications turned off", says,
                     {"pane": macos_line(pane_text(client))})
        record.check("nothing is claimed",
                     holds_for("unclaimed", lambda: host.delivery(it.delivery)["status"] == "pending", 3))
        record.check("no banner is presented", not it.log.settled(banner(app.bundle)))


def scenario_click(record: Record, scratch: Scratch) -> None:
    with Prompted(record, scratch, "Allow") as it:
        assert it.app and it.host and it.log and it.app.client
        client = it.app.client
        critic = it.host.agent_id("Critic")
        it.log.wait_for("macOS presents the delivery", banner(it.app.bundle), 15)
        drawn(record, it.log, it.app.bundle)
        record.check("the delivery is on screen as a banner", True)
        record.check("another conversation is open first", client.page("main", do="view")["selected"] != critic)
        screen_step("clicking the banner")
        pressed = scratch.press("clicking the banner")
        press_when_shown("clicking the banner", "press-group", DELIVERY_TITLE)
        try:
            wait_until("the window opens the conversation",
                       lambda: client.page("main", do="view")["selected"] == critic, 10)
        except Failed:
            pressed["missed"] = True
            raise
        record.check("the click opens the conversation the delivery is about", True)


def scenario_away(record: Record, scratch: Scratch) -> None:
    with Prompted(record, scratch, "Allow", count=4) as it:
        assert it.app and it.host and it.log
        host, log, bundle = it.host, it.log, it.app.bundle
        wait_until("the window claims every waiting delivery",
                   lambda: all(host.delivery(d)["delivered"] for d in host.waiting), 15)
        record.check("all four waiting deliveries are claimed", True, {"waiting": len(host.waiting)})
        log.wait_for("macOS presents a banner", banner(bundle), 10)
        drawn(record, log, bundle)
        holds_for("one banner", lambda: len(log.matching(banner(bundle))) == 1, 3)
        presented = len(log.settled(banner(bundle)))
        record.check("they are presented as one banner", presented == 1, {"banners": presented})


def scenario_capture(record: Record, scratch: Scratch) -> None:
    out = Path(os.environ.get("GUACA_PROBE_CAPTURES", ROOT / "src-tauri/target/probe/captures"))
    if not helper("access")["screenCapture"]:
        raise Failed("capturing: Screen Recording is not allowed for the app that runs this command "
                     "(System Settings, Privacy & Security, Screen & System Audio Recording)")
    host = Host(scratch)
    record.secrets.append(host.token)
    host.seed()
    app = App(scratch)
    try:
        client = app.launch()
        connect(app, host)
        wait_until("the panel is made", lambda: app.state()["menubar"]["attached"], 20)
        person_away("capturing the test windows")
        client.ask("clickIcon")
        wait_until("the panel opens", lambda: app.state()["menubar"]["panelVisible"], 10)
        out.mkdir(parents=True, exist_ok=True)
        shots = helper("capture", str(app.pid), str(out / f"probe-{record.index}.png"))
        record.check("captured only the test build's windows", bool(shots["files"]), shots)
        client.ask("clickIcon")
    finally:
        app.quit()
        host.stop()
        app.forget()


MINUTE_MS = 6000

PRESENCE_MINUTE_MS = 30000


def ready(record: Record, scratch: Scratch, minute_ms: int | None = None) -> Prompted:
    it = Prompted(record, scratch, "Allow", minute_ms=minute_ms)
    it.__enter__()
    assert it.app and it.host and it.log
    it.log.wait_for("macOS presents the waiting delivery", banner(it.app.bundle), 15)
    it.host.use(StandIn(scratch))
    return it


def click_banner(record: Record, app: App, scope: str, what: str, give_back: bool = True) -> None:
    before = front()
    screen_step(what)
    pressed = app.scratch.press(what)
    press_when_shown(what, "press-group", scope)
    came = bool(wait_quietly(lambda: front().get("pid") == app.pid, 5))
    pressed["missed"] = not came
    record.check(f"{what}: the test build comes forward", came)
    if give_back:
        hand_back(app, before)


def focused(app: App) -> bool:
    return any(w["label"] == "main" and w["focused"] for w in app.state()["windows"])


def wait_quietly(check: Callable[[], Any], seconds: float) -> Any:
    try:
        return wait_until("x", check, seconds)
    except Failed:
        return None


def leave(app: App, how: str) -> None:
    if how == "hide":
        assert app.client
        app.client.ask("hide")
    else:
        run(["osascript", "-e", 'tell application "Finder" to activate'], check=True)
    wait_until("the test build is no longer in front", lambda: front().get("pid") != app.pid, 10)


def scenario_question(record: Record, scratch: Scratch) -> None:
    it = ready(record, scratch)
    try:
        assert it.app and it.host and it.log and it.app.client
        host, app, log, client = it.host, it.app, it.log, it.app.client
        researcher = host.agent_id("Researcher")
        shown = len(log.settled(banner(app.bundle)))
        host.send("Researcher", f"{StandIn.ASK}: ask me something")
        record.check("the question is presented as a banner",
                     log.reaches("macOS presents the question", banner(app.bundle), shown + 1, 20))
        drawn(record, log, app.bundle)
        record.check("its row is the question, claimed",
                     [r["status"] for r in host.rows("needs_you") if "question" in r["title"]] == ["delivered"])
        record.check("another conversation is open first", client.page("main", do="view")["selected"] != researcher)
        click_banner(record, app, "Researcher has a question", "clicking the question")
        wait_until("the window opens the conversation", lambda: client.page("main", do="view")["selected"] == researcher, 15)
        record.check("its click opens the conversation", True)
    finally:
        it.__exit__(None, None, None)


def finished_after_leaving(record: Record, scratch: Scratch, how: str) -> None:
    it = ready(record, scratch, PRESENCE_MINUTE_MS)
    try:
        assert it.app and it.host and it.log and it.app.client
        host, app, log, client = it.host, it.app, it.log, it.app.client
        scribe = host.agent_id("Scribe")
        click_banner(record, app, DELIVERY_TITLE, "clicking the first banner, to be at the window",
                     give_back=False)
        record.check("the window has focus", wait_quietly(lambda: focused(app), 5) is not None, stop=True)
        leave(app, how)
        shown = len(log.settled(banner(app.bundle)))
        host.send("Scribe", "Say something short; this is a probe test.")
        wait_until("the run finishes", lambda: host.rows("outcome"), 30)
        presented = log.reaches("macOS presents the finished run", banner(app.bundle), shown + 1,
                                14 * PRESENCE_MINUTE_MS / 1000 + 20)
        record.check(f"after {how}: the finished run is presented as a banner", presented,
                     None if presented else {"rows": [{k: r[k] for k in ("status", "last_error")} for r in host.rows("outcome")]})
        if not presented:
            return
        drawn(record, log, app.bundle)
        click_banner(record, app, StandIn.REPLY.splitlines()[0], "clicking the finished run")
        wait_until("the window opens the conversation", lambda: client.page("main", do="view")["selected"] == scribe, 15)
        record.check("its click opens the conversation", True)
    finally:
        it.__exit__(None, None, None)


def scenario_hidden(record: Record, scratch: Scratch) -> None:
    finished_after_leaving(record, scratch, "hide")


def scenario_switched(record: Record, scratch: Scratch) -> None:
    finished_after_leaving(record, scratch, "switch")


def scenario_fold(record: Record, scratch: Scratch) -> None:
    it = ready(record, scratch, MINUTE_MS)
    try:
        assert it.app and it.host and it.log and it.app.client
        host, app, log, client = it.host, it.app, it.log, it.app.client
        click_banner(record, app, DELIVERY_TITLE, "clicking the first banner, to be at the window")
        time.sleep(2 * MINUTE_MS / 1000 + 2)
        shown = len(log.settled(banner(app.bundle)))
        host.send("Scribe", "Say something short; this is a probe test.")
        host.send("Manager", "Say something short; this is a probe test.")
        wait_until("both runs finish", lambda: len(host.rows("outcome")) == 2, 30)
        record.check("the gathered banner is presented",
                     log.reaches("macOS presents the gathered item", banner(app.bundle), shown + 1,
                                 12 * MINUTE_MS / 1000 + 20), stop=True)
        drawn(record, log, app.bundle)
        click_banner(record, app, "2 finished", "clicking the gathered banner")
        wait_until("For you opens",
                   lambda: client.page("main", do="find", role="dialog", name="For you")["count"] == 1, 15)
        record.check("its click opens For you", True)
        presented = len(log.settled(banner(app.bundle)))
        record.check("both runs are one banner", presented == shown + 1, {"banners": presented - shown})
    finally:
        it.__exit__(None, None, None)


def scenario_testclick(record: Record, scratch: Scratch) -> None:
    with Prompted(record, scratch, "Allow") as it:
        assert it.app and it.log and it.app.client
        app, client = it.app, it.app.client
        it.log.wait_for("macOS presents the waiting delivery", banner(app.bundle), 15)
        open_notifications(client)
        wait_until("the pane says macOS allows it", lambda: PANE["allowed"] in pane_text(client), 5)
        client.page("main", do="click", role="button", name="Send a test notification")
        record.check("the test banner is presented", it.log.reaches("macOS presents the test", banner(app.bundle), 2, 10), stop=True)
        click_banner(record, app, TEST_BODY, "clicking the test banner")


class Listener:

    def __init__(self, scratch: Scratch) -> None:
        from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

        self.received: list[dict] = []
        listener = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_: Any) -> None:
                pass

            def do_POST(self) -> None:
                body = self.rfile.read(int(self.headers.get("content-length", 0))).decode(errors="replace")
                listener.received.append({"title": self.headers.get("Title"), "body": body,
                                          "delivery": self.headers.get("X-Guaca-Delivery"),
                                          "priority": self.headers.get("Priority")})
                self.send_response(200)
                self.end_headers()

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}/probe"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        scratch.track(self.server.shutdown)

    def titled(self, title: str) -> list[dict]:
        return [push for push in self.received if push["title"] == title]


def answer_the_question(host: Host) -> None:
    asked = wait_until("the question waits for an answer", lambda: host.call("pending_approvals"), 10)
    host.call("answer_question", id=asked[0]["id"], answer=StandIn.ANSWER)


def scenario_push(record: Record, scratch: Scratch) -> None:
    host = Host(scratch, MINUTE_MS)
    record.secrets.append(host.token)
    host.seed()
    host.use(StandIn(scratch))
    phone = Listener(scratch)
    host.call("update_settings", patch={"delivery": {"url": phone.url, "format": "ntfy", "content": "outcome"}})
    try:
        host.send("Researcher", f"{StandIn.ASK}: ask me something")
        pushed = wait_until("the question is pushed", lambda: phone.titled("Needs you"), 20)
        record.check("the question is pushed", pushed[0]["body"].startswith("Researcher has a question"),
                     {"body": pushed[0]["body"], "priority": pushed[0]["priority"]})
        answer_the_question(host)
        answered = wait_until("the answered run is pushed", lambda: phone.titled("Researcher"), 10 * MINUTE_MS / 1000 + 20)
        record.check("the answered run is pushed with its headline", answered[0]["body"] == StandIn.REPLY.splitlines()[0],
                     {"body": answered[0]["body"], "priority": answered[0]["priority"]})
        host.send("Scribe", "Say something short; this is a probe test.")
        done = wait_until("the finished run is pushed", lambda: phone.titled("Scribe"), 10 * MINUTE_MS / 1000 + 20)
        record.check("the finished run is pushed with its headline", done[0]["body"] == StandIn.REPLY.splitlines()[0],
                     {"body": done[0]["body"], "priority": done[0]["priority"]})
        record.check("each is pushed once",
                     holds_for("once", lambda: len(phone.received) == 3, 3),
                     {"pushes": [p["title"] for p in phone.received]})
        record.check("the pushes name what they are about",
                     all(p["delivery"] for p in phone.received))
    finally:
        host.call("update_settings", patch={"delivery": {"url": ""}})
        host.stop()


def scenario_hiddenpush(record: Record, scratch: Scratch) -> None:
    it = ready(record, scratch, MINUTE_MS)
    try:
        assert it.app and it.host and it.log and it.app.client
        host, app = it.host, it.app
        phone = Listener(scratch)
        host.call("update_settings", patch={"delivery": {"url": phone.url, "format": "ntfy", "content": "outcome"}})
        click_banner(record, app, DELIVERY_TITLE, "clicking the first banner, to be at the window",
                     give_back=False)
        record.check("the window has focus", wait_quietly(lambda: focused(app), 5) is not None, stop=True)
        leave(app, "hide")
        host.send("Scribe", "Say something short; this is a probe test.")
        wait_until("the run finishes", lambda: host.rows("outcome"), 30)
        pushed = wait_quietly(lambda: phone.titled("Scribe"), 14 * MINUTE_MS / 1000 + 20)
        record.check("after hide: the finished run is pushed", bool(pushed),
                     {"body": pushed[0]["body"]} if pushed else
                     {"rows": [{k: r[k] for k in ("status", "last_error")} for r in host.rows("outcome")]})
        record.check("it is pushed once", holds_for("once", lambda: len(phone.titled("Scribe")) <= 1, 3),
                     {"pushes": [p["title"] for p in phone.received]})
    finally:
        if it.host:
            it.host.call("update_settings", patch={"delivery": {"url": ""}})
        it.__exit__(None, None, None)


def scenario_refused(record: Record, scratch: Scratch) -> None:
    it = ready(record, scratch)
    try:
        assert it.app and it.host and it.log and it.app.client
        host, app, log, client = it.host, it.app, it.log, it.app.client
        phone = Listener(scratch)
        host.call("update_settings", patch={"delivery": {"url": phone.url, "format": "ntfy", "content": "outcome"}})
        click_banner(record, app, DELIVERY_TITLE, "clicking the first banner, to be at the window",
                     give_back=False)
        shown = len(log.settled(banner(app.bundle)))
        client.ask("refuseNextBanner")
        host.send("Researcher", f"{StandIn.ASK}: ask me something")

        def released() -> dict | None:
            rows = [r for r in host.rows("needs_you") if "question" in r["title"] and r["destination"] == "clients"]
            return rows[0] if rows and rows[0]["status"] == "pending" and rows_released(host, rows[0]["id"]) else None

        row = wait_until("the window gives the question back", released, 20)
        record.check("the refused question is given back", True)
        pushed = wait_until("the given-back question is pushed", lambda: phone.titled("Needs you"), 40)
        record.check("it is pushed although the window is present", bool(pushed),
                     {"body": pushed[0]["body"]})
        record.check("it is pushed once", holds_for("once", lambda: len(phone.titled("Needs you")) == 1, 5))
        record.check("macOS presents no banner for it", len(log.settled(banner(app.bundle))) == shown)
        record.check("the window that gave it back does not take it again",
                     [r["status"] for r in host.rows("needs_you") if r["id"] == row["id"]] == ["pending"])
    finally:
        it.__exit__(None, None, None)


def rows_released(host: Host, delivery: str) -> bool:
    with sqlite3.connect(f"file:{host.db}?mode=ro", uri=True) as conn:
        return (conn.execute("SELECT releases FROM outbox WHERE id=?", (delivery,)).fetchone() or [0])[0] >= 1


def scenario_panelstable(record: Record, scratch: Scratch) -> None:
    host = Host(scratch)
    record.secrets.append(host.token)
    host.seed()
    app = App(scratch)
    before = None
    try:
        client = app.launch()
        connect(app, host)
        wait_until("the panel is made", lambda: app.state()["menubar"]["attached"], 20)
        client.ask("close", window="main")
        wait_until("the window closes", lambda: not visible(app, "main"), 10)
        person_away("opening and reopening the panel")
        before = front()
        client.ask("clickIcon")
        wait_until("the panel opens", lambda: app.state()["menubar"]["panelVisible"], 10)
        client.ask("clickIcon")
        wait_until("the panel closes", lambda: not app.state()["menubar"]["panelVisible"], 10)
        client.ask("clickIcon")

        def stable() -> bool:
            state = app.state()
            return state["menubar"]["panelVisible"] and any(
                w["label"] == "menubar" and w["visible"] and w["focused"] for w in state["windows"]
            )

        wait_until("the panel opens again with the keyboard", stable, 10)
        record.check("the reopened panel stays visible and focused for three seconds",
                     holds_for("the reopened panel stays visible and focused", stable, 3))
    finally:
        if before:
            hand_back(app, before)
        app.quit()
        host.stop()
        app.forget()


def scenario_draft(record: Record, scratch: Scratch) -> None:
    host = Host(scratch)
    record.secrets.append(host.token)
    host.seed()
    app = App(scratch)
    try:
        client = app.launch()
        connect(app, host)
        wait_until("the panel is made", lambda: app.state()["menubar"]["attached"], 20)
        client.ask("close", window="main")
        wait_until("the window closes", lambda: not visible(app, "main"), 10)
        screen_step("reading the panel")
        client.ask("clickIcon")
        wait_until("the panel opens", lambda: app.state()["menubar"]["panelVisible"], 10)
        wait_until("the panel lists the group", lambda: all(a in client.page("menubar", do="text") for a in AGENTS), 10)
        draft = "probe draft meant for Researcher only"

        def field(agent: str) -> str:
            groups = helper("describe", str(app.pid), f"Conversation with {agent}")
            return " ".join(line for group in groups for line in group.split(" | ") if "AXTextArea" in line)

        def open_agent(agent: str) -> None:
            names = client.page("menubar", do="find", role="button")["names"]
            client.page("menubar", do="click", role="button", name=next(n for n in names if n.startswith(agent)))
            wait_until(f"{agent}'s conversation opens",
                       lambda: client.page("menubar", do="find", role="textbox", name=f"Message {agent}")["count"] == 1, 10)

        def back() -> None:
            client.page("menubar", do="click", role="button", name="Back")
            wait_until("the list is back", lambda: client.page("menubar", do="find", role="textbox")["count"] == 0, 10)

        open_agent("Researcher")
        client.page("menubar", do="type", role="textbox", name="Message Researcher", text=draft)
        wait_until("the draft is in Researcher's box", lambda: draft in field("Researcher"), 5)
        record.check("the draft is in Researcher's box", True)
        back()
        open_agent("Critic")
        record.check("Critic's box does not hold it", draft not in field("Critic"), {"critic": field("Critic")[:120]})
        back()
        open_agent("Researcher")
        record.check("coming back, Researcher's box still holds it", wait_quietly(lambda: draft in field("Researcher"), 5) is not None)
        client.ask("clickIcon")
        wait_until("the panel closes", lambda: not app.state()["menubar"]["panelVisible"], 10)
        client.ask("clickIcon")
        wait_until("the panel opens again", lambda: app.state()["menubar"]["panelVisible"], 10)
        if client.page("menubar", do="find", role="textbox", name="Message Researcher")["count"] == 0:
            open_agent("Researcher")
        record.check("after the panel closes and opens, it still holds it",
                     wait_quietly(lambda: draft in field("Researcher"), 5) is not None)
        client.ask("clickIcon")
    finally:
        app.quit()
        host.stop()
        app.forget()


LONG_AWAY_S = 6 * 60


def scenario_longaway(record: Record, scratch: Scratch) -> None:
    with Prompted(record, scratch, "Allow", minute_ms=6000) as it:
        assert it.app and it.host and it.log and it.app.client
        host, app, log = it.host, it.app, it.log
        log.wait_for("macOS presents the waiting delivery", banner(app.bundle), 15)
        host.use(StandIn(scratch))
        before = front()
        screen_step("clicking the first banner, to be at the window")
        pressed = scratch.press("clicking the first banner")
        press_when_shown("clicking the first banner", "press-group", DELIVERY_TITLE)
        try:
            wait_until("the test build comes forward", lambda: front().get("pid") == app.pid, 5)
        except Failed:
            pressed["missed"] = True
            raise
        hand_back(app, before)
        hidden = time.monotonic()
        while time.monotonic() < hidden + LONG_AWAY_S:
            time.sleep(5)
        shown = len(log.matching(banner(app.bundle)))
        host.send("Researcher", f"{StandIn.ASK}: ask me something")
        host.send("Scribe", "Say something short; this is a probe test.")
        wait_until("both runs leave a row", lambda: len(
            [r for r in host.rows() if r["kind"] == "outcome" or "question" in r["title"]]) == 2, 30)
        wait_until("the window takes the question", lambda: any(
            "question" in r["title"] and r["status"] == "delivered" for r in host.rows("needs_you")), 30)
        answer_the_question(host)
        def taken() -> list[dict]:
            return [{k: r[k] for k in ("kind", "status")} for r in host.rows()
                    if r["kind"] == "outcome" or "question" in r["title"]]

        claimed = wait_quietly(lambda: len(taken()) == 3 and all(r["status"] == "delivered" for r in taken()), 90)
        record.check("after six minutes hidden: the window takes the question and both finished runs",
                     claimed is not None, None if claimed else {"rows": taken()})
        if claimed:
            presented = wait_quietly(lambda: len(log.matching(banner(app.bundle))) >= shown + 2, 10)
            record.check("and each is a banner", presented is not None,
                         {"banners": len(log.matching(banner(app.bundle))) - shown})


def scenario_escape(record: Record, scratch: Scratch) -> None:
    host = Host(scratch)
    record.secrets.append(host.token)
    host.seed()
    app = App(scratch)
    try:
        client = app.launch()
        connect(app, host)
        wait_until("the panel is made", lambda: app.state()["menubar"]["attached"], 20)
        client.ask("close", window="main")
        wait_until("the window closes", lambda: not visible(app, "main"), 10)
        run(["open", "-a", "Finder"], check=True)
        wait_until("Finder is in front", lambda: front().get("bundle") == "com.apple.finder", 10)
        screen_step("clicking the icon for real")
        before = front()
        helper("click-icon", str(app.pid))
        record.check("a real click opens the panel",
                     wait_quietly(lambda: app.state()["menubar"]["panelVisible"], 5) is not None, stop=True)
        keyboard = wait_quietly(lambda: any(w["label"] == "menubar" and w["focused"] for w in app.state()["windows"]), 3)
        record.check("the panel has the keyboard", keyboard is not None,
                     None if keyboard else {"front": front().get("bundle")})
        helper("escape")
        closed = wait_quietly(lambda: not app.state()["menubar"]["panelVisible"], 5)
        record.check("a real Escape closes the panel", closed is not None)
        if keyboard is None:
            record.observe("the app that was in front is in front again", "not reached: the panel never had the keyboard")
        else:
            record.check("the app that was in front is in front again",
                         wait_quietly(lambda: front().get("pid") == before.get("pid"), 5) is not None,
                         {"before": before.get("bundle"), "after": front().get("bundle")})
        if not closed:
            client.ask("clickIcon")
    finally:
        app.quit()
        host.stop()
        app.forget()


SCENARIOS: dict[str, Callable[[Record, Scratch], None]] = {
    "launch": scenario_launch,
    "permission": scenario_permission,
    "notification": scenario_notification,
    "panel": scenario_panel,
    "panelstable": scenario_panelstable,
    "allow": scenario_allow,
    "button": scenario_button,
    "busy": scenario_busy,
    "deny": scenario_deny,
    "click": scenario_click,
    "away": scenario_away,
    "capture": scenario_capture,
    "longaway": scenario_longaway,
    "question": scenario_question,
    "hidden": scenario_hidden,
    "switched": scenario_switched,
    "fold": scenario_fold,
    "testclick": scenario_testclick,
    "push": scenario_push,
    "hiddenpush": scenario_hiddenpush,
    "refused": scenario_refused,
    "draft": scenario_draft,
    "escape": scenario_escape,
}

SUMMARIES: dict[str, str] = {
    "launch": "A background launch stays in the background; a normal launch comes to the front.",
    "permission": "A bundle macOS has never seen: not asked, then asked when a delivery waits.",
    "notification": "Whatever this Mac allows the test bundle, a delivery and the test button do what the pane says.",
    "panel": "The menu bar panel opens, lists the group, and closing it hands the keyboard back.",
    "panelstable": "A channel-close followed immediately by a channel-reopen leaves the panel visible and focused for three seconds.",
    "allow": "Screen: Allow on the question, and the waiting delivery is shown as a banner.",
    "button": "Screen: once macOS allows the bundle, the Settings test button raises a banner.",
    "busy": "Screen: a delivery handed over while the window's main thread is busy is drawn, and its click opens the conversation.",
    "deny": "Screen: Don't Allow on the question, and the pane says notifications are off.",
    "click": "Screen: a delivery's banner, clicked, opens the conversation it is about.",
    "away": "Screen: four deliveries waiting when the window connects are one banner, and all four are taken.",
    "capture": "Screen: one image of the test build's panel and one of its window, and nothing else.",
    "longaway": "Screen: Guaca hidden for six minutes; a question and a finished run are each a banner.",
    "question": "Screen: a run that asks the operator a question is a banner at once, and its click opens the conversation.",
    "hidden": "Screen: at the window once, then Guaca hidden; a run that finishes is a banner.",
    "switched": "Screen: at the window once, then another app in front; a run that finishes is a banner.",
    "fold": "Screen: two runs that finish while the operator is away are one banner, and its click opens For you.",
    "testclick": "Screen: a click on the Settings test banner brings Guaca forward.",
    "push": "Every window closed: a question and a finished run each reach the push address once.",
    "hiddenpush": "Screen: at the window once, then Guaca hidden; a run that finishes reaches the push address once.",
    "refused": "Screen: a banner the system refuses is given back, and pushed once although the window is present.",
    "draft": "Screen: a draft typed to one agent in the panel is not on another, and is there when you come back; read through Accessibility.",
    "escape": "Screen: a real click on the icon gives the panel the keyboard; a real Escape closes it and gives the keyboard back.",
}



def sweep() -> int:
    listed = run([LSREGISTER, "-dump"]).stdout
    copies = set(re.findall(rf"path:\s+(/\S[^\n]*?/target/probe/apps/guaca-probe-[^/\n]+/{re.escape(APP_NAME)}\.app)", listed))
    stale = sorted(path for path in copies if not Path(path).exists())
    for path in stale:
        run([LSREGISTER, "-u", path])
    return len(stale)


def build() -> None:
    steps = [
        (["pnpm", "tauri", "build", "--debug", "--bundles", "app", "--features", "probe",
          "--config", "src-tauri/tauri.probe.conf.json"], ROOT),
        (["cargo", "build", "--no-default-features", "--features", "server,probe-host", "--bin", "guacad"], TAURI),
    ]
    for args, cwd in steps:
        built = subprocess.run(args, cwd=cwd, stdout=sys.stderr, stderr=sys.stderr)
        if built.returncode != 0:
            raise SystemExit(f"build failed: {' '.join(args)}")


def attempt(name: str, index: int, keep: bool) -> tuple[Record, dict]:
    scratch = Scratch()
    record = Record(name, index, [])
    evidence = None
    started = time.monotonic()
    try:
        SCENARIOS[name](record, scratch)
    except Deferred as why:
        record.deferred = True
        record.steps.append({"step": str(why), "deferred": True})
    except (Failed, Refused) as why:
        if not record.steps or record.steps[-1].get("pass", True):
            record.steps.append({"step": str(why), "pass": False})
    except Exception as error:
        record.steps.append({"step": f"the probe itself failed: {error!r}", "pass": False})
    finally:
        scratch.stop_all()
        if not record.passed and not record.deferred:
            evidence = scratch.retain_failure()
        if keep:
            record.steps.append({"step": "scratch kept", "observed": str(scratch.root)})
        else:
            scratch.close()
    entry = record.result()
    if not record.passed and not record.deferred:
        entry["evidence"] = str(evidence) if evidence else None
    entry["seconds"] = round(time.monotonic() - started, 1)
    missed = [line[:200] for log in scratch.logs for line in log.missed]
    if missed:
        entry["streamMissed"] = missed
    return record, entry


def summarize(results: list[dict]) -> dict:
    ran = [r for r in results if r["outcome"] != "not run"]
    never_ran = [name for name in dict.fromkeys(r["scenario"] for r in results)
                 if not any(r["scenario"] == name for r in ran)]
    counts = {outcome: sum(r["outcome"] == outcome for r in results) for outcome in ("pass", "fail", "not run")}
    by_scenario: dict[str, list[list]] = {}
    for result in ran:
        by_scenario.setdefault(result["scenario"], []).append(
            [(s["step"], s.get("pass"), s.get("observed")) for s in result["steps"]]
        )
    return {
        "pass": counts["fail"] == 0 and not never_ran,
        "counts": counts,
        "notRun": [{"scenario": r["scenario"], "run": r["run"], "reason": r["steps"][-1]["step"]}
                   for r in results if r["outcome"] == "not run"],
        "neverRan": never_ran,
        "runs": results,
        "identical": {name: all(r == runs[0] for r in runs) for name, runs in by_scenario.items()},
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=USAGE, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("scenarios", nargs="*", help="which to run; all of them if none")
    parser.add_argument("--runs", type=int, default=3, help="times in a row to run each (default 3)")
    parser.add_argument("--no-build", action="store_true", help="reuse the last test build")
    parser.add_argument("--keep", action="store_true", help="leave scratch directories for inspection")
    parser.add_argument("--list", action="store_true", help="list the scenarios and exit")
    args = parser.parse_args()

    if args.list:
        for name in SCENARIOS:
            print(f"{name:14} {SUMMARIES.get(name, '')}")
        return 0
    if sys.platform != "darwin":
        print("the probe drives macOS; run it on a Mac", file=sys.stderr)
        return 2
    unknown = [name for name in args.scenarios if name not in SCENARIOS]
    if unknown:
        print(f"no such scenario: {', '.join(unknown)} (see --list)", file=sys.stderr)
        return 2
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    if not args.no_build:
        build()
    if not BUILT_APP.exists() or not GUACAD.exists():
        print("no test build; run without --no-build", file=sys.stderr)
        return 2

    awake = subprocess.Popen(["caffeinate", "-u", "-d", "-i", "-w", str(os.getpid())])
    results = []
    try:
        for name in args.scenarios or list(SCENARIOS):
            for index in range(1, args.runs + 1):
                _, entry = attempt(name, index, args.keep)
                results.append(entry)
                said = {"pass": "pass", "fail": "FAIL", "not run": f"not run, {entry['steps'][-1]['step']}"}
                print(f"{name} run {index}: {said[entry['outcome']]}", file=sys.stderr)
    finally:
        awake.terminate()

    summary = summarize(results)
    summary["forgotten"] = sweep()
    print(json.dumps(summary, indent=2))
    return 0 if summary["pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
