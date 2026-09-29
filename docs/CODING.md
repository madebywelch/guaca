# Coding

An agent can be given a terminal: a directory of its own on the host, the
shell that starts there, three file tools, and a coding harness it can hand a
change to. `domain/terminal.rs` is the shape and the rules that need no disk,
`terminal.rs` is the disk, `shell.rs` is the small door, `coding/` is the
program that does the long work, and `server/console.rs` is the operator's
own way in.

The host is wherever `guacad` runs: a container on the operator's Mac, or a
box they connected to. Nothing here runs on the operator's own machine outside
that, and every path an agent sees is the host's.

## Why a directory of the agent's own, and not a repository

A repository used to be a row. The operator linked a directory, or on a box
the workspace cloned a remote, and the row carried a credential, a commit
identity, a harness, a push gate and a worktree policy, with at most one agent
assigned. By the end each agent already worked in a git worktree of its own
rather than in the operator's checkout, and the host already ran in a
container, so the row was bookkeeping about a directory the agent owned
anyway. It was also most of what an operator had to understand before an agent
could write a line: link, clone, token, identity, harness, gate, bench, assign.

The terminal replaces it, and each part of the row went somewhere simpler.

- **The remote** is the agent's to clone. `git clone` in its own directory,
  like anybody else.
- **The credential** is the host's. The desktop's container and a box both
  hold git and `gh` sign-ins on their persistent volume, and a `GH_TOKEN`
  secret granted to an agent signs in both for that agent alone. See
  *Credentials belong to the host* below.
- **The commit identity** is the host's git config.
- **The harness and the gate** are the agent's, because they are about how
  this agent's work happens, and one crew can hold an engineer on each plan.
- **The worktree policy** is gone. The agent's directory is its own, so there
  is nothing to share and nothing to reset behind its back.
- **The footing**, what a job is told about the tree it starts in, stayed
  exactly as it was.

Migration 54 carried the one decision worth keeping: an agent that was put in
a repository was trusted with a shell there, so it arrives with a terminal and
the harness and gate it worked under. What was in the repository stays where
it was on disk; the agent starts in an empty directory and clones what it
needs.

## Three doors, sized by the work

**`shell` runs one line and answers in the turn.** `git clone`, `git status`,
`gh pr view`, `gh pr merge`, one test. It waits, it is killed after
`shell::PATIENCE`, and its output is clipped to `KEPT` bytes with both ends
kept. Every call starts in the agent's directory, so a line that needs to be
inside a repository says `cd guaca && ...` or `git -C guaca ...`.

**`read`, `write` and `edit` look at and change files.** They exist beside the
shell because a model editing through it writes `sed` expressions and heredocs
that break on the first quote or dollar sign in the text. Every coding harness
ships an exact-replacement edit for that reason, and this is the same tool:
each `old_text` has to appear exactly once, every replacement is matched
against the file as it was, and if any of them does not match nothing is
written. `read` pages a file at 2,000 lines or 50 KB, whichever comes first,
and says where it is in the file. `read` takes an absolute path anywhere,
because reading changes nothing and the shell can read anywhere anyway;
`write` and `edit` stay inside the directory, resolved through links, because
an agent editing a file outside the directory it was given is an agent wrong
about where it is standing. That is a refusal about the ordinary case and not
confinement.

The names are pi's and Claude Code's, which is what a model reaches for. They
sit one word away from `read_file`, which reopens an attachment somebody sent,
so each description says which one it is in its first sentence.

**`code` hands a change to a coding harness for minutes.** A Guaca turn is one
model call plus `max_tool_rounds` rounds, twenty-four by default, inside a
conversation bounded at sixty model calls, and there is no compaction inside a
turn. A real change to a repository is a few hundred tool calls, its own
context window and its own compaction. Reaching that with the file tools means
raising both limits to coding scale, and both are per group, so the guard that
keeps a crew of eight from talking forever comes off for every agent in every
crew. So the harness keeps its own loop, its own context and its own budget,
and Guaca spends one tool round starting it. `code` returns as soon as the
process is up, and the answer arrives minutes later as a message on a fresh
run. Awaited inside the tool call, the agent would read as `Thinking` for the
length of a change, its inbox would back up behind it, and every routine that
came due would be skipped.

The prompt's `## Your terminal` section says which door is for what, because a
tool list is read while deciding how to do something and the prompt is read
while deciding whether it can be done at all.

## It is not a sandbox, and must never be described as one

The shell, the file tools and the harness all run as the host's user, with that
user's credentials and network. A command can reach anywhere that user can.
The directory is where work starts and where an agent's files are kept, not a
boundary around them. What makes handing it over defensible is that the
operator decides which agents get one, and git is the undo for what happens in
a repository.

That is also why there is no second setting per door. The decision was taken
when the operator gave the agent a terminal; a switch per tool would be a second
place to say the same thing, and the first agent to find one off would report
its terminal as broken rather than as fenced.

On a desktop the host is a container, so the agent does not see the operator's
files unless the container mounts them. That is a property of where the host
runs, not a boundary this code draws.

## Credentials belong to the host

git and `gh` are signed in where the host runs, once, and every agent's
terminal uses that. The image configures git system-wide to ask `gh` for
GitHub credentials (`credential.https://github.com.helper = !gh auth
git-credential`), so two arrangements work without anything in Guaca holding a
token:

- **The host's own sign-in.** `gh auth login` as the host's user, from
  **Open terminal** in any agent's Terminal panel (below). Outside the app the
  same user is `docker exec -it <container> gh auth login` on a desktop and
  `docker compose exec guacad gh auth login` on a box. It lives on the
  persistent volume.
- **A secret per agent.** A `GH_TOKEN` secret granted to one agent reaches its
  shell and its coding jobs as an environment variable, and `gh` and git both
  read it. That is how one agent can push to a repository another cannot: the
  scope is the token's, chosen when it was made on GitHub.

The commit author is the host's git config: `git config --global user.name`
and `user.email` as the host's user. Secrets cannot set it, because `GIT_*`
names are reserved against exactly that kind of override (`connector::reserved`).

What went away is Guaca's own credential plumbing: a plaintext
credential-store file per repository, reuse of saved tokens across
repositories, per-repository identity, and a GitHub App broker with its own
container, private key and `gh` wrapper. Every one of those was a second place
for a credential to live beside the one the harness programs already read, and
the programs were the ones pushing.

## The operator has a door of their own

Every sign-in an agent is refused for is a command somebody types on the host:
`gh auth login`, `claude auth login`, `codex login --device-auth`. Each asks
questions, draws a menu or waits for a code, so none of them can go through
`shell`, whose stdin is closed. Before this door, an agent refused a push said
*run `gh auth login`*, correctly, to an operator with no idea where: the
answer was `docker exec -it` into a container they may never have looked at,
or `ssh` to a box and then that.

**Open terminal**, in the agent's Terminal panel, is that door.
`server/console.rs` starts `bash` on a pseudo-terminal in the agent's
directory, as the host's user, and a socket at `/v1/console/<agent>` carries
it to xterm.js over the window. It is what `docker exec -it <container> bash`
gave and nothing more, behind the token every other route asks for, and it
grants nothing that token did not: a connector run as a program is already a
command of the operator's choosing, run on this host. Four decisions in it are
not defaults.

- **No secret Guaca holds is in it.** A secret granted to the agent reaches the
  agent's `shell` and its jobs and never this, because this draws in the
  webview and a secret's value never does. The names are removed as well as
  not added, so a `GH_TOKEN` the host was started with cannot stand in for the
  sign-in being made: `gh auth login` will not store one while it is set.
- **It lives exactly as long as its socket.** Done, a dropped network and a
  heartbeat nobody answers all close the master side, which hangs up `bash`,
  which hangs up what it started; a shell still there two seconds later is
  killed. Nothing is kept for a page that might come back, because a shell
  nobody is watching, holding the host's sign-ins, is the one thing this must
  not leave behind. `nohup` and `setsid` still work, because that is what they
  are for.
- **Nothing typed reaches a transcript.** It is the operator's shell standing
  in the agent's directory, not the agent's. The agent sees what changed on
  disk and nothing of how.
- **Escape is the shell's.** A sign-in's menu, readline and every editor read
  it, so the only way out of the view is Done.

It is `bash` because that is what `shell` runs, so the operator sees what the
agent sees, and interactive because its input is a terminal, so it reads the
host user's `~/.bashrc` as `docker exec -it` did. The prompt's credentials
bullet names the button, so an agent refused for a sign-in says where to make
it.

## The gate is asked from one function, for every door

`Gate::AskBeforePushing` stops a push, a merge, a pull request or a release,
whether the line says so itself or keeps it in one of the repository's own
scripts. It stops `shell` and each harness's command tool at the same commands,
decided by the same `bridge::outward` call, answered on the same desk through
the same `Runtime::ask_about_push`. Two doors that disagreed about what counts
as outward-facing would be a gate an agent walks around by picking the other
tool, which is worse than no gate at all: the operator switched it on and
would be told it was holding.

`Asker` is everything that differs. A job is not a turn and its agent may be
idle or answering somebody else, so its request does not move the dot beside
the agent's name; a `shell` call is a turn genuinely stopped mid-inference and
has to say so.

A denial refuses the *call*, not the outward-facing part of it, exactly as a
hook's `deny` does. `touch a.txt && git push` runs neither half. A refusal that
ran the first command and stopped at the second would leave the tree in a
state nobody asked for and nobody was told about.

### The line is read, and then what the line runs is read

Reading the words alone is what one level of indirection walks straight past.
`./scripts/ship.sh` is not `git push` and never will be, so a repository whose
release lives in a script had a gate that was switched on, said it was holding,
and stopped nothing.

So a line that names nothing outward-facing is looked at again, for the scripts
*in this tree* that it runs: a `package.json` entry behind `npm run release`,
`pnpm release` or `yarn release`, and a file behind `./scripts/ship.sh` or
`bash scripts/ship.sh`. Each one is read and asked the same question, three
deep, with a set carried across the whole walk so that two scripts that run
each other are read once rather than forever.

A `cd` moves where the rest of the line runs, and what comes after it is read
from there. That is the ordinary shape of a line in a terminal holding several
repositories, `cd guaca && npm run release`, and read from the top of the
terminal it named a `package.json` that is not there. A `cd` out of the tree
leads somewhere nothing is read.

The card carries both halves, because they are not the same sentence:
*wants to run `git push`, by way of `guaca/package.json "release"`*. What is
being authorized is the thing that cannot be taken back, and the script is how
the agent got there. The `Command` detail field holds the line the model
actually wrote, flags and all, which is the one detail on a card that changes
the answer.

### And what it deliberately does not read

A Makefile target, a compiled program, anything it cannot parse as text. Those
are indirection too, and reading none of them is a decision rather than a gap.
Treating *there is something here I cannot see through* as a reason to ask
sounds like the safe direction and is not: the ordinary case for it is
`./target/release/app` and `./node_modules/.bin/vite`, and a gate that parks a
turn for running the binary it just compiled is the wrong yes that teaches an
operator to switch it off.

### One no settles the question for the rest of the run

A model that has just been refused a push tries the push. `Runs::refused`
holds what each run has already been told no about, keyed by the outward
action the card named rather than by the line, and `ask_about_push` reads it
before it parks anything. A no is remembered and nothing else is: an expiry is
the operator being somewhere else rather than answering. It is per run, so the
operator's next message starts clean. A standing grant to act on the
operator's behalf outranks it, because it is the broader answer and the newer
one.

### It is not a boundary

A shell line is not something anything can parse without a shell, and a
command that wanted to get around this could. What it buys is that the
ordinary push, made by an agent doing what it was asked, is one somebody sees
first.

## The gate and the harness are the agent's

Not the crew's, and not a repository's. The harness is about which plan pays
for this agent's code, and one crew can hold an engineer on a Claude plan and
another on OpenRouter. The gate is about whether this agent's pushes need a
hand on them. Both are columns on the agent, beside `has_terminal`, and both
survive taking the terminal away: an operator who gives it back means the agent
they configured.

A job reads both when it starts, and a job already running keeps what it
started with. The result an agent is told names the harness that actually ran,
because a conversation about Codex does not prove Codex ran.

The gate is off by default, and that is not caution about a migration.
`APPENDED_PROMPT` tells every job that it is running unattended and that nobody
will answer a question. An operator turning it on is an operator saying they
will be there.

## There are three harnesses because a subscription is spent by one program

This is the part that is not a preference and not a configuration surface.

`pi` can hold an Anthropic OAuth credential and dial the Messages API with it.
What comes back is a 400 saying *You're out of extra usage*, while `claude` on
the same machine, signed in to the same account, runs the same work off the
plan. The same fact is written down from the other end in `PROTOCOL.md`, where
it is why Guaca's own turns cannot be paid for with a Claude sign-in except by
running the program.

So the three map onto what an operator pays with:

- **A Claude plan** is spent by Claude Code.
- **A ChatGPT plan** is spent by Codex, signed in with `codex login`. That is
  a separate sign-in from Guaca's own ChatGPT provider.
- **An API key**, OpenRouter's or any provider's, is spent by `pi`: either the
  one `pi` is signed in to on the host, or Guaca's own, lent through a relay
  that never hands it over (below).

The sign-in, the extensions and the rules file belong to the program and stay
there. The model and the effort default to the program's own setting too, and
the operator can choose them per agent the way the program's own window does,
because a harness whose model cannot be picked from the app is one the operator
has to leave the app to drive.

## A job's model and effort are the program's own flags

Each program already takes both, and Guaca passes the operator's choice as the
program's own argument and nothing more: `--model` and `--effort` for Claude
Code, `model` on `thread/start` and `effort` on `turn/start` for Codex,
`--model` and `--thinking` for `pi`. Nothing chosen is nothing passed, which is
what every job ran on before the choice existed.

`coding_tuning` keeps one row per agent *per harness*. One field for all three
is the mistake `InferenceConfig` made with two providers: the programs share no
model names and no effort words, every switch broke the model, and switching
back did not put it right. An engineer moved to Codex for an afternoon because
a Claude plan ran out comes back to the Claude model it had.

The panel suggests models asked of the program rather than listed here, because
the list is the program's and moves with its releases and its sign-in. None of
the three spends a model call to answer:

- Claude Code answers the SDK's `initialize` control request with what `/model`
  would offer. Its first entry is `default`, which is the absence of a choice;
  the first entry that resolves to the same model is marked as the default
  instead, which is `claude-fable-5-1` on 2.1.283 and the alias `opus[1m]` on
  2.1.260.
- Codex answers `model/list`, each model with the efforts it advertises. Its
  schema types an effort as any non-empty string "advertised by the model", so
  a word the model does not take is not refused at the door: the panel offers
  the chosen model's own list rather than the union.
- `pi` answers RPC `get_available_models` with every model its sign-ins reach.

A stored value is checked against `Harness::efforts` and refused if a model
name starts with `-`, because it is handed over as the argument after `--model`
and would otherwise be read as a flag.

## pi can be paid for with Guaca's key, and never holds it

`pi` takes a key from its auth file or the environment, and either is somewhere
its own `bash` tool can print. A README the job reads can ask it to, and a key
the operator pasted into Guaca would leave the machine in a job's transcript.
So when the operator sets pi to Guaca's key, the key stays in this process and
`coding/relay.rs` lends it:

- The relay listens on loopback and admits one thing: `POST
  /v1/chat/completions` with a live job's token. It sends the body on to the
  lent endpoint with the real key on it, and streams the answer back
  unchanged, delimited by the connection closing.
- The job is handed a provider override in a `pi` extension, which pi documents
  for proxies and gateways: against OpenRouter, pi's own `openrouter` provider
  with only its address and key replaced, so pi's catalog keeps each model's
  context, thinking and price; anywhere else, a provider of the one model the
  job runs, because pi will not run a model it has no entry for. Measured
  against pi 0.84.4 with both forms.
- The token is minted per job and dropped with it. Off this machine it is
  nothing, and after the job it is refused.
- Every call is metered on its way back: the `usage` the endpoint reports is
  recorded against the job's run in the same table and the same live tally a
  turn's own call is. The key is Guaca's, so its spend is in Guaca's account.

It is chosen, never inferred: *nothing about who pays is inferred* holds here
as everywhere. No model chosen is the key's own, `default_model`. No key is a
job refused in the turn that asked, with both ways on: paste a key, or set pi
back to its own sign-in.

### Which key is Guaca's is the agent's group's question

The endpoint, key and endpoint model are layered the way a turn's are, the
agent's group over the app (`GroupInference::endpoint`), and read whoever pays
for the group's turns. That last part is the point. A crew on a ChatGPT sign-in
keeps the endpoint and key it held before, kept rather than blanked so it can go
back, and an operator who gave a crew an OpenRouter key and its turns a
subscription wants the replies on the plan and pi on the key. Lending only the
app's key refused that crew with a key sitting in its own settings, and the
panel said Guaca had none. The model is the endpoint's own and never the
subscription's, which the endpoint refuses by name; that is why the lend is not
`inference_for`, which collapses to the provider paying for turns.

The panel names the group when the key is the group's own, because two crews'
keys can be two accounts and the bill is the thing the operator is choosing.

## A job is told where it is standing before it is told what to do

A harness handed a brief starts editing where it is standing, and where it is
standing is wherever the last job left the tree. Nothing prompts it to look at
the branch first: a job that opens a pull request ends on the branch it made,
the operator merges it, and every job after that starts on a feature branch
whose work has already landed. Weeks of work can stack on top of it before
anybody notices, and the transcript reads correctly throughout.

So `repo::footing` reads the tree at the moment a job starts, and
`Runtime::start_job` puts it in front of the brief: the branch, whether it is
clean, whether it tracks anything, whether that branch is already contained in
the default one, and whether a pull request is open for it. Then one rule the
facts resolve to.

Both halves are load-bearing. Facts alone are not enough: a model handed a
branch name and a count decides for itself what to do with them, and the
decision it makes silently is to carry on where it is standing. A rule alone
cannot be written safely: *start from the default branch* over uncommitted work
destroys it. The facts are what let the rule be conditional, and uncommitted
work is checked before anything else and overrides every other case: do not
switch, do not stash, do not clean, work from here or stop and say why.

A directory that is not a repository is said to be one. The standing prompt
tells every job that commits are its only undo, and a job that believed that
while standing somewhere git cannot see would edit for forty minutes with no
checkpoint at all. The brief says to clone the repository the work belongs in,
or `git init` before the first edit.

Four details are not guesses.

- **The merge test runs against `origin/main`, not `main`.** A branch merged
  upstream has landed whether or not the local copy was ever pulled.
  `default_branch` returns a name for the prose and a ref for the test,
  because they are two different things.
- **`origin/HEAD` first, `main` and `master` only after it.** The repository's
  own answer beats a guess, and where nothing published one, the preamble says
  so and asks the harness to decide.
- **Every count is against the last fetch, and the preamble says so even when
  they are zero.** Fetching on the agent's behalf is not this app's call; the
  harness is the thing standing in the directory with a shell.
- **A repository with no commits says so.** Git names an unborn HEAD after the
  branch the first commit will create, so without it the preamble says *on
  branch `main`* and *there is no `main`* two lines apart.

Where the work lands is not decided here. That is the brief's to say.

## One job per agent, and that is the whole lock

`Runtime::start_job` keeps one running job per agent. Two harnesses in one
directory interleave their edits and run git against each other, and nothing
downstream could say which of them wrote what. An agent's terminal is its own,
so the agent is the thing that can only take one at a time; two changes at
once are two agents, talking in the crew they share.

`shell` takes no lock. One line is the same thing as somebody typing in a
terminal while a job runs, and refusing it would take away the read an agent
most wants while a job is running, which is what the job is doing.

## One process lifecycle, three of what genuinely differs

What the three share is the shape of a job: one process, in one directory,
speaking JSON one object per line, under one forty-five minute ceiling, with
the same redaction on everything it says. Where they differ is who holds the
conversation. Claude Code's stdout is a stream that ends, so `coding/mod.rs`
holds its spawn, read loop and kill, and the hooks in `coding/bridge.rs` are
how it is reached. `pi --mode rpc` and Codex's app-server keep stdin open for
the life of the job and take commands on it, so `coding/pi.rs` and
`coding/codex.rs` each drive their own process and end it by closing stdin once
the turn settles. Each submodule holds its argument vector and the fold from an
event into an `Outcome`.

Three details of the vectors are load-bearing and none of them is guessable.

- **`claude` refuses `--output-format stream-json` without `--verbose`.** The
  refusal is on the command line, so the failure is a job that never starts
  rather than a job that fails.
- **Every harness runs in the mode that does not ask.** `pi` has no permission
  system of its own; Claude Code is given `--permission-mode
  bypassPermissions`; Codex runs with `approvalPolicy: "never"` unless the gate
  is on. There is nobody to answer an ordinary prompt: the job is started by an
  agent and runs unattended for many minutes.
- **No harness is asked for a session-less run.** A session on disk is what
  lets the operator open the same work in their own terminal, which is the
  difference between a harness the app runs and a black box.

The tool tables are separate per harness for the same kind of reason. `pi`'s
built-ins are lowercase and carry `path`; Claude Code's are capitalized and
carry `file_path`. Merged, one program's field name gets read out of the
other's arguments, and a wrong guess prints somebody's file contents into a
channel. Anything not in a table draws no detail at all.

## Whatever the program's own terminal can do to a job, Guaca can

The test is the operator's own terminal. Anything they could do to a job there
(correct it mid-turn, stop it, carry it on tomorrow, open it themselves) has to
be possible from the app, through the same interface the program exposes for
it, or the harness is a black box the app happens to run.

| | Claude Code | Codex | `pi` |
|---|---|---|---|
| Correct it while it runs | hook mailbox | `turn/steer` | RPC `steer` |
| Stop it | SDK `interrupt` on stdin | `turn/interrupt` | RPC `abort` |
| Carry it on after | `--resume <id>` | `thread/resume` | the same `--session-id` |
| Choose the model | `--model` | `model` | `--model` |
| Choose the effort | `--effort` | `effort` | `--thinking` |
| Ask before a push | `PreToolUse` hook | approval callbacks | extension, answered over RPC |
| Open it yourself | `claude --resume <id>` | `codex resume <id>` | `pi --session <id>` |

All of it goes through `code`, for an agent, and `message_coding_job` and
`stop_coding_job`, for the operator. `continue` and the operator's box are one
call: into the job if one is running, or a new turn in the last session if not.
The runtime decides which, because the job can end between the keystroke and
the call, and words typed at a job that just finished are a follow-up rather
than an error.

`code` returns the moment the process is up. The cost used to be paid at the
other end: for up to forty-five minutes a job was write-only.

**Claude Code** is reached through `coding/bridge.rs`. Hooks run at fixed
points in its own loop, are handed the event on stdin, and what they print back
is acted on. Guaca writes a settings file and a three-line `sh` script per job,
passes them with `--settings`, and answers the hook over a loopback socket:

- **A mailbox.** `message_coding_job` stages a correction; the `PostToolUse`
  hook delivers it as `additionalContext` at the job's next tool boundary, and
  the `Stop` hook delivers one typed at minute forty-four instead of letting
  the job finish without it. `take_mail` reads and clears together, because
  mail delivered twice is an instruction the model was given twice.
- **A gate**, when the agent asks for one: a `PreToolUse` hook on `Bash` whose
  `deny` overrides `--permission-mode bypassPermissions`.
- **Two ways to report**, on a small MCP server passed with `--mcp-config`:
  `note_progress` and `report_pull_request`.

Everything the Claude bridge adds is an improvement on a job that already
worked, which is why every part of it fails open: a bridge that did not start,
a `curl` that is not installed and a server that already dropped the job all
end with an empty answer and an exit status of zero. The gate is the
exception, and fails closed.

**Codex** is reached through its own protocol: `turn/steer` with the active
turn id, acknowledged before Guaca reports it sent; a correction sent before
the first turn exists is held until it does. `turn/interrupt` stops it, and
the turn comes back `interrupted` rather than failed. Command approval
callbacks are the gate.

**`pi`** runs in `--mode rpc`, the interface its own editor integrations use.
The brief is a `prompt` command rather than an argument, a correction is
`steer`, which pi puts in front of the model after the tool calls it is
running, and a stop is `abort`, which ends the turn and keeps the session. The
job is over at `agent_settled`, not `agent_end`: pi can retry after an
`agent_end`, and closing stdin then would cut off the retry. `RPC_FLOOR` is the
version all of this was measured against, and an older `pi` is refused with the
update command rather than driven over a protocol nothing here has checked.

pi has no permission system of its own, so its gate is an extension, written to
a scratch directory per job and loaded with `-e`. It decides nothing: every
`bash` call asks `ctx.ui.confirm`, which arrives as an `extension_ui_request`,
and Guaca reads the line with `bridge::outward` and answers
`extension_ui_response`, so an ordinary line is confirmed without asking
anybody and a push waits on the desk. Before the brief is sent, Guaca asks
`get_commands` for the command the extension registers. A gate that did not
load is a job that does not start, because the alternative is a push the
operator asked to be asked about, run by a job that looked gated.

### The three things the Claude bridge rests on are behavior, not flags

None of them can be checked offline, which is why `tests/coding.rs` keeps an
`#[ignore]`d half that asks the real program. Measured against 2.1.247:

- A `PreToolUse` hook answering `permissionDecision: "deny"` **overrides
  `--permission-mode bypassPermissions`**. Without that the gate would be a
  suggestion.
- A `Stop` hook's `reason` reaches the model, as a synthetic user message. That
  is what lets the `Stop` hook block *and* deliver in one call, which is what
  makes it terminate.
- `additionalContext` from `PostToolUse` is put in front of the model before
  its next round.

### Stopping leaves what it committed, and the session

Each program is stopped the way its own interface stops it, so the stop is
something the program recorded: the Codex turn ends `interrupted`, the pi turn
`aborted`, and Claude Code's `result` comes back `error_during_execution` with
`terminal_reason` `aborted_streaming`, which the driver reads as the stop it
asked for rather than a failure. Each is in the session a follow-up resumes.
Each gets `STOP_GRACE` to end on its own before the process is killed anyway.

Claude Code is reached for this over stdin, which is why its brief is the first
message on stdin rather than an argument: `--input-format stream-json` keeps the
pipe open for the SDK's control requests. In that mode it does not exit at
`result` but waits for another message, so the driver closes stdin there.
Measured against 2.1.260 and 2.1.283, and the live half of `tests/coding.rs`
interrupts one and resumes it.

A host that is stopping stops every job this way too, and that is what lets an
update carry a job on rather than end it. Nobody is told the job stopped,
because it has not: it is kept in `put_down_jobs`, and the next host continues
the same session, told the host restarted and that its last step may not have
finished. A job stopped before Codex named its thread has no session to
continue and is started over on its brief. *A stopping host puts its work
down* in `HOSTING.md` is the rest of it.

### A program is ended by closing its input, not by a kill

All three exit in milliseconds when their stdin closes, and a kill is not the
same thing to them. A `pi` that is killed leaves the next `pi` to start waiting
about thirty seconds before it answers anything, measured against 0.84.4: a
model listing that killed pi made every panel after it open half a minute late,
and the same would hold for a job started after one. So a job, a stop that was
answered and a listing all end by closing stdin, and the kill is for a process
that has not gone five seconds later.

Nothing is reverted, because the commits a job is told to make as it goes are
the operator's checkpoints and throwing them away is not that button's
decision. The agent that started the job is told the work is *partly* done and
nobody has checked which part, because an agent told only that the job stopped
reports the work as not done and leaves the operator to discover half of it on
a branch. A stop the agent asked for with `code` `stop` is not reported back
to it: it has just decided that. Taking the terminal away and deleting the
agent both stop its job the same way.

### A session is carried on by the program that wrote it

`coding_sessions` holds one row per agent: the harness, its session id, and the
directory it ran in. Stored rather than held in memory, because a follow-up
often comes the next day. Only the last one: an agent runs one job at a time,
and the session before the last is one it has already moved on from.

The id is chosen where the program lets it be chosen. `--session-id` takes a
UUID for both Claude Code and `pi`, so Guaca picks one before the job starts,
and one value is then the job's address on the bridge, the key of its mailbox,
and what the operator hands to `--resume` or `--session`. Codex names its own
thread, so the id is read back from `thread/start` as soon as it exists,
before the turn it runs can end or be stopped.

A follow-up is refused, with the way on, in three cases: no job has run, the
agent's harness has been switched since (the ids mean nothing to the other two
programs), or the directory the session ran in is gone. A follow-up the
operator typed is sent as *The operator says: ...*, and the agent is told the
operator went round it when the result comes back, because it is the one that
will be asked what came of it.

## A job inherits the host's own Claude Code setup, and that is deliberate

`coding/claude_code.rs` passes no `--strict-mcp-config` and no
`--setting-sources`, which is the exact opposite of `llm/claude.rs`, and the
two are right for opposite reasons. A turn there is answered by a program that
should have this app's tools and nothing else. A job here is a coding agent
working in a repository, where the host user's `CLAUDE.md`, the repository's
own settings and the MCP servers configured there are what make it good at the
work. `--settings` and `--mcp-config` are both additive.

Measured on one machine on 2026-08-27 against 2.1.247, a job started this way
loaded 16 MCP servers, 229 tools, 100 slash commands and 8 agent definitions,
none of which Guaca chose. The one hazard is a blocking `Stop` hook of the
host user's own, which holds a Guaca job against its own completion until the
forty-five minute ceiling. Guaca cannot fix that from outside; the ceiling is
the backstop.

## A failed turn is not a job with nothing to do

Every harness reports a failed turn *inside* its stream and can still exit zero
about it. Read by exit code and text alone that is indistinguishable from a job
that found nothing to change.

It cost an afternoon. An expired Codex token turned every coding job in a live
workspace into a silent no-op, every agent dutifully reported that nothing
needed doing, and `pi auth check` called the provider ready throughout. So
`Outcome` carries `failed`, it is taken from the *last* message rather than the
first so a turn that failed and was retried is not a failed job, and
`job_finished` reports it before the empty case.

It also reaches the operator. `CodingJobFailed` raises a banner that names the
directory, the program that stopped, and the harness's own words: with three
harnesses, the way out of the commonest failure here is another one.

## Codex runs through its official CLI

`Harness::Codex` starts `codex app-server --listen stdio://` in the job's
directory. Codex owns model selection and authentication. On a box, as the
daemon's user, run `codex login --device-auth`. The image pins the CLI version;
its configuration, credentials and sessions live in the persistent home
volume. The control contract is measured against Codex 0.153.3.

Before creating a thread, the runner asks that same app-server for
`account/read`. If its provider requires OpenAI authentication and no account
is present, the job ends with the sign-in command before any model request.

A correction uses `turn/steer` with the active turn id, and Guaca reports it
sent only after Codex acknowledges that id. **Ask me before pushing** selects
Codex's `untrusted` approval policy and user reviewer, verified in the thread
response before any work starts; command approval callbacks go through the same
`bridge::outward` and operator decision as the other doors. The CLI keeps its
own configured command rules, so an allow rule there can bypass its approval
callback. Keep outward commands out of those rules when relying on this gate.

## What is not here

Any confinement. See above.

Nor is the spend, except where Guaca paid it. Every harness reads its own auth,
and a job's cost does not appear in this app's usage table because this app did
not spend it; what a job reports back is what the harness says it cost. A pi
job on Guaca's key is the exception, metered through the relay.

## What repositories left on disk is put away at boot

Before terminals, a workspace kept a work tree per agent per repository under
`<data>/worktrees`, the clones a box made under `<data>/repos`, and a token per
clone under `<config>/repo-credentials`. Nothing reads any of them now, and
`leftovers.rs` runs at every boot and tidies them where nothing is lost by it:

- The tokens go unconditionally. A credential nothing uses is one nobody will
  remember to rotate.
- A work tree with nothing uncommitted, whose commit is still held by the
  repository it came from, is removed through `git worktree remove`, which
  also takes it out of that repository's own list. An operator's own checkout
  keeps every branch; a clone this workspace made keeps a commit only until the
  clone goes, so there the commit has to be on a remote.
- A work tree holding work that exists nowhere else is moved into the terminal
  of the agent it belonged to with `git worktree move`, still linked, so the
  agent finds it where it now works.
- A clone goes once it is clean, has nothing unpushed and backs no work tree.
- Anything else is kept, and logged with the reason every time the workspace
  opens, until somebody deals with it.

It runs in the background, because it runs git and a workspace must not wait on
a slow disk to open, and it is a no-op once there is nothing left.

## Testing it

`tests/coding.rs`. The offline half puts real stand-in executables on `PATH`,
because the thing being tested is a process: a fake in front of the fold would
be a test of the fold, which already has one beside it in each submodule. Each
stand-in records the argument vector it was handed into the directory it was
started in, so the suite can assert that an agent set to Claude Code starts
`claude` with Claude Code's vector, in the directory `code` named, which is the
seam nothing else can see.

The runtime tests give an agent a terminal whose directory *is* a test
repository, so a job started with no directory runs where the stand-in's
instructions were written. The ordinary arrangement, a repository in a
directory of its own inside the terminal, has its own test.

The `shell` and file-tool tests need no stand-in, because the programs they run
are `bash` and the filesystem. They are the only place the doors are checked
against each other: that a line runs in the terminal and answers inside the
turn, that the gate stops the same commands through every door and that a
denial runs no part of the line, that an ordinary line is not stopped, that
what `write` made `edit` changes and `shell` reads back, and that a line still
runs while the agent's job is going.

The operator's shell is tested over its socket in `tests/server.rs`, against
real `bash`: refused with a sentence for an agent with no terminal, closed to
a wrong token, drawn at the size the page measured and told when it changes,
started without the agent's secrets, and gone, with the program it was
running, once the socket closes.

The three stand-ins are Python programs in `tests/fixtures/` that speak each
protocol's real shapes: Claude Code's stream-json in both directions, including
the `interrupt` and `initialize` control requests and the wait for stdin to
close after `result`; Codex's app-server; pi's RPC, including a provider an
extension readdresses, which the stand-in really calls, so the relay is tested
end to end from the job's side.

The `#[ignore]`d half asks the real programs whether they still accept those
vectors and still behave the way this build reads them. Three of those tests
spend nothing and are worth running on any machine with the programs on it:
the model listings, and pi paying through the relay against a loopback
endpoint. The interrupt test spends one small call on the host's Claude plan,
on haiku. Run the half against the versions the image pins, not only the ones
on the machine: `PATH=<dir with the pinned binaries>:$PATH` in front of the
command is enough.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test coding
cargo test --manifest-path src-tauri/Cargo.toml --test coding -- --ignored
```
