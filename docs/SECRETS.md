# Secrets

A group has a Secrets tab. The operator saves a service label, its environment
variable, a value, and the agents allowed to use it. A new secret defaults to
nobody. Manage replaces the value and the recipient list in one transaction;
a blank replacement keeps the current value. Forget removes the secret and its
grants. None of these actions changes a repository's remote or Git sign-in.
For a deployment, add `CLOUDFLARE_API_TOKEN` and select the deploying agent.
Repository authentication remains in Repositories.

## One store, three execution paths

This extends the existing `connectors` store. It does not add another store
beside credentials formerly shown under Plugins. Migration 52 grants existing
credentials to the group's current, non-discarded agents, preserving their
access. Future hires need explicit grants.

`connector_agents` records recipients. The grant writer validates the whole
list before committing and rejects agents outside the secret's group. Both
`agent_connectors` (names for the prompt) and `connector_env` (values for
execution) join against current group membership and discard state. Moving an
agent cannot carry the former group's secrets with it.

The runtime resolves grants before each repository shell and coding job, and
when constructing the client for a computer command. It supplies environment
variables to Bash, Pi, Claude Code, Codex's app-server, or an E2B command. Values
are not command-line arguments, appended instructions, repository config, or
files written by Guaca. Coding instructions name the available variables.
Registered variable names are removed from inherited child environments before
the selected values are supplied, so an ungranted name cannot silently fall
back to that name in the daemon environment. Process-control variables such as
`PATH`, `BASH_ENV`, `NODE_OPTIONS`, and `GUACA_*` are refused as secret names.

Rotation and revocation apply to subsequent commands and jobs. An existing
process already holds its environment; stop it before revoking access when
immediate withdrawal is needed. Guaca cannot retract a credential already used
or saved by another program. Revoking the token at its issuer is what invalidates
such copies.

## What is kept out of Guaca

The input form is write-only. Metadata contains the variable, service, note,
recipient IDs, timestamps, and whether a value exists. The legacy `secretHint`
field is returned empty, including for short secrets. Export does not include
secret values or their grants. Inbound credential types do not derive Debug
with the value on it.

Shell output is scrubbed before it is clipped, with overlap retained across
pipe reads. Computer output, coding progress, final answers, failures and bridge
reports are scrubbed before they reach Guaca's transcript or events. Redaction
matches exact values, JSON escaping, percent encoding, and standard Base64.
It does not recognize every possible transformation and is not a substitute
for limiting what an authorized program can do.

## Storage and trust boundary

Values remain in the existing plaintext SQLite database, including its journal
and backups. This is not an encrypted vault and has no external key-management
service. Protect the backend volume and its backups accordingly. Other existing
credentials, including provider settings and saved Git credentials, retain
their existing storage and lifecycle.

A grant controls which environment Guaca constructs. Repository shells and
coding harnesses still execute under the backend's OS account. They are trusted
programs with that account's filesystem and network access, not mutually isolated
security principals. A coding harness may read its own environment or include
tool output in its own model context and session files before Guaca receives
anything to redact. Strong isolation from those programs requires separate
process identities or containers and a credential broker. The UI and docs must
not promise that environment injection makes that impossible.

## Verification

The store tests cover no default grant, cross-group rejection, atomic rotation,
revocation, moved and discarded agents, deletion, and the legacy migration.
The hosted IPC test covers write-only creation, update and deletion. Real child
process tests cover all three harnesses and the repository shell. Chunk tests
cover Unicode values, overlapping secrets and clipping boundaries. Component
tests cover explicit selection, rotation, failed saves, and the separate tab.
