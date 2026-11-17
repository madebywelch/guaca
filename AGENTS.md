# Guaca

Local desktop app. You talk to LLM agents; the agents talk to each other. Tauri
v2, React + TypeScript front, Rust back.

The agent runtime lives in Rust, not the webview: each agent is a `tokio` task
with its own inbox, and your API key never crosses into the webview. The
frontend renders state and forwards intent; agent logic written there is in the
wrong half of the repo.

## Before you change something

Find its row in [docs/ROUTING.md](docs/ROUTING.md) and read the document and
the gotchas file it names. The rules below, with their reasons and the rest of
what holds everywhere, are in [docs/INVARIANTS.md](docs/INVARIANTS.md).

## Rules

- Anything crossing IPC is camelCase. A tagged enum needs `rename_all_fields` too.
- Migrations are forward-only and numbered. Never edit one; add another.
- A secret never reaches a model, a prompt, a transcript, an event or the webview.
- A provider branch in the runtime, the prompt or the guard is the wrong half.
- Every length is named: sizes, spaces, radii, durations, easings, shadows are tokens.
- New behavior needs a test that would fail without it. Failure paths first.
- American spelling, everywhere this repo writes its own words.
- No new comment, suppression or prose file without the owner's grant; `./scripts/ci.sh grants` names the fix.
- No machine signature: no `Co-authored-by` trailer, no "Generated with" footer.

## Verify

```sh
./scripts/ci.sh          # everything; `web` or `rust` for one half
```

Messaging, a prompt, streaming or the budget also need the suites in *Verify*
in [docs/TESTING.md](docs/TESTING.md).
