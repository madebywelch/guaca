#!/usr/bin/env python3
"""A stdio MCP server for tests: one JSON-RPC message per line."""
import json
import os
import sys

MODE = sys.argv[1] if len(sys.argv) > 1 else ""
if MODE == "die":
    print("fixture: cannot find its configuration", file=sys.stderr, flush=True)
    sys.exit(3)

# Plenty of real servers print a banner before speaking MCP.
print("fixture server starting", flush=True)
print("fixture: stderr is for people", file=sys.stderr, flush=True)

def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()

TOOLS = [
    {"name": "echo", "description": "Says it back.",
     "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}},
    {"name": "whoami", "description": "Reports its environment."},
    {"name": "fail", "description": "Always refuses."},
]

for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if "id" not in message:
        continue
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": message["id"], "result": {
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "fixture", "version": "1"}}})
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"tools": TOOLS}})
    elif method == "tools/call":
        # A server may ask its client something mid-call; Guaca offers nothing
        # and must say so rather than leave it waiting.
        send({"jsonrpc": "2.0", "id": "srv-1", "method": "roots/list", "params": {}})
        answer = json.loads(sys.stdin.readline())
        refused = "error" in answer
        name = message["params"]["name"]
        args = message["params"].get("arguments") or {}
        if name == "echo":
            content = [{"type": "text", "text": f"said: {args.get('text', '')}"}]
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"content": content}})
        elif name == "whoami":
            leaked = "CARGO_PKG_NAME" in os.environ
            text = f"token={os.environ.get('FIXTURE_TOKEN', 'none')} leaked={leaked} refused={refused}"
            send({"jsonrpc": "2.0", "id": message["id"],
                  "result": {"content": [{"type": "text", "text": text}]}})
        else:
            send({"jsonrpc": "2.0", "id": message["id"], "result": {
                "isError": True, "content": [{"type": "text", "text": "fail always fails"}]}})
    else:
        send({"jsonrpc": "2.0", "id": message["id"],
              "error": {"code": -32601, "message": f"no {method}"}})
