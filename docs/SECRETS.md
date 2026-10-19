# Secrets

A group has a Secrets tab. The operator saves a service label, its environment
variable, a value, and the agents allowed to use it. A new secret defaults to
nobody. Manage replaces the value and the recipient list in one transaction;
a blank replacement keeps the current value. Forget removes the secret and its
grants. For a deployment, add `CLOUDFLARE_API_TOKEN` and select the deploying
agent.

## One store, one execution path

This extends the existing `connectors` store. It does not add another store
beside credentials formerly shown under Plugins. Migration 52 grants existing
credentials to the group's current, non-discarded agents, preserving their
access. Future hires need explicit grants.

`connector_agents` records recipients. The grant writer validates the whole
list before committing and rejects agents outside the secret's group. Both
`agent_connectors` (names for the prompt) and `connector_env` (values for
execution) join against current group membership and discard state. Moving an
agent cannot carry the former group's secrets with it.

The runtime resolves grants when constructing the client for a computer
command, in `Runtime::ensure_computer`, and supplies them as environment
variables to every E2B command that client runs. Values are not command-line
arguments, appended instructions, git config, or files written by Guaca. The
prompt names the available variables. Process-control variables such as
`PATH`, `BASH_ENV`, `NODE_OPTIONS`, and `GUACA_*` are refused as secret names.

Rotation and revocation apply to subsequent commands. An existing
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

Computer output and the provider's error messages are scrubbed before they
reach Guaca's transcript or events. Redaction
matches exact values, JSON escaping, percent encoding, and standard Base64.
It does not recognize every possible transformation and is not a substitute
for limiting what an authorized program can do.

## Storage and trust boundary

Values remain in the existing plaintext SQLite database, including its journal
and backups. This is not an encrypted vault and has no external key-management
service. Protect the backend volume and its backups accordingly. Other existing
credentials, including provider settings, retain
their existing storage and lifecycle.

A grant controls which environment Guaca constructs. A command on a computer
runs with that environment and can read, transform or transmit what it holds;
redaction applies only to what comes back to Guaca. The UI and docs must not
promise that environment injection makes that impossible.

## Verification

The store tests cover no default grant, cross-group rejection, atomic rotation,
revocation, moved and discarded agents, deletion, and the legacy migration.
The hosted IPC test covers write-only creation, update and deletion. The
computer client's tests cover scrubbed output and error trailers. Chunk tests
cover Unicode values, overlapping secrets and clipping boundaries. Component
tests cover explicit selection, rotation, failed saves, and the separate tab.
