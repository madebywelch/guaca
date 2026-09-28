#!/usr/bin/env python3
"""Offline Claude Code peer, in `-p --input-format stream-json` mode. Every line
is a real event, control request or control response shape, as measured against
Claude Code 2.1.283.

Driven by files in the directory it is started in, like the other stand-ins,
because the environment is process-wide and these tests run concurrently:

  .argv           written: every argument, one per `<<>>` separator
  .claude_prompt  written: the first user message it was sent, whole
  .say            printed verbatim in place of the canned run
  .exit           the exit status after `.say`
  .linger         seconds the tool call takes, reading an interrupt meanwhile
  .noisy          256 KB on stderr
  .secret_probe   the answer is CLOUDFLARE_API_TOKEN's value
  .interrupted    written: an interrupt arrived mid-turn

Like the real program in this mode, it does not exit after `result`: it waits
for more input, and ends when its stdin is closed. A driver that forgets to
close it holds the job until the ceiling, which is what the suite would see.
"""
import json
import os
import queue
import sys
import threading
import time
from pathlib import Path

if "--version" in sys.argv:
    print("stand-in")
    raise SystemExit()
Path(".argv").write_text("".join(arg + "\n<<>>\n" for arg in sys.argv[1:]))
if Path(".noisy").exists():
    os.write(2, b"x" * (256 * 1024))

incoming = queue.Queue()


def read():
    for raw in sys.stdin:
        incoming.put(json.loads(raw))
    incoming.put(None)


threading.Thread(target=read, daemon=True).start()


def send(value):
    print(json.dumps(value), flush=True)


def wait_for_eof():
    while incoming.get() is not None:
        pass


def result(text, subtype="success", error=False, **extra):
    send({"type": "result", "subtype": subtype, "is_error": error, "result": text, "total_cost_usd": 0.12, **extra})


first = incoming.get()
if first is not None and first.get("type") == "control_request" and first["request"].get("subtype") == "initialize":
    # What the SDK learns `/model`'s list from, cut to three.
    send({"type": "control_response", "response": {"subtype": "success", "request_id": first["request_id"], "response": {"models": [
        {"value": "default", "resolvedModel": "claude-fable-5-1", "displayName": "Default (recommended)", "supportedEffortLevels": ["low", "medium", "high", "xhigh", "max"]},
        {"value": "claude-fable-5-1", "resolvedModel": "claude-fable-5-1", "displayName": "Fable 5.1", "description": "For your toughest challenges", "supportedEffortLevels": ["low", "medium", "high", "xhigh", "max"]},
        {"value": "haiku", "resolvedModel": "claude-haiku-4-5-20251001", "displayName": "Haiku 4.5", "description": "Fastest for quick answers"},
    ]}}})
    wait_for_eof()
    raise SystemExit()
if first is None or first.get("type") != "user":
    sys.stderr.write("expected a user message first\n")
    raise SystemExit(2)
content = first["message"]["content"]
if isinstance(content, list):
    content = "".join(part.get("text", "") for part in content)
Path(".claude_prompt").write_text(content)

if Path(".say").exists():
    said = Path(".say").read_text()
    # The program ends every event with a newline; a test's file may not.
    sys.stdout.write(said if said.endswith("\n") else said + "\n")
    sys.stdout.flush()
    if Path(".exit").exists():
        raise SystemExit(int(Path(".exit").read_text().strip()))
    if '"result"' in said:
        wait_for_eof()
    raise SystemExit()

if Path(".secret_probe").exists():
    value = os.environ["CLOUDFLARE_API_TOKEN"]
    send({"type": "assistant", "message": {"model": "claude-opus-5", "content": [{"type": "text", "text": value}]}})
    result(value)
    wait_for_eof()
    raise SystemExit()

send({"type": "system", "subtype": "init", "model": "claude-opus-5", "session_id": "stand-in"})
send({"type": "assistant", "message": {"model": "claude-opus-5", "content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "npm test"}}]}})
if Path(".linger").exists():
    until = time.time() + float(Path(".linger").read_text().strip())
    while time.time() < until:
        try:
            command = incoming.get(timeout=0.05)
        except queue.Empty:
            continue
        if command is None:
            raise SystemExit(1)
        if command.get("type") == "control_request" and command["request"].get("subtype") == "interrupt":
            Path(".interrupted").write_text("interrupted")
            send({"type": "control_response", "response": {"subtype": "success", "request_id": command["request_id"], "response": {"still_queued": []}}})
            result("", "error_during_execution", True, stop_reason="tool_use", terminal_reason="aborted_streaming")
            wait_for_eof()
            raise SystemExit(1)
result("Fixed the flaky test and pushed.")
wait_for_eof()
