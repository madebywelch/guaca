
from __future__ import annotations

import json
import re
import sqlite3
import threading
import urllib.request
from dataclasses import dataclass
from typing import Callable

from . import look
from .browser import label
from .world import STAND_IN_KEY, STAND_IN_MODEL, Call, Reply, World, call, held, refuse, say, text_of

AGENT = "Chief of Staff"
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
    if ui.find("combobox", "Recipient") and ui.value("combobox", "Recipient") != to:
        open_agent(w, to)
    ui.fill("combobox", f"Write to {to}", text)
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
        n for n in w.ui.within("list", "Today's timeline", "button") if label(n).startswith(f"{name}, ")])
    w.ui.press(face[0])
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


LOOK_SURFACES = ("light", "dark")
LOOK_SCREENS = ("Today", "a conversation", "Settings", "New agent", "the Groups sheet", "Search",
                "the command menu")
AGENT_TABS = ("Now", "Memory", "Notebook", "Routines", "Profile")
SETTINGS_TABS = ("Host", "Provider", "Limits", "Permissions", "Machines", "Account", "Appearance",
                 "Notifications", "Shortcuts", "Skills", "Compost", "About")
PLACES = ("Artifacts", "Calendar", "For you, 1 question")


def show(w: World, screen: str) -> None:
    ui = w.ui
    if screen == "Today":
        if ui.find("button", "Today"):
            ui.click("button", "Today")
        w.wait("Today is shown", lambda: ui.find("region", "Today"))
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
    timeline: !!document.querySelector('.ribbon'),
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
    ui.key("k", mod=True)
    w.wait("the command menu opens", lambda: ui.find("dialog", "Command menu"))
    ui.type("textbox", "Command menu", f"Settings: {section}")
    w.wait(f"Settings: {section} is chosen", lambda: menu_choice(w) == f"Settings: {section}")
    ui.key("Enter")
    w.wait("the menu closes", lambda: not ui.find("dialog", "Command menu"))


def scenario_settings(w: World) -> None:
    w.model.script = hello
    w.start()
    first_run(w)
    settled(w, 1)
    ui = w.ui
    show(w, "Today")

    ui.viewport(*look.FRAME)
    gear = ui.one("button", "App settings")
    ui.press(gear)
    w.wait("Settings opens", lambda: ui.find("region", "Settings"))
    page = ui.evaluate(SETTINGS_PAGE)
    w.check("the gear opens Settings as a page in the window, with the timeline and a back control, and no dialog",
            not ui.find("dialog", "", exact=False) and not page["inDialog"] and page["timeline"]
            and bool(ui.find("button", "Today")) and section_shown(w) == "General",
            observed={"dialogs": ui.names("dialog"), "inDialog": page["inDialog"], "timeline": page["timeline"],
                      "back": bool(ui.find("button", "Today")), "section": section_shown(w)}, shot=True)

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
            ui.click("button", "Today")
        else:
            ui.key(",", mod=True)
        w.wait(f"{leave} leaves Settings", lambda: not ui.find("region", "Settings"))
        returns[leave] = {"today": bool(ui.find("region", "Today")),
                          "focus": w.wait("the focus comes back", lambda: ui.focused()
                                          if ui.focused() == ("button", "App settings") else None)}
    ui.key(",", mod=True)
    opened = bool(w.wait("Command-comma opens Settings", lambda: ui.find("region", "Settings")))
    ui.key(",", mod=True)
    w.wait("Command-comma again leaves it", lambda: not ui.find("region", "Settings"))
    w.check("Escape, the back control and Command-comma each return to where Settings was opened, with the focus where it was",
            opened and all(r["today"] and r["focus"] == ("button", "App settings") for r in returns.values()),
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
    w.check("the command menu opens every section straight to itself, from Today and from the page",
            all(straight[s] == s for s in SETTINGS_SECTIONS) and all(within[s] == s for s in SETTINGS_SECTIONS),
            observed={"fromToday": straight, "fromThePage": within})

    open_agent(w, AGENT)
    ui.key(",", mod=True)
    w.wait("Settings opens over the agent's page", lambda: ui.find("region", "Settings"))
    gone = not ui.find("log", f"Conversation with {AGENT}")
    ui.click("button", AGENT)
    w.wait("the back control leaves Settings", lambda: not ui.find("region", "Settings"))
    w.check("opened from an agent's page, the back control is named for that agent and returns to it",
            gone and bool(ui.find("log", f"Conversation with {AGENT}")),
            observed={"replaced": gone, "back": bool(ui.find("log", f"Conversation with {AGENT}"))}, shot=True)

    show(w, "Today")
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
    if not w.ui.find("region", "Running", exact=False):
        return ""
    return w.ui.evaluate("document.querySelector('.home__running')?.innerText ?? ''") or ""


def running_for(w: World) -> int | None:
    found = re.search(r"(?:(\d+)m )?(\d+)s", running_text(w))
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
    show(w, "Today")
    w.check("with nothing running, Today has no Running section",
            not w.ui.find("region", "Running", exact=False), observed=running_text(w))
    w.ui.click("button", f"{AGENT}, ", exact=False)
    w.wait(f"{AGENT}'s conversation opens", lambda: w.ui.find("log", f"Conversation with {AGENT}"))
    write(w, AGENT, LOOK_INTO)
    show(w, "Today")
    w.wait("Today lists the conversation as running", lambda: READING in running_text(w))
    first = running_for(w)
    marks = started_marks(w)
    w.check("Running shows the agent, its latest note, the time since the Host started the work, and Stop",
            AGENT in running_text(w) and first is not None and len(marks) == 1 and marks[0] is not None
            and w.ui.find("button", f"Stop {AGENT}'s conversation"),
            observed={"agent": AGENT in running_text(w), "note": READING in running_text(w),
                      "clock": first is not None, "marked": [m is not None for m in marks]},
            detail=running_text(w), shot=True)
    w.wait("the clock ticks past three seconds", lambda: (running_for(w) or 0) >= 3)
    w.check("the clock on Running ticks while the turn works", (running_for(w) or 0) > (first or 0),
            observed=True, detail={"first": first, "now": running_for(w)})

    held_for = running_for(w) or 0
    w.host.stop()
    before.set()
    w.host.start()
    w.open_browser()
    show(w, "Today")
    w.wait("the next Host picks the turn up", picked.is_set)
    w.wait("Today lists the conversation again", lambda: AGENT in running_text(w))
    again = running_for(w)
    w.check("after a restart mid-turn the clock goes on from the first Host's start rather than from zero",
            started_marks(w) == marks and again is not None and again >= held_for,
            observed={"sameStart": started_marks(w) == marks, "carriedOn": again is not None and again >= held_for},
            detail={"before": held_for, "after": again, "marks": marks}, shot=True)

    w.ui.click("button", f"Stop {AGENT}'s conversation")
    after.set()
    ended = settled(w, 2, "the stopped conversation settles")
    w.wait("Running leaves Today once the conversation ends", lambda: not running_text(w))
    w.check("Stop ends the conversation, and it leaves Running for Finished",
            ended[-1]["status"] == "stopped" and not w.ui.find("region", "Running", exact=False)
            and w.ui.find("region", "Finished", exact=False),
            observed={"outcomes": ended}, shot=True)
    no_unscripted(w)


ASK_BOTH = "Ask Researcher and Analyst what changed in the quarterly filings."
ASK_AGAIN = "Ask them both about the lease note."
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
    return w.ui.evaluate("[...document.querySelectorAll('.handoff__card')].at(-1)?.innerText ?? ''") or ""


def working_line(w: World) -> str:
    return w.ui.evaluate("document.querySelector('.working')?.innerText ?? ''") or ""


def scenario_handoff(w: World) -> None:
    analyst = threading.Event()
    never = threading.Event()

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
        if c.speaker == "Researcher" and (FILINGS_Q in c.last_user or LEASE_Q in c.last_user):
            return say(R_SAYS)
        if c.speaker == "Analyst" and FILINGS_Q in c.last_user:
            return held(analyst, say(A_SAYS))
        if c.speaker == "Analyst" and LEASE_Q in c.last_user:
            return held(never, say(A_SAYS))
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
    w.wait("the card shows Researcher's answer while Analyst works", lambda: R_SAYS in card_text(w)
           and "writing" in card_text(w))
    answer_px = ui.evaluate("getComputedStyle(document.querySelector('.handoff__answer .md')).fontSize")
    faces = ui.evaluate("document.querySelectorAll('.handoff__card .avatar').length")
    w.check("a live card shows each colleague's face, the question, a status word with time, and the answer at reading size",
            ui.find("region", "Asked 2 colleagues") and faces == 2 and card_text(w).count(FILINGS_Q) == 2
            and "answered in" in card_text(w) and re.search(r"writing, \d+s", card_text(w))
            and answer_px == "16px",
            observed={"faces": faces, "questions": card_text(w).count(FILINGS_Q), "answerSize": answer_px,
                      "answered": "answered in" in card_text(w)},
            detail=card_text(w), shot=True)
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
    for surface in LOOK_SURFACES:
        ui.evaluate("localStorage.setItem('guac.prefs', JSON.stringify({...JSON.parse("
                    "localStorage.getItem('guac.prefs') || '{}'), surface: '%s', look: 2})); location.reload()"
                    % surface)
        w.wait(f"the page comes back {surface}", lambda: ui.evaluate(
            "document.documentElement.dataset.surface") == surface and ui.find("button", "App settings"))
        ui.viewport(*look.FRAME)
        open_agent(w, AGENT)
        w.wait("the live card is drawn again", lambda: "writing" in card_text(w))
        measure(w, f"a hand-off in {surface}", surface, seen)
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

    analyst.set()
    w.wait("the card folds once both have answered", lambda: ui.find(
        "button", "Asked Researcher and Analyst, both answered", exact=False))
    w.check("after the reply the hand-off folds to one line that says both answered",
            not ui.find("region", "Asked 2 colleagues") and BOTH_SAID in conversation(w),
            observed={"folded": True, "card": bool(ui.find("region", "Asked 2 colleagues"))}, shot=True)
    ui.click("button", "Asked Researcher and Analyst, both answered", exact=False)
    w.check("the folded line opens onto both answers",
            R_SAYS in card_text(w) and A_SAYS in card_text(w), observed=True, detail=card_text(w))
    ui.click("button", "Asked Researcher and Analyst, both answered", exact=False)
    w.wait("the opened hand-off folds again", lambda: not ui.find("region", "Asked 2 colleagues"))

    settled(w, 2)
    write(w, AGENT, ASK_AGAIN)
    w.wait("the second hand-off is live", lambda: card_text(w).count(LEASE_Q) == 2 and R_SAYS in card_text(w))
    w.wait("the status line offers Stop", lambda: ui.find("button", "Stop"))
    ui.click("button", "Stop")
    never.set()
    ended = settled(w, 3, "the stopped conversation settles")
    w.wait("the stopped hand-off folds", lambda: ui.find(
        "button", "Asked Researcher and Analyst, Researcher answered", exact=False))
    w.check("a hand-off cut short by Stop folds and says who answered, instead of ticking on",
            ended[-1]["status"] == "stopped" and not ui.find("region", "Asked 2 colleagues")
            and not ui.find("button", "Stop"),
            observed={"outcome": ended[-1]["status"]}, shot=True)
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
            and w.host.scalar("SELECT count(*) FROM agents") == 3,
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


IDLE = ("Researcher", "Critic", "Archivist")
NAMES_APART = ("(() => { const r = [...document.querySelectorAll('.ribbon__name')]"
               ".map(e => e.getBoundingClientRect()); for (let i = 0; i < r.length; i++)"
               " for (let j = i + 1; j < r.length; j++) { const a = r[i], b = r[j];"
               " if (a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom)"
               " return false; } return r.length > 0; })()")
SEEN = ("((name) => { const face = [...document.querySelectorAll('.ribbon__face')]"
        ".find(f => f.querySelector('.ribbon__name').textContent === name);"
        " const through = (e) => { let o = 1; for (; e; e = e.parentElement)"
        " o *= Number(getComputedStyle(e).opacity); return o; };"
        " return { name: through(face.querySelector('.ribbon__name')),"
        " portrait: through(face.querySelector('.ribbon__portrait')) }; })(%s)")


def timeline(w: World) -> list[str]:
    return w.ui.names_within("list", "Today's timeline", "button")


def scenario_ribbon(w: World) -> None:
    w.model.script = hello
    w.start()
    w.ui.viewport(*WINDOW)
    first_run(w)
    settled(w, 1)
    ui = w.ui
    for name in IDLE:
        new_agent(w, name, "Wait to be asked.")
    w.wait("every new agent has a face on today's timeline",
           lambda: all(any(n.startswith(f"{name}, ") for n in timeline(w)) for name in IDLE))
    idle = [n for n in timeline(w) if n.split(", ")[0] in IDLE]
    w.check("an agent that has done nothing today has a face of its own, named and called idle",
            sorted(idle) == sorted(f"{name}, idle" for name in IDLE), observed=sorted(idle), shot=True)
    w.check("no face on the timeline is called asleep, and there is no pile",
            not [n for n in timeline(w) if "asleep" in n] and not ui.evaluate(
                "document.querySelector('.ribbon__pile') !== null"),
            observed=[n for n in timeline(w) if "asleep" in n], detail=timeline(w))
    w.check("at 1280 wide no two names on the timeline overlap", bool(ui.evaluate(NAMES_APART)),
            observed=bool(ui.evaluate(NAMES_APART)))

    open_agent(w, AGENT)
    seen = ui.evaluate(SEEN % json.dumps(IDLE[0]))
    w.check("with another agent's page open, an idle face fades its drawing and not its name",
            seen["name"] == 1 and seen["portrait"] < 1, observed=seen, shot=True)
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
    ui.click("heading", group)
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


def group_clear(w: World) -> bool:
    return bool(w.ui.evaluate(
        "(() => { const pane = document.querySelector('.onboarding'); const bar = document.querySelector('.onboarding-actions');"
        " if (!pane || !bar) return false; const top = pane.getBoundingClientRect().top, floor = bar.getBoundingClientRect().top;"
        " const rows = [...document.querySelectorAll('.onboarding-group li')];"
        " return rows.length === 3 && rows.every(li => { const r = li.getBoundingClientRect();"
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
    fields = {name: bool(ui.find("textbox", name, exact=False)) and ui.on_screen("textbox", name, exact=False)
              for name in ("API key", "Inference endpoint", "Default model")}
    w.check("the key, endpoint and model fields are on screen without scrolling", all(fields.values()),
            observed=fields, shot=True, stop=False)

    model = {"value": ui.property("textbox", "Default model", "value"),
             "placeholder": ui.property("textbox", "Default model", "placeholder")}
    w.check("a Custom endpoint suggests no other provider's model",
            not model["value"] and "anthropic/" not in model["placeholder"], observed=model, shot=True, stop=False)

    ui.fill("textbox", "Inference endpoint", w.model.base)
    ui.fill("textbox", "Default model", NEEDS_KEY)
    ui.fill("textbox", "API key", WRONG_KEY, exact=False)
    said_, shown = refusal(w, "the endpoint refuses the wrong key, and the form says so")
    w.check("a refusal is shown beside the buttons, not under them", shown,
            observed={"onScreen": shown}, detail=said_, shot=True)

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

    def script(c: Call) -> Reply | None:
        if c.speaker == AGENT and c.last_user.endswith(QUIET_PLEASE):
            return say(NOTED)
        return hello(c)

    w.model.script = script
    w.start()
    first_run(w)
    settled(w, 1)
    ui = w.ui

    ui.click("button", "Today")
    w.wait("Today is shown", lambda: ui.find("region", "Today"))
    w.check("the focus is in the field on Today",
            w.wait("the field takes the focus", lambda: ui.focused() == ("textbox", "What do you need?")),
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
    w.check("Escape closes the menu with a sentence in it, sends nothing and gives the field back",
            w.wait("the menu closes", lambda: not ui.find("dialog", "Command menu"))
            and w.wait("the field takes the focus back", lambda: ui.focused() == ("textbox", "What do you need?"))
            and len(said(w, AGENT)) == before and not w.host.scalar(
                "SELECT count(*) FROM messages WHERE parts LIKE ?", f"%{QUIET_PLEASE}%"),
            observed={"focused": ui.focused(), "said": len(said(w, AGENT)) - before})

    ui.key("k", mod=True)
    w.wait("the command menu opens a third time", lambda: ui.find("dialog", "Command menu"))
    ui.type("textbox", "Command menu", QUIET_PLEASE)
    w.wait("three words or more select writing them", lambda: menu_choice(w) == f"Write to {AGENT}")
    ui.key("Enter")
    w.wait("the reply arrives", lambda: NOTED in said(w, AGENT))
    w.check("three words or more and Return write to the agent, and its reply arrives",
            said(w, AGENT)[-1] == NOTED and not ui.find("dialog", "Command menu"),
            observed={"said": said(w, AGENT)[-2:]})

    ui.key("1", mod=True)
    w.wait("Command 1 opens the first agent", lambda: ui.find("log", f"Conversation with {AGENT}"))
    w.check("Command 1 opens the first agent with the focus in its composer",
            w.wait("the composer takes the focus", lambda: ui.focused() == ("combobox", f"Write to {AGENT}")),
            observed=ui.focused(), shot=True)
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
    w.wait("the question is pending too", lambda: len(approvals(w)) == 2)

    ui.click("button", "Today")
    w.wait("Today shows both requests", lambda: ui.find("button", "Allow") and ui.find("button", "Early"))
    w.wait("the field takes the focus", lambda: ui.focused() == ("textbox", "What do you need?"))
    ui.key("Escape")
    first = w.wait("Escape selects the first card", lambda: selected_card(w))
    w.check("Escape leaves the field and selects the first request, with its keys on its buttons",
            ui.focused()[0] == "article" and ui.focused()[1].startswith(AGENT) and "Allow" in first
            and ("⌘↩" in first or "Ctrl+↩" in first),
            observed={"focused": ui.focused(), "card": first}, shot=True)

    ui.key("j")
    second = w.wait("J selects the next card", lambda: RESEARCHER in ui.focused()[1] and selected_card(w))
    ui.key("k")
    w.check("J and K move the selection, and an option shows its number only while selected",
            "Early1" in re.sub(r"\s+", "", second)
            and w.wait("K goes back", lambda: ui.focused()[1].startswith(AGENT)),
            observed={"second": second, "focused": ui.focused()})

    ui.key("Enter", mod=True)
    w.wait("the Allow is held with an Undo", lambda: ui.find("button", "Undo", exact=False))
    w.check("Command Return holds the Allow with an Undo, sends nothing yet and selects the next request",
            [a["state"] for a in approvals(w)] == ["pending", "pending"] and RESEARCHER in ui.focused()[1],
            observed={"approvals": approvals(w), "focused": ui.focused()}, shot=True)
    ui.key("z", mod=True)
    w.check("Command Z takes the Allow back: the card returns and nothing was sent",
            w.wait("the card returns", lambda: len(ui.find("button", "Allow")) == 1
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
    w.check("after the last answer the focus is back in the field",
            w.wait("the field takes the focus back", lambda: ui.focused() == ("textbox", "What do you need?")),
            observed=ui.focused(), shot=True)
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


def scenario_firstgroup(w: World) -> None:
    researcher = threading.Event()
    analyst = threading.Event()

    def script(c: Call) -> Reply | None:
        if c.last_user == "ping":
            return say("pong")
        if c.speaker == LEAD:
            if ANALYST_SAYS in c.last_user:
                return say(CAR_RESULT)
            if RESEARCHER_SAYS in c.last_user:
                return say("Market Researcher answered.")
            if "buy or lease my next car" in c.last_user:
                return say("I asked them both.") if c.tool_results() else call(
                    "send_message", to=[MARKET, FINANCE], text=CAR_Q, intent="work")
        if c.speaker == MARKET and CAR_Q in c.last_user:
            return held(researcher, say(RESEARCHER_SAYS))
        if c.speaker == FINANCE and CAR_Q in c.last_user:
            return held(analyst, say(ANALYST_SAYS))
        return None

    w.model.script = script
    w.start()
    ui = w.ui
    ui.viewport(*WINDOW)
    w.wait("the first visit offers a first task", lambda: ui.find("button", GROUP_TASK))
    w.check("the first step says a task goes to a group of three and keeps Create one agent",
            "hires a group of three" in ui.text() and bool(ui.find("button", "Create one agent")),
            observed={"group": "hires a group of three" in ui.text(),
                      "alone": bool(ui.find("button", "Create one agent"))}, shot=True)
    ui.click("button", GROUP_TASK)
    brief = ui.value("textbox", "Your first brief")
    w.check("the suggested brief asks both colleagues at once and does not open by asking the person",
            f"Ask {MARKET} and {FINANCE} at the same time" in brief
            and not brief.lower().startswith("ask me") and brief.lower().count("ask me") == 1,
            observed=brief)
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
    w.check("the Start step names the three it will hire and what each is for, before any is hired",
            bool(ui.find("region", "Your group for this task")) and [row.split(" ")[0] for row in listed]
            == ["Chief", "Market", "Financial"] and all(len(row.split(" ")) > 4 for row in listed)
            and not w.host.scalar("SELECT count(*) FROM agents"),
            observed=listed, shot=True)
    clear = {}
    for width, height in ((900, 700), WINDOW):
        ui.viewport(width, height)
        clear[f"{width}x{height}"] = group_clear(w)
    w.check("the three colleagues and what each is for clear the pinned bar at 900x700 and 1280x820",
            all(clear.values()), observed=clear)

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
        shown[f"{width}x{height}"] = {"alert": alert_shown(w), "group": group_clear(w)}
    w.check("a Start that fails says why beside Start, on screen without scrolling at 900x700 and 1280x820",
            all(seen["alert"] and seen["group"] for seen in shown.values()), observed=shown, shot=True)
    w.host.start()
    w.wait("the page is back with the Host", lambda: ui.find("button", "App settings"))
    w.wait("Start task is offered again", lambda: ui.find("button", "Start task")
           and not ui.property("button", "Start task", "disabled"))
    ui.click("button", "Start task")
    w.wait("Start task opens the coordinator's conversation", lambda: ui.find("log", f"Conversation with {LEAD}"))
    hired = sorted(r["name"] for r in w.host.rows("SELECT name FROM agents"))
    w.check("Start hires the group of three exactly once",
            hired == sorted([LEAD, MARKET, FINANCE]), observed=hired)
    w.wait("the coordinator's hand-off is live", lambda: card_text(w).count(CAR_Q) == 2)
    faces = ui.evaluate("document.querySelectorAll('.handoff__card .avatar').length")
    talk = between(w)
    w.check("the coordinator asks both colleagues on its first turn, in one message, and the person sees the card",
            [(m["from"], m["to"], m["text"]) for m in talk]
            == [(LEAD, MARKET, CAR_Q), (LEAD, FINANCE, CAR_Q)] and faces == 2
            and bool(ui.find("region", "Asked 2 colleagues")),
            observed={"talk": [(m["from"], m["to"]) for m in talk], "faces": faces}, shot=True)
    w.wait("the status line says who it waits for", lambda: "waiting for" in working_line(w))
    w.check("under the coordinator a status line names both colleagues it waits for",
            working_line(w).startswith(f"Messaged {MARKET} and {FINANCE}, waiting for"),
            observed=re.sub(r"\d+s", "Ns", working_line(w)))
    show(w, "Today")
    w.wait("Today lists the coordinator's conversation as running", lambda: LEAD in running_text(w))
    w.check("behind the conversation, Today shows the group's work as running",
            LEAD in running_text(w), observed=LEAD in running_text(w), shot=True)
    open_agent(w, LEAD)

    researcher.set()
    w.wait("the card shows Market Researcher's answer", lambda: RESEARCHER_SAYS in card_text(w))
    analyst.set()
    w.wait("the result reaches the person", lambda: CAR_RESULT in conversation(w))
    ended = settled(w, 1, "the group's conversation settles")
    w.check("the coordinator comes back with one result after both answered, and the run ends there",
            said(w, LEAD)[-1] == CAR_RESULT and ended[-1]["status"] == "done"
            and [m["from"] for m in between(w)].count(MARKET) == 1
            and [m["from"] for m in between(w)].count(FINANCE) == 1,
            observed={"outcome": ended[-1]["status"], "colleagues": sorted(m["from"] for m in between(w)
                                                                             if m["from"] != LEAD)},
            detail={"modelCalls": len(w.model.calls), "said": said(w, LEAD)}, shot=True)
    no_unscripted(w)


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
        "Start task opens the conversation while the agent is still answering",
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
    "running": Scenario(scenario_running, "Today lists a conversation while it runs, with the agent, its latest note, "
                        "a clock from when the Host started the work and Stop; a Host restarted mid-turn keeps the "
                        "clock going from the first start, and Stop moves the conversation to Finished.",
    (
        "with nothing running, Today has no Running section",
        "Running shows the agent, its latest note, the time since the Host started the work, and Stop",
        "the clock on Running ticks while the turn works",
        "after a restart mid-turn the clock goes on from the first Host's start rather than from zero",
        "Stop ends the conversation, and it leaves Running for Finished",
        "every model call was one the scenario scripted",
    )),
    "handoff": Scenario(scenario_handoff, "An agent asks two colleagues; a live card shows each face, the question, a "
                        "status word with time and the answer at reading size, the status line says who it waits "
                        "for with Stop, and the transcript keeps two text edges; after the replies the card folds "
                        "to one line; a hand-off cut short by Stop folds and says who answered.",
    (
        "a live card shows each colleague's face, the question, a status word with time, and the answer at reading size",
        "under the running turn a status line says what it is doing, for how long, with Stop",
        "text in the transcript starts on two left edges",
        "the person's turn sits in a bubble that stands apart from the page",
        "a turn scrolled under the tabs fades out instead of being cut by them",
        "the hand-off keeps to the six type roles, the control family and the radii in light and dark",
        "an agent opened mid-turn shows its status line without scrolling",
        "after the reply the hand-off folds to one line that says both answered",
        "the folded line opens onto both answers",
        "a hand-off cut short by Stop folds and says who answered, instead of ticking on",
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
    "look": Scenario(scenario_look, "Today, a conversation, Settings, New agent, the Groups sheet and Search, in "
                     "light and dark, each keep to the six type roles, eight styles at most, the control heights "
                     "and the radii; a screen with a size, weight, corner or control outside them fails, naming "
                     "the element and the token to use.",
    (
        "every screen in light keeps to the six type roles, the control family and the radii",
        "every screen in dark keeps to the six type roles, the control family and the radii",
        "every model call was one the scenario scripted",
    )),
    "scrollbars": Scenario(scenario_scrollbars, "Today, a conversation, Settings, New agent, the Groups sheet, "
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
                           "three agents it will hire and what each is for, clear of the pinned bar; a Start that cannot reach the Host "
                           "says why beside Start, on screen, and hires nobody; Start then hires the three exactly once and opens the coordinator's "
                           "conversation, where the coordinator asks both colleagues at once, the live hand-off "
                           "card and status line show it, Today shows it running, and one result comes back.",
    (
        "the first step says a task goes to a group of three and keeps Create one agent",
        "the suggested brief asks both colleagues at once and does not open by asking the person",
        "the Start step names the three it will hire and what each is for, before any is hired",
        "the three colleagues and what each is for clear the pinned bar at 900x700 and 1280x820",
        "a Start that fails hires nobody and sends nothing, and keeps the brief",
        "a Start that fails says why beside Start, on screen without scrolling at 900x700 and 1280x820",
        "Start hires the group of three exactly once",
        "the coordinator asks both colleagues on its first turn, in one message, and the person sees the card",
        "under the coordinator a status line names both colleagues it waits for",
        "behind the conversation, Today shows the group's work as running",
        "the coordinator comes back with one result after both answered, and the run ends there",
        "every model call was one the scenario scripted",
    )),
    "newagent": Scenario(scenario_newagent, "A person opens New agent in a 1280 by 820 window and creates an agent "
                         "with a keyboard press on its one Create agent button, which is on screen without "
                         "scrolling; a press with no name creates nothing.",
    (
        "Create agent is one button, on screen without scrolling",
        "with no name, a press on Create agent creates nothing and keeps the dialog",
        "a keyboard press on Create agent creates the agent and closes the dialog",
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
    "ribbon": Scenario(scenario_ribbon, "Three agents that have done nothing today each get a face of their own "
                       "on the timeline, named and called idle, with no pile and no name over another at 1280 "
                       "wide; while another agent's page is open their drawings fade and their names do not.",
    (
        "an agent that has done nothing today has a face of its own, named and called idle",
        "no face on the timeline is called asleep, and there is no pile",
        "at 1280 wide no two names on the timeline overlap",
        "with another agent's page open, an idle face fades its drawing and not its name",
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
    "settings": Scenario(scenario_settings, "A person opens Settings from the gear as a page in the window, with the "
                         "timeline kept and a back control, and its thirteen sections listed without scrolling at "
                         "1280 by 820 and 900 by 700; a section scrolls on its own with Save kept on screen, Escape, "
                         "the back control and Command-comma each return to where it was opened with the focus back, "
                         "the command menu opens every section straight to itself, and New agent, New group and a "
                         "group's settings fit a 900 by 700 window with their actions on screen; the selected section "
                         "has a fill of its own, and the list still fits with the no-key banner, the host update "
                         "notice and the unreachable host banner showing.",
    (
        "the gear opens Settings as a page in the window, with the timeline and a back control, and no dialog",
        "at 1280x820 and 900x700 the section list shows all thirteen sections without scrolling",
        "a long section scrolls on its own, and the section list stays where it was",
        "with an unsaved change, Save stays on screen after scrolling to the end of a section",
        "Escape, the back control and Command-comma each return to where Settings was opened, with the focus where it was",
        "the command menu opens every section straight to itself, from Today and from the page",
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
    "connect": Scenario(scenario_connect, "A person connects a Custom endpoint in a 1280 by 820 window with the "
                        "key, endpoint and model fields on screen and no other provider's model suggested, and an "
                        "endpoint that needs no key passes with none; a refusal is reported beside the buttons, and "
                        "one for a missing key is about that endpoint rather than another provider or place.",
    (
        "the key, endpoint and model fields are on screen without scrolling",
        "a Custom endpoint suggests no other provider's model",
        "a refusal is shown beside the buttons, not under them",
        "with no key, the refusal is about this endpoint and names no other provider or place",
        "an endpoint that needs no key passes the test with none, and is kept",
        "every model call was one the scenario scripted",
    )),
    "menu": Scenario(scenario_menu, "A person on Today finds the focus in the field, opens the command menu on "
                     "Command K, switches to dark with one word and Return, writes to an agent with a sentence and "
                     "Return, and opens the first agent on Command 1 with the focus in its composer; a sentence "
                     "left in the menu by Escape is not sent.",
    (
        "the focus is in the field on Today",
        "one word, dark, and Return switch the reading surface to dark",
        "Escape closes the menu with a sentence in it, sends nothing and gives the field back",
        "three words or more and Return write to the agent, and its reply arrives",
        "Command 1 opens the first agent with the focus in its composer",
        "every model call was one the scenario scripted",
    )),
    "queue": Scenario(scenario_queue, "Two agents wait on a person, who answers both from Today by keyboard: "
                      "Escape selects the first card, J and K move, Command Return allows, a number picks an answer, "
                      "and the focus returns to the field; an Allow taken back with Command Z is never sent.",
    (
        "Escape leaves the field and selects the first request, with its keys on its buttons",
        "J and K move the selection, and an option shows its number only while selected",
        "Command Return holds the Allow with an Undo, sends nothing yet and selects the next request",
        "Command Z takes the Allow back: the card returns and nothing was sent",
        "the held answers are each sent once, and the agents are told",
        "after the last answer the focus is back in the field",
        "every model call was one the scenario scripted",
    )),
}
