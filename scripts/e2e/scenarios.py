
from __future__ import annotations

import json
import re
import sqlite3
import threading
import urllib.request
from dataclasses import dataclass
from typing import Callable

from .world import STAND_IN_KEY, STAND_IN_MODEL, Call, Reply, World, call, held, refuse, say, text_of

AGENT = "Executive Assistant"
FIRST_TASK = "Something else Start with a task in your own words."
API_KEY = "API key Use OpenRouter or OpenAI, with usage billed to that account."
WRONG_KEY = "verify-wrong-key"
HELLO = "Hello. I am ready when you are."


def first_run(w: World, brief: str = "Say hello.", reply_to_brief: str = HELLO) -> None:
    ui = w.ui
    w.wait("the first visit offers a first task", lambda: ui.find("button", FIRST_TASK))
    ui.click("button", FIRST_TASK)
    ui.fill("textbox", "Your first brief", brief)
    ui.click("button", "Continue")
    w.wait("the Connect your AI step", lambda: ui.find("button", "Test and continue"))
    ui.click("button", API_KEY)
    ui.choose("combobox", "API provider", "Custom endpoint")
    w.wait("the endpoint fields", lambda: ui.find("textbox", "Inference endpoint"))
    ui.fill("textbox", "Inference endpoint", w.model.base)
    ui.fill("textbox", "Default model", STAND_IN_MODEL)
    ui.fill("textbox", "API key", STAND_IN_KEY, exact=False)
    ui.click("button", "Test and continue")
    w.wait("the endpoint test passes", lambda: ui.find("button", "Start task"))
    ui.click("button", "Start task")
    w.wait("the first agent answers its brief", lambda: reply_to_brief in conversation(w))


def conversation(w: World) -> str:
    logs = w.ui.find("log", "Conversation with", exact=False)
    if not logs:
        return ""
    return w.ui.evaluate(
        "Array.from(document.querySelectorAll('[role=log]')).map(e => e.innerText).join('\\n')"
    ) or ""


def write(w: World, to: str, text: str) -> None:
    ui = w.ui
    if ui.find("combobox", "Recipient") and ui.value("combobox", "Recipient") != to:
        ui.choose("combobox", "Recipient", to)
    ui.fill("combobox", f"Write to {to}\u2026", text)
    ui.click("button", "Send")


def new_agent(w: World, name: str, instructions: str) -> None:
    ui = w.ui
    ui.click("button", "Make something new")
    ui.click("menuitem", "New agent", exact=False)
    w.wait("the New agent dialog", lambda: ui.find("dialog", "New agent"))
    ui.fill("textbox", "Name", name)
    ui.fill("textbox", "Instructions", instructions)
    ui.click("button", "Create agent")
    w.wait(f"{name} exists", lambda: w.host.scalar(
        "SELECT count(*) FROM agents WHERE name = ? AND lifecycle != 'terminated'", name))
    w.wait("the dialog closes", lambda: not ui.find("dialog", "New agent"))


def open_agent(w: World, name: str) -> None:
    if w.ui.find("log", f"Conversation with {name}"):
        return
    face = w.wait(f"{name} has a face on today's timeline", lambda: [
        n for n in w.ui.names_within("list", "Today's timeline", "button") if n.startswith(f"{name}, ")])
    w.ui.click("button", face[0])
    w.wait(f"{name}'s conversation opens", lambda: w.ui.find("log", f"Conversation with {name}"))


def agent_id(w: World, name: str) -> str:
    return w.host.scalar("SELECT id FROM agents WHERE name = ?", name)


def outcomes(w: World) -> list[dict]:
    return w.host.rows("SELECT o.status, o.trigger, a.name AS agent FROM outcomes o "
                       "JOIN agents a ON a.id = o.agent_id ORDER BY o.finished_at, o.rowid")


def settled(w: World, count: int, what: str = "every conversation settles") -> list[dict]:
    w.wait(what, lambda: len(outcomes(w)) >= count)
    return outcomes(w)


def said(w: World, agent: str) -> list[str]:
    texts = []
    for row in w.host.rows("SELECT m.parts FROM messages m JOIN agents a ON a.id = m.from_agent "
                           "WHERE a.name = ? AND m.to_kind = 'human' ORDER BY m.created_at, m.rowid", agent):
        texts += [part["text"] for part in json.loads(row["parts"]) if part.get("type") == "text"]
    return texts


def between(w: World) -> list[dict]:
    rows = w.host.rows("SELECT f.name AS sender, t.name AS recipient, m.parts, m.run_id, m.intent "
                       "FROM messages m JOIN agents f ON f.id = m.from_agent JOIN agents t ON t.id = m.to_agent "
                       "WHERE m.from_kind = 'agent' AND m.to_kind = 'agent' ORDER BY m.created_at, m.rowid")
    return [{"from": r["sender"], "to": r["recipient"], "run": r["run_id"], "intent": r["intent"],
             "text": " ".join(p["text"] for p in json.loads(r["parts"]) if p.get("type") == "text")}
            for r in rows]


def no_unscripted(w: World) -> None:
    w.check("every model call was one the scenario scripted", not w.model.unscripted,
            observed=len(w.model.unscripted), detail=w.model.unscripted or None)


def scenario_reply(w: World) -> None:

    def script(c: Call) -> Reply | None:
        if c.key != STAND_IN_KEY:
            return refuse(401, "Incorrect API key provided.")
        if c.last_user == "ping":
            return say("pong")
        if c.speaker != AGENT:
            return None
        if c.last_user.endswith("Say hello."):
            return say(HELLO)
        if c.last_user.endswith("What is two and two?"):
            return say("Four.")
        return None

    w.model.script = script
    w.start()
    ui = w.ui

    w.wait("the first visit offers a first task", lambda: ui.find("button", FIRST_TASK))
    ui.click("button", FIRST_TASK)
    ui.fill("textbox", "Your first brief", "Say hello.")
    ui.click("button", "Continue")
    w.wait("the Connect your AI step", lambda: ui.find("button", "Test and continue"))
    ui.click("button", API_KEY)
    ui.choose("combobox", "API provider", "Custom endpoint")
    w.wait("the endpoint fields", lambda: ui.find("textbox", "Inference endpoint"))
    ui.fill("textbox", "Inference endpoint", w.model.base)
    ui.fill("textbox", "Default model", STAND_IN_MODEL)
    ui.fill("textbox", "API key", WRONG_KEY, exact=False)
    ui.click("button", "Test and continue")
    alert = w.wait("the endpoint test fails and says so", lambda: [
        t for t in (ui.evaluate("Array.from(document.querySelectorAll('[role=alert]')).map(e=>e.innerText)") or [])
        if t.strip()])
    w.check("a rejected key is reported in the form", "401" in alert[0] or "key" in alert[0].lower(),
            observed=alert, shot=True)
    kept = w.host.call("get_settings")
    w.check("a rejected key saves nothing and starts no agent",
            kept["baseUrl"] != w.model.base and not kept["apiKeySet"]
            and not w.host.scalar("SELECT count(*) FROM agents"),
            observed={"endpointSaved": kept["baseUrl"] == w.model.base, "keySaved": kept["apiKeySet"],
                      "agents": w.host.scalar("SELECT count(*) FROM agents")})

    ui.fill("textbox", "API key", STAND_IN_KEY, exact=False)
    ui.click("button", "Test and continue")
    w.wait("the endpoint test passes", lambda: ui.find("button", "Start task"))
    kept = w.host.call("get_settings")
    w.check("the right key passes the test and is kept on the Host",
            kept["baseUrl"] == w.model.base and kept["apiKeySet"],
            observed={"endpointSaved": kept["baseUrl"] == w.model.base, "keySaved": kept["apiKeySet"]})
    ui.click("button", "Start task")
    w.wait("the agent's reply is shown", lambda: HELLO in conversation(w))
    first = settled(w, 1, "the first conversation settles")
    w.check("the brief reached the agent and its reply is in the conversation",
            HELLO in conversation(w) and "Say hello." in conversation(w),
            observed={"outcomes": first, "said": said(w, AGENT)}, shot=True)

    write(w, AGENT, "What is two and two?")
    w.wait("the second reply is shown", lambda: "Four." in conversation(w))
    both = settled(w, 2, "the second conversation settles")
    w.check("a message written in the field is answered in the same conversation",
            said(w, AGENT) == [HELLO, "Four."],
            observed={"outcomes": both, "said": said(w, AGENT)}, shot=True)
    w.check("the page was told about each reply as it happened",
            sum(1 for e in w.events if e.get("type") == "runSettled") >= 2,
            observed=sorted({e.get("type") for e in w.events if e.get("type") in (
                "messageAppended", "streamStarted", "streamEnded", "runSettled", "outcomeRecorded")}))
    no_unscripted(w)


def scenario_resume(w: World) -> None:
    first = "First thing to do."
    second = "Second thing, while you are at it."
    answer = "Both noted."
    seen = []

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        if c.speaker != AGENT:
            return None
        if c.said(first):
            counts = [sum(text_of(m).count(text) for m in c.messages if m.get("role") != "system")
                      for text in (first, second)]
            seen.append(counts)
            return say(answer) if counts == [1, 1] else None
        if c.last_user.endswith("Say hello."):
            return say(HELLO)
        return None

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)
    ui = w.ui
    ui.click("button", f"Actions for {AGENT}")
    ui.click("button", "Pause")
    w.wait("the agent is paused", lambda: w.host.scalar(
        "SELECT lifecycle FROM agents WHERE name = ?", AGENT) == "paused")
    calls = len(w.model.calls)
    write(w, AGENT, first)
    write(w, AGENT, second)
    w.wait("both messages are queued", lambda: w.host.scalar("SELECT count(*) FROM pending_runs") == 2)
    w.wait("both queued messages are visible", lambda: first in conversation(w) and second in conversation(w))
    w.check("a paused agent keeps both messages queued without calling the model",
            len(w.model.calls) == calls and first in conversation(w) and second in conversation(w),
            observed={"queued": w.host.scalar("SELECT count(*) FROM pending_runs"),
                      "calledModel": len(w.model.calls) != calls}, shot=True)
    ui.click("button", f"Actions for {AGENT}")
    ui.click("button", "Resume")
    w.wait("the resumed agent answers", lambda: answer in conversation(w))
    w.wait("both queued conversations settle", lambda: w.host.scalar(
        "SELECT count(*) FROM pending_runs") == 0
        and sum(e.get("type") == "runSettled" for e in w.events) == 3)
    w.check("Resume answers both queued messages once and releases both conversations",
            seen == [[1, 1]] and said(w, AGENT) == [HELLO, answer],
            observed={"promptCopies": seen, "said": said(w, AGENT),
                      "queued": w.host.scalar("SELECT count(*) FROM pending_runs")}, shot=True)
    no_unscripted(w)


CRITIC = "Critic"
ASK_CRITIC = "Ask Critic whether the plan holds."
QUESTION = "Does the plan hold up? Answer in one line."
VERDICT = "The plan holds."
RELAYED = "Critic says the plan holds."
VOLLEY = "Keep a volley going with Critic."
SERVE = "Your serve."


def scenario_peers(w: World) -> None:

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        acted = bool(c.tool_results())
        if c.speaker == AGENT:
            if c.last_user.endswith("Say hello."):
                return say(HELLO)
            if c.last_user.endswith(ASK_CRITIC):
                return say("I asked Critic.") if acted else call(
                    "send_message", to=[CRITIC], text=QUESTION, intent="work")
            if VERDICT in c.last_user:
                return say(RELAYED)
        if c.speaker == CRITIC and QUESTION in c.last_user:
            return say(VERDICT)
        if c.speaker in (AGENT, CRITIC) and (c.last_user.endswith(VOLLEY) or SERVE in c.last_user):
            other = CRITIC if c.speaker == AGENT else AGENT
            served = re.search(rf"{SERVE} (\d+)", c.last_user)
            volley = f"{SERVE} {int(served.group(1)) + 1 if served else 1}"
            return say("Sent.") if acted else call("send_message", to=[other], text=volley, intent="work")
        if c.speaker in (AGENT, CRITIC) and c.last_user.endswith(("Sent.", "Noted.")):
            return say("Noted.")
        return None

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)
    new_agent(w, CRITIC, "You review plans and say whether they hold.")
    w.check("a second agent joins the group", agent_id(w, CRITIC), observed=sorted(
        r["name"] for r in w.host.rows("SELECT name FROM agents")), shot=True)

    write(w, AGENT, ASK_CRITIC)
    w.wait("the answer is relayed to the person", lambda: RELAYED in conversation(w))
    ended = settled(w, 2, "the conversation between the two agents settles")
    talk = between(w)
    w.check("the question went to Critic and its answer came back in the same run",
            [(m["from"], m["to"], m["text"]) for m in talk] == [(AGENT, CRITIC, QUESTION), (CRITIC, AGENT, VERDICT)]
            and len({m["run"] for m in talk}) == 1,
            observed=[{k: m[k] for k in ("from", "to", "text", "intent")} for m in talk])
    w.check("the conversation ended with the relayed answer and nothing after it",
            said(w, AGENT)[-1] == RELAYED and ended[-1]["status"] == "done",
            observed={"outcomes": ended, "said": said(w, AGENT)}, shot=True)
    calls = len(w.model.calls)

    write(w, AGENT, VOLLEY)
    stopped = settled(w, 3, "a volley that never stops on its own still ends")
    volley = [m for m in between(w) if m["text"].startswith(SERVE)]
    refusals = [r for e in w.model.calls[calls:] for r in e["toolResults"] if "hops from the operator" in r]
    w.check("the guard ended the volley at the hop limit, and the run settled",
            refusals and 0 < len(volley) <= 12 and len(w.model.calls) - calls <= 40,
            observed={"serves": len(volley), "outcome": stopped[-1], "refusal": refusals[:1]},
            detail={"modelCalls": len(w.model.calls) - calls}, shot=True)
    no_unscripted(w)


CALL_ME = "Please call me Robin from now on."
CALL_ME_SAM = "Please call me Sam from now on."
MOVE_ENDPOINT = "Move yourself to another endpoint."


def ask_name(c: Call, name: str) -> Reply:
    results = c.tool_results()
    if not results:
        return call("settings", action="update", changes={"operatorName": name})
    return say(f"Settings answered: {results[-1][:160]}")


def operator_name(w: World) -> str:
    return w.host.call("get_settings")["operatorName"]


def approvals(w: World) -> list[dict]:
    return w.host.rows("SELECT action, state FROM approvals ORDER BY created_at, rowid")


def scenario_permission(w: World) -> None:

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        if c.speaker != AGENT:
            return None
        if c.last_user.endswith("Say hello."):
            return say(HELLO)
        if c.last_user.endswith(CALL_ME):
            return ask_name(c, "Robin")
        if c.last_user.endswith(CALL_ME_SAM):
            return ask_name(c, "Sam")
        if c.last_user.endswith(MOVE_ENDPOINT):
            results = c.tool_results()
            if not results:
                return call("settings", action="update", changes={"baseUrl": "http://127.0.0.1:9/v1"})
            return say(f"Settings answered: {results[-1][:160]}")
        return None

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)
    ui = w.ui

    write(w, AGENT, CALL_ME)
    w.wait("the request is shown in the conversation", lambda: ui.find("button", "Allow"))
    w.check("the turn is parked on a request, and nothing has changed yet",
            approvals(w) == [{"action": "changeSettings", "state": "pending"}] and operator_name(w) == "",
            observed={"approvals": approvals(w), "operatorName": operator_name(w)}, shot=True)
    ui.click("button", "Allow")
    settled(w, 2, "the allowed turn finishes")
    w.check("an allowed change is made, and the agent is told",
            operator_name(w) == "Robin" and said(w, AGENT)[-1].startswith("Settings answered"),
            observed={"approvals": approvals(w), "operatorName": operator_name(w), "said": said(w, AGENT)[-1]},
            shot=True)

    write(w, AGENT, CALL_ME_SAM)
    w.wait("the second request is shown", lambda: ui.find("button", "Deny"))
    ui.click("button", "Deny")
    settled(w, 3, "the denied turn finishes")
    w.check("a denied change is not made, and the agent is told",
            operator_name(w) == "Robin" and [a["state"] for a in approvals(w)][-1] != "allow",
            observed={"approvals": approvals(w), "operatorName": operator_name(w), "said": said(w, AGENT)[-1]},
            shot=True)

    before = len(approvals(w))
    endpoint = w.host.call("get_settings")["baseUrl"]
    write(w, AGENT, MOVE_ENDPOINT)
    settled(w, 4, "the refused turn finishes")
    w.check("a setting only the person may change is refused without asking anyone",
            len(approvals(w)) == before and w.host.call("get_settings")["baseUrl"] == endpoint,
            observed={"asked": len(approvals(w)) - before, "said": said(w, AGENT)[-1]})
    no_unscripted(w)


SUMMARIZE = "Summarize the week for me."
SUMMARY = "This week: three plans, one review."
DRAFT = "Draft the status report."
DRAFTED = "The status report is drafted."
BOOK = "Book the meeting room."
BOOKED = "The meeting room is booked."
NOT_STARTED = (f"{AGENT} could not start on this because the workspace could not record it. Nothing was done. "
               "It will be delivered again when the host next starts.")


def refuse_start_marks(w: World, refuse: bool) -> None:
    conn = sqlite3.connect(w.host.db, timeout=5)
    try:
        conn.execute("CREATE TRIGGER refuse_start BEFORE UPDATE OF started_at ON pending_runs "
                     "BEGIN SELECT RAISE(ABORT, 'disk failure'); END" if refuse else "DROP TRIGGER refuse_start")
        conn.commit()
    finally:
        conn.close()


def asked(w: World, ask: str) -> int:
    return sum(1 for e in w.model.calls if e["asked"].endswith(ask) and not e["toolResults"])


def scenario_restart(w: World) -> None:
    stopping = threading.Event()
    crashing = threading.Event()

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        if c.speaker != AGENT:
            return None
        if c.last_user.endswith("Say hello."):
            return say(HELLO)
        if c.last_user.endswith(SUMMARIZE):
            return held(stopping, say(SUMMARY))
        if c.last_user.endswith(DRAFT):
            return held(crashing, say(DRAFTED))
        if c.last_user.endswith(BOOK):
            return say(BOOKED)
        return None

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)

    write(w, AGENT, SUMMARIZE)
    w.wait("the turn's model call is in the air", lambda: asked(w, SUMMARIZE) == 1)
    status = w.host.stop()
    stopping.set()
    log = w.host.log_text()
    w.check("a stopped Host exits cleanly after putting the turn down",
            status == 0 and "putting work down for the next one" in log and "conversations=1" in log,
            observed={"exit": status, "putDown": "conversations=1" in log})
    w.host.start()
    w.open_browser()
    w.wait("the next Host finishes the turn", lambda: SUMMARY in said(w, AGENT))
    open_agent(w, AGENT)
    w.wait("the finished turn is shown", lambda: SUMMARY in conversation(w))
    picked = settled(w, 2, "the picked-up conversation settles")
    w.check("the next Host delivered the message again and the reply arrived once",
            asked(w, SUMMARIZE) == 2 and said(w, AGENT).count(SUMMARY) == 1 and picked[-1]["status"] == "done",
            observed={"asked": asked(w, SUMMARIZE), "said": said(w, AGENT), "outcomes": picked}, shot=True)

    write(w, AGENT, DRAFT)
    w.wait("the second turn's model call is in the air", lambda: asked(w, DRAFT) == 1)
    w.host.kill()
    crashing.set()
    w.host.start()
    w.open_browser()
    open_agent(w, AGENT)
    w.wait("the crashed turn is offered again", lambda: w.ui.find("button", "Try again"))
    w.check("a crashed turn is reported interrupted and is not repeated by itself",
            asked(w, DRAFT) == 1 and DRAFTED not in said(w, AGENT),
            observed={"asked": asked(w, DRAFT), "outcomes": outcomes(w)}, shot=True)
    w.ui.click("button", "Try again")
    w.wait("Try again finishes the turn", lambda: DRAFTED in said(w, AGENT))
    w.check("Try again runs the turn once more and the reply arrives",
            asked(w, DRAFT) == 2 and said(w, AGENT).count(DRAFTED) == 1,
            observed={"asked": asked(w, DRAFT), "said": said(w, AGENT)}, shot=True)

    refuse_start_marks(w, True)
    write(w, AGENT, BOOK)
    w.wait("the conversation says the message could not start", lambda: NOT_STARTED in conversation(w))
    w.check("a message whose start cannot be recorded says so at once, runs nothing and offers no Try again",
            asked(w, BOOK) == 0 and not w.ui.find("button", "Try again"),
            observed={"asked": asked(w, BOOK), "tryAgain": bool(w.ui.find("button", "Try again"))}, shot=True)
    refuse_start_marks(w, False)
    w.host.stop()
    w.host.start()
    w.open_browser()
    open_agent(w, AGENT)
    w.wait("the next Host delivers the message that could not start", lambda: BOOKED in said(w, AGENT))
    w.check("the next Host delivers the message that could not start, and the reply arrives once",
            asked(w, BOOK) == 1 and said(w, AGENT).count(BOOKED) == 1,
            observed={"asked": asked(w, BOOK), "said": said(w, AGENT)}, shot=True)
    no_unscripted(w)


HOURS = "Find the vendor's opening hours."
NUMBER = "And find their phone number."
LOOKING = "Looking up the vendor."
DIALING = "Looking up the number."
FOUND = "Their number is 555-0100."
CUT_SHORT = (f"You stopped another conversation while {AGENT} was working on this one too, so this was not "
             "finished. Review any actions already taken before retrying.")


def scenario_stop(w: World) -> None:
    looking = threading.Event()
    dialing = threading.Event()

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        if c.speaker != AGENT:
            return None
        if c.last_user.endswith("Say hello."):
            return say(HELLO)
        after_tool = len(c.messages) > 1 and c.messages[-2].get("role") == "tool"
        if c.last_user.endswith(HOURS) and not c.tool_results():
            return held(looking, call("note_progress", note=LOOKING))
        if c.last_user.endswith(NUMBER) and after_tool:
            return call("note_progress", note=DIALING)
        if c.last_user.endswith(NUMBER) and c.tool_results():
            return held(dialing, say("Hours and number found."))
        if c.last_user.endswith(NUMBER):
            return say(FOUND)
        return None

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)

    write(w, AGENT, HOURS)
    w.wait("the first conversation's model call is in the air", lambda: asked(w, HOURS) == 1)
    write(w, AGENT, NUMBER)
    w.wait("the second conversation's message is written", lambda: w.host.scalar(
        "SELECT count(*) FROM messages WHERE parts LIKE ?", f"%{NUMBER}%"))
    looking.set()
    w.wait("a call is made after the second message was taken in", lambda: any(
        e["asked"].endswith(NUMBER) and e["toolResults"] for e in w.model.calls))
    w.check("the turn took the second conversation in and acted on it",
            sum(1 for e in w.model.calls if e["asked"].endswith(NUMBER)) == 2,
            observed=[e["asked"].endswith(NUMBER) for e in w.model.calls[-2:]])

    w.wait("the turn offers Stop", lambda: w.ui.find("button", "Stop"))
    w.ui.click("button", "Stop")
    w.wait("the second conversation says it was cut short", lambda: CUT_SHORT in conversation(w))
    dialing.set()
    ended = settled(w, 3, "both conversations settle")
    calls = len(w.model.calls)
    w.check("stopping one conversation tells the other at once that it was not finished, with Try again",
            sorted(o["status"] for o in ended[1:]) == ["interrupted", "stopped"]
            and w.ui.find("button", "Try again") and FOUND not in said(w, AGENT),
            observed={"outcomes": ended, "tryAgain": bool(w.ui.find("button", "Try again"))}, shot=True)

    w.ui.click("button", "Try again")
    w.wait("Try again answers the second conversation", lambda: FOUND in said(w, AGENT))
    w.check("Try again sends the cut-short message once more and it is answered",
            len(w.model.calls) == calls + 1 and said(w, AGENT).count(FOUND) == 1,
            observed={"said": said(w, AGENT)}, shot=True)
    no_unscripted(w)


FAILED_TOGETHER = (f"{AGENT} was working on this and another conversation together when a model call failed, so "
                   "this was not finished. Review any actions already taken before retrying.")


def scenario_failed(w: World) -> None:
    looking = threading.Event()

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        if c.speaker != AGENT:
            return None
        if c.last_user.endswith("Say hello."):
            return say(HELLO)
        after_tool = len(c.messages) > 1 and c.messages[-2].get("role") == "tool"
        if c.last_user.endswith(HOURS) and not c.tool_results():
            return held(looking, call("note_progress", note=LOOKING))
        if c.last_user.endswith(NUMBER) and after_tool:
            return call("note_progress", note=DIALING)
        if c.last_user.endswith(NUMBER) and c.tool_results():
            return refuse(500, "The model is overloaded.")
        return None

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)

    write(w, AGENT, HOURS)
    w.wait("the first conversation's model call is in the air", lambda: asked(w, HOURS) == 1)
    write(w, AGENT, NUMBER)
    w.wait("the second conversation's message is written", lambda: w.host.scalar(
        "SELECT count(*) FROM messages WHERE parts LIKE ?", f"%{NUMBER}%"))
    looking.set()
    w.wait("the second conversation says the turn could not finish it", lambda: FAILED_TOGETHER in conversation(w))
    ended = settled(w, 3, "both conversations settle")
    w.check("a failed model call tells the other conversation the turn acted on at once, with Try again on each",
            sorted(o["status"] for o in ended[1:]) == ["failed", "interrupted"]
            and len(w.ui.find("button", "Try again")) == 2,
            observed={"outcomes": ended, "tryAgain": len(w.ui.find("button", "Try again"))}, shot=True)
    no_unscripted(w)


INSTRUCTION = "Check the failed invoice."


def routine_runs(w: World) -> list[dict]:
    return w.host.rows("SELECT kind, error FROM routine_runs ORDER BY id")


def scenario_routine(w: World) -> None:

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        if c.speaker != AGENT:
            return None
        if c.last_user.endswith("Say hello."):
            return say(HELLO)
        if INSTRUCTION in c.last_user:
            invoice = re.search(r"in_\w+", c.last_user)
            return say(f"Invoice {invoice.group(0) if invoice else 'unknown'} checked.")
        return None

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)
    ui = w.ui

    ui.click("button", "Routines")
    w.wait("the Routines tab", lambda: ui.find("button", "Add a routine"))
    ui.click("button", "Add a routine")
    w.wait("the new routine form", lambda: ui.find("textbox", "Instruction"))
    ui.fill("textbox", "Name", "Invoice watch")
    ui.fill("textbox", "Instruction", INSTRUCTION)
    ui.choose("combobox", "Trigger", "When an event is posted")
    ui.fill("textbox", "Service", "billing")
    ui.fill("textbox", "Event", "invoice.failed")
    ui.click("button", "Create routine")
    hidden = w.wait("the routine shows where to post", lambda: re.search(
        r"curl -X POST (\S+) .*Authorization: Bearer ([^'\s]+)", ui.text(), re.S))
    address = hidden.group(1)
    ui.send("Browser.grantPermissions", {"origin": ui.evaluate("location.origin"),
                                         "permissions": ["clipboardReadWrite", "clipboardSanitizedWrite"]},
            session=False)
    ui.send("Emulation.setFocusEmulationEnabled", {"enabled": True})
    ui.click("button", "Copy")
    w.wait("the line is copied", lambda: ui.find("button", "Copied"))
    masked_after_copy = "Authorization: Bearer " + "•" * 8 in ui.text()
    copied = ui.send("Runtime.evaluate", {"expression": "navigator.clipboard.readText()",
                                          "awaitPromise": True, "returnByValue": True})["result"].get("value") or ""
    w.wait("Copied goes back to Copy", lambda: ui.find("button", "Copy"))
    w.check("Copied goes back to Copy on its own",
            ui.find("button", "Copy") and not ui.find("button", "Copied"),
            observed={"labels": [name for name in ui.names("button") if name.startswith("Cop")]})
    ui.click("button", "Show secret")
    shown = w.wait("the secret is shown once asked for", lambda: re.search(
        r"Authorization: Bearer ([\w-]+)", ui.text()))
    secret = shown.group(1)
    w.secrets.append(secret)
    carried = sum(secret in json.dumps(event) for event in w.events)
    w.check("the secret is shown only when asked for, and no event carried it",
            hidden.group(2) == "•" * 8 and carried == 0,
            observed={"before": hidden.group(2), "events_carrying_it": carried})
    w.check("Copy puts the line with the real secret on the clipboard and leaves it masked on the page",
            copied.startswith(f"curl -X POST {address} ") and f"Authorization: Bearer {secret}'" in copied
            and masked_after_copy,
            observed={"carries_secret": secret in copied, "masked_after_copy": masked_after_copy})
    ui.click("button", "Hide secret")
    rehidden = w.wait("the secret is masked again", lambda: ui.find("button", "Show secret") and ui.text())
    w.check("Hide secret masks the line again",
            secret not in rehidden and "Authorization: Bearer " + "•" * 8 in rehidden,
            observed={"shows_secret": secret in rehidden, "masked": "•" * 8 in rehidden})
    w.check("the routine is kept, switched on, waiting on the event",
            w.host.rows("SELECT name, fires, active FROM routines")
            == [{"name": "Invoice watch", "fires": "event:billing/invoice.failed", "active": 1}]
            and address.endswith("/events/billing/invoice.failed"),
            observed={"routines": w.host.rows("SELECT name, fires, active FROM routines"),
                      "path": address.split("127.0.0.1", 1)[-1].split("/", 1)[-1]})
    ui.click("button", "Conversation")

    unsigned = w.host.post("/events/billing/invoice.failed", {"invoice": "in_0"})
    forged = w.host.post("/events/billing/invoice.failed", {"invoice": "in_0"},
                         {"authorization": "Bearer not-the-secret"})
    nobody = w.host.post("/events/billing/invoice.paid", {"invoice": "in_0"},
                         {"authorization": f"Bearer {secret}"})
    w.check("a post without the secret, or to an event nobody waits on, fires nothing",
            unsigned[0] == 401 and forged[0] == 401 and nobody[0] == 404 and not routine_runs(w),
            observed={"unsigned": unsigned[0], "forged": forged[0], "nobody": nobody[0],
                      "fired": len(routine_runs(w))})

    keyed = {"authorization": f"Bearer {secret}", "idempotency-key": "delivery-1"}
    first = w.host.post("/events/billing/invoice.failed", {"invoice": "in_42"}, keyed)
    w.wait("the agent acts on the event", lambda: "Invoice in_42 checked." in said(w, AGENT))
    fired = settled(w, 2, "the routine's run settles")
    w.check("one delivery fires the routine once, with its body, in a run of its own",
            first[0] == 200 and json.loads(first[1]).get("delivered") == 1
            and len(routine_runs(w)) == 1 and fired[-1]["trigger"] == "event",
            observed={"answer": json.loads(first[1]), "fired": routine_runs(w), "outcome": fired[-1]}, shot=True)

    again = w.host.post("/events/billing/invoice.failed", {"invoice": "in_42"}, keyed)
    w.check("a retry of the same delivery does not fire it again",
            again[0] == 200 and len(routine_runs(w)) == 1
            and sum(1 for e in w.model.calls if INSTRUCTION in e["asked"]) == 1,
            observed={"answer": json.loads(again[1]), "fired": len(routine_runs(w))})
    ui.click("button", "Routines")
    row = w.wait("the routine is listed", lambda: ui.find("button", "Invoice watch When Billing reports", exact=False))
    ui.click("button", (row[0]["name"] or {})["value"])
    w.wait("the routine opens", lambda: ui.find("button", "Test run"))
    w.wait("the run history lists the firing", lambda: "Run history" in ui.text() and "No runs yet." not in ui.text())
    w.check("the routine's history shows the one firing", True, observed=len(routine_runs(w)))
    no_unscripted(w)


def decisions(w: World) -> list[dict]:
    return w.host.rows("SELECT status, origin, answer FROM decisions ORDER BY created_at, rowid")


def latest_name(c: Call) -> str:
    for message in reversed(c.messages):
        text = text_of(message)
        if message.get("role") == "user" and (text.endswith(CALL_ME) or text.endswith(CALL_ME_SAM)):
            return "Sam" if text.endswith(CALL_ME_SAM) else "Robin"
    return ""


def grants(w: World) -> list[dict]:
    return [{"granted": row["granted_at"] is not None, "used": row["used_at"] is not None} for row in
            w.host.rows("SELECT granted_at, used_at FROM permission_grants ORDER BY rowid")]


def lapse_into_for_you(w: World, waiting: int) -> None:
    ui = w.ui
    w.wait("the request is shown in the conversation", lambda: ui.find("button", "Allow"))
    w.wait("the unanswered request lapses into For you",
           lambda: [d for d in decisions(w) if d["status"] == "pending"], timeout=120)
    label = f"For you, {waiting} waiting"
    w.wait("For you counts the waiting decision", lambda: ui.find("button", label))
    ui.click("button", label)
    w.wait("For you opens on the decision", lambda: ui.find("dialog", "For you") and ui.find("button", "Allow"))


def scenario_lapse(w: World) -> None:

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        if c.speaker != AGENT:
            return None
        if c.last_user.endswith("Say hello."):
            return say(HELLO)
        if c.last_user.endswith(CALL_ME):
            return ask_name(c, "Robin")
        if c.last_user.endswith(CALL_ME_SAM):
            return ask_name(c, "Sam")
        if '"answer":"Allow"' in c.last_user or '"answer":"Deny"' in c.last_user:
            return ask_name(c, latest_name(c))
        return None

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)
    ui = w.ui

    write(w, AGENT, CALL_ME)
    lapse_into_for_you(w, 1)
    w.check("the lapsed request waits in For you and nothing has changed",
            approvals(w) == [{"action": "changeSettings", "state": "expired"}]
            and decisions(w) == [{"status": "pending", "origin": "approval", "answer": None}]
            and operator_name(w) == "" and outcomes(w)[-1]["status"] == "needs_you",
            observed={"approvals": approvals(w), "decisions": decisions(w), "operatorName": operator_name(w),
                      "outcome": outcomes(w)[-1]}, shot=True)
    ui.click("button", "Allow")
    w.wait("the agent asks again and the change is made", lambda: operator_name(w) == "Robin")
    settled(w, 3, "the answered decision's run settles")
    w.check("an Allow in For you lets the same change through once, without parking again",
            operator_name(w) == "Robin" and len(approvals(w)) == 1 and grants(w) == [{"granted": True, "used": True}],
            observed={"approvals": approvals(w), "decisions": decisions(w), "grants": grants(w),
                      "said": said(w, AGENT)[-1]})

    ui.key("Escape")
    w.wait("For you closes", lambda: not ui.find("dialog", "For you"))
    write(w, AGENT, CALL_ME_SAM)
    lapse_into_for_you(w, 1)
    ui.click("button", "Deny")
    w.wait("the agent asks again and is parked again",
           lambda: [a for a in approvals(w) if a["state"] == "pending"])
    w.check("a Deny in For you grants nothing: the next ask waits for a person again",
            operator_name(w) == "Robin"
            and grants(w) == [{"granted": True, "used": True}, {"granted": False, "used": False}]
            and [a["state"] for a in approvals(w)] == ["expired", "expired", "pending"],
            observed={"approvals": approvals(w), "decisions": decisions(w), "grants": grants(w),
                      "operatorName": operator_name(w)}, shot=True)
    no_unscripted(w)


REVIEWED = "d" * 40
MOVED = "f" * 40
IMAGE = "ghcr.io/madebywelch/guaca/guacad@sha256:"
UPDATE = "Back up and update host"
MAIN_MOVED = "Main is now at fffffff, not ddddddd. Check for updates and review it again."
LOST = "Guaca lost the Host's reply, so it can't tell whether the update started."


def scenario_update(w: World) -> None:
    assert w.updater, "the update scenario runs beside a stand-in updater"
    updater = w.updater
    feed = {"commit": REVIEWED}
    other: dict = {"updating": False, "operation": None}
    running = {"image": f"{IMAGE}{'0' * 64}", "version": ""}

    def another(stage: str) -> dict:
        return {"attemptId": "another-client", "stage": stage, "backup": None,
                "previousImage": running["image"], "targetImage": f"{IMAGE}{'e' * 64}",
                "targetVersion": running["version"], "apiGeneration": 1, "port": None,
                "preserved": None, "error": None}

    def answer(ask: dict) -> dict | None:
        if ask.get("ask") == "status":
            return {"ok": {"channel": "main", "updating": other["updating"], "running": running,
                           "operation": other["operation"], "error": None}}
        if ask.get("commit") == REVIEWED:
            feed["commit"] = MOVED
            other.update(updating=True, operation=another("Backing up data"))
            return {"err": MAIN_MOVED}
        return None

    def release() -> dict:
        commit = feed["commit"]
        return {"automatic": True, "checkedAt": "2026-11-17T00:00:00Z", "error": None, "channel": "main",
                "latest": {"schema": 1, "channel": "main", "version": running["version"], "commit": commit,
                           "image": f"{IMAGE}{'e' * 64}", "apiGeneration": 1, "clientMinimum": 1,
                           "clientMaximum": 1, "notes": f"https://github.com/madebywelch/guaca/commit/{commit}"}}

    def asked() -> list[str]:
        return [str(ask.get("commit"))[:7] for ask in updater.updates()]

    updater.script = answer
    w.start()
    with urllib.request.urlopen(f"{w.host.origin}/health", timeout=10) as health:
        running["version"] = json.loads(health.read())["version"]
    ui = w.ui
    ui.answer("*/v1/updates*", release)
    ui.evaluate("location.reload()")
    w.wait("the page is back with the release check answered", lambda: ui.find("button", "App settings"))

    def reviewing() -> bool:
        if ui.find("button", UPDATE):
            return True
        if ui.find("button", "Review update"):
            ui.click("button", "Review update")
        return False

    w.wait("the update banner opens the panel", reviewing)
    offered = "Update this host to main at ddddddd." in ui.text()
    w.check("a box on main offers the build of main that was reviewed", offered and ui.find("button", UPDATE),
            observed={"offered": offered}, shot=True)

    ui.click("button", UPDATE)
    w.wait("the panel shows the update that is running", lambda: "Backing up data · step 3 of 5" in ui.text())
    shown = ui.text()
    w.check("a refused update says why, under the update another client started and its step",
            MAIN_MOVED in shown and "Starting update" not in shown and LOST not in shown
            and asked() == [REVIEWED[:7]],
            observed={"refusal": MAIN_MOVED in shown, "step": "Backing up data · step 3 of 5" in shown,
                      "startingUpdate": "Starting update" in shown, "lostReply": LOST in shown,
                      "asked": asked()}, shot=True)

    other.update(updating=False, operation=another("Host updated"))
    w.wait("the panel offers the build main moved to", lambda: ui.find("button", UPDATE)
           and "Update this host to main at fffffff." in ui.text())
    ended = ui.text()
    w.check("when the other update ends, the panel offers main's new build and claims nothing as its own",
            "Host updated to" not in ended and "main at fffffff" in ended,
            observed={"claimed": "Host updated to" in ended, "offers": "main at fffffff" in ended}, shot=True)

    ui.click("button", UPDATE)
    w.wait("the panel says the reply was lost", lambda: LOST in ui.text())
    before = len(updater.asked)
    ui.click("button", "Check again")
    w.wait("Check again asks the updater", lambda: len(updater.asked) > before)
    w.check("an updater that does not answer leaves the outcome unknown, and nothing is sent twice",
            LOST in ui.text() and not ui.find("button", UPDATE) and asked() == [REVIEWED[:7], MOVED[:7]],
            observed={"lostReply": LOST in ui.text(), "offersAgain": bool(ui.find("button", UPDATE)),
                      "asked": asked()}, shot=True)


WINDOW = (1280, 820)
TRAVEL = "Travel Planner"


def hello(c: Call) -> Reply | None:
    if c.last_user == "ping":
        return say("pong")
    if c.speaker == AGENT and c.last_user.endswith("Say hello."):
        return say(HELLO)
    return None


def scenario_newagent(w: World) -> None:
    w.model.script = hello
    w.start()
    w.ui.viewport(*WINDOW)
    first_run(w)
    settled(w, 1)
    ui = w.ui

    ui.click("button", "Make something new")
    ui.click("menuitem", "New agent", exact=False)
    w.wait("the New agent dialog", lambda: ui.find("dialog", "New agent"))
    w.check("Create agent is one button, on screen without scrolling",
            len(ui.find("button", "Create agent")) == 1 and ui.on_screen("button", "Create agent"),
            observed={"buttons": len(ui.find("button", "Create agent")),
                      "onScreen": bool(ui.find("button", "Create agent")) and ui.on_screen("button", "Create agent")},
            shot=True)

    ui.click("button", "Create agent")
    w.check("with no name, a press on Create agent creates nothing and keeps the dialog",
            ui.property("button", "Create agent", "disabled") and bool(ui.find("dialog", "New agent"))
            and w.host.scalar("SELECT count(*) FROM agents") == 1,
            observed={"disabled": ui.property("button", "Create agent", "disabled"),
                      "dialog": bool(ui.find("dialog", "New agent")),
                      "agents": w.host.scalar("SELECT count(*) FROM agents")})

    ui.fill("textbox", "Name", TRAVEL)
    ui.send("DOM.focus", {"backendNodeId": ui.one("button", "Create agent")["backendDOMNodeId"]})
    ui.key("Enter")
    w.wait(f"{TRAVEL} exists", lambda: w.host.scalar(
        "SELECT count(*) FROM agents WHERE name = ? AND lifecycle != 'terminated'", TRAVEL))
    w.wait("the dialog closes", lambda: not ui.find("dialog", "New agent"))
    w.check("a keyboard press on Create agent creates the agent and closes the dialog",
            not ui.find("dialog", "New agent")
            and w.host.scalar("SELECT count(*) FROM agents WHERE name = ?", TRAVEL) == 1,
            observed={"dialog": bool(ui.find("dialog", "New agent")),
                      "created": w.host.scalar("SELECT count(*) FROM agents WHERE name = ?", TRAVEL)},
            shot=True)
    no_unscripted(w)


@dataclass(frozen=True)
class Scenario:
    run: Callable[[World], None]
    proves: str
    claims: tuple[str, ...]
    minute_ms: int | None = None
    updater: bool = False


def unproved(claims: tuple[str, ...], steps: list[dict]) -> list[str]:
    held = {step["step"] for step in steps if step["pass"]}
    return [claim for claim in claims if claim not in held]


SCENARIOS: dict[str, Scenario] = {
    "resume": Scenario(scenario_resume, "A person queues two messages while an agent is paused and resumes it; "
                       "paused work calls no model, and Resume answers each message without duplicating its prompt.",
    (
        "a paused agent keeps both messages queued without calling the model",
        "Resume answers both queued messages once and releases both conversations",
        "every model call was one the scenario scripted",
    )),
    "reply": Scenario(scenario_reply, "A person sets up their first agent, writes to it and gets a reply; a key the "
                      "endpoint rejects is refused before anything is saved.",
    (
        "a rejected key is reported in the form",
        "a rejected key saves nothing and starts no agent",
        "the right key passes the test and is kept on the Host",
        "the brief reached the agent and its reply is in the conversation",
        "a message written in the field is answered in the same conversation",
        "the page was told about each reply as it happened",
        "every model call was one the scenario scripted",
    )),
    "peers": Scenario(scenario_peers, "An agent asks another, the answer comes back and the conversation ends; two "
                      "agents that would volley forever are stopped at the hop limit and the run still ends.",
    (
        "a second agent joins the group",
        "the question went to Critic and its answer came back in the same run",
        "the conversation ended with the relayed answer and nothing after it",
        "the guard ended the volley at the hop limit, and the run settled",
        "every model call was one the scenario scripted",
    )),
    "permission": Scenario(scenario_permission, "An agent asks to change a setting, the person allows it in the "
                           "conversation and it changes; a denied request changes nothing, and a setting no agent "
                           "may touch is refused without asking.",
    (
        "the turn is parked on a request, and nothing has changed yet",
        "an allowed change is made, and the agent is told",
        "a denied change is not made, and the agent is told",
        "a setting only the person may change is refused without asking anyone",
        "every model call was one the scenario scripted",
    )),
    "lapse": Scenario(scenario_lapse, "A request nobody answers lapses into For you, where an Allow lets the same "
                      "change through once without asking again; a Deny there grants nothing, and the next ask is "
                      "parked again.",
    (
        "the lapsed request waits in For you and nothing has changed",
        "an Allow in For you lets the same change through once, without parking again",
        "a Deny in For you grants nothing: the next ask waits for a person again",
        "every model call was one the scenario scripted",
    ), minute_ms=1000),
    "restart": Scenario(scenario_restart, "A Host stopped while a turn is in the air puts it down, and the next Host "
                        "finishes it; a Host that crashes mid-turn shows the turn as interrupted, repeats nothing, "
                        "and Try again finishes it; a message whose start cannot be recorded says so at once and the "
                        "next Host delivers it.",
    (
        "a stopped Host exits cleanly after putting the turn down",
        "the next Host delivered the message again and the reply arrived once",
        "a crashed turn is reported interrupted and is not repeated by itself",
        "Try again runs the turn once more and the reply arrives",
        "a message whose start cannot be recorded says so at once, runs nothing and offers no Try again",
        "the next Host delivers the message that could not start, and the reply arrives once",
        "every model call was one the scenario scripted",
    )),
    "stop": Scenario(scenario_stop, "A person stops a conversation while its turn is also working on a second "
                     "one; the second is told at once that it was not finished and Try again finishes it, rather "
                     "than it waiting for the next Host or running again by itself.",
    (
        "the turn took the second conversation in and acted on it",
        "stopping one conversation tells the other at once that it was not finished, with Try again",
        "Try again sends the cut-short message once more and it is answered",
        "every model call was one the scenario scripted",
    )),
    "failed": Scenario(scenario_failed, "A model call fails while a turn is also working on a second conversation; "
                       "the second is told at once that it was not finished, with Try again, rather than ending "
                       "silently with the turn.",
    (
        "a failed model call tells the other conversation the turn acted on at once, with Try again on each",
        "every model call was one the scenario scripted",
    )),
    "routine": Scenario(scenario_routine, "A person adds a routine on an event, copies its curl line, reveals its secret and hides it again, and one delivery "
                        "fires it once and the agent acts on it; the secret is hidden until asked for, a retry of "
                        "that delivery does not fire it again, and a post without the secret or to an event nobody "
                        "waits on fires nothing.",
    (
        "the secret is shown only when asked for, and no event carried it",
        "Copy puts the line with the real secret on the clipboard and leaves it masked on the page",
        "Copied goes back to Copy on its own",
        "Hide secret masks the line again",
        "the routine is kept, switched on, waiting on the event",
        "a post without the secret, or to an event nobody waits on, fires nothing",
        "one delivery fires the routine once, with its body, in a run of its own",
        "a retry of the same delivery does not fire it again",
        "the routine's history shows the one firing",
        "every model call was one the scenario scripted",
    )),
    "update": Scenario(scenario_update, "A person updates a box that follows main while another client's update is "
                       "running; this click is refused because main moved, and the panel says why under the "
                       "running update's step, then offers main's new build; an updater that does not answer "
                       "leaves the outcome unknown and nothing is sent twice.",
    (
        "a box on main offers the build of main that was reviewed",
        "a refused update says why, under the update another client started and its step",
        "when the other update ends, the panel offers main's new build and claims nothing as its own",
        "an updater that does not answer leaves the outcome unknown, and nothing is sent twice",
    ), updater=True),
    "newagent": Scenario(scenario_newagent, "A person opens New agent in a 1280 by 820 window and creates an agent "
                         "with a keyboard press on its one Create agent button, which is on screen without "
                         "scrolling; a press with no name creates nothing.",
    (
        "Create agent is one button, on screen without scrolling",
        "with no name, a press on Create agent creates nothing and keeps the dialog",
        "a keyboard press on Create agent creates the agent and closes the dialog",
        "every model call was one the scenario scripted",
    )),
}
