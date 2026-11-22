import json
import math
import re
import socket
import subprocess
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import Mock, call, patch

import probe


SECRET = "fake-secret-never-retain"


class Channel:
    def __init__(self, path, answer):
        self.answer = answer
        self.requests = []
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.socket.bind(str(path))
        self.socket.listen()
        self.socket.settimeout(0.4)
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()

    def serve(self):
        for _ in range(2):
            try:
                connection = self.socket.accept()[0]
            except OSError:
                return
            with connection:
                with connection.makefile("rb") as stream:
                    request = json.loads(stream.readline())
                self.requests.append(request)
                value = self.answer(request)
                try:
                    connection.sendall(json.dumps({"id": request["id"], "ok": True, "value": value}).encode() + b"\n")
                except BrokenPipeError:
                    pass

    def close(self):
        self.thread.join(timeout=1)
        self.socket.close()


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="probe-unit-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        built = self.root / "built.app"
        (built / "Contents").mkdir(parents=True)
        for name, value in {
            "BUILT_APP": built,
            "APPS": self.root / "apps",
            "FAILURES": self.root / "failures",
            "run": Mock(return_value=subprocess.CompletedProcess([], 0, "", "")),
        }.items():
            patcher = patch.object(probe, name, value, create=True)
            patcher.start()
            self.addCleanup(patcher.stop)
        self.scratch = None
        self.client = None

    def answer(self, request):
        if request["op"] == "state":
            return {
                "pid": 42, "identifier": SECRET, "notifications": SECRET,
                "windows": [{"label": "main", "visible": True, "focused": False, "minimized": False,
                             "title": SECRET}, {"label": SECRET, "visible": True}],
                "menubar": {"attached": True, "panel": True, "panelVisible": False,
                            "title": SECRET, "tooltip": SECRET, "glyph": SECRET},
                "payload": SECRET,
            }
        return {"rows": [{"phase": "select.committed", "id": 7, "value": 2, "ms": 12.5,
                           "message": SECRET},
                          {"phase": SECRET, "id": 7, "value": 2, "ms": 12.5},
                          {"phase": "select.committed", "id": SECRET, "value": 2, "ms": 12.5}],
                "ms": 13, "token": SECRET}

    def scenario(self, record, scratch, *, error=True, diagnostic=None):
        self.scratch = scratch
        for path in (scratch.root / "app.log", scratch.root / "host.log", scratch.host / "config.json"):
            path.write_text(SECRET)
        row = {"phase": "toggle.enter", "id": 7, "value": 2, "us": 100, "main": True, "body": SECRET}
        invalid = [{**row, key: value} for key, value in (
            ("phase", SECRET), ("phase", {}), ("id", SECRET), ("id", True), ("id", -1),
            ("id", 2**64), ("value", SECRET), ("value", float("nan")),
            ("us", SECRET), ("us", float("inf")), ("main", SECRET))]
        (scratch.channel / "native.jsonl").write_text(
            "\n".join(json.dumps(value) for value in [row, *invalid]) + "\n" + SECRET + "\n")
        app = probe.App(scratch)
        self.client = app.client = Mock()
        channel = Channel(app.socket, diagnostic or self.answer)
        self.addCleanup(channel.close)
        try:
            if error:
                raise probe.Failed("original failure " + SECRET)
            record.check("a non-stopping failure", False)
        finally:
            app.quit()

    def run_attempt(self, scenario):
        with patch.dict(probe.SCENARIOS, {"unit": scenario}), patch.object(
            probe.App, "alive", lambda app: app.client is not None
        ):
            return probe.attempt("unit", 1, False)

    def evidence(self, entry):
        directory = Path(entry["evidence"])
        self.assertTrue(directory.is_relative_to(self.root / "failures"))
        self.assertFalse(self.scratch.root.exists())
        self.assertFalse(self.scratch.apps.exists())
        self.assertEqual([path.name for path in directory.iterdir()], ["evidence.json"])
        self.assertEqual(directory.stat().st_mode & 0o777, 0o700)
        self.assertEqual(directory.parent.stat().st_mode & 0o777, 0o700)
        self.assertEqual((directory / "evidence.json").stat().st_mode & 0o777, 0o600)
        text = (directory / "evidence.json").read_text()
        self.assertNotIn(SECRET, text)
        return json.loads(text)

    def test_exception_in_scenario_finally_retains_evidence_after_cleanup(self):
        record, entry = self.run_attempt(self.scenario)
        self.assertFalse(record.passed)
        self.assertEqual(entry["steps"][-1], {"step": "original failure " + SECRET, "pass": False})
        evidence = self.evidence(entry)
        self.assertEqual(evidence["native"]["rows"], [
            {"phase": "toggle.enter", "id": 7, "value": 2, "us": 100, "main": True}])
        snapshot = evidence["snapshots"][0]
        self.assertEqual(snapshot["page"], {"rows": [
            {"phase": "select.committed", "id": 7, "value": 2, "ms": 12.5}], "ms": 13})
        self.assertEqual(snapshot["state"], {
            "windows": {"main": {"visible": True, "focused": False, "minimized": False}},
            "menubar": {"attached": True, "panel": True, "panelVisible": False}})
        self.client.ask.assert_called_once_with("quit")
        self.client.close.assert_called_once()

    def test_non_stopping_failure_and_unique_attempt_directories(self):
        def scenario(record, scratch):
            self.scenario(record, scratch, error=False)
        first = self.run_attempt(scenario)[1]
        second = self.run_attempt(scenario)[1]
        self.assertNotEqual(first["evidence"], second["evidence"])
        self.assertEqual(len(self.evidence(second)["snapshots"]), 1)

    def test_missing_or_unreadable_native_trace(self):
        for unreadable in (False, True):
            with self.subTest(unreadable=unreadable):
                def scenario(record, scratch):
                    self.scratch = scratch
                    if unreadable:
                        (scratch.channel / "native.jsonl").mkdir()
                    record.check("failed before launch", False)
                evidence = self.evidence(self.run_attempt(scenario)[1])
                self.assertEqual(evidence["native"], {"readable": False, "rows": []})

    def test_existing_evidence_is_never_overwritten(self):
        with patch.object(probe.uuid, "uuid4", return_value=Mock(hex="same-attempt")):
            first = self.run_attempt(self.scenario)[1]
            path = Path(first["evidence"]) / "evidence.json"
            before = path.read_bytes()
            second = self.run_attempt(self.scenario)[1]
        self.assertIsNone(second["evidence"])
        self.assertEqual(path.read_bytes(), before)
        self.assertFalse(self.scratch.root.exists())

    def test_failed_diagnostics_do_not_replace_failure_or_skip_cleanup(self):
        def diagnostic(request):
            return {"rows": SECRET, "ms": SECRET} if request["op"] == "page" else SECRET
        entry = self.run_attempt(lambda record, scratch: self.scenario(
            record, scratch, diagnostic=diagnostic))[1]
        evidence = self.evidence(entry)
        self.assertEqual(entry["steps"][-1]["step"], "original failure " + SECRET)
        self.assertEqual(evidence["snapshots"][0]["page"], {"rows": []})
        self.client.close.assert_called_once()

    def test_capture_exception_still_quits_and_cleans_up(self):
        with patch.object(probe, "diagnostic_snapshot", side_effect=RuntimeError(SECRET)):
            entry = self.run_attempt(self.scenario)[1]
        self.assertEqual(self.evidence(entry)["snapshots"], [])
        self.assertEqual(entry["steps"][-1]["step"], "original failure " + SECRET)
        self.client.ask.assert_called_once_with("quit")
        self.client.close.assert_called_once()

    def test_trace_read_error_keeps_other_evidence(self):
        original = Path.open

        def open_(path, *args, **kwargs):
            if path.name == "native.jsonl" and args == ("rb",):
                raise PermissionError(SECRET)
            return original(path, *args, **kwargs)

        with patch.object(Path, "open", open_):
            entry = self.run_attempt(self.scenario)[1]
        evidence = self.evidence(entry)
        self.assertEqual(evidence["native"], {"readable": False, "rows": []})
        self.assertEqual(len(evidence["snapshots"]), 1)

    def test_evidence_write_error_does_not_skip_cleanup(self):
        original = probe.os.open

        def open_(path, *args, **kwargs):
            if Path(path).name == "evidence.json":
                raise PermissionError(SECRET)
            return original(path, *args, **kwargs)

        with patch.object(probe.os, "open", open_):
            entry = self.run_attempt(self.scenario)[1]
        self.assertIsNone(entry["evidence"])
        self.assertFalse(self.scratch.root.exists())
        self.assertFalse(self.scratch.apps.exists())
        self.assertEqual(entry["steps"][-1]["step"], "original failure " + SECRET)

    def test_native_and_page_rows_are_bounded(self):
        native = {"phase": "toggle.completed", "id": 1, "value": False, "us": 2, "main": True}
        page = {"phase": "notification.remembered", "id": 1, "value": True, "ms": 2}
        self.assertEqual(probe.evidence_rows([native] * 5000, probe.NATIVE_PHASES, "us"), [native] * 4096)
        self.assertEqual(probe.evidence_rows([page] * 600, probe.PAGE_PHASES, "ms"), [page] * 256)

    def test_launch_passes_native_trace_path(self):
        scratch = probe.Scratch()
        self.addCleanup(scratch.close)
        app = probe.App(scratch)
        with patch.object(probe, "Client") as client, patch.object(probe, "wait_until"):
            client.return_value.ask.return_value = {"pid": 42}
            app.launch()
        args = probe.run.call_args.args[0]
        trace = f"GUACA_PROBE_TRACE={scratch.channel}/native.jsonl"
        self.assertIn(trace, args)
        self.assertEqual(args[args.index(trace) - 1], "--env")
        app.client = app.pid = None

    def launch(self, scratch, app, hello):
        with patch.object(probe, "Client") as client, patch.object(probe, "wait_until"):
            client.return_value.ask.return_value = hello
            app.launch()
        args = probe.run.call_args.args[0]
        return next(arg.split("=", 1)[1] for arg in args if arg.startswith("GUACA_PROBE_TRACE="))

    def write_trace(self, path, *phases):
        rows = [{"phase": phase, "id": 0, "value": 0, "us": index, "main": True}
                for index, phase in enumerate(phases)]
        Path(path).write_text("".join(json.dumps(row) + "\n" for row in rows))

    def append_trace(self, path, *phases):
        with open(path, "a") as trace:
            for phase in phases:
                trace.write(json.dumps({"phase": phase, "id": 9, "value": 3, "us": 50, "main": True}) + "\n")

    def retained(self, scratch):
        directory = scratch.retain_failure()
        self.assertIsNotNone(directory)
        return json.loads((directory / "evidence.json").read_text())

    def test_relaunch_in_one_run_writes_a_trace_per_launch(self):
        scratch = probe.Scratch()
        self.addCleanup(scratch.close)
        first, second = probe.App(scratch), probe.App(scratch, folder="second")
        paths = [self.launch(scratch, first, {"pid": 42, "trace": True}),
                 self.launch(scratch, first, {"pid": 43, "trace": True}),
                 self.launch(scratch, second, {"pid": 44, "trace": True})]
        first.client = first.pid = second.client = second.pid = None
        self.assertEqual(paths, [str(scratch.channel / name) for name in
                                 ("native.jsonl", "native-2.jsonl", "native-3.jsonl")])
        self.write_trace(paths[0], "trace.opened", "app.did_hide")
        self.write_trace(paths[1], "trace.opened")
        self.write_trace(paths[2], "trace.opened", "window.open", "window.reveal")
        evidence = self.retained(scratch)
        self.assertEqual(evidence["traces"], [
            {"launch": 1, "health": "healthy", "events": 1},
            {"launch": 2, "health": "healthy", "events": 0},
            {"launch": 3, "health": "healthy", "events": 2}])
        self.assertEqual([row["phase"] for row in evidence["native"]["rows"]], [
            "trace.opened", "app.did_hide", "trace.opened", "trace.opened", "window.open", "window.reveal"])
        self.assertTrue(evidence["native"]["readable"])

    def test_a_writer_that_cannot_open_its_file_reads_as_failed(self):
        scratch = probe.Scratch()
        self.addCleanup(scratch.close)
        app = probe.App(scratch)
        refused = self.launch(scratch, app, {"pid": 42, "trace": False})
        silent = self.launch(scratch, app, {"pid": 43, "trace": True})
        empty = self.launch(scratch, app, {"pid": 44, "trace": True})
        app.client = app.pid = None
        self.write_trace(refused, "trace.opened")
        Path(empty).write_text("")
        evidence = self.retained(scratch)
        self.assertEqual(evidence["traces"], [
            {"launch": 1, "health": "writer failed", "events": 0},
            {"launch": 2, "health": "writer failed", "events": 0},
            {"launch": 3, "health": "writer failed", "events": 0}])
        self.assertNotIn(silent, json.dumps(evidence))

    def test_a_missed_banner_press_is_attributed(self):
        outcomes = {
            (): "no native response arrived",
            ("notification.response",): "a native response arrived and no click was emitted",
            ("notification.response", "notification.default", "notification.clicked"):
                "a click was emitted and the page did not act",
        }
        for after, attribution in outcomes.items():
            with self.subTest(after=after):
                scratch = probe.Scratch()
                self.addCleanup(scratch.close)
                app = probe.App(scratch)
                path = self.launch(scratch, app, {"pid": 42, "trace": True})
                app.client = app.pid = None
                self.write_trace(path, "trace.opened")
                self.append_trace(path, "notification.response", "notification.clicked")
                scratch.press("clicking the first banner")
                self.append_trace(path, "window.reveal")
                missed = scratch.press("clicking the second banner")
                self.append_trace(path, *after)
                missed["missed"] = True
                evidence = self.retained(scratch)
                self.assertEqual(evidence["presses"], [
                    {"press": "clicking the second banner", "attribution": attribution}])

    def test_a_missed_press_without_a_healthy_trace_is_not_attributed(self):
        scratch = probe.Scratch()
        self.addCleanup(scratch.close)
        app = probe.App(scratch)
        self.launch(scratch, app, {"pid": 42, "trace": False})
        app.client = app.pid = None
        scratch.press("clicking the test banner")["missed"] = True
        self.assertEqual(self.retained(scratch)["presses"], [
            {"press": "clicking the test banner",
             "attribution": "the native trace writer failed, so the press cannot be attributed"}])

    def test_a_press_that_worked_is_not_attributed(self):
        scratch = probe.Scratch()
        self.addCleanup(scratch.close)
        scratch.press("clicking the banner")
        self.assertEqual(self.retained(scratch)["presses"], [])

    def test_silent_channel_has_a_bounded_capture(self):
        def diagnostic(request):
            time.sleep(0.15)
            return self.answer(request)
        with patch.object(probe, "DIAGNOSTIC_TIMEOUT", 0.03, create=True):
            started = time.monotonic()
            entry = self.run_attempt(lambda record, scratch: self.scenario(
                record, scratch, diagnostic=diagnostic))[1]
        self.assertLess(time.monotonic() - started, 0.5)
        self.assertEqual(self.evidence(entry)["snapshots"][0], {})
        self.client.close.assert_called_once()

    def test_success_and_deferral_leave_no_evidence(self):
        for deferred in (False, True):
            def scenario(record, scratch):
                self.scratch = scratch
                if deferred:
                    raise probe.Deferred("try later")
            _, entry = self.run_attempt(scenario)
            self.assertNotIn("evidence", entry)
            self.assertFalse(self.scratch.root.exists())
        self.assertFalse((self.root / "failures").exists())


class PanelStableTests(unittest.TestCase):
    def test_close_hidden_reopen_stays_focused_or_detects_late_hide(self):
        for outcome in ("stable", "hidden", "unfocused"):
            with self.subTest(outcome=outcome):
                clock = [0.0]
                opened = [False]
                main = [True]
                client = Mock()
                app = Mock(client=client)
                app.launch.return_value = client

                def ask(op, **fields):
                    if op == "close":
                        main[0] = False
                    if op == "clickIcon":
                        opened[0] = not opened[0]

                def state():
                    panel = opened[0] and not (outcome == "hidden" and clock[0] >= 0.75)
                    focused = panel and not (outcome == "unfocused" and clock[0] >= 0.75)
                    return {"menubar": {"attached": True, "panelVisible": panel}, "windows": [
                        {"label": "main", "visible": main[0]},
                        {"label": "menubar", "visible": panel, "focused": focused}]}

                client.ask.side_effect = ask
                app.state.side_effect = state
                record = probe.Record("panelstable", 1, [])
                with patch.object(probe, "Host"), patch.object(probe, "App", return_value=app), \
                     patch.object(probe, "connect"), patch.object(probe, "person_away"), \
                     patch.object(probe, "front", return_value={"pid": 1}), patch.object(probe, "hand_back"), \
                     patch.object(probe.time, "monotonic", side_effect=lambda: clock[0]), \
                     patch.object(probe.time, "sleep", side_effect=lambda seconds: clock.__setitem__(0, clock[0] + seconds)):
                    probe.scenario_panelstable(record, Mock())
                self.assertEqual(record.passed, outcome == "stable")
                self.assertEqual(client.ask.call_args_list, [
                    call("close", window="main"), call("clickIcon"), call("clickIcon"), call("clickIcon")])
                self.assertGreaterEqual(clock[0], 3 if outcome == "stable" else 0.75)
                app.quit.assert_called_once()
                app.forget.assert_called_once()



class Clock:
    def __init__(self):
        self.now = 0.0

    def sleep(self, seconds):
        self.now += seconds


class LapsingHost:
    WINDOW_S = 60

    def __init__(self, clock, phone=None):
        self.clock = clock
        self.phone = phone
        self.token = "probe-token"
        self.asked_at = None
        self.answered = False
        self.outbox = []
        self.calls = []

    def lapsed(self):
        return not self.answered and self.asked_at is not None and self.clock.now - self.asked_at >= self.WINDOW_S

    def seed(self):
        pass

    def use(self, model):
        pass

    def stop(self):
        pass

    def finish(self, agent):
        self.outbox.append({"kind": "outcome", "title": agent, "status": "pending"})
        if self.phone is not None and not self.lapsed():
            self.phone.push(agent, probe.StandIn.REPLY.splitlines()[0])

    def send(self, agent, text):
        self.calls.append(("send", agent))
        if probe.StandIn.ASK in text:
            self.asked_at = self.clock.now
            self.outbox.append({"kind": "needs_you", "title": "Researcher has a question", "status": "delivered"})
            if self.phone is not None:
                self.phone.push("Needs you", "Researcher has a question. It waits for your answer for up to ten minutes.")
        elif self.asked_at is not None and not self.answered and self.phone is not None:
            self.outbox.append({"kind": "outcome", "title": agent, "status": "pending"})
        else:
            self.finish(agent)

    def call(self, name, **args):
        self.calls.append((name, args))
        if name == "pending_approvals":
            return [] if self.answered or self.lapsed() else [{"id": "question"}]
        if name == "answer_question":
            self.answered = True
            self.finish("Researcher")
        return None

    def rows(self, kind=None):
        if self.lapsed() and not any(r["title"] == "Researcher" for r in self.outbox):
            self.finish("Researcher")
            for row in self.outbox:
                if row["title"] != "Researcher":
                    row["status"] = "delivered"
        elif self.answered and self.clock.now - self.asked_at >= self.WINDOW_S:
            for row in self.outbox:
                row["status"] = "delivered"
        return [dict(r) for r in self.outbox if kind is None or r["kind"] == kind]


class Phone:
    def __init__(self):
        self.received = []
        self.url = "http://127.0.0.1:1/probe"

    def push(self, title, body):
        self.received.append({"title": title, "body": body, "priority": "3", "delivery": "probe"})

    def titled(self, title):
        return [p for p in self.received if p["title"] == title]


class ScaledWindowTests(unittest.TestCase):
    def clocked(self, clock):
        return (patch.object(probe.time, "monotonic", side_effect=lambda: clock.now),
                patch.object(probe.time, "sleep", side_effect=clock.sleep))

    def test_push_answers_the_question_before_a_second_run_can_be_gathered_with_it(self):
        clock, phone = Clock(), Phone()
        host = LapsingHost(clock, phone)
        record = probe.Record("push", 1, [])
        ticking, sleeping = self.clocked(clock)
        with patch.object(probe, "Host", return_value=host), patch.object(probe, "StandIn"), \
             patch.object(probe, "Listener", return_value=phone), ticking, sleeping:
            probe.StandIn.ASK = "PROBE-ASK"
            probe.StandIn.REPLY = "Probe test run finished.\nscripted"
            probe.StandIn.ANSWER = "First"
            probe.scenario_push(record, Mock())
        names = [c[0] if c[0] != "send" else ("send", c[1]) for c in host.calls]
        self.assertLess(names.index("answer_question"), names.index(("send", "Scribe")))
        self.assertTrue(record.passed, record.steps)
        self.assertEqual([p["title"] for p in phone.received], ["Needs you", "Researcher", "Scribe"])

    def test_longaway_answers_the_question_once_the_window_has_taken_it(self):
        clock = Clock()
        host = LapsingHost(clock)
        banners = []
        app = Mock(pid=7, bundle="probe.bundle")
        log = Mock()
        log.matching.side_effect = lambda _: banners
        it = Mock(app=app, host=host, log=log)
        prompted = Mock()
        prompted.return_value.__enter__ = Mock(return_value=it)
        prompted.return_value.__exit__ = Mock(return_value=False)

        def rows(kind=None, real=host.rows):
            found = real(kind)
            banners[:] = ["banner"] * (1 + any(r["kind"] == "outcome" and r["status"] == "delivered" for r in host.outbox))
            return found
        host.rows = rows
        record = probe.Record("longaway", 1, [])
        ticking, sleeping = self.clocked(clock)
        with patch.object(probe, "Prompted", prompted), patch.object(probe, "StandIn"), \
             patch.object(probe, "screen_step"), patch.object(probe, "press_when_shown"), \
             patch.object(probe, "front", return_value={"pid": 7}), patch.object(probe, "hand_back"), \
             patch.object(probe, "LONG_AWAY_S", 0), ticking, sleeping:
            probe.StandIn.ASK = "PROBE-ASK"
            probe.StandIn.REPLY = "Probe test run finished."
            probe.StandIn.ANSWER = "First"
            probe.scenario_longaway(record, Mock())
        self.assertTrue(record.passed, record.steps)
        self.assertTrue(host.answered)
        self.assertEqual(sorted(r["title"] for r in host.outbox),
                         ["Researcher", "Researcher has a question", "Scribe"])


class PersonAwayTests(unittest.TestCase):
    def away(self, idle, since_own):
        with patch.object(probe, "idle_seconds", return_value=idle), \
             patch.object(probe.time, "monotonic", return_value=1000.0), \
             patch.dict(probe.OWN_INPUT, {"at": 1000.0 - since_own}):
            probe.person_away("clicking")

    def test_input_after_the_probe_own_input_is_a_person(self):
        with self.assertRaises(probe.Deferred):
            self.away(idle=1.0, since_own=3.0)

    def test_input_with_no_probe_input_is_a_person(self):
        with self.assertRaises(probe.Deferred):
            self.away(idle=1.0, since_own=float("inf"))

    def test_the_probe_own_click_and_key_are_not_a_person(self):
        self.away(idle=2.0, since_own=2.0)
        self.away(idle=2.2, since_own=2.0)

    def test_an_idle_mac_is_away(self):
        self.away(idle=probe.AWAY, since_own=float("inf"))

    def test_only_helpers_that_post_input_are_stamped(self):
        done = subprocess.CompletedProcess([], 0, '{"seconds": 9}', "")
        for args, stamped in ((("click-icon", "7"), True), (("escape",), True), (("idle",), False),
                              (("press-group", "x"), False)):
            with patch.object(probe, "run", return_value=done), \
                 patch.object(probe.time, "monotonic", return_value=55.0), \
                 patch.object(probe, "HELPER", Mock(exists=Mock(return_value=True), stat=Mock(return_value=Mock(st_mtime=2)))), \
                 patch.object(probe, "HELPER_SOURCE", Mock(stat=Mock(return_value=Mock(st_mtime=1)))), \
                 patch.dict(probe.OWN_INPUT, {"at": -1.0}):
                probe.helper(*args)
                self.assertEqual(probe.OWN_INPUT["at"] == 55.0, stamped, args)


class SummaryTests(unittest.TestCase):
    def entry(self, scenario, run, outcome):
        steps = [{"step": "clicking: someone is using the Mac; try again later", "deferred": True}] \
            if outcome == "not run" else [{"step": "a step", "pass": outcome == "pass"}]
        return {"scenario": scenario, "run": run, "pass": outcome == "pass", "deferred": outcome == "not run",
                "outcome": outcome, "steps": steps}

    def test_a_put_off_run_is_counted_apart_and_never_as_a_pass(self):
        summary = probe.summarize([self.entry("escape", 1, "pass"), self.entry("escape", 2, "not run")])
        self.assertEqual(summary["counts"], {"pass": 1, "fail": 0, "not run": 1})
        self.assertEqual(summary["notRun"], [{"scenario": "escape", "run": 2,
                                              "reason": "clicking: someone is using the Mac; try again later"}])
        self.assertTrue(summary["pass"])
        self.assertEqual(summary["neverRan"], [])

    def test_a_scenario_that_never_ran_fails_the_probe(self):
        summary = probe.summarize([self.entry("launch", 1, "pass")] +
                                  [self.entry("escape", n, "not run") for n in (1, 2, 3)])
        self.assertFalse(summary["pass"])
        self.assertEqual(summary["neverRan"], ["escape"])
        self.assertNotIn("escape", summary["identical"])

    def test_a_failure_fails_the_probe(self):
        summary = probe.summarize([self.entry("push", 1, "fail"), self.entry("push", 2, "pass")])
        self.assertFalse(summary["pass"])
        self.assertEqual(summary["counts"]["fail"], 1)

    def test_a_put_off_attempt_records_not_run(self):
        record = probe.Record("escape", 2, [])
        record.deferred = True
        self.assertEqual(record.result()["outcome"], "not run")
        self.assertFalse(record.result()["pass"])


class SettingsCopyTests(unittest.TestCase):
    dialog = (probe.ROOT / "src/components/SettingsPage.tsx").read_text()

    def test_the_pane_lines_are_every_line_the_dialog_can_show(self):
        block = self.dialog.split("const MAC_ACCESS", 1)[1].split("};", 1)[0]
        self.assertEqual(set(probe.PANE), set(re.findall(r"^  (\w+):", block, re.M)))
        for line in probe.PANE.values():
            self.assertIn(f'"{line}"', block)

    def test_the_test_banner_is_the_one_the_dialog_sends(self):
        self.assertIn(f'"{probe.TEST_BODY}"', self.dialog)


class PresenceWindowTests(unittest.TestCase):
    def test_a_window_that_stays_says_so_before_the_scaled_away_window_ends(self):
        view = (probe.ROOT / "src/lib/view.ts").read_text()
        heartbeat = int(re.search(r"^export const HEARTBEAT_MS = ([\d_]+);$", view, re.M).group(1).replace("_", ""))
        rust = (probe.ROOT / "src-tauri/src/domain/view.rs").read_text()
        away = re.search(r"^pub const AWAY_AFTER_MS: i64 = ([\d *]+);$", rust, re.M).group(1)
        scaled_away = math.prod(int(n) for n in away.split("*")) * probe.PRESENCE_MINUTE_MS // 60_000
        self.assertGreater(scaled_away, heartbeat)


class ScrollBarTests(unittest.TestCase):
    def box(self, left, width, gutter=0):
        return {"left": left, "right": left + width, "width": width, "gutter": gutter,
                "viewport": {"width": 1280, "height": 820}}

    def laid(self, shift=0, classic_gutter=15, overlay_gutter=0):
        def one(gutter, shift):
            return {"sections": self.box(179, 186, gutter), "newAgent": self.box(336 + shift, 608),
                    "general": self.box(187, 153), "yourName": self.box(389 + shift, 688),
                    "pane": self.box(365 + shift, 736, 10)}
        return {"classic": one(classic_gutter, shift), "overlay": one(overlay_gutter, 0)}

    def test_a_layout_pushed_aside_by_classic_bars_is_a_fault(self):
        faults = probe.bars_apart(self.laid(shift=-17))
        self.assertIn("newAgent sits -17 pt off the window's center with classic scroll bars", faults)
        self.assertTrue(any(f.startswith("yourName is 688 wide at 372") for f in faults), faults)

    def test_a_mode_that_did_not_take_is_a_fault_rather_than_a_pass(self):
        faults = probe.bars_apart(self.laid(classic_gutter=0))
        self.assertEqual(faults, ["classic scroll bars did not take: the section list has a 0 pt gutter"])
        self.assertEqual(probe.bars_apart(self.laid(overlay_gutter=15)),
                         ["overlay scroll bars did not take: the section list has a 15 pt gutter"])

    def held(self, reads):
        values = iter(reads)

        def polls(what, check, *_, **__):
            for _ in reads:
                value = check()
                if value:
                    return value
            raise probe.Failed(what)

        with patch.object(probe, "wait_until", side_effect=polls):
            try:
                return probe.held_still("New agent holds still", lambda: next(values))
            except probe.Failed as why:
                return why

    def test_a_dialog_still_growing_is_measured_only_once_it_holds_still(self):
        growing = [self.box(342.08, 595.84), self.box(340.694, 598.612)]
        self.assertIsInstance(self.held(growing), probe.Failed)
        self.assertEqual(self.held([*growing, self.box(336, 608), self.box(336, 608)]), self.box(336, 608))

    def test_the_same_layout_in_both_modes_has_no_fault(self):
        self.assertEqual(probe.bars_apart(self.laid()), [])

    def test_a_ring_after_a_pointer_click_or_a_cut_off_ring_is_a_fault(self):
        row = {"role": "button", "name": "Researcher, idle"}
        self.assertEqual(probe.ring_faults("classic", {**row, "ring": True, "clipped": False},
                                           {**row, "ring": True, "clipped": False}),
                         ["with classic scroll bars the panel shows a focus ring on 'Researcher, idle' after a pointer click"])
        self.assertEqual(probe.ring_faults("classic", {}, {**row, "ring": True, "clipped": True}),
                         ["with classic scroll bars the ring on 'Researcher, idle' is cut off"])
        self.assertEqual(probe.ring_faults("overlay", {}, {**row, "ring": False, "clipped": False}),
                         ["with overlay scroll bars Tab focused 'Researcher, idle' and drew no ring"])
        self.assertEqual(probe.ring_faults("overlay", {}, None),
                         ["with overlay scroll bars Tab in the panel reached no agent row"])
        self.assertEqual(probe.ring_faults("overlay", {}, {**row, "ring": True, "clipped": False}), [])

    def test_launch_sets_scroll_bars_for_the_app_alone(self):
        scratch = probe.Scratch()
        self.addCleanup(scratch.close)
        with patch.object(probe, "run") as run, patch.object(probe.shutil, "copytree"), \
                patch.object(probe, "Client") as client, patch.object(probe, "wait_until"):
            run.return_value = subprocess.CompletedProcess([], 0, "", "")
            client.return_value.ask.return_value = {"pid": 42}
            app = probe.App(scratch)
            app.launch(background=False, hidden=False, scroll_bars="Always")
            classic = run.call_args.args[0]
            app.launch()
            plain = run.call_args.args[0]
        app.client = app.pid = None
        self.assertEqual(classic[-4:], [str(app.path), "--args", "-AppleShowScrollBars", "Always"])
        self.assertNotIn("--args", plain)

    def test_a_real_key_is_stamped_as_the_probe_own_input(self):
        done = subprocess.CompletedProcess([], 0, '{"key": "tab"}', "")
        with patch.object(probe, "run", return_value=done), \
             patch.object(probe.time, "monotonic", return_value=77.0), \
             patch.object(probe, "HELPER", Mock(exists=Mock(return_value=True), stat=Mock(return_value=Mock(st_mtime=2)))), \
             patch.object(probe, "HELPER_SOURCE", Mock(stat=Mock(return_value=Mock(st_mtime=1)))), \
             patch.dict(probe.OWN_INPUT, {"at": -1.0}):
            probe.helper("key", "tab")
            self.assertEqual(probe.OWN_INPUT["at"], 77.0)


class TrayTests(unittest.TestCase):
    WAITING = {"amber": 47, "sides": 40, "outlined": 38, "eye": 4,
               "help": "Guaca · Researcher is waiting on you · 120 tokens this session", "title": "1"}
    CLEAR = {"amber": 0, "sides": 0, "outlined": 0, "eye": 0, "help": "Guaca · 1 agent working", "title": ""}

    def states(self, **changes):
        return {"idle": self.CLEAR, "waiting": self.WAITING, "held": self.CLEAR, "heldPending": 1,
                "undone": self.WAITING, **changes}

    def test_an_amber_glyph_with_its_eye_but_no_outline_is_a_fault(self):
        eye_only = {**self.WAITING, "outlined": 6}
        self.assertFalse(probe.waiting_glyph(eye_only))
        self.assertEqual(probe.tray_faults(self.states(waiting=eye_only)),
                         ["waiting: a dark outline meets the amber on 6 of 40 sides"])

    def test_an_outlined_glyph_without_its_eye_is_a_fault(self):
        self.assertEqual(probe.tray_faults(self.states(undone={**self.WAITING, "eye": 0})),
                         ["undone: the amber glyph has 0 dark eye pixels"])

    def test_amber_with_nothing_waiting_or_a_wrong_count_is_a_fault(self):
        self.assertEqual(probe.tray_faults(self.states(idle={**self.CLEAR, "amber": 30})),
                         ["with nothing waiting the glyph shows 30 amber pixels"])
        self.assertEqual(probe.tray_faults(self.states(waiting={**self.WAITING, "title": "2"})),
                         ["waiting: the count beside the glyph reads '2'"])

    def test_a_glyph_still_amber_while_the_answer_is_held_is_a_fault(self):
        faults = probe.tray_faults(self.states(held=self.WAITING))
        self.assertEqual(len(faults), 1)
        self.assertTrue(faults[0].startswith("while the answer is held the glyph shows 47 amber pixels"), faults)

    def test_a_held_read_taken_after_the_answer_was_sent_is_a_fault(self):
        self.assertEqual(probe.tray_faults(self.states(heldPending=0)),
                         ["the held answer was read with 0 questions on the host, not one held"])

    def test_undo_that_brings_back_neither_glyph_nor_tooltip_is_a_fault(self):
        self.assertEqual(probe.tray_faults(self.states(undone=self.CLEAR)),
                         ["undone: the glyph shows 0 amber pixels while a question waits",
                          "undone: the tooltip reads 'Guaca · 1 agent working'",
                          "undone: the count beside the glyph reads ''"])

    def test_the_poll_waits_for_the_tooltip_as_well_as_the_glyph(self):
        self.assertFalse(probe.tray_shows({**self.WAITING, "help": "Guaca · 1 agent working"}, True))
        self.assertTrue(probe.tray_shows(self.WAITING, True))
        self.assertFalse(probe.tray_shows({**self.CLEAR, "help": self.WAITING["help"]}, False))
        self.assertTrue(probe.tray_shows(self.CLEAR, False))

    def test_the_whole_round_trip_has_no_fault(self):
        self.assertEqual(probe.tray_faults(self.states()), [])


class ConnectTests(unittest.TestCase):
    def test_connect_waits_for_the_reloaded_page_to_show_the_workspace(self):
        documents = iter([1, 1, 1, 2, 2])
        shown = iter([0, 1])
        calls = []

        def page(window, do, **call):
            calls.append(do)
            if do == "ready":
                return {"loaded": "complete", "since": next(documents)}
            if do == "view":
                return {"agents": list(probe.AGENTS)}
            if do == "find":
                return {"count": next(shown)}
            return None

        app = Mock(client=Mock(page=Mock(side_effect=page)))
        probe.connect(app, Mock(origin="http://127.0.0.1:1", token="t"))
        self.assertEqual(calls, ["ready", "ready", "attach", "ready", "ready", "view", "find", "ready", "view", "find"])


class StandInTests(unittest.TestCase):
    def test_the_stand_in_keeps_the_answer_it_was_given(self):
        scratch = probe.Scratch()
        self.addCleanup(scratch.close)
        stand_in = probe.StandIn(scratch)
        asked = {"role": "user", "content": f"{probe.StandIn.ASK}: ask me something"}
        self.assertEqual(stand_in.answers, [])
        stand_in.answer([asked])
        self.assertEqual(stand_in.answers, [])
        stand_in.answer([asked, {"role": "assistant", "content": ""}, {"role": "tool", "content": "Second"}])
        self.assertEqual(stand_in.answers, ["Second"])
        stand_in.answer([{"role": "user", "content": "carry on"}])
        self.assertEqual(stand_in.answers, ["Second"])


class CrowdedSettingsTests(unittest.TestCase):
    def laid(self, list_height=640, banner_x=530, width=900, banners=1):
        return {"window": {"x": 320, "y": 65, "width": width, "height": 700, "layer": 0},
                "sections": [{"x": 330, "y": 110, "width": 186, "height": list_height}],
                "tabs": {name: [{"x": 338, "y": 120 + 40 * i, "width": 170, "height": 32}]
                         for i, name in enumerate(probe.SETTINGS_SECTIONS)},
                "banner": [{"x": banner_x, "y": 110, "width": 400, "height": 20}] * banners}

    def test_a_banner_over_the_section_list_is_a_fault(self):
        self.assertEqual(probe.crowded_faults("dark", self.laid(banner_x=340)),
                         ["dark: the no-key banner starts at 340, over the section list ending at 516"])

    def test_a_section_cut_off_by_a_shorter_list_is_a_fault(self):
        faults = probe.crowded_faults("light", self.laid(list_height=500))
        self.assertIn("light: About is cut off by the section list", faults)
        self.assertNotIn("light: General is cut off by the section list", faults)

    def test_a_window_that_kept_its_size_or_a_missing_banner_fails_instead_of_passing(self):
        self.assertEqual(probe.crowded_faults("light", self.laid(width=1280)), ["light: the window is 1280 by 700"])
        self.assertEqual(probe.crowded_faults("light", self.laid(banners=0)),
                         ["light: 1 section lists and 0 no-key banners on screen"])

    def test_every_section_in_the_list_with_the_banner_beside_it_has_no_fault(self):
        self.assertEqual(probe.crowded_faults("dark", self.laid()), [])


class BannerPressTests(unittest.TestCase):
    SCREEN = {"x": 0, "y": 0, "width": 1920, "height": 1080}

    def press(self, frames, refuse=False, how="press-group", absent=0):
        calls = []
        placed = iter(frames)

        def helper(*args):
            calls.append(args[0])
            if args[0] == "group-frame":
                return {"frame": {**next(placed), "name": "x"}, "screen": self.SCREEN}
            if calls.count(args[0]) <= absent:
                raise probe.Failed("nothing on screen names Guaca Probe")
            if refuse:
                raise probe.Failed("the press was refused (AXError -25204)")
            return {"pressed": "banner"}

        def polls(what, check, *_, **__):
            for _ in frames:
                value = check()
                if value:
                    return value
            raise probe.Failed(what)

        with patch.object(probe, "helper", side_effect=helper), \
             patch.object(probe, "notification_center", return_value=7), \
             patch.object(probe, "wait_until", side_effect=polls):
            try:
                return probe.press_when_shown("clicking the banner", how, "Guaca probe"), calls
            except probe.Failed as why:
                return why, calls

    def test_a_banner_still_sliding_in_is_never_pressed(self):
        sliding = [{"x": 2100 - 60 * n, "y": 142, "width": 344, "height": 74} for n in range(4)]
        result, calls = self.press(sliding)
        self.assertIsInstance(result, probe.Failed)
        self.assertNotIn("press-group", calls)

    def test_a_banner_moving_on_screen_is_never_pressed(self):
        moving = [{"x": 1560 - 60 * n, "y": 142, "width": 344, "height": 74} for n in range(4)]
        result, calls = self.press(moving)
        self.assertIsInstance(result, probe.Failed)
        self.assertNotIn("press-group", calls)

    def test_a_banner_half_off_screen_is_never_pressed_even_when_still(self):
        result, calls = self.press([{"x": 1786, "y": 142, "width": 344, "height": 74}] * 3)
        self.assertIsInstance(result, probe.Failed)
        self.assertNotIn("press-group", calls)

    def test_a_prompt_not_yet_on_screen_is_looked_for_again(self):
        result, calls = self.press([{}] * 4, how="press", absent=2)
        self.assertEqual(result, {"pressed": "banner"})
        self.assertEqual(calls, ["press", "press", "press"])

    def test_a_press_that_fails_is_never_repeated(self):
        still = {"x": 1560, "y": 142, "width": 344, "height": 74}
        result, calls = self.press([still] * 4, refuse=True)
        self.assertIsInstance(result, probe.Failed)
        self.assertEqual(calls.count("press-group"), 1)

    def test_a_settled_banner_is_pressed_once(self):
        moving = {"x": 1700, "y": 142, "width": 344, "height": 74}
        still = {"x": 1560, "y": 142, "width": 344, "height": 74}
        result, calls = self.press([moving, still, still, still])
        self.assertEqual(result, {"pressed": "banner"})
        self.assertEqual(calls.count("press-group"), 1)


if __name__ == "__main__":
    unittest.main()
