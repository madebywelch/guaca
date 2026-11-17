import http.client
import json
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from pathlib import Path

from e2e.browser import Browser, BrowserError, split_frames
from e2e.scenarios import SCENARIOS
from e2e.world import (UNSCRIPTED, Call, Egress, Failed, Model, call, held, redact, refuse, say, stream,
                       until)


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


class Pipe(unittest.TestCase):
    def test_a_message_split_across_reads_is_held_until_its_end(self) -> None:
        whole, rest = split_frames(b'{"id":1}\0{"id"')
        self.assertEqual(whole, [b'{"id":1}'])
        whole, rest = split_frames(rest + b':2}\0')
        self.assertEqual((whole, rest), ([b'{"id":2}'], b""))

    def test_a_browser_that_dies_at_launch_says_why_and_never_breaks_the_pipe(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / "chromium"
            binary.write_text("#!/bin/sh\necho 'No usable sandbox!' >&2\nexit 133\n")
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
