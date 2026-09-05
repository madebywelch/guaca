#!/usr/bin/env python3
"""GitHub App broker and its Git/gh clients. The PEM belongs only to the broker."""
import argparse
import base64
import datetime
import hmac
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.error import HTTPError, URLError
from urllib.parse import urlparse
from urllib.request import HTTPRedirectHandler, Request, build_opener


class Failure(Exception):
    pass


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def exchange(url, token, method="GET", body=None):
    headers = {"Authorization": "Bearer " + token, "User-Agent": "guaca-github",
               "Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28"}
    data = None if body is None else json.dumps(body).encode()
    if data is not None:
        headers["Content-Type"] = "application/json"
    try:
        with build_opener(NoRedirect()).open(Request(url, data=data, headers=headers, method=method), timeout=25) as response:
            return json.load(response)
    except HTTPError as error:
        # Upstream bodies and request headers can contain credentials. Never reflect them.
        status = error.code
        error.close()
        raise Failure(f"GitHub authorization request failed (HTTP {status}); check installation access and permissions") from None
    except (URLError, OSError, ValueError):
        raise Failure("Could not reach the credential service or GitHub") from None


def repository(value):
    if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", value):
        raise Failure("Use a GitHub owner/repository name")
    owner, name = value.split("/")
    if owner in (".", "..") or name in (".", ".."):
        raise Failure("Invalid repository name")
    if name.endswith(".git"):
        name = name[:-4]
    if not name or name in (".", ".."):
        raise Failure("Invalid repository name")
    return (owner + "/" + name).lower()


def remote_repository(remote):
    parsed = urlparse(remote)
    if parsed.scheme != "https" or parsed.netloc.lower() != "github.com" or parsed.query or parsed.fragment:
        raise Failure("GitHub App access requires an https://github.com/owner/repository origin")
    return repository(parsed.path.removeprefix("/"))


class UserGrants:
    """One human authorization per repository in this trusted workspace.

    Refresh tokens stay on the broker's private persistent volume. A device
    flow never needs a client secret. No failure falls back to a bot actor.
    """
    def __init__(self, broker):
        self.broker = broker
        self.pending = {}
        self.lock = threading.RLock()
        self.base = broker.config.get("oauthUrl", "https://github.com").rstrip("/")
        if self.base != "https://github.com" and not re.fullmatch(r"http://(127\.0\.0\.1|localhost):[0-9]+", self.base):
            raise Failure("GitHub OAuth must use github.com; loopback is allowed for offline tests")

    def file(self, name):
        name = repository(name)
        if name not in self.broker.allowed:
            raise Failure("Repository is outside this workspace's GitHub allowlist")
        directory = self.broker.config.get("userStateDir")
        if not directory:
            raise Failure("Configure a private persistent userStateDir on the GitHub broker")
        return Path(directory) / (hashlib.sha256(name.encode()).hexdigest() + ".json")

    def oauth(self, path, data):
        data = dict(data, client_id=str(self.broker.config["clientId"]))
        request = Request(self.base + path, data=json.dumps(data).encode(), method="POST",
                          headers={"Accept": "application/json", "Content-Type": "application/json", "User-Agent": "guaca-github"})
        try:
            with build_opener(NoRedirect()).open(request, timeout=25) as response:
                answer = json.load(response)
            if not isinstance(answer, dict):
                raise ValueError()
            return answer
        except HTTPError as error:
            error.close()
            raise Failure("GitHub user authorization request failed") from None
        except (URLError, OSError, ValueError):
            raise Failure("Could not complete GitHub user authorization") from None

    def read(self, name):
        file = self.file(name)
        try:
            value = json.loads(file.read_text())
            if value["repository"] != name or value["clientId"] != str(self.broker.config["clientId"]) or value["installationId"] != self.broker.installation:
                raise ValueError()
            return value
        except FileNotFoundError:
            return None
        except (OSError, KeyError, ValueError):
            raise Failure("GitHub user authorization is unavailable; sign in again") from None

    def save(self, name, value):
        file = self.file(name)
        file.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        temporary = file.with_name("." + secrets.token_hex(16))
        try:
            descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, "w") as output:
                json.dump(value, output)
                output.flush()
                os.fsync(output.fileno())
            os.replace(temporary, file)
        finally:
            temporary.unlink(missing_ok=True)

    @staticmethod
    def public(grant):
        if not grant:
            return {"status": "signedOut", "login": None, "author": None}
        return {"status": "authorized", "login": grant["login"],
                "author": {"name": grant["login"], "email": f'{grant["userId"]}+{grant["login"]}@users.noreply.github.com'}}

    def status(self, name):
        with self.lock:
            return self.public(self.read(repository(name)))

    def start(self, name):
        name = repository(name)
        self.file(name)
        with self.lock:
            info = exchange(f"{self.broker.api}/repos/{name}", self.broker.token(name)["token"])
            answer = self.oauth("/login/device/code", {})
            if answer.get("error"):
                raise Failure("Enable Device flow in this GitHub App's settings, then try again")
            if answer.get("verification_uri") != "https://github.com/login/device":
                raise Failure("GitHub returned an unexpected verification address")
            now = time.time()
            self.pending = {key: flow for key, flow in self.pending.items() if flow["expires"] > now and flow["repository"] != name}
            if len(self.pending) >= 32:
                raise Failure("Too many GitHub sign-ins are waiting; finish or let them expire")
            handle = secrets.token_urlsafe(32)
            interval = max(5, int(answer["interval"]))
            expires = min(900, int(answer["expires_in"]))
            self.pending[handle] = {"repository": name, "repositoryId": info["id"], "device": answer["device_code"],
                                    "expires": now + expires, "next": now + interval, "interval": interval}
            return {"flowId": handle, "userCode": answer["user_code"], "verificationUri": answer["verification_uri"],
                    "expiresIn": expires, "interval": interval}

    def checked_grant(self, name, answer):
        if not answer.get("access_token") or answer.get("token_type", "").lower() != "bearer":
            raise Failure("GitHub returned no user access token")
        token = answer["access_token"]
        user = exchange(self.broker.api + "/user", token)
        if user.get("type") != "User" or not isinstance(user.get("id"), int) or not re.fullmatch(r"[A-Za-z0-9-]+", user.get("login", "")):
            raise Failure("Authorize GitHub with a personal user account")
        # repository_id can be ignored when the user lacks access. Verify the
        # resulting grant is scoped to exactly the requested repository.
        scope = exchange(f"{self.broker.api}/user/installations/{self.broker.installation}/repositories?per_page=100", token)
        if scope.get("total_count") != 1 or {repository(r["full_name"]) for r in scope.get("repositories", [])} != {name}:
            raise Failure("GitHub did not restrict user authorization to this repository")
        info = exchange(f"{self.broker.api}/repos/{name}", token)
        if not info.get("permissions", {}).get("push"):
            raise Failure("Your GitHub account needs write access to this repository")
        now = time.time()
        expires = now + int(answer.get("expires_in", 0)) if "expires_in" in answer else None
        if expires is not None and expires <= now + 60:
            raise Failure("GitHub returned a nearly expired user token")
        return {"repository": name, "clientId": str(self.broker.config["clientId"]), "installationId": self.broker.installation,
                "login": user["login"], "userId": user["id"], "accessToken": token, "expires": expires,
                "refreshToken": answer.get("refresh_token"), "refreshExpires": now + int(answer.get("refresh_token_expires_in", 0))}

    def poll(self, name, handle):
        name = repository(name)
        self.file(name)
        with self.lock:
            flow = self.pending.get(handle)
            if not flow or flow["repository"] != name:
                raise Failure("GitHub sign-in is no longer pending; start again")
            now = time.time()
            if now >= flow["expires"]:
                del self.pending[handle]
                raise Failure("GitHub sign-in expired; start again")
            if now < flow["next"]:
                return {"status": "pending", "interval": flow["interval"]}
            answer = self.oauth("/login/oauth/access_token", {"device_code": flow["device"], "grant_type": "urn:ietf:params:oauth:grant-type:device_code", "repository_id": flow["repositoryId"]})
            if answer.get("error") in ("authorization_pending", "slow_down"):
                if answer["error"] == "slow_down":
                    flow["interval"] += 5
                flow["next"] = time.time() + flow["interval"]
                return {"status": "pending", "interval": flow["interval"]}
            del self.pending[handle]
            if answer.get("error"):
                raise Failure("GitHub sign-in was denied or expired; start again")
            grant = self.checked_grant(name, answer)
            self.save(name, grant)
            return self.public(grant)

    def token(self, name):
        name = repository(name)
        with self.lock:
            grant = self.read(name)
            if not grant:
                raise Failure("Sign in to GitHub under Git access before opening a pull request")
            if grant["expires"] is not None and grant["expires"] <= time.time() + 300:
                if not grant.get("refreshToken") or grant["refreshExpires"] <= time.time():
                    raise Failure("GitHub user authorization expired; sign in again under Git access")
                answer = self.oauth("/login/oauth/access_token", {"grant_type": "refresh_token", "refresh_token": grant["refreshToken"]})
                if answer.get("error"):
                    raise Failure("GitHub user authorization was revoked or expired; sign in again")
                updated = self.checked_grant(name, answer)
                if updated["userId"] != grant["userId"]:
                    raise Failure("GitHub changed the authorized user; sign in again")
                self.save(name, updated)
                grant = updated
            return {"token": grant["accessToken"]}

    def disconnect(self, name):
        name = repository(name)
        with self.lock:
            self.file(name).unlink(missing_ok=True)
            self.pending = {key: flow for key, flow in self.pending.items() if flow["repository"] != name}
            return self.public(None)


class Broker:
    def __init__(self, config):
        self.config = config
        token_file = Path(config["tokenFile"])
        if config.get("initializeToken") and not token_file.exists():
            token_file.parent.mkdir(parents=True, exist_ok=True)
            descriptor = os.open(token_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, "w") as output:
                output.write(secrets.token_hex(32))
            if "tokenUid" in config:
                os.chown(token_file, int(config["tokenUid"]), int(config["tokenUid"]))
        self.authorization = token_file.read_text().strip()
        if len(self.authorization) < 32:
            raise Failure("Broker authentication token must contain at least 32 characters")
        self.installation = int(config["installationId"])
        self.api = config.get("apiUrl", "https://api.github.com").rstrip("/")
        parsed = urlparse(self.api)
        if self.api != "https://api.github.com" and not (parsed.scheme == "http" and parsed.hostname in ("127.0.0.1", "localhost") and not parsed.username and not parsed.query):
            raise Failure("GitHub API must be api.github.com; loopback is allowed for offline tests")
        self.allowed = {repository(r) for r in config.get("repositories", [])}
        if not self.allowed:
            raise Failure("Configure the repositories this workspace may access")
        self.cache = {}
        self.lock = threading.Lock()
        self.users = UserGrants(self)

    def jwt(self):
        def b64(raw):
            return base64.urlsafe_b64encode(raw).rstrip(b"=")
        now = int(time.time())
        claims = {"iat": now - 60, "exp": now + 300, "iss": str(self.config["clientId"])}
        signing = b64(b'{"alg":"RS256","typ":"JWT"}') + b"." + b64(json.dumps(claims).encode())
        try:
            signed = subprocess.run(["openssl", "dgst", "-sha256", "-sign", self.config["privateKeyFile"]],
                                    input=signing, capture_output=True, timeout=10, check=True)
        except (OSError, subprocess.SubprocessError):
            raise Failure("Could not sign GitHub App authentication; check the private key") from None
        return (signing + b"." + b64(signed.stdout)).decode()

    def status(self):
        installation = exchange(f"{self.api}/app/installations/{self.installation}", self.jwt())
        if installation.get("suspended_at"):
            raise Failure("The GitHub App installation is suspended")
        return {"installationId": self.installation, "account": installation["account"]["login"],
                "repositories": sorted(self.allowed), "permissions": installation["permissions"]}

    def token(self, name):
        name = repository(name)
        if name not in self.allowed:
            raise Failure("This repository is not authorized for this workspace's credential service")
        with self.lock:
            cached = self.cache.get(name)
            if cached and cached[0] > time.time() + 300:
                return cached[1]
            jwt = self.jwt()
            installation = exchange(f"{self.api}/repos/{name}/installation", jwt)
            if installation.get("id") != self.installation or installation.get("suspended_at"):
                raise Failure("Repository does not belong to the configured active GitHub App installation")
            # The installation check binds owner as well as name. Mint for ONE repository.
            answer = exchange(f"{self.api}/app/installations/{self.installation}/access_tokens", jwt,
                              "POST", {"repositories": [name.split("/")[1]]})
            granted = {repository(r["full_name"]) for r in answer.get("repositories", [])}
            if granted != {name}:
                raise Failure("GitHub did not return a token scoped to exactly the requested repository")
            if answer.get("permissions", {}).get("contents") != "write":
                raise Failure("GitHub App needs Contents: read and write for coding repositories")
            expires = datetime.datetime.fromisoformat(answer["expires_at"].replace("Z", "+00:00")).timestamp()
            if expires <= time.time() + 300 or not answer.get("token"):
                raise Failure("GitHub returned a missing or nearly expired installation token")
            result = {"token": answer["token"], "expiresAt": answer["expires_at"]}
            self.cache[name] = (expires, result)
            return result

    def invalidate(self, name):
        with self.lock:
            self.cache.pop(repository(name), None)


def handler(broker):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            self.handle_api()

        def do_POST(self):
            self.handle_api()

        def handle_api(self):
            self.connection.settimeout(30)
            if self.command == "GET" and self.path == "/health":
                self.respond(200, {"status": "ok"})
                return
            if not hmac.compare_digest(self.headers.get("Authorization", "").encode(), ("Bearer " + broker.authorization).encode()):
                self.respond(401, {"error": "Credential service authentication failed"})
                return
            try:
                if self.command == "GET" and self.path == "/v1/status":
                    self.respond(200, broker.status())
                    return
                size = int(self.headers.get("Content-Length", "0"))
                if self.command != "POST" or self.path not in ("/v1/token", "/v1/invalidate", "/v1/user/start", "/v1/user/poll", "/v1/user/status", "/v1/user/token", "/v1/user/disconnect") or not 0 < size <= 4096:
                    self.respond(400, {"error": "Invalid credential request"})
                    return
                data = json.loads(self.rfile.read(size))
                name = data["repository"]
                if self.path.startswith("/v1/user/"):
                    action = self.path.rsplit("/", 1)[1]
                    result = broker.users.poll(name, data["flowId"]) if action == "poll" else getattr(broker.users, action)(name)
                    self.respond(200, result)
                    return
                if self.path == "/v1/invalidate":
                    broker.invalidate(name)
                    self.respond(200, {})
                else:
                    self.respond(200, broker.token(name))
            except Failure as error:
                self.respond(403, {"error": str(error)})
            except (KeyError, TypeError, ValueError, OSError):
                self.respond(400, {"error": "Invalid credential request or response"})

        def respond(self, code, value):
            payload = json.dumps(value).encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
    return Handler


def connection(file):
    try:
        config = json.loads(Path(file).read_text())
        parsed = urlparse(config["url"])
        if parsed.scheme not in ("http", "https") or not parsed.hostname or parsed.username or parsed.query or parsed.fragment:
            raise Failure("Invalid credential service URL")
        config["repository"] = repository(config["repository"])
        config["authorization"] = Path(config["tokenFile"]).read_text().strip()
        return config
    except (OSError, ValueError, KeyError):
        raise Failure("GitHub App connection was removed or is unavailable; reconnect in Git access") from None


def request_token(config):
    return exchange(config["url"].rstrip("/") + "/v1/token", config["authorization"], "POST",
                    {"repository": config["repository"]})["token"]


def credential(file, operation):
    # Git's get/store/erase protocol; never persist an installation token.
    if operation == "store":
        return
    fields = dict(line.rstrip("\n").split("=", 1) for line in sys.stdin if "=" in line)
    config = connection(file)
    if fields.get("protocol") != "https" or fields.get("host", "").lower() != "github.com" or repository(fields.get("path", "")) != config["repository"]:
        raise Failure("Git credential request does not match this repository")
    if operation == "erase":
        exchange(config["url"].rstrip("/") + "/v1/invalidate", config["authorization"], "POST", {"repository": config["repository"]})
    elif operation == "get":
        print("username=x-access-token\npassword=" + request_token(config) + "\n")
    else:
        raise Failure("Unsupported Git credential operation")


def gh(args):
    binary = os.environ.get("GUACA_GH_BINARY", "/usr/bin/gh")
    lookup = subprocess.run(["git", "config", "--get", "guaca.githubConnection"], capture_output=True, text=True)
    if lookup.returncode:
        os.execv(binary, [binary, *args])
    config = connection(lookup.stdout.strip())
    # A copied or repointed checkout must not spend the previous repo's token.
    origin = subprocess.run(["git", "remote", "get-url", "origin"], capture_output=True, text=True, check=True)
    if remote_repository(origin.stdout.strip()) != config["repository"]:
        raise Failure("Origin changed; reconnect GitHub App access for this repository")
    if args[:2] == ["auth", "token"]:
        raise Failure("GitHub App tokens are provided to commands, not printed")
    for index, arg in enumerate(args):
        if arg in ("-R", "--repo") and index + 1 < len(args):
            if repository(args[index + 1]) != config["repository"]:
                raise Failure("The GitHub command targets another repository")
        if arg.startswith("--repo=") and repository(arg.split("=", 1)[1]) != config["repository"]:
            raise Failure("The GitHub command targets another repository")
        if arg.startswith("--hostname"):
            raise Failure("GitHub App commands use github.com")
    environment = dict(os.environ)
    for name in ("GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"):
        environment.pop(name, None)
    try:
        user_token = exchange(config["url"].rstrip("/") + "/v1/user/token", config["authorization"], "POST", {"repository": config["repository"]})["token"]
    except Failure:
        raise Failure("GitHub user access is unavailable. Sign in again under this repository's Git access; pull requests cannot use bot credentials") from None
    environment.update(GH_TOKEN=user_token, GH_HOST="github.com", GH_REPO=config["repository"], GH_PROMPT_DISABLED="1")
    os.execve(binary, [binary, *args], environment)


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="mode", required=True)
    serve = sub.add_parser("serve")
    serve.add_argument("config")
    helper = sub.add_parser("credential")
    helper.add_argument("connection")
    helper.add_argument("operation", choices=("get", "store", "erase"))
    sub.add_parser("gh", help="Run the GitHub CLI with repository credentials")
    try:
        # gh owns its flags, including leading options such as --version and
        # --repo. argparse's REMAINDER still rejects some leading options.
        if sys.argv[1:2] == ["gh"]:
            gh(sys.argv[2:])
            return
        parsed = parser.parse_args()
        if parsed.mode == "serve":
            config = json.loads(Path(parsed.config).read_text())
            broker = Broker(config)
            address, port = config.get("listen", "127.0.0.1:8791").rsplit(":", 1)
            server = ThreadingHTTPServer((address, int(port)), handler(broker))
            print("GitHub credential service listening", flush=True)
            server.serve_forever()
        elif parsed.mode == "credential":
            credential(parsed.connection, parsed.operation)
    except (Failure, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        message = str(error) if isinstance(error, Failure) else "GitHub App configuration or command failed"
        print(message, file=sys.stderr)
        raise SystemExit(1) from None


if __name__ == "__main__":
    main()
