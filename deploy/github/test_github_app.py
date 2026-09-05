import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("github_app", Path(__file__).with_name("github_app.py"))
github = importlib.util.module_from_spec(spec)
spec.loader.exec_module(github)


class GitHubAppTest(unittest.TestCase):
    def test_cli_forwards_github_flags_without_parsing_or_reordering(self):
        for args in (["--version"], ["--help"], ["--repo", "owner/repo", "pr", "list"],
                     ["api", "user", "--jq", ".login"], []):
            with self.subTest(args=args), patch("sys.argv", ["github-helper.py", "gh", *args]), patch.object(github, "gh") as command:
                github.main()
                command.assert_called_once_with(args)

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.auth = self.root / "auth"
        self.auth.write_text("a" * 64)
        self.broker = github.Broker({"clientId": "test-client", "installationId": 42,
                                    "privateKeyFile": "/not-mounted-in-worker", "tokenFile": str(self.auth),
                                    "repositories": ["owner/repo"]})
        self.calls = []
        self.installed = 42
        self.granted = ["owner/repo"]
        self.broker.jwt = lambda: "test-jwt"
        self.expires = "2099-01-01T00:00:00Z"
        self.patcher = patch.object(github, "exchange", self.api)
        self.patcher.start()

    def tearDown(self):
        self.patcher.stop()
        self.directory.cleanup()

    def api(self, url, token, method="GET", body=None):
        self.calls.append((url, method, body))
        if url.endswith("/installation"):
            return {"id": self.installed, "suspended_at": None}
        return {"token": "installation-secret", "expires_at": self.expires,
                "repositories": [{"full_name": r} for r in self.granted], "permissions": {"contents": "write"}}

    def test_denies_unknown_repository_before_contacting_github(self):
        for name in ("other/repo", "owner/another", "../repo", "owner/repo?token=secret"):
            with self.assertRaises(github.Failure):
                self.broker.token(name)
        self.assertEqual(self.calls, [])

    def test_installation_mismatch_never_mints(self):
        self.installed = 43
        with self.assertRaises(github.Failure):
            self.broker.token("owner/repo")
        self.assertEqual(len(self.calls), 1)

    def test_one_repo_tokens_cache_refresh_and_invalidate(self):
        first = self.broker.token("Owner/Repo.git")
        self.assertEqual(self.calls[-1][2], {"repositories": ["repo"]})
        self.assertEqual(self.broker.token("owner/repo"), first)
        self.assertEqual(len(self.calls), 2)
        self.broker.cache["owner/repo"] = (time.time() + 200, first)
        self.broker.token("owner/repo")
        self.assertEqual(len(self.calls), 4)
        self.broker.invalidate("owner/repo")
        self.broker.token("owner/repo")
        self.assertEqual(len(self.calls), 6)

    def test_wider_or_expired_tokens_are_rejected(self):
        self.granted = ["owner/repo", "owner/another"]
        with self.assertRaises(github.Failure):
            self.broker.token("owner/repo")
        self.granted = ["owner/repo"]
        self.expires = "2000-01-01T00:00:00Z"
        with self.assertRaises(github.Failure):
            self.broker.token("owner/repo")
        self.assertFalse(self.broker.cache)

    def test_revocation_fails_after_invalidation_without_stale_fallback(self):
        self.broker.token("owner/repo")
        self.broker.invalidate("owner/repo")
        with patch.object(github, "exchange", side_effect=github.Failure("Access revoked")):
            with self.assertRaisesRegex(github.Failure, "revoked"):
                self.broker.token("owner/repo")
        self.assertFalse(self.broker.cache)

    def test_git_helper_binds_host_and_path_and_does_not_store_tokens(self):
        file = self.root / "connection.json"
        file.write_text(json.dumps({"url": "http://127.0.0.1:1", "tokenFile": str(self.auth), "repository": "owner/repo"}))
        with patch.object(github, "request_token", return_value="temporary-password") as token:
            for source in ("protocol=https\nhost=evil.test\npath=owner/repo.git\n", "protocol=https\nhost=github.com\npath=other/repo\n"):
                with patch("sys.stdin", io.StringIO(source)), self.assertRaises(github.Failure):
                    github.credential(file, "get")
            token.assert_not_called()
            output = io.StringIO()
            with patch("sys.stdin", io.StringIO("protocol=https\nhost=github.com\npath=owner/repo.git\n")), contextlib.redirect_stdout(output):
                github.credential(file, "get")
            self.assertEqual(output.getvalue(), "username=x-access-token\npassword=temporary-password\n\n")
            github.credential(file, "store")
            self.assertNotIn("temporary-password", file.read_text())

    def test_http_authentication_and_error_redaction(self):
        # Exercise actual HTTP framing and authentication, bypassing the mocked API client.
        self.patcher.stop()
        server = ThreadingHTTPServer(("127.0.0.1", 0), github.handler(self.broker))
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            url = f"http://127.0.0.1:{server.server_port}/v1/token"
            with self.assertRaisesRegex(github.Failure, "401"):
                github.exchange(url, "wrong-secret", "POST", {"repository": "owner/repo"})
            with self.assertRaisesRegex(github.Failure, "403"):
                github.exchange(url, "a" * 64, "POST", {"repository": "other/repo"})
        finally:
            server.shutdown()
            server.server_close()
            thread.join()
            self.patcher.start()

    def test_jwt_signature_with_generated_fixture_key(self):
        # Verify the signature, issuer and bounded claims using a disposable RSA key.
        import base64
        key, public, signed = [self.root / p for p in ("key.pem", "public.pem", "signature")]
        subprocess.run(["openssl", "genrsa", "-out", str(key), "2048"], capture_output=True, check=True)
        subprocess.run(["openssl", "rsa", "-in", str(key), "-pubout", "-out", str(public)], capture_output=True, check=True)
        self.broker.config["privateKeyFile"] = str(key)
        jwt = github.Broker.jwt(self.broker)
        header, payload, signature = jwt.split(".")
        decode = lambda s: base64.urlsafe_b64decode(s + "=" * (-len(s) % 4))
        claims = json.loads(decode(payload))
        self.assertEqual(claims["iss"], "test-client")
        self.assertLessEqual(claims["exp"] - int(time.time()), 600)
        signed.write_bytes(decode(signature))
        verified = subprocess.run(["openssl", "dgst", "-sha256", "-verify", str(public), "-signature", str(signed)], input=(header + "." + payload).encode(), capture_output=True)
        self.assertEqual(verified.returncode, 0)


if __name__ == "__main__":
    unittest.main()
