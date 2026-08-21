# Guaca

A local desktop app where you talk to LLM agents and those agents talk to each
other. Slack-shaped: a rail of agents on the left, a conversation on the right,
and an activity view showing every message they send between themselves.

![Guaca, showing a crew of eleven agents, with the Manager's own computer open beside the transcript as it works through a Craigslist search](docs/img/guaca.png)

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

Open Settings and choose how turns get paid for. Two ways:

- **A ChatGPT subscription.** Press **Sign in** on the Provider pane, enter the
  code it shows you in the browser it opens, and your Plus, Pro, Team or
  Enterprise plan covers the work with no per-token bill. It needs a paid plan:
  a free account signs in and then cannot make a single call, which Guaca tells
  you at the sign-in rather than leaving you to find out.
- **An endpoint and a key.** OpenRouter by default, and anything
  OpenAI-compatible after that. A local llama.cpp or LM Studio server works
  without a code change: point **Inference endpoint** at it and leave the key
  blank if it does not need one.

Then press **Test connection**, which checks the endpoint, the credential and the
model separately. You find out which one is wrong rather than watching an agent
sit silent.

Whichever you choose applies to every agent, and a group can still point itself
somewhere else. A group that names its own endpoint or key uses those; a group
that only names a model runs that model on whatever the app is paying with.

**A Claude subscription cannot be used this way, and that is Anthropic's rule
rather than a missing feature.** Consumer Claude OAuth tokens are restricted to
Claude Code and Claude.ai, enforced at their servers, and using one elsewhere
breaches the Consumer Terms and risks the account. Claude models still work here:
they are what OpenRouter serves by default. `docs/PROTOCOL.md` has the detail.

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
| `use_screen` | Looks at the screen, clicks, types, drags |
| `browse` | Uses its own browser, which knows where everything on a page is |

The first three of those need a computer and the last needs a browser. Both are
optional, both are separate, and an agent is offered only the tools for what it
actually has. See below.

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

## The menu bar

Guaca keeps working when the window is not in front of you, so there is an
avocado in the menu bar saying what it is doing. An outline means nothing is
running, a filled one means something is, and it turns red with a count beside
it when an agent is parked waiting on you. Hovering says the same thing in one
line, with what the session has cost.

Opening it lists what is waiting on you, who is working, and the spend twice:
this session and all time. A permission request can be answered from there,
with the fields of the request under it, so noticing a parked turn and dealing
with it is one gesture instead of a trip back into the app. The same rule
applies as in the conversation: **always allow** is offered for adding agents
and never for acting in your name.

Two things to do from there. **Open Guaca**, and **stop everything running**,
which appears only when there is something to stop.

Closing the window leaves Guaca in the menu bar rather than quitting it, because
agents keep their own appointments and a routine set for every morning should
not stop firing the first time you tidy your screen. Command-Q and **Quit
Guaca** still quit.

macOS only, for now.

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

## Computers and browsers

An agent can be given two things, and they are not the same thing.

**A computer**, with an [E2B](https://e2b.dev) key: a Linux machine with a
desktop, a shell and a screen. The agent works it the way a person does, by
looking at a picture of the screen and aiming a pointer at it. That is
approximate, which is why the web does not belong here, and it is the only way
to use anything that is not a web page: an application, a file, an installer, a
terminal window.

- Every screen action answers with a fresh picture, so the agent is always
  looking at the result of what it just did rather than at a memory of the
  screen two actions ago.
- The machine sleeps after fifteen idle minutes and keeps its disk, so it wakes
  up still signed in to whatever it was signed in to.
- Sandboxes are private, and the viewer is a loopback proxy that holds the
  access tokens, so no URL that reaches a machine is ever guessable or exposed
  to the webview.
- A machine that no live agent refers to is released on the next launch. A
  forgotten sandbox bills exactly like a used one.

**A browser**, with a [Kernel](https://kernel.sh) key: a Chrome in the cloud and
nothing else. Chrome already knows where every link, button and field is, so the
agent asks the page rather than guessing at pixels. This is what the web is for.

- Sign-ins survive. Each agent gets a profile of its own, and a browser is
  created from it, so an account you signed it in to last week is open in a
  browser that did not exist a second ago.
- It goes quiet within seconds of the last action and stops billing, and comes
  back the moment the agent uses it again.
- The live view is where you take over, and taking over is the point: signing an
  agent in is the one thing only you can do.

Both panes sit at the top of the panel beside the conversation: a live picture
while you read, interactive when you click one open. Neither appears without its
key, and the tools that need it are not offered to the model either, so an agent
never spends a turn discovering that something was never set up.

## Connectors

An agent with a computer can already reach almost anything. What it cannot do is
know what it is already allowed into: sign its browser in to LinkedIn and it
will still tell you it has no way to post. The access was never missing. The
knowledge was.

Two halves, and which one you get is decided by the service rather than by a
preference.

**Sign in for the agent.** Open its browser, or its computer's screen, log in to
whatever you like, and that is the whole procedure. Nothing is declared
anywhere: Chrome is holding the cookies, so Guaca asks it what it is signed in
to and puts the answer in that agent's prompt and on every other agent's roster.
Log out and it disappears the same way. There is nowhere to type a password
because Guaca never handles one.

Which of the two you signed in on is recorded and shown, because the two have
unrelated cookie jars. An agent told only "you can reach Gmail" when the session
is on its computer's screen will call `browse`, be shown a login page, and
report the account as broken.

**Credentials, on the group.** For an API with a plain token, paste it into the
group's settings and name a variable. Every machine in that group gets it in the
environment of every command it runs.

Two things follow from where each half physically lives, and both are load
bearing:

- A session is cookies in **one** jar, so it belongs to one agent.
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
  subscription.json  the ChatGPT sign-in, written 0600, absent until you sign in
  workspace/     one markdown file per agent: its memory
  files/         everything you or an agent attached, by content hash
```

The API key is stored in `config.json` in plaintext, and the ChatGPT sign-in in
`subscription.json` beside it. Guaca is a local app with no auth, and a key
encrypted with a key sitting beside it would be theatre. The honest answer is the
OS keychain, and that is a deliberate follow-up rather than something faked here.
It matters more for the sign-in than for the key: that credential belongs to a
ChatGPT account with more than Guaca behind it. **Sign out** removes the file.

Deleting an agent is a soft delete: it leaves the rail and can never be messaged
again, its computer and browser are destroyed along with the browser profile
holding its sign-ins, and its memory goes, but what it already said stays
readable and its name becomes free to reuse. **Start fresh** on a group
resets its whole crew (transcripts, routines, memories and spend) while keeping
the agents themselves.

## Working on it

```sh
./scripts/ci.sh          # lint, typecheck, build, every suite
GUAC_LOG=guac=debug pnpm app
```

`AGENTS.md` is the map for anyone, human or otherwise, about to change
something: where everything lives, and which file to read first for the part
being changed. `docs/ARCHITECTURE.md` is the long version, including why agent
conversations end rather than going round forever, which is the hardest thing
here. `docs/ROUTINES.md`, `docs/MACHINES.md`, `docs/BROWSERS.md` and
`docs/WORKSPACE.md` cover the subsystems it does not, and `docs/PROTOCOL.md`
records what the interoperability literature contributed.

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
