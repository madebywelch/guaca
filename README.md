# Guac

A local, no-auth desktop app where you talk to LLM agents and those agents talk
to each other. Slack-shaped: a rail of agents on the left, a conversation on the
right, and a `#activity` channel showing every message the agents send each
other.

Everything runs on your machine. The only thing that leaves is what you and your
agents type, sent to whichever OpenAI-compatible endpoint you configure.

## Run it

```sh
pnpm install
pnpm app          # dev, with hot reload
pnpm app:build    # produces a installable bundle
```

Then open Settings and paste an OpenRouter key. Press **Test connection** to
check the endpoint, key, and model separately before you wonder why an agent is
silent.

The endpoint is configurable, so a local llama.cpp or LM Studio server works
without a code change. Point **Inference endpoint** at it and leave the key
blank if it does not need one.

## Try the thing it is for

1. On first run, choose **Add a starter crew**. You get Manager, Researcher,
   Critic, and Scribe.
2. Open **Manager** and send: `Introduce yourself to all the other agents.`
3. Watch the rail. Each inter-agent message draws a pulse travelling between the
   two agents in the sender's colour.
4. Open any other agent to see the introduction it received, and `#activity` to
   see the whole cascade in one stream.

Messaging is asynchronous and non-blocking throughout. Manager does not wait for
Chef; all four peers think at once.

## How it is put together

```
src/                 React + TypeScript. A view over the runtime, nothing more.
src-tauri/src/
  domain/            AgentCard, Envelope, ids. No I/O.
  runtime/
    guard.rs         The loop guard. Read this one first.
    mod.rs           Agent actors and the message bus.
    prompt.rs        Prompt assembly, including the trust boundary.
    events.rs        Events pushed to the UI.
  llm/               OpenAI-compatible client, SSE decoding, tool definitions.
  db/                SQLite. Two tables, plain SQL, coded migrations.
  commands.rs        The entire IPC surface.
  app.rs             The only file that knows Tauri exists.
```

The agent runtime lives in Rust, not the webview. Each agent is a `tokio` task
with its own inbox, so sending is enqueue-and-return and N agents genuinely run
concurrently. It also means your API key never crosses into the webview.

`docs/ARCHITECTURE.md` covers the design decisions. `docs/PROTOCOL.md` records
what the agent-interoperability literature contributed and what had to be
invented.

## Agents talking to each other

Agents get exactly two tools:

- `directory()` lists the other agents with their skills.
- `send_message(to, text)` queues a message and returns immediately.

Bidirectional agent messaging does not terminate on its own, so Guac bounds it.
Five limits, all adjustable in Settings:

| Limit | Default | Stops |
|---|---|---|
| Model calls per conversation | 40 | Runaway spend, whatever the shape |
| Relay depth | 4 | Long delegation chains |
| Messages between any two agents | 3 | Two agents ping-ponging |
| Recipients per send | 8 | One message blasting the whole roster |
| Identical message to the same peer | 1 | An agent restating itself |

When a limit is hit the agent is told why, in words it can act on, and the
reason appears in the transcript. Nothing is dropped silently.

## Data

- Database: `~/Library/Application Support/com.madebywelch.guac/guac.db`
- Config: the same directory, `config.json`, written `0600`.

The API key is stored in that file in plaintext. Guac is a local no-auth app,
and a key encrypted with a key sitting beside it would be theatre. If you want
real secret storage the honest answer is the OS keychain, and that is a
deliberate follow-up rather than something faked here.

Deleting an agent is a soft delete: it leaves the rail and can never be messaged
again, but what it already said stays readable in the other agents' channels and
its name becomes free to reuse.

## Development

```sh
pnpm check          # lint and format
pnpm typecheck
pnpm test           # frontend
cargo test --manifest-path src-tauri/Cargo.toml   # runtime, guard, storage, client
```

The Rust suite includes end-to-end cascade tests that drive the real runtime
against a scripted OpenAI-compatible server, so tool-call assembly, the guard,
channel routing, and batching are all covered without a network or a window.

`GUAC_LOG=guac=debug pnpm app` turns up the logging.
