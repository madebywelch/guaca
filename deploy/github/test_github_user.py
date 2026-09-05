import importlib.util
import json
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("github_app", Path(__file__).with_name("github_app.py"))
github = importlib.util.module_from_spec(spec)
spec.loader.exec_module(github)


class GitHubUserTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        (root / "authorization").write_text("a" * 64)
        self.config = {"clientId": "fixture-client", "installationId": 42,
                       "privateKeyFile": "/not-read", "tokenFile": str(root / "authorization"),
                       "userStateDir": str(root / "private-users"), "repositories": ["owner/repo"]}
        self.broker = github.Broker(self.config)
        self.broker.token = lambda name: {"token": "installation-fixture"}
        self.users = self.broker.users
        self.scope = ["owner/repo"]
        self.user_id = 7
        self.write = True
        self.answer = {"access_token": "user-fixture", "token_type": "bearer", "expires_in": 28800,
                       "refresh_token": "refresh-fixture", "refresh_token_expires_in": 15897600}
        self.oauth_calls = []
        self.users.oauth = self.oauth
        self.patcher = patch.object(github, "exchange", self.api)
        self.patcher.start()
        self.addCleanup(self.patcher.stop)

    def api(self, url, token, method="GET", body=None):
        if url.endswith("/user"):
            return {"id": self.user_id, "login": "human", "type": "User"}
        if "/user/installations/42/repositories" in url:
            return {"total_count": len(self.scope), "repositories": [{"full_name": name} for name in self.scope]}
        if url.endswith("/repos/owner/repo"):
            return {"id": 99, "permissions": {"push": self.write}}
        raise AssertionError("Unexpected fixture API route")

    def oauth(self, path, data):
        self.oauth_calls.append((path, data))
        if path.endswith("/device/code"):
            return {"device_code": "private-device-fixture", "user_code": "USER-CODE", "verification_uri": "https://github.com/login/device", "interval": 5, "expires_in": 900}
        return dict(self.answer)

    def authorize(self):
        flow = self.users.start("owner/repo")
        self.users.pending[flow["flowId"]]["next"] = 0
        result = self.users.poll("owner/repo", flow["flowId"])
        return flow, result

    def test_scope_and_write_failures_never_save_a_user_token(self):
        for scope, write in [(["owner/repo", "owner/other"], True), ([], True), (["owner/repo"], False)]:
            self.scope, self.write = scope, write
            with self.assertRaises(github.Failure):
                self.authorize()
            self.assertIsNone(self.users.read("owner/repo"))
            with self.assertRaises(github.Failure):
                self.users.token("owner/repo")

    def test_codes_are_bound_to_one_repository_and_polling_honors_backoff(self):
        flow = self.users.start("owner/repo")
        shown = json.dumps(flow)
        self.assertNotIn("private-device-fixture", shown)
        self.assertNotIn("access_token", shown)
        self.assertEqual(self.users.poll("owner/repo", flow["flowId"])["status"], "pending")
        self.assertEqual(len(self.oauth_calls), 1)
        with self.assertRaises(github.Failure):
            self.users.poll("owner/other", flow["flowId"])
        self.answer = {"error": "slow_down"}
        pending = self.users.pending[flow["flowId"]]
        pending["next"] = 0
        self.assertEqual(self.users.poll("owner/repo", flow["flowId"])["interval"], 10)
        self.assertGreater(pending["next"], time.time() + 8)
        self.assertEqual(self.oauth_calls[-1][1]["repository_id"], 99)

    def test_denied_expired_and_replaced_flows_cannot_authorize(self):
        first = self.users.start("owner/repo")
        second = self.users.start("owner/repo")
        with self.assertRaises(github.Failure):
            self.users.poll("owner/repo", first["flowId"])
        self.users.pending[second["flowId"]]["expires"] = 0
        with self.assertRaises(github.Failure):
            self.users.poll("owner/repo", second["flowId"])
        self.answer = {"error": "access_denied"}
        with self.assertRaises(github.Failure):
            self.authorize()
        self.assertEqual(self.users.status("owner/repo")["status"], "signedOut")

    def test_human_profile_and_tokens_survive_restart_without_exposing_secrets(self):
        _, status = self.authorize()
        self.assertEqual(status["author"], {"name": "human", "email": "7+human@users.noreply.github.com"})
        self.assertNotIn("user-fixture", json.dumps(status))
        self.assertNotIn("refresh-fixture", json.dumps(status))
        self.assertEqual(self.users.file("owner/repo").stat().st_mode & 0o777, 0o600)
        restarted = github.Broker(self.config).users
        self.assertEqual(restarted.status("owner/repo"), status)
        self.assertEqual(restarted.token("owner/repo")["token"], "user-fixture")

    def test_expiring_tokens_refresh_once_and_rotate_both_credentials(self):
        self.authorize()
        grant = self.users.read("owner/repo")
        grant["expires"] = 0
        self.users.save("owner/repo", grant)
        self.answer.update(access_token="replacement-user", refresh_token="replacement-refresh")
        self.assertEqual(self.users.token("owner/repo")["token"], "replacement-user")
        self.assertEqual(self.users.token("owner/repo")["token"], "replacement-user")
        refresh = [data for _, data in self.oauth_calls if data.get("grant_type") == "refresh_token"]
        self.assertEqual(refresh, [{"grant_type": "refresh_token", "refresh_token": "refresh-fixture"}])
        self.assertEqual(self.users.read("owner/repo")["refreshToken"], "replacement-refresh")

    def test_failed_refresh_never_falls_back_to_bot_or_stale_token(self):
        self.authorize()
        grant = self.users.read("owner/repo")
        grant["expires"] = 0
        self.users.save("owner/repo", grant)
        self.answer = {"error": "bad_refresh_token"}
        with self.assertRaises(github.Failure):
            self.users.token("owner/repo")
        self.users.disconnect("owner/repo")
        self.assertEqual(self.users.status("owner/repo")["status"], "signedOut")
        with self.assertRaises(github.Failure):
            self.users.token("owner/repo")

    def test_refresh_cannot_change_account_or_expand_repository_scope(self):
        self.authorize()
        grant = self.users.read("owner/repo")
        grant["expires"] = 0
        self.users.save("owner/repo", grant)
        self.user_id = 8
        with self.assertRaises(github.Failure):
            self.users.token("owner/repo")
        self.user_id = 7
        self.scope.append("owner/other")
        with self.assertRaises(github.Failure):
            self.users.token("owner/repo")

    def test_repository_names_cannot_collide_in_persistent_storage(self):
        self.broker.allowed.update(["a--b/c", "a/b--c"])
        self.assertNotEqual(self.users.file("a--b/c"), self.users.file("a/b--c"))


if __name__ == "__main__":
    unittest.main()
