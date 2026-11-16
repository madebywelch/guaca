#!/usr/bin/env python3
"""Run Guaca's Mac-only behavior as scenarios a machine can check.

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

from __future__ import annotations

import argparse
import json
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

# What the Notifications pane says about macOS, by the app's own reading.
# Kept equal to MAC_ACCESS in src/components/SettingsDialog.tsx.
PANE = {
    "allowed": "macOS allows Guaca to show notifications.",
    "notAsked": "macOS has not asked you about Guaca yet.",
    "denied": "macOS has notifications turned off for Guaca.",
}
SENT = "Sent. It should be on your screen now."

# A banner the probe raises says what it is.
DELIVERY_TITLE = "Guaca probe: test notification"
DELIVERY_BODY = "Raised by scripts/probe.py while testing a build. Nothing needs you."


class Failed(Exception):
    """A step that did not hold. The scenario stops at it."""


class Refused(Exception):
    """The app answered a request with why it could not do it. Inside a wait,
    that is "not yet"; anywhere else it fails the step."""


class Deferred(Exception):
    """A screen step that did not run because someone is using the Mac."""


def redact(text: str, secrets_: list[str]) -> str:
    for secret in secrets_:
        if secret:
            text = text.replace(secret, "<redacted>")
    return re.sub(r"#token=[^\s\"']+", "#token=<redacted>", text)


def wait_until(what: str, check: Callable[[], Any], timeout: float, every: float = 0.1) -> Any:
    """The value of check() once it is truthy, or Failed naming what never happened."""
    deadline = time.monotonic() + timeout
    last = ""
    while True:
        try:
            value = check()
        except Failed:
            raise
        except Exception as error:  # a condition that cannot be read yet is not one that failed
            value, last = None, str(error)
        if value:
            return value
        if time.monotonic() >= deadline:
            raise Failed(f"{what}: not within {timeout:g}s" + (f" (last: {last})" if last else ""))
        time.sleep(every)


def run(args: list[str], **kwargs: Any) -> subprocess.CompletedProcess:
    return subprocess.run(args, capture_output=True, text=True, **kwargs)


# Observers ------------------------------------------------------------------


def helper(*args: str) -> Any:
    """One question to macOS through scripts/probe/system.swift."""
    if not HELPER.exists() or HELPER.stat().st_mtime < HELPER_SOURCE.stat().st_mtime:
        HELPER.parent.mkdir(parents=True, exist_ok=True)
        built = run(["swiftc", "-O", str(HELPER_SOURCE), "-o", str(HELPER)])
        if built.returncode != 0:
            raise Failed(f"could not build the system helper: {built.stderr.strip()[-400:]}")
    answer = run([str(HELPER), *args], timeout=30)
    value = json.loads(answer.stdout.strip().splitlines()[-1]) if answer.stdout.strip() else None
    if answer.returncode != 0:
        raise Failed((value or {}).get("error") or answer.stderr.strip() or "the system helper failed")
    return value


def front() -> dict:
    return helper("front")


class FrontWatch:
    """Every app that becomes frontmost while it runs, from macOS's own event."""

    def __init__(self) -> None:
        helper("front")  # builds the helper before the clock matters
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
    """macOS's log, from the moment it starts, filtered to one predicate.

    Two readers of one predicate. `log stream` is live, and it drops lines
    when the daemons it watches are busy: usernoted, sweeping the bundles every
    earlier run registered, has lost the very line a step waited for while the
    persisted log still held it. So the stream answers "seen" quickly, and
    only `log show` over the same window answers "not there". A line only
    the second reader found is listed in `missed`, so a run says how often
    its live observer was wrong.
    """

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
        """The same predicate read back from the log macOS kept. Raises Failed
        when it cannot be read, which makes the question unknown rather than
        answered."""
        shown = run(["log", "show", "--style", "ndjson", "--info", "--debug",
                     "--start", self.since, "--predicate", self.predicate], timeout=60)
        if shown.returncode != 0:
            raise Failed(f"unknown: the log could not be read back ({shown.stderr.strip()[-200:]})")
        return self._messages(shown.stdout)

    def matching(self, pattern: str) -> list[str]:
        rx = re.compile(pattern)
        return [m for m in self.messages() if rx.search(m)]

    def settled(self, pattern: str) -> list[str]:
        """Every matching line, the stream's and the persisted log's: what a
        count or an absence is asserted on."""
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
        """Whether at least `count` lines match within `timeout`: the stream
        first, and the persisted log before saying no."""
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
    """What the notification daemons and the app itself say about this bundle."""
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
"""Seconds without a key or the pointer before the Mac counts as left alone."""


def idle_seconds() -> float:
    return float(helper("idle")["seconds"])


def person_away(what: str) -> None:
    """Before a step that moves the keyboard: only while nobody is typing or
    pointing. A step that would take the front from someone at work is put
    off, never forced."""
    if idle_seconds() < AWAY:
        raise Deferred(f"{what}: someone is using the Mac; try again later")


def screen_step(what: str) -> None:
    """Before a step that acts on system UI: Accessibility, and nobody at the Mac."""
    access = helper("access")
    if not access["accessibility"]:
        raise Failed(
            f"{what}: Accessibility is not allowed for the app that runs this command "
            "(System Settings, Privacy & Security, Accessibility)"
        )
    if access["locked"]:
        # Behind the lock screen Notification Center draws neither the
        # question nor a banner, so there is nothing to press. A display that
        # went to sleep raises the same shield with no password asked, and
        # waking it lowers the shield: only a session still locked after a
        # wake needs a person.
        run(["caffeinate", "-u", "-t", "2"])
        try:
            wait_until("the lock screen lowers once the display wakes", lambda: not helper("access")["locked"], 10, every=0.5)
        except Failed:
            raise Failed(f"{what}: the screen is locked, and only a person can unlock it") from None
    person_away(what)


# The scratch world ----------------------------------------------------------


class Scratch:
    """One run's directories, all under one temporary root, removed at the end."""

    def __init__(self) -> None:
        self.root = Path(tempfile.mkdtemp(prefix="guaca-probe-"))
        self.home = self.root / "home"
        self.channel = self.root / "run"
        self.host = self.root / "host"
        for dir_ in (self.home, self.channel, self.host):
            dir_.mkdir(mode=0o700)
        # Not beside the rest: macOS will not show notifications for an app
        # in a temporary directory ("Failed to find or validate client"), so
        # the copy lives under the build's own output, out of the repository.
        self.apps = APPS / self.root.name
        self.apps.mkdir(parents=True)
        self.started: list[Callable[[], None]] = []
        self.logs: list[UnifiedLog] = []

    def track(self, stop: Callable[[], None]) -> None:
        """Something this run started, stopped when the run ends however it
        ends, so a scenario that fails half way leaves nothing behind."""
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
    """An OpenAI-compatible endpoint on loopback that the scratch Host is
    pointed at, so a scenario can make an agent really run without a key and
    without spending anything.

    A turn whose newest message from the operator contains `ASK` asks the
    operator a question with `ask_operator`; every other turn answers with
    REPLY, whose first line is the run's headline. The Host's own code does
    everything else: the outcome, the parked question, the outbox row.
    """

    ASK = "PROBE-ASK"
    REPLY = "Probe test run finished.\nNothing here needs you; this is a scripted reply."
    QUESTION = "Probe test question: which one?"

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
        """The stream for one call, in the shape every compatible provider sends."""
        asked = next((m for m in reversed(messages) if m.get("role") == "user"), {})
        text = asked.get("content") or ""
        if isinstance(text, list):
            text = " ".join(part.get("text", "") for part in text if isinstance(part, dict))
        answered = any(m.get("role") == "tool" for m in messages[messages.index(asked) + 1:]) if asked else False
        usage = {"prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120}
        if self.ASK in text and not answered:
            call = {"index": 0, "id": "call_ask", "type": "function",
                    "function": {"name": "ask_operator",
                                 "arguments": json.dumps({"question": self.QUESTION, "options": ["First", "Second"]})}}
            return [{"choices": [{"index": 0, "delta": {"tool_calls": [call]}}]},
                    {"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}], "usage": usage}]
        return [{"choices": [{"index": 0, "delta": {"content": self.REPLY}}]},
                {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "usage": usage}]


class Host:
    """guacad on loopback, with a data directory nobody else uses."""

    def __init__(self, scratch: Scratch, minute_ms: int | None = None) -> None:
        self.scratch = scratch
        # A shorter minute for the outbox's two waits (src-tauri/src/probe_clock.rs),
        # so a scenario about being away does not take twelve real minutes.
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
        """The seeded group, and `count` deliveries waiting for a window if
        asked. Returns the first one's id; `self.waiting` holds them all.

        The schema is the Host's to make, so it is started once and stopped
        before anything is written, and started again to read it.
        """
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
        """One command on the Host, over the route a browser uses."""
        request = urllib.request.Request(
            f"{self.origin}/v1/call",
            data=json.dumps({"name": name, "args": args}).encode(),
            headers={"content-type": "application/json", "authorization": f"Bearer {self.token}"},
            method="POST",
        )
        with urllib.request.urlopen(request, timeout=30) as answer:
            return json.loads(answer.read())["ok"]

    def use(self, model: StandIn) -> None:
        """Turns paid for by the stand-in. The key is a placeholder the
        provider check needs; it is sent nowhere but loopback."""
        self.call("update_settings", patch={"provider": "compatible", "baseUrl": model.base,
                                            "apiKey": "probe-stand-in", "defaultModel": "probe/stand-in"})

    def send(self, agent: str, text: str) -> None:
        """The operator writes to an agent, which starts a run."""
        self.call("send_message", agentId=self.agent_id(agent), text=text)

    def rows(self, kind: str | None = None) -> list[dict]:
        with sqlite3.connect(f"file:{self.db}?mode=ro", uri=True) as conn:
            conn.row_factory = sqlite3.Row
            found = [dict(row) for row in conn.execute(
                "SELECT id, kind, destination, status, title, body, target, last_error, next_attempt_at "
                "FROM outbox ORDER BY created_at")]
        return [row for row in found if kind is None or row["kind"] == kind]


class Client:
    """The probe's control channel: one JSON line out, one back."""

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
    """A copy of the test build in the scratch directory, and the process it becomes."""

    def __init__(self, scratch: Scratch, bundle: str = IDENTIFIER, folder: str = "") -> None:
        self.scratch = scratch
        self.bundle = bundle
        self.path = scratch.apps / folder / f"{APP_NAME}.app"
        shutil.copytree(BUILT_APP, self.path, symlinks=True)
        plist = self.path / "Contents/Info.plist"
        if bundle != IDENTIFIER:
            run(["plutil", "-replace", "CFBundleIdentifier", "-string", bundle, str(plist)], check=True)
        # The build is only linker-signed. Signed whole, with the identifier
        # it now has, it is the bundle macOS keys its notification switch on.
        signed = run(["codesign", "--force", "--deep", "--sign", "-", "--identifier", bundle, str(self.path)])
        if signed.returncode != 0:
            raise Failed(f"could not sign the test build: {signed.stderr.strip()}")
        self.socket = scratch.channel / "probe.sock"
        self.client: Client | None = None
        self.pid: int | None = None
        scratch.track(self.forget)
        scratch.track(self.quit)

    def launch(self, quiet: bool = True, background: bool = True, hidden: bool = True) -> Client:
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
            "GUAC_LOG": "guac=info,warn",
        }
        if quiet:
            env["GUACA_PROBE_QUIET"] = "1"
        for key, value in env.items():
            args += ["--env", f"{key}={value}"]
        log = str(self.scratch.root / "app.log")
        args += ["--stdout", log, "--stderr", log, "-a", str(self.path)]
        opened = run(args)
        if opened.returncode != 0:
            raise Failed(f"could not launch the test build: {opened.stderr.strip()}")
        wait_until("the probe channel opens", self.socket.exists, 30)
        self.client = Client(self.socket)
        self.pid = self.client.ask("hello")["pid"]
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
        # A launch whose channel never opened has no pid to ask; the copy's
        # own path still names its process, and nothing else runs from it.
        run(["pkill", "-9", "-f", str(self.path / "Contents/MacOS/")])

    def forget(self) -> None:
        """Out of Launch Services, so no trace of the copy outlives the run."""
        run([LSREGISTER, "-u", str(self.path)])


# Steps a scenario shares ----------------------------------------------------


def connect(app: App, host: Host) -> None:
    """Points the test window at the scratch Host and waits for its workspace."""
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
    """Settings, then its Notifications section, as a person would press them."""
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
    """The sentence the Notifications pane writes about macOS, if any."""
    found = re.search(r"(macOS [^.]*\.|This copy of Guaca is not running[^.]*\.)", text)
    return found.group(1) if found else None


class Record:
    """One run of one scenario: the steps, in order, each passing or not."""

    def __init__(self, scenario: str, index: int, secrets_: list[str]) -> None:
        self.scenario = scenario
        self.index = index
        self.steps: list[dict] = []
        self.secrets = secrets_
        self.deferred = False

    def check(self, step: str, holds: bool, detail: Any = None, stop: bool = False) -> bool:
        """Records a step. With stop, a step that fails ends the run: what
        follows depends on it."""
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

    def result(self) -> dict:
        return {"scenario": self.scenario, "run": self.index, "pass": self.passed,
                "deferred": self.deferred, "steps": self.steps}


# Scenarios ------------------------------------------------------------------


def background_launch(record: Record, app: App, label: str, quiet: bool) -> None:
    """One launch asked to stay in the background, watched for ten seconds.
    With quiet, the app's own launch-time activation is turned off, and the
    launch is held to staying back; without it, what happens is recorded."""
    person_away("launching where the app may take the front")
    watch = FrontWatch()
    try:
        app.launch(quiet=quiet, background=True, hidden=False)
        try:
            wait_until("the test build exits", lambda: not app.alive(), 10)
            exited = True
        except Failed:
            exited = False
        took = watch.activated(app.pid or -1)
        focused = [w["label"] for w in app.state()["windows"] if w["focused"]]
        hand_back(app, watch.before)
        record.check(f"{label}: still running after ten seconds", not exited)
        record.check(f"{label}: the app that was in front is in front again",
                     front().get("pid") == watch.before.get("pid"))
        if quiet:
            record.check(f"{label}: did not become the frontmost app", not took,
                         {"activations": [e for e in watch.events if e.get("pid") == app.pid]})
            record.check(f"{label}: no window took the keyboard", not focused, {"focused": focused})
        else:
            record.observe(f"{label}: became the frontmost app", took)
            record.observe(f"{label}: windows holding the keyboard", focused)
    finally:
        watch.stop()
        app.quit()


def scenario_launch(record: Record, scratch: Scratch) -> None:
    """What a background launch does to the front, and whose doing it is."""
    plain = App(scratch, fresh_bundle(), "plain")
    app = App(scratch, fresh_bundle(), "quiet")
    for copy in (plain, app):
        signed = codesign(copy.path)
        record.check("the test build is signed as its own bundle", signed["valid"], signed, stop=True)
    try:
        background_launch(record, plain, "first background launch of a new bundle", quiet=False)
        background_launch(record, plain, "second background launch of the same bundle", quiet=False)
        # Every other scenario launches a bundle macOS has never seen with
        # the app's own activation turned off. This is that launch, on a
        # second new bundle, so it is held to what the others rely on.
        background_launch(record, app, "first background launch of a new bundle, activation off", quiet=True)

        person_away("launching in the foreground")
        watch = FrontWatch()
        try:
            app.launch(quiet=False, background=False, hidden=False)
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
        plain.forget()
        app.forget()


def hand_back(app: App, before: dict) -> None:
    """If the test build is in front, hide it, which gives the keyboard back."""
    if app.client and front().get("pid") == app.pid:
        app.client.ask("hide")
        wait_until("the app that was in front is in front again",
                   lambda: front().get("pid") != app.pid, 10)


def holds_for(what: str, check: Callable[[], Any], seconds: float) -> bool:
    """Whether check() stays truthy for the whole window: how an absence is
    observed. The window is the assertion, not a guess at how long to wait."""
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
    """Why macOS kept the bundle's banner off the screen, if it did.

    usernoted logs "Presenting ... as banner" either way; the decision is the
    Do Not Disturb resolution before it. A Focus, or the display being shared
    or mirrored ("display shared", the default while Screen Sharing is
    connected), silences everything but the system's own questions.
    """
    pattern = (rf"interruptionSuppression: silence;.*resolutionReason: ([^;]+);"
               rf".*bundleIdentifier: {re.escape(bundle)};")
    for message in log.settled(pattern):
        found = re.search(pattern, message)
        if found:
            return found.group(1)
    return None


def drawn(record: Record, log: UnifiedLog, bundle: str) -> None:
    """A banner macOS says it presented is on screen only if nothing silenced it."""
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
    """Presses system UI as soon as Notification Center has drawn it.

    The log says macOS presented something a moment before the element exists,
    and a banner is up for only a few seconds, so the press is retried until it
    lands, and a miss says what was there instead.
    """
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
    """A bundle macOS has never seen: not asked, then asked when a delivery waits."""
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
    """Whatever this Mac allows the test bundle, a delivery and the test button do what the pane says."""
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
        # A pass means a different thing on each branch, and only "allowed"
        # shows a banner, so the run says which one it took.
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
    """The menu bar panel opens, lists the group, and closing it hands the keyboard back."""
    host = Host(scratch)
    record.secrets.append(host.token)
    host.seed()
    app = App(scratch)
    try:
        client = app.launch()
        connect(app, host)
        wait_until("the panel is made for the window's host", lambda: app.state()["menubar"]["attached"], 20)
        record.check("the panel knows the window's host", True)
        # Closed the way a person closes it, which hides it: the panel hands
        # the keyboard back only when there is no window to stay in.
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
    """A fresh bundle, a waiting delivery, and the question answered by
    pressing a button on it. Whatever was started is stopped on the way out,
    however far it got: a question nobody answers is left on someone's screen."""

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
            # From here the window's main thread is kept busy, so the answer,
            # the claim and the hand-off to macOS all happen while it is.
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
    """Screen: Allow on the question, and the waiting delivery is shown as a banner."""
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
    """Screen: once macOS allows the bundle, the Settings test button raises a banner."""
    with Prompted(record, scratch, "Allow") as it:
        assert it.app and it.log and it.app.client
        app, client = it.app, it.app.client
        it.log.wait_for("macOS presents the waiting delivery", banner(app.bundle), 15)
        drawn(record, it.log, app.bundle)
        open_notifications(client)
        wait_until("the pane says macOS allows it", lambda: PANE["allowed"] in pane_text(client), 5)
        client.page("main", do="click", role="button", name="Send a test notification")
        # Nothing asks the app anything until macOS has answered, so that if a
        # banner is only handed over while the main run loop is idle, as it was
        # through the notification plugin, this test still sees it fail.
        shown = it.log.reaches("macOS presents the test", banner(app.bundle), 2, 10)
        record.check("the test button's banner is presented", shown,
                     None if shown else {"pane says sent": SENT in pane_text(client)})


def scenario_busy(record: Record, scratch: Scratch) -> None:
    """Screen: a delivery handed over while the window's main thread is busy is drawn, and its click opens the conversation."""
    with Prompted(record, scratch, "Allow", busy=10000) as it:
        assert it.app and it.host and it.log and it.app.client
        client, bundle = it.app.client, it.app.bundle
        critic = it.host.agent_id("Critic")
        it.log.wait_for("macOS presents the delivery", banner(bundle), 15)
        drawn(record, it.log, bundle)
        record.check("a delivery handed over while the main thread is busy is drawn", True)
        record.check("another conversation is open first", client.page("main", do="view")["selected"] != critic)
        screen_step("clicking the banner")
        press_when_shown("clicking the banner", "press-group", DELIVERY_TITLE)
        wait_until("the window opens the conversation",
                   lambda: client.page("main", do="view")["selected"] == critic, 15)
        record.check("its click opens the conversation", True)


def scenario_deny(record: Record, scratch: Scratch) -> None:
    """Screen: Don't Allow on the question, and the pane says notifications are off."""
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
    """Screen: a delivery's banner, clicked, opens the conversation it is about."""
    with Prompted(record, scratch, "Allow") as it:
        assert it.app and it.host and it.log and it.app.client
        client = it.app.client
        critic = it.host.agent_id("Critic")
        it.log.wait_for("macOS presents the delivery", banner(it.app.bundle), 15)
        drawn(record, it.log, it.app.bundle)
        record.check("the delivery is on screen as a banner", True)
        record.check("another conversation is open first", client.page("main", do="view")["selected"] != critic)
        # A banner stays up for a few seconds, so it is pressed as soon as it
        # can be found, and found as soon as it is drawn.
        screen_step("clicking the banner")
        press_when_shown("clicking the banner", "press-group", DELIVERY_TITLE)
        wait_until("the window opens the conversation",
                   lambda: client.page("main", do="view")["selected"] == critic, 10)
        record.check("the click opens the conversation the delivery is about", True)


def scenario_away(record: Record, scratch: Scratch) -> None:
    """Screen: four deliveries waiting when the window connects are one banner, and all four are taken."""
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
    """Screen: one image of the test build's panel and one of its window, and nothing else."""
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


# Scenarios that run agents ---------------------------------------------------
#
# These point the scratch Host at a StandIn, so what reaches macOS is a real
# outbox row a real run wrote, not one seeded into SQLite.

MINUTE_MS = 6000
"""A minute on the Host of a scenario about being away: two minutes become
twelve seconds, and the ten-minute gathering one minute."""

PRESENCE_MINUTE_MS = 30000
"""A minute on the Host of a scenario about whether the operator left. The
window says it is still there every HEARTBEAT_MS (45 seconds, in view.ts), so
the two minutes before away have to stay longer than that: at MINUTE_MS a
window that wrongly keeps saying it is there would still go away between two
of its own heartbeats, and the scenario could not tell it from one that said
it had left."""


def ready(record: Record, scratch: Scratch, minute_ms: int | None = None) -> Prompted:
    """A fresh bundle allowed to notify, its first banner shown, and its Host
    answering through a StandIn."""
    it = Prompted(record, scratch, "Allow", minute_ms=minute_ms)
    it.__enter__()
    assert it.app and it.host and it.log
    it.log.wait_for("macOS presents the waiting delivery", banner(it.app.bundle), 15)
    it.host.use(StandIn(scratch))
    return it


def click_banner(record: Record, app: App, scope: str, what: str, give_back: bool = True) -> None:
    """Presses the test build's banner that names `scope`; the test build
    comes forward, and unless asked not to, the app that was in front gets the
    front back."""
    before = front()
    screen_step(what)
    press_when_shown(what, "press-group", scope)
    record.check(f"{what}: the test build comes forward",
                 bool(wait_quietly(lambda: front().get("pid") == app.pid, 5)))
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
    """The operator leaves the window: by hiding Guaca, or by switching to
    Finder, which every Mac has and which holds nothing of anyone's here."""
    if how == "hide":
        assert app.client
        app.client.ask("hide")
    else:
        run(["osascript", "-e", 'tell application "Finder" to activate'], check=True)
    wait_until("the test build is no longer in front", lambda: front().get("pid") != app.pid, 10)


def scenario_question(record: Record, scratch: Scratch) -> None:
    """Screen: a run that asks the operator a question is a banner at once, and its click opens the conversation."""
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
    """At the window once, then gone: a run that finishes is a banner."""
    it = ready(record, scratch, PRESENCE_MINUTE_MS)
    try:
        assert it.app and it.host and it.log and it.app.client
        host, app, log, client = it.host, it.app, it.log, it.app.client
        scribe = host.agent_id("Scribe")
        click_banner(record, app, DELIVERY_TITLE, "clicking the first banner, to be at the window",
                     give_back=False)
        record.check("the window has focus", wait_quietly(lambda: focused(app), 5) is not None, stop=True)
        # The window had focus; now the operator leaves it.
        leave(app, how)
        shown = len(log.settled(banner(app.bundle)))
        host.send("Scribe", "Say something short; this is a probe test.")
        wait_until("the run finishes", lambda: host.rows("outcome"), 30)
        # Two minutes away and ten gathering, on this Host's minute, and some room.
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
    """Screen: at the window once, then Guaca hidden; a run that finishes is a banner."""
    finished_after_leaving(record, scratch, "hide")


def scenario_switched(record: Record, scratch: Scratch) -> None:
    """Screen: at the window once, then another app in front; a run that finishes is a banner."""
    finished_after_leaving(record, scratch, "switch")


def scenario_fold(record: Record, scratch: Scratch) -> None:
    """Screen: two runs that finish while the operator is away are one banner, and its click opens For you."""
    it = ready(record, scratch, MINUTE_MS)
    try:
        assert it.app and it.host and it.log and it.app.client
        host, app, log, client = it.host, it.app, it.log, it.app.client
        click_banner(record, app, DELIVERY_TITLE, "clicking the first banner, to be at the window")
        # Away is two minutes after the last focus, on this Host's minute.
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
    """Screen: a click on the Settings test banner brings Guaca forward."""
    with Prompted(record, scratch, "Allow") as it:
        assert it.app and it.log and it.app.client
        app, client = it.app, it.app.client
        it.log.wait_for("macOS presents the waiting delivery", banner(app.bundle), 15)
        open_notifications(client)
        wait_until("the pane says macOS allows it", lambda: PANE["allowed"] in pane_text(client), 5)
        client.page("main", do="click", role="button", name="Send a test notification")
        record.check("the test banner is presented", it.log.reaches("macOS presents the test", banner(app.bundle), 2, 10), stop=True)
        click_banner(record, app, TEST_BODY, "clicking the test banner")


TEST_BODY = "This is what a notification looks like."
"""The Settings test banner's body, kept equal to SettingsDialog.tsx."""


class Listener:
    """A push address on loopback: every POST the Host sends is kept, with
    its headers, so a scenario reads back what a phone would have shown."""

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


def scenario_push(record: Record, scratch: Scratch) -> None:
    """Every window closed: a question and a finished run each reach the push address once."""
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
        host.send("Scribe", "Say something short; this is a probe test.")
        done = wait_until("the finished run is pushed", lambda: phone.titled("Scribe"), 10 * MINUTE_MS / 1000 + 20)
        record.check("the finished run is pushed with its headline", done[0]["body"] == StandIn.REPLY.splitlines()[0],
                     {"body": done[0]["body"], "priority": done[0]["priority"]})
        record.check("each is pushed once",
                     holds_for("once", lambda: len(phone.received) == 2, 3),
                     {"pushes": [p["title"] for p in phone.received]})
        record.check("the pushes name what they are about",
                     all(p["delivery"] for p in phone.received))
    finally:
        host.call("update_settings", patch={"delivery": {"url": ""}})
        host.stop()


def scenario_hiddenpush(record: Record, scratch: Scratch) -> None:
    """Screen: at the window once, then Guaca hidden; a run that finishes reaches the push address once."""
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
        # Two minutes away and ten gathering, on this Host's minute, and some room.
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
    """Screen: a banner the system refuses is given back, and pushed once although the window is present."""
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


def scenario_draft(record: Record, scratch: Scratch) -> None:
    """Screen: a draft typed to one agent in the panel is not on another, and is there when you come back; read through Accessibility."""
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
            """What the message box holds, as assistive technology reads it."""
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
"""How long the operator stays away in the long-away scenario, in real
seconds. WebKit suspends a page nobody can see after about five minutes, by
the wall clock; no shorter minute on the Host stands in for that."""


def scenario_longaway(record: Record, scratch: Scratch) -> None:
    """Screen: Guaca hidden for six minutes; a question and a finished run are each a banner."""
    # The Host's minute is short so the finished run's ten-minute gathering
    # does not add ten more; the six minutes hidden are real.
    with Prompted(record, scratch, "Allow", minute_ms=6000) as it:
        assert it.app and it.host and it.log and it.app.client
        host, app, log = it.host, it.app, it.log
        log.wait_for("macOS presents the waiting delivery", banner(app.bundle), 15)
        host.use(StandIn(scratch))
        before = front()
        screen_step("clicking the first banner, to be at the window")
        press_when_shown("clicking the first banner", "press-group", DELIVERY_TITLE)
        wait_until("the test build comes forward", lambda: front().get("pid") == app.pid, 5)
        hand_back(app, before)
        hidden = time.monotonic()
        while time.monotonic() < hidden + LONG_AWAY_S:
            time.sleep(5)
        shown = len(log.matching(banner(app.bundle)))
        host.send("Researcher", f"{StandIn.ASK}: ask me something")
        host.send("Scribe", "Say something short; this is a probe test.")
        wait_until("both runs leave a row", lambda: len(
            [r for r in host.rows() if r["kind"] == "outcome" or "question" in r["title"]]) == 2, 30)
        def both() -> list[dict]:
            return [{k: r[k] for k in ("kind", "status")} for r in host.rows()
                    if r["kind"] == "outcome" or "question" in r["title"]]

        # The question is offered at once and the finished run when its
        # gathering closes, a minute on this Host; then a few seconds each.
        claimed = wait_quietly(lambda: all(r["status"] == "delivered" for r in both()), 90)
        record.check("after six minutes hidden: the window takes the question and the finished run",
                     claimed is not None, None if claimed else {"rows": both()})
        if claimed:
            presented = wait_quietly(lambda: len(log.matching(banner(app.bundle))) >= shown + 2, 10)
            record.check("and each is a banner", presented is not None,
                         {"banners": len(log.matching(banner(app.bundle))) - shown})


def scenario_escape(record: Record, scratch: Scratch) -> None:
    """Screen: a real click on the icon gives the panel the keyboard; a real Escape closes it and gives the keyboard back."""
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
            # Never having left, the app in front proves nothing about the hand-back.
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


# The command ----------------------------------------------------------------


def sweep() -> int:
    """Forgets every copy of the test build Launch Services still lists from a
    probe's folder of copies, in this checkout or another, once the copy is
    gone from disk: a run killed before its teardown leaves one behind.
    Returns how many. What usernoted remembers about a bundle, and the line
    System Settings lists for it, belong to the system and stay."""
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
    """One run of one scenario in its own scratch world."""
    scratch = Scratch()
    record = Record(name, index, [])
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
        if keep:
            scratch.stop_all()
            record.steps.append({"step": "scratch kept", "observed": str(scratch.root)})
        else:
            scratch.close()
    entry = record.result()
    entry["seconds"] = round(time.monotonic() - started, 1)
    # Lines the live stream never delivered and the persisted log had: each
    # one a step that would have been read as macOS doing nothing.
    missed = [line[:200] for log in scratch.logs for line in log.missed]
    if missed:
        entry["streamMissed"] = missed
    return record, entry


def patient(patience: float, deferrals: list[dict], entry: dict) -> bool:
    """A run put off because someone was at the Mac is tried again once
    nobody has touched it for AWAY seconds, for as long as patience allows.
    Returns whether to try again."""
    deferrals.append({"scenario": entry["scenario"], "run": entry["run"], "step": entry["steps"][-1]["step"]})
    print(f"{entry['scenario']} run {entry['run']}: put off, someone is using the Mac", file=sys.stderr)
    deadline = time.monotonic() + patience
    while time.monotonic() < deadline:
        if idle_seconds() >= AWAY:
            return True
        time.sleep(2)
    return False


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("scenarios", nargs="*", help="which to run; all of them if none")
    parser.add_argument("--runs", type=int, default=3, help="times in a row to run each (default 3)")
    parser.add_argument("--no-build", action="store_true", help="reuse the last test build")
    parser.add_argument("--keep", action="store_true", help="leave scratch directories for inspection")
    parser.add_argument("--patience", type=float, default=1800,
                        help="seconds to wait for the Mac to be left alone before a put-off run gives up (default 1800)")
    parser.add_argument("--list", action="store_true", help="list the scenarios and exit")
    args = parser.parse_args()

    if args.list:
        for name, scenario in SCENARIOS.items():
            print(f"{name:14} {(scenario.__doc__ or '').strip()}")
        return 0
    if sys.platform != "darwin":
        print("the probe drives macOS; run it on a Mac", file=sys.stderr)
        return 2
    unknown = [name for name in args.scenarios if name not in SCENARIOS]
    if unknown:
        print(f"no such scenario: {', '.join(unknown)} (see --list)", file=sys.stderr)
        return 2
    # Stopped from outside, the run still cleans up: the app was started by
    # open and the Host in its own session, so neither dies with this process.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    if not args.no_build:
        build()
    if not BUILT_APP.exists() or not GUACAD.exists():
        print("no test build; run without --no-build", file=sys.stderr)
        return 2

    # A banner, the permission question and a capture all need a lit
    # display, and a display that sleeps raises the lock screen's shield even
    # with no password asked. One assertion wakes the display, declares the
    # user active and keeps the display and the Mac awake from before the
    # first screen step to after the last, without touching the Mac's own
    # sleep settings. -w ends it with this process however that exits.
    awake = subprocess.Popen(["caffeinate", "-u", "-d", "-i", "-w", str(os.getpid())])
    results = []
    deferrals: list[dict] = []
    try:
        for name in args.scenarios or list(SCENARIOS):
            for index in range(1, args.runs + 1):
                record, entry = attempt(name, index, args.keep)
                while record.deferred and patient(args.patience, deferrals, entry):
                    record, entry = attempt(name, index, args.keep)
                results.append(entry)
                print(f"{name} run {index}: {'pass' if entry['pass'] else 'deferred' if entry['deferred'] else 'FAIL'}", file=sys.stderr)
    finally:
        awake.terminate()

    summary: dict = {"pass": all(r["pass"] for r in results), "deferrals": deferrals, "runs": results}
    summary["forgotten"] = sweep()
    by_scenario: dict[str, list[list]] = {}
    for result in results:
        by_scenario.setdefault(result["scenario"], []).append(
            [(s["step"], s.get("pass"), s.get("observed")) for s in result["steps"]]
        )
    summary["identical"] = {name: all(r == runs[0] for r in runs) for name, runs in by_scenario.items()}
    print(json.dumps(summary, indent=2))
    return 0 if summary["pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
