#!/usr/bin/env python3
"""Opt-in, read-only GitHub check with the PEM mounted only into the broker.
Requires built GUACAD_IMAGE and GUACA_GITHUB_BROKER_IMAGE, plus the same App
variables as github-app-live.py. Removes its containers, volumes and network.
"""
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import time
import urllib.request


def main():
    stem = "guaca-app-check-" + secrets.token_hex(4)
    runtime, broker = stem + "-runtime", stem + "-broker"
    client_volume, data_volume, user_volume = stem + "-client", stem + "-data", stem + "-users"
    authorization = secrets.token_hex(32)
    environment = dict(os.environ, GUACA_TOKEN=authorization)
    name = os.environ["GUACA_TEST_GITHUB_REPOSITORY"]

    def docker(*args, check=True):
        result = subprocess.run(["docker", *args], env=environment, capture_output=True, text=True, timeout=120)
        if check and result.returncode:
            raise RuntimeError("Docker check failed: " + result.stderr[:500])
        return result

    with tempfile.TemporaryDirectory(prefix="guaca-app-container-") as temporary:
        config = Path(temporary) / "config.json"
        config.write_text(json.dumps({"clientId": os.environ["GUACA_TEST_GITHUB_CLIENT_ID"],
                                     "installationId": int(os.environ["GUACA_TEST_GITHUB_INSTALLATION_ID"]),
                                     "repositories": [name], "privateKeyFile": "/run/secrets/github_private_key",
                                     "tokenFile": "/run/github-client/token", "userStateDir": "/var/lib/guaca-github/users", "initializeToken": True,
                                     "tokenUid": 1000, "listen": "0.0.0.0:8791"}))
        config.chmod(0o600)
        try:
            docker("network", "create", stem)
            for volume in (client_volume, data_volume, user_volume):
                docker("volume", "create", volume)
            docker("run", "-d", "--init", "--read-only", "--name", broker, "--network", stem,
                   "--mount", f"type=bind,source={config},target=/run/secrets/github_config,readonly",
                   "--mount", f"type=bind,source={os.environ['GUACA_TEST_GITHUB_PRIVATE_KEY_FILE']},target=/run/secrets/github_private_key,readonly",
                   "-v", client_volume + ":/run/github-client", "-v", user_volume + ":/var/lib/guaca-github", os.environ["GUACA_GITHUB_BROKER_IMAGE"])
            for _ in range(30):
                if docker("exec", broker, "python3", "-c", "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8791/health',timeout=1)", check=False).returncode == 0:
                    break
                time.sleep(1)
            else:
                raise RuntimeError("Credential service did not become healthy")
            docker("run", "-d", "--init", "--name", runtime, "--network", stem,
                   "-v", data_volume + ":/var/lib/guaca", "-v", client_volume + ":/run/github-client:ro",
                   "-e", "GUACA_TOKEN", "-e", "GUACA_GITHUB_BROKER=http://" + broker + ":8791",
                   "-e", "GUACA_GITHUB_BROKER_TOKEN_FILE=/run/github-client/token",
                   "-p", "127.0.0.1::8787", os.environ["GUACAD_IMAGE"])
            port = docker("port", runtime, "8787/tcp").stdout.strip().rsplit(":", 1)[1]
            base = "http://127.0.0.1:" + port
            for _ in range(50):
                try:
                    with urllib.request.urlopen(base + "/health", timeout=1):
                        break
                except OSError:
                    time.sleep(.2)
            else:
                raise RuntimeError("Runtime did not become healthy")

            def call(method, args=None):
                request = urllib.request.Request(base + "/v1/call", data=json.dumps({"name": method, "args": args or {}}).encode(),
                                                 headers={"Authorization": "Bearer " + authorization, "Content-Type": "application/json"})
                with urllib.request.urlopen(request, timeout=60) as response:
                    result = json.load(response)
                if "ok" not in result:
                    raise RuntimeError("Guaca container command failed: " + method)
                return result["ok"]
            groups = call("list_groups")
            group = groups[0] if groups else call("create_group", {"draft": {"name": "Container verification"}})
            repo = call("create_github_repository", {"draft": {"groupId": group["id"], "remote": "https://github.com/" + name + ".git", "harness": "claude", "bench": "own"}})
            assert call("repository_connection", {"id": repo["id"]})["githubApp"]
            author = {"name": "Container Engineer", "email": "engineer@example.com"}
            updated = call("set_repository_author", {"id": repo["id"], "author": author})
            assert updated["githubApp"] and updated["author"] == author
            # Local commit only. The read-only remote test never pushes a ref.
            docker("exec", "-w", repo["path"], runtime, "git", "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-m", "test: container commit attribution")
            actual = docker("exec", "-w", repo["path"], runtime, "git", "log", "-1", "--format=%an <%ae>").stdout.strip()
            assert actual == "Container Engineer <engineer@example.com>"
            assert "succeeded" in call("check_repository_connection", {"id": repo["id"]})
            connection = docker("exec", "-w", repo["path"], runtime, "git", "config", "--get", "guaca.githubConnection").stdout.strip()
            helper = str(Path(connection).parent / "github-helper.py")
            assert call("repository_github_user", {"id": repo["id"]})["status"] == "signedOut"
            unsigned = docker("exec", "-w", repo["path"], runtime, "python3", helper, "gh", "api", "repos/" + name, "--jq", ".full_name", check=False)
            assert unsigned.returncode != 0, "gh must not fall back to the bot before user authorization"
            login = docker("exec", "-w", repo["path"], runtime, "/bin/bash", "-lc", "gh api user", check=False)
            assert login.returncode != 0 and "GitHub user access is unavailable" in login.stderr, "login shells must use the App helper and require user authorization"
            docker("exec", runtime, "test", "!", "-e", "/run/secrets/github_private_key")
            mounts = json.loads(docker("inspect", runtime, "--format", "{{json .Mounts}}").stdout)
            assert all("private-key" not in m["Source"] and "private_key" not in m["Destination"] for m in mounts)
            assert all(m.get("Name") != user_volume for m in mounts)
            docker("exec", runtime, "ssh", "-V")
            print(json.dumps({"repository": name, "checks": ["container App clone", "configured commit author", "Git read/push dry run", "gh requires user authorization", "PEM and user grants absent from runtime mounts", "SSH installed"]}))
        finally:
            docker("rm", "-f", runtime, broker, check=False)
            docker("volume", "rm", client_volume, data_volume, user_volume, check=False)
            docker("network", "rm", stem, check=False)


if __name__ == "__main__":
    main()
