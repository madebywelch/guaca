# Guaca

**A desktop app for AI agents that work together.**

![Guaca with a group of agents working across repositories](docs/img/guaca.png)

Run Guaca locally on your computer or connect it to a VPS. The backend workspace
runs in a container; you choose where it lives. The desktop is your window into
that workspace. On an always-on VPS, your agents keep working when you close
your laptop. A local workspace runs while your computer is awake.

## Build a crew

Organize your agents into **groups**: isolated spaces with their own collection
of agents, settings, and **plugins**. Plugins are MCP servers that connect your
agents to other services. Choose from the included connections or add your own.
You control which agents can use each plugin and its tools.

Agents communicate with you and each other, delegate work, and keep their own
memory and working notes. They can also have a browser through **Kernel** or a
computer through **E2B**, configured with the respective vendor's API key.

## Give them a repository

Link a repository inside a group, then **drag an agent onto it** to give that
agent access. Use an existing Git directory available to the backend or clone
a remote repository into the workspace.

Choose **Claude Code, Codex, or pi** as the coding harness. Guaca coordinates the
work and brings progress and results back into the conversation. You can steer
a running job and configure approval before pushes and pull requests. Connect
Git credentials so agents can pull and push to your remote repositories.

## Get started

The desktop app currently supports **macOS**. To build and install from a
checkout, you need Node.js, pnpm, Rust, and the Xcode Command Line Tools:

```sh
./scripts/install.sh --no-pull --launch
```

For a local workspace, have Docker installed and running before installing.
The script builds the matching backend image. Choose **On this Mac** when Guaca
opens, or **Remote host** to connect to a backend on your VPS. You can change
that choice later in **Settings → Workspace**. A remote connection does not
require Docker on your Mac.

Managed Guaca compute spaces are planned for a later release. Local and
self-hosted workspaces are available now.

## Go deeper

[Hosting and setup](docs/HOSTING.md) ·
[Plugins](docs/PLUGINS.md) ·
[Coding](docs/CODING.md) ·
[Architecture](docs/ARCHITECTURE.md)

For development, run `pnpm install` and `pnpm app`. Run `./scripts/ci.sh` for the
project checks.

[GNU AGPL v3](LICENSE)
