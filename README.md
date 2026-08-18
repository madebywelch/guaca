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

Right-click any agent in the rail to pin it to the top, to duplicate it, or to
open its profile. Pinning and duplicating are one click because they are the
ones you do; a name and a set of instructions are written once and read often,
so editing a profile is deliberately one click further away.

## What an agent can do

Ten tools, described to the model and visible in the transcript when used:

| Tool | What it does |
|---|---|
| `directory` | Lists the other agents in its group, with their skills |
| `send_message` | Queues a message to one or more of them, with any files, and returns |
| `update_notes` | Rewrites its own memory, which it is shown every turn |
| `schedule` | Sets or cancels its own routines |
| `create_agent` | Proposes a new colleague, and waits for you to allow it |
| `request_permission` | Stops and asks you before acting in your name |
| `run_command` | A shell on its own machine |
| `open_on_desktop` | Launches something on that machine's screen |
| `use_screen` | Looks at the screen, clicks, types |
| `browse` | Drives Chrome through the DevTools protocol |

The last four need a computer, which is optional; see below.

## Files

Drag a file onto the window and it goes with your next message. Agents send
files to each other the same way, so a draft one of them wrote arrives on
another's machine ready to be worked on.

What an agent gets depends on what the file is. A picture it looks at. Text it
reads. A Word document, a spreadsheet or anything else lands in `~/inbox` on its
own computer, where it opens it with whatever it needs: this app does not learn
file formats, because the machine already knows more of them than it ever would.

One copy is kept per file rather than per recipient, so the same document sent
to four agents is one file on disk. 25 MB each.

## Asking you first

Two things an agent cannot do on its own.

**Adding another agent**, because it changes who you have and each one costs
money to run. **Acting outside the workspace in your name**: sending mail as
you, submitting a form, buying something.

Both stop the agent mid-turn and put a card in the conversation with two
buttons. Nothing happens until you answer, and the answer is kept beside what
was asked. An agent told by a colleague that you have already authorised
something asks you rather than taking a peer's word for it, and the question
comes from the agent that will actually do the thing rather than the one
relaying the request.

**Always allow** exists for adding agents, scoped to one agent asking about one
thing, and is listed on that agent where you can take it back. It is
deliberately absent for acting in your name: a standing yes there would cover
every future send rather than the one in front of you.

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

Each agent has a memory file it maintains itself with `update_notes`, shown to
it at the start of every turn. It is the only thing that survives between
conversations. You can read and edit it in the agent's profile.

Memory and notes are one file under two names: memory is what an agent is told
it has, notes is what the tool and the files are called. Ask an agent to update
its memory, to remember something, or to make a note of it, and all three land
in the same place.

An agent can also keep its own schedule: "check the listings every weekday" is a
routine, stored as a next-due time rather than a timer, so it survives a
restart. A routine has a name, an instruction, and a trigger: every hour, every
day, weekdays, every week, every month, or once. Calendar repeats keep the time
of day they were set for, across a clock change and, for weekdays, across the
weekend.

Routines are listed in the panel beside the conversation, one line each. Open
one and the panel becomes that routine: switch it off without deleting it, fire
a test run that does not move the schedule, rewrite what it says, retime it, and
read what it has actually done.

## Computers

An agent can be given a Linux machine with a desktop, a browser and a shell —
on this Mac, or hosted with an [E2B](https://e2b.dev) key. The screen sits at
the top of the panel beside the conversation: a live picture while you read,
and interactive when you click it open.

- The machine sleeps after fifteen idle minutes and keeps its disk, so it wakes
  up still signed in to whatever it was signed in to.
- Machines are private, and the viewer is a loopback proxy that holds whatever
  reaching one takes — a token, or an address on this Mac — so no URL that
  reaches a machine is ever guessable or exposed to the webview.
- A machine that no live agent refers to is released on the next launch. A
  forgotten one bills, or holds twenty gigabytes, exactly like a used one.

**Local computers (Apple Container).** On an Apple silicon Mac running macOS 26
with [Apple Container](https://github.com/apple/container/releases) 1.2.x
installed, the machines run here instead: one lightweight Linux VM per agent,
no account and no bill. Settings has a **Computer provider** selector, and
*Automatic* prefers Apple Container whenever it is installed. Local machines
cannot see your files, but they may reach services this Mac or your local
network exposes, so E2B is still the option with an off-device boundary. The
desktop image is pulled the first time an agent needs a machine; until it is
published, build it with `computer-image/build.sh` and point
`GUAC_COMPUTER_IMAGE` at what that made.

With no provider ready — no Apple Container, no E2B key — none of this appears,
the other four tools are not offered, and the agent is told it has no machine
rather than discovering it halfway through a command.

## Connectors

An agent with a computer can already reach almost anything. What it cannot do is
know what it is already allowed into: sign its browser in to LinkedIn and it
will still tell you it has no way to post. The access was never missing. The
knowledge was.

Two halves, and which one you get is decided by the service rather than by a
preference.

**Sign in on the agent's computer.** Open its screen, log in to whatever you
like, and that is the whole procedure. Nothing is declared anywhere: Chrome is
holding the cookies, so Guaca asks the browser what it is signed in to and puts
the answer in that agent's prompt and on every other agent's roster. Log out and
it disappears the same way. There is nowhere to type a password because Guaca
never handles one.

**Credentials, on the group.** For an API with a plain token, paste it into the
group's settings and name a variable. Every machine in that group gets it in the
environment of every command it runs.

Two things follow from where each half physically lives, and both are load
bearing:

- A browser session is cookies on **one** machine, so it belongs to one agent.
  The rest of the crew is told who holds it, in the same roster that lists
  skills, so an agent asked to post to LinkedIn says "Researcher can do that"
  rather than "I am not signed in". A skill is a claim an agent wrote about
  itself; a session is a fact read off a disk.
- A credential is a string, so the whole group gets it, and **it never reaches
  the model.** It goes from SQLite into the environment of a sandbox command and
  nowhere else: not into a prompt, not into the transcript, not into the
  webview, not onto the sandbox's disk. The agent is told the variable's name
  and told not to print it.

Detection is deliberately cautious, because a wrong claim is worse than a
missing one: an agent that believes it can read Gmail wastes a turn finding out
it cannot, and you see a broken account rather than an absent one. Sites are
recognised by the cookie that actually means somebody logged in, so a browser
holding `google.com` cookies it collected while signed out is correctly reported
as signed out. Anything not on that list is only mentioned if the browser has
genuinely visited it *and* holds a cookie implying an identity, and it is passed
to the agent as a maybe. On a real profile holding a thousand cookies across
three hundred domains, that combination reported exactly the one account the
machine actually had.

The recognised-service list lives in `domain/signin.rs` and adding one is a
line. There is no OAuth: a local, open-source app cannot honestly ship "Log in
with Google", because Gmail scopes are restricted and verification is per-app.
Signing in on the agent's own browser gets you Gmail today, through the same
door a person uses.

Being signed in is also what makes a hostile web page worth writing, so every
page an agent reads arrives labelled as content rather than instruction, and the
system prompt says what a signed-in agent must stop short of. See **Credit**.

## Data

Everything lives in one directory:

```
~/Library/Application Support/com.madebywelch.guac/
  guac.db        agents, groups, messages, routines, usage, permissions
  config.json    settings, written 0600
  workspace/     one markdown file per agent: its memory
  files/         everything you or an agent attached, by content hash
```

The API key is stored in `config.json` in plaintext. Guaca is a local app with
no auth, and a key encrypted with a key sitting beside it would be theatre. The
honest answer is the OS keychain, and that is a deliberate follow-up rather than
something faked here.

Deleting an agent is a soft delete: it leaves the rail and can never be messaged
again, its computer is destroyed and its memory goes, but what it already said
stays readable and its name becomes free to reuse. **Start fresh** on a group
resets its whole crew (transcripts, routines, memories and spend) while keeping
the agents themselves.

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

Connectors have two kinds rather than one because of *Beyond Browsing: API-Based
Web Agents* by Yueqi Song, Frank Xu, Shuyan Zhou and Graham Neubig
([arXiv 2410.16464](https://arxiv.org/abs/2410.16464)). Putting API-calling and
browsing agents on the same WebArena tasks, they found APIs beat browsing, and a
hybrid that could choose beat both, by 24.0 points absolute over browsing alone.
The design that follows is not "an API when there is one, a browser otherwise":
it is telling one agent about both and letting it pick, which is what the
prompt's **What you can reach** section is for.

The security half comes from *BrowseSafe: Understanding and Preventing Prompt
Injection Within AI Browser Agents* by Kaiyuan Zhang, Mark Tenenholtz, Kyle
Polley, Jerry Ma, Denis Yarats and Ninghui Li
([arXiv 2511.20597](https://arxiv.org/abs/2511.20597)). Its useful move is to
benchmark injections that drive real-world *actions* rather than text output,
which is exactly what a signed-in session turns a web page into: the payload no
longer has to talk an agent into obtaining access, because it already has the
operator's. Guaca takes the architectural half of their defence-in-depth
argument, which is what a local app can actually hold: page content is labelled
at the point it enters the turn, credentials never enter the model's context at
all, and the signed-in agent is told where to stop. Neither paper's authors
endorse any of this.

## Licence

[GNU AGPL v3](LICENSE). You can use, modify and run it, including commercially;
if you distribute it or run a modified version as a network service, that
version has to be published under the same licence.
