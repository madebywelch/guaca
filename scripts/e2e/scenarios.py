
from __future__ import annotations

import json
import re
import sqlite3
import threading
import urllib.request
from dataclasses import dataclass
from typing import Callable

from . import look
from .browser import Browser, label
from .world import STAND_IN_KEY, STAND_IN_MODEL, Call, Reply, World, admitted, call, held, refuse, say, text_of

AGENT = "Chief of Staff"
FIRST_TASK = "Something else Start with a task in your own words."
API_KEY = "API key Use OpenRouter or OpenAI, with usage billed to that account."
WRONG_KEY = "verify-wrong-key"
HELLO = "Hello. I am ready when you are."
ROSTER = "Agent conversations"
STATUSES = ("Idle", "Paused", "Working", "Has a question", "Has a result", "Task failed", "Needs attention")


def roster_entry(accessible_name: str, name: str) -> re.Match | None:
    return re.fullmatch(rf"{re.escape(name)}, ({'|'.join(STATUSES)})", accessible_name)


def roster_rows(w: World) -> list[dict]:
    return [row for row in reversed(w.ui.within("navigation", ROSTER, "button"))
            if label(row).rsplit(", ", 1)[-1] in STATUSES]


def roster_status(w: World, name: str) -> str:
    for row in roster_rows(w):
        if match := roster_entry(label(row), name):
            return match[1]
    return ""


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
    w.wait("Start task opens the first agent's conversation",
           lambda: ui.find("log", "Conversation with", exact=False))
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
    open_agent(w, to)
    ui.fill("combobox", f"Write to {to}", text)
    ui.click("button", "Send")


def new_agent(w: World, name: str, instructions: str) -> None:
    ui = w.ui
    ui.click("button", "Make something new")
    ui.click("menuitem", "New agent", exact=False)
    w.wait("the New agent dialog", lambda: ui.find("dialog", "New agent"))
    ui.fill("textbox", "Name", name)
    ui.click("DisclosureTriangle", "Instructions, skills and model")
    ui.fill("textbox", "Instructions", instructions)
    ui.click("button", "Create agent")
    w.wait(f"{name} exists", lambda: w.host.scalar(
        "SELECT count(*) FROM agents WHERE name = ? AND lifecycle != 'terminated'", name))
    w.wait("the dialog closes", lambda: not ui.find("dialog", "New agent"))


def open_agent(w: World, name: str) -> None:
    if w.ui.find("log", f"Conversation with {name}"):
        return
    rows = w.wait(f"{name} has one roster button", lambda: [
        row for row in roster_rows(w) if roster_entry(label(row), name)])
    if len(rows) != 1:
        raise AssertionError(f"Expected one roster button for {name}, found {[label(row) for row in rows]}")
    w.ui.press(rows[0])
    w.wait(f"{name}'s conversation opens", lambda: w.ui.find("log", f"Conversation with {name}"))


def questions(w: World, count: int) -> None:
    name = f"{count} {'question' if count == 1 else 'questions'} waiting"
    w.ui.click("button", name)
    w.wait("the Questions tray opens", lambda: w.ui.find("dialog", "Questions"))


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
    answering = threading.Event()

    def script(c: Call) -> Reply | None:
        if c.key != STAND_IN_KEY:
            return refuse(401, "Incorrect API key provided.")
        if c.last_user == "ping":
            return say("pong")
        if c.speaker != AGENT:
            return None
        if c.last_user.endswith("Say hello."):
            return held(answering, say(HELLO))
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
    w.wait("the brief's model call is in the air", lambda: asked(w, "Say hello.") == 1)
    opened = w.wait("Start task opens the agent's page", lambda: ui.find("navigation", f"{AGENT}'s page"))
    w.check("Start task opens the conversation while the agent is still answering",
            bool(ui.find("log", f"Conversation with {AGENT}")) and HELLO not in conversation(w),
            observed={"conversationShown": bool(ui.find("log", f"Conversation with {AGENT}")),
                      "answered": HELLO in conversation(w)},
            detail={"page": label(opened[0])}, shot=True)
    answering.set()
    w.wait("the agent's reply is shown", lambda: HELLO in conversation(w))
    first = settled(w, 1, "the first conversation settles")
    w.check("the brief reached the agent and its reply is in the conversation",
            HELLO in conversation(w) and "Say hello." in conversation(w),
            observed={"outcomes": first, "said": said(w, AGENT)}, shot=True)
    w.wait("the reply is drawn as a result", lambda: ui.find("article", f"Result from {AGENT}", exact=False))
    alone = ui.evaluate(LONE_RESULT)
    w.check("a lone agent's reply lands as a result card under its name, with no hand-off record",
            alone == {"name": AGENT, "text": HELLO, "records": 0},
            observed=alone, shot=True)

    write(w, AGENT, "What is two and two?")
    w.wait("the second reply is shown", lambda: "Four." in conversation(w))
    both = settled(w, 2, "the second conversation settles")
    w.check("a message written in the composer is answered in the same conversation",
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


LOOK_SURFACES = ("light", "dark")
LOOK_SCREENS = ("Questions", "a conversation", "Settings", "New agent", "the Groups sheet", "Search",
                "the command menu")
AGENT_TABS = ("Now", "Memory", "Notebook", "Routines", "Profile")
SETTINGS_TABS = ("Host", "Provider", "Limits", "Permissions", "Machines", "Account", "Appearance",
                 "Notifications", "Shortcuts", "Skills", "Compost", "About")
PLACES = ("Artifacts", "Calendar", "For you, 1 question")


def show(w: World, screen: str) -> None:
    ui = w.ui
    if screen == "Questions":
        if not ui.find("dialog", "Questions"):
            questions(w, 1)
    elif screen == "a conversation":
        open_agent(w, AGENT)
        w.wait("the request waits in the conversation", lambda: ui.find("button", "Allow"))
    elif screen == "Settings":
        ui.click("button", "App settings")
        w.wait("Settings opens", lambda: ui.find("region", "Settings"))
    elif screen == "New agent":
        ui.click("button", "Make something new")
        ui.click("menuitem", "New agent", exact=False)
        w.wait("the New agent dialog", lambda: ui.find("dialog", "New agent"))
    elif screen == "the Groups sheet":
        ui.click("button", "Agents")
        w.wait("the Groups sheet opens", lambda: ui.find("dialog", "Groups and agents"))
    elif screen == "Search":
        ui.click("button", "Search")
        w.wait("Search opens", lambda: ui.find("dialog", "Search", exact=False))
    elif screen == "the command menu":
        ui.key("k", mod=True)
        w.wait("the command menu opens", lambda: ui.find("dialog", "Command menu"))


def dismiss(w: World) -> None:
    ui = w.ui
    if ui.find("dialog", "", exact=False) or ui.find("region", "Settings"):
        ui.key("Escape")
    w.wait("every dialog and Settings close on Escape",
           lambda: not ui.find("dialog", "", exact=False) and not ui.find("region", "Settings"))
    w.wait("the page has finished moving", lambda: ui.evaluate(look.SETTLED))


def measure(w: World, screen: str, surface: str, seen: dict[str, look.Verdict]) -> None:
    ui = w.ui
    w.wait(f"{screen} has finished moving", lambda: ui.evaluate(look.SETTLED))
    seen[screen] = look.judge(f"{screen} ({surface})", ui.evaluate(look.MEASURE))
    w.capture(f"{screen} in {surface}")


def sweep(w: World, seen: dict[str, look.Verdict]) -> None:
    ui = w.ui
    open_agent(w, AGENT)
    for tab in AGENT_TABS:
        ui.click("button", tab)
        measure(w, f"the agent's {tab} tab", "light", seen)
    ui.click("button", "Conversation")
    ui.click("button", "App settings")
    w.wait("Settings opens", lambda: ui.find("region", "Settings"))
    for tab in SETTINGS_TABS:
        ui.click("tab", tab)
        w.wait(f"the {tab} pane is shown", lambda: ui.find("tab", tab)[0].get("properties") is not None)
        measure(w, f"Settings, {tab}", "light", seen)
    dismiss(w)
    for place in PLACES:
        ui.click("button", place)
        measure(w, f"{place.split(',')[0]} from the status bar", "light", seen)
        ui.key("Escape")
        dismiss(w)


def look_world(w: World) -> None:

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        if c.speaker != AGENT:
            return None
        if c.last_user.endswith("Say hello."):
            return say(HELLO)
        if c.last_user.endswith(CALL_ME):
            return ask_name(c, "Robin")
        return None

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)
    new_agent(w, "Researcher", "Find sources and say where each came from.")
    write(w, AGENT, CALL_ME)
    w.wait("the request is shown", lambda: w.ui.find("button", "Allow"))


def look_surface(w: World, surface: str) -> None:
    ui = w.ui
    ui.evaluate("localStorage.setItem('guac.prefs', JSON.stringify({...JSON.parse("
                "localStorage.getItem('guac.prefs') || '{}'), surface: '%s', look: 2})); location.reload()"
                % surface)
    w.wait(f"the page comes back {surface}", lambda: ui.evaluate(
        "document.documentElement.dataset.surface") == surface and ui.find("button", "App settings"))
    ui.viewport(*look.FRAME)


def scenario_controls(w: World) -> None:
    look_world(w)
    look_surface(w, "light")
    ui = w.ui
    field = ("combobox", f"Write to {AGENT}")
    ui.fill(*field, "   ")
    w.check("Send is unavailable for a blank draft",
            ui.property("button", "Send", "disabled"), stop=False)
    ui.fill(*field, "")
    composer = ui.evaluate("""(() => {
      const box = document.activeElement.closest('.composer');
      const ring = getComputedStyle(box);
      return {ring: [ring.outlineColor, ring.outlineWidth, ring.outlineOffset],
              shadow: ring.boxShadow,
              send: box.querySelector('button[aria-label="Send"]').textContent.trim(),
              recipient: box.querySelector('label:has(select) > [aria-hidden="true"]')?.innerText.trim(),
              face: !!box.querySelector('label:has(select) .avatar')};
    })()""")
    w.check("the composer has an icon Send and a recipient pill with a face",
            composer["send"] == "" and composer["recipient"] == AGENT and composer["face"],
            observed={k: v for k, v in composer.items() if k not in ("ring", "shadow")}, stop=False)

    edges = """['.composer', '.result', '.msg__bubble', '.ask'].map(
      selector => document.querySelector(selector)?.getBoundingClientRect().left)"""
    columns = {}
    for width, height in (look.FRAME, (390, 700)):
        ui.viewport(width, height)
        columns[width] = ui.evaluate(edges)
    w.check("the composer, result, brief and question start on the same edge",
            all(None not in values and max(values) - min(values) < 1 for values in columns.values()),
            observed=columns, stop=False)
    ui.viewport(*look.FRAME)

    show(w, "Settings")
    ui.click("tab", "General")
    ui.click("textbox", "Your name")
    field_ring = ui.evaluate("""(() => {
      const s = getComputedStyle(document.activeElement);
      return [s.outlineColor, s.outlineWidth, s.outlineOffset];
    })()""")
    neutral = ui.evaluate("""(() => {
      const probe = document.createElement('span');
      probe.style.color = 'var(--faint)';
      document.body.append(probe);
      const color = getComputedStyle(probe).color;
      probe.remove();
      return color;
    })()""")
    w.check("the focused composer and name field share one neutral ring without a halo",
            composer["ring"] == field_ring and field_ring[0] == neutral
            and field_ring[1] != "0px" and composer["shadow"] == "none",
            observed={"composer": composer["ring"], "field": field_ring}, stop=False)

    ui.click("tab", "Appearance")
    short = ui.within("radiogroup", "Reading surface", "radio")
    long = ui.within("radiogroup", "Reading size", "radio")
    w.check("short choices are segments and longer choices are visible radios",
            len(short) == 3 and len(long) == 5 and ui.evaluate("""(() => {
              const short = document.querySelector('[aria-label="Reading surface: Light"]');
              const long = document.querySelector('[aria-label="Reading size: 16px"]');
              return getComputedStyle(short).opacity === '0'
                && getComputedStyle(long).opacity === '1' && long.checked;
            })()"""), observed={"surface": len(short), "sizes": len(long)})
    ui.click("radio", "Reading surface: Light")
    ui.key("ArrowDown")
    w.check("one arrow changes the selected segment and applies the new surface",
            ui.focused() == ("radio", "Reading surface: Dark")
            and ui.property("radio", "Reading surface: Dark", "checked")
            and not ui.property("radio", "Reading surface: Light", "checked")
            and ui.evaluate("document.documentElement.dataset.surface") == "dark")
    ui.key("Tab")
    w.check("Tab leaves the radio group instead of stopping on each segment",
            ui.focused() == ("radio", "Theme: Violet"))
    ui.click("radio", "Contrast: More")
    w.check("a custom look does not falsely mark any preset selected",
            not any(ui.property("radio", label(node), "checked")
                    for node in ui.within("radiogroup", "Theme presets", "radio")))
    ui.click("tab", "Notifications")
    w.check("long notification choices use visible radios even with only two options",
            len(ui.within("radiogroup", "What a push says", "radio")) == 2
            and ui.evaluate("""(() => {
              const options = [...document.querySelectorAll(
                '[aria-label="Include what finished"], [aria-label="Only say that something finished"]'
              )];
              return options.length === 2 && options.every(input => getComputedStyle(input).opacity === '1');
            })()"""))
    ui.evaluate("""(() => {
      const send = window.fetch;
      window.formatSaves = 0;
      window.fetch = (url, init) => {
        if (!String(url).endsWith('/v1/call') || !init?.body ||
            JSON.parse(init.body).args?.patch?.delivery?.format !== 'json') return send(url, init);
        window.formatSaves++;
        return new Promise(resolve => {
          window.rejectFormatSave = () => {
            window.fetch = send;
            resolve(new Response(JSON.stringify({
              err: {kind: 'storage', message: 'Could not save the notification format.'}
            }), {status: 500}));
          };
        });
      };
    })()""")
    ui.click("radio", "Format: ntfy")
    ui.key("ArrowDown")
    w.wait("the first format save is pending", lambda: ui.evaluate("window.formatSaves === 1"))
    held_focus = ui.focused()
    ui.key(" ")
    ui.evaluate("window.rejectFormatSave()")
    w.wait("the refusal is visible", lambda: "Could not save the notification format." in ui.text())
    w.check("a refused notification format keeps focus and the saved value without a second write",
            held_focus == ("radio", "Format: JSON webhook")
            and ui.focused() == held_focus and ui.evaluate("window.formatSaves") == 1
            and ui.property("radio", "Format: ntfy", "checked")
            and w.host.call("get_settings")["delivery"]["format"] == "ntfy",
            observed={"pendingFocus": held_focus, "afterRefusal": ui.focused(),
                      "writes": ui.evaluate("window.formatSaves")})
    ui.key(" ")
    w.check("a notification format stays selected after the Host saves it",
            w.wait("the saved format", lambda: w.host.call("get_settings")["delivery"]["format"] == "json"
                   and ui.property("radio", "Format: JSON webhook", "checked")
                   and not ui.property("radio", "Format: ntfy", "checked")))
    dismiss(w)
    draft = "Keep this as a draft."
    ui.fill(*field, draft)
    ui.choose("combobox", "Recipient", "Market Researcher")
    w.check("the recipient pill moves the unsent draft to the selected conversation",
            w.wait("the new conversation holds the draft", lambda:
                   ui.find("combobox", "Write to Market Researcher")
                   and ui.value("combobox", "Write to Market Researcher") == draft)
            and w.host.scalar("SELECT count(*) FROM messages WHERE parts LIKE ?", f"%{draft}%") == 0)
    no_unscripted(w)


def scenario_look(w: World) -> None:
    look_world(w)
    ui = w.ui
    for surface in LOOK_SURFACES:
        look_surface(w, surface)
        seen: dict[str, look.Verdict] = {}
        for screen in LOOK_SCREENS:
            show(w, screen)
            measure(w, screen, surface, seen)
            dismiss(w)
        faults = [fault for verdict in seen.values() for fault in verdict.faults]
        styles = {screen: verdict.styles for screen, verdict in seen.items()}
        if surface == "light":
            sweep(w, seen)
            faults = [fault for verdict in seen.values() for fault in verdict.faults]
            styles = {screen: verdict.styles for screen, verdict in seen.items()}
            w.check("every screen in light keeps to the six type roles, the control family and the radii",
                    not faults, observed=styles, detail=faults or None, stop=False)
        else:
            w.check("every screen in dark keeps to the six type roles, the control family and the radii",
                    not faults, observed=styles, detail=faults or None, stop=False)
    no_unscripted(w)


def laid_out(w: World, what: str) -> dict:
    last: list[dict] = []

    def still() -> dict | None:
        if not (w.ui.evaluate(look.SETTLED) and w.ui.evaluate("document.fonts.status") == "loaded"):
            return None
        now = w.ui.evaluate(look.LAYOUT)
        held = now if last and last[-1] == now else None
        last.append(now)
        return held

    return w.wait(f"{what} holds still", still)


def bars_hold(w: World, surface: str) -> tuple[list[str], dict[str, dict]]:
    ui = w.ui
    laid: dict[str, dict[str, dict]] = {}
    for mode in ("overlay", "classic"):
        ui.scrollbars(mode == "classic")
        look_surface(w, surface)
        for screen in LOOK_SCREENS:
            show(w, screen)
            laid.setdefault(screen, {})[mode] = laid_out(w, screen)
            w.capture(f"{screen} in {surface}, {mode} scroll bars")
            dismiss(w)
    ui.scrollbars(False)
    faults: list[str] = []
    for screen, modes in laid.items():
        faults += [f"{screen} ({surface}): {fault}" for fault in look.moved(modes["overlay"], modes["classic"])]
        for mode, layout in modes.items():
            if (edges := layout.get("horizontal")) and (
                abs(edges["left"] - edges["right"]) > 1 or edges["scrimGutter"]
            ):
                faults.append(f"{screen} ({surface}, {mode}): dialog horizontal placement is {edges}")
    for mode, layout in laid["New agent"].items():
        if layout["rows"] != [11, 10]:
            faults.append(f"New agent ({surface}, {mode} scroll bars) lays its Character grid out in rows of "
                          f"{layout['rows']}; at {look.FRAME[0]}x{look.FRAME[1]} it is [11, 10]")
    observed = {screen: {"rows": modes["classic"]["rows"], "scrolls": modes["classic"]["scrollers"],
                         "position": modes["classic"].get("horizontal")}
                for screen, modes in laid.items()}
    return faults, observed


def scenario_scrollbars(w: World) -> None:
    look_world(w)
    w.wait("the first answer is marked seen", lambda: w.host.scalar(
        "SELECT count(*) FROM outcomes WHERE seen_at IS NULL") == 0)
    faults, observed = bars_hold(w, "light")
    w.check("every screen in light lays out the same with classic scroll bars as with overlay ones",
            not faults, observed=observed, detail=faults or None, shot=False, stop=False)
    faults, observed = bars_hold(w, "dark")
    w.check("every screen in dark lays out the same with classic scroll bars as with overlay ones",
            not faults, observed=observed, detail=faults or None, shot=False, stop=False)
    ui = w.ui
    ui.scrollbars(True)
    show(w, "Settings")
    ui.evaluate("""{
      const style = document.createElement('style');
      style.textContent = '.settings__nav::-webkit-scrollbar, .settings__pane::-webkit-scrollbar { width: 17px; }';
      document.head.append(style);
    }""")
    measured = []
    for width in ("none", "auto"):
        ui.evaluate(f"""document.querySelectorAll('.settings__nav, .settings__pane')
          .forEach(node => node.style.scrollbarWidth = '{width}')""")
        laid_out(w, "the Settings controls")
        measured.append(ui.evaluate(look.SETTINGS_CONTROLS))
    overlay, classic = measured
    w.check("Settings keeps the General tab and Your name field in place with a native gutter",
            overlay["gutters"] == [0, 0] and classic["gutters"] == [17, 17]
            and all(abs(a - b) <= 1 for key in ("general", "yourName")
                    for a, b in zip(overlay[key], classic[key])),
            observed=measured)
    no_unscripted(w)


SETTINGS_SECTIONS = ("General",) + SETTINGS_TABS
SETTINGS_FRAMES = ((1280, 820), (900, 700))
SETTINGS_PAGE = """
(() => {
  const box = (el) => { const r = el.getBoundingClientRect(); return [Math.round(r.top), Math.round(r.bottom)]; };
  const nav = document.querySelector('.settings__nav');
  const pane = document.querySelector('.settings__pane');
  const tabs = [...nav.querySelectorAll('[role=tab]')];
  return {
    navScrolls: nav.scrollHeight > nav.clientHeight,
    tabs: tabs.length,
    tabsShown: tabs.filter((t) => { const [top, bottom] = box(t); return top >= 0 && bottom <= innerHeight; }).length,
    navTop: Math.round(nav.getBoundingClientRect().top),
    paneScrolls: pane.scrollHeight > pane.clientHeight,
    paneTop: pane.scrollTop,
    inDialog: !!document.querySelector('.settings').closest('[role=dialog], .scrim'),
    roster: !!document.querySelector('.roster'),
    notices: document.querySelectorAll('main .banner').length,
    selectedFill: getComputedStyle(nav.querySelector('[aria-selected=true]')).backgroundColor,
    listFill: getComputedStyle(document.querySelector('.app > main')).backgroundColor,
    selectedWeight: Number(getComputedStyle(nav.querySelector('[aria-selected=true]')).fontWeight),
    otherWeight: Number(getComputedStyle(nav.querySelector('[aria-selected=false]')).fontWeight),
  };
})()
"""
SHEET_FITS = """
(() => {
  const dialog = [...document.querySelectorAll('[role=dialog]')].at(-1);
  const r = dialog.getBoundingClientRect();
  const foot = [...dialog.querySelectorAll('button')].filter((b) => /^(Cancel|Create agent|Create group|Delete|Close)$/.test(b.textContent.trim()));
  return {
    top: Math.round(r.top), bottom: Math.round(r.bottom), height: innerHeight,
    actions: foot.map((b) => { const q = b.getBoundingClientRect(); return [b.textContent.trim(), q.top >= 0 && q.bottom <= innerHeight && q.bottom <= r.bottom]; }),
  };
})()
"""


def section_shown(w: World) -> str:
    return selected_tab(w)[0] if selected_tab(w) else ""


def menu_section(w: World, section: str) -> None:
    ui = w.ui
    title = "App settings" if section == "General" else f"Settings: {section}"
    ui.key("k", mod=True)
    w.wait("the command menu opens", lambda: ui.find("dialog", "Command menu"))
    ui.type("textbox", "Command menu", title)
    w.wait(f"{title} is chosen", lambda: menu_choice(w) == title)
    ui.key("Enter")
    w.wait("the menu closes", lambda: not ui.find("dialog", "Command menu"))


def scenario_settings(w: World) -> None:
    w.model.script = hello
    w.start()
    first_run(w)
    settled(w, 1)
    ui = w.ui
    ui.viewport(*look.FRAME)
    gear = ui.one("button", "App settings")
    ui.press(gear)
    w.wait("Settings opens", lambda: ui.find("region", "Settings"))
    page = ui.evaluate(SETTINGS_PAGE)
    w.check("the gear opens Settings as a full-width page with a back control, and no dialog",
            not ui.find("dialog", "", exact=False) and not page["inDialog"] and not page["roster"]
            and bool(ui.find("button", AGENT)) and section_shown(w) == "General",
            observed={"dialogs": ui.names("dialog"), "inDialog": page["inDialog"], "roster": page["roster"],
                      "back": bool(ui.find("button", AGENT)), "section": section_shown(w)}, shot=True)

    fits = {}
    for width, height in SETTINGS_FRAMES:
        ui.viewport(width, height)
        w.wait(f"Settings lays out at {width}x{height}", lambda: ui.evaluate(look.SETTLED))
        fits[f"{width}x{height}"] = {key: value for key, value in ui.evaluate(SETTINGS_PAGE).items()
                                     if key in ("navScrolls", "tabs", "tabsShown")}
        w.capture(f"Settings at {width}x{height}")
    w.check("at 1280x820 and 900x700 the section list shows all thirteen sections without scrolling",
            all(not f["navScrolls"] and f["tabs"] == 13 and f["tabsShown"] == 13 for f in fits.values()),
            observed=fits)

    ui.click("tab", "Appearance")
    w.wait("the Appearance pane is shown", lambda: ui.find("heading", "Appearance"))
    before = ui.evaluate(SETTINGS_PAGE)
    ui.evaluate("document.querySelector('.settings__pane').scrollTop = 100000")
    after = ui.evaluate(SETTINGS_PAGE)
    w.check("a long section scrolls on its own, and the section list stays where it was",
            before["paneScrolls"] and after["paneTop"] > 0 and after["navTop"] == before["navTop"]
            and ui.on_screen("tab", "General") and ui.on_screen("tab", "About"),
            observed={"paneScrolls": before["paneScrolls"], "scrolled": after["paneTop"] > 0,
                      "listStayed": after["navTop"] == before["navTop"]}, shot=True)

    ui.click("tab", "Provider")
    ui.fill("textbox", "Give up on a call after", "45")
    ui.evaluate("document.querySelector('.settings__pane').scrollTop = 100000")
    w.wait("Provider has scrolled", lambda: ui.evaluate("document.querySelector('.settings__pane').scrollTop") > 0)
    w.check("with an unsaved change, Save stays on screen after scrolling to the end of a section",
            ui.on_screen("button", "Save") and ui.on_screen("button", "Discard changes"),
            observed={"save": ui.on_screen("button", "Save")}, shot=True)
    ui.click("button", "Discard changes")
    w.wait("nothing is left to save", lambda: not ui.find("button", "Save"))

    returns = {}
    for leave in ("Escape", "back", "Command-comma"):
        if not ui.find("region", "Settings"):
            ui.press(ui.one("button", "App settings"))
            w.wait("Settings opens", lambda: ui.find("region", "Settings"))
        if leave == "Escape":
            ui.key("Escape")
        elif leave == "back":
            ui.click("button", AGENT)
        else:
            ui.key(",", mod=True)
        w.wait(f"{leave} leaves Settings", lambda: not ui.find("region", "Settings"))
        returns[leave] = {"conversation": bool(ui.find("log", f"Conversation with {AGENT}")),
                          "focus": w.wait("the focus comes back", lambda: ui.focused()
                                          if ui.focused() == ("button", "App settings") else None)}
    ui.key(",", mod=True)
    opened = bool(w.wait("Command-comma opens Settings", lambda: ui.find("region", "Settings")))
    ui.key(",", mod=True)
    w.wait("Command-comma again leaves it", lambda: not ui.find("region", "Settings"))
    w.check("Escape, the back control and Command-comma each return to where Settings was opened, with the focus where it was",
            opened and all(r["conversation"] and r["focus"] == ("button", "App settings") for r in returns.values()),
            observed=returns)

    straight = {}
    for section in SETTINGS_SECTIONS:
        menu_section(w, section)
        straight[section] = w.wait(f"Settings opens on {section}", lambda: section_shown(w) == section
                                   and ui.find("region", "Settings")) and section_shown(w)
        ui.key("Escape")
        w.wait("Settings closes", lambda: not ui.find("region", "Settings"))
    ui.key(",", mod=True)
    w.wait("Settings opens", lambda: ui.find("region", "Settings"))
    within = {}
    for section in reversed(SETTINGS_SECTIONS):
        menu_section(w, section)
        within[section] = w.wait(f"the page turns to {section}", lambda: section_shown(w) == section) and section_shown(w)
    ui.key("Escape")
    w.wait("Settings closes", lambda: not ui.find("region", "Settings"))
    w.check("the command menu opens every section straight to itself, from a conversation and from Settings",
            all(straight[s] == s for s in SETTINGS_SECTIONS) and all(within[s] == s for s in SETTINGS_SECTIONS),
            observed={"fromConversation": straight, "fromSettings": within})

    open_agent(w, AGENT)
    ui.key(",", mod=True)
    w.wait("Settings opens over the agent's page", lambda: ui.find("region", "Settings"))
    gone = not ui.find("log", f"Conversation with {AGENT}")
    ui.click("button", AGENT)
    w.wait("the back control leaves Settings", lambda: not ui.find("region", "Settings"))
    w.check("opened from an agent's page, the back control is named for that agent and returns to it",
            gone and bool(ui.find("log", f"Conversation with {AGENT}")),
            observed={"replaced": gone, "back": bool(ui.find("log", f"Conversation with {AGENT}"))}, shot=True)

    ui.viewport(900, 700)
    group = w.host.scalar("SELECT name FROM groups ORDER BY rowid LIMIT 1")
    sheets = {}
    for sheet in ("New agent", "New group", "Group settings"):
        if sheet == "Group settings":
            show(w, "the Groups sheet")
            ui.click("button", f"{group} settings")
        else:
            ui.click("button", "Make something new")
            ui.click("menuitem", sheet, exact=False)
        w.wait(f"{sheet} opens", lambda: ui.find("dialog", sheet))
        w.wait(f"{sheet} has finished moving", lambda: ui.evaluate(look.SETTLED))
        sheets[sheet] = ui.evaluate(SHEET_FITS)
        w.capture(f"{sheet} at 900x700")
        while ui.find("dialog", "", exact=False):
            open_now = len(ui.names("dialog"))
            ui.key("Escape")
            w.wait("the sheet on top closes", lambda: len(ui.names("dialog")) < open_now)
    w.check("New agent, New group and a group's settings fit a 900x700 window with their actions on screen",
            all(f["top"] >= 0 and f["bottom"] <= f["height"] and f["actions"] and all(shown for _, shown in f["actions"])
                for f in sheets.values()),
            observed={k: {"fits": v["top"] >= 0 and v["bottom"] <= v["height"], "actions": v["actions"]}
                      for k, v in sheets.items()})

    ui.viewport(*look.FRAME)
    ui.key(",", mod=True)
    w.wait("Settings opens", lambda: ui.find("region", "Settings"))
    fill = ui.evaluate(SETTINGS_PAGE)
    w.check("the selected section has a fill of its own against the page and keeps its heavier weight",
            fill["selectedFill"] != fill["listFill"] and fill["selectedWeight"] > fill["otherWeight"],
            observed={key: fill[key] for key in ("selectedFill", "listFill", "selectedWeight", "otherWeight")},
            shot=True)

    crowded = {}

    def crowd(state: str, notices: int) -> None:
        for width, height in SETTINGS_FRAMES:
            ui.viewport(width, height)
            w.wait(f"Settings lays out at {width}x{height}", lambda: ui.evaluate(look.SETTLED))
            fit = ui.evaluate(SETTINGS_PAGE)
            crowded[f"{state} at {width}x{height}"] = {
                "fits": not fit["navScrolls"] and fit["tabs"] == 13 and fit["tabsShown"] == 13,
                "notices": fit["notices"], "expected": notices}
            w.capture(f"Settings with {state} at {width}x{height}")

    w.host.call("update_settings", patch={"baseUrl": "https://openrouter.ai/api/v1", "apiKey": ""})
    w.wait("the page asks for an API key", lambda: ui.find("button", "Go to Provider"))
    crowd("the no-key banner", 1)

    with urllib.request.urlopen(f"{w.host.origin}/health", timeout=10) as health:
        version = json.loads(health.read())["version"]
    ui.answer("*/v1/updates*", lambda: {
        "automatic": True, "checkedAt": "2026-11-17T00:00:00Z", "error": None, "channel": "main",
        "latest": {"schema": 1, "channel": "main", "version": version, "commit": REVIEWED,
                   "image": f"{IMAGE}{'e' * 64}", "apiGeneration": 1, "clientMinimum": 1, "clientMaximum": 1,
                   "notes": f"https://github.com/madebywelch/guaca/commit/{REVIEWED}"}})

    def offered() -> bool:
        if ui.find("button", "Review update"):
            return True
        ui.evaluate("window.dispatchEvent(new Event('focus'))")
        return False

    w.wait("the host update notice shows", offered)
    crowd("the no-key banner and the host update notice", 2)

    w.host.stop()
    w.wait("the page says the host cannot be reached", lambda: ui.find("button", "Retry"))
    crowd("the no-key banner, the host update notice and the unreachable host banner", 3)
    w.check("with the no-key banner, then the host update notice, then the unreachable host banner showing, the section list still shows all thirteen sections without scrolling at 1280x820 and 900x700",
            all(c["fits"] and c["notices"] == c["expected"] for c in crowded.values()), observed=crowded)
    ui.viewport(*look.FRAME)
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


LOOK_INTO = "Look into the quarterly filings."
READING = "Reading the quarterly filings."
FILINGS = "The filings are in order."
PICKED_UP = "restarted partway through your turn"


def running_text(w: World) -> str:
    return w.ui.evaluate("document.querySelector('.working')?.innerText ?? ''") or ""


def running_for(w: World) -> int | None:
    clock = w.ui.evaluate("document.querySelector('.working__end .working__for')?.textContent ?? ''")
    found = re.fullmatch(r"(?:(\d+)m )?(\d+)s", clock)
    return int(found.group(1) or 0) * 60 + int(found.group(2)) if found else None


def started_marks(w: World) -> list:
    return [row["started_at"] for row in w.host.rows("SELECT started_at FROM pending_runs")]


def scenario_running(w: World) -> None:
    before = threading.Event()
    after = threading.Event()
    picked = threading.Event()

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        if c.speaker != AGENT:
            return None
        if c.last_user.endswith("Say hello."):
            return say(HELLO)
        if c.said(LOOK_INTO) and c.said(PICKED_UP):
            picked.set()
            return held(after, say(FILINGS))
        if c.last_user.endswith(LOOK_INTO) and not c.tool_results():
            return call("note_progress", note=READING)
        if c.last_user.endswith(LOOK_INTO):
            return held(before, say(FILINGS))
        return None

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)
    new_agent(w, "Researcher", "Wait to be asked.")
    open_agent(w, AGENT)
    w.wait("the settled agent is idle", lambda: roster_status(w, AGENT) == "Idle")
    w.check("with nothing running, the roster says Idle and the conversation has no Stop",
            not w.ui.find("button", "Stop") and not running_text(w), observed=roster_status(w, AGENT))
    write(w, AGENT, LOOK_INTO)
    w.wait("the roster and conversation show the running work",
           lambda: roster_status(w, AGENT) == "Working" and READING in running_text(w))
    first = running_for(w)
    marks = started_marks(w)
    w.check("the running conversation shows its latest note, time since the Host started the work, and Stop",
            first is not None and len(marks) == 1 and marks[0] is not None and w.ui.find("button", "Stop"),
            observed={"status": roster_status(w, AGENT), "note": READING in running_text(w),
                      "clock": first is not None, "marked": [m is not None for m in marks]},
            detail=running_text(w), shot=True)
    w.wait("the clock ticks past three seconds", lambda: (running_for(w) or 0) >= 3)
    w.check("the clock in the conversation ticks while the turn works", (running_for(w) or 0) > (first or 0),
            observed=True, detail={"first": first, "now": running_for(w)})

    open_agent(w, "Researcher")
    w.ui.evaluate("location.reload()")
    w.wait("reload opens the running conversation rather than the last open one",
           lambda: w.ui.find("log", f"Conversation with {AGENT}") and running_for(w))
    w.check("reload prefers the running conversation to the last open conversation",
            started_marks(w) == marks and asked(w, LOOK_INTO) == 1,
            observed={"sameStart": started_marks(w) == marks, "modelCalls": asked(w, LOOK_INTO)})

    held_for = running_for(w) or 0
    w.host.stop()
    before.set()
    w.host.start()
    w.open_browser()
    w.wait("the next Host picks the turn up", picked.is_set)
    again = w.wait("the window opens on its running conversation with the clock",
                   lambda: w.ui.find("log", f"Conversation with {AGENT}")
                   and roster_status(w, AGENT) == "Working" and running_for(w))
    w.check("after a restart mid-turn the clock goes on from the first Host's start rather than from zero",
            started_marks(w) == marks and again is not None and again >= held_for,
            observed={"sameStart": started_marks(w) == marks, "carriedOn": again is not None and again >= held_for},
            detail={"before": held_for, "after": again, "marks": marks}, shot=True)

    w.ui.click("button", "Stop")
    after.set()
    ended = settled(w, 2, "the stopped conversation settles")
    w.wait("the conversation shows why work stopped", lambda: "You stopped this conversation" in conversation(w))
    w.wait("the roster clears Working and the conversation loses Stop",
           lambda: roster_status(w, AGENT) not in ("", "Working") and not w.ui.find("button", "Stop"))
    w.check("Stop ends the run and replaces the working controls with the stopped transcript",
            ended[-1]["status"] == "stopped" and not w.ui.find("button", "Stop"),
            observed={"outcomes": ended}, shot=True)
    no_unscripted(w)


ASK_BOTH = "Ask Researcher and Analyst what changed in the quarterly filings."
ASK_AGAIN = "Ask them both about the lease note."
ASK_FAILING = "Ask Analyst for the unavailable figures."
FILINGS_Q = "What changed in the quarterly filings since last quarter?"
LEASE_Q = "What does the lease note change?"
R_SAYS = "Revenue rose four percent."
A_SAYS = "Margins held at 31 percent."
BOTH_SAID = "Both answered: revenue up four percent, margins held."
TWO_EDGES = """(() => {
  const log = document.querySelector('[role=log]');
  const lefts = new Set();
  const walk = document.createTreeWalker(log, NodeFilter.SHOW_TEXT);
  for (let node = walk.nextNode(); node; node = walk.nextNode()) {
    if (!node.textContent.trim() || node.parentElement.closest('.handoff__card, .handoff__fold, .msg__head, .footnote')) continue;
    const range = document.createRange();
    range.selectNodeContents(node);
    const rect = range.getClientRects()[0];
    if (rect && rect.width > 0) lefts.add(Math.round(rect.left));
  }
  return [...lefts].sort((a, b) => a - b);
})()"""


def card_text(w: World) -> str:
    return w.ui.evaluate("[...document.querySelectorAll('.task-map__graph')].at(-1)?.innerText ?? ''") or ""


def working_line(w: World) -> str:
    return w.ui.evaluate("document.querySelector('.working')?.innerText ?? ''") or ""


TASK_MAP_FIT = """(() => {
  const stack = document.querySelector('.task-map-stack');
  const graph = stack?.querySelector('.task-map__graph');
  if (!graph) return null;
  const bounds = stack.getBoundingClientRect();
  const clip = graph.getBoundingClientRect();
  const branches = [...graph.querySelectorAll('.task-map__branches > li:not([hidden])')];
  const more = graph.parentElement.querySelector('.task-map__return button');
  const moreRect = more?.getBoundingClientRect();
  const option = graph.querySelector('li:not([hidden]) .task-map__actions button');
  const optionRect = option?.getBoundingClientRect();
  const rootNodes = [...graph.querySelectorAll('.task-map__root > div')];
  const roots = rootNodes.map(el => el.getBoundingClientRect());
  const branchList = graph.querySelector('.task-map__branches');
  const branchBounds = branchList.getBoundingClientRect();
  const near = (a, b) => Math.abs(a - b) <= 1;
  const connector = (el, pseudo, border, length) => {
    const style = el && getComputedStyle(el, pseudo);
    return Boolean(style && style.content !== 'none' && parseFloat(style[border]) === 1
      && parseFloat(style[length]) > 0
      && style.position === 'absolute');
  };
  const split = roots.length === 2 && roots[0].height > 0 && roots[1].height > 0
    && roots[0].bottom <= roots[1].top;
  const body = stack.parentElement;
  const pane = body.getBoundingClientRect();
  const style = getComputedStyle(body);
  let column = body.querySelector('[role=log]');
  while (column && column.parentElement !== body) column = column.parentElement;
  const names = branches.map(el => el.querySelector('.task-map__person').innerText.trim());
  return {
    viewport: [innerWidth, innerHeight],
    map: bounds.height,
    open: graph.closest('.task-map').getBoundingClientRect().height,
    pane: pane.height,
    width: Math.round(body.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight)),
    transcript: column ? Math.round(column.getBoundingClientRect().width) : null,
    mapWidth: Math.round(graph.closest('.task-map').getBoundingClientRect().width),
    chip: Boolean(document.querySelector('[role=log] .handoff')),
    names,
    partial: branches.filter(el => {
      const branch = el.getBoundingClientRect();
      return branch.top < Math.max(bounds.top, clip.top) - 1
        || branch.bottom > Math.min(bounds.bottom, clip.bottom) + 1
        || branch.left < clip.left - 1 || branch.right > clip.right + 1;
    }).length,
    answerVisible: Boolean(optionRect && optionRect.top >= clip.top && optionRect.bottom <= clip.bottom + 1),
    more: moreRect && moreRect.top >= bounds.top && moreRect.bottom <= bounds.bottom + 1
      && getComputedStyle(more).visibility === 'visible' ? more.innerText.trim() : null,
    horizontal: split && roots[0].left === roots[1].left
      && roots.every(root => root.right < branches[0]?.getBoundingClientRect().left),
    connected: connector(rootNodes[0], '::after', 'borderLeftWidth', 'height')
      && near(roots[0].bottom + parseFloat(getComputedStyle(rootNodes[0], '::after').height), roots[1].top)
      && connector(rootNodes[1], '::after', 'borderTopWidth', 'width')
      && near(roots[1].right + parseFloat(getComputedStyle(rootNodes[1], '::after').width), branchBounds.left)
      && connector(branchList, '::before', 'borderLeftWidth', 'height')
      && branches.length > 0 && branches.every(branch =>
        connector(branch, '::before', 'borderTopWidth', 'width')
        && near(branch.getBoundingClientRect().left - parseFloat(getComputedStyle(branch, '::before').width),
          branchBounds.left)),
    centeredLead: split && near(roots[0].top + roots[1].bottom, branchBounds.top + branchBounds.bottom),
    stacked: split && roots[1].bottom < branches[0]?.getBoundingClientRect().top,
    leadStatus: rootNodes[1]?.querySelector('p')?.innerText.trim() ?? '',
    replyOnOneLine: branches.some(branch => {
      const box = branch.getBoundingClientRect();
      const name = branch.querySelector('.task-map__person')?.getBoundingClientRect();
      const state = branch.querySelector('.task-map__branch-head > span')?.getBoundingClientRect();
      const reply = branch.querySelector('.task-map__reply button')?.getBoundingClientRect();
      return reply?.height > 0 && [name, state].every(rect => rect?.height > 0
        && rect.top < reply.bottom && reply.top < rect.bottom && rect.left >= box.left
        && rect.right <= reply.left) && reply.right <= box.right;
    }),
  };
})()"""


TASK_MAP_FRAMES = ((1280, 820), (900, 700), (700, 700))
TRANSCRIPT_WIDTH = 736
HORIZONTAL_FROM = 740


def hidden_branches(fit: dict) -> int:
    return int(re.match(r"Show (\d+) more", fit["more"])[1]) if fit["more"] else 0


def painted_task_map(ui: Browser) -> dict | None:
    return ui.evaluate(f"""(async () => {{
      const nextFrame = () => new Promise(resolve => requestAnimationFrame(resolve));
      const measure = () => {TASK_MAP_FIT};
      await nextFrame();
      const first = measure();
      await nextFrame();
      const second = measure();
      await nextFrame();
      const third = measure();
      return JSON.stringify(first) === JSON.stringify(second)
        && JSON.stringify(second) === JSON.stringify(third) ? third : null;
    }})()""", await_promise=True)


def map_matrix(w: World, case: str, surface: str, names: tuple[str, ...], layouts: dict,
               question: bool) -> None:
    ui = w.ui
    frames = (*TASK_MAP_FRAMES, (900, 820), (900, 920), (900, 560)) if case == "two colleagues" else TASK_MAP_FRAMES
    for width, height in frames:
        ui.viewport(width, height)
        fit = w.wait(f"the {case} map settles at {width}x{height}",
                     lambda: (fit := painted_task_map(ui)) and fit["viewport"] == [width, height]
                     and fit["partial"] == 0 and len(fit["names"]) + hidden_branches(fit) == len(names)
                     and (not question or width == 700 or fit["answerVisible"]) and fit)
        rest = []
        if fit["more"]:
            ui.click("button", fit["more"])
            w.wait("the colleagues disclosure opens", lambda: ui.find("dialog", "More colleagues"))
            rest = ui.evaluate("[...document.querySelectorAll('.task-map__detail .task-map__branches > li')]"
                               ".map(el => el.querySelector('.task-map__person').innerText.trim())")
            if case == "fan-out" and width == 1280:
                w.capture(f"{case} colleagues {surface} {width}x{height}")
            ui.click("button", "Close")
            w.wait("the colleagues disclosure closes", lambda: not ui.find("dialog", "More colleagues"))
        w.capture(f"{case} {surface} {width}x{height}")
        layouts[f"{case} {surface} {width}x{height}"] = {**fit, "rest": rest}


def lays_out_by_pane(d: dict, question: bool) -> bool:
    return (d["partial"] == 0 and d["horizontal"] == (d["width"] >= HORIZONTAL_FROM)
            and d["connected"] == d["horizontal"] and (not d["horizontal"] or d["centeredLead"])
            and d["transcript"] is not None and abs(d["transcript"] - min(TRANSCRIPT_WIDTH, d["width"])) <= 1
            and d["mapWidth"] <= d["width"] and d["map"] <= .4 * d["pane"] + 1 and not d["chip"]
            and (d["viewport"][0] != 1280 or d["mapWidth"] > d["transcript"])
            and (d["viewport"] != [900, 920] or d["stacked"])
            and (not question or d["viewport"][0] == 700 or d["answerVisible"]))


def opens_remainder(d: dict, names: tuple[str, ...]) -> bool:
    return sorted(d["names"] + d["rest"]) == sorted(names) and not set(d["names"]) & set(d["rest"])


def scenario_handoff(w: World) -> None:
    analyst = threading.Event()
    never = threading.Event()
    analyst_failure = threading.Event()

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        results = c.tool_results()
        if c.speaker == AGENT:
            if c.last_user.endswith("Say hello."):
                return say(HELLO)
            for ask, question in ((ASK_BOTH, FILINGS_Q), (ASK_AGAIN, LEASE_Q)):
                if c.last_user.endswith(ask):
                    return say("I asked them both.") if results else call(
                        "send_message", to=["Researcher", "Analyst"], text=question, intent="work")
            if A_SAYS in c.last_user:
                return say(BOTH_SAID)
            if R_SAYS in c.last_user:
                return say("Researcher answered.")
            if c.last_user.endswith(ASK_FAILING):
                return say("I asked Analyst to check.") if results else call(
                    "send_message", to=["Analyst"], text="Find the unavailable figures.", intent="work")
        if c.speaker == "Researcher" and (FILINGS_Q in c.last_user or LEASE_Q in c.last_user):
            return say(R_SAYS)
        if c.speaker == "Analyst" and FILINGS_Q in c.last_user:
            return held(analyst, say(A_SAYS))
        if c.speaker == "Analyst" and LEASE_Q in c.last_user:
            return held(never, say(A_SAYS))
        if c.speaker == "Analyst" and "Find the unavailable figures." in c.last_user:
            return held(analyst_failure, refuse(500, "The model is overloaded."))
        return None

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)
    new_agent(w, "Researcher", "Find sources and say where each came from.")
    new_agent(w, "Analyst", "Read the numbers and say what moved.")
    open_agent(w, AGENT)
    write(w, AGENT, ASK_BOTH)
    ui = w.ui
    w.wait("the map shows Researcher's answer while Analyst works", lambda: R_SAYS in card_text(w)
           and "waiting for Analyst" in working_line(w))
    fit = w.wait("the live map holds its layout through consecutive paints", lambda: painted_task_map(ui))
    w.check("a pinned branch is fully visible or behind the colleagues control",
            fit["partial"] == 0 and len(fit["names"]) + (1 if fit["more"] == "Show 1 more colleague" else 0) == 2
            and not ui.find("button", "Open task map"), observed=fit)
    w.check("the pinned map shows each colleague, the work still underway, and the result already sent",
            ui.find("region", "Tasks in progress") and "Researcher" in fit["names"]
            and "Sent a message" in card_text(w) and "waiting for Analyst" in working_line(w),
            observed={"shown": fit["names"], "running": "waiting for Analyst" in working_line(w),
                      "answered": "Sent a message" in card_text(w)},
            detail=card_text(w), shot=True)
    if fit["more"]:
        ui.click("button", fit["more"])
        w.check("the disclosure reveals the whole hidden colleague branch",
                ui.find("dialog", "More colleagues") and ui.within("dialog", "More colleagues", "button")
                and "Analyst" in ui.evaluate("document.querySelector('.task-map__detail').innerText"),
                observed="Analyst")
        ui.click("button", "Close")
    else:
        w.check("the disclosure reveals the whole hidden colleague branch",
                "Analyst" in fit["names"], observed="Analyst is already visible")
    w.wait("the status line says who it waits for", lambda: "waiting for Analyst" in working_line(w))
    w.check("under the running turn a status line says what it is doing, for how long, with Stop",
            working_line(w).startswith("Messaged Researcher and Analyst, waiting for Analyst")
            and re.search(r"\d+s", working_line(w)) and ui.find("button", "Stop"),
            observed=re.sub(r"\d+s", "Ns", working_line(w)))
    edges = ui.evaluate(TWO_EDGES)
    w.check("text in the transcript starts on two left edges", len(edges) <= 2, observed=len(edges), detail=edges)
    fills = ui.evaluate("""(() => {
  const bubble = document.querySelector('.msg__bubble');
  let under = bubble.parentElement;
  while (under && ['rgba(0, 0, 0, 0)', 'transparent'].includes(getComputedStyle(under).backgroundColor))
    under = under.parentElement;
  return {bubble: getComputedStyle(bubble).backgroundColor, page: under && getComputedStyle(under).backgroundColor};
})()""")
    w.check("the person's turn sits in a bubble that stands apart from the page",
            fills["bubble"] not in ("rgba(0, 0, 0, 0)", fills["page"]), observed=fills)
    mask = ui.evaluate("getComputedStyle(document.querySelector('[role=log]')).maskImage")
    w.check("a turn scrolled under the tabs fades out instead of being cut by them",
            mask.startswith("linear-gradient(rgba(0, 0, 0, 0)"), observed=mask)
    seen: dict[str, look.Verdict] = {}
    peers = ("Researcher", "Analyst")
    layouts = {}
    for surface in LOOK_SURFACES:
        ui.viewport(*look.FRAME)
        ui.evaluate("localStorage.setItem('guac.prefs', JSON.stringify({...JSON.parse("
                    "localStorage.getItem('guac.prefs') || '{}'), surface: '%s', look: 2})); location.reload()"
                    % surface)
        w.wait(f"the page comes back {surface}", lambda: ui.evaluate(
            "document.documentElement.dataset.surface") == surface and ui.find("button", "App settings"))
        w.wait("the roster restores the running coordinator", lambda: roster_status(w, AGENT) == "Working")
        open_agent(w, AGENT)
        w.wait("the live map is drawn again", lambda: R_SAYS in card_text(w))
        map_matrix(w, "two colleagues", surface, peers, layouts, question=False)
        ui.viewport(*look.FRAME)
        w.wait("the live map is drawn at the measured frame", lambda: R_SAYS in card_text(w)
               and ui.evaluate(TASK_MAP_FIT)["viewport"] == list(look.FRAME))
        measure(w, f"a hand-off in {surface}", surface, seen)
    w.check("the two-colleague map lays out by its pane's width in both themes at every frame",
            all(lays_out_by_pane(d, question=False)
                and ("Researcher" not in d["names"] or "Analyst" in d["names"][:1])
                and (not d["horizontal"] or d["leadStatus"] == "Waiting for Analyst")
                and (d["viewport"] not in ([900, 700], [900, 820]) or
                     (d["names"] == ["Analyst", "Researcher"] and d["replyOnOneLine"]))
                and (d["viewport"] != [900, 560] or
                     (d["names"] == ["Analyst"] and d["rest"] == ["Researcher"]))
                for d in layouts.values()), observed=layouts)
    w.check("the colleagues disclosure opens exactly the branches the two-colleague map hides",
            all(opens_remainder(d, peers) for d in layouts.values()),
            observed={key: {"shown": d["names"], "rest": d["rest"]} for key, d in layouts.items()})
    faults = [fault for verdict in seen.values() for fault in verdict.faults]
    w.check("the hand-off keeps to the six type roles, the control family and the radii in light and dark",
            not faults, observed={s: v.styles for s, v in seen.items()}, detail=faults or None)
    placed = ui.evaluate("""(() => {
  const line = document.querySelector('.working');
  const box = line.getBoundingClientRect();
  return {inLog: Boolean(line.closest('[role=log]')), onScreen: box.top >= 0 && box.bottom <= innerHeight};
})()""")
    w.check("an agent opened mid-turn shows its status line without scrolling",
            placed == {"inLog": False, "onScreen": True}, observed=placed)

    ui.click("button", "Open Researcher’s message")
    w.wait("the colleague's message opens before delivery",
           lambda: ui.find("dialog", "Message from Researcher"))
    analyst.set()
    w.wait("the map folds once the result lands", lambda: ui.evaluate(
        "[...document.querySelectorAll('.pane__scroll .task-map__summary')]"
        ".some(summary => summary.innerText.includes('Hand-off complete'))"))
    w.check("a colleague's open message stays readable after the pinned task settles",
            ui.find("dialog", "Message from Researcher")
            and ui.evaluate("document.querySelector('.task-map__detail .md')?.innerText") == R_SAYS,
            observed=R_SAYS)
    ui.click("button", "Close")
    w.check("after the reply the hand-off folds to one line that says both answered",
            not card_text(w) and BOTH_SAID in conversation(w)
            and ui.find("button", "2 of 2 answered", exact=False),
            observed={"folded": not card_text(w), "answered": 2}, shot=True)
    ui.click("button", "Hand-off complete", exact=False)
    w.check("the folded line opens onto both answers",
            R_SAYS in card_text(w) and A_SAYS in card_text(w), observed=True, detail=card_text(w))
    ui.click("button", "Open Researcher’s message")
    w.check("opening a colleague's message shows their recorded words",
            ui.evaluate("document.querySelector('.task-map__detail .md')?.innerText") == R_SAYS,
            observed=R_SAYS)
    ui.click("button", "Close")
    ui.click("button", "Hand-off complete", exact=False)
    w.wait("the opened hand-off folds again", lambda: not card_text(w))

    settled(w, 2)
    write(w, AGENT, ASK_AGAIN)
    w.wait("the second hand-off is live", lambda: R_SAYS in card_text(w))
    w.wait("the status line offers Stop", lambda: ui.find("button", "Stop"))
    w.wait("the ongoing hand-off has a start mark", lambda: started_marks(w))
    ui.click("button", "Stop")
    w.wait("the host records Stop before the held answer can come back", lambda: not started_marks(w))
    never.set()
    ended = settled(w, 3, "the stopped conversation settles")
    w.wait("the stopped hand-off folds", lambda: ui.find(
        "button", "1 of 2 answered", exact=False))
    w.check("a hand-off cut short by Stop folds and says who answered, instead of ticking on",
            ended[-1]["status"] == "stopped" and not card_text(w)
            and not ui.find("button", "Stop"),
            observed={"outcome": ended[-1]["status"]}, shot=True)
    write(w, AGENT, ASK_FAILING)
    w.wait("the lead's dispatch note is visible during the delegated work",
           lambda: "I asked Analyst to check." in conversation(w))
    w.wait("the failed task already has a live map", lambda: "Analyst" in card_text(w))
    ui.evaluate("""(() => {
      const fetchFromHost = window.fetch;
      window.fetch = (input, options) => {
        if (String(input).endsWith('/v1/call') && options?.body &&
            JSON.parse(options.body).name === 'task_record') {
          return Promise.reject(new TypeError('The task record could not be read.'));
        }
        return fetchFromHost(input, options);
      };
      window.restoreTaskReads = () => { window.fetch = fetchFromHost; };
    })()""")
    analyst_failure.set()
    failed = settled(w, 4, "the delegated failure is recorded")
    w.wait("the task offers to retry the failed read", lambda: ui.find("button", "Retry task map"))
    w.check("a failed map read cannot turn a progress note into a result",
            not ui.evaluate("[...document.querySelectorAll('.msg--result')].some(el => "
                            "el.innerText.includes('I asked Analyst to check.'))"), observed=True)
    ui.evaluate("window.restoreTaskReads()")
    ui.click("button", "Retry task map")
    w.wait("the map records that the hand-off ended without a result", lambda: ui.evaluate(
        "[...document.querySelectorAll('.task-map__summary')].some(el => "
        "el.innerText.includes('Hand-off failed') && el.innerText.includes('unavailable figures'))"))
    if not ui.evaluate("[...document.querySelectorAll('.task-map')].some(map => map.querySelector('.task-map__summary')"
                       "?.getAttribute('aria-expanded') === 'true')"):
        ui.click("button", "Hand-off failed", exact=False)
    branch = w.wait("the failed colleague's branch is drawn", lambda: ui.evaluate(
        "(() => { const node = document.querySelector('.task-map__node[data-failed]');"
        " return node && {status: node.querySelector('.task-map__branch-head > span')?.innerText,"
        " said: node.querySelector('.task-map__failure .task-map__excerpt')?.innerText,"
        " retry: [...node.querySelectorAll('button')].map(b => b.innerText).includes('Try again')}; })()"))
    w.check("a colleague whose model call failed is marked failed on the map, in plain words, with Try again",
            branch == {"status": "Failed", "said": FAILED_PLAINLY.format(name="Analyst"), "retry": True},
            observed=branch)
    w.check("a colleague's failure does not promote the lead's progress note to a result",
            failed[-1]["status"] == "failed" and not ui.evaluate(
                "[...document.querySelectorAll('.msg--result')].some(el => "
                "el.innerText.includes('I asked Analyst to check.'))"), observed=failed[-1]["status"])
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
    edges = w.ui.evaluate("""['.composer', ...document.querySelectorAll('.wire--notice > .chip')].map(
      item => (typeof item === 'string' ? document.querySelector(item) : item).getBoundingClientRect().left)""")
    w.check("both failure notices start at the composer edge",
            len(edges) == 3 and max(edges) - min(edges) < 1, observed=edges)
    no_unscripted(w)


FAILED_PLAINLY = "{name} could not reply: the provider had a problem on its side. Wait a minute, then try again."
CHECK_INVOICE = "Check the failed invoice."
FIGURES = "Find the unavailable figures."
FIGURES_FOUND = "The figures are in the appendix."
NOTICE_INK = """(() => {
  const chip = [...document.querySelectorAll('.chip--error')].at(-1);
  if (!chip) return null;
  const said = chip.querySelector('.chip__said');
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 1;
  const pen = canvas.getContext('2d', {willReadFrequently: true});
  const layers = [];
  for (let el = chip; el; el = el.parentElement) layers.push(getComputedStyle(el).backgroundColor);
  pen.fillStyle = document.documentElement.dataset.surface === 'dark' ? '#000' : '#fff';
  pen.fillRect(0, 0, 1, 1);
  for (const layer of layers.reverse()) { pen.fillStyle = layer; pen.fillRect(0, 0, 1, 1); }
  const back = [...pen.getImageData(0, 0, 1, 1).data];
  pen.fillStyle = getComputedStyle(said).color;
  pen.fillRect(0, 0, 1, 1);
  const ink = [...pen.getImageData(0, 0, 1, 1).data];
  const light = ([r, g, b]) => [r, g, b].map(v => v / 255)
    .map(v => v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)
    .reduce((sum, v, i) => sum + v * [0.2126, 0.7152, 0.0722][i], 0);
  const [high, low] = [light(ink), light(back)].sort((a, b) => b - a);
  const retry = [...chip.querySelectorAll('button')].find(b => b.innerText.startsWith('Try again'));
  if (!retry) return null;
  const probe = document.createElement('div');
  probe.style.height = 'var(--control-h)';
  document.body.append(probe);
  const control = probe.getBoundingClientRect().height;
  probe.remove();
  const box = retry.getBoundingClientRect();
  return {ratio: Math.round((high + 0.05) / (low + 0.05) * 100) / 100,
          retryOnOneLine: retry.scrollWidth <= retry.clientWidth && box.height < control * 1.5,
          retryAtControlHeight: control > 0 && Math.abs(box.height - control) < 1};
})()"""


def scenario_failure(w: World) -> None:
    analyst = threading.Event()
    retrying = threading.Event()

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        if c.speaker == AGENT and c.last_user.endswith("Say hello."):
            return say(HELLO)
        if c.speaker == AGENT and c.last_user.endswith(CHECK_INVOICE):
            return refuse(500, "The model is overloaded.")
        if c.speaker == "Analyst" and c.last_user.endswith(FIGURES):
            return say(FIGURES_FOUND) if retrying.is_set() else held(analyst, refuse(500, "The model is overloaded."))
        return None

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)
    ui = w.ui
    new_agent(w, "Analyst", "Answer questions about figures.")

    write(w, AGENT, CHECK_INVOICE)
    w.wait("the failure is shown where it happened", lambda: ui.find("button", "Try again"))
    words = ui.evaluate("[...document.querySelectorAll('.chip--error .chip__said')].at(-1)?.innerText")
    shown = conversation(w)
    w.check("a failed model call says what happened and what to do, in plain words, with no status code",
            words == FAILED_PLAINLY.format(name=AGENT)
            and not any(word in shown for word in ("HTTP", "500", "overloaded", "inference endpoint")),
            observed={"said": words, "speaksHttp": "HTTP" in shown})
    ui.click("button", "Details")
    detail = w.wait("Details opens", lambda: ui.evaluate(
        "(() => { const d = [...document.querySelectorAll('.chip__detail')].at(-1); return d && !d.hidden && d.innerText; })()"))
    w.check("Details shows the provider's own words for someone who wants them",
            "HTTP 500" in detail and "The model is overloaded." in detail, observed=detail)
    ink = {}
    for surface in LOOK_SURFACES:
        look_surface(w, surface)
        open_agent(w, AGENT)
        ink[surface] = w.wait(f"the notice is drawn in {surface}", lambda: ui.evaluate(NOTICE_INK))
    w.check("the failure's words measure at least 4.5 to 1 on their background in light and dark, with Try again on one line",
            all(m["ratio"] >= 4.5 and m["retryOnOneLine"] for m in ink.values()),
            observed={s: m["retryOnOneLine"] for s, m in ink.items()}, detail=ink, shot=True)
    w.check("Try again is a standard control at the standard control height",
            all(m["retryAtControlHeight"] for m in ink.values()),
            observed={s: m["retryAtControlHeight"] for s, m in ink.items()})

    ui.click("button", "Now")
    now = w.wait("the Now tab says the last run failed", lambda: ui.evaluate(
        "(() => { const line = document.querySelector('.doing__line'); return line && line.innerText; })()"))
    w.check("the Now tab calls the last run failed, in the same words, and offers Try again",
            now.startswith("Last run failed") and FAILED_PLAINLY.format(name=AGENT) in now
            and bool(ui.find("button", "Try again")), observed=bool(ui.find("button", "Try again")), detail=now)
    ui.click("button", "Conversation")

    write(w, "Analyst", FIGURES)
    w.wait("the message to Analyst is recorded", lambda: w.host.scalar(
        "SELECT count(*) FROM messages WHERE parts LIKE ?", f"%{FIGURES}%"))
    open_agent(w, AGENT)
    analyst.set()
    ended = settled(w, 3, "Analyst's failure is recorded")
    w.wait("the roster says Analyst's task failed", lambda: roster_status(w, "Analyst") == "Task failed")
    w.check("an agent whose last task failed is listed as failed, not as needing attention",
            ended[-1]["status"] == "failed" and roster_status(w, "Analyst") == "Task failed",
            observed=roster_status(w, "Analyst"))
    ui.click("button", "For you")
    w.wait("For you opens", lambda: ui.find("dialog", "For you", exact=False))
    regions = w.wait("the failure is filed on its own", lambda: ui.evaluate("""(() => {
      const named = (id) => document.getElementById(id)?.closest('section')?.innerText ?? null;
      const stalled = named('for-you-unfinished');
      return stalled && {stalled, finished: named('for-you-finished')};
    })()"""))
    w.check("For you files the failed task under Did not finish with Try again, never with finished work",
            FAILED_PLAINLY.format(name="Analyst").split(".")[0] in regions["stalled"]
            and "Try again" in regions["stalled"]
            and "could not reply" not in (regions["finished"] or ""),
            observed={"stalled": "Analyst could not reply" in regions["stalled"],
                      "finishedHasFailure": "could not reply" in (regions["finished"] or "")}, shot=True)
    retrying.set()
    ui.press(next(node for node in ui.within("region", "Did not finish", "button") if label(node) == "Try again"))
    again = settled(w, 4, "the task sent again settles")
    w.wait("Did not finish empties once the task is sent again", lambda: not ui.evaluate(
        "Boolean(document.getElementById('for-you-unfinished'))"))
    w.check("Try again from For you sends the same task again and it finishes",
            again[-1] == {"status": "done", "trigger": "retry", "agent": "Analyst"}
            and FIGURES_FOUND in said(w, "Analyst"),
            observed=again[-1])
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
    label = f"For you, {waiting} question" if waiting == 1 else f"For you, {waiting} questions"
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
    w.wait("the page is back with the release check answered", lambda: admitted(ui))

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
CHEF = "Chef"


def hello(c: Call) -> Reply | None:
    if c.last_user == "ping":
        return say("pong")
    if c.speaker == AGENT and c.last_user.endswith("Say hello."):
        return say(HELLO)
    return None


SEASON_STATE = """(() => ({
  season: document.documentElement.dataset.season || null,
  greeting: document.querySelector('.roster__greeting')?.innerText.trim() || null,
  avatars: document.querySelectorAll('.roster .avatar, .pane__header .avatar').length,
  accents: [...document.querySelectorAll('.roster .avatar__accent, .pane__header .avatar__accent')]
    .map((accent) => accent.getAttribute('data-accent')),
  garland: !!document.querySelector('.pane__garland'),
  scene: !!document.querySelector('.roster__scene'),
}))()"""


def utc_ms(year: int, month: int, day: int, hour: int = 0, minute: int = 0, second: int = 0) -> int:
    from datetime import datetime, timezone
    return int(datetime(year, month, day, hour, minute, second, tzinfo=timezone.utc).timestamp() * 1000)


def on_day(w: World, at_ms: int, what: str, season: str | None) -> dict:
    w.ui.clock(at_ms)
    w.ui.evaluate("window.dispatchEvent(new Event('focus'))")
    return w.wait(what, lambda: (state := w.ui.evaluate(SEASON_STATE))["season"] == season and state)


def scenario_season(w: World) -> None:
    w.model.script = hello
    w.start()
    first_run(w)
    settled(w, 1)
    ui = w.ui
    open_agent(w, AGENT)

    ui.clock(utc_ms(2026, 11, 26, 9))
    ui.evaluate("location.reload()")
    w.wait("the page comes back on Thanksgiving", lambda: ui.find("button", "App settings")
           and ui.evaluate(SEASON_STATE)["avatars"] > 0)
    ui.viewport(*look.FRAME)
    open_agent(w, AGENT)
    thanksgiving = w.wait("Thanksgiving is drawn", lambda: (state := ui.evaluate(SEASON_STATE))["greeting"] and state)
    w.check("on the fourth Thursday of November the root, the greeting and every face say Thanksgiving",
            thanksgiving["season"] == "thanksgiving" and thanksgiving["greeting"] == "Happy Thanksgiving"
            and thanksgiving["accents"] == ["thanksgiving"] * thanksgiving["avatars"]
            and thanksgiving["garland"] and thanksgiving["scene"],
            observed=thanksgiving, shot=True)
    seen = look.judge("a conversation on Thanksgiving", ui.evaluate(look.MEASURE))
    w.check("the decorated conversation keeps to the six type roles, the control family and the radii",
            not seen.faults, observed=seen.styles, detail=seen.faults or None)

    after = on_day(w, utc_ms(2026, 11, 27, 0, 0, 30), "the season ends the day after", None)
    w.check("the day after, focusing the window clears the season, the greeting and every accent",
            after == {**after, "season": None, "greeting": None, "accents": [], "garland": False, "scene": False},
            observed=after, shot=True)

    christmas = on_day(w, utc_ms(2026, 12, 25, 9), "Christmas is drawn", "christmas")
    christmas = w.wait("the greeting says Merry Christmas",
                       lambda: (state := ui.evaluate(SEASON_STATE))["greeting"] == "Merry Christmas" and state)
    w.check("on December 25 the root, the greeting and every face say Christmas",
            christmas["accents"] == ["christmas"] * christmas["avatars"] and christmas["garland"]
            and christmas["scene"], observed=christmas, shot=True)

    ui.click("button", "App settings")
    w.wait("Settings opens", lambda: ui.find("region", "Settings"))
    ui.click("tab", "Appearance")
    w.wait("the Appearance pane is shown", lambda: ui.find("heading", "Appearance"))
    ui.click("switch", "Seasonal looks")
    dismiss(w)
    off = w.wait("switching seasonal looks off clears the season",
                 lambda: (state := ui.evaluate(SEASON_STATE))["season"] is None and state)
    w.check("with Seasonal looks off, Christmas Day draws no palette, greeting, accent or decoration",
            off == {**off, "greeting": None, "accents": [], "garland": False, "scene": False}
            and ui.evaluate("localStorage.getItem('guac.prefs')").count('"seasonal":false') == 1,
            observed=off, shot=True)
    no_unscripted(w)


def scenario_newagent(w: World) -> None:
    w.model.script = hello
    w.start()
    w.ui.viewport(*WINDOW)
    first_run(w)
    settled(w, 1)
    ui = w.ui
    chosen = "document.querySelector('.swatches input:checked')?.closest('label')?.style.getPropertyValue('--swatch')"

    ui.click("button", "Make something new")
    ui.click("menuitem", "New agent", exact=False)
    w.wait("the New agent dialog", lambda: ui.find("dialog", "New agent"))
    w.check("Create agent is one button, on screen without scrolling",
            len(ui.find("button", "Create agent")) == 1 and ui.on_screen("button", "Create agent"),
            observed={"buttons": len(ui.find("button", "Create agent")),
                      "onScreen": bool(ui.find("button", "Create agent")) and ui.on_screen("button", "Create agent")},
            shot=True)

    opened_on = ui.focused()
    ui.key("Enter")
    ui.click("button", "Create agent")
    w.check("with no name, Return in Name and a press on Create agent create nothing and keep the dialog",
            opened_on == ("textbox", "Name") and ui.property("button", "Create agent", "disabled")
            and bool(ui.find("dialog", "New agent")) and w.host.scalar("SELECT count(*) FROM agents") == 3,
            observed={"focus": list(opened_on), "disabled": ui.property("button", "Create agent", "disabled"),
                      "dialog": bool(ui.find("dialog", "New agent")),
                      "agents": w.host.scalar("SELECT count(*) FROM agents")})

    ui.fill("textbox", "Name", TRAVEL)
    stops: list[str] = []
    while ui.focused() != ("button", "Create agent") and len(stops) < 40:
        ui.key("Tab")
        stops.append(ui.focused()[0])
    w.check("from Name, Create agent is five Tabs away, one stop each for Color, Character and the options",
            stops == ["radio", "radio", "DisclosureTriangle", "button", "button"]
            and not ui.find("textbox", "Instructions"),
            observed={"stops": stops, "instructionsInTabPath": bool(ui.find("textbox", "Instructions"))},
            detail={"names": ui.names("radio")[:3]}, shot=True)

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
    landed = w.wait(f"the focus lands in {TRAVEL}'s composer", lambda: (
        lambda f: f if f[1].startswith(f"Write to {TRAVEL}") else None)(ui.focused()))
    w.check("after Create, the focus is in the new agent's composer",
            landed[0] == "combobox" and bool(ui.find("log", f"Conversation with {TRAVEL}")),
            observed={"role": landed[0], "composer": landed[1].startswith(f"Write to {TRAVEL}")}, shot=True)

    ui.click("button", "Make something new")
    ui.click("menuitem", "New agent", exact=False)
    w.wait("the New agent dialog", lambda: ui.find("dialog", "New agent"))
    ui.fill("textbox", "Name", CHEF)
    ui.key("Tab")
    before = ui.evaluate(chosen)
    ui.key("ArrowDown")
    after = ui.evaluate(chosen)
    moved_within = ui.focused()[0] == "radio"
    ui.key("Tab", shift=True)
    ui.key("Enter")
    w.wait(f"{CHEF} exists", lambda: w.host.scalar(
        "SELECT count(*) FROM agents WHERE name = ? AND lifecycle != 'terminated'", CHEF))
    w.wait("the dialog closes", lambda: not ui.find("dialog", "New agent"))
    w.check("an arrow key changes the color inside its one stop, and Return in Name creates the agent with it",
            moved_within and before != after
            and w.host.scalar("SELECT color FROM agents WHERE name = ?", CHEF) == after,
            observed={"withinGroup": moved_within, "changed": before != after,
                      "saved": w.host.scalar("SELECT color FROM agents WHERE name = ?", CHEF) == after},
            detail={"before": before, "after": after})
    landed = w.wait(f"the focus lands in {CHEF}'s composer", lambda: (
        lambda f: f if f[1].startswith(f"Write to {CHEF}") else None)(ui.focused()))
    w.check("after Return in Name, the focus is in that agent's composer",
            landed[0] == "combobox", observed={"role": landed[0]}, shot=True)
    no_unscripted(w)


BOOK_FLIGHT = "Book the flight."
WHICH = "Which flight, the early one or the late one?"


def scenario_answer(w: World) -> None:

    def script(c: Call) -> Reply | None:
        if c.speaker == AGENT and c.last_user.endswith(BOOK_FLIGHT):
            results = c.tool_results()
            if not results:
                return call("ask_operator", question=WHICH, options=["Early", "Late"])
            return say(f"Booked: {results[-1][:80]}")
        return hello(c)

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)
    ui = w.ui

    write(w, AGENT, BOOK_FLIGHT)
    w.wait("the question is shown in the conversation", lambda: ui.find("button", "Early"))
    w.wait("the composer keeps the focus after sending", lambda: ui.focused() == ("combobox", f"Write to {AGENT}"))
    w.wait("the composer offers Tab to answer", lambda: f"to answer {AGENT}" in ui.text())
    ui.key("Tab")
    w.check("with a question waiting, Tab from the composer reaches its first answer",
            ui.focused() == ("button", "Early"), observed=ui.focused(), shot=True)
    ui.key("Enter")
    settled(w, 2, "the answered turn finishes")
    w.check("Return answers the question, and the agent is told the answer",
            [a["state"] for a in approvals(w)] == ["answered"] and "Early" in said(w, AGENT)[-1],
            observed={"approvals": [a["state"] for a in approvals(w)], "said": said(w, AGENT)[-1]})

    ui.click("combobox", f"Write to {AGENT}")
    w.wait("the composer has the focus again", lambda: ui.focused() == ("combobox", f"Write to {AGENT}"))
    ui.key("Tab")
    w.check("with nothing waiting, Tab leaves the composer for its own next control",
            ui.focused() == ("button", "Attach a file"), observed=ui.focused())
    no_unscripted(w)


WRITE_NOTE = "Write the note about the meeting move."
DRAFTED_NOTE = ("Draft\n\n---\n\nHi everyone,\n\nStarting next week, our weekly meeting moves from Monday to "
                "Thursday morning at [time].\n\nSee you there!")
DRAFT_ROW = "Hi everyone, Starting next week, our weekly meeting moves from Monday to Thursday morning at [time]."
BUY_OR_LEASE = "Should I buy the car or lease it?"
WORKED_OUT = ("I'll work this out for you...\n\nBuy: it costs $19,626 over three years against $21,000 to lease."
              "\n\n## The arithmetic\n\n- Loan: 36 payments of $676.65, with $15,267 still owed")
WORKED_ROW = "Buy: it costs $19,626 over three years against $21,000 to lease."
PLAN_CLEANUP = "Plan the cleanup day."
PLANNED = ("Book the park for Saturday the 12th, then recruit 30 volunteers in two weeks.\n\n## First step\n\n"
           "Call the parks office on Monday.")
PLANNED_ROW = "Book the park for Saturday the 12th, then recruit 30 volunteers in two weeks."
LOAN_LEFT = "How much is left on the car loan after three years?"
LOAN_SUM = ("rate = 6% / 12\npayment = 35000 * rate / (1 - (1 + rate) ^ -60)\n"
            "owed = payment * (1 - (1 + rate) ^ -24) / rate\nround(owed)")
LOAN_WORKED = "round(owed) = 15267"
LOAN_ROW = "$15,267 is still owed on the loan after three years."
FINISHED_ROWS = """(() => {
  const section = document.getElementById('for-you-finished')?.closest('section');
  if (!section) return null;
  return [...section.querySelectorAll('.finished-line')].map((row) => ({
    who: row.querySelector('.finished-line__who').innerText.trim(),
    headline: row.title,
  }));
})()"""


def scenario_headline(w: World) -> None:
    answers = threading.Event()
    notes = {("Writer", WRITE_NOTE): DRAFTED_NOTE, ("Analyst", BUY_OR_LEASE): WORKED_OUT,
             (AGENT, PLAN_CLEANUP): PLANNED}

    worked: list[str] = []

    def script(c: Call) -> Reply | None:
        for (speaker, brief), note in notes.items():
            if c.speaker == speaker and c.last_user.endswith(brief):
                return held(answers, say(note))
        if c.speaker == "Accountant" and c.last_user.endswith(LOAN_LEFT):
            if "calculator" not in c.tools:
                return say("No calculator was offered.")
            if not c.tool_results():
                return held(answers, call("calculator", expression=LOAN_SUM))
            worked.append(c.tool_results()[-1])
            return say("I'll work out the balance first.\n\n## The sums\n\n" + c.tool_results()[-1]
                       + "\n\nBottom line: " + LOAN_ROW)
        return hello(c)

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)
    ui = w.ui
    new_agent(w, "Writer", "You write notes for the team.")
    new_agent(w, "Analyst", "You work out what things cost.")
    new_agent(w, "Accountant", "You keep the loan figures.")
    for (speaker, brief) in [*notes, ("Accountant", LOAN_LEFT)]:
        write(w, speaker, brief)
        w.wait(f"the message to {speaker} is recorded", lambda brief=brief: w.host.scalar(
            "SELECT count(*) FROM messages WHERE parts LIKE ?", f"%{brief}%"))
    ui.click("button", "For you")
    w.wait("For you opens", lambda: ui.find("dialog", "For you", exact=False))
    answers.set()
    settled(w, 5, "the four tasks finish")
    rows = w.wait("For you lists the four results as new", lambda: (r := ui.evaluate(FINISHED_ROWS))
                  and len(r) == 4 and r)
    shown = {row["who"]: row["headline"] for row in rows}
    kept = said(w, "Accountant")
    w.check("an agent works a sum out with the calculator, and its Bottom line is the For you headline",
            len(worked) == 1 and LOAN_WORKED in worked[0] and shown.get("Accountant") == LOAN_ROW,
            observed={"calculator": worked, "headline": shown.get("Accountant")}, shot=True)
    w.check("the conversation keeps the Bottom line in the full result",
            len(kept) == 1 and kept[0].endswith("Bottom line: " + LOAN_ROW) and LOAN_WORKED in kept[0],
            observed=kept)
    shown.pop("Accountant", None)
    expected = {"Writer": DRAFT_ROW, "Analyst": WORKED_ROW, AGENT: PLANNED_ROW}
    w.check("a result that opens with a label or a promise is listed by the answer after it",
            shown.get("Writer") == DRAFT_ROW and shown.get("Analyst") == WORKED_ROW,
            observed=shown, shot=True)
    w.check("a result with nothing before its answer is listed by its own first line",
            shown == expected, observed=shown)
    w.check("the conversation keeps every result word for word",
            said(w, "Writer") == [DRAFTED_NOTE] and said(w, "Analyst") == [WORKED_OUT]
            and said(w, AGENT)[-1] == PLANNED,
            observed={"Writer": said(w, "Writer") == [DRAFTED_NOTE],
                      "Analyst": said(w, "Analyst") == [WORKED_OUT], AGENT: said(w, AGENT)[-1] == PLANNED})
    no_unscripted(w)


def selected_tab(w: World) -> list[str]:
    return [str((n.get("name") or {}).get("value") or "") for n in w.ui.ax_nodes()
            if (n.get("role") or {}).get("value") == "tab" and any(
                p.get("name") == "selected" and (p.get("value") or {}).get("value")
                for p in n.get("properties", []))]


def scenario_tabs(w: World) -> None:
    w.model.script = hello
    w.start()
    first_run(w)
    settled(w, 1)
    ui = w.ui

    ui.evaluate("document.activeElement && document.activeElement.blur()")
    ui.key(",", mod=True)
    w.wait("Settings opens", lambda: ui.find("region", "Settings"))
    stops = []
    for _ in range(4):
        ui.key("Tab")
        stops.append(ui.focused())
        if stops[-1][0] == "tab":
            break
    w.check("the first Tab stop in Settings is the open tab", stops[-1] == ("tab", "General"),
            observed=stops)

    ui.key("ArrowDown")
    moved = ui.focused()
    w.check("an arrow key moves to the next tab and opens it",
            moved[0] == "tab" and moved[1] != "General" and selected_tab(w) == [moved[1]],
            observed={"focused": moved, "selected": selected_tab(w)})
    ui.key("End")
    last = ui.focused()
    ui.key("Home")
    w.check("Home and End reach the first and the last tab",
            last[0] == "tab" and last[1] != "General" and ui.focused() == ("tab", "General")
            and selected_tab(w) == ["General"],
            observed={"end": last, "home": ui.focused()})

    presses = 0
    while ui.focused() != ("tab", "Appearance") and presses < 16:
        ui.key("ArrowDown")
        presses += 1
    w.check("Appearance is reached and opened with arrow keys",
            selected_tab(w) == ["Appearance"] and ui.find("heading", "Appearance"),
            observed={"selected": selected_tab(w)}, detail={"presses": presses}, shot=True)

    ui.key("Tab")
    inside = ui.focused()
    w.check("Tab goes from the tab list into the open panel",
            inside[0] not in ("tab", "") and selected_tab(w) == ["Appearance"], observed=inside)
    ui.key("Tab", shift=True)
    back = ui.focused()
    ui.key("Tab", shift=True)
    w.check("the tab list is one Tab stop",
            back == ("tab", "Appearance") and ui.focused()[0] != "tab",
            observed={"back": back, "before": ui.focused()})
    no_unscripted(w)


ROSTER_GEOMETRY = """(() => {
  const roster = document.querySelector('.roster');
  const scroller = roster.querySelector('.roster__scroll');
  const before = scroller.scrollTop;
  const range = document.createRange();
  const within = (inside, outside) =>
    inside.left >= outside.left - 1 && inside.right <= outside.right + 1 &&
    inside.top >= outside.top - 1 && inside.bottom <= outside.bottom + 1;
  const overlap = (a, b) =>
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
  const rows = [...roster.querySelectorAll('.roster__agent')];
  const measured = rows.map(row => {
    row.scrollIntoView({block: 'nearest', inline: 'nearest'});
    const name = row.querySelector('.roster__name');
    const status = row.querySelector('.roster__status');
    const shortcut = row.querySelector('kbd')?.getBoundingClientRect();
    const box = row.getBoundingClientRect();
    range.selectNodeContents(name);
    const text = range.getBoundingClientRect();
    const nameBox = name.getBoundingClientRect();
    const statusBox = status.getBoundingClientRect();
    const clipped = name.scrollWidth > name.clientWidth || name.scrollHeight > name.clientHeight;
    const visible = clipped ? nameBox : text;
    const style = getComputedStyle(name);
    const tooltip = name.title === name.textContent;
    const lines = new Set([...range.getClientRects()]
      .filter(fragment => within(fragment, nameBox)).map(fragment => Math.round(fragment.top)));
    return {
      name: name.textContent.trim(), clipped, tooltip,
      lines: lines.size,
      ownLine: nameBox.bottom <= statusBox.top + 1 && (!shortcut ||
        (nameBox.bottom <= shortcut.top + 1 && statusBox.right <= shortcut.left + 1
          && statusBox.top < shortcut.bottom && shortcut.top < statusBox.bottom
          && Math.abs(nameBox.right - shortcut.right) <= 1)),
      visible: name.checkVisibility({opacityProperty: true, visibilityProperty: true})
        && visible.width > 0 && visible.height > 0 && within(visible, nameBox)
        && within(visible, box) && within(visible, scroller.getBoundingClientRect())
        && within(visible, {left: 0, right: innerWidth, top: 0, bottom: innerHeight})
        && !overlap(visible, statusBox)
        && rows.every(other => other === row || !overlap(visible, other.getBoundingClientRect()))
        && (!clipped || (style.webkitLineClamp === '2' && style.overflow === 'hidden')),
    };
  });
  scroller.scrollTop = before;
  return measured;
})()"""


def scenario_roster(w: World) -> None:
    long_name = "Researcher for International Market Operations"
    names = (AGENT, MARKET, "Product Manager", "Researcher", "Researcher West", "Critic",
             "Archivist", "Writer", "Analyst", "Editor", long_name)
    asking = names[9]
    parked = threading.Event()
    draft = "Keep this draft while switching conversations."

    def script(c: Call) -> Reply | None:
        if c.speaker == asking and c.last_user.endswith(BOOK_FLIGHT):
            return held(parked, call("ask_operator", question=WHICH, options=["Early", "Late"]))
        return hello(c)

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)
    ui = w.ui
    for name in names[3:]:
        new_agent(w, name, "Wait to be asked.")
    open_agent(w, AGENT)
    w.wait("the roster has every agent", lambda: len(roster_rows(w)) == len(names))
    w.wait("opening the first conversation clears its result", lambda: roster_status(w, AGENT) == "Idle")
    initial = [label(row) for row in roster_rows(w)]
    matches = [roster_entry(row, name) for row, name in zip(initial, names)]
    shortcuts = ui.evaluate("""[...document.querySelectorAll('.roster__agent')].map(row => [
      row.getAttribute('aria-keyshortcuts'),
      row.querySelector('kbd')?.checkVisibility() ? row.querySelector('kbd').textContent.trim() : null
    ])""")
    w.check("each agent has one ordered row, with shortcuts only for the first nine",
            all(matches) and all(m[1] == "Idle" for m in matches[1:])
            and len(shortcuts) == len(names)
            and all(key == f"Meta+{i} Control+{i}" and shown and shown.endswith(str(i))
                    for i, (key, shown) in enumerate(shortcuts[:9], 1))
            and shortcuts[9:] == [[None, None], [None, None]],
            observed={"buttons": initial, "shortcuts": shortcuts})
    w.check("the conversation has no Today destination or spare writing field",
            not ui.find("button", "Today") and not ui.find("region", "Today")
            and not ui.find("textbox", "What do you need?"),
            observed=ui.names("navigation"))

    write(w, asking, BOOK_FLIGHT)
    w.wait("the late row shows Working", lambda: roster_status(w, asking) == "Working")
    open_agent(w, names[1])
    ui.fill("combobox", f"Write to {names[1]}", draft)
    parked.set()
    w.wait("the late row shows the pending question", lambda: roster_status(w, asking) == "Has a question")
    current = [label(row) for row in roster_rows(w)]
    w.check("a new question changes the tenth status without reordering or switching conversations",
            [row for i, row in enumerate(current) if i != 9] ==
            [row for i, row in enumerate(initial) if i != 9]
            and current[9] == f"{asking}, Has a question"
            and ui.find("log", f"Conversation with {names[1]}") and ui.find("button", "1 question waiting"),
            observed=current, shot=True)

    readable = {}
    for width, height in SETTINGS_FRAMES:
        ui.viewport(width, height)
        w.wait("roster layout settles", lambda: ui.evaluate(look.SETTLED) and ui.evaluate("document.fonts.status") == "loaded")
        readable[f"{width}x{height}"] = ui.evaluate(ROSTER_GEOMETRY)
        w.capture(f"the roster at {width}x{height}")
    w.check("desktop names wrap before they are shortened and stay above the status and shortcut",
            all([row["name"] for row in rows] == list(names)
                and all(row["visible"] and 1 <= row["lines"] <= 2 and row["ownLine"]
                        and (not row["clipped"] or row["tooltip"]) for row in rows)
                and rows[0]["lines"] == 1 and not rows[0]["clipped"]
                and rows[-1]["lines"] == 2 and rows[-1]["clipped"]
                for rows in readable.values())
            and readable["1280x820"][1]["lines"] == 1 and not readable["1280x820"][1]["clipped"]
            and readable["900x700"][1]["lines"] == 2 and not readable["900x700"][1]["clipped"],
            observed=readable)

    open_agent(w, names[9])
    w.wait("the clicked row focuses its question", lambda: ui.focused() == ("button", "Early"))
    compact = {}
    compact_geometry = {}
    for height in (355, 320):
        ui.viewport(320, height)
        w.wait(f"the app fits the {height}-pixel viewport and the compact picker is visible",
               lambda: ui.evaluate(f"document.querySelector('.app').getBoundingClientRect().height === {height}")
               and ui.find("combobox", "Switch agent") and ui.evaluate(look.SETTLED))
        compact[str(height)] = (ui.on_screen("combobox", f"Write to {asking}") and ui.on_screen("button", "Send")
                                and ui.evaluate("document.querySelector('.composer').getBoundingClientRect().bottom "
                                                "<= document.querySelector('.app').getBoundingClientRect().bottom"))
        compact_geometry[str(height)] = ui.evaluate("""Object.fromEntries(['.app', '.roster', '.pane__header', '.agent-tabs__list',
              '.pane__body', '.working', '.composer', '.composer__input', '.composer__send', '.statusbar'].map(s => {
                const el = document.querySelector(s), r = el.getBoundingClientRect();
                return [s, [r.top, r.bottom, getComputedStyle(el).paddingBlock]];
              }))""")
    w.check("the composer and Send stay visible with a question at keyboard height",
            all(compact.values()), observed=compact, detail=compact_geometry, shot=True)
    field = ("combobox", f"Write to {asking}")
    ui.fill(*field, "@")
    w.wait("the mention picker opens at keyboard height", lambda: ui.find("listbox", "Agents"))
    for _ in range(5):
        ui.key("ArrowDown")
    mention = w.wait("a keyboard-chosen mention stays within the picker and on screen", lambda: ui.evaluate("""
      (() => {
        const composer = document.querySelector('.composer');
        const list = composer.querySelector('.mentions').getBoundingClientRect();
        const active = composer.querySelector('.mentions__item[aria-selected="true"]');
        const row = active.getBoundingClientRect();
        const limit = document.querySelector('.app').getBoundingClientRect();
        if (composer.getBoundingClientRect().bottom > limit.bottom || list.top < limit.top
            || row.top < list.top || row.bottom > list.bottom || row.bottom > limit.bottom) return null;
        return active.querySelector('.mentions__name').textContent;
      })()
    """), timeout=4)
    w.capture("the selected mention stays on screen at keyboard height")
    ui.key("Enter")
    w.check("keyboard selection inserts a visible mention without sending at keyboard height",
            ui.value(*field) == f"@{mention} " and ui.focused() == field
            and w.host.scalar("SELECT count(*) FROM messages WHERE parts LIKE ?", f"%@{mention}%") == 0,
            observed=mention)
    ui.fill(*field, "")
    ui.viewport(900, 700)
    ui.key("Tab", mod=True)
    w.wait("Control Tab opens the next row", lambda: ui.find("log", f"Conversation with {names[10]}"))
    ui.key("Tab", mod=True, shift=True)
    w.wait("Control Shift Tab returns to the question owner", lambda: ui.find("log", f"Conversation with {asking}"))
    ui.key("2", mod=True)
    w.wait("the second shortcut opens the exact name", lambda: ui.find("log", f"Conversation with {names[1]}"))
    w.check("mouse, Control Tab in both directions, and the numbered shortcut keep the draft with its agent",
            ui.value("combobox", f"Write to {names[1]}") == draft, observed=ui.focused(), shot=True)

    ui.key("k", mod=True)
    w.wait("Command menu opens", lambda: ui.find("dialog", "Command menu"))
    ui.type("textbox", "Command menu", "dark")
    choice = w.wait("the menu chooses a reading surface", lambda: menu_choice(w))
    ui.key("f", mod=True)
    w.wait("Search opens above the menu", lambda: ui.find("dialog", "Search", exact=False))
    ui.key("Tab", mod=True)
    ui.key("Tab", mod=True, shift=True)
    ui.key("Tab", mod=True)
    ui.key("Escape")
    w.wait("Escape closes only Search", lambda: not ui.find("dialog", "Search", exact=False)
           and ui.find("dialog", "Command menu"))
    kept_menu = ui.value("textbox", "Command menu") == "dark" and menu_choice(w) == choice
    ui.key("Escape")
    w.wait("Escape then closes the menu", lambda: not ui.find("dialog", "Command menu"))
    w.check("stacked Search and its keys leave the command menu and underlying draft unchanged",
            kept_menu and ui.find("log", f"Conversation with {names[1]}")
            and ui.value("combobox", f"Write to {names[1]}") == draft,
            observed={"menu": kept_menu, "focus": ui.focused()}, shot=True)

    ui.evaluate("location.reload()")
    w.wait("reload opens the waiting question's conversation", lambda: ui.find("log", f"Conversation with {asking}")
           and ui.find("button", "Early"))
    open_agent(w, names[1])
    w.check("reload lands on the pending question but keeps the other conversation's draft",
            ui.value("combobox", f"Write to {names[1]}") == draft,
            observed=ui.focused(), shot=True)
    ui.send("Emulation.setUserAgentOverride", {"userAgent": ui.evaluate("navigator.userAgent"), "platform": "MacIntel"})
    ui.evaluate("location.reload()")
    w.wait("the Mac platform reload opens the waiting conversation",
           lambda: ui.find("log", f"Conversation with {asking}") and ui.find("button", "Early"))
    open_agent(w, AGENT)
    ui.key("k", mod=True)
    w.wait("the command menu opens with the platform shortcut labels",
           lambda: ui.find("dialog", "Command menu"))
    labels = ui.evaluate("""(() => {
      const first = document.querySelector('.roster__agent kbd').textContent.trim();
      const title = document.querySelector('.roster__name').textContent;
      const menu = [...document.querySelectorAll('.palette__row')].find(
        row => row.querySelector('.palette__title')?.textContent === title
      );
      return [first, [...(menu?.querySelectorAll('kbd') ?? [])].map(key => key.textContent.trim()).at(-1)];
    })()""")
    w.check("the roster and command menu both show Command 1 on macOS",
            labels == ["⌘1", "⌘1"], observed=labels)
    no_unscripted(w)


def scenario_grouptabs(w: World) -> None:
    w.model.script = hello
    w.start()
    first_run(w)
    settled(w, 1)
    ui = w.ui

    group = w.host.scalar("SELECT name FROM groups ORDER BY rowid LIMIT 1")
    show(w, "the Groups sheet")
    ui.click("button", f"{group} settings")
    w.wait("the group's settings open", lambda: ui.find("dialog", "Group settings"))
    heading, = (node for node in ui.within("dialog", "Group settings", "heading") if label(node) == group)
    ui.press(heading)
    stops = []
    for _ in range(6):
        ui.key("Tab")
        stops.append(ui.focused())
        if stops[-1][0] == "tab":
            break
    w.check("the first Tab stop in a group's settings is the open tab", stops[-1] == ("tab", "General"),
            observed=stops)

    ui.key("ArrowDown")
    moved = ui.focused()
    w.check("an arrow key moves to the next section and opens it",
            moved == ("tab", "Provider") and selected_tab(w) == ["Provider"],
            observed={"focused": moved, "selected": selected_tab(w)}, shot=True)
    ui.key("ArrowUp")
    back = ui.focused()
    w.check("the opposite arrow key moves back and opens that section",
            back == ("tab", "General") and selected_tab(w) == ["General"],
            observed={"focused": back, "selected": selected_tab(w)})
    ui.key("End")
    last = ui.focused()
    ui.key("Home")
    w.check("Home and End reach the first and the last section",
            last[0] == "tab" and last[1] != "General" and ui.focused() == ("tab", "General")
            and selected_tab(w) == ["General"],
            observed={"end": last, "home": ui.focused()})

    ui.key("Tab")
    inside = ui.focused()
    w.check("Tab goes from the section list into the open panel",
            inside[0] not in ("tab", "") and selected_tab(w) == ["General"], observed=inside)
    ui.key("Tab", shift=True)
    returned = ui.focused()
    ui.key("Tab", shift=True)
    w.check("the section list is one Tab stop",
            returned == ("tab", "General") and ui.focused()[0] != "tab",
            observed={"back": returned, "before": ui.focused()})
    no_unscripted(w)


NEEDS_KEY = "verify/needs-key"


def to_connect(w: World) -> None:
    ui = w.ui
    w.wait("the first visit offers a first task", lambda: ui.find("button", FIRST_TASK))
    ui.click("button", FIRST_TASK)
    ui.fill("textbox", "Your first brief", "Say hello.")
    ui.click("button", "Continue")
    w.wait("the Connect your AI step", lambda: ui.find("button", "Test and continue"))
    ui.click("button", API_KEY)
    ui.choose("combobox", "API provider", "Custom endpoint")
    w.wait("the endpoint fields", lambda: ui.find("textbox", "Inference endpoint"))
    ui.evaluate("document.activeElement && document.activeElement.blur()")
    ui.evaluate("document.querySelector('.onboarding').scrollTop = 0")


def alert_shown(w: World) -> bool:
    return bool(w.ui.evaluate(
        "(() => { const a = [...document.querySelectorAll('.onboarding-actions [role=alert]')].find(e => e.innerText.trim());"
        " if (!a) return false; const r = a.getBoundingClientRect();"
        " const seen = (x, y) => { const hit = document.elementFromPoint(x, y); return hit !== null && a.contains(hit); };"
        " return r.top >= 0 && r.bottom <= innerHeight && seen(r.left + 4, r.bottom - 4) && seen(r.right - 4, r.top + 4); })()"))


def group_clear(w: World, count: int = 3) -> bool:
    return bool(w.ui.evaluate(
        "(() => { const pane = document.querySelector('.onboarding'); const bar = document.querySelector('.onboarding-actions');"
        " if (!pane || !bar) return false; const top = pane.getBoundingClientRect().top, floor = bar.getBoundingClientRect().top;"
        " const rows = [...document.querySelectorAll('.onboarding-group li')];"
        f" return rows.length === {count} && rows.every(li => {{ const r = li.getBoundingClientRect();"
        " return r.top >= top && r.bottom <= floor; }); })()"))


def refusal(w: World, why: str) -> tuple[str, bool]:
    ui = w.ui
    ui.click("button", "Test and continue")
    alert = w.wait(why, lambda: [
        t for t in (ui.evaluate("Array.from(document.querySelectorAll('[role=alert]')).map(e=>e.innerText)") or [])
        if t.strip()])
    return alert[0], alert_shown(w)


def scenario_connect(w: World) -> None:

    def script(c: Call) -> Reply | None:
        if c.body.get("model") == NEEDS_KEY and c.key != STAND_IN_KEY:
            return refuse(401, "Missing or wrong API key.")
        return hello(c)

    w.model.script = script
    w.start()
    ui = w.ui
    ui.viewport(*WINDOW)
    to_connect(w)
    fields = {}
    for width, height in ((900, 700), WINDOW):
        ui.viewport(width, height)
        for name in ("API key", "Inference endpoint", "Default model"):
            fields[f"{width}x{height} {name}"] = (bool(ui.find("textbox", name, exact=False))
                                                  and ui.on_screen("textbox", name, exact=False))
    w.check("the key, endpoint and model fields are on screen without scrolling at 900x700 and 1280x820",
            all(fields.values()), observed=fields, shot=True, stop=False)

    model = {"value": ui.property("textbox", "Default model", "value"),
             "placeholder": ui.property("textbox", "Default model", "placeholder")}
    w.check("a Custom endpoint suggests no other provider's model",
            not model["value"] and "anthropic/" not in model["placeholder"], observed=model, shot=True, stop=False)

    ui.fill("textbox", "Inference endpoint", w.model.base)
    ui.fill("textbox", "Default model", NEEDS_KEY)
    ui.fill("textbox", "API key", WRONG_KEY, exact=False)
    ui.click("button", "Test and continue")
    said_ = w.wait("the endpoint refuses the wrong key, and the key field says so", lambda: ui.evaluate(
        "(() => { const a = document.querySelector('.onboarding-endpoint [role=alert]'); return a && a.innerText.trim(); })()"))
    marked = ui.evaluate(
        "(() => { const a = document.querySelector('.onboarding-endpoint [role=alert]');"
        " const key = document.querySelector('input[type=password]'); const r = a.getBoundingClientRect();"
        " const bar = document.querySelector('.onboarding-actions').getBoundingClientRect();"
        " const probe = document.createElement('span'); probe.style.color = 'var(--danger)'; a.append(probe);"
        " const danger = getComputedStyle(probe).color; probe.remove();"
        " return {onScreen: r.top >= 0 && r.bottom <= bar.top, invalid: key.getAttribute('aria-invalid') === 'true',"
        " described: key.getAttribute('aria-describedby') === a.id, danger: getComputedStyle(a).color === danger}; })()")
    plain = {"plain": said_.startswith("This endpoint did not accept that key.") and "HTTP" not in said_,
             "inBar": bool(ui.evaluate("!!document.querySelector('.onboarding-actions [role=alert]')"))}
    ui.type("textbox", "API key", "x", exact=False)
    cleared = not ui.evaluate("!!document.querySelector('.onboarding-endpoint [role=alert]')") and not ui.evaluate(
        "document.querySelector('input[type=password]').hasAttribute('aria-invalid')")
    w.check("a rejected key is said at the key field in plain words, as an error that clears on retyping", all(marked.values()) and plain["plain"] and not plain["inBar"] and cleared,
            observed={**marked, **plain, "cleared": cleared}, detail=said_, shot=True)

    ui.fill("textbox", "API key", "", exact=False)
    said_, shown = refusal(w, "the endpoint refuses a request with no key, and the form says so")
    w.check("with no key, the refusal is about this endpoint and names no other provider or place",
            shown and "127.0.0.1" in said_ and "OpenRouter" not in said_ and "Settings" not in said_,
            observed={"onScreen": shown, "namesEndpoint": "127.0.0.1" in said_,
                      "namesOpenRouter": "OpenRouter" in said_, "namesSettings": "Settings" in said_},
            detail=said_, shot=True)

    ui.fill("textbox", "Default model", STAND_IN_MODEL)
    ui.click("button", "Test and continue")
    w.wait("the endpoint test passes", lambda: ui.find("button", "Start task"))
    kept = w.host.call("get_settings")
    w.check("an endpoint that needs no key passes the test with none, and is kept",
            kept["baseUrl"] == w.model.base and not kept["apiKeySet"],
            observed={"endpointSaved": kept["baseUrl"] == w.model.base, "keySaved": kept["apiKeySet"]})
    no_unscripted(w)


QUIET_PLEASE = "Please keep the notes short."
NOTED = "Short notes it is."
RESEARCHER = "Researcher"


def menu_choice(w: World) -> str:
    return w.ui.evaluate(
        "(document.querySelector('[data-selected=\"true\"] .palette__title') || {}).textContent || ''") or ""


def scenario_menu(w: World) -> None:
    brief = "Plan the trip."
    itinerary = "The Lisbon itinerary is ready."

    def script(c: Call) -> Reply | None:
        if c.speaker == AGENT and c.last_user.endswith(QUIET_PLEASE):
            return say(NOTED)
        if c.speaker == AGENT and c.last_user.endswith(brief):
            return say(itinerary)
        return hello(c)

    w.model.script = script
    w.start()
    first_run(w, brief, itinerary)
    settled(w, 1)
    ui = w.ui

    w.check("the conversation puts the focus in the composer",
            w.wait("the composer takes the focus", lambda: ui.focused() == ("combobox", f"Write to {AGENT}")),
            observed=ui.focused())

    ui.key("k", mod=True)
    w.wait("Command K opens the command menu", lambda: ui.find("dialog", "Command menu"))
    sections = ui.evaluate("Array.from(document.querySelectorAll('.palette__section')).map(e => e.textContent)")
    ui.type("textbox", "Command menu", "dark")
    w.wait("dark selects a setting", lambda: menu_choice(w) == "Reading surface: Dark")
    w.wait("the menu has finished moving", lambda: ui.evaluate(look.SETTLED))
    w.capture("the command menu after typing dark")
    ui.key("Enter")
    w.check("one word, dark, and Return switch the reading surface to dark",
            w.wait("the page turns dark", lambda: ui.evaluate("document.documentElement.dataset.surface") == "dark")
            and not ui.find("dialog", "Command menu"),
            observed={"sections": sections, "surface": ui.evaluate("document.documentElement.dataset.surface")},
            shot=True)

    ui.key("k", mod=True)
    w.wait("the command menu opens again", lambda: ui.find("dialog", "Command menu"))
    before = len(said(w, AGENT))
    ui.type("textbox", "Command menu", QUIET_PLEASE)
    ui.key("Escape")
    w.check("Escape closes the menu with a sentence in it, sends nothing and returns to the composer",
            w.wait("the menu closes", lambda: not ui.find("dialog", "Command menu"))
            and w.wait("the composer takes the focus back", lambda: ui.focused() == ("combobox", f"Write to {AGENT}"))
            and len(said(w, AGENT)) == before and not w.host.scalar(
                "SELECT count(*) FROM messages WHERE parts LIKE ?", f"%{QUIET_PLEASE}%"),
            observed={"focused": ui.focused(), "said": len(said(w, AGENT)) - before})

    ui.key("k", mod=True)
    w.wait("the command menu opens to find a word", lambda: ui.find("dialog", "Command menu"))
    calls = len(w.model.calls)
    sent = w.host.scalar("SELECT count(*) FROM messages WHERE from_kind = 'human'")
    ui.type("textbox", "Command menu", "lisbon")
    w.wait("the message that holds the word is chosen", lambda: menu_choice(w) == f"{AGENT} → You")
    order = ui.evaluate("Array.from(document.querySelectorAll('.palette__title')).map(e => e.textContent)")
    w.capture("the command menu with a word from a message")
    ui.key("Enter")
    found = ("document.querySelector('[data-found=true]')?.innerText.includes(%s) && "
             "!document.querySelector('[role=dialog][aria-label=\"Command menu\"]')") % json.dumps(itinerary)
    w.check("one word and Return open the message that holds it and send nothing",
            w.wait("the message is highlighted and the menu closes", lambda: ui.evaluate(found))
            and len(w.model.calls) == calls
            and w.host.scalar("SELECT count(*) FROM messages WHERE from_kind = 'human'") == sent
            and order[-1] == f"Write to {AGENT}",
            observed={"order": order, "modelCalls": len(w.model.calls) - calls}, shot=True)

    ui.key("k", mod=True)
    w.wait("the command menu opens a fourth time", lambda: ui.find("dialog", "Command menu"))
    ui.type("textbox", "Command menu", QUIET_PLEASE)
    write_key = ("Array.from(document.querySelectorAll('.palette__row')).find(row => row.querySelector("
                 "'.palette__title')?.textContent === %s)?.querySelector('.palette__key')?.textContent || ''"
                 ) % json.dumps(f"Write to {AGENT}")
    w.wait("the Write row offers Command Return",
           lambda: (lambda key: key.endswith("↩") and key != "↩")(ui.evaluate(write_key)))
    choice = menu_choice(w)
    calls = len(w.model.calls)
    ui.key("Enter")
    kept = ui.find("dialog", "Command menu") and len(w.model.calls) == calls and not w.host.scalar(
        "SELECT count(*) FROM messages WHERE parts LIKE ?", f"%{QUIET_PLEASE}%")
    ui.key("Enter", mod=True)
    w.wait("the reply arrives", lambda: NOTED in said(w, AGENT))
    w.check("Return never writes a sentence, and Command Return writes it to the agent, whose reply arrives",
            kept and choice != f"Write to {AGENT}" and said(w, AGENT)[-1] == NOTED
            and not ui.find("dialog", "Command menu"),
            observed={"chosen": choice, "keptOnReturn": bool(kept), "said": said(w, AGENT)[-2:]})

    new_agent(w, "Editor", "Wait to be asked.")
    open_agent(w, "Editor")
    ui.key("1", mod=True)
    w.wait("Command 1 opens the first agent", lambda: ui.find("log", f"Conversation with {AGENT}"))
    w.check("Command 1 opens the first agent with the focus in its composer",
            w.wait("the composer takes the focus", lambda: ui.focused() == ("combobox", f"Write to {AGENT}")),
            observed=ui.focused(), shot=True)
    no_unscripted(w)


def scenario_search(w: World) -> None:
    brief = "Review the quarterly ledger."
    reply = "The ledger is ready."
    attachment = "ledger-notes.txt"

    def script(c: Call) -> Reply | None:
        if c.last_user.endswith(brief):
            return say(reply)
        if c.last_user.startswith(f"The attached file {attachment} contains:"):
            return say("Noted.")
        return hello(c)

    w.model.script = script
    w.start()
    first_run(w, brief, reply)
    settled(w, 1)
    ui = w.ui
    agent_count = w.host.scalar("SELECT count(*) FROM agents")

    ui.key("f", mod=True)
    w.wait("Find opens", lambda: ui.find("dialog", "Search"))
    ui.fill("textbox", "Search Guaca", "new agent")
    w.wait("Find settles without an action", lambda: "Nothing matching" in ui.text()
           or ui.find("button", "New agent", exact=False))
    ui.key("Enter")
    tabs = [label(one) for one in reversed(ui.within("dialog", "Search", "tab"))]
    w.check("Find only offers messages and files and cannot run New agent",
            tabs == ["All", "Messages", "Files"] and not ui.find("dialog", "New agent")
            and w.host.scalar("SELECT count(*) FROM agents") == agent_count, observed=tabs, shot=True)
    dismiss(w)

    ui.evaluate("""(() => {
      const dataTransfer = new DataTransfer();
      dataTransfer.items.add(new File(["Follow up on the ledger."], "ledger-notes.txt", {type: "text/plain"}));
      window.dispatchEvent(new DragEvent("drop", {dataTransfer, bubbles: true, cancelable: true}));
    })()""")
    w.wait("the file is staged", lambda: ui.find("button", f"Remove {attachment}"))
    ui.fill("combobox", f"Write to {AGENT}", "Keep these notes.")
    ui.click("button", "Send")
    settled(w, 2)
    ui.key("f", mod=True)
    w.wait("Find opens again", lambda: ui.find("dialog", "Search"))
    ui.click("tab", "Files")
    ui.fill("textbox", "Search Guaca", attachment)
    w.wait("Find locates the attached file", lambda: any(
        label(one).startswith(attachment) for one in ui.within("dialog", "Search", "button")))
    ui.key("Enter")
    w.check("Find opens the file's message without running an app action",
            w.wait("Find closes on the file's conversation",
                   lambda: ui.evaluate(
                       "!document.querySelector('[role=dialog][aria-label=\"Search\"]') && "
                       "!!document.querySelector('[data-found=true]')?.innerText.includes('Keep these notes.')")),
            observed={"file": attachment, "conversation": AGENT}, shot=True)

    ui.click("button", f"Open {attachment}")
    w.wait("the file reader opens", lambda: ui.find("dialog", attachment))
    w.wait("the reader has finished moving", lambda: ui.evaluate(look.SETTLED))
    reader = ui.evaluate("""(() => {
      const dialog = document.querySelector('[aria-label="ledger-notes.txt"][role=dialog]');
      const close = dialog.querySelector('button[aria-label=Close]');
      const box = close.getBoundingClientRect();
      return {
        backdrop: document.elementFromPoint(4, 4)?.closest('.scrim') === dialog.parentElement,
        close: document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2)?.closest('button') === close,
      };
    })()""")
    ui.key("f", mod=True)
    w.wait("Find opens above the reader", lambda: ui.find("dialog", "Search"))
    w.wait("Find has finished moving", lambda: ui.evaluate(look.SETTLED))
    above = ui.evaluate("""(() => {
      const input = document.querySelector('input[aria-label="Search Guaca"]');
      const box = input.getBoundingClientRect();
      return document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2) === input;
    })()""")
    ui.key("Escape")
    w.wait("Escape closes only Find", lambda: not ui.find("dialog", "Search") and ui.find("dialog", attachment))
    w.check("the file reader covers the window without clipping and Find stacks above it",
            reader == {"backdrop": True, "close": True} and above,
            observed={**reader, "findOnTop": above}, shot=True)
    dismiss(w)

    show(w, "the Groups sheet")
    ui.click("button", "Command menu", exact=False)
    w.wait("the Groups sheet opens the command menu", lambda: ui.focused() == ("textbox", "Command menu"))
    ui.fill("textbox", "Command menu", "ledger is ready")
    w.wait("the command menu finds the message itself", lambda: ui.find("button", reply, exact=False))
    w.capture("the command menu with a matching message")
    ui.click("button", reply, exact=False)
    landing = """(() => {
      const dialogs = [...document.querySelectorAll('[role=dialog]')].map(el => el.getAttribute('aria-label'));
      const highlight = [...document.querySelectorAll('[data-found=true]')].some(
        el => el.innerText.includes('The ledger is ready.'));
      return {menuOpen: dialogs.includes('Command menu'), findOpen: dialogs.includes('Search'),
        groupsOpen: dialogs.includes('Groups and agents'), highlight};
    })()"""
    expected = {"menuOpen": False, "findOpen": False, "groupsOpen": False, "highlight": True}
    w.check("the Groups sheet uses the command menu and a message hit opens without another palette",
            w.wait("the message is highlighted and the search surfaces close",
                   lambda: ui.evaluate(landing) == expected), observed=expected)
    dismiss(w)

    geometry = {}
    descriptions = {}
    for name in ("Search", "the command menu", "New agent", "the Groups sheet", "Artifacts", "Calendar", "For you"):
        if name in ("Artifacts", "Calendar", "For you"):
            ui.click("button", name, exact=False)
            w.wait(f"{name} opens", lambda: ui.find("dialog", name))
        else:
            show(w, name)
        w.wait(f"{name} has finished moving", lambda: ui.evaluate(look.SETTLED))
        geometry[name] = ui.evaluate("""(() => {
          const dialog = [...document.querySelectorAll('.scrim > [role=dialog]')].at(-1);
          const box = dialog.getBoundingClientRect();
          const color = getComputedStyle(dialog.parentElement).backgroundColor;
          return {top: Math.round(box.top), width: Math.round(box.width), color};
        })()""")
        if name == "Calendar":
            w.wait("the empty calendar is shown", lambda: "Nothing on this calendar" in ui.text())
            descriptions[name] = ui.text().count("Guaca's own calendar")
        if name == "Artifacts":
            w.wait("the empty artifacts list is shown", lambda: "keeps any artifacts yet" in ui.text())
            descriptions[name] = ui.text().count("Ask an agent for a page")
            descriptions["Artifacts footer"] = ui.text().count("Pages your agents keep.")
        dismiss(w)
    w.check("dialogs and sheets share one top offset, two widths, and a lighter light scrim",
            len({box["top"] for box in geometry.values()}) == 1
            and len({box["width"] for box in geometry.values()}) == 2
            and all(box["color"] == "rgba(11, 11, 10, 0.25)" for box in geometry.values()),
            observed=geometry)
    w.check("Calendar and Artifacts explain their empty lists without repeating the description",
            descriptions == {"Calendar": 1, "Artifacts": 1, "Artifacts footer": 0},
            observed=descriptions)
    no_unscripted(w)


def selected_card(w: World) -> str:
    return w.ui.evaluate(
        "(document.querySelector('.desk__card[data-selected]') || {}).innerText || ''") or ""


def scenario_queue(w: World) -> None:

    def script(c: Call) -> Reply | None:
        if c.speaker == AGENT and c.last_user.endswith(CALL_ME):
            return ask_name(c, "Robin")
        if c.speaker == RESEARCHER and c.last_user.endswith(BOOK_FLIGHT):
            results = c.tool_results()
            if not results:
                return call("ask_operator", question=WHICH, options=["Early", "Late"])
            return say(f"Booked: {results[-1][:80]}")
        return hello(c)

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)
    ui = w.ui
    new_agent(w, RESEARCHER, "Find sources and say where each came from.")
    write(w, AGENT, CALL_ME)
    w.wait("the permission request is pending", lambda: approvals(w) == [{"action": "changeSettings", "state": "pending"}])
    ui.key("k", mod=True)
    w.wait("the command menu opens", lambda: ui.find("dialog", "Command menu"))
    ui.type("textbox", "Command menu", RESEARCHER)
    w.wait("the name selects the agent", lambda: menu_choice(w) == RESEARCHER)
    ui.key("Enter")
    w.wait(f"{RESEARCHER}'s conversation opens", lambda: ui.find("log", f"Conversation with {RESEARCHER}"))
    write(w, RESEARCHER, BOOK_FLIGHT)
    runs = w.wait("both requests are pending", lambda: rows if len(rows := w.host.rows(
        "SELECT run_id FROM approvals WHERE state = 'pending' ORDER BY created_at, rowid")) == 2 else None)
    ui.evaluate("location.reload()")
    w.wait("reload opens the soonest-due request's conversation",
           lambda: ui.find("log", f"Conversation with {AGENT}") and ui.find("button", "Allow"))
    w.check("reload opens the earliest waiting conversation without changing either run",
            runs == w.host.rows("SELECT run_id FROM approvals WHERE state = 'pending' ORDER BY created_at, rowid"),
            observed={"originalRuns": len(runs)}, shot=True)

    open_agent(w, RESEARCHER)
    composer = ("combobox", f"Write to {RESEARCHER}")
    draft = "Ask about a seat after the flight is booked."
    ui.click(*composer)
    ui.key("Escape")
    w.wait("Escape from the empty composer opens Questions", lambda: ui.find("dialog", "Questions"))
    ui.key("Escape")
    w.wait("Escape closes Questions", lambda: not ui.find("dialog", "Questions"))
    ui.fill(*composer, draft)
    questions(w, 2)
    ui.key("Escape")
    w.wait("Escape closes Questions", lambda: not ui.find("dialog", "Questions"))
    w.check("Escape closes Questions without answering or switching conversations",
            ui.find("log", f"Conversation with {RESEARCHER}") and ui.value(*composer) == draft
            and [a["state"] for a in approvals(w)] == ["pending", "pending"],
            observed=approvals(w))

    questions(w, 2)
    first = w.wait("Questions selects and focuses the first card",
                   lambda: ui.focused()[0] == "article" and ui.focused()[1].startswith(AGENT) and selected_card(w))
    other_keys = ui.evaluate("document.querySelectorAll('.desk__card:not([data-selected]) .desk__key').length")
    w.check("Questions selects the first request and shows the keys on its buttons",
            ui.focused()[0] == "article" and ui.focused()[1].startswith(AGENT) and "Allow" in first
            and ("⌘↩" in first or "Ctrl+↩" in first),
            observed={"focused": ui.focused(), "card": first}, shot=True)

    ui.key("j")
    second = w.wait("J selects the next card", lambda: RESEARCHER in ui.focused()[1] and selected_card(w))
    ui.key("k")
    w.check("J and K move the selection, and an option shows its number only while selected",
            other_keys == 0 and "Early1" in re.sub(r"\s+", "", second)
            and w.wait("K goes back", lambda: ui.focused()[1].startswith(AGENT)),
            observed={"second": second, "focused": ui.focused()})

    for _ in range(8):
        ui.key("Tab")
        if ui.focused() == ("button", "Early"):
            break
    w.check("Tab brings the answer and its selected card into agreement",
            ui.focused() == ("button", "Early") and "Early" in selected_card(w))
    ui.key("Enter", mod=True)
    w.check("Command Return cannot authorize an earlier permission from a later question",
            not ui.find("button", "Undo", exact=False)
            and [a["state"] for a in approvals(w)] == ["pending", "pending"])
    ui.key("k")

    ui.key("Enter", mod=True)
    w.wait("the Allow is held with an Undo and the next card takes focus",
           lambda: ui.find("button", "Undo", exact=False) and RESEARCHER in ui.focused()[1])
    w.check("Command Return holds the Allow with an Undo, sends nothing yet and selects the next request",
            [a["state"] for a in approvals(w)] == ["pending", "pending"] and RESEARCHER in ui.focused()[1],
            observed={"approvals": approvals(w), "focused": ui.focused()}, shot=True)
    ui.key("z", mod=True)
    w.check("Command Z takes the Allow back: the card returns and nothing was sent",
            w.wait("the card returns", lambda: any(label(n) == "Allow" for n in ui.within("dialog", "Questions", "button"))
                   and not ui.find("button", "Undo", exact=False))
            and [a["state"] for a in approvals(w)] == ["pending", "pending"] and operator_name(w) == "",
            observed={"approvals": approvals(w), "operatorName": operator_name(w)})

    ui.key("k")
    w.wait("the first card is selected again", lambda: ui.focused()[1].startswith(AGENT))
    ui.key("Enter", mod=True)
    w.wait("the selection moves to the question", lambda: RESEARCHER in ui.focused()[1])
    ui.key("1")
    w.wait("both answers are sent after the Undo has passed", lambda: sorted(
        a["state"] for a in approvals(w)) == ["allow", "answered"] and operator_name(w) == "Robin")
    w.check("the held answers are each sent once, and the agents are told",
            operator_name(w) == "Robin"
            and w.wait(f"{RESEARCHER} replies with the answer", lambda: "Early" in " ".join(said(w, RESEARCHER))),
            observed={"approvals": approvals(w), "said": said(w, RESEARCHER)[-1:]})
    w.check("after the last answer Questions closes and the original composer gets its draft and focus back",
            w.wait("the original composer takes focus back",
                   lambda: not ui.find("dialog", "Questions") and ui.focused() == composer)
            and ui.value(*composer) == draft, observed=ui.focused(), shot=True)
    settled(w, 3)
    w.check("both answers finish their original runs",
            {r["run_id"] for r in runs} == {r["run_id"] for r in w.host.rows(
                "SELECT run_id FROM outcomes WHERE run_id IN (?, ?)", *(r["run_id"] for r in runs))},
            observed={"runs": len(runs)})
    ui.evaluate("location.reload()")
    w.wait("without running or waiting work, reload opens the last conversation",
           lambda: ui.find("log", f"Conversation with {RESEARCHER}") and ui.find(*composer))
    w.check("after both runs settle, reload returns to the last conversation and its draft",
            ui.value(*composer) == draft, observed=ui.focused(), shot=True)
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


GROUP_TASK = "Research a decision Compare options and make the tradeoffs clear."
LEAD, MARKET, FINANCE = "Chief of Staff", "Market Researcher", "Financial Analyst"
CAR_Q = "Buy or lease the next car: what do your sources and the numbers say?"
RESEARCHER_SAYS = "Sources lean to leasing under 12,000 miles a year; buying wins past five years."
ANALYST_SAYS = "Over three years leasing costs about 2,100 dollars less; past year four buying is cheaper."
CAR_RESULT = ("Lease, if you replace the car within three years. Market Researcher and Financial Analyst "
              "agree on the three-year cost and disagree on resale, which matters only past year four.")
SOLO_RESULT = ("Leasing costs about 2,100 dollars less over three years, assuming a 32,000 dollar car and "
               "12,000 miles a year, so lease if you replace the car within three years. Buy if you keep it past year four.")
DEEPER = "Take a deeper pass at what I asked Financial Analyst"


def scenario_firstgroup(w: World) -> None:
    solo = threading.Event()
    researcher = threading.Event()
    analyst = threading.Event()
    followup = threading.Event()

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        if c.speaker == LEAD:
            if c.last_user.endswith("Check the mileage assumption."):
                if not c.tool_results():
                    return call("note_progress", note="Checking the mileage assumption")
                return held(followup, say("The estimate assumes twelve thousand miles a year."))
            if ANALYST_SAYS in c.last_user:
                return say(CAR_RESULT)
            if RESEARCHER_SAYS in c.last_user:
                return say("Market Researcher answered.")
            if DEEPER in c.last_user:
                return say("I asked them both.") if c.tool_results() else call(
                    "send_message", to=[MARKET, FINANCE], text=CAR_Q, intent="work")
        if c.speaker == MARKET and CAR_Q in c.last_user:
            return held(researcher, say(RESEARCHER_SAYS))
        if c.speaker == FINANCE and CAR_Q in c.last_user:
            return held(analyst, say(ANALYST_SAYS))
        if c.speaker == FINANCE and c.last_user.endswith("Ask me a question only if you cannot go on without one.") \
                and "buy or lease my next car" in c.last_user and DEEPER not in c.last_user:
            return held(solo, say(SOLO_RESULT))
        return None

    w.model.script = script
    w.start()
    ui = w.ui
    ui.viewport(*WINDOW)
    w.wait("the first visit offers a first task", lambda: ui.find("button", GROUP_TASK))
    w.check("the first step says one agent answers first, colleagues can follow, and keeps Create one agent",
            "hires the agent suited to it" in ui.text() and "bring in two colleagues" in ui.text()
            and bool(ui.find("button", "Create one agent")),
            observed={"one": "hires the agent suited to it" in ui.text(),
                      "later": "bring in two colleagues" in ui.text(),
                      "alone": bool(ui.find("button", "Create one agent"))}, shot=True)
    w.check("the first run shows its steps alone, with no window buttons, sidebar or places, and focus on the first task",
            ui.names("navigation") == [] and ui.focused() == ("button", GROUP_TASK),
            observed={"navigation": ui.names("navigation"), "focus": list(ui.focused())})
    ui.click("button", GROUP_TASK)
    brief = ui.value("textbox", "Your first brief")
    w.check("the suggested brief is for one agent to finish alone, names no colleague, and does not open by asking the person",
            "buy or lease my next car" in brief and not any(name in brief for name in (LEAD, MARKET, FINANCE))
            and not brief.lower().startswith("ask me") and brief.lower().count("ask") == 1,
            observed=brief)
    task_seen = {}
    for width, height in ((900, 700), WINDOW):
        ui.viewport(width, height)
        task_seen[f"{width}x{height}"] = (ui.on_screen("textbox", "Your first brief")
                                          and ui.on_screen("button", "Continue"))
    w.check("the first step shows the brief and Continue without scrolling at 900x700 and 1280x820",
            all(task_seen.values()), observed=task_seen)
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
    listed = ui.evaluate("[...document.querySelectorAll('.onboarding-group li')].map(li => li.innerText.replace(/\\s+/g, ' ').trim())")
    later = ui.evaluate("document.querySelector('.onboarding-later')?.innerText.trim()")
    w.check("the Start step names the one agent it will hire, what it is for, and who can join later, before any is hired",
            bool(ui.find("region", "Your agent for this task")) and [row.split(" ")[0] for row in listed]
            == ["Financial"] and all(len(row.split(" ")) > 4 for row in listed)
            and later == f"Once its result is in, you can bring in {LEAD} and {MARKET} for a deeper pass."
            and not w.host.scalar("SELECT count(*) FROM agents"),
            observed={"listed": listed, "later": later}, shot=True)
    clear = {}
    for width, height in ((900, 700), WINDOW):
        ui.viewport(width, height)
        clear[f"{width}x{height}"] = group_clear(w, 1)
    w.check("the agent and what it is for clear the pinned bar at 900x700 and 1280x820",
            all(clear.values()), observed=clear)
    ready = {"focus": list(ui.focused())}
    for width, height in ((900, 700), WINDOW):
        ui.viewport(width, height)
        ready[f"{width}x{height}"] = ui.on_screen("textbox", "Your first brief")
    w.check("the Start step shows the brief it will send without scrolling, with focus on Start task",
            ready["focus"] == ["button", "Start task"] and ready["900x700"] and ready["1280x820"], observed=ready)

    w.host.stop()
    ui.click("button", "Start task")
    alert = w.wait("a Start that cannot reach the Host says so", lambda: [
        t for t in (ui.evaluate("Array.from(document.querySelectorAll('[role=alert]')).map(e=>e.innerText)") or [])
        if t.strip()])
    w.check("a Start that fails hires nobody and sends nothing, and keeps the brief",
            not w.host.scalar("SELECT count(*) FROM agents") and not w.host.scalar("SELECT count(*) FROM messages")
            and "buy or lease" in ui.value("textbox", "Your first brief"),
            observed={"agents": w.host.scalar("SELECT count(*) FROM agents"),
                      "messages": w.host.scalar("SELECT count(*) FROM messages")}, detail=alert, shot=True)
    shown = {}
    for width, height in ((900, 700), WINDOW):
        ui.viewport(width, height)
        shown[f"{width}x{height}"] = {"alert": alert_shown(w), "group": group_clear(w, 1)}
    w.check("a Start that fails says why beside Start, on screen without scrolling at 900x700 and 1280x820",
            all(seen["alert"] and seen["group"] for seen in shown.values()), observed=shown, shot=True)
    w.host.start()
    w.wait("the page is back with the Host", lambda: admitted(ui))
    w.wait("Start task is offered again", lambda: ui.find("button", "Start task")
           and not ui.property("button", "Start task", "disabled"))
    ui.click("button", "Start task")
    w.wait("Start task opens the agent's conversation", lambda: ui.find("log", f"Conversation with {FINANCE}"))
    hired = sorted(r["name"] for r in w.host.rows("SELECT name FROM agents"))
    w.check("Start hires the one agent exactly once", hired == [FINANCE], observed=hired)
    groups = sorted(r["name"] for r in w.host.rows("SELECT name FROM groups"))
    w.wait("the agent's page names its group", lambda: "Research" in (ui.evaluate(
        "document.querySelector('.pane__about')?.innerText") or ""))
    w.check("the starter group is named for the task it was hired for, on the agent's page",
            groups == ["Research"], observed={"groups": groups,
                                             "about": ui.evaluate("document.querySelector('.pane__about')?.innerText")})
    w.wait("the brief's model call is in the air", lambda: any(c["speaker"] == FINANCE for c in w.model.calls))
    w.wait("the roster marks the agent Working", lambda: roster_status(w, FINANCE) == "Working")
    w.check("while it works, the conversation offers no crew yet",
            not ui.find("region", "A deeper pass"), observed=roster_status(w, FINANCE))
    solo.set()
    w.wait("the agent's reply is shown", lambda: SOLO_RESULT in conversation(w))
    first = settled(w, 1, "the first task settles")
    brief_calls = [c for c in w.model.calls if c.get("speaker")]
    w.check("the first result comes from one agent in one model call, with no message to or from a colleague",
            said(w, FINANCE) == [SOLO_RESULT] and first[-1]["status"] == "done" and first[-1]["agent"] == FINANCE
            and between(w) == [] and len(brief_calls) == 1
            and w.host.scalar("SELECT count(*) FROM agents") == 1,
            observed={"outcome": first[-1], "agentMessages": len(between(w)), "modelCalls": len(brief_calls)},
            detail={"calls": [{"speaker": c.get("speaker"), "asked": c.get("asked", "")[:80]} for c in brief_calls]})
    ui.click("button", "For you")
    w.wait("For you lists the first result", lambda: ui.evaluate(finished_result("Leasing")))
    ui.evaluate("document.querySelector('.for-you .finished-line[title^=\"Leasing\"]').closest('details')?.setAttribute('open', '')")
    row = w.wait("the first result's row is shown", lambda: (r := ui.evaluate(finished_result("Leasing"))) and r["byline"] and r)
    w.check("For you shows the first result under the one agent's name, with nobody else credited",
            row["byline"] == FINANCE and SOLO_RESULT.startswith(row["headline"].removesuffix("…")),
            observed=row, shot=True)
    ui.click("button", row["headline"], exact=False)
    w.wait("the first result is drawn as a result", lambda: ui.find("article", f"Result from {FINANCE}", exact=False))
    offer = w.wait("the crew offer sits under the result", lambda: ui.evaluate(CREW_OFFER))
    w.check("under the first result, one quiet line offers the crew, naming who would join, with no dialog",
            offer["after"] and LEAD in offer["text"] and MARKET in offer["text"] and offer["dialogs"] == 0
            and offer["buttons"] == ["Bring them in", "Not now"] and offer["inLog"],
            observed=offer, shot=True)
    w.check("the crew offer adds no question, notification or For you entry of its own",
            len(outcomes(w)) == 1 and not w.host.scalar("SELECT count(*) FROM decisions")
            and not ui.find("button", "1 question waiting"),
            observed={"outcomes": len(outcomes(w)), "decisions": w.host.scalar("SELECT count(*) FROM decisions")})

    ui.click("button", "Bring them in")
    w.wait("Bring them in opens the coordinator's conversation", lambda: ui.find("log", f"Conversation with {LEAD}"))
    hired = sorted(r["name"] for r in w.host.rows("SELECT name FROM agents"))
    w.check("one action hires the coordinator and the colleague into the same group, once",
            hired == sorted([LEAD, MARKET, FINANCE])
            and w.host.scalar("SELECT count(DISTINCT group_id) FROM agents") == 1,
            observed=hired)
    sent = w.host.rows("SELECT m.parts FROM messages m JOIN agents a ON a.id = m.to_agent "
                       "WHERE a.name = ? AND m.from_kind = 'human'", LEAD)
    deeper = " ".join(p["text"] for r in sent for p in json.loads(r["parts"]) if p.get("type") == "text")
    w.check("the deeper pass carries the person's first brief to the coordinator and names both colleagues",
            len(sent) == 1 and deeper.startswith(DEEPER) and "buy or lease my next car" in deeper
            and f"Ask {FINANCE} and {MARKET} at the same time" in deeper,
            observed=deeper[:200])
    w.wait("the coordinator's hand-off is live", lambda: CAR_Q in card_text(w))
    fit = ui.evaluate(TASK_MAP_FIT)
    talk = between(w)
    w.check("the coordinator asks both colleagues on its first turn, in one message, and the person sees the card",
            [(m["from"], m["to"], m["text"]) for m in talk]
            == [(LEAD, MARKET, CAR_Q), (LEAD, FINANCE, CAR_Q)]
            and fit["partial"] == 0 and len(fit["names"]) + (1 if fit["more"] == "Show 1 more colleague" else 0) == 2
            and bool(ui.find("region", "Tasks in progress")),
            observed={"talk": [(m["from"], m["to"]) for m in talk], "shown": fit["names"]}, shot=True)
    w.wait("the status line says who it waits for", lambda: "waiting for" in working_line(w))
    w.check("under the coordinator a status line names both colleagues it waits for",
            working_line(w).startswith(f"Messaged {MARKET} and {FINANCE}, waiting for"),
            observed=re.sub(r"\d+s", "Ns", working_line(w)))
    w.wait("the roster marks the coordinator Working", lambda: roster_status(w, LEAD) == "Working")
    w.check("beside the conversation, the roster shows the coordinator working",
            bool(ui.find("log", f"Conversation with {LEAD}")), observed=roster_status(w, LEAD), shot=True)

    w.wait("the roster shows all three agents working",
           lambda: all(roster_status(w, name) == "Working" for name in (LEAD, MARKET, FINANCE)))
    researcher.set()
    w.wait("Market Researcher has answered", lambda: [m["from"] for m in between(w)].count(MARKET) == 1)
    open_agent(w, MARKET)
    analyst.set()
    w.wait("the roster shows the coordinator has a result", lambda: roster_status(w, LEAD) == "Has a result")
    ui.click("button", "For you")
    row = w.wait("For you lists the result", lambda: ui.evaluate(finished_result("Lease,")))
    kept = row["headline"].removesuffix("…")
    w.check("For you shows who produced the result and who helped, with a preview cut on a boundary",
            row["byline"] == f"{LEAD}, with {MARKET} and {FINANCE}:"
            and CAR_RESULT.startswith(kept) and (kept == CAR_RESULT or CAR_RESULT[len(kept)] == " "),
            observed=row, shot=True)
    ui.click("button", row["headline"], exact=False)
    w.wait("the result reaches the person", lambda: CAR_RESULT in conversation(w))
    ended = settled(w, 2, "the group's conversation settles")
    w.check("the coordinator comes back with one result after both answered, and the run ends there",
            said(w, LEAD)[-1] == CAR_RESULT and ended[-1]["status"] == "done"
            and [m["from"] for m in between(w)].count(MARKET) == 1
            and [m["from"] for m in between(w)].count(FINANCE) == 1,
            observed={"outcome": ended[-1]["status"], "colleagues": sorted(m["from"] for m in between(w)
                                                                             if m["from"] != LEAD)},
            detail={"modelCalls": len(w.model.calls), "said": said(w, LEAD)}, shot=True)
    w.wait("the result is drawn with its task map", lambda: ui.find("article", f"Result from {LEAD}", exact=False)
           and ui.find("button", "Hand-off complete", exact=False))
    frame = ui.evaluate(RESULT_FRAME)
    w.check("the result is a raised card under the coordinator's name, and the brief before it is folded below it in weight",
            frame["name"] == LEAD and frame["raised"] and frame["text"] == CAR_RESULT
            and not frame["fillerRaised"] and frame["briefHeight"] < frame["resultHeight"],
            observed={"name": frame["name"], "raised": frame["raised"], "fillerRaised": frame["fillerRaised"],
                      "briefShorter": frame["briefHeight"] < frame["resultHeight"]}, detail=frame, shot=True)
    w.check("the hand-off settles into a record right above the result that says both answered",
            "2 of 2 answered" in frame["record"] and frame["recordAbove"],
            observed={"record": frame["record"], "above": frame["recordAbove"]})
    ui.click("button", "Hand-off complete", exact=False)
    w.wait("the record opens onto both answers", lambda: ANALYST_SAYS in card_text(w))
    w.check("the record opens onto both answers",
            RESEARCHER_SAYS in card_text(w) and ANALYST_SAYS in card_text(w),
            observed={"researcher": RESEARCHER_SAYS in card_text(w), "analyst": ANALYST_SAYS in card_text(w)}, shot=True)
    ui.press(next(button for button in ui.within("region", "Task map", "button") if label(button) == MARKET))
    w.wait("the colleague thread opens", lambda: ui.find("log", f"Conversation with {MARKET}"))
    open_agent(w, LEAD)
    w.wait("the current agent row restores the conversation and result",
           lambda: ui.find("article", f"Result from {LEAD}", exact=False))
    ui.press(next(button for button in ui.within("region", "Task map", "button") if label(button) == MARKET))
    w.wait("the colleague thread is open again", lambda: ui.find("log", f"Conversation with {MARKET}"))
    ui.click("button", "For you")
    w.wait("Earlier shows the already opened result",
           lambda: ui.evaluate("document.querySelector('.for-you .finished-line[title^=\"Lease,\"]') !== null"))
    ui.evaluate("document.querySelector('.for-you .finished-line[title^=\"Lease,\"]').closest('details').open = true")
    ui.click("button", row["headline"], exact=False)
    w.wait("opening a result leaves the colleague thread and highlights the result",
           lambda: ui.evaluate("!!document.querySelector('[role=log] [data-found] .msg--result')"))
    w.check("the roster and For you can return from a colleague thread to the current agent",
            bool(ui.find("log", f"Conversation with {LEAD}")))
    w.wait("the opened result's highlight fades, so the log stops centering on it",
           lambda: ui.evaluate("!document.querySelector('[role=log] [data-found]')"))
    folded = ui.evaluate(BRIEF_HEIGHT)
    ui.click("button", "Show all")
    w.wait("the brief opens", lambda: ui.find("button", "Show less"))
    opened = ui.evaluate(BRIEF_HEIGHT)
    w.check("the person's long brief is folded to three lines and opens whole on request",
            folded["lines"] <= 3 and opened["lines"] > 3 and opened["text"].startswith(DEEPER),
            observed={"folded": folded["lines"] <= 3, "opens": opened["lines"] > 3}, detail={"folded": folded, "opened": opened})
    write(w, LEAD, "Check the mileage assumption.")
    w.wait("the follow-up model call starts", lambda: asked(w, "Check the mileage assumption.") == 1)
    w.wait("the follow-up note appears", lambda: "Checking the mileage assumption" in working_line(w))
    w.check("the roster shows Working while the earlier result stays in the conversation",
            roster_status(w, LEAD) == "Working" and bool(ui.find("article", f"Result from {LEAD}", exact=False)),
            observed=roster_status(w, LEAD), shot=True)
    followup.set()
    settled(w, 3, "the follow-up settles")
    no_unscripted(w)


def first_task_alone(w: World, task: str, agent: str, starts: str, result: str, crew: tuple[str, str]) -> None:
    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        if c.speaker == agent and starts in c.last_user:
            return say(result)
        return None

    w.model.script = script
    w.start()
    ui = w.ui
    ui.viewport(*WINDOW)
    w.wait("the first visit offers a first task", lambda: ui.find("button", task))
    ui.click("button", task)
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
    w.wait("Start task opens the agent's conversation", lambda: ui.find("log", f"Conversation with {agent}"))
    w.wait("the agent's reply is shown", lambda: result in conversation(w))
    ended = settled(w, 1, "the first task settles")
    calls = [c for c in w.model.calls if c["speaker"]]
    hired = sorted(r["name"] for r in w.host.rows("SELECT name FROM agents"))
    w.check("the first result comes from one agent in one model call, with no message to or from a colleague",
            hired == [agent] and said(w, agent) == [result] and ended[-1]["status"] == "done"
            and between(w) == [] and len(calls) == 1,
            observed={"hired": hired, "agentMessages": len(between(w)), "modelCalls": len(calls),
                      "outcome": ended[-1]}, shot=True)
    offer = w.wait("the crew offer sits under the result", lambda: ui.evaluate(CREW_OFFER))
    w.check("under the first result, one quiet line offers the crew, naming who would join, with no dialog",
            offer["after"] and all(name in offer["text"] for name in crew) and offer["dialogs"] == 0
            and offer["buttons"] == ["Bring them in", "Not now"],
            observed=offer)
    ui.click("button", "Not now")
    w.wait("Not now takes the offer away", lambda: not ui.find("region", "A deeper pass"))
    ui.evaluate("location.reload()")
    w.wait("the page is back after a reload", lambda: ui.find("log", f"Conversation with {agent}"))
    w.wait("the result is drawn again", lambda: ui.find("article", f"Result from {agent}", exact=False))
    w.check("Not now hires nobody, and the offer stays gone after a reload",
            not ui.find("region", "A deeper pass")
            and sorted(r["name"] for r in w.host.rows("SELECT name FROM agents")) == [agent],
            observed=sorted(r["name"] for r in w.host.rows("SELECT name FROM agents")))
    no_unscripted(w)


def scenario_firstplan(w: World) -> None:
    first_task_alone(w, "Plan a project Turn an idea into a manageable first step.", "Product Manager",
                     "neighborhood cleanup day", "Book the park, then recruit 30 volunteers in two weeks.",
                     ("Chief of Staff", "QA Tester"))


def scenario_firstwrite(w: World) -> None:
    first_task_alone(w, "Draft content Find the words for something you need to say.", "Content Marketer",
                     "weekly meeting moves", "Hi all, from next week our weekly meeting moves to Thursday morning.",
                     ("Chief of Staff", "Technical Writer"))


def picked_first_task(w: World, task: str, agent: str) -> None:
    w.start()
    ui = w.ui
    ui.viewport(*WINDOW)
    w.wait("the first visit offers a first task", lambda: ui.find("button", task))
    ui.click("button", task)
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
    w.wait("Start task opens the agent's conversation", lambda: ui.find("log", f"Conversation with {agent}"))


def first_row(w: World) -> dict:
    ui = w.ui
    ui.click("button", "For you")
    return w.wait("For you lists the first result", lambda: ui.evaluate(FIRST_ROW_SHOWN))


FIRST_ROW_SHOWN = """(() => {
  const row = document.querySelector('.for-you .finished-line');
  return row ? { headline: row.title, who: row.querySelector('.finished-line__who')?.innerText.trim() } : null;
})()"""
WEEKDAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]
WRONG_DAY_SENT = "Weekdays that are wrong"
FIXED_PLAN = "Bottom line: hold the cleanup on {day}, and book the park today."
UNCHECKED_SENT = "neither in what you were given nor in anything `calculator` returned"
SUMS = "buy = 35000 * 0.06\nlease = 500 * 36 + 3000"
GUESSED = "Lease costs $21,000 and buying saves $107.\n\nBottom line: buy, it saves $107."
CHECKED = "Lease costs $21,000 against $2,100 of interest.\n\nBottom line: buy, on these sums."


def scenario_weekdayfix(w: World) -> None:
    import datetime
    date = datetime.date.today() + datetime.timedelta(days=17)
    true_day = WEEKDAYS[date.weekday()]
    wrong_day = WEEKDAYS[(date.weekday() + 1) % 7]
    when = f"{date.day} {date.strftime('%B')} {date.year}"
    asked: list[str] = []
    fixed = FIXED_PLAN.format(day=f"{true_day}, {when}")

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        if c.speaker == "Product Manager" and WRONG_DAY_SENT in c.last_user:
            asked.append(c.last_user)
            return say("The plan, on the right day.\n\n" + fixed)
        if c.speaker == "Product Manager" and "neighborhood cleanup day" in c.last_user:
            return say(f"Bottom line: hold the cleanup on {wrong_day}, {when}, and book the park today.")
        return None

    w.model.script = script
    picked_first_task(w, "Plan a project Turn an idea into a manageable first step.", "Product Manager")
    row = first_row(w)
    settled(w, 1, "the first task settles")
    kept = said(w, "Product Manager")
    w.check("an answer with a wrong weekday goes back once with the true one, and only the corrected answer is kept",
            len(asked) == 1 and f"{when} is a {true_day}, not a {wrong_day}" in asked[0]
            and kept == ["The plan, on the right day.\n\n" + fixed],
            observed={"sentBack": asked, "kept": kept}, shot=True)
    w.check("For you is headed by the corrected Bottom line",
            row["headline"] == fixed.removeprefix("Bottom line: "), observed=row)
    no_unscripted(w)


def scenario_figurefix(w: World) -> None:
    asked: list[str] = []

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        if c.speaker != "Financial Analyst":
            return None
        if UNCHECKED_SENT in c.last_user:
            asked.append(c.last_user)
            return say(CHECKED)
        if "buy or lease" in c.last_user:
            return say(GUESSED) if c.tool_results() else call("calculator", expression=SUMS)
        return None

    w.model.script = script
    picked_first_task(w, "Research a decision Compare options and make the tradeoffs clear.", "Financial Analyst")
    row = first_row(w)
    settled(w, 1, "the first task settles")
    kept = said(w, "Financial Analyst")
    w.check("after a sum, a figure the calculator never gave goes back once, named, and the corrected answer is kept",
            len(asked) == 1 and "$107" in asked[0] and "$21,000" not in asked[0] and kept == [CHECKED],
            observed={"sentBack": asked, "kept": kept}, shot=True)
    w.check("For you is headed by the corrected Bottom line", row["headline"] == "buy, on these sums.", observed=row)
    no_unscripted(w)


WEB_ASK = "Read the page at the address in this message: http://169.254.169.254/latest/meta-data/"
WEB_SEARCH_ASK = "Search the web for lease or buy."
SEARCH_KEY = "BSA-verify-search-key-4r7t"
INWARD_REFUSED = "on this host's own network"
NO_SEARCH_SAID = "This endpoint cannot search the web for agents."


def scenario_web(w: World) -> None:
    turns: list[Call] = []

    def web_tools() -> list[list[str]]:
        return [[name for name in c.tools if name in ("web_search", "read_page")] for c in turns]

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        if c.speaker != AGENT:
            return None
        turns.append(c)
        if c.last_user.endswith("Say hello."):
            return say(HELLO)
        if c.last_user.endswith(WEB_ASK):
            if not c.tool_results():
                return call("read_page", url="http://169.254.169.254/latest/meta-data/")
            return say("That address is refused: " + c.tool_results()[-1][:200])
        if c.last_user.endswith(WEB_SEARCH_ASK):
            if not c.tool_results():
                return call("web_search", query="lease or buy")
            return say("The search came back: " + c.tool_results()[-1][:200])
        if c.last_user.endswith("Are you still reading the web?"):
            return say("No.")
        return None

    w.model.script = script
    w.secrets.append(SEARCH_KEY)
    w.start()
    first_run(w)
    settled(w, 1)
    offered = web_tools()
    w.check("with only a local endpoint connected, an agent is offered read_page and not a search nothing can run",
            offered and offered[-1] == ["read_page"], observed=offered)

    write(w, AGENT, WEB_ASK)
    settled(w, 2, "the page read settles")
    refused = [c.tool_results()[-1] for c in turns if c.last_user.endswith(WEB_ASK) and c.tool_results()]
    reached = [a for a in w.egress.attempts if "169.254" in a]
    w.check("a page at a cloud metadata address is refused on the host, and nothing was sent anywhere",
            refused and INWARD_REFUSED in refused[-1] and refused[-1].startswith("Error:") and not reached,
            observed={"refused": bool(refused) and INWARD_REFUSED in refused[-1], "egress": reached},
            detail=refused[-1][:400] if refused else None)

    ui = w.ui
    ui.press(ui.one("button", "App settings"))
    w.wait("Settings opens", lambda: ui.find("region", "Settings"))
    ui.click("tab", "Provider")
    w.wait("the Provider pane is shown", lambda: ui.find("heading", "Provider"))
    w.check("Settings says this endpoint cannot search, and where a key would send a query",
            NO_SEARCH_SAID in ui.text() and bool(ui.find("textbox", "Web search key")),
            observed={"said": NO_SEARCH_SAID in ui.text(), "field": bool(ui.find("textbox", "Web search key"))},
            shot=True)
    ui.fill("textbox", "Web search key", SEARCH_KEY)
    ui.click("button", "Save")
    w.wait("the key is saved", lambda: "using the key here" in ui.text())
    w.check("with a Brave Search key saved, Settings names Brave Search as where a query goes, and shows only its end",
            "Agents search with Brave Search" in ui.text() and SEARCH_KEY not in ui.text()
            and "...4r7t" in ui.text(),
            observed={"brave": "Agents search with Brave Search" in ui.text(), "hint": "...4r7t" in ui.text()},
            shot=True)
    ui.key("Escape")
    w.wait("Settings closes", lambda: not ui.find("region", "Settings"))

    write(w, AGENT, WEB_SEARCH_ASK)
    settled(w, 3, "the search settles")
    offered = web_tools()
    searched = [a for a in w.egress.attempts if "api.search.brave.com" in a]
    elsewhere = [a for a in w.egress.attempts if "brave" not in a and "search" in a]
    w.check("once a key is set the agent is offered web_search, and the query goes to Brave Search and nowhere else",
            offered[-1] == ["web_search", "read_page"] and searched and not elsewhere,
            observed={"offered": offered[-1], "egress": w.egress.attempts})

    agent = agent_id(w, AGENT)
    w.host.call("set_agent_web", id=agent, given=False)
    write(w, AGENT, "Are you still reading the web?")
    settled(w, 4, "the last turn settles")
    w.check("an agent switched off the web is offered neither web tool",
            web_tools()[-1] == [], observed=web_tools()[-1])
    no_unscripted(w)


LONE_RESULT = """(() => {
  const result = [...document.querySelectorAll('.msg--result')].at(-1);
  return {
    name: result.querySelector('.msg__name').innerText,
    text: result.querySelector('.result__body').innerText.trim(),
    records: document.querySelectorAll('[role=log] .handoff').length,
  };
})()"""

MAP_LEASE = "Compare the lease options."
MAP_BUY = "Compare the purchase options."
MAP_LEASE_ASK = "Which lease term should I compare?"
MAP_BUY_ASK = "Which purchase term should I compare?"


def scenario_taskmap(w: World) -> None:
    lease = ("Lease Researcher", "Mileage Analyst", "Pricing Analyst", "Car Reviewer")
    buy = "Purchase Researcher"
    specialist = "Tax Specialist"
    finish_support = threading.Event()

    def script(c: Call) -> Reply | None:
        if c.speaker == AGENT:
            for brief, names, work in ((MAP_LEASE, lease, "Research the lease."),
                                       (MAP_BUY, (buy,), "Research the purchase.")):
                if c.last_user.endswith(brief):
                    return say("I asked the group.") if c.tool_results() else call(
                        "send_message", to=list(names), text=work, intent="work")
            if "analysis is complete." in c.last_user:
                return say("The comparison is complete.")
        for name, work, question in ((lease[0], "Research the lease.", MAP_LEASE_ASK),
                                     (buy, "Research the purchase.", MAP_BUY_ASK)):
            if c.speaker == name and work in c.last_user:
                return say("The analysis is complete.") if c.tool_results() else call(
                    "ask_operator", question=question, options=["Three years", "Five years"])
        if c.speaker == lease[1] and "Research the lease." in c.last_user and not c.tool_results():
            return call("send_message", to=[specialist], text="Check the lease tax.", intent="work")
        if c.speaker in (*lease[1:], specialist):
            return held(finish_support, say("The supporting analysis is complete."))
        return hello(c)

    w.model.script = script
    w.start()
    ui = w.ui
    first_run(w)
    settled(w, 1)
    w.check("no task map appears for a direct exchange",
            not ui.find("region", "Task map"), observed=True, shot=True)
    for name in (*lease, buy, specialist):
        new_agent(w, name, "Compare purchase and lease options.")
    open_agent(w, AGENT)
    write(w, AGENT, MAP_LEASE)
    w.wait("the first colleague asks on its own branch", lambda: MAP_LEASE_ASK in card_text(w))
    w.wait("the first dispatch note is recorded", lambda: "I asked the group." in conversation(w))
    w.wait("the specialist received the nested request", lambda: w.model.by(specialist)
           and any("Check the lease tax." in call["asked"] for call in w.model.by(specialist)))
    write(w, AGENT, MAP_BUY)
    maps = w.wait("two tasks are pinned, both with pending questions", lambda: ui.evaluate("""(() => {
      const maps = [...document.querySelectorAll('.task-map-stack .task-map')];
      if (maps.length !== 2 || maps.filter(el => el.innerText.includes('waiting for you')).length !== 2) return null;
      return { pinned: maps.length, open: maps.filter(el => el.querySelector('.task-map__graph')).length };
    })()"""))
    w.check("two pinned maps leave only the task with the earliest question open",
            maps == {"pinned": 2, "open": 1} and MAP_LEASE_ASK in card_text(w)
            and MAP_BUY_ASK not in card_text(w), observed=maps, shot=True)
    fit = ui.evaluate(TASK_MAP_FIT)
    w.check("the map shows at most three whole branches and offers one control for the rest",
            fit["partial"] == 0 and 1 <= len(fit["names"]) <= 3
            and re.match(rf"Show {4 - len(fit['names'])} more colleague", fit["more"] or ""),
            observed=fit)
    fanout, asking = {}, {}
    for surface in LOOK_SURFACES:
        ui.viewport(1280, 820)
        ui.evaluate("localStorage.setItem('guac.prefs', JSON.stringify({...JSON.parse("
                    "localStorage.getItem('guac.prefs') || '{}'), surface: '%s', look: 2})); location.reload()"
                    % surface)
        w.wait("the requested surface is shown", lambda: ui.evaluate(
            "document.documentElement.dataset.surface") == surface and ui.find("button", "2 questions waiting"))
        open_agent(w, AGENT)
        w.wait("the pinned questions return", lambda: MAP_LEASE_ASK in card_text(w))
        if ui.find("dialog", "Questions"):
            ui.click("button", "Done")
            w.wait("the startup question tray closes", lambda: not ui.find("dialog", "Questions"))
        map_matrix(w, "fan-out", surface, lease, fanout, question=True)
        ui.viewport(1280, 820)
        ui.click("button", "Hide map", exact=False)
        w.wait("the question-only task opens", lambda: MAP_BUY_ASK in card_text(w))
        map_matrix(w, "question only", surface, (buy,), asking, question=True)
    layouts = {**fanout, **asking}
    w.check("the open task map covers at most three tenths of the pane in both themes and window sizes",
            all(d["open"] <= .3 * d["pane"] + 1 and d["partial"] == 0
                and (d["viewport"][0] == 700 or d["answerVisible"]) for d in layouts.values())
            and all(d["more"] for d in fanout.values()), observed=layouts)
    w.check("the fan-out and question-only maps lay out by their pane's width in both themes at every frame",
            all(lays_out_by_pane(d, question=True) for d in layouts.values()),
            observed={key: {k: d[k] for k in ("viewport", "width", "transcript", "mapWidth", "horizontal",
                                             "connected", "centeredLead")}
                      for key, d in layouts.items()})
    w.check("the colleagues disclosure opens exactly the branches the fan-out map hides",
            all(opens_remainder(d, lease) for d in fanout.values())
            and all(opens_remainder(d, (buy,)) for d in asking.values()),
            observed={key: {"shown": d["names"], "rest": d["rest"]} for key, d in layouts.items()})
    insets = {}
    for width, height in (*TASK_MAP_FRAMES, (390, 700)):
        ui.viewport(width, height)
        insets[width] = ui.evaluate("""(() => {
          if (!document.querySelector('.task-map-stack:not([hidden]) .task-map__graph')) return null;
          return ['.composer', '.msg__bubble', '.result'].map(selector =>
            document.querySelector(selector)?.getBoundingClientRect().left);
        })()""")
    w.check("the composer and transcript share one inset beneath the live task map",
            all(edges and None not in edges and max(edges) - min(edges) < 1 for edges in insets.values()),
            observed=insets)
    ui.viewport(900, 700)
    ui.click("button", f"Task map {MAP_LEASE}", exact=False)
    w.wait("the urgent question is back in the map", lambda: MAP_LEASE_ASK in card_text(w))
    ui.click("button", "Hide map", exact=False)
    w.wait("the other task opens", lambda: MAP_BUY_ASK in card_text(w))
    w.check("manually folding a live task keeps it folded while another question is open",
            MAP_LEASE_ASK not in card_text(w) and MAP_BUY_ASK in card_text(w), observed=True, shot=True)
    ui.click("combobox", f"Write to {AGENT}")
    ui.key("Escape")
    w.check("Escape from the composer still opens the Questions tray with a branch waiting",
            ui.find("dialog", "Questions"), observed=True)
    ui.click("button", "Done")
    ui.click("combobox", f"Write to {AGENT}")
    ui.key("Tab")
    w.wait("Tab from the empty composer focuses the open branch's question", lambda: ui.evaluate(
        "Boolean(document.activeElement?.closest('.task-map-stack .task-map__question'))"))
    ui.key("1")
    w.wait("the selected question is answered on the host", lambda: w.host.scalar(
        "SELECT count(*) FROM approvals WHERE state='answered' AND summary=?", MAP_BUY_ASK))
    w.check("Tab and a number answer the question on the open branch",
            w.host.scalar("SELECT count(*) FROM approvals WHERE state='pending' AND summary=?", MAP_LEASE_ASK),
            observed=True)
    ui.click("button", f"Task map {MAP_LEASE}", exact=False)
    w.wait("the lease question is open again", lambda: MAP_LEASE_ASK in card_text(w))
    ui.click("button", "Type an answer")
    w.check("opening the typed answer puts the keyboard in the field",
            ui.evaluate("document.activeElement.getAttribute('aria-label')") == "Type an answer",
            observed=True)
    ui.send("Input.insertText", {"text": "Use four years"})
    ui.key("Enter")
    w.wait("the typed text is recorded against the lease question", lambda: w.host.scalar(
        "SELECT answer FROM approvals WHERE state='answered' AND summary=?", MAP_LEASE_ASK) == "Use four years")
    w.check("the typed answer and shortcut stay with their recorded questions",
            w.host.scalar("SELECT answer FROM approvals WHERE summary=?", MAP_BUY_ASK) == "Three years",
            observed={"lease": "Use four years", "purchase": "Three years"})
    fit = w.wait("the lease map settles after the answer while its colleagues keep working",
                 lambda: specialist in working_line(w) and MAP_LEASE_ASK not in card_text(w)
                 and painted_task_map(ui))
    if not ui.find("button", specialist):
        ui.click("button", fit["more"])
    w.wait("the nested colleague's link can be clicked",
           lambda: ui.find("button", specialist) and ui.on_screen("button", specialist))
    ui.click("button", specialist)
    w.wait("the nested specialist's conversation loads the recorded hand-off",
           lambda: ui.find("log", f"Conversation with {specialist}")
           and "Check the lease tax." in conversation(w))
    w.check("a nested hand-off opens the named colleague's conversation",
            ui.find("heading", specialist) and "Check the lease tax." in conversation(w),
            observed=specialist)
    finish_support.set()
    no_unscripted(w)


OFFSITE = "Plan the team offsite in Lisbon."
OFFSITE_ASK = "Find a venue and a budget for a two-day offsite in Lisbon."
OFFSITE_BUDGET = "Keep the venue under 12,000 euros?"
ROOM_FRAMES = ((1280, 820), (900, 700))

ROOM = """(() => {
  const log = document.querySelector('[role=log]').getBoundingClientRect();
  const header = document.querySelector('.pane__header');
  const foot = document.querySelector('.roster__foot');
  return {
    window: [innerWidth, innerHeight],
    share: log.height / innerHeight,
    header: Math.round(header.getBoundingClientRect().height),
    title: getComputedStyle(header.querySelector('.pane__title')).fontSize,
    tabsInHeader: !!header.querySelector('nav'),
    rowsAbove: [...document.querySelectorAll('.app > :not(.roster, main)')].filter(el => {
      const box = el.getBoundingClientRect();
      return box.width > innerWidth / 2 && box.height > 0;
    }).length,
    bar: !!document.querySelector('main .statusbar'),
    footDate: foot ? /day,/.test(foot.innerText) || foot.innerText.includes(new Date().toLocaleDateString([], { month: 'short', day: 'numeric' })) : false,
  };
})()"""

FOCUSED_QUESTION = """(() => {
  const here = document.activeElement?.closest('[data-question]');
  return here && here.matches('.task-map-stack .task-map__question') ? here.dataset.question : null;
})()"""


def room(w: World) -> dict:
    ui = w.ui
    seen = {}
    for width, height in ROOM_FRAMES:
        ui.viewport(width, height)
        w.wait("the frame settles", lambda: ui.evaluate(look.SETTLED) and painted_task_map(ui))
        seen[f"{width}x{height}"] = ui.evaluate(ROOM)
    return seen


def holds_room(seen: dict) -> bool:
    return all(d["share"] >= .4 and d["tabsInHeader"] and d["title"] == "20px" and d["rowsAbove"] == 0
               and not d["bar"] and d["footDate"] for d in seen.values())


def scenario_room(w: World) -> None:
    planner = "Product Manager"
    asking = threading.Event()
    budgeting = threading.Event()

    def script(c: Call) -> Reply | None:
        if c.speaker == AGENT and c.last_user.endswith(OFFSITE):
            return say("I asked them both.") if c.tool_results() else call(
                "send_message", to=[MARKET, planner], text=OFFSITE_ASK, intent="work")
        if c.speaker == MARKET and OFFSITE_ASK in c.last_user:
            if c.tool_results():
                return say("Venue held under budget.")
            return held(asking, call("ask_operator", question=OFFSITE_BUDGET, options=["Under 12,000", "Up to 15,000"]))
        if c.speaker == planner and OFFSITE_ASK in c.last_user:
            return held(budgeting, say("Budget drafted."))
        if c.speaker == AGENT and ("Venue held" in c.last_user or "Budget drafted." in c.last_user):
            return say("The offsite is planned.")
        return hello(c)

    w.model.script = script
    w.start()
    ui = w.ui
    first_run(w)
    settled(w, 1)
    write(w, AGENT, OFFSITE)
    w.wait("both colleagues are working on the map", lambda: "Working" in card_text(w)
           and "waiting for" in working_line(w))
    working = room(w)
    w.check("while the group works the transcript keeps two fifths of the window, under a one-row header and no status bar",
            holds_room(working), observed={k: {"room": d["share"] >= .4, "header": d["header"] < 80} for k, d in working.items()},
            detail=working, shot=True)

    asking.set()
    w.wait("the colleague's question is on its branch", lambda: OFFSITE_BUDGET in card_text(w))
    question = room(w)
    w.check("with a question on the map the transcript keeps two fifths of the window at 1280x820 and 900x700",
            holds_room(question), observed={k: d["share"] >= .4 for k, d in question.items()}, detail=question, shot=True)

    pending = lambda: w.host.scalar("SELECT count(*) FROM approvals WHERE state='pending' AND summary=?", OFFSITE_BUDGET)
    asked = w.host.scalar("SELECT id FROM approvals WHERE summary=?", OFFSITE_BUDGET)
    launches = {}
    for width, height in ROOM_FRAMES:
        ui.viewport(width, height)
        ui.evaluate("location.reload()")
        w.wait("the page is back", lambda: admitted(ui) and ui.find("log", f"Conversation with {AGENT}"))
        focus = w.wait("focus lands on the question's branch", lambda: ui.evaluate(FOCUSED_QUESTION), timeout=10)
        launches[f"{width}x{height}"] = {"focus": focus == asked, "tray": bool(ui.find("dialog", "Questions"))}
    w.check("launch opens the asking task's conversation with focus on its branch and no Questions tray",
            all(one["focus"] and not one["tray"] for one in launches.values()), observed=launches, shot=True)

    ui.key("Escape")
    kept = ui.evaluate(FOCUSED_QUESTION) == asked and not ui.find("dialog", "Questions")
    ui.click("combobox", f"Write to {AGENT}")
    ui.key("Escape")
    found = w.wait("Escape from the composer focuses the question on its branch",
                   lambda: ui.evaluate(FOCUSED_QUESTION) == asked, timeout=5)
    ui.key("1")
    w.wait("the number answers the question on the host", lambda: not pending())
    w.check("Escape focuses the waiting question wherever it is, and 1 answers it",
            kept and found and not ui.find("dialog", "Questions")
            and w.host.scalar("SELECT answer FROM approvals WHERE id=?", asked) == "Under 12,000",
            observed={"kept": kept, "found": bool(found)})
    budgeting.set()
    w.wait("the lead answers with the plan", lambda: "The offsite is planned." in conversation(w))
    no_unscripted(w)


RESULT_FRAME = """(() => {
  const result = [...document.querySelectorAll('.msg--result')].at(-1);
  const card = result.querySelector('.result');
  const page = getComputedStyle(document.querySelector('[role=log]')).backgroundColor;
  const style = getComputedStyle(card);
  const rows = [...document.querySelectorAll('[role=log] > div[data-message]')];
  const at = rows.findIndex((row) => row.contains(result));
  const record = rows[at].querySelector('.task-map__summary');
  const filler = rows.slice(0, at).map((row) => row.querySelector('.msg:not([data-operator])')).filter(Boolean).at(-1);
  const brief = document.querySelector('.msg[data-operator] .msg__bubble');
  return {
    name: result.querySelector('.msg__name').innerText,
    text: result.querySelector('.result__body').innerText.trim(),
    raised: style.backgroundColor !== page && style.boxShadow !== 'none',
    fillerRaised: filler ? getComputedStyle(filler).boxShadow !== 'none' : null,
    resultHeight: card.getBoundingClientRect().height,
    briefHeight: brief.getBoundingClientRect().height,
    record: record ? record.innerText : null,
    recordAbove: !!record && !!(record.compareDocumentPosition(card) & Node.DOCUMENT_POSITION_FOLLOWING),
  };
})()"""

BRIEF_HEIGHT = """(() => {
  const brief = document.querySelector('.msg[data-operator] .msg__brief');
  const lead = parseFloat(getComputedStyle(brief.querySelector('p') || brief).lineHeight);
  return { lines: Math.round(brief.getBoundingClientRect().height / lead), text: brief.innerText.trim() };
})()"""

def finished_result(starts: str) -> str:
    return """(() => {
  const row = document.querySelector('.for-you .finished-line[title^="%s"]');
  if (!row) return null;
  return {
    byline: row.querySelector('.finished-line__who').innerText.trim(),
    headline: row.title,
  };
})()""" % starts


CREW_OFFER = """(() => {
  const offer = document.querySelector('[role=log] .crew-offer');
  const result = [...document.querySelectorAll('[role=log] .msg--result')].at(-1);
  if (!offer || !result) return null;
  return {
    text: offer.querySelector('.crew-offer__text').innerText.trim(),
    after: !!(result.compareDocumentPosition(offer) & Node.DOCUMENT_POSITION_FOLLOWING),
    inLog: !!offer.closest('[role=log]'),
    buttons: [...offer.querySelectorAll('button')].map(b => b.innerText.trim()),
    dialogs: document.querySelectorAll('[role=dialog], [role=alertdialog], dialog[open]').length,
  };
})()"""


PHONES = ((360, 800), (390, 844), (430, 932))
SURFACES = ("light", "dark")
LONG_NAME = "Researcher for International Market Operations"
NAME_FLOOR = 96
TABS_FLOOR = 44
SCROLL_ROUNDING = 2
CHECK_TOTALS = "Check the totals."
MEASURE_FIX = "Can the measure fix go out today?"
MEASURE_QUESTION = "Review the measure fix now, or wait for Monday's build?"
ORDINARY_DAY = utc_ms(2026, 6, 17, 12)

PHONE_FACTS = """(() => {
  const shown = (el) => el.checkVisibility({ opacityProperty: true, visibilityProperty: true })
    && el.getBoundingClientRect().width > 0 && el.getBoundingClientRect().height > 0;
  const text = (el) => el.textContent.replace(/\\s+/g, ' ').trim();
  const rect = (el) => { const r = el.getBoundingClientRect(); return { left: r.left, right: r.right, top: r.top, bottom: r.bottom }; };
  const foot = document.querySelector('.roster__foot');
  const items = [...foot.querySelectorAll('button, .roster__day, .roster__greeting')].filter(shown).map((el) => {
    const range = document.createRange();
    range.selectNodeContents(el);
    const r = el.matches('button') ? el.getBoundingClientRect() : range.getBoundingClientRect();
    return { name: text(el), left: r.left, right: r.right, top: r.top, bottom: r.bottom };
  });
  const title = document.querySelector('.pane__title');
  const tabs = document.querySelector('.agent-tabs__list');
  const strip = document.querySelector('.roster__scroll');
  const cards = [...strip.querySelectorAll('.roster__agent')];
  const stripAt = strip.scrollLeft;
  strip.scrollLeft = 0;
  const first = cards[0].getBoundingClientRect().left;
  strip.scrollLeft = strip.scrollWidth;
  const last = cards.at(-1).getBoundingClientRect().right;
  strip.scrollLeft = stripAt;
  const tabsAt = tabs.scrollLeft;
  tabs.scrollLeft = tabs.scrollWidth;
  const lastTab = tabs.lastElementChild.getBoundingClientRect().right;
  tabs.scrollLeft = tabsAt;
  const header = document.querySelector('.pane__header');
  return {
    width: innerWidth,
    foot: { ...rect(foot), spill: foot.scrollWidth - foot.clientWidth, items },
    name: { text: title.textContent, shown: title.clientWidth, whole: title.scrollWidth, ...rect(title) },
    header: { ...rect(header), spill: header.scrollWidth - header.clientWidth },
    tabs: { ...rect(tabs), lastEnd: lastTab },
    more: rect(document.querySelector('.pane__more')),
    strip: { ...rect(strip), first, last },
    chip: rect(document.querySelector('.roster__questions')),
    picker: [...document.querySelectorAll('.roster__compact option')].map((o) => o.textContent).filter((t) => /\\((Ctrl|\u2318|\u2303)/.test(t)),
  };
})()"""

SCREEN_FACTS = """(() => {
  const shown = (el) => el.checkVisibility({ opacityProperty: true, visibilityProperty: true })
    && el.getBoundingClientRect().width > 0 && el.getBoundingClientRect().height > 0;
  const named = (el) => (typeof el.className === 'string' && el.className.split(' ')[0]) || el.tagName.toLowerCase();
  const held = (el) => {
    for (let up = el.parentElement; up && up !== document.body; up = up.parentElement) {
      if (/hidden|auto|scroll|clip/.test(getComputedStyle(up).overflowX)) {
        const r = up.getBoundingClientRect();
        if (r.left >= -0.5 && r.right <= innerWidth + 0.5) return true;
      }
    }
    return false;
  };
  const off = [...document.body.querySelectorAll('*')].filter((el) => {
    if (el.closest('svg, [hidden]') || !shown(el)) return false;
    const r = el.getBoundingClientRect();
    return (r.right > innerWidth + 0.5 || r.left < -0.5) && !held(el);
  }).map(named);
  const hints = [...document.querySelectorAll('kbd, .desk__key, .toast__key')].filter(shown).map((el) => el.textContent.trim());
  return { page: document.documentElement.scrollWidth, width: innerWidth, off: [...new Set(off)], hints };
})()"""

PHONE_CLAIMS = ("foot", "name", "hints", "strip", "fit")


def phone_faults(facts: dict, ordinary: bool) -> dict[str, list[str]]:
    foot, name, header, tabs = facts["foot"], facts["name"], facts["header"], facts["tabs"]
    items = foot["items"]
    row = [f"{a['name']!r} is drawn over {b['name']!r}" for index, a in enumerate(items) for b in items[index + 1:]
           if min(a["right"], b["right"]) - max(a["left"], b["left"]) > 1
           and min(a["bottom"], b["bottom"]) - max(a["top"], b["top"]) > 1]
    row += [f"{item['name']!r} is drawn off the edge" for item in items
            if item["left"] < foot["left"] - 0.5 or item["right"] > min(foot["right"], facts["width"]) + 0.5]
    row += [f"the foot scrolls by {foot['spill']:g}px"] if foot["spill"] > 1 else []
    cut = name["whole"] > name["shown"] + 1
    called = []
    if ordinary and cut:
        called.append(f"{name['text']!r} is cut to {name['shown']}px of {name['whole']}px")
    if not ordinary and name["shown"] < NAME_FLOOR:
        called.append(f"{name['text']!r} is cut to {name['shown']}px, under {NAME_FLOOR}px")
    if name["left"] < header["left"] - 0.5 or name["right"] > header["right"] + 0.5 or header["spill"] > 1:
        called.append("the header is wider than the screen")
    if tabs["right"] - tabs["left"] < TABS_FLOOR:
        called.append(f"the tabs have {tabs['right'] - tabs['left']:g}px to scroll in")
    if tabs["lastEnd"] > tabs["right"] + SCROLL_ROUNDING or tabs["right"] > facts["more"]["left"] + 0.5:
        called.append("the last tab cannot be scrolled clear of the edge")
    strip, chip = facts["strip"], facts["chip"]
    reach = []
    if chip["right"] > strip["left"] + 0.5:
        reach.append("the questions chip covers the strip")
    if strip["first"] < strip["left"] - 0.5 or strip["last"] > strip["right"] + 0.5:
        reach.append("an agent cannot be scrolled fully into view")
    return {"foot": row, "name": called, "strip": reach}


def phone_screen(w: World, seen: dict[str, list[str]], cell: str, what: str) -> None:
    facts = w.ui.evaluate(SCREEN_FACTS)
    seen["hints"] += [f"{cell}, {what}: {hint!r}" for hint in facts["hints"]]
    if facts["page"] > facts["width"] or facts["off"]:
        seen["fit"].append(f"{cell}, {what}: {facts['off'] or 'the page'} is wider than {facts['width']}px")


def pointer_settles(w: World, touch: bool) -> None:
    ui = w.ui
    w.wait(f"the page reports {'a touch screen' if touch else 'a mouse'}",
           lambda: ui.evaluate(f"matchMedia('(hover: none) and (pointer: coarse)').matches === {str(touch).lower()}"))
    ui.evaluate("new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)))",
                await_promise=True)


def phone_cell(w: World, seen: dict[str, list[str]], surface: str, size: tuple[int, int], name: str,
               waiting: int) -> None:
    ui = w.ui
    ui.phone(*size)
    cell = f"{surface} {size[0]}x{size[1]}, {name!r}, {waiting} waiting"
    w.wait(f"the page is {size[0]} wide", lambda: ui.evaluate(f"innerWidth === {size[0]}") and ui.evaluate(look.SETTLED))
    pointer_settles(w, True)
    open_agent(w, AGENT)
    open_agent(w, name)
    w.wait(f"the header names {name}", lambda: ui.evaluate("document.querySelector('.pane__title')?.textContent") == name)
    w.wait("the phone layout settles", lambda: ui.evaluate(look.SETTLED))
    facts = ui.evaluate(PHONE_FACTS)
    for key, found in phone_faults(facts, name == "Dara").items():
        if found and not seen[key]:
            w.capture(f"{key} fault, {cell}")
        seen[key] += [f"{cell}: {one}" for one in found]
    seen["hints"] += [f"{cell}, the agent picker: {option!r}" for option in facts["picker"]]
    phone_screen(w, seen, cell, "the conversation")
    if (waiting, name, size[0]) == (1, "Dara", 390):
        w.capture(f"the conversation, {cell}")
    if waiting != 1 or name != "Dara":
        return
    ui.click("button", "Now")
    w.wait("the Now tab opens", lambda: ui.evaluate("!!document.querySelector('.agent-panel')"))
    phone_screen(w, seen, cell, "the Now tab")
    ui.click("button", "Conversation")
    w.wait("the conversation returns", lambda: ui.find("log", f"Conversation with {name}"))
    ui.click("button", "For you, 1 question")
    w.wait("For you opens", lambda: ui.evaluate("!!document.querySelector('.for-you')"))
    w.wait("For you has finished moving", lambda: ui.evaluate(look.SETTLED))
    phone_screen(w, seen, cell, "For you")
    ui.key("Escape")
    w.wait("For you closes", lambda: not ui.evaluate("!!document.querySelector('.for-you')"))
    ui.click("button", "Agents")
    w.wait("the Groups sheet opens", lambda: ui.find("dialog", "Groups and agents"))
    w.wait("the Groups sheet has finished moving", lambda: ui.evaluate(look.SETTLED))
    phone_screen(w, seen, cell, "the Groups sheet")
    ui.click("button", "Command menu")
    w.wait("the command menu opens", lambda: ui.find("dialog", "Command menu"))
    w.wait("the command menu has finished moving", lambda: ui.evaluate(look.SETTLED))
    phone_screen(w, seen, cell, "the command menu")
    ui.key("Escape")
    w.wait("the command menu closes", lambda: not ui.find("dialog", "Command menu"))
    dismiss(w)


def asks(w: World, name: str, waiting: int) -> None:
    w.host.call("send_message", agentId=agent_id(w, name), text=MEASURE_FIX)
    w.wait(f"{name} has a question", lambda: roster_status(w, name) == "Has a question")
    w.wait(f"{waiting} waiting", lambda: w.ui.find("button", f"{waiting} question{'s' if waiting > 1 else ''} waiting"))


def scenario_phone(w: World) -> None:

    def script(c: Call) -> Reply | None:
        if c.last_user.endswith(MEASURE_FIX):
            return call("ask_operator", question=MEASURE_QUESTION, options=["Review now", "Wait for Monday"])
        if c.last_user.endswith(CHECK_TOTALS):
            return say("The totals match.")
        return hello(c)

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)
    ui = w.ui
    ui.clock(ORDINARY_DAY)
    for name in ("Pip", "Dara", LONG_NAME):
        new_agent(w, name, "Wait to be asked.")
    open_agent(w, AGENT)
    w.host.call("send_message", agentId=agent_id(w, "Pip"), text=CHECK_TOTALS)
    w.wait("Pip has a result", lambda: roster_status(w, "Pip") == "Has a result")

    seen: dict[str, list[str]] = {key: [] for key in PHONE_CLAIMS}
    for waiting in (0, 1, 3):
        if waiting == 1:
            asks(w, "Dara", 1)
        if waiting == 3:
            asks(w, LONG_NAME, 2)
            asks(w, AGENT, 3)
        for surface in SURFACES:
            look_surface(w, surface)
            for name in ("Dara", LONG_NAME):
                for size in PHONES:
                    phone_cell(w, seen, surface, size, name, waiting)

    ui.desktop(390, 844)
    w.wait("a narrow window with a mouse is laid out as one column", lambda: ui.evaluate("innerWidth === 390")
           and ui.evaluate(look.SETTLED))
    pointer_settles(w, False)
    mouse = ui.evaluate("[...document.querySelectorAll('.roster__agent kbd')].filter(k => k.checkVisibility()).length")

    w.check("the foot row's items sit apart and inside the screen at every phone width, surface and question count",
            not seen["foot"], observed=len(seen["foot"]), detail=seen["foot"][:6] or None, stop=False)
    w.check("an agent's name is shown whole, or shortened no further than a readable width, with its tabs still reachable",
            not seen["name"], observed=len(seen["name"]), detail=seen["name"][:6] or None, stop=False)
    w.check("no keyboard hint is drawn under touch emulation, and a narrow window with a mouse still draws them",
            not seen["hints"] and mouse > 0, observed={"touch": len(seen["hints"]), "mouse": mouse},
            detail=seen["hints"][:6] or None, stop=False)
    w.check("the questions chip never covers the agent strip, and every agent scrolls fully into view",
            not seen["strip"], observed=len(seen["strip"]), detail=seen["strip"][:6] or None, stop=False)
    w.check("the conversation, the Now tab, For you, the Groups sheet and the command menu fit every phone width",
            not seen["fit"], observed=len(seen["fit"]), detail=seen["fit"][:6] or None, stop=False)
    no_unscripted(w)


SCENARIOS: dict[str, Scenario] = {
    "phone": Scenario(scenario_phone, "On a 360, 390 and 430 pixel phone, in paper and ink, with a short and a long "
                      "agent name and none, one and several questions waiting, the foot row's items never overlap, "
                      "the agent's name stays readable, no keyboard hint is drawn and the strip stays reachable; "
                      "the same checks fail on any cell that overlaps, cuts the name to a letter or shows a key.",
    (
        "the foot row's items sit apart and inside the screen at every phone width, surface and question count",
        "an agent's name is shown whole, or shortened no further than a readable width, with its tabs still reachable",
        "no keyboard hint is drawn under touch emulation, and a narrow window with a mouse still draws them",
        "the questions chip never covers the agent strip, and every agent scrolls fully into view",
        "the conversation, the Now tab, For you, the Groups sheet and the command menu fit every phone width",
        "every model call was one the scenario scripted",
    )),
    "room": Scenario(scenario_room, "The conversation keeps two fifths of the window while a group works and while a "
                     "question waits on the map, under a one-row header with no status bar; launch focuses the "
                     "waiting question where it sits, without the Questions tray, and Escape then 1 answers it.",
    (
        "while the group works the transcript keeps two fifths of the window, under a one-row header and no status bar",
        "with a question on the map the transcript keeps two fifths of the window at 1280x820 and 900x700",
        "launch opens the asking task's conversation with focus on its branch and no Questions tray",
        "Escape focuses the waiting question wherever it is, and 1 answers it",
        "every model call was one the scenario scripted",
    )),
    "taskmap": Scenario(scenario_taskmap, "Delegated tasks pin their maps independently and answer on a "
                        "branch; a one-to-one reply has no map.",
    (
        "no task map appears for a direct exchange",
        "two pinned maps leave only the task with the earliest question open",
        "the map shows at most three whole branches and offers one control for the rest",
        "the open task map covers at most three tenths of the pane in both themes and window sizes",
        "the fan-out and question-only maps lay out by their pane's width in both themes at every frame",
        "the colleagues disclosure opens exactly the branches the fan-out map hides",
        "the composer and transcript share one inset beneath the live task map",
        "manually folding a live task keeps it folded while another question is open",
        "Tab and a number answer the question on the open branch",
        "Escape from the composer still opens the Questions tray with a branch waiting",
        "opening the typed answer puts the keyboard in the field",
        "the typed answer and shortcut stay with their recorded questions",
        "a nested hand-off opens the named colleague's conversation",
        "every model call was one the scenario scripted",
    )),
    "controls": Scenario(scenario_controls, "A person changes an exclusive setting by keyboard and uses the "
                         "conversation composer; an empty draft cannot send and a custom look selects no preset.",
    (
        "Send is unavailable for a blank draft",
        "the composer has an icon Send and a recipient pill with a face",
        "the composer, result, brief and question start on the same edge",
        "the focused composer and name field share one neutral ring without a halo",
        "short choices are segments and longer choices are visible radios",
        "one arrow changes the selected segment and applies the new surface",
        "Tab leaves the radio group instead of stopping on each segment",
        "a custom look does not falsely mark any preset selected",
        "long notification choices use visible radios even with only two options",
        "a refused notification format keeps focus and the saved value without a second write",
        "a notification format stays selected after the Host saves it",
        "the recipient pill moves the unsent draft to the selected conversation",
        "every model call was one the scenario scripted",
    )),
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
        "Start task opens the conversation while the agent is still answering",
        "the brief reached the agent and its reply is in the conversation",
        "a lone agent's reply lands as a result card under its name, with no hand-off record",
        "a message written in the composer is answered in the same conversation",
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
    "running": Scenario(scenario_running, "The roster shows working status while the conversation shows its note, "
                        "clock and Stop; a Host restarted mid-turn keeps the clock going from the first start, "
                        "and Stop leaves the reason in the transcript.",
    (
        "with nothing running, the roster says Idle and the conversation has no Stop",
        "the running conversation shows its latest note, time since the Host started the work, and Stop",
        "the clock in the conversation ticks while the turn works",
        "reload prefers the running conversation to the last open conversation",
        "after a restart mid-turn the clock goes on from the first Host's start rather than from zero",
        "Stop ends the run and replaces the working controls with the stopped transcript",
        "every model call was one the scenario scripted",
    )),
    "handoff": Scenario(scenario_handoff, "An agent asks two colleagues; a live card shows each face, the question, a "
                        "status word with time and the answer at reading size, the status line says who it waits "
                        "for with Stop, and the transcript keeps two text edges; after the replies the card folds "
                        "to one line; a hand-off cut short by Stop folds and says who answered.",
    (
        "the pinned map shows each colleague, the work still underway, and the result already sent",
        "a pinned branch is fully visible or behind the colleagues control",
        "the disclosure reveals the whole hidden colleague branch",
        "the two-colleague map lays out by its pane's width in both themes at every frame",
        "the colleagues disclosure opens exactly the branches the two-colleague map hides",
        "under the running turn a status line says what it is doing, for how long, with Stop",
        "text in the transcript starts on two left edges",
        "the person's turn sits in a bubble that stands apart from the page",
        "a turn scrolled under the tabs fades out instead of being cut by them",
        "the hand-off keeps to the six type roles, the control family and the radii in light and dark",
        "an agent opened mid-turn shows its status line without scrolling",
        "after the reply the hand-off folds to one line that says both answered",
        "a colleague's open message stays readable after the pinned task settles",
        "the folded line opens onto both answers",
        "opening a colleague's message shows their recorded words",
        "a hand-off cut short by Stop folds and says who answered, instead of ticking on",
        "a colleague whose model call failed is marked failed on the map, in plain words, with Try again",
        "a colleague's failure does not promote the lead's progress note to a result",
        "a failed map read cannot turn a progress note into a result",
        "every model call was one the scenario scripted",
    )),
    "failed": Scenario(scenario_failed, "A model call fails while a turn is also working on a second conversation; "
                       "the second is told at once that it was not finished, with Try again, rather than ending "
                       "silently with the turn.",
    (
        "a failed model call tells the other conversation the turn acted on at once, with Try again on each",
        "both failure notices start at the composer edge",
        "every model call was one the scenario scripted",
    )),
    "failure": Scenario(scenario_failure, "A model call fails: the conversation says what happened and what to do in "
                        "plain words, with the provider's words behind Details, legible in light and dark; the Now "
                        "tab, the roster and For you call it failed, and Try again from For you sends it again.",
    (
        "a failed model call says what happened and what to do, in plain words, with no status code",
        "Details shows the provider's own words for someone who wants them",
        "the failure's words measure at least 4.5 to 1 on their background in light and dark, with Try again on one line",
        "Try again is a standard control at the standard control height",
        "the Now tab calls the last run failed, in the same words, and offers Try again",
        "an agent whose last task failed is listed as failed, not as needing attention",
        "For you files the failed task under Did not finish with Try again, never with finished work",
        "Try again from For you sends the same task again and it finishes",
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
    "season": Scenario(scenario_season, "On the fourth Thursday of November and on December 25 the window marks "
                       "its root, writes the greeting under the date and puts an accent on every face, and the "
                       "next day, read when the window is focused, clears all of it; with Seasonal looks switched "
                       "off, Christmas Day draws none of it.",
    (
        "on the fourth Thursday of November the root, the greeting and every face say Thanksgiving",
        "the decorated conversation keeps to the six type roles, the control family and the radii",
        "the day after, focusing the window clears the season, the greeting and every accent",
        "on December 25 the root, the greeting and every face say Christmas",
        "with Seasonal looks off, Christmas Day draws no palette, greeting, accent or decoration",
        "every model call was one the scenario scripted",
    )),
    "look": Scenario(scenario_look, "Questions, a conversation, Settings, New agent, the Groups sheet and Search, in "
                     "light and dark, each keep to the six type roles, eight styles at most, the control heights "
                     "and the radii; a screen with a size, weight, corner or control outside them fails, naming "
                     "the element and the token to use.",
    (
        "every screen in light keeps to the six type roles, the control family and the radii",
        "every screen in dark keeps to the six type roles, the control family and the radii",
        "every model call was one the scenario scripted",
    )),
    "scrollbars": Scenario(scenario_scrollbars, "Questions, a conversation, Settings, New agent, the Groups sheet, "
                           "Search and the command menu, in light and dark at 1280x820, put every element in the "
                           "same place whether scroll bars overlay the page or take room beside it, and New agent "
                           "shows its Character grid in two rows either way; a region whose "
                           "scroll bar takes width from its content fails, naming what moved.",
    (
        "every screen in light lays out the same with classic scroll bars as with overlay ones",
        "every screen in dark lays out the same with classic scroll bars as with overlay ones",
        "Settings keeps the General tab and Your name field in place with a native gutter",
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
    "firstgroup": Scenario(scenario_firstgroup, "A person picks a suggested first task; the Start step names the "
                           "one agent it will hire and who can join later, clear of the pinned bar; a Start that cannot reach the Host "
                           "says why beside Start, on screen, and hires nobody; Start then hires that one agent exactly once into a group "
                           "named for the task, and its first result comes back in one model call with no message to a colleague. "
                           "A quiet line under that result offers the crew; one action hires the coordinator and a colleague, "
                           "the coordinator asks both colleagues at once, the live hand-off card, status line and roster show it "
                           "running, and one deeper result comes back.",
    (
        "the first step says one agent answers first, colleagues can follow, and keeps Create one agent",
        "the first run shows its steps alone, with no window buttons, sidebar or places, and focus on the first task",
        "the suggested brief is for one agent to finish alone, names no colleague, and does not open by asking the person",
        "the first step shows the brief and Continue without scrolling at 900x700 and 1280x820",
        "the Start step names the one agent it will hire, what it is for, and who can join later, before any is hired",
        "the agent and what it is for clear the pinned bar at 900x700 and 1280x820",
        "the Start step shows the brief it will send without scrolling, with focus on Start task",
        "a Start that fails hires nobody and sends nothing, and keeps the brief",
        "a Start that fails says why beside Start, on screen without scrolling at 900x700 and 1280x820",
        "Start hires the one agent exactly once",
        "the starter group is named for the task it was hired for, on the agent's page",
        "while it works, the conversation offers no crew yet",
        "the first result comes from one agent in one model call, with no message to or from a colleague",
        "For you shows the first result under the one agent's name, with nobody else credited",
        "under the first result, one quiet line offers the crew, naming who would join, with no dialog",
        "the crew offer adds no question, notification or For you entry of its own",
        "one action hires the coordinator and the colleague into the same group, once",
        "the deeper pass carries the person's first brief to the coordinator and names both colleagues",
        "the coordinator asks both colleagues on its first turn, in one message, and the person sees the card",
        "under the coordinator a status line names both colleagues it waits for",
        "beside the conversation, the roster shows the coordinator working",
        "For you shows who produced the result and who helped, with a preview cut on a boundary",
        "the coordinator comes back with one result after both answered, and the run ends there",
        "the result is a raised card under the coordinator's name, and the brief before it is folded below it in weight",
        "the hand-off settles into a record right above the result that says both answered",
        "the record opens onto both answers",
        "the roster and For you can return from a colleague thread to the current agent",
        "the person's long brief is folded to three lines and opens whole on request",
        "the roster shows Working while the earlier result stays in the conversation",
        "every model call was one the scenario scripted",
    )),
    "firstplan": Scenario(scenario_firstplan, "A person picks Plan a project; Product Manager alone answers in one "
                          "model call with no message to a colleague, a quiet line under the result offers the crew, "
                          "and Not now hires nobody and stays gone after a reload.",
    (
        "the first result comes from one agent in one model call, with no message to or from a colleague",
        "under the first result, one quiet line offers the crew, naming who would join, with no dialog",
        "Not now hires nobody, and the offer stays gone after a reload",
        "every model call was one the scenario scripted",
    )),
    "weekdayfix": Scenario(scenario_weekdayfix, "A person picks Plan a project; Product Manager first names a wrong "
                           "weekday, the answer goes back once with the true one, and the corrected answer is the one "
                           "kept and the one For you is headed by.",
    (
        "an answer with a wrong weekday goes back once with the true one, and only the corrected answer is kept",
        "For you is headed by the corrected Bottom line",
        "every model call was one the scenario scripted",
    )),
    "figurefix": Scenario(scenario_figurefix, "A person picks Research a decision; Financial Analyst works a sum with "
                          "the calculator and then states a figure it never worked out, the answer goes back once "
                          "naming it, and the corrected answer is the one kept and the one For you is headed by.",
    (
        "after a sum, a figure the calculator never gave goes back once, named, and the corrected answer is kept",
        "For you is headed by the corrected Bottom line",
        "every model call was one the scenario scripted",
    )),
    "firstwrite": Scenario(scenario_firstwrite, "A person picks Draft content; Content Marketer alone answers in one "
                           "model call with no message to a colleague, a quiet line under the result offers the crew, "
                           "and Not now hires nobody and stays gone after a reload.",
    (
        "the first result comes from one agent in one model call, with no message to or from a colleague",
        "under the first result, one quiet line offers the crew, naming who would join, with no dialog",
        "Not now hires nobody, and the offer stays gone after a reload",
        "every model call was one the scenario scripted",
    )),
    "newagent": Scenario(scenario_newagent, "A person opens New agent in a 1280 by 820 window, reaches its one "
                         "Create agent button in five Tabs from Name with the optional parts folded, and creates "
                         "agents with a press on it and with Return in Name, landing in the new agent's composer "
                         "each time; Return or a press with no name creates nothing.",
    (
        "Create agent is one button, on screen without scrolling",
        "with no name, Return in Name and a press on Create agent create nothing and keep the dialog",
        "from Name, Create agent is five Tabs away, one stop each for Color, Character and the options",
        "a keyboard press on Create agent creates the agent and closes the dialog",
        "after Create, the focus is in the new agent's composer",
        "an arrow key changes the color inside its one stop, and Return in Name creates the agent with it",
        "after Return in Name, the focus is in that agent's composer",
        "every model call was one the scenario scripted",
    )),
    "answer": Scenario(scenario_answer, "An agent asks a question while its conversation is open, and Tab from "
                       "the composer, which still has the focus, reaches the first answer and Return sends it; with "
                       "nothing waiting, Tab from the composer goes where it always went.",
    (
        "with a question waiting, Tab from the composer reaches its first answer",
        "Return answers the question, and the agent is told the answer",
        "with nothing waiting, Tab leaves the composer for its own next control",
        "every model call was one the scenario scripted",
    )),
    "headline": Scenario(scenario_headline, "Four agents finish while For you is open, and For you lists a draft "
                         "under the label Draft and a verdict after \"I'll work this out for you...\" by the answer "
                         "after the lead-in; a plan with nothing before it keeps its own first line; a balance "
                         "worked out with the calculator is listed by its Bottom line, and every conversation "
                         "keeps the words as written.",
    (
        "an agent works a sum out with the calculator, and its Bottom line is the For you headline",
        "the conversation keeps the Bottom line in the full result",
        "a result that opens with a label or a promise is listed by the answer after it",
        "a result with nothing before its answer is listed by its own first line",
        "the conversation keeps every result word for word",
        "every model call was one the scenario scripted",
    )),
    "roster": Scenario(scenario_roster, "Eleven agents stay readable and ordered at two window sizes, and the tenth "
                       "can ask without switching the conversation; click, step and jump without losing a draft, "
                       "keep Search from moving the conversation under the command menu, and reload into the question.",
    (
        "each agent has one ordered row, with shortcuts only for the first nine",
        "the conversation has no Today destination or spare writing field",
        "a new question changes the tenth status without reordering or switching conversations",
        "desktop names wrap before they are shortened and stay above the status and shortcut",
        "the composer and Send stay visible with a question at keyboard height",
        "keyboard selection inserts a visible mention without sending at keyboard height",
        "mouse, Control Tab in both directions, and the numbered shortcut keep the draft with its agent",
        "stacked Search and its keys leave the command menu and underlying draft unchanged",
        "reload lands on the pending question but keeps the other conversation's draft",
        "the roster and command menu both show Command 1 on macOS",
        "every model call was one the scenario scripted",
    )),
    "grouptabs": Scenario(scenario_grouptabs, "A person opens a group's settings from the Groups sheet, reaches "
                          "its section list in one Tab, moves between sections with the arrow keys, Home and End "
                          "and Tabs into the open panel; the list is a single Tab stop in both directions.",
    (
        "the first Tab stop in a group's settings is the open tab",
        "an arrow key moves to the next section and opens it",
        "the opposite arrow key moves back and opens that section",
        "Home and End reach the first and the last section",
        "Tab goes from the section list into the open panel",
        "the section list is one Tab stop",
        "every model call was one the scenario scripted",
    )),
    "web": Scenario(scenario_web, "With only a local endpoint connected an agent reads a page without a browser "
                    "and is not offered a search nothing can run; a page on the host's own network is refused "
                    "with nothing sent; Settings says where a query would go, and once a Brave Search key is saved "
                    "the agent searches and the query goes to Brave Search alone; switched off, an agent gets "
                    "neither tool.",
    (
        "with only a local endpoint connected, an agent is offered read_page and not a search nothing can run",
        "a page at a cloud metadata address is refused on the host, and nothing was sent anywhere",
        "Settings says this endpoint cannot search, and where a key would send a query",
        "with a Brave Search key saved, Settings names Brave Search as where a query goes, and shows only its end",
        "once a key is set the agent is offered web_search, and the query goes to Brave Search and nowhere else",
        "an agent switched off the web is offered neither web tool",
        "every model call was one the scenario scripted",
    )),
    "settings": Scenario(scenario_settings, "A person opens Settings from the gear as a page in the window, with the "
                         "roster kept and a back control, and its thirteen sections listed without scrolling at "
                         "1280 by 820 and 900 by 700; a section scrolls on its own with Save kept on screen, Escape, "
                         "the back control and Command-comma each return to where it was opened with the focus back, "
                         "the command menu opens every section straight to itself, and New agent, New group and a "
                         "group's settings fit a 900 by 700 window with their actions on screen; the selected section "
                         "has a fill of its own, and the list still fits with the no-key banner, the host update "
                         "notice and the unreachable host banner showing.",
    (
        "the gear opens Settings as a full-width page with a back control, and no dialog",
        "at 1280x820 and 900x700 the section list shows all thirteen sections without scrolling",
        "a long section scrolls on its own, and the section list stays where it was",
        "with an unsaved change, Save stays on screen after scrolling to the end of a section",
        "Escape, the back control and Command-comma each return to where Settings was opened, with the focus where it was",
        "the command menu opens every section straight to itself, from a conversation and from Settings",
        "opened from an agent's page, the back control is named for that agent and returns to it",
        "New agent, New group and a group's settings fit a 900x700 window with their actions on screen",
        "the selected section has a fill of its own against the page and keeps its heavier weight",
        "with the no-key banner, then the host update notice, then the unreachable host banner showing, "
        "the section list still shows all thirteen sections without scrolling at 1280x820 and 900x700",
        "every model call was one the scenario scripted",
    )),
    "tabs": Scenario(scenario_tabs, "A person opens Settings from the keyboard, reaches the tab list in one Tab, "
                     "moves between tabs with the arrow keys and Tabs into the open panel; the list is a single "
                     "Tab stop in both directions.",
    (
        "the first Tab stop in Settings is the open tab",
        "an arrow key moves to the next tab and opens it",
        "Home and End reach the first and the last tab",
        "Appearance is reached and opened with arrow keys",
        "Tab goes from the tab list into the open panel",
        "the tab list is one Tab stop",
        "every model call was one the scenario scripted",
    )),
    "connect": Scenario(scenario_connect, "A person connects a Custom endpoint with the key, endpoint and model "
                        "fields on screen at 900 by 700 and 1280 by 820 and no other provider's model suggested, and "
                        "an endpoint that needs no key passes with none; a rejected key is said in plain words at the "
                        "key field and clears as it is retyped, and a refusal for a missing key is reported beside "
                        "the buttons and is about that endpoint rather than another provider or place.",
    (
        "the key, endpoint and model fields are on screen without scrolling at 900x700 and 1280x820",
        "a Custom endpoint suggests no other provider's model",
        "a rejected key is said at the key field in plain words, as an error that clears on retyping",
        "with no key, the refusal is about this endpoint and names no other provider or place",
        "an endpoint that needs no key passes the test with none, and is kept",
        "every model call was one the scenario scripted",
    )),
    "search": Scenario(scenario_search, "Find opens files and the command menu opens messages and actions; "
                       "Find cannot run an action and dialogs share their geometry.", (
        "Find only offers messages and files and cannot run New agent",
        "Find opens the file's message without running an app action",
        "the file reader covers the window without clipping and Find stacks above it",
        "the Groups sheet uses the command menu and a message hit opens without another palette",
        "dialogs and sheets share one top offset, two widths, and a lighter light scrim",
        "Calendar and Artifacts explain their empty lists without repeating the description",
    )),
    "menu": Scenario(scenario_menu, "A person in a conversation finds the focus in the composer, opens the command menu on "
                     "Command K, switches to dark with one word and Return, opens a message by a word in it with "
                     "Return, writes to an agent with a sentence only on Command Return, and opens the first agent on "
                     "Command 1 with the focus in its composer; a sentence left in the menu by Escape is not sent.",
    (
        "the conversation puts the focus in the composer",
        "one word, dark, and Return switch the reading surface to dark",
        "Escape closes the menu with a sentence in it, sends nothing and returns to the composer",
        "one word and Return open the message that holds it and send nothing",
        "Return never writes a sentence, and Command Return writes it to the agent, whose reply arrives",
        "Command 1 opens the first agent with the focus in its composer",
        "every model call was one the scenario scripted",
    )),
    "queue": Scenario(scenario_queue, "Reload opens the earliest waiting conversation without replacing a run. "
                      "Questions selects the first card, J and K move, Command Return allows, a number picks an "
                      "answer, and focus returns to the composer; Escape sends nothing, and Command Z takes back a held Allow.",
    (
        "reload opens the earliest waiting conversation without changing either run",
        "Escape closes Questions without answering or switching conversations",
        "Questions selects the first request and shows the keys on its buttons",
        "J and K move the selection, and an option shows its number only while selected",
        "Tab brings the answer and its selected card into agreement",
        "Command Return cannot authorize an earlier permission from a later question",
        "Command Return holds the Allow with an Undo, sends nothing yet and selects the next request",
        "Command Z takes the Allow back: the card returns and nothing was sent",
        "the held answers are each sent once, and the agents are told",
        "after the last answer Questions closes and the original composer gets its draft and focus back",
        "both answers finish their original runs",
        "after both runs settle, reload returns to the last conversation and its draft",
        "every model call was one the scenario scripted",
    )),
}
