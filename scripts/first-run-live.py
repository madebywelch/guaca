#!/usr/bin/env python3
from __future__ import annotations

USAGE = """Run the first run's suggested tasks against a live model, on purpose.

    OPENROUTER_API_KEY=... scripts/first-run-live.py                  each task once
    OPENROUTER_API_KEY=... scripts/first-run-live.py --runs 3 plan    one task three times
    GUACA_TEST_MODEL=anthropic/claude-haiku-4.5 scripts/first-run-live.py write

Tasks: research, plan, write. Each run starts a fresh Host (scripts/verify.py
build makes it) and a fresh browser, picks the task, keeps the pre-filled
OpenRouter card, types only the key, and presses Start task. It reports the
seconds from Start to the first For you row, the model calls, tokens and cost
the Host recorded, and the full text of the result.

The Host may reach openrouter.ai and nothing else. Every run spends money, so
neither ./scripts/ci.sh nor CI runs this. The key is read from the environment,
typed into the page, and redacted from everything the run leaves behind.
"""

import json
import os
import select
import socket
import statistics
import sys
import tempfile
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from e2e.scenarios import API_KEY, WINDOW, between, outcomes, said
from e2e.world import GUACAD, WEB, World, redact

ALLOWED = "openrouter.ai:443"
TASKS = {
    "research": ("Research a decision Compare options and make the tradeoffs clear.", "Financial Analyst"),
    "plan": ("Plan a project Turn an idea into a manageable first step.", "Product Manager"),
    "write": ("Draft content Find the words for something you need to say.", "Content Marketer"),
}
FIRST_ROW = """(() => {
  const row = document.querySelector('.for-you .finished-line');
  return row ? { headline: row.title, who: row.querySelector('.finished-line__who')?.innerText.trim() } : null;
})()"""


class OpenRouterOnly:

    def __init__(self) -> None:
        self.attempts: list[str] = []
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
            threading.Thread(target=self._tunnel, args=(conn,), daemon=True).start()

    def _tunnel(self, conn: socket.socket) -> None:
        with conn:
            conn.settimeout(10)
            head = b""
            try:
                while b"\r\n\r\n" not in head and len(head) < 8192:
                    chunk = conn.recv(4096)
                    if not chunk:
                        return
                    head += chunk
            except OSError:
                return
            method, _, rest = head.split(b"\r\n", 1)[0].decode(errors="replace").partition(" ")
            target = rest.split(" ")[0]
            self.attempts.append(f"{method} {target}")
            if method != "CONNECT" or target != ALLOWED:
                conn.sendall(b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                return
            try:
                upstream = socket.create_connection(("openrouter.ai", 443), timeout=10)
            except OSError:
                conn.sendall(b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                return
            with upstream:
                conn.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
                conn.settimeout(None)
                upstream.settimeout(None)
                ends = [conn, upstream]
                while self._open:
                    ready, _, _ = select.select(ends, [], [], 1)
                    for end in ready:
                        try:
                            data = end.recv(65536)
                        except OSError:
                            return
                        if not data:
                            return
                        (upstream if end is conn else conn).sendall(data)

    def stop(self) -> None:
        self._open = False
        self.sock.close()


def run(task: str, n: int, key: str, model: str, evidence: Path) -> dict:
    title, agent = TASKS[task]
    w = World(f"live-{task}", n, evidence)
    w.secrets.append(key)
    w.egress.stop()
    w.egress = w.host.egress = OpenRouterOnly()
    try:
        w.start()
        ui = w.ui
        ui.viewport(*WINDOW)
        w.wait("the first visit offers the task", lambda: ui.find("button", title))
        ui.click("button", title)
        ui.click("button", "Continue")
        w.wait("the Connect your AI step", lambda: ui.find("button", "Test and continue"))
        ui.click("button", API_KEY)
        w.wait("the pre-filled OpenRouter card", lambda: ui.value("textbox", "Inference endpoint") == "https://openrouter.ai/api/v1")
        if model:
            ui.fill("textbox", "Default model", model)
        ui.fill("textbox", "API key", key, exact=False)
        ui.click("button", "Test and continue")
        w.wait("the key test passes", lambda: ui.find("button", "Start task"), 120)
        started = time.monotonic()
        ui.click("button", "Start task")
        w.wait("Start task opens the agent's conversation", lambda: ui.find("log", f"Conversation with {agent}"))
        ui.click("button", "For you")
        row = w.wait("For you lists the first result", lambda: ui.evaluate(FIRST_ROW), 300)
        seconds = time.monotonic() - started
        ended = w.wait("the task settles", lambda: outcomes(w), 120)
        usage = w.host.rows("SELECT model, prompt, completion, cached, cost FROM usage")
        return {
            "task": task, "run": n, "seconds": round(seconds, 1),
            "outcome": ended[-1]["status"], "forYou": row,
            "model": sorted({u["model"] for u in usage}),
            "calls": len(usage),
            "tokensIn": sum(u["prompt"] for u in usage),
            "tokensOut": sum(u["completion"] for u in usage),
            "cached": sum(u["cached"] for u in usage),
            "cost": round(sum(u["cost"] or 0 for u in usage), 4),
            "agentMessages": len(between(w)),
            "questions": w.host.scalar("SELECT count(*) FROM decisions"),
            "refused": [a for a in w.egress.attempts if not a.endswith(ALLOWED)],
            "result": "\n\n".join(said(w, agent)),
            "tools": tool_calls(w),
        }
    finally:
        w.close()


def tool_calls(w: World) -> list[dict]:
    found = []
    for row in w.host.rows("SELECT parts FROM messages ORDER BY created_at, rowid"):
        for part in json.loads(row["parts"]):
            if part.get("type") == "toolCall":
                outcome = part.get("outcome") or {}
                found.append({"name": part.get("name"), "arguments": part.get("arguments"),
                              "outcome": outcome.get("summary") or outcome.get("error") or outcome.get("reason")})
    return found


def report(r: dict) -> str:
    if "error" in r:
        return f"== {r['task']} run {r['run']}: FAILED: {r['error']}\n"
    head = (f"== {r['task']} run {r['run']}: {r['seconds']} s from Start to the first For you row, "
            f"{r['calls']} model call{'s' if r['calls'] != 1 else ''} ({', '.join(r['model'])}), "
            f"{r['tokensIn']:,} tokens in ({r['cached']:,} cached), {r['tokensOut']:,} out, ${r['cost']:.4f}, "
            f"outcome {r['outcome']}, {r['agentMessages']} agent messages, {r['questions']} questions")
    if r["refused"]:
        head += f", refused egress {r['refused']}"
    tools = "".join(f"Tool: {t['name']} {json.dumps(t['arguments'])} -> {t['outcome']}\n" for t in r["tools"])
    return f"{head}\nFor you: {r['forYou']['headline']}\n{tools}\n{r['result']}\n"


def main(argv: list[str]) -> int:
    if any(arg in ("-h", "--help") for arg in argv):
        print(USAGE)
        return 0
    runs, tasks, as_json = 1, [], False
    args = iter(argv)
    for arg in args:
        if arg == "--runs":
            runs = int(next(args))
        elif arg == "--json":
            as_json = True
        elif arg in TASKS:
            tasks.append(arg)
        else:
            print(f"unknown argument {arg!r}\n\n{USAGE}", file=sys.stderr)
            return 2
    key = os.environ.get("OPENROUTER_API_KEY", "").strip()
    if not key:
        print("OPENROUTER_API_KEY is not set; this script spends real money and needs it", file=sys.stderr)
        return 2
    missing = [str(p) for p in (GUACAD, WEB / "index.html") if not p.exists()]
    if missing:
        print(f"not built: {', '.join(missing)}; run scripts/verify.py build", file=sys.stderr)
        return 2
    model = os.environ.get("GUACA_TEST_MODEL", "").strip()
    results = []
    with tempfile.TemporaryDirectory(prefix="guaca-live-") as evidence:
        for task in tasks or list(TASKS):
            for n in range(1, runs + 1):
                try:
                    r = run(task, n, key, model, Path(evidence))
                except Exception as error:
                    r = {"task": task, "run": n, "error": redact(f"{type(error).__name__}: {error}", [key])}
                results.append(r)
                print(redact(json.dumps(r) if as_json else report(r), [key]), flush=True)
    finished = [r for r in results if "error" not in r]
    if not as_json and finished:
        medians = {task: statistics.median(r["seconds"] for r in finished if r["task"] == task)
                   for task in dict.fromkeys(r["task"] for r in finished)}
        print("medians: " + ", ".join(f"{task} {seconds:.1f} s" for task, seconds in medians.items())
              + f"; total ${sum(r['cost'] for r in finished):.4f} over {len(finished)} runs")
    return 1 if len(finished) < len(results) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
