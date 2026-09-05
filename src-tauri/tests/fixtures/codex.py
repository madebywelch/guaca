#!/usr/bin/env python3
"""Offline app-server peer. Every line is a real v2 request/response shape."""
import json
import os
from pathlib import Path
import sys

if "--version" in sys.argv:
    print("codex-cli 0.153.3")
    raise SystemExit()
Path(".argv").write_text("".join(arg + "\n<<>>\n" for arg in sys.argv[1:]))
if Path(".noisy").exists():
    os.write(2, b"x" * (256 * 1024))

thread = "codex-session"
turn = "codex-turn"
held = Path(".codex_hold").exists()
gated = Path(".codex_gate").exists()


def send(value):
    print(json.dumps(value), flush=True)


def reply(request, result):
    send({"id": request["id"], "result": result})


def item(kind, **fields):
    send({"method": "item/completed", "params": {"threadId": thread, "turnId": turn, "item": {"id": kind, "type": kind, **fields}}})


def finish():
    item("agentMessage", text="Fixed the flaky test and pushed.")
    failed = Path(".codex_failure").exists()
    send({"method": "turn/completed", "params": {"threadId": thread, "turn": {"id": turn, "status": "failed" if failed else "completed", "error": {"message": "fixture failed after editing"} if failed else None}}})


for raw in sys.stdin:
    request = json.loads(raw)
    with Path(".rpc.jsonl").open("a") as log:
        log.write(raw)
    method = request.get("method")
    if method == "initialize":
        reply(request, {"userAgent": "fixture"})
    elif method == "thread/start":
        policy = "never" if Path(".codex_bad_policy").exists() else request["params"]["approvalPolicy"]
        reply(request, {"thread": {"id": thread}, "model": "fixture-mini", "approvalPolicy": policy, "approvalsReviewer": "user"})
    elif method == "turn/start":
        reply(request, {"turn": {"id": turn, "status": "inProgress"}})
        send({"method": "turn/started", "params": {"threadId": thread, "turn": {"id": turn}}})
        if Path(".codex_early").exists():
            raise SystemExit()
        item("commandExecution", command="npm test")
        if gated:
            command = Path(".codex_command").read_text() if Path(".codex_command").exists() else "git push origin HEAD"
            send({"id": 90, "method": "item/commandExecution/requestApproval", "params": {"threadId": thread, "turnId": turn, "itemId": "push", "command": command, "cwd": os.getcwd()}})
        elif not held:
            finish()
    elif method == "turn/steer":
        assert request["params"]["threadId"] == thread
        assert request["params"]["expectedTurnId"] == turn
        if Path(".codex_finish_before_ack").exists():
            finish()
            send({"id": request["id"], "error": {"code": -32600, "message": "Turn already completed"}})
        elif Path(".codex_reject_steer").exists():
            send({"id": request["id"], "error": {"code": -32600, "message": "Correction rejected by fixture"}})
        else:
            reply(request, {"turnId": turn})
            Path(".steered").write_text(request["params"]["input"][0]["text"])
            if not gated:
                finish()
    elif request.get("id") == 90:
        if request["result"]["decision"] == "accept":
            Path(".pushed").write_text("accepted")
        Path(".verdict").write_text(request["result"]["decision"])
        finish()
