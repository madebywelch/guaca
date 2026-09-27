# Coding

An agent can be given a terminal: a directory of its own on the host, the
shell that starts there, three file tools, and a coding harness it can hand a
change to. `domain/terminal.rs` is the shape and the rules that need no disk,
`terminal.rs` is the disk, `shell.rs` is the small door, and `coding/` is the
program that does the long work.

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

- **The host's own sign-in.** `gh auth login` as the host's user, in the
  container: `docker exec -it <container> gh auth login` on a desktop,
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
- **An API key**, OpenRouter's or any provider's, is spent by `pi`, configured
  with that key the way `pi` is configured anywhere.

What is *not* a choice here is everything inside a harness. The model, the
thinking level, the extensions, the rules file and the sign-in all belong to
the program and stay there. `coding/pi.rs` passes no `--provider` and no
`--model`, and `coding/claude_code.rs` passes no `--model`, for the reason the
binary is found on `PATH` rather than configured: a second place to say
something is a second place for it to be wrong.

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

What `pi` and Claude Code share is the shape of a job: one process, in one
directory, whose stdout is a stream of JSON objects, one per line, that ends.
So `coding/mod.rs` holds the spawn, the read loop, the ceiling, the kill and
the exit handling, and each submodule holds the argument vector and the fold
from an event into an `Outcome`. Codex owns a bidirectional app-server session
in `coding/codex.rs`, ending its process after the active turn completes.

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

## A job can be reached while it runs, on two of the three

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

**Codex** is reached through its own protocol: `turn/steer` with the active
turn id, acknowledged before Guaca reports it sent, and command approval
callbacks for the gate.

**`pi`** has neither in this build and gets none of it. Everything the bridge
adds is an improvement on a job that already worked, which is why every part
of it fails open: a bridge that did not start, a `curl` that is not installed
and a server that already dropped the job all end with an empty answer and an
exit status of zero.

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

### Stopping leaves what it committed

The process is killed where it stands. Nothing is reverted, because the commits
a job is told to make as it goes are the operator's checkpoints and throwing
them away is not that button's decision. The agent that started the job is
told the work is *partly* done and nobody has checked which part, because an
agent told only that the job stopped reports the work as not done and leaves
the operator to discover half of it on a branch. Taking the terminal away and
deleting the agent both stop its job the same way.

### The session id is chosen, not read back

`--session-id` takes a UUID, so Guaca picks one before a Claude job starts. One
value is then the job's address on the bridge, the key of its mailbox, and what
an operator hands to `claude --resume` to open the same work in their own
terminal.

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

Nor is the spend. Every harness reads its own auth, and a job's cost does not
appear in this app's usage table because this app did not spend it. What a job
reports back is what the harness says it cost.

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

The `#[ignore]`d half asks the real programs whether they still accept those
vectors and still answer in the shape this build reads. It spends the host
user's own plan.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test coding
cargo test --manifest-path src-tauri/Cargo.toml --test coding -- --ignored
```
