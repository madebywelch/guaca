#!/usr/bin/env python3
"""Offline pi RPC peer. Every line is a real `pi --mode rpc` command, response or
event shape, as measured against pi 0.84.4.

Driven by files in the directory it is started in, like the other stand-ins,
because the environment is process-wide and these tests run concurrently:

  .argv          written: every argument, one per `<<>>` separator
  .say           printed verbatim in place of the canned turn
  .exit          the exit status after `.say`
  .linger        seconds the turn takes, reading steer and abort meanwhile
  .noisy         256 KB on stderr
  .secret_probe  the answer is CLOUDFLARE_API_TOKEN's value
  .pi_command    the command the gated bash call asks about
  .pi_no_gate    the gate extension is named but does not load
  .steered       written: the last steer
  .aborted       written: an abort arrived mid-turn
  .verdict       written: `confirmed` or `declined`, from the gate
  .pushed        written: the gated command was allowed to run
  .pi_history    written: one line per prompt this session has had
  .pi_prompt     written: the last prompt, whole
  .extensions    written: every extension it was handed, whole
  .relayed       written: status and start of the answer, when a provider
                 extension readdressed a provider and the turn called it

A provider an extension registers is called for real: the turn posts one chat
completion to the `baseUrl` it names with the `apiKey` it names, which is what
pi does with an override, so a test can watch the call reach the other end.

A `--model` it has no entry for is run rather than refused, as pi runs one: its
provider's default, renamed, with the default's limits. `get_state` reports
that copy and `get_available_models` does not list it.

  .pi_catalog_late  the catalog lists the model, with its own limits, but only
                    after the copy was settled on, as pi's first download of
                    its catalog does
  .model_set        written: the entry `set_model` put in the copy's place
"""
import json
import os
import queue
import re
import sys
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

if "--version" in sys.argv:
    print("0.84.4")
    raise SystemExit()
Path(".argv").write_text("".join(arg + "\n<<>>\n" for arg in sys.argv[1:]))
if Path(".noisy").exists():
    os.write(2, b"x" * (256 * 1024))

args = sys.argv[1:]
session = args[args.index("--session-id") + 1] if "--session-id" in args else ""
extensions = [Path(args[i + 1]).read_text() for i, arg in enumerate(args) if arg == "-e"]
Path(".extensions").write_text("\n".join(extensions))
gated = any("guaca-gate" in source for source in extensions) and not Path(".pi_no_gate").exists()
provider = None
for source in extensions:
    found = re.search(r'registerProvider\("([^"]+)", \{.*?baseUrl: ("[^"]*"), apiKey: ("[^"]*")', source, re.S)
    if found:
        provider = {"id": found.group(1), "baseUrl": json.loads(found.group(2)), "apiKey": json.loads(found.group(3)),
                    "models": [json.loads(id) for id in re.findall(r'\{ id: ("[^"]*")', source)]}
model = args[args.index("--model") + 1] if "--model" in args else ""


def entry(id, owner, name, reasoning=False, context=128000, output=16384):
    return {"id": id, "provider": owner, "name": name, "reasoning": reasoning, "contextWindow": context, "maxTokens": output}


def asked():
    if "--provider" in args:
        return args[args.index("--provider") + 1], model
    owner, _, name = model.partition("/")
    return owner, name


def catalog(settled=False):
    models = [entry("claude-opus-4-7", "anthropic", "Claude Opus 4.7", True, 1000000, 128000)]
    if provider and provider["id"] == "openrouter":
        models.append(entry("moonshotai/kimi-k2.6", "openrouter", "Kimi K2.6", True, 262144, 235929))
        models.append(entry("qwen/qwen3-coder", "openrouter", "Qwen3 Coder"))
    elif provider:
        models += [entry(id, provider["id"], id) for id in provider["models"]]
    if Path(".pi_catalog_late").exists() and not settled:
        owner, name = asked()
        models.append(entry(name, owner, name, True, 1048576, 131072))
    return models


def resolved():
    """The model pi settles on at start, before the catalog it downloads lands."""
    known = catalog(settled=True)
    if not model:
        return known[0]
    owner, name = asked()
    found = next((m for m in known if m["provider"] == owner and m["id"] == name), None)
    if found:
        return found
    default = next((m for m in known if m["provider"] == owner), known[0])
    return {**default, "id": name, "name": name, "provider": owner}
incoming = queue.Queue()


def read():
    for raw in sys.stdin:
        incoming.put(json.loads(raw))
    incoming.put(None)


threading.Thread(target=read, daemon=True).start()


def send(value):
    print(json.dumps(value), flush=True)


def respond(command, **extra):
    send({"id": command.get("id"), "type": "response", "command": command["type"], "success": True, **extra})


def assistant(text, stop="stop"):
    send({"type": "message_end", "message": {"role": "assistant", "model": "gpt-5.6", "content": [{"type": "text", "text": text}] if text else [], "stopReason": stop}})


def settle():
    send({"type": "agent_end", "messages": [], "willRetry": False})
    send({"type": "agent_settled"})


def handle_aside(command):
    """A command that arrives while a turn is running. True means stop."""
    kind = command["type"]
    if kind == "steer":
        Path(".steered").write_text(command["message"])
        respond(command)
    elif kind == "abort":
        Path(".aborted").write_text("aborted")
        respond(command)
        return True
    elif kind == "extension_ui_response":
        pending.append(command)
    return False


pending = []


def wait_for_dialog(dialog):
    while True:
        for held in list(pending):
            if held.get("id") == dialog:
                pending.remove(held)
                return held
        command = incoming.get()
        if command is None:
            raise SystemExit()
        if command["type"] == "extension_ui_response" and command.get("id") == dialog:
            return command
        if handle_aside(command):
            return None


def call_provider():
    body = json.dumps({"model": model, "messages": [{"role": "user", "content": "relay probe"}], "stream": True}).encode()
    request = urllib.request.Request(provider["baseUrl"] + "/chat/completions", data=body, method="POST",
                                     headers={"authorization": "Bearer " + provider["apiKey"], "content-type": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=10) as answer:
            status, text = answer.status, answer.read().decode()
    except urllib.error.HTTPError as refused:
        status, text = refused.code, refused.read().decode()
    Path(".relayed").write_text(json.dumps({"status": status, "body": text[:400]}))


def turn(prompt):
    Path(".pi_prompt").write_text(prompt["message"])
    if provider:
        call_provider()
    with Path(".pi_history").open("a") as history:
        history.write(session + " " + prompt["message"].replace("\n", " ")[:200] + "\n")
    if Path(".say").exists():
        sys.stdout.write(Path(".say").read_text())
        sys.stdout.flush()
        if Path(".exit").exists():
            raise SystemExit(int(Path(".exit").read_text().strip()))
        settle()
        return
    send({"type": "agent_start"})
    send({"type": "tool_execution_start", "toolCallId": "call_1", "toolName": "bash", "args": {"command": "npm test"}})
    if gated:
        command = Path(".pi_command").read_text() if Path(".pi_command").exists() else "git push origin HEAD"
        send({"type": "extension_ui_request", "id": "gate-1", "method": "confirm", "title": "guaca:gate", "message": command})
        answer = wait_for_dialog("gate-1")
        if answer is None:
            assistant("", "aborted")
            settle()
            return
        allowed = answer.get("confirmed") is True
        Path(".verdict").write_text("confirmed" if allowed else "declined")
        if allowed:
            Path(".pushed").write_text("allowed")
    if Path(".linger").exists():
        until = time.time() + float(Path(".linger").read_text().strip())
        while time.time() < until:
            try:
                command = incoming.get(timeout=0.05)
            except queue.Empty:
                continue
            if command is None:
                raise SystemExit()
            if handle_aside(command):
                assistant("", "aborted")
                settle()
                return
    text = os.environ["CLOUDFLARE_API_TOKEN"] if Path(".secret_probe").exists() else "Fixed the flaky test and pushed."
    send({"type": "message_update", "usage": {"cost": {"total": 0.12}}, "assistantMessageEvent": {"type": "text_delta", "contentIndex": 0, "delta": text}})
    assistant(text)
    settle()


while True:
    command = incoming.get()
    if command is None:
        break
    kind = command["type"]
    if kind == "get_available_models":
        respond(command, data={"models": catalog()})
    elif kind == "get_state":
        respond(command, data={"model": resolved(), "sessionId": session, "isStreaming": False})
    elif kind == "set_model":
        found = next((m for m in catalog() if m["provider"] == command["provider"] and m["id"] == command["modelId"]), None)
        if found:
            Path(".model_set").write_text(json.dumps(found))
            respond(command, data=found)
        else:
            send({"id": command.get("id"), "type": "response", "command": "set_model", "success": False,
                  "error": "Model not found: %s/%s" % (command["provider"], command["modelId"])})
    elif kind == "get_commands":
        commands = [{"name": "guaca-gate", "description": "Guaca's push gate", "source": "extension"}] if gated else []
        respond(command, data={"commands": commands})
    elif kind == "prompt":
        respond(command)
        turn(command)
    elif kind in ("steer", "abort"):
        respond(command)
    elif kind == "extension_ui_response":
        pending.append(command)
