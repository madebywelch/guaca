#!/usr/bin/env python3
"""Opt-in live App check on a disposable repository; creates a branch and draft PR.
No model calls. Credentials stay in process memory or outside the checkout.
"""
import importlib.util
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request
from http.server import ThreadingHTTPServer

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("github_app", ROOT / "deploy/github/github_app.py")
github = importlib.util.module_from_spec(spec)
spec.loader.exec_module(github)


def main():
    name = github.repository(os.environ["GUACA_TEST_GITHUB_REPOSITORY"])
    daemon = os.environ["GUACAD"]
    with tempfile.TemporaryDirectory(prefix="guaca-github-live-") as temporary:
        root = Path(temporary)
        broker_auth = root / "broker-token"
        broker_auth.write_text(secrets.token_hex(32))
        broker_auth.chmod(0o600)
        broker = github.Broker({"clientId": os.environ["GUACA_TEST_GITHUB_CLIENT_ID"],
                                "installationId": int(os.environ["GUACA_TEST_GITHUB_INSTALLATION_ID"]),
                                "privateKeyFile": os.environ["GUACA_TEST_GITHUB_PRIVATE_KEY_FILE"],
                                "tokenFile": str(broker_auth), "repositories": [name]})
        service = ThreadingHTTPServer(("127.0.0.1", 0), github.handler(broker))
        worker = threading.Thread(target=service.serve_forever, daemon=True)
        worker.start()
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        workspace_token = secrets.token_hex(32)
        environment = dict(os.environ, GUACA_ROOT=str(root / "workspace"), GUACA_BIND=f"127.0.0.1:{port}",
                           GUACA_TOKEN=workspace_token, GUACA_WEB=str(ROOT / "dist"),
                           GUACA_GITHUB_BROKER=f"http://127.0.0.1:{service.server_port}",
                           GUACA_GITHUB_BROKER_TOKEN_FILE=str(broker_auth))
        # Model and global GitHub credentials cannot make this test pass accidentally.
        for key in ("GH_TOKEN", "GITHUB_TOKEN", "OPENAI_API_KEY", "CODEX_API_KEY", "ANTHROPIC_API_KEY", "OPENROUTER_API_KEY"):
            environment.pop(key, None)
        running = subprocess.Popen([daemon], env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            base = f"http://127.0.0.1:{port}"
            for _ in range(100):
                try:
                    with urllib.request.urlopen(base + "/health", timeout=1):
                        break
                except OSError:
                    time.sleep(.1)
            else:
                raise RuntimeError("Daemon did not start")

            def call(method, args=None):
                payload = json.dumps({"name": method, "args": args or {}}).encode()
                request = urllib.request.Request(base + "/v1/call", data=payload, headers={"Authorization": "Bearer " + workspace_token, "Content-Type": "application/json"})
                with urllib.request.urlopen(request, timeout=60) as response:
                    result = json.load(response)
                if "ok" not in result:
                    raise RuntimeError(f"Guaca command failed: {method}")
                return result["ok"]

            groups = call("list_groups")
            group = groups[0] if groups else call("create_group", {"draft": {"name": "GitHub App verification"}})
            remote = "https://github.com/" + name + ".git"
            repo = call("create_github_repository", {"draft": {"groupId": group["id"], "name": "GitHub App verification", "remote": remote, "harness": "codex", "bench": "own", "gate": "askBeforePushing"}})
            checkout = Path(repo["path"])

            def git(args, cwd=checkout, allow=False):
                result = subprocess.run(["git", "-C", str(cwd), *args], capture_output=True, text=True, env=environment, timeout=60)
                if not allow and result.returncode:
                    raise RuntimeError(f"git {args[0]} failed (exit {result.returncode})")
                return result

            # Inspect origin before the test's remote writes. Never use the developer's origin.
            print(git(["remote", "-v"]).stdout.strip(), flush=True)
            assert git(["remote", "get-url", "origin"]).stdout.strip() == remote
            token = broker.token(name)["token"]
            info = github.exchange("https://api.github.com/repos/" + name, token)
            branch_base = info["default_branch"]
            if git(["rev-parse", "--verify", "HEAD"], allow=True).returncode:
                # An empty disposable repository needs a base before GitHub can open a PR.
                (checkout / "README.md").write_text("Disposable repository for Guaca GitHub App integration checks.\n")
                git(["checkout", "-B", branch_base])
                git(["add", "README.md"])
                git(["-c", "commit.gpgsign=false", "commit", "-m", "test: initialize GitHub App fixture"])
                git(["push", "origin", branch_base])
            else:
                git(["fetch", "origin"])
                git(["checkout", branch_base])
                git(["pull", "--ff-only", "origin", branch_base])
            branch = "guaca-app-check-" + time.strftime("%Y%m%d-%H%M%S")
            bench = root / "engineer-worktree"
            git(["worktree", "add", "-b", branch, str(bench), "origin/" + branch_base])
            (bench / "guaca-app-check.txt").write_text("GitHub App authenticated clone, worktree push, and pull request.\n")
            git(["add", "guaca-app-check.txt"], bench)
            git(["-c", "commit.gpgsign=false", "commit", "-m", "test: verify Guaca GitHub App access"], bench)
            # Invalidate to prove a later Git command can obtain a replacement token.
            broker.invalidate(name)
            git(["push", "-u", "origin", branch], bench)
            git(["fetch", "origin"], bench)
            connection = git(["config", "--get", "guaca.githubConnection"], bench).stdout.strip()
            helper = Path(connection).parent / "github-helper.py"
            body = root / "pr-body.md"
            body.write_text("Verifies Guaca's GitHub App credentials can clone a private repository, push from an engineer worktree, and create a pull request.\n\nThis is a disposable integration check; no merge is needed.\n")
            gh_environment = dict(environment, GUACA_GH_BINARY=shutil.which("gh"))
            created = subprocess.run(["python3", str(helper), "gh", "pr", "create", "--draft", "--base", branch_base, "--head", branch,
                                      "--title", "test: verify Guaca GitHub App integration", "--body-file", str(body)],
                                     cwd=bench, env=gh_environment, capture_output=True, text=True, timeout=60)
            if created.returncode:
                raise RuntimeError("GitHub App pull-request creation failed: " + created.stderr[:500])
            url = created.stdout.strip()
            assert url.startswith("https://github.com/" + name + "/pull/")
            pr = github.exchange("https://api.github.com/repos/" + name + "/pulls/" + url.rsplit("/", 1)[1], broker.token(name)["token"])
            assert pr["draft"] and pr["user"]["type"] == "Bot"
            assert call("repository_connection", {"id": repo["id"]})["githubApp"]
            assert "succeeded" in call("check_repository_connection", {"id": repo["id"]})
            call("clear_repository_credential", {"id": repo["id"]})
            assert git(["fetch", "origin"], bench, allow=True).returncode != 0
            print(json.dumps({"installationId": broker.installation, "repository": name, "pullRequest": url, "actor": pr["user"]["login"], "checks": ["daemon App clone", "worktree push", "token replacement", "bot draft PR", "read/push check", "disconnect"]}), flush=True)
        finally:
            running.terminate()
            try:
                running.wait(timeout=15)
            except subprocess.TimeoutExpired:
                running.kill()
                running.wait()
            service.shutdown()
            service.server_close()
            worker.join()


if __name__ == "__main__":
    main()
