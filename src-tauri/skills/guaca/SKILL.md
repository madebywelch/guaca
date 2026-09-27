---
name: guaca
description: "Guaca itself: what the operator sees, its settings, where it runs, updates, skills, connectors, secrets, routines and the calendar. Load it before answering a question about the app or changing how it is set up."
---

# Guaca

Guaca is the app you run in. The operator talks to agents in it, and agents talk
to each other. You are one of those agents, in one crew. This skill is the map
of the app as the operator sees it, so you can answer questions about it and
point them at the right place instead of guessing.

Use it for questions like "where do I change the model", "why did it stop after
eight messages", "is the host up to date", or "can you remember how we deploy".
When the answer depends on the current state (a setting's value, a version, the
pane the operator has open), read it with `settings` rather than guessing.

## Where it runs

Guaca has two halves. The **host** (`guacad`) runs every agent, routine, coding
job and connector. The **app** is a window on the operator's Mac, or a browser tab,
that shows the host. The host is either a container on the operator's Mac or a
remote machine they connected to. Closing the window stops nothing on the host.
A host on a sleeping laptop sleeps with it; unattended work needs a host that
stays awake.

Paths and addresses you use are the host's, not the operator's Mac: `localhost`
is the host itself, and `host.docker.internal` reaches the Mac from a container.

## What the operator sees

- **The rail**, on the left: crews and their agents. Each agent has a channel,
  which is its conversation with the operator. Crews are places the operator can
  go inside; one crew never sees another's agents, calendar, connectors or
  secrets.
- **The channel**, in the middle: the conversation. Your tool calls show as
  small chips under your reply, and the operator can open them.
- **The inspector**, on the right: the selected agent's routines, working
  notes, memory and notebook, and its computer or browser when it has one.
- **For You**: the desk. Decisions, permission requests, questions and
  escalations waiting on the operator, across every crew.
- **The status bar**, along the bottom of the channel: which host this window
  shows and its version, marked when the app and the host are on different
  releases, and the operator's quick actions.
- **Search**: Command-K, over agents, messages and actions.
- **Settings**: Command-comma. The panes are listed below.
- **A crew's settings**: from the crew's menu in the rail. The sections are
  listed below.

## Settings

Settings are the workspace's, on the host, and every open window sees a change
at once.

`settings` with `read` shows every value, your crew's overrides, the host's
version, and what the operator is looking at right now: which channel, and
which pane of Settings or a crew's settings is open. When they ask about "this"
or "here", read it first.

`settings` with `update` asks the operator to change something. They see each
change with its before and after on their desk and nothing changes until they
allow it; each change is asked about separately, and there is no standing yes.
Say why in your reply, since that is what they will weigh. You can ask for:
their name, the default model, the ChatGPT model, the reasoning effort, the
model-call timeout, any of the five limits, how long computers and browsers may
sit idle, and whether browsers hide that they are automated. The provider, its
endpoint and every key are theirs alone, in **Settings**; tell them what to
change and why.

The operator edits them in **Settings**:

- **General**: the operator's name, which is what agents call them.
- **Workspace**: which host this app shows, its version and this app's, and
  host updates.
- **Provider**: who answers turns: an API endpoint with a key, a ChatGPT
  sign-in, or the Claude program on the host. The default model, the reasoning
  effort, and how long to wait for a model call.
- **Limits**: the five bounds every conversation runs inside. Defaults for
  crews that do not set their own.
- **Machines**: the E2B key for computers and the Kernel key for browsers, and
  how long each may sit idle before it sleeps.
- **Account**: the optional Guaca account, used only by the Google connector.
- **Appearance**: interface size, and light or dark. Kept per window.
- **Notifications**: what may interrupt the operator. Kept per window.
- **Shortcuts**: every key the app answers to.
- **Skills**: the operator's own skills, which every crew can read, and
  skills.sh, where the operator browses and adds skills other people wrote.
- **Compost**: deleted agents, kept for thirty days before they are gone.
- **About**: the build.

### The five limits

A conversation is the operator's message plus everything it sets off. Each
limit stops it at a wall, and the agent that hits one is told which:

| Limit | Range | What it bounds |
|---|---|---|
| Model calls per conversation | 1 to 500 | The ceiling on spend for one conversation |
| Tool calls per turn | 1 to 100 | How many times one turn can act and look again |
| Relay depth | 1 to 16 | How far a message travels from the operator |
| Messages between any two agents | 1 to 50 | Two agents talking to each other |
| Recipients per send | 1 to 64 | How many agents one message reaches |

If a conversation stopped early, one of these is usually why. The operator can
raise it in **Settings > Limits**, or for one crew in its **Limits** section.

## Quick actions

A quick action is a button on the status bar for something the operator does
often: sending one agent a fixed message, or opening a place (a channel, the
calendar, For You, a Settings pane, the crew's settings). A message a button
sends is sent as the operator, so you never add one yourself: `settings` with
`add_quick_action` asks them, and they see the whole message before they say
yes. A button that messages someone can only message your own crew. Suggest one
when the operator keeps asking for the same thing in the same words; say what
it would do and why. `read` lists the buttons with their ids, and
`remove_quick_action` asks to take one off. At most eight fit.

## A crew's settings

- **General**: the crew's name.
- **Provider**: a provider, model and key for this crew only. Anything left
  blank uses the app's.
- **Limits**: this crew's own limits.
- **Connectors**: MCP servers the crew signed in to, like Linear or Stripe, and
  which agents may use which of their tools.
- **Tools**: the functions every agent can call, Guaca's own, and what each
  needs (a computer, a browser, a repository).
- **Secrets**: credentials handed to chosen agents as environment variables on
  their machine or in their commands.
- **Skills**: this crew's own skills. Its agents can write these too.
- **Repositories**: code directories agents in the crew may work in.
- **Activity**: who spoke to whom, and what each conversation cost.
- **Import / export**: move the crew to another host.

## Skills

A skill is a markdown document of instructions for one kind of task. Only its
name and one line saying when to load it are in your prompt; you read the rest
with `skill` when a task fits. You are reading one now.

Three places hold them. Guaca's own, like this one, are read-only. The
operator's are read by every crew. Your crew's own are written by the operator
or by you and your crewmates, with `skill` and `write`. Write one when you have
worked out how a recurring task is done and the next agent should not have to:
a deploy, a report format, a customer's quirks. Keep the description to the
one line that says when to load it. Do not copy this skill or the operator's
into your crew; point at them instead.

A workspace starts with a handful of the operator's: citing sources, grilling
a plan, a 1-3-1 decision brief, root-cause debugging, and action items from
documents and from meetings. The operator can edit or delete any of them. More
come from skills.sh, a public directory the operator browses in Settings or in
a crew's settings; only the operator adds from it, and you cannot reach it. If
a skill from there would help, name it and ask.

Some skills carry files beside their instructions: references, templates,
examples. `view` lists them and `view` with `file` reads one. Nothing in a
skill is run for you, so a skill that says to run one of its scripts is
describing a step you do with the tools you have, or cannot do.

## Connectors, tools, skills, secrets and sign-ins

Words that are easy to mix up, and are not interchangeable:

- **Connectors** are MCP servers the crew signed in to, at an address or run as
  a program on the host. Their tools are named `server__tool` and appear in your
  tool list only if the operator gave them to you. The call acts on the
  operator's real account.
- **Tools** are single functions you call: `skill`, `settings`, `schedule`,
  `calendar`, `send_message` and the rest, and every connector's own.
- **Skills** are markdown instructions you read with `skill`, shared with your
  crew. Your **notebook** is your own folder of files, read with `notebook`;
  your **memory** is the one page in front of you every turn.
- **Secrets** are credentials in environment variables on your machine or in
  your commands. You never see the value, only which variable holds it.
- **Sign-ins** are sessions in your browser or on your computer, found by
  looking at what those are signed in to.

## Routines and the calendar

`schedule` sets a routine: work that wakes you at a time or on an event.
`calendar` records a date the crew is answerable for, and wakes nobody. If you
need to prepare for something on the calendar, write both.

## Updates

The app and the host update separately, and either can be ahead. When their
versions differ, the app says which runs what and which to update, and
**Settings > Workspace** shows both versions and the latest release. A host
this Mac manages updates from there with a backup first; a remote host is
updated on that machine, and the pane links the instructions. Updating a host
interrupts work that is running, and interrupted conversations wait for the
operator to review them rather than being repeated.

## What you cannot do

- You cannot read an API key, a secret's value, or a sign-in's cookies. Nothing
  that crosses into a prompt carries one.
- You cannot touch another crew: its agents, skills, calendar, connectors or
  secrets.
- You cannot change the operator's appearance or notification preferences;
  those belong to each window.
