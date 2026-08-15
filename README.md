# Guaca

A local desktop app where you talk to LLM agents and those agents talk to each
other. Slack-shaped: a rail of agents on the left, a conversation on the right,
and an activity view showing every message they send each other.

![Guaca: a manager reporting to the operator while three agents work, with one
agent's computer open over the transcript](docs/img/guaca.png)

*Four agents on one errand. The rail shows who is typing and what the group has
spent; the panel is one agent's own machine, mid-browse.*

Everything runs on your machine. The only thing that leaves is what you and your
agents type, sent to whichever OpenAI-compatible endpoint you configure.

Tauri v2, React + TypeScript front, Rust back.

## Run it

```sh
pnpm install
pnpm app          # dev, with hot reload
pnpm app:build    # an installable bundle
```

Open Settings, put in your name and an OpenRouter key, and press **Test
connection** — it checks the endpoint, the key and the model separately, so you
find out which one is wrong rather than watching an agent sit silent.

The endpoint is configurable, so a local llama.cpp or LM Studio server works
without a code change. Point **Inference endpoint** at it and leave the key
blank if it does not need one.

## Try the thing it is for

1. On first run, choose **Add a starter crew**: Manager, Researcher, Critic and
   Scribe.
2. Open **Manager** and send `Introduce yourself to your team.`
3. Watch the rail. Each message between agents draws a pulse travelling between
   them in the sender's colour.
4. Open any other agent to see what it received, or the activity view to see the
   whole thing as a sequence diagram, one board per run.

Messaging is asynchronous throughout. Manager does not wait for Researcher; all
four peers think at once.

## What an agent can do

Eight tools, described to the model and visible in the transcript when used:

| Tool | What it does |
|---|---|
| `directory` | Lists the other agents in its group, with their skills |
| `send_message` | Queues a message to one or more of them, and returns |
| `update_notes` | Rewrites its own memory, which it is shown every turn |
| `schedule` | Sets or cancels its own routines |
| `run_command` | A shell on its own machine |
| `open_on_desktop` | Launches something on that machine's screen |
| `use_screen` | Looks at the screen, clicks, types |
| `browse` | Drives Chrome through the DevTools protocol |

The last four need a computer, which is optional; see below.

## Groups

A group is an isolation boundary. Agents in different groups cannot see or
message each other, and a name in another group does not resolve — it reads
exactly like a name belonging to nobody, so the roster cannot be probed across
the line.

Each group can pin its own model, endpoint and key. Settings resolve agent over
group over app, so an expensive crew and a cheap one can run side by side.

Every group header carries a running token count, a cost, and a sparkline of the
last ninety seconds, so a crew working on its own errands looks different from a
crew that is stuck.

## Memory and routines

Each agent has a notes file it maintains itself with `update_notes`, shown to it
at the start of every turn. It is the only thing that survives between
conversations. You can read and edit it in the agent editor.

An agent can also keep its own schedule: "check the listings every five hours"
is a routine, stored as a next-due time rather than a timer, so it survives a
restart. Routines are listed in the agent editor and can be written, retimed and
deleted by hand.

## Computers

With an [E2B](https://e2b.dev) key configured, an agent can be given a Linux
machine with a desktop, a browser and a shell. The screen appears in the corner
of its channel: a live picture while you read, and interactive when expanded.

- The machine sleeps after fifteen idle minutes and keeps its disk, so it wakes
  up still signed in to whatever it was signed in to.
- Sandboxes are private, and the viewer is a loopback proxy that holds the
  access tokens, so no URL that reaches a machine is ever guessable or exposed
  to the webview.
- A machine that no live agent refers to is released on the next launch. A
  forgotten sandbox bills exactly like a used one.

Without an E2B key none of this appears, and the other four tools are not
offered.

## Why agents stop talking

Bidirectional messaging does not terminate on its own. Two things stop it.

The first is `expects_reply`, which is what actually makes cascades converge:
answering somebody does not demand an answer back, and once both sides have had
their say the exchange is finished for that run. Getting this wrong is the
single largest source of "why are my agents still talking", and it is the reason
the eval suite below exists.

The second is the guard, which is the backstop rather than the mechanism. Five
limits, all adjustable in Settings:

| Limit | Default | Stops |
|---|---|---|
| Model calls per run | 60 | Runaway spend, whatever the shape |
| Relay depth | 8 | Long delegation chains |
| Messages between any two agents | 6 | Two agents ping-ponging |
| Recipients per send | 8 | One message blasting the whole roster |
| Identical message to the same peer | 1 | An agent restating itself |

Budget counts model calls, not agent turns: one turn can make several while
working through tool results, and counting turns lets a bounded run bill many
times over.

When a limit is hit, the agent is told why in words it can act on, and the
reason appears on the transcript chip. Nothing is dropped silently.

## Evals

The test suite answers "does the runtime do what it was told". The evals answer
a different question: given an instruction someone would actually type, is the
resulting traffic reasonable? Every cascade defect this app has had passed the
first and failed the second, because each individual message was fine and the
shape was not.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test evals   # scripted, in CI
./scripts/evals.sh                                             # live, costs money
```

The scripted ones run a stub model playing a specific bad habit on purpose and
check the runtime contains it. The live ones run the real prompts against your
configured model and print the whole conversation, which is the only way to see
that a prompt change made agents chattier.

`src-tauri/src/eval.rs` is the analyser: it reads a run's envelopes and names
what went wrong, and every fault it reports is decidable from the messages
rather than judged.

## How it is put together

```
src/                 React + TypeScript. A view over the runtime, nothing more.
src-tauri/src/
  domain/            AgentCard, Envelope, Routine, ids. No I/O.
  runtime/
    guard.rs         The loop guard. Read this one first.
    mod.rs           Agent actors and the message bus.
    prompt.rs        Prompt assembly, including the trust boundary.
    events.rs        Events pushed to the UI.
  llm/               OpenAI-compatible client, SSE decoding, tool definitions.
  db/                SQLite. Plain SQL, numbered migrations.
  e2b.rs             Sandboxes: the machines agents work on.
  proxy.rs           Loopback viewer for those machines.
  eval.rs            Reads a run and says whether it communicated sensibly.
  commands.rs        The entire IPC surface.
  app.rs             The only file that knows Tauri exists.
```

The agent runtime lives in Rust, not the webview. Each agent is a `tokio` task
with its own inbox, so sending is enqueue-and-return and N agents genuinely run
concurrently. It also means your API key never crosses into the webview.

`docs/ARCHITECTURE.md` covers the design decisions. `docs/PROTOCOL.md` records
what the agent-interoperability literature contributed and what had to be
invented.

## Data

Everything lives in one directory:

```
~/Library/Application Support/com.madebywelch.guac/
  guac.db        agents, groups, messages, routines, usage
  config.json    settings, written 0600
  workspace/     one markdown file per agent: its notes
```

The API key is stored in `config.json` in plaintext. Guaca is a local app with
no auth, and a key encrypted with a key sitting beside it would be theatre. The
honest answer is the OS keychain, and that is a deliberate follow-up rather than
something faked here.

Deleting an agent is a soft delete: it leaves the rail and can never be messaged
again, its computer is destroyed and its notes go, but what it already said
stays readable and its name becomes free to reuse. **Start fresh** on a group
resets its whole crew — transcripts, routines, notes and spend — while keeping
the agents themselves.

## Development

```sh
./scripts/ci.sh          # lint, typecheck, build, both suites
./scripts/ci.sh rust     # just the Rust half
GUAC_LOG=guac=debug pnpm app
```

The Rust suite includes end-to-end tests that drive the real runtime against a
scripted OpenAI-compatible server, so tool-call assembly, the guard, channel
routing and batching are covered without a network or a window. If you change
messaging, those are the ones that will catch you.

`AGENTS.md` is the short version for anyone, human or otherwise, about to change
something.

## Status

A working app that its author uses, not a product. It is macOS-first: the paths
above are macOS paths, and nothing else has been tried. Expect rough edges,
particularly around sandboxes, which are the newest part.

## Credit

The shape of this app — agents you talk to that also talk to each other, in a
room you can watch — is heavily inspired by Grok bot.

Its message layer is derived from the four agent interoperability protocols
(MCP, ACP, A2A, ANP) and from the survey comparing them, *A survey of agent
interoperability protocols* by Abul Ehtesham, Aditi Singh, Gaurav Kumar Gupta
and Saket Kumar ([arXiv 2505.02279](https://arxiv.org/abs/2505.02279)). A2A in
particular gave the Agent Card, discovery as a first-class operation, and card
versioning. `docs/PROTOCOL.md` records what was taken from each, what was cut,
and what had to be invented — chiefly termination, which none of them specify.

## Licence

[GNU AGPL v3](LICENSE). You can use, modify and run it, including commercially;
if you distribute it or run a modified version as a network service, that
version has to be published under the same licence.
