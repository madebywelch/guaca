import http.client
import importlib.util
import inspect
import io
import json
import socket
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock
import urllib.error
import urllib.request
from pathlib import Path

from e2e.browser import Browser, BrowserError, split_frames
from e2e.scenarios import SCENARIOS, unproved
from e2e.world import (UNSCRIPTED, Call, Egress, Failed, Model, Updater, call, held, redact, refuse, say,
                       stream, until)


def post(base: str, body: dict, key: str = "k") -> tuple[int, bytes]:
    request = urllib.request.Request(f"{base}/chat/completions", data=json.dumps(body).encode(),
                                     headers={"content-type": "application/json",
                                              "authorization": f"Bearer {key}"}, method="POST")
    try:
        with urllib.request.urlopen(request, timeout=10) as answer:
            return answer.status, answer.read()
    except urllib.error.HTTPError as error:
        return error.code, error.read()


def conversation(*messages: tuple[str, str]) -> dict:
    return {"messages": [{"role": role, "content": text} for role, text in messages]}


class Rendering(Browser):
    def __init__(self, appears_after: int | None, attached: bool = True, lands: bool = True) -> None:
        self.appears_after = appears_after
        self.attached = attached
        self.lands = lands
        self.looks = 0
        self.sent: list[tuple[str, str]] = []

    def ax_nodes(self) -> list[dict]:
        self.looks += 1
        shown = self.appears_after is not None and self.looks > self.appears_after
        return [{"role": {"value": "button"}, "name": {"value": "Deny"}, "backendDOMNodeId": 7}] if shown else []

    def _center(self, node: dict) -> tuple[float, float]:
        return 1.0, 1.0

    def _arm(self, node: dict, x: float, y: float) -> str:
        return "armed" if self.attached else "gone"

    def _landed(self, token: str) -> bool:
        return self.lands

    def send(self, method: str, params: dict | None = None, session: bool = True, timeout: float = 30) -> dict:
        self.sent.append((method, (params or {}).get("type", "")))
        return {}


class Pipe(unittest.TestCase):
    def test_a_message_split_across_reads_is_held_until_its_end(self) -> None:
        whole, rest = split_frames(b'{"id":1}\0{"id"')
        self.assertEqual(whole, [b'{"id":1}'])
        whole, rest = split_frames(rest + b':2}\0')
        self.assertEqual((whole, rest), ([b'{"id":2}'], b""))

    def test_names_within_a_dialog_leave_out_the_same_names_behind_it(self) -> None:
        page = Rendering(appears_after=None)
        page.ax_nodes = lambda: [
            {"nodeId": "1", "role": {"value": "main"}, "childIds": ["2"]},
            {"nodeId": "2", "role": {"value": "button"}, "name": {"value": "Robin finished"}},
            {"nodeId": "3", "role": {"value": "dialog"}, "name": {"value": "Agents"}, "childIds": ["4"]},
            {"nodeId": "4", "role": {"value": "navigation"}, "childIds": ["5", "6"]},
            {"nodeId": "5", "role": {"value": "button"}, "name": {"value": "Robin done 1m"}},
            {"nodeId": "6", "role": {"value": "button"}, "name": {"value": "Sam"}, "ignored": True},
        ]
        self.assertEqual(page.names_within("dialog", "Agents", "button"), ["Robin done 1m"])

    def test_a_click_waits_for_a_button_the_page_is_still_rendering(self) -> None:
        page = Rendering(appears_after=3)
        page.click("button", "Deny")
        self.assertEqual([kind for _, kind in page.sent], ["mouseMoved", "mousePressed", "mouseReleased"])

    def test_a_click_on_a_button_that_never_appears_fails_naming_it(self) -> None:
        page = Rendering(appears_after=None)
        with mock.patch("e2e.browser.ACTIONABLE_SECONDS", 0.3):
            with self.assertRaisesRegex(BrowserError, "no button named 'Deny'"):
                page.click("button", "Deny")
        self.assertEqual(page.sent, [])

    def test_a_button_gone_by_the_time_it_is_clicked_fails_at_once_naming_it(self) -> None:
        page = Rendering(appears_after=0, attached=False)
        face = page.find("button", "Deny")[0]
        started = time.monotonic()
        with mock.patch("e2e.browser.ACTIONABLE_SECONDS", 60):
            with self.assertRaisesRegex(BrowserError, "button 'Deny' was gone from the page"):
                page.press(face)
        self.assertLess(time.monotonic() - started, 1)
        self.assertEqual(page.sent, [])

    def test_a_click_that_misses_a_button_that_moved_fails_naming_it(self) -> None:
        page = Rendering(appears_after=0, lands=False)
        with self.assertRaisesRegex(BrowserError, "never reached the button 'Deny'; it moved"):
            page.click("button", "Deny")
        self.assertEqual([kind for _, kind in page.sent], ["mouseMoved", "mousePressed", "mouseReleased"])

    def test_a_click_waits_for_a_button_still_moving_into_place(self) -> None:
        class Rising(Rendering):
            boxes = [[0, 20, 10, 20, 10, 30, 0, 30], [0, 12, 10, 12, 10, 22, 0, 22]]
            boxes += [[0, 0, 10, 0, 10, 10, 0, 10]] * 2

            def send(self, method: str, params: dict | None = None, session: bool = True,
                     timeout: float = 30) -> dict:
                if method == "DOM.getContentQuads":
                    return {"quads": [self.boxes.pop(0) if len(self.boxes) > 1 else self.boxes[0]]}
                self.sent.append((method, (params or {}).get("y", "")))
                return {}

        page = Rising(appears_after=0)
        page._center = lambda node: Browser._center(page, node)
        page.click("button", "Deny")
        self.assertEqual({y for method, y in page.sent if method == "Input.dispatchMouseEvent"}, {5.0})

    def test_a_press_waits_for_a_face_still_sliding_into_place_and_clicks_where_it_stops(self) -> None:
        class Sliding(Rendering):
            boxes = [[0, 0, 10, 0, 10, 10, 0, 10], [13, 0, 23, 0, 23, 10, 13, 10]]
            boxes += [[26, 0, 36, 0, 36, 10, 26, 10]] * 2

            def send(self, method: str, params: dict | None = None, session: bool = True,
                     timeout: float = 30) -> dict:
                if method == "DOM.getContentQuads":
                    return {"quads": [self.boxes.pop(0) if len(self.boxes) > 1 else self.boxes[0]]}
                self.sent.append((method, (params or {}).get("x", "")))
                return {}

        page = Sliding(appears_after=0)
        page._center = lambda node: Browser._center(page, node)
        page.press(page.find("button", "Deny")[0])
        self.assertEqual({x for method, x in page.sent if method == "Input.dispatchMouseEvent"}, {31.0})

    def test_a_press_on_a_face_that_never_stops_moving_says_so_and_not_that_it_was_gone(self) -> None:
        class Shaking(Rendering):
            frame = 0

            def send(self, method: str, params: dict | None = None, session: bool = True,
                     timeout: float = 30) -> dict:
                if method == "DOM.getContentQuads":
                    self.frame += 1
                    x = self.frame % 2 * 26
                    return {"quads": [[x, 0, x + 10, 0, x + 10, 10, x, 10]]}
                self.sent.append((method, ""))
                return {}

        page = Shaking(appears_after=0)
        page._center = lambda node: Browser._center(page, node)
        with mock.patch("e2e.browser.ACTIONABLE_SECONDS", 0.3):
            with self.assertRaises(BrowserError) as raised:
                page.press(page.find("button", "Deny")[0])
        self.assertRegex(str(raised.exception), r"button 'Deny' was still moving .*from \(\d+, 5\) to \(\d+, 5\)")
        self.assertNotIn("gone", str(raised.exception))
        self.assertNotIn("Input.dispatchMouseEvent", [method for method, _ in page.sent])

    def test_a_press_on_a_face_with_no_box_fails_at_once_as_gone(self) -> None:
        class Removed(Rendering):
            def send(self, method: str, params: dict | None = None, session: bool = True,
                     timeout: float = 30) -> dict:
                if method == "DOM.getContentQuads":
                    return {"quads": []}
                self.sent.append((method, ""))
                return {}

        page = Removed(appears_after=0)
        page._center = lambda node: Browser._center(page, node)
        started = time.monotonic()
        with mock.patch("e2e.browser.ACTIONABLE_SECONDS", 60):
            with self.assertRaisesRegex(BrowserError, "button 'Deny' was gone from the page.*no box"):
                page.press(page.find("button", "Deny")[0])
        self.assertLess(time.monotonic() - started, 1)

    def test_a_browser_that_dies_at_launch_says_why_and_never_breaks_the_pipe(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / "chromium"
            binary.write_text("#!/bin/sh\necho '[1:1:FATAL:zygote_host_impl_linux.cc] No usable sandbox!' >&2\n"
                              "for frame in 1 2 3 4 5 6; do echo \"#$frame 0x0\" >&2; done\nexit 133\n")
            binary.chmod(0o755)
            browser = Browser(Path(temp) / "run" / "profile", binary=str(binary))
            with self.assertRaisesRegex(BrowserError, r"exit 133.*No usable sandbox"):
                browser.open("about:blank")
            with self.assertRaisesRegex(BrowserError, "No usable sandbox"):
                browser.send("Page.captureScreenshot")
            browser.close()


class Reading(unittest.TestCase):
    def test_a_call_names_its_speaker_and_the_newest_thing_said_to_it(self) -> None:
        c = Call(1, conversation(("system", "You are Critic, a reviewer.\nSay noted when done."),
                                 ("user", "[OPERATOR]\nfirst"), ("assistant", "ok"), ("user", "second")))
        self.assertEqual((c.speaker, c.last_user), ("Critic", "second"))

    def test_the_system_prompt_is_never_something_anyone_said(self) -> None:
        c = Call(1, conversation(("system", "You are Critic. Say noted when done."), ("user", "hello")))
        self.assertFalse(c.said("noted"))
        self.assertTrue(c.said("hello"))

    def test_tool_results_are_only_those_since_the_newest_message(self) -> None:
        c = Call(1, {"messages": [{"role": "user", "content": "a"}, {"role": "tool", "content": "old"},
                                  {"role": "user", "content": "b"}, {"role": "tool", "content": "new"}]})
        self.assertEqual(c.tool_results(), ["new"])


class Stream(unittest.TestCase):
    def test_words_and_calls_end_with_the_finish_the_runtime_reads(self) -> None:
        frames = stream(call("send_message", to=["Critic"], text="hi", intent="work"))
        self.assertEqual(frames[-1]["choices"][0]["finish_reason"], "tool_calls")
        self.assertEqual(json.loads(frames[0]["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"]),
                         {"to": ["Critic"], "text": "hi", "intent": "work"})
        text = "".join(f["choices"][0]["delta"].get("content", "") for f in stream(say("a" * 30)))
        self.assertEqual(text, "a" * 30)


class Stand(unittest.TestCase):
    def setUp(self) -> None:
        self.dir = tempfile.TemporaryDirectory()
        self.model = Model(Path(self.dir.name))

    def tearDown(self) -> None:
        self.model.stop()
        self.dir.cleanup()

    def test_an_unscripted_call_is_answered_and_recorded(self) -> None:
        status, body = post(self.model.base, {"stream": False, **conversation(("user", "surprise"))})
        self.assertEqual(status, 200)
        self.assertIn(UNSCRIPTED, body.decode())
        self.assertEqual(len(self.model.unscripted), 1)

    def test_a_script_that_throws_counts_as_unscripted(self) -> None:
        self.model.script = lambda c: 1 / 0
        post(self.model.base, {"stream": False, **conversation(("user", "x"))})
        self.assertEqual(len(self.model.unscripted), 1)

    def test_a_refusal_answers_with_its_status(self) -> None:
        self.model.script = lambda c: refuse(401, "bad key") if c.key != "good" else say("ok")
        self.assertEqual(post(self.model.base, {"stream": False}, key="bad")[0], 401)
        self.assertEqual(post(self.model.base, {"stream": False}, key="good")[0], 200)
        self.assertNotIn("good", (Path(self.dir.name) / "model.jsonl").read_text())

    def test_a_held_reply_waits_for_its_gate(self) -> None:
        gate = threading.Event()
        self.model.script = lambda c: held(gate, say("late"))
        answers: list = []
        worker = threading.Thread(target=lambda: answers.append(post(self.model.base, {"stream": False})))
        worker.start()
        until("the call arrives", lambda: self.model.calls, 10)
        self.assertEqual(answers, [])
        gate.set()
        worker.join(10)
        self.assertEqual(answers[0][0], 200)

    def test_a_streamed_reply_is_server_sent_events_ending_in_done(self) -> None:
        self.model.script = lambda c: say("streamed")
        host, port = self.model.base.split("//")[1].split("/")[0].split(":")
        conn = http.client.HTTPConnection(host, int(port), timeout=10)
        conn.request("POST", "/v1/chat/completions", json.dumps({"stream": True}),
                     {"content-type": "application/json"})
        body = conn.getresponse().read().decode()
        self.assertTrue(body.rstrip().endswith("data: [DONE]"))


def cli():
    spec = importlib.util.spec_from_file_location("verify_cli", Path(__file__).with_name("verify.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Starting(unittest.TestCase):
    def setUp(self) -> None:
        self.verify = cli()
        self.order: list[str] = []
        ready = {key: {"ok": True} for key in ("web", "guacad", "guacadClock", "chromium")}
        self.patches = [
            mock.patch.object(self.verify, "doctor", return_value={**ready, "commit": "c", "ok": True}),
            mock.patch.object(self.verify, "attempt", side_effect=self.attempted),
            mock.patch.object(sys, "argv", ["verify.py", "reply", "peers"]),
            mock.patch("sys.stdout", new_callable=io.StringIO),
            mock.patch("sys.stderr", new_callable=io.StringIO),
        ]
        for patch in self.patches:
            patch.start()

    def tearDown(self) -> None:
        for patch in reversed(self.patches):
            patch.stop()

    def attempted(self, name: str, run: int, evidence: Path, keep: bool) -> dict:
        self.order.append(name)
        return {"scenario": name, "run": run, "pass": True, "seconds": 0, "steps": []}

    def test_the_browser_is_started_once_before_any_scenario_starts_its_clock(self) -> None:
        with mock.patch.object(self.verify, "start_browser", side_effect=lambda: self.order.append("browser") or 1.0):
            self.assertEqual(self.verify.main(), 0)
        self.assertEqual(self.order, ["browser", "reply", "peers"])

    def test_a_browser_that_never_starts_runs_no_scenario_and_says_so(self) -> None:
        never = BrowserError("Target.createTarget: no answer within 120s")
        with mock.patch.object(self.verify, "start_browser", side_effect=never):
            self.assertEqual(self.verify.main(), 2)
        self.assertEqual(self.order, [])
        said = json.loads(sys.stdout.getvalue())
        self.assertIn("no answer within 120s", said["missing"]["browser"]["error"])


class Updating(unittest.TestCase):
    def ask(self, path: Path, request: dict) -> bytes:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as conn:
            conn.settimeout(10)
            conn.connect(str(path))
            conn.sendall(json.dumps(request).encode() + b"\n")
            return conn.makefile("rb").readline()

    def test_the_stand_in_updater_answers_its_script_one_line_and_records_the_ask(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            updater = Updater(Path(directory) / "updater.sock", Path(directory))
            updater.script = lambda ask: {"err": "no"} if ask["ask"] == "update" else {"ok": {"updating": False}}
            self.assertEqual(json.loads(self.ask(updater.path, {"ask": "status"})), {"ok": {"updating": False}})
            self.assertEqual(json.loads(self.ask(updater.path, {"ask": "update", "version": "1"})), {"err": "no"})
            self.assertEqual(updater.updates(), [{"ask": "update", "version": "1"}])
            updater.stop()

    def test_a_script_that_returns_nothing_closes_without_an_answer(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            updater = Updater(Path(directory) / "updater.sock", Path(directory))
            self.assertEqual(self.ask(updater.path, {"ask": "update", "version": "1"}), b"")
            self.assertEqual(len(updater.updates()), 1)
            updater.stop()


class Leaving(unittest.TestCase):
    def test_egress_refuses_and_records_where_it_was_going(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            egress = Egress(Path(directory) / "egress.jsonl")
            proxy = urllib.request.ProxyHandler({"http": egress.address})
            with self.assertRaises(urllib.error.HTTPError) as refused:
                urllib.request.build_opener(proxy).open("http://example.com/x", timeout=10)
            self.assertEqual(refused.exception.code, 403)
            until("the attempt is recorded", lambda: egress.attempts, 5)
            self.assertEqual(egress.attempts, ["GET http://example.com/x"])
            egress.stop()


class Evidence(unittest.TestCase):
    def test_secrets_and_invitations_are_redacted(self) -> None:
        text = redact("token abc123 at http://h/#token=abc123 and key s3cret", ["abc123", "s3cret"])
        self.assertNotIn("abc123", text)
        self.assertNotIn("s3cret", text)

    def test_a_claim_no_passing_step_made_is_unproved(self) -> None:
        steps = [{"step": "the reply arrives", "pass": True}, {"step": "the key is kept", "pass": False}]
        claims = ("the reply arrives", "the key is kept", "the run ends")
        self.assertEqual(unproved(claims, steps), ["the key is kept", "the run ends"])

    def test_every_claim_is_a_check_its_scenario_makes(self) -> None:
        for name, scenario in SCENARIOS.items():
            source = inspect.getsource(sys.modules[scenario.run.__module__])
            for claim in scenario.claims:
                self.assertIn(f'check("{claim}"', source.replace("check(\n", "check("), f"{name}: {claim}")

    def test_a_wait_that_never_holds_fails_naming_what_never_happened(self) -> None:
        with self.assertRaises(Failed) as failed:
            until("the reply arrives", lambda: False, 0.2)
        self.assertIn("the reply arrives", str(failed.exception))

    def test_every_scenario_says_what_it_proves(self) -> None:
        for name, scenario in SCENARIOS.items():
            self.assertTrue(scenario.proves.endswith("."), name)
            self.assertIn(";", scenario.proves, f"{name} names no failure path")


if __name__ == "__main__":
    unittest.main()
