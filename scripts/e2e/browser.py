
from __future__ import annotations

import base64
import json
import os
import shutil
import subprocess
import threading
import time
from pathlib import Path
from typing import Any, Callable

CANDIDATES = ("chromium", "chromium-browser", "google-chrome-stable", "google-chrome")

SEEDED_RANDOM = """(() => {
  let seed = 0x6775616361;
  Math.random = () => {
    seed = (seed + 0x6D2B79F5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
})();"""

WINDOW = (1280, 860)
ACTIONABLE_SECONDS = 10


class BrowserError(Exception):
    pass


class BrowserGone(BrowserError):
    pass


def find_chromium() -> str | None:
    override = os.environ.get("GUACA_VERIFY_CHROMIUM")
    if override:
        return override if Path(override).exists() else None
    for name in CANDIDATES:
        found = shutil.which(name)
        if found:
            return found
    return None


def split_frames(buffer: bytes) -> tuple[list[bytes], bytes]:
    *whole, rest = buffer.split(b"\0")
    return [frame for frame in whole if frame], rest


class Browser:

    def __init__(self, profile: Path, binary: str | None = None, proxy: str | None = None) -> None:
        binary = binary or find_chromium()
        if not binary:
            raise BrowserError("no Chromium found; install one or set GUACA_VERIFY_CHROMIUM")
        profile.mkdir(parents=True, exist_ok=True)
        to_child_r, to_child_w = os.pipe()
        from_child_r, from_child_w = os.pipe()

        def wire() -> None:
            os.dup2(to_child_r, 3)
            os.dup2(from_child_w, 4)
            os.set_inheritable(3, True)
            os.set_inheritable(4, True)

        args = [
            binary,
            "--headless=new",
            "--remote-debugging-pipe",
            f"--user-data-dir={profile}",
            f"--window-size={WINDOW[0]},{WINDOW[1]}",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-networking",
            "--disable-background-timer-throttling",
            "--disable-renderer-backgrounding",
            "--disable-component-update",
            "--disable-sync",
            "--disable-extensions",
            "--disable-features=Translate,MediaRouter,OptimizationHints,NetworkTimeServiceQuerying",
            "--password-store=basic",
            "--use-mock-keychain",
            "--hide-scrollbars",
            "--mute-audio",
            "--force-color-profile=srgb",
            "--lang=en-US",
            "about:blank",
        ]
        if proxy:
            args.insert(1, f"--proxy-server={proxy}")
        if os.geteuid() == 0:
            args.insert(1, "--no-sandbox")
        self.log = profile.parent / "chromium.log"
        with open(self.log, "ab") as log:
            self.proc = subprocess.Popen(
                args,
                stdin=subprocess.DEVNULL,
                stdout=log,
                stderr=log,
                close_fds=False,
                preexec_fn=wire,
                start_new_session=True,
            )
        os.close(to_child_r)
        os.close(from_child_w)
        self._out = to_child_w
        self._in = from_child_r
        self._lock = threading.Lock()
        self._next = 0
        self._answers: dict[int, dict] = {}
        self._cond = threading.Condition()
        self._listeners: list[Callable[[dict], None]] = []
        self.session: str | None = None
        threading.Thread(target=self._read, daemon=True).start()


    def _read(self) -> None:
        rest = b""
        while True:
            try:
                chunk = os.read(self._in, 1 << 16)
            except OSError:
                chunk = b""
            if not chunk:
                with self._cond:
                    self._answers[-1] = {"error": {"message": "the browser closed its pipe"}}
                    self._cond.notify_all()
                return
            frames, rest = split_frames(rest + chunk)
            for frame in frames:
                message = json.loads(frame)
                if "id" in message:
                    with self._cond:
                        self._answers[message["id"]] = message
                        self._cond.notify_all()
                else:
                    for listener in list(self._listeners):
                        listener(message)

    def listen(self, listener: Callable[[dict], None]) -> None:
        self._listeners.append(listener)

    def send(self, method: str, params: dict | None = None, session: bool = True, timeout: float = 30) -> dict:
        with self._lock:
            self._next += 1
            ident = self._next
        message: dict[str, Any] = {"id": ident, "method": method, "params": params or {}}
        if session and self.session:
            message["sessionId"] = self.session
        try:
            os.write(self._out, json.dumps(message).encode() + b"\0")
        except BrokenPipeError:
            raise BrowserGone(f"{method}: {self.why_gone()}") from None
        deadline = time.monotonic() + timeout
        with self._cond:
            while ident not in self._answers:
                if -1 in self._answers:
                    raise BrowserGone(f"{method}: {self.why_gone()}")
                left = deadline - time.monotonic()
                if left <= 0:
                    raise BrowserError(f"{method}: no answer within {timeout:g}s")
                self._cond.wait(left)
            answer = self._answers.pop(ident)
        if "error" in answer:
            raise BrowserError(f"{method}: {answer['error'].get('message')}")
        return answer.get("result", {})

    def why_gone(self) -> str:
        try:
            code = self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            code = None
        lines = self.log.read_text(errors="replace").strip().splitlines() if self.log.exists() else []
        said = [line for line in lines if ":FATAL:" in line][:1] or lines[-5:]
        return f"the browser went away (exit {code}): " + (" | ".join(said) or "it said nothing")

    def open(self, url: str, within: float = 30) -> None:
        target = self.send("Target.createTarget", {"url": "about:blank"}, session=False, timeout=within)["targetId"]
        self.session = self.send(
            "Target.attachToTarget", {"targetId": target, "flatten": True}, session=False, timeout=within
        )["sessionId"]
        for domain in ("Page", "Runtime", "Network", "DOM", "Accessibility"):
            self.send(f"{domain}.enable")
        self.send("Emulation.setDeviceMetricsOverride",
                  {"width": WINDOW[0], "height": WINDOW[1], "deviceScaleFactor": 1, "mobile": False})
        self.send("Emulation.setTimezoneOverride", {"timezoneId": "UTC"})
        self.send("Emulation.setLocaleOverride", {"locale": "en-US"})
        self.send("Page.addScriptToEvaluateOnNewDocument", {"source": SEEDED_RANDOM})
        self.send("Page.navigate", {"url": url})

    def close(self) -> None:
        if self.proc.poll() is None:
            try:
                self.send("Browser.close", session=False, timeout=5)
            except BrowserError:
                pass
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        for fd in (self._out, self._in):
            try:
                os.close(fd)
            except OSError:
                pass


    def answer(self, pattern: str, body: Callable[[], Any]) -> None:
        def paused(message: dict) -> None:
            if message.get("method") != "Fetch.requestPaused":
                return
            fulfill = {
                "requestId": message["params"]["requestId"],
                "responseCode": 200,
                "responseHeaders": [{"name": "content-type", "value": "application/json"}],
                "body": base64.b64encode(json.dumps(body()).encode()).decode(),
            }
            threading.Thread(target=self.send, args=("Fetch.fulfillRequest", fulfill), daemon=True).start()

        self.listen(paused)
        self.send("Fetch.enable", {"patterns": [{"urlPattern": pattern, "requestStage": "Request"}]})

    def evaluate(self, expression: str) -> Any:
        result = self.send("Runtime.evaluate", {"expression": expression, "returnByValue": True})
        if "exceptionDetails" in result:
            raise BrowserError(f"evaluate: {result['exceptionDetails'].get('text')}")
        return result["result"].get("value")

    def ax_nodes(self) -> list[dict]:
        return self.send("Accessibility.getFullAXTree")["nodes"]

    def find(self, role: str, name: str, exact: bool = True) -> list[dict]:
        found = []
        for node in self.ax_nodes():
            if node.get("ignored") or "backendDOMNodeId" not in node:
                continue
            if (node.get("role") or {}).get("value") != role:
                continue
            label = str((node.get("name") or {}).get("value") or "")
            if (label == name) if exact else (name in label):
                found.append(node)
        return found

    def names(self, role: str) -> list[str]:
        return sorted({
            str((node.get("name") or {}).get("value") or "")
            for node in self.ax_nodes()
            if not node.get("ignored") and (node.get("role") or {}).get("value") == role
        })

    def names_within(self, container_role: str, container_name: str, role: str) -> list[str]:
        nodes = self.ax_nodes()
        by_id = {node["nodeId"]: node for node in nodes}
        roots = [node for node in nodes if (node.get("role") or {}).get("value") == container_role
                 and str((node.get("name") or {}).get("value") or "") == container_name]
        found, stack = [], [child for root in roots for child in root.get("childIds", [])]
        while stack:
            node = by_id.get(stack.pop())
            if node is None:
                continue
            stack.extend(node.get("childIds", []))
            if not node.get("ignored") and (node.get("role") or {}).get("value") == role:
                found.append(str((node.get("name") or {}).get("value") or ""))
        return sorted(found)

    def one(self, role: str, name: str, exact: bool = True) -> dict:
        found = self.find(role, name, exact)
        if len(found) == 1:
            return found[0]
        if not found:
            shown = ", ".join(repr(n) for n in self.names(role) if n) or "none"
            raise BrowserError(f"no {role} named {name!r}; {role}s shown: {shown}")
        raise BrowserError(f"{len(found)} {role}s named {name!r}; a step needs exactly one")

    def snapshot(self) -> str:
        nodes = self.ax_nodes()
        by_id = {node["nodeId"]: node for node in nodes}
        lines: list[str] = []

        def walk(node: dict, depth: int) -> None:
            role = (node.get("role") or {}).get("value") or ""
            name = str((node.get("name") or {}).get("value") or "")
            quiet = node.get("ignored") or role in ("generic", "none", "InlineTextBox", "LineBreak")
            if not quiet and (name or role not in ("StaticText", "paragraph", "group")):
                lines.append(f"{'  ' * depth}{role}" + (f" {json.dumps(name)}" if name else ""))
                depth += 1
            for child in node.get("childIds", []):
                if child in by_id:
                    walk(by_id[child], depth)

        for root in (node for node in nodes if not node.get("parentId")):
            walk(root, 0)
        return "\n".join(lines) + "\n"

    def text(self) -> str:
        return self.evaluate("document.body ? document.body.innerText : ''") or ""

    def screenshot(self, path: Path) -> None:
        data = self.send("Page.captureScreenshot", {"format": "png"})["data"]
        path.write_bytes(base64.b64decode(data))


    def _center(self, node: dict) -> tuple[float, float]:
        backend = node["backendDOMNodeId"]
        self.send("DOM.scrollIntoViewIfNeeded", {"backendNodeId": backend})
        quads = self.send("DOM.getContentQuads", {"backendNodeId": backend}).get("quads") or []
        if not quads:
            raise BrowserError("the element has no box on the page; it is not visible")
        quad = quads[0]
        return sum(quad[0::2]) / 4, sum(quad[1::2]) / 4

    def click(self, role: str, name: str, exact: bool = True) -> None:
        deadline = time.monotonic() + ACTIONABLE_SECONDS
        point = None
        while point is None:
            try:
                point = self._center(self.one(role, name, exact))
            except BrowserGone:
                raise
            except BrowserError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.1)
        x, y = point
        for kind in ("mouseMoved", "mousePressed", "mouseReleased"):
            self.send("Input.dispatchMouseEvent",
                      {"type": kind, "x": x, "y": y, "button": "left", "clickCount": 1})

    def type(self, role: str, name: str, text: str, exact: bool = True) -> None:
        self.click(role, name, exact)
        self.send("Input.insertText", {"text": text})

    def value(self, role: str, name: str, exact: bool = True) -> str:
        return str((self.one(role, name, exact).get("value") or {}).get("value") or "")

    def choose(self, role: str, name: str, option: str, exact: bool = True) -> None:
        self.send("DOM.focus", {"backendNodeId": self.one(role, name, exact)["backendDOMNodeId"]})
        for direction in ("ArrowUp", "ArrowDown"):
            for _ in range(32):
                if self.value(role, name, exact) == option:
                    return
                before = self.value(role, name, exact)
                self.key(direction)
                if self.value(role, name, exact) == before:
                    break
        raise BrowserError(f"{role} {name!r} has no option {option!r}")

    def fill(self, role: str, name: str, text: str, exact: bool = True) -> None:
        self.click(role, name, exact)
        for kind in ("keyDown", "keyUp"):
            self.send("Input.dispatchKeyEvent", {"type": kind, "key": "a", "code": "KeyA",
                                                 "windowsVirtualKeyCode": 65, "modifiers": 2,
                                                 "commands": ["selectAll"] if kind == "keyDown" else []})
        self.send("Input.insertText", {"text": text})

    def key(self, key: str) -> None:
        code, keycode, text = {"Enter": ("Enter", 13, "\r"), "Escape": ("Escape", 27, None),
                               "Tab": ("Tab", 9, None), "ArrowDown": ("ArrowDown", 40, None),
                               "ArrowUp": ("ArrowUp", 38, None)}[key]
        base: dict[str, Any] = {"key": key, "code": code, "windowsVirtualKeyCode": keycode}
        if text:
            base["text"] = text
        self.send("Input.dispatchKeyEvent", {"type": "keyDown", **base})
        self.send("Input.dispatchKeyEvent", {"type": "keyUp", **base})
