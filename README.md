# Guaca

A local desktop app where you talk to LLM agents and those agents talk to each
other. Slack-shaped: a rail of agents on the left, a conversation on the right,
and an activity view showing every message they send between themselves.

![Guaca, showing four agents working on one errand with one agent's computer open over the transcript](docs/img/guaca.png)

Ask the Manager for something. It works out who on its team can help, sends
them each a message, and they think at once rather than in turn. You watch it
happen: who is typing, who wrote to whom, what the group has spent so far. When
they are done you get one answer, not a transcript to wade through.

Everything runs on your machine. The only thing that leaves is what you and your
agents type, sent to whichever OpenAI-compatible endpoint you point it at.

## The crew

![Eight of the characters: an avocado, a tomato, a lime, a chilli, a radish, an ear of corn, a mushroom and a carrot, each looking a different way](docs/img/crew.svg)

Sixteen of them, one per agent. They are not decoration: an agent's character is
how you find it in a rail of eight, and it is doing something at any moment. It
blinks and glances around while idle, looks toward whoever it is writing to,
winds up and throws when a message goes out, and gets visibly hit when one
arrives. Watching the rail tells you what your crew is doing before you have
read a single word of it.

## Run it

```sh
pnpm install
pnpm app          # dev, with hot reload
pnpm app:build    # an installable bundle
```

Open Settings, put in your name and an OpenRouter key, and press **Test
connection**, which checks the endpoint, the key and the model separately. You
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
message each other, and a name in another group does not resolve. It reads
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
resets its whole crew (transcripts, routines, notes and spend) while keeping the
agents themselves.

## Working on it

```sh
./scripts/ci.sh          # lint, typecheck, build, every suite
GUAC_LOG=guac=debug pnpm app
```

`AGENTS.md` is the short version for anyone, human or otherwise, about to change
something: what is surprising, what will bite, and how to check.
`docs/ARCHITECTURE.md` is the long version, including why agent conversations
end rather than going round forever, which is the hardest thing here.
`docs/PROTOCOL.md` records what the interoperability literature contributed.

## Status

A working app that its author uses, not a product. It is macOS-first: the paths
above are macOS paths, and nothing else has been tried. Expect rough edges,
particularly around sandboxes, which are the newest part.

## Credit

The shape of this app, agents you talk to that also talk to each other in a room
you can watch, is heavily inspired by Grok bot.

Its message layer is derived from the four agent interoperability protocols
(MCP, ACP, A2A, ANP) and from the survey comparing them, *A survey of agent
interoperability protocols* by Abul Ehtesham, Aditi Singh, Gaurav Kumar Gupta
and Saket Kumar ([arXiv 2505.02279](https://arxiv.org/abs/2505.02279)). A2A in
particular gave the Agent Card, discovery as a first-class operation, and card
versioning. `docs/PROTOCOL.md` records what was taken from each, what was cut,
and what had to be invented, chiefly termination, which none of them specify.

## Licence

[GNU AGPL v3](LICENSE). You can use, modify and run it, including commercially;
if you distribute it or run a modified version as a network service, that
version has to be published under the same licence.
