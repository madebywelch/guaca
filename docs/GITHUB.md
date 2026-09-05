# GitHub App access

Guaca can authenticate Git and GitHub CLI operations through a GitHub App.
The App is independent of the coding harness: Codex, Claude Code and shell
commands use the same repository connection. Existing token and SSH connections
remain available. No Guaca account or guaca.bot service is required.

## Commit authorship

The operator is the commit author. Set **Commit author name** and **Commit author
email** when linking a remote or under its **Git access** panel. Use an email
associated with the user's GitHub account or their GitHub-provided noreply
address. The identity lives in the repository's Git config, shared with its
engineer worktrees, independently of the harness and credential helper. New
clones can inherit an explicitly configured backend identity, but cannot invent
one from the container user. Existing linked directories retain their config.
Older clones with `guaca <guaca@localhost>` need an explicit identity update;
Guaca cannot infer a human from an App installation owned by an organization.
Only future commits change. Git configuration supplies the normal author and
committer defaults; explicit Git environment variables or `--author` can override
those defaults as usual.

An installation token authenticates a push as the App without changing commit
authorship. Pull requests opened with that token are still authored by the App.
Opening PRs as the human requires a separate GitHub App user authorization flow,
which is not implemented here. It is distinct from installing the App and from
setting a commit email. These settings do not grant account access or verify
ownership of an email.

GitHub documents [commit attribution](https://docs.github.com/en/account-and-profile/how-tos/email-preferences/setting-your-commit-email-address)
and [acting on behalf of a user](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/authenticating-with-a-github-app-on-behalf-of-a-user).

## What runs where

A separate credential service holds the App's PEM private key. A workspace
receives only an authenticated connection to that service. The supplied Compose
overlay mounts the private key only into the credential service, which mounts
no repositories and executes no coding jobs. The runtime and its jobs cannot
read that mount. Never mount the Docker socket into either container.

The broker configuration binds one installation and an explicit repository
allowlist. A request cannot select another installation or a repository outside
that list. Before minting a token, the broker checks that GitHub associates the
repository with the configured installation. Every returned token must name
exactly that repository. Installation tokens remain in the broker's memory;
Git receives one through its credential-helper pipe and `gh` receives one in
its own process environment. They are never stored in Git remote URLs.

The Git helper obtains credentials on demand. The `gh` wrapper does the same
for each invocation, including from a linked agent worktree. Cached tokens are
replaced five minutes before expiry. A Git rejection invalidates the cached
token; failures never fall back to an older token or the backend's personal
GitHub account. Switching back to a personal token is an explicit UI action.
GitHub's normal permissions and branch protection still apply.

This is one trusted workspace, not isolation between agents or groups. A coding
job can read its workspace's broker authentication file and request credentials
for its configured repositories. Use a separate runtime and broker scope for
each unrelated customer. The official App's private key must stay in trusted
operator infrastructure; never ship it in a self-hosted image or configuration.

## Register an App

A self-hoster owns their own App. Register it under the GitHub account or
organization whose repositories it will access. Use Contents and Pull requests
read/write; Metadata read-only; Actions and Checks read-only for CI inspection.
Workflows write access is needed if engineers may modify `.github/workflows`.
Install it on selected repositories. Record its Client ID and Installation ID,
and generate a PEM private key. Guaca uses the Client ID as the JWT issuer;
the numeric App ID also works.

For this installation-token setup, callbacks, user OAuth and webhooks are not
required. Registration is currently manual. A hosted installation-onboarding
flow and an automated App-manifest registration flow are not implemented here.
Do not treat a browser-supplied installation ID as proof of ownership when
building that onboarding flow.

## Start the self-hosted containers

Copy `deploy/github/config.example.json` to a private configuration directory
outside your checkout. Set `clientId`, `installationId`, and `repositories`.
Leave its container paths, `listen`, `initializeToken`, and `tokenUid` as shown.
The repository entries are `owner/repository` names, without an HTTPS prefix.

Keep the PEM outside your checkout too. Set these two variables to absolute
paths on the machine running Docker:

```sh
export GUACA_GITHUB_CONFIG_FILE=/absolute/path/github-config.json
export GUACA_GITHUB_PRIVATE_KEY_FILE=/absolute/path/guaca.private-key.pem

docker compose -f docker-compose.yml -f docker-compose.github.yml up -d --build
```

The broker creates a random authentication token in the `github-client` volume,
owned by UID 1000 with mode 0600. Guaca mounts that volume read-only. The broker
has no published port. Its HTTP address is on the private Compose network;
use HTTPS if placing a broker on another host.

In a group's repository panel, choose **Clone a remote**, enter its HTTPS GitHub
URL, and check **Use the GitHub App configured on this backend**. Choose the
harness and assign an engineer as usual. For an existing HTTPS GitHub checkout,
open **Git access** and choose **Connect GitHub App**. The initial access check
runs before existing Git credentials are replaced.

**Disconnect GitHub App** removes that repository's local connection. Subsequent
Git and `gh` commands fail until it is reconnected or deliberately given another
credential. It does not revoke already issued tokens at GitHub. Remove the
repository from the GitHub App installation to revoke upstream access too.

Adding another repository requires both granting it to the GitHub App
installation and adding it to the broker's allowlist, then restarting the
broker. Update `GUACA_GITHUB_PRIVATE_KEY_FILE` and recreate the broker to rotate
the PEM. Retire the previous key in GitHub after verification. Existing GitHub
App settings and signing keys remain owned by the self-hoster.

## Without Compose

Run `python3 deploy/github/github_app.py serve /path/broker-config.json` under a
separate OS user or on a separate host. Python 3.10+ and OpenSSL are required.
A separate process under the same OS user does not protect its PEM from coding
jobs. Set `tokenFile` to a broker authentication file of at least 32 characters,
and arrange for the runtime to read that file without reading the PEM. The
runtime needs Python 3.10+ and `gh` on its PATH.

Configure the runtime with `GUACA_GITHUB_BROKER` and
`GUACA_GITHUB_BROKER_TOKEN_FILE`. These are operator configuration, not a Guaca
account dependency. Desktop users need the broker too; use the supplied
containers for the strongest simple separation.

## Verification

`./scripts/ci.sh` runs offline broker tests, JWT verification using a generated
fixture key, and real Git/gh process tests. These cover repository scoping,
installation mismatch, near-expiry renewal, invalidation, denial, worktree
inheritance, disconnect, and switching back to a personal token.

For an explicitly disposable GitHub repository, `scripts/github-app-live.py`
starts a temporary broker and daemon, links through the daemon API, pushes from
an engineer worktree, and creates a draft PR as the App. It leaves the PR and
branch for review, unmerged. An empty repository receives an initial README
commit so it has a PR base. The test checks that GitHub attributes the worktree
commit to the supplied user while the PR actor remains the App. No model is called.

```sh
export GUACAD=/absolute/path/to/guacad
export GUACA_TEST_GITHUB_CLIENT_ID=your-client-id
export GUACA_TEST_GITHUB_INSTALLATION_ID=123456
export GUACA_TEST_GITHUB_REPOSITORY=owner/disposable-repository
export GUACA_TEST_GITHUB_PRIVATE_KEY_FILE=/absolute/path/private-key.pem
export GUACA_TEST_GIT_AUTHOR_NAME=your-name
export GUACA_TEST_GIT_AUTHOR_EMAIL=your-github-noreply-email
export GUACA_TEST_GITHUB_USER=your-github-login
python3 scripts/github-app-live.py
```

The container boundary has its own read-only live check. Build the runtime with
`./scripts/image.sh`, build the broker with
`docker build -t guaca-github-broker:check deploy/github`, and set `GUACAD_IMAGE`
to the runtime image that script produced and `GUACA_GITHUB_BROKER_IMAGE` to
`guaca-github-broker:check`. With the same `GUACA_TEST_GITHUB_*` variables above,
run `python3 scripts/github-app-container.py`. It verifies private-repository
clone, Git and `gh` access, the absent PEM mount in the runtime, and the SSH
client. All temporary containers, volumes, and networks are removed.

GitHub's contracts: [installation tokens](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-an-installation-access-token-for-a-github-app),
[JWT issuer and expiration](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-json-web-token-jwt-for-a-github-app).
