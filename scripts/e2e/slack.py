from __future__ import annotations

import base64
import hashlib
import json
import socket
import struct
import threading
import urllib.parse
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

from .world import until

GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
TEAM = "T0GUACA"
BOT_USER = "UGUACA"
BOT_ID = "BGUACA"
DM = "D0GUACA"
APP_METHODS = {"apps.connections.open"}


@dataclass
class Said:
    envelope: str
    channel: str
    ts: str


class Connection:

    def __init__(self, sock: socket.socket) -> None:
        self.sock = sock
        self.lock = threading.Lock()
        self.open = True

    def send(self, payload: dict, opcode: int = 0x1) -> bool:
        data = json.dumps(payload).encode() if opcode == 0x1 else b""
        return self.frame(opcode, data)

    def frame(self, opcode: int, data: bytes) -> bool:
        size = len(data)
        if size < 126:
            head = struct.pack("!BB", 0x80 | opcode, size)
        elif size < 65536:
            head = struct.pack("!BBH", 0x80 | opcode, 126, size)
        else:
            head = struct.pack("!BBQ", 0x80 | opcode, 127, size)
        with self.lock:
            try:
                self.sock.sendall(head + data)
                return True
            except OSError:
                self.open = False
                return False

    def drop(self) -> None:
        self.open = False
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass


def read_exact(sock: socket.socket, size: int) -> bytes:
    data = b""
    while len(data) < size:
        chunk = sock.recv(size - len(data))
        if not chunk:
            raise ConnectionError("closed")
        data += chunk
    return data


def read_frame(sock: socket.socket) -> tuple[int, bytes]:
    first, second = read_exact(sock, 2)
    size = second & 0x7F
    if size == 126:
        size = struct.unpack("!H", read_exact(sock, 2))[0]
    elif size == 127:
        size = struct.unpack("!Q", read_exact(sock, 8))[0]
    mask = read_exact(sock, 4) if second & 0x80 else b"\0\0\0\0"
    data = bytes(byte ^ mask[index % 4] for index, byte in enumerate(read_exact(sock, size)))
    return first & 0x0F, data


def fields_of(body: bytes, kind: str) -> dict:
    if "json" in kind:
        try:
            parsed = json.loads(body or b"{}")
            return parsed if isinstance(parsed, dict) else {}
        except ValueError:
            return {}
    return {key: values[-1] for key, values in urllib.parse.parse_qs(body.decode(errors="replace")).items()}


def ts_order(ts: str) -> tuple[int, int]:
    seconds, _, fraction = ts.partition(".")
    return int(seconds or 0), int((fraction + "000000")[:6])


class Slack:

    def __init__(self, evidence: Path | None = None) -> None:
        self.requests: list[dict] = []
        self.scripted: dict[str, list[tuple[int, dict, dict]]] = {}
        self.history: dict[str, list[dict]] = {}
        self.connections: list[Connection] = []
        self.revoked = False
        self._acks: list[str] = []
        self._lock = threading.Lock()
        self._clock = 1_700_000_000
        self._serial = 0
        self._log = evidence / "slack.jsonl" if evidence else None
        slack = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *_: Any) -> None:
                pass

            def do_POST(self) -> None:
                length = int(self.headers.get("content-length", 0))
                body = self.rfile.read(length)
                method = self.path.removeprefix("/api/").split("?")[0]
                bearer = self.headers.get("authorization", "").removeprefix("Bearer ").strip()
                fields = fields_of(body, self.headers.get("content-type", ""))
                status, answer, headers = slack.answer(method, bearer, fields)
                data = json.dumps(answer).encode() if status == 200 else b""
                self.send_response(status)
                for name, value in headers.items():
                    self.send_header(name, value)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self) -> None:
                key = self.headers.get("sec-websocket-key")
                if self.path != "/link" or not key:
                    self.send_response(404)
                    self.send_header("content-length", "0")
                    self.end_headers()
                    return
                accept = base64.b64encode(hashlib.sha1((key + GUID).encode()).digest()).decode()
                self.send_response(101)
                self.send_header("upgrade", "websocket")
                self.send_header("connection", "Upgrade")
                self.send_header("sec-websocket-accept", accept)
                self.end_headers()
                self.wfile.flush()
                self.close_connection = True
                slack.hold(Connection(self.connection))

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        port = self.server.server_address[1]
        self.origin = f"http://127.0.0.1:{port}"
        self.socket_url = f"ws://127.0.0.1:{port}/link"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def record(self, entry: dict) -> None:
        if self._log:
            with self._log.open("a") as log:
                log.write(json.dumps(entry) + "\n")

    def next_ts(self) -> str:
        with self._lock:
            self._serial += 1
            return f"{self._clock}.{self._serial:06}"

    def answer(self, method: str, bearer: str, fields: dict) -> tuple[int, dict, dict]:
        token = "app" if bearer.startswith("xapp-") else "bot" if bearer.startswith("xoxb-") else "none"
        entry = {"method": method, "token": token, "fields": fields}
        with self._lock:
            self.requests.append(entry)
            queued = self.scripted.get(method) or []
            scripted = queued.pop(0) if queued else None
        self.record({"request": entry})
        if scripted:
            return scripted
        if self.revoked:
            return 200, {"ok": False, "error": "token_revoked"}, {}
        wanted = "app" if method in APP_METHODS else "bot"
        if token != wanted:
            return 200, {"ok": False, "error": "invalid_auth"}, {}
        return 200, self.web(method, fields), {}

    def web(self, method: str, fields: dict) -> dict:
        channel = str(fields.get("channel", ""))
        if method == "apps.connections.open":
            return {"ok": True, "url": self.socket_url}
        if method == "auth.test":
            return {"ok": True, "user_id": BOT_USER, "team_id": TEAM}
        if method == "chat.postMessage":
            ts = self.next_ts()
            message = {"type": "message", "ts": ts, "bot_id": BOT_ID, "user": BOT_USER,
                       "text": fields.get("text", ""), "blocks": fields.get("blocks", [])}
            if fields.get("thread_ts"):
                message["thread_ts"] = fields["thread_ts"]
            if fields.get("metadata"):
                message["metadata"] = fields["metadata"]
            self.keep(channel, message)
            return {"ok": True, "channel": channel, "ts": ts, "message": message}
        if method == "chat.update":
            for message in self.history.get(channel, []):
                if message["ts"] == fields.get("ts"):
                    message.update(text=fields.get("text", ""), blocks=fields.get("blocks", []))
            return {"ok": True, "channel": channel, "ts": fields.get("ts")}
        if method == "conversations.history":
            oldest = ts_order(str(fields.get("oldest", "0")))
            messages = [m for m in self.history.get(channel, [])
                        if "thread_ts" not in m or m["thread_ts"] == m["ts"]]
            messages = [m for m in messages if ts_order(m["ts"]) > oldest]
            return {"ok": True, "messages": self.shown(sorted(messages, key=lambda m: ts_order(m["ts"]), reverse=True), fields),
                    "response_metadata": {"next_cursor": ""}}
        if method == "conversations.replies":
            root = str(fields.get("ts", ""))
            oldest = ts_order(str(fields.get("oldest", "0")))
            messages = [m for m in self.history.get(channel, []) if m["ts"] == root or m.get("thread_ts") == root]
            messages = [m for m in messages if m["ts"] == root or ts_order(m["ts"]) > oldest]
            return {"ok": True, "messages": self.shown(sorted(messages, key=lambda m: ts_order(m["ts"])), fields),
                    "response_metadata": {"next_cursor": ""}}
        return {"ok": True}

    def shown(self, messages: list[dict], fields: dict) -> list[dict]:
        if str(fields.get("include_all_metadata", "")).lower() == "true":
            return [dict(m) for m in messages]
        return [{k: v for k, v in m.items() if k != "metadata"} for m in messages]

    def keep(self, channel: str, message: dict) -> None:
        with self._lock:
            history = self.history.setdefault(channel, [])
            history.append(message)
            root = message.get("thread_ts")
            if root and root != message["ts"]:
                for parent in history:
                    if parent["ts"] == root:
                        parent["thread_ts"] = root
                        parent["reply_count"] = parent.get("reply_count", 0) + 1

    def hold(self, connection: Connection) -> None:
        with self._lock:
            self.connections.append(connection)
        self.record({"connected": len(self.connections)})
        connection.send({"type": "hello", "num_connections": 1})
        try:
            while connection.open:
                opcode, data = read_frame(connection.sock)
                if opcode == 0x8:
                    connection.frame(0x8, b"")
                    break
                if opcode == 0x9:
                    connection.frame(0xA, data)
                    continue
                if opcode != 0x1:
                    continue
                try:
                    envelope = json.loads(data).get("envelope_id")
                except ValueError:
                    envelope = None
                if envelope:
                    with self._lock:
                        self._acks.append(envelope)
                    self.record({"ack": envelope})
        except (ConnectionError, OSError):
            pass
        connection.open = False

    def live(self) -> Connection:
        return until("a Socket Mode connection", lambda: next(
            (c for c in reversed(self.connections) if c.open), None), timeout=30)

    def push(self, envelope: dict) -> str:
        if not self.live().send(envelope):
            raise ConnectionError("the Socket Mode connection closed while pushing")
        self.record({"pushed": envelope})
        return envelope.get("envelope_id", "")

    def envelope_id(self) -> str:
        with self._lock:
            self._serial += 1
            return f"env-{self._serial}"

    def say(self, text: str, user: str, channel: str = DM, thread: str | None = None) -> Said:
        ts = self.next_ts()
        event = {"type": "message", "channel": channel, "user": user, "text": text, "ts": ts,
                 "channel_type": "im" if channel.startswith("D") else "channel"}
        if thread:
            event["thread_ts"] = thread
        self.keep(channel, {k: v for k, v in event.items() if k not in ("channel", "channel_type")})
        envelope = self.envelope_id()
        self.push({"type": "events_api", "envelope_id": envelope, "accepts_response_payload": False,
                   "retry_attempt": 0,
                   "payload": {"type": "event_callback", "team_id": TEAM, "event": event,
                               "authorizations": [{"team_id": TEAM, "user_id": BOT_USER, "is_bot": True}]}})
        return Said(envelope, channel, ts)

    def press(self, action_id: str, user: str, channel: str | None = None, message_ts: str | None = None) -> str:
        card = self.card(action_id, channel, message_ts)
        container = {"type": "message", "channel_id": card["channel"], "message_ts": card["ts"]}
        if card.get("thread_ts"):
            container["thread_ts"] = card["thread_ts"]
        envelope = self.envelope_id()
        self.push({"type": "interactive", "envelope_id": envelope, "accepts_response_payload": False,
                   "payload": {"type": "block_actions", "team": {"id": TEAM}, "user": {"id": user},
                               "channel": {"id": card["channel"]}, "container": container,
                               "actions": [{"type": "button", "action_id": action_id,
                                            "value": action_id.rsplit(":", 1)[-1],
                                            "action_ts": self.next_ts()}]}})
        return envelope

    def card(self, action_id: str, channel: str | None, message_ts: str | None) -> dict:
        for post in reversed(self.posts()):
            buttons = [element.get("action_id") for block in post.get("blocks") or [] if block.get("type") == "actions"
                       for element in block.get("elements") or []]
            if action_id in buttons or (channel and message_ts and post.get("channel") == channel):
                if message_ts and post.get("ts") != message_ts:
                    continue
                return post
        if channel and message_ts:
            return {"channel": channel, "ts": message_ts}
        raise LookupError(f"no posted card has a button {action_id}")

    def posts(self) -> list[dict]:
        with self._lock:
            found = []
            for channel, messages in self.history.items():
                for message in messages:
                    if message.get("bot_id") == BOT_ID:
                        found.append({**message, "channel": channel})
            return sorted(found, key=lambda m: ts_order(m["ts"]))

    def acks(self) -> list[str]:
        with self._lock:
            return list(self._acks)

    def calls(self, method: str) -> list[dict]:
        with self._lock:
            return [entry["fields"] for entry in self.requests if entry["method"] == method]

    def script(self, method: str, status: int = 200, body: dict | None = None,
               headers: dict | None = None) -> None:
        with self._lock:
            self.scripted.setdefault(method, []).append((status, body or {"ok": True}, headers or {}))

    def disconnect(self) -> None:
        self.live().send({"type": "disconnect", "reason": "refresh_requested"})

    def drop(self) -> None:
        self.live().drop()

    def stop(self) -> None:
        for connection in self.connections:
            connection.drop()
        self.server.shutdown()
        self.server.server_close()
