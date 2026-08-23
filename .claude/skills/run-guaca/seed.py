#!/usr/bin/env python3
"""Fill a Guaca workspace with a transcript that draws every row a channel has.

Written for looking at the UI, so it costs nothing and needs no key: the rows
go straight into SQLite in the shape the runtime writes them, and the app draws
them through the real read path when it opens.

It refuses to write to a database that already holds messages. There is one
profile per bundle identifier and every workspace on the machine shares it, so
the failure mode this is guarding against is seeding test rows into the
operator's own workspace. Rename that profile aside first and let the app make
a fresh one; `--force` is there for when you have done exactly that and want it
anyway.
"""

from __future__ import annotations

import argparse
import json
import os
import sqlite3
import sys
import time
import uuid

PROFILE = "~/Library/Application Support/com.madebywelch.guac"
GROUP = "00000000-0000-4000-8000-000000000001"

CREW = [
    ("Manager", "avocado", "#4e6b16", ["Plans work", "Delegates by fit"]),
    ("Researcher", "lime", "#1c7a51", ["Finds sources", "Checks claims"]),
    ("Critic", "pit", "#8a5a2f", ["Reviews drafts", "Finds holes"]),
    ("Scribe", "leaf", "#5b665e", ["Writes things up"]),
]


def ok(summary: str) -> dict:
    return {"status": "ok", "summary": summary}


def tool(name: str, arguments: dict, outcome: dict) -> dict:
    return {"type": "toolCall", "name": name, "arguments": arguments, "outcome": outcome}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--db", default=os.path.join(os.path.expanduser(PROFILE), "guac.db"))
    ap.add_argument("--force", action="store_true", help="write even if the database already holds messages")
    args = ap.parse_args()

    if not os.path.exists(args.db):
        print(f"no database at {args.db}\nstart the app once to create the schema, then run this", file=sys.stderr)
        return 1

    conn = sqlite3.connect(args.db)
    held = conn.execute("SELECT count(*) FROM messages").fetchone()[0]
    if held and not args.force:
        print(
            f"{args.db} already holds {held} messages.\n"
            "This looks like a workspace somebody is using. Rename the profile aside and let the\n"
            "app make a fresh one, or pass --force if you are certain.",
            file=sys.stderr,
        )
        return 1

    now = int(time.time() * 1000)
    ids: dict[str, str] = {}
    for order, (name, avatar, color, skills) in enumerate(CREW):
        agent_id = str(uuid.uuid4())
        ids[name] = agent_id
        conn.execute(
            "INSERT INTO agents (id,name,avatar,color,model,system_prompt,skills,lifecycle,version,"
            "created_at,updated_at,group_id,pinned,rail_order) VALUES (?,?,?,?,'',?,?,'active',1,?,?,?,0,?)",
            (agent_id, name, avatar, color, f"You are {name}.", json.dumps(skills), now, now, GROUP, order),
        )

    manager, researcher = ids["Manager"], ids["Researcher"]
    run = str(uuid.uuid4())
    clock = now - 95 * 60 * 1000
    rows: list[tuple] = []

    def add(frm, frm_id, to, to_id, parts, trust, intent="work", gap=4000):
        nonlocal clock
        clock += gap
        rows.append((str(uuid.uuid4()), run, manager, frm, frm_id, to, to_id,
                     json.dumps(parts), trust, 0, 0, None, clock, intent))

    add("human", None, "agent", manager,
        [{"type": "text", "text": "Check the top three stories on cnn, then summarize them."}], "operator")

    # One turn writes one envelope, however many tools it reached for. This is
    # what the trail row folds.
    add("agent", manager, "system", None, [
        tool("directory", {}, ok("3 agent(s): Researcher, Critic, Scribe")),
        tool("browse", {"action": "open", "url": "https://www.cnn.com"}, ok("open in the browser")),
        tool("browse", {"action": "read"}, ok("read in the browser")),
        tool("browse", {"action": "click", "id": 14}, ok("click in the browser")),
        tool("browse", {"action": "read"}, ok("read in the browser")),
        tool("run_command", {"command": "python3 -c 'import sys; print(len(sys.stdin.read()))'"},
             ok("exit 0, 4 bytes out")),
        # Never folds: the operator's audit trail for their own tokens.
        tool("run_command", {"command": 'curl -s -H "Authorization: Bearer $STRIPE_KEY" https://api.stripe.com/v1/charges'},
             ok("used Stripe ($STRIPE_KEY) · exit 0, 812 bytes out")),
        # Never folds either.
        tool("run_command", {"command": "cat /tmp/headlines.json"},
             {"status": "failed", "error": "your computer is not available (sandbox timed out)"}),
        tool("use_screen", {"action": "look"}, ok("looked at the screen (1280x800)")),
        tool("update_notes", {"content": "# Manager\n\n- Reads CNN first thing.\n- Three bullets, no preamble."},
             ok("Memory saved (62 characters).")),
    ], "system", intent="courtesy")

    # Peer traffic, which the channel collapses into a burst.
    add("agent", researcher, "agent", manager,
        [{"type": "text", "text": "The market number matches Reuters."}], "peer")
    add("agent", researcher, "agent", manager,
        [{"type": "text", "text": "Nothing new on the storm since the 14:00 bulletin."}], "peer", gap=900)

    add("agent", manager, "human", None, [{"type": "text", "text": (
        "Three stories, top of the hour:\n\n"
        "1. **Markets** — the index closed down 1.2%.\n"
        "2. **Weather** — a storm makes landfall overnight.\n"
        "3. **Politics** — the vote moved to Thursday.\n\n"
        "Want me to keep watching any of these?")}], "peer")

    # A guard stop, and a send that named nobody: neither folds.
    add("agent", manager, "system", None, [
        tool("send_message", {"text": "keeping an eye on this"},
             {"status": "refused", "reason": "Refused: name a recipient. Put one or more agent names in `to`."}),
        {"type": "notice", "kind": "guardStop", "text": "hop limit (8) reached"},
    ], "system", intent="courtesy")

    conn.executemany(
        "INSERT INTO messages (id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,"
        "expects_reply,cause,created_at,intent) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)", rows)
    conn.commit()
    print(f"seeded {len(CREW)} agents and {len(rows)} messages into {args.db}")
    print("open Manager's channel: bubbles, a peer burst, a folded trail, a failure, a credential and a guard stop")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
