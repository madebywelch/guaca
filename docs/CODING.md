# Coding

An agent can be given a repository: a directory on the operator's own machine
that is the root of a git work tree. `domain/repository.rs` is the shape and the
rules that need no disk, `repo.rs` is the disk, and `coding/` is the program that
does the writing.

Guaca does not write code. It starts something that does, in the directory the
operator linked, and reads what comes back. There are two of those things, `pi`
and Claude Code, and the operator installs and signs in to whichever ones they
use. Why an agent gets a directory rather than a tier, why it gets at most one,
and why the linked path has to be the root, are in `domain/repository.rs`. What
follows is what will bite you in the code.

## A turn and a coding task are different units of work

This is the whole reason there is a harness at all rather than `read` and `edit`
tools in this runtime.

A guaca turn is one model call plus `max_tool_rounds` rounds, twenty-four by
default, inside a conversation bounded at sixty model calls. A real change to a
repository is a few hundred tool calls, its own context window and its own
compaction. Reaching that with tools here means raising both limits to coding
scale, and both are per group, so the guard that keeps a crew of eight from
talking forever comes off for every agent in every crew.

So the harness keeps its own loop, its own context and its own budget, and Guaca
spends one tool round starting it. The `code` tool returns as soon as the process
is up, and the answer arrives minutes later as a message on a fresh run. Awaited
inside the tool call, the agent would read as `Thinking` for the length of a
change to a codebase, its inbox would back up behind it, and every routine that
came due would be skipped.

## A repository has two doors, and the small one is `shell`

The section above is the argument for the harness. It is not an argument for the
harness being the *only* way in, and for a while it was, which turned out to be
the more expensive half.

An agent given a repository could do exactly one thing with it: hand a brief to
a coding harness and wait minutes for a message. That is right for a change to a
codebase and wrong for everything else an agent needs from a directory. `git
status`. `git log -1`. Reading a file. `gh pr view`. `gh pr merge 98`. Each of
those is a question with an answer, and each of them cost a whole coding job:
its own process, its own context window, its own spend on somebody's plan, and
an answer that arrives after the turn that wanted it has ended.

The failure that made it worth fixing was not the cost. It was what an agent
does when the one door will not open. A harness refuses to start for ordinary
reasons — a spent plan, a program not installed, another job already in the work
tree — and an agent holding a repository, asked to merge a pull request, then
reports to the operator that it has no shell and no way to reach GitHub. Both
sentences are true of a design with one door in it, and neither is true of the
machine it is running on, where `gh` is installed and signed in.

So `shell` runs one line in the repository and hands back what it printed, in
the turn that asked. It is offered on exactly the same condition as `code` —
`Surfaces::repository`, which is the operator having put this agent in a
directory — and it is the same directory. `src-tauri/src/shell.rs` is the whole
of the running; `Runtime::run_in_repository` is the gate and the lookup in front
of it.

### It adds directness, not reach

The line runs as the operator, in their directory, with their credentials and
their network. That is the same sentence *What is not here* writes below, and it
was already true: a coding job in that directory runs arbitrary commands under
`--permission-mode bypassPermissions`, with the operator's own MCP servers and
hooks loaded. Nothing an agent can do through `shell` was out of reach through
`code`. What is new is that it costs a second instead of ten minutes.

That is also why there is no new operator setting. The decision was taken when
they linked the directory and put an agent in it; a second switch would be a
second place to say the same thing, and the first agent to find the switch off
would report the repository as broken rather than as fenced.

### The gate is asked from the same function, and that is the point

`Gate::AskBeforePushing` stops a job before a push, a merge, a pull request or a
release, whether the line says so itself or keeps it in one of the repository's
own scripts. It stops `shell` at the same commands, decided by the same
`bridge::outward` call, answered on the same desk through the same
`Runtime::ask_about_push`. Two doors into one directory that disagreed about
what counts as outward-facing would be a gate an agent walks around by picking
the other tool, which is worse than no gate at all: the operator switched it on
and would be told it was holding.

`Asker` is everything that differs. A job is not a turn and its agent may be
idle or answering somebody else, so its request does not move the dot beside the
agent's name; a `shell` call is a turn genuinely stopped mid-inference and has
to say so. The card says which it is, because an operator deciding needs to know
whether they are looking at a job that has been running for ten minutes or at
the agent they are talking to.

A denial refuses the *call*, not the outward-facing part of it, exactly as the
hook's `deny` does. `touch a.txt && git push` runs neither half. A refusal that
ran the first command and stopped at the second would leave the tree in a state
nobody asked for and nobody was told about.

### Two bounds, and neither is confinement

A job gets forty-five minutes because it is its own unit of work. A `shell` call
happens inside a turn, so it gets `shell::PATIENCE`, and its output is clipped
to `KEPT` bytes with both ends kept and the middle counted. Those keep one call
from taking a turn or a context window with it. They are not a boundary and must
not be described as one: what confines this is the directory the operator chose
and the fact that git can undo what happens inside it.

Both bounds say what to do next when they bite, because a tool result an agent
cannot act on is one it reruns unchanged. A killed line names `code` as the tool
for work that takes minutes. A clipped stream names `head`, `tail` and `grep` in
the gap where the middle was.

### No lock, deliberately

`Runtime::start_job` takes one per work tree, because two harnesses in a
directory interleave their edits over minutes and nothing downstream could say
which of them wrote what. One line is not that. It is the same thing as the
operator typing in their own terminal while a job runs, which nothing here
prevents and which is ordinary.

Refusing it would also take away the read an agent most wants while a job is
running, which is what the job is doing — and would put back a version of the
failure the busy refusal already caused once: an agent told to wait for a
message its own turn was holding up.

### A shell and a computer are two machines

An agent with both a computer and a repository is holding two tools that run
shell commands on two unrelated filesystems, which is the `browse`/`use_screen`
hazard one level up. So each description names the other, and only when the
other is actually in the list: `run_command` says the repository is not on that
machine, `shell` says the sandbox is not where the codebase is. A model reads
one description and takes the nearest shell.

A connector's value reaches a sandbox's environment and nothing else, so a
`shell` line naming `$STRIPE_KEY` names a variable that is not set. That is why
`credentials_named_in` is not applied here: prefixing the summary with `used
Stripe` would put a spend in the operator's audit trail that never happened.

## There are two harnesses because a subscription is spent by one program

This is the part that is not a preference and not a configuration surface.

`pi` can hold an Anthropic OAuth credential and dial the Messages API with it.
What comes back is a 400 saying *You're out of extra usage*, while `claude` on
the same machine, signed in to the same account, runs the same work off the plan.
The same fact is already written down from the other end in `PROTOCOL.md`, where
it is why Guaca's own turns cannot be paid for with a Claude sign-in: consumer
OAuth tokens are restricted to the programs they were issued to.

So an operator whose ChatGPT plan is spent and whose Claude plan is not cannot be
helped by any amount of configuration on one harness. They need the other
program. Two variants of `Harness` are the whole of the answer, and a provider
flag on a single command line is not: it would produce exactly the refusal above.

What is *not* a choice here is everything inside a harness. The model, the
thinking level, the extensions, the rules file and the sign-in all belong to the
program and stay there. `coding/pi.rs` passes no `--provider` and no `--model`,
and `coding/claude_code.rs` passes no `--model`, for the reason the binary is
found on `PATH` rather than configured: a second place to say something is a
second place for it to be wrong.

## The choice is on the repository

Not on the workspace, and not on the agent.

Not the workspace, because it is the same shape as the note: a fact about how
work happens *here*. One codebase can be the one whose plan still pays and
another can be the one on an API key, and a single global answer cannot say that.

Not the agent, because two agents in one directory running two different programs
is two coding agents in one work tree, which is what `Runtime::start_job` takes a
lock to prevent. That lock is per repository and is held for the life of the job:
two harnesses in one directory interleave their edits and run git against each
other, and nothing downstream could say which of them wrote what.

The column is `repositories.harness`, added by migration 40 with a default of
`'pi'`, which is a statement of fact rather than a preference: it is what every
row written before it was already running. An unrecognized value reads back as
`pi` rather than failing the row, because the only way to write one is a newer
build and then a downgrade, and refusing to read it would take the repository off
the one panel where the operator could fix it.

## Editing one carries no path, and the type is what says so

`update_repository` takes a `RepositoryEdit`: a name, a note and a harness, and
nothing else. The path is not a parameter because a different directory is a
different repository, and whoever was given this one was given that directory.

It is a separate type rather than a `RepositoryDraft` with the path left out,
because that is what it was and it did not work. A draft validates the path
first, so an edit routed through one has to invent a stand-in, the stand-in was
`/`, and `/` is the empty string once `clean` takes its trailing separator off.
Every rename, every note and every harness switch was refused with *a repository
needs a directory; pick one to link*, about a directory the operator had already
picked and could read on the row above the box. Neither the panel nor the store
was wrong, and both have tests, which is why it lasted from the day repositories
shipped: the only untested seam was the shape being passed between them.

The lesson is not "check the stand-in". It is that a command whose whole rule is
*this field is not editable* must not take a type with that field on it.

## One process lifecycle, two of what genuinely differs

What the two harnesses share is the shape of a job: one process, in one
directory, whose stdout is a stream of JSON objects, one per line, that ends. So
`coding/mod.rs` holds the spawn, the read loop, the ceiling, the kill and the
exit handling, and each submodule holds the two things that differ: the argument
vector, and the fold from an event into an `Outcome`.

Three details of the vectors are load-bearing and none of them is guessable.

- **`claude` refuses `--output-format stream-json` without `--verbose`.** The
  refusal is on the command line, so the failure is a job that never starts
  rather than a job that fails.
- **Both run in the mode that does not ask.** `pi` has no permission system of
  its own and says so in its own documentation; Claude Code is given
  `--permission-mode bypassPermissions`. There is nobody to answer: the job is
  started by an agent, runs unattended for many minutes, and reads its stdin from
  `/dev/null`. A prompt on this path is not a safety control, it is a process
  that hangs until the ceiling kills it.
- **Neither is asked for a session-less run.** A session on disk is what lets the
  operator open the same work in their own terminal with `pi -c` or `claude -c`,
  which is the difference between a harness the app runs and a black box.

The two tool tables are separate for the same kind of reason. `pi`'s built-ins
are lowercase and carry `path`; Claude Code's are capitalized and carry
`file_path`. Merged, one program's field name gets read out of the other's
arguments, and a wrong guess there prints somebody's file contents into a
channel. Anything not in a table draws no detail at all, which is what every MCP
tool the operator has connected falls into.

## Each agent gets a work tree of its own, and Guaca resets it

The linked directory is the operator's own checkout, and until this existed a
coding job ran in it. That made three things true at once.

The job and the operator shared one branch. Two agents in one codebase could not
work at the same time, because `Runtime::start_job` takes a lock per work tree
and there was only ever one. And a job that opened a pull request left the tree
standing on the branch it made: the work landed, the branch was over, and the
rail went on naming it for weeks.

`Bench::Own` is a linked git worktree per agent, under the app's data directory,
one job per agent in it. The operator's checkout is never checked out, never
switched and never cleaned. Two agents are two directories, so the lock moved
from the repository to the directory, which is what the invariant was always
about: two harnesses in one work tree interleave their edits, and two harnesses
in two work trees cannot see each other.

And because Guaca owns the tree, it can put it back. `repo::prepare` runs before
every job: fetch, make the tree if it is not there, and reset it to the default
branch whenever nothing would be lost.

**Whenever nothing would be lost is `Footing::resettable`, and each of its three
refusals is work that exists in exactly one place.** Uncommitted changes, which
is what a job killed at the ceiling leaves. Commits that neither the default
branch nor a remote has. And a repository with no default branch or no commits,
which are the two states with nowhere to be put back *to*. What is deliberately
not a reason to hold is a branch that is pushed with a pull request open: its
work is safe on the remote and the next brief is usually about something else, so
the tree goes back and a job that needs that branch checks it out again, which is
one command against a tree that otherwise sits on a landed branch for a month.

**The reset is at the start of a job and never at the end**, which is the same
argument the next section makes about the footing, for the same reason. A job
killed at the forty-five minute ceiling never runs its cleanup and a job that
died on a spent plan never reached it. Preparation at the start always happens.

**It is a choice, and the reason is ignored files.** A worktree is a fresh
checkout, so it has no `node_modules`, no `target`, no `.venv` and no `.env` —
which is the exact thing `repo.rs`'s own header says the linked-directory design
exists to avoid. Long-lived *per agent* rather than per job is what makes the
trade work: the cost is paid once and the caches survive every later job, and the
preamble says where the operator's checkout is so a job that needs a gitignored
file can go and get it. That is a good trade in most repositories and a bad one
in a few — submodules, LFS, a checkout large enough that a second one is real
disk — and those keep `Bench::Shared`.

**Three things the preamble says that the job cannot work out from inside the
directory.** That the tree is its own and nobody else works in it. That on a
fresh one nothing git ignores is present yet, and where a copy would be. And the
prohibition: git's stash belongs to the repository rather than to the work tree,
so it is shared with the operator's own checkout and with every other agent, and
a pop in any of them takes whatever was pushed last. Nothing about standing in a
worktree hints at that.

**`shell` moves with `code`.** `Runtime::run_in_repository` resolves the same
bench, through `repo::ensure_bench` rather than `prepare`: an agent whose job
works in a worktree and whose `git status` reads the operator's checkout is being
told about a tree it is not working in, and that read is the one it most wants
while a job is going. `ensure_bench` is the half that does not fetch and does not
reset, because a line run inside a turn must not pay for a network round trip and
must not move a branch somebody asked a question about.

**A tree that could not be made refuses the job.** Falling back to the linked
directory would put a harness in the operator's own checkout holding a lock taken
on a path it is not in, so a second agent failing the same way would join it
there. `CodingError::NoWorkTree` names the directory and the two ways out.

**A new repository gets `Own` and an existing one keeps `Shared`.** This is the
one setting whose SQL default and Rust default disagree, and migration 44 says
why: `ALTER TABLE ... DEFAULT` only ever runs against rows that already exist,
and moving somebody's jobs into a directory with none of their dependencies in
it, on launch, with no gesture, is not an upgrade's decision to take.

## A job is told where it is standing before it is told what to do

A harness handed a brief starts editing where it is standing, and where it is
standing is wherever the last job left the tree. Nothing prompts it to look at
the branch first, and nothing else in the app will: a job that opens a pull
request ends on the branch it made, the operator merges it, and every job after
that starts on a feature branch whose work has already landed. Weeks of work can
stack on top of it before anybody notices, and the transcript reads correctly
throughout.

So `repo::footing` reads the tree at the moment a job starts, and
`Runtime::start_job` puts it in front of the brief: the branch, whether it is
clean, whether it tracks anything, whether that branch is already contained in
the default one, and whether a pull request is open for it. Then one rule the
facts resolve to.

The obvious fix is the other end, a standing rule in `APPENDED_PROMPT` about
what a job leaves behind, and it does not hold. A job killed at the ceiling
never runs its cleanup. A job that opened a pull request should still be on its
branch. Cleanup at the end is a step that sometimes does not happen; the footing
at the start always does.

Everything here still applies on a bench, and is read from the bench rather than
from the linked directory. The two are not alternatives: the footing tells the
harness what to do about a tree it found, and the bench is a tree Guaca can also
*act* on. `Footing::resettable` is that same question asked from the other end,
and it is only ever asked of a tree this app owns.

Both halves of the preamble are load-bearing, and this is the part to keep.
Facts alone are not enough: a model handed a branch name and a count decides for
itself what to do with them, and the decision it makes silently is to carry on
where it is standing. A rule alone cannot be written safely, which is the
sharper half. *Start from the default branch* over uncommitted work destroys it,
and this is the operator's own machine rather than a sandbox. The facts are what
let the rule be conditional, and uncommitted work is checked before anything
else and overrides every other case: do not switch, do not stash, do not clean,
work from here or stop and say why.

Four details are not guesses.

- **The merge test runs against `origin/main`, not `main`.** A branch merged
  upstream has landed whether or not the local copy was ever pulled, and a local
  default nobody has updated in a month calls every landed branch work in
  flight. `default_branch` returns a name for the prose and a ref for the test,
  because they are two different things.
- **`origin/HEAD` first, `main` and `master` only after it.** The repository's
  own answer beats a guess, and where nothing published one, the preamble says
  so and asks the harness to decide rather than sending it to a branch that does
  not exist.
- **Every count is against the last fetch, and the preamble says so even when
  they are zero.** Zero is where it misleads rather than where it is safe to
  drop: a branch merged upstream an hour ago and never fetched reads here as
  work in flight, level with its upstream. Fetching on the operator's behalf is
  not this app's call. The harness is the thing standing in the directory with a
  shell.
- **A repository with no commits says so.** Git names an unborn HEAD after the
  branch the first commit will create, so without it the preamble says *on
  branch `main`* and *there is no `main`* two lines apart, and a model handed a
  contradiction resolves it by guessing. A fresh `git init` is an ordinary thing
  to link.

Where the work lands is not decided here. That is already the brief's to say,
and a standing rule that overrode it would be a second answer to a question that
has one. What the footing settles is only where the work starts.

## A job can be reached while it runs, on one of the two

`code` returns the moment the process is up. That is the whole shape of the
feature and it is right: awaited inside the tool call, the agent reads as
`Thinking` for the length of a change to a codebase.

The cost used to be paid at the other end. For up to forty-five minutes a job
was write-only: Guaca read its stdout and could say nothing back. An operator
watching one go the wrong way at minute three had one move, which was to wait
thirty-seven more minutes and start another. There was also no way to end one.
Stopping the conversation that started it does nothing, because that run settled
minutes earlier and the job is not on it; the ceiling was the only thing that
ever stopped a job going wrong.

`coding/bridge.rs` is the second half. Claude Code has an interface besides its
stdout: hooks run at fixed points in its own loop, are handed the event on
stdin, and what they print back is acted on. Guaca writes a settings file and a
three-line `sh` script per job, passes them with `--settings`, and answers the
hook over a loopback socket. Three things follow from that.

- **A mailbox.** `message_coding_job` stages a correction; the `PostToolUse`
  hook delivers it as `additionalContext` at the job's next tool boundary.
- **A gate**, when the repository asks for one. Below.
- **Two ways to report**, on a small MCP server passed with `--mcp-config`:
  `note_progress` and `report_pull_request`.

`pi` has none of it and gets none of it. That is a difference between the
harnesses rather than a gap, and it is why every part of the bridge fails open:
a bridge that did not start, a `curl` that is not installed and a server that
already dropped the job all end with an empty answer and an exit status of zero,
which is the job that worked before any of this existed.

### The three things it rests on are behavior, not flags

None of them can be checked offline, which is why `tests/coding.rs` keeps an
`#[ignore]`d half that asks the real program. Measured against 2.1.247:

- A `PreToolUse` hook answering `permissionDecision: "deny"` **overrides
  `--permission-mode bypassPermissions`**, which is the mode every job here runs
  in. Without that the gate would be a suggestion.
- A `Stop` hook's `reason` reaches the model, as a synthetic user message
  reading `Stop hook feedback: …`. That is what lets the `Stop` hook block *and*
  deliver in one call, which is what makes it terminate: a block that did not
  deliver would refuse to stop forever and be killed at the ceiling.
- `additionalContext` from `PostToolUse` is put in front of the model before its
  next round.

### The mailbox is delivered once, from either end

`take_mail` reads and clears together. Delivered twice is an instruction the
model was given twice, and a job handed the same correction on four tool calls
does the thing four times. The `Stop` hook is the other end and exists for one
case: a correction typed at minute forty-four, which without it lands after the
job has decided it is done.

### Stopping leaves what it committed

The process is killed where it stands. Nothing is reverted, because the commits
a job is told to make as it goes are the operator's checkpoints and throwing
them away is not that button's decision. The agent that started the job is told,
on the same path a finished one takes and for the same reason: an agent never
told waits forever, and answers "I started that and have not heard back", which
is true and useless. What it is told is that the work is *partly* done and
nobody has checked which part, because an agent told only that the job stopped
reports the work as not done and leaves the operator to discover half of it on a
branch.

### The session id is chosen, not read back

`--session-id` takes a UUID, so Guaca picks one before the job starts. One value
is then the job's address on the bridge, the key of its mailbox, and what an
operator hands to `claude --resume` to open the same work in their own terminal.
That last one is the reason: `claude -c` resumes whatever ran last in the
directory, which after two jobs is the wrong one. Chosen rather than parsed also
means a job killed at the ceiling, and a job that died before its first event,
both still have one.

### A program too old for it still runs the job

`coding::presence` reads `--version` and decides whether to wire a bridge at
all. Below the floor the job runs exactly as every job ran before the bridge
existed. The other direction was never on the table: refusing to run would take
away a harness that works, to protect a feature that is an addition to it. An
unreadable version string is treated as too old for the same reason, since the
alternative is wiring a job to a contract nothing has ever checked.

## The gate is a decision the operator takes per repository

`repositories.gate`, added by migration 42, default `'open'`, which is what
every job did before this existed.

Off by default is not caution about the migration. `APPENDED_PROMPT` tells every
job that it is running unattended and that nobody will answer a question.
Switching this on for everybody would make that sentence false in every
repository at once, and a job that believes it while a hook silently holds it is
a job that reports a push it never made. An operator turning it on is an
operator saying they will be there.

What it gates is the handful of things that leave the work tree under the
operator's own name and cannot be taken back by git: a push, a pull request, a
merge, a release. `bridge::outward` reads the shell line, and it errs toward
asking, because a wrong yes costs one card on the desk and a wrong no is the
behavior the operator switched it on to stop.

### The line is read, and then what the line runs is read

Reading the words alone is what one level of indirection walks straight past.
`./scripts/ship.sh` is not `git push` and never will be, so a repository whose
release lives in a script had a gate that was switched on, said it was holding,
and stopped nothing. That is worse than having no gate: the operator turned it
on and was told it was working.

So a line that names nothing outward-facing is looked at again, for the scripts
*in this tree* that it runs. A `package.json` entry behind `npm run release`,
`pnpm release` or `yarn release`; a file in the work tree behind
`./scripts/ship.sh` or `bash scripts/ship.sh`. Each one is read and asked the
same question, three deep, with a set carried across the whole walk so that two
scripts that run each other are read once rather than forever.

The card then has to carry both halves, because they are not the same sentence.
`./scripts/ship.sh` is what the agent asked for and says nothing about what it
does; `git push` is what the operator is being asked to allow. `Reach` is that
pair, and the summary leads with the push: *wants to run `git push` in guaca, by
way of `./scripts/ship.sh`*. That order is the emphasis, not a preference. What
is being authorized is the thing that cannot be taken back, and the script is
how the agent got there.

The `Command` detail field holds the line the model actually wrote, flags and
all, which is what it should always have held: it carried the summary instead,
so an operator was shown `git push` about a `git push --force`, which is the one
detail on a card that changes the answer.

### And what it deliberately does not read

A Makefile target. A compiled program. Anything it cannot parse as text. Those
are indirection too, and reading none of them is a decision rather than a gap.

The alternative is to treat *there is something here I cannot see through* as a
reason to ask, which sounds like the safe direction and is not. The ordinary
case for it is `./target/release/app` and `./node_modules/.bin/vite`: every
locally built program in the tree, on every line that runs one. A gate that
parks a turn for running the binary it just compiled is exactly the wrong yes
that teaches an operator to switch it off, and a gate that is off holds nothing
at all. The rule stays what the paragraph below says it is: the ordinary case,
followed honestly, and not a boundary.

### One no settles the question for the rest of the run

A model that has just been refused a push tries the push. That is ordinary
rather than confused — the refusal it reads back says the operator did not allow
it, not that they never will — and the operator is the one who pays for it, in a
second card and a third, for a question they are sitting there answering.

`Runs::refused` holds what each run has already been told no about, and
`ask_about_push` reads it before it parks anything. Three things about the shape
of it are load-bearing:

- **A no is remembered and nothing else is.** An expiry is the operator being
  somewhere else rather than the operator answering. Held against them, a
  request nobody saw would refuse the one they would have seen two minutes
  later.
- **It is keyed by the outward action the card named, not by the line.**
  Somebody who refused `git push` refused it whether the next attempt spells it
  with a different flag or reaches it through a script. A key that told those
  apart would remember nothing a retry could not walk around.
- **It is per run, and that is the whole of how it is forgotten.** The
  operator's next message is a new run, so a no holds for the work it was said
  about and for nothing after it. Nothing is stored, for the reason a job's lock
  is not: a refusal that outlived the process would be a repository quietly
  refusing pushes with no decision anybody could find behind it.

An ordinary run's entry goes when the run settles, on the same line `stopped`
goes. A job's run never settles, because nothing is ever booked against it, so
it is dropped where the job ends, beside `release_parked`.

It parks as `ProtectedAction::ActOnBehalf` rather than as a new variant, because
that is what that one already means. It also means the standing grant means what
it says: an operator who has told this agent it may act on their behalf is not
asked again per push, and a grant given after a refusal outranks it. That is the
one place the run's memory is consulted alongside the store, and it is the right
way round: the grant is the broader answer and the newer one.

`Runtime::park_with` is `park` with one difference, and it is one thing: whether
the agent's own activity is this request's to move. A parked turn is an agent
genuinely stopped mid-inference and the dot beside its name has to say so. A
coding job is not a turn: it outlived the one that started it by many minutes,
and the agent that owns it may be idle or answering somebody else. Marking it
`AwaitingApproval` would put a false state on a working agent. Everything else,
the row, the waker, the window and the expiry, is shared, because a second copy
of those is a second place for a request to be left waiting on nobody.

The job's requests are filed against a run of its own, minted in `start_job`.
That is what makes `release_parked` the way a job ending closes whatever it was
waiting on, rather than a second sweep written for this.

**It is not a boundary and must never be described as one.** A shell line is not
something anything can parse without a shell, and a job that wanted to get
around this could. The process runs as the operator, with their credentials and
their network, which was true before the gate existed. What it buys is that the
ordinary push, made by a job doing what it was asked, is one somebody sees
first.

## A job inherits the operator's own Claude Code setup, and that is deliberate

`coding/claude_code.rs` passes no `--strict-mcp-config` and no
`--setting-sources`, which is the exact opposite of `llm/claude.rs`, and the two
are right for opposite reasons. A turn there is answered by a program that
should have this app's tools and nothing else. A job here is a coding agent
working in the operator's own repository, where their `CLAUDE.md`, their project
settings and their MCP servers are the thing that makes it good at the work.
`--settings` and `--mcp-config` are both additive, so the bridge adds to that
setup rather than replacing it.

The cost is real and worth writing down. Measured on one machine on 2026-08-27
against 2.1.247, a job started this way loaded 16 MCP servers, 229 tools, 100
slash commands and 8 agent definitions, none of which Guaca chose. That is the
operator's own configuration doing what they set it up to do, in their own
repository, and it is not a defect. It does mean a job can reach whatever they
have connected.

**The one hazard is a blocking `Stop` hook.** The operator's own hooks run in a
job here, and a `Stop` hook of theirs that answers `{"decision": "block"}` will
hold a Guaca job against its own completion in a loop nothing in `coding/mod.rs`
can see, until the forty-five minute ceiling. Guaca cannot fix that from
outside, and it is not hypothetical: it is the mechanism Warp's own
`oz-harness-support` plugin uses, and any harness-integration plugin an operator
has installed may do the same. The ceiling is the backstop.

## A failed turn is not a job with nothing to do

Both programs report a failed turn *inside* their stream and can still exit zero
about it, with no content and no answer. Read by exit code and text alone that is
indistinguishable from a job that found nothing to change.

It cost an afternoon. An expired Codex token turned every coding job in a live
workspace into a silent no-op, every agent dutifully reported that nothing needed
doing, and `pi auth check` called the provider ready throughout. So `Outcome`
carries `failed`, it is taken from the *last* message rather than the first so a
turn that failed and was retried is not a failed job, and `job_finished` reports
it before the empty case.

It also reaches the operator and not only the agent. A spent credential on the
operator's own machine is the operator's problem, and a sentence about it inside
one agent's transcript is a sentence nobody reads. `CodingJobFailed` raises a
banner that names the repository, the program that stopped, and the harness's own
words: with two harnesses, the way out of the commonest failure here is the other
one, and a banner that does not say which was running leaves the operator
guessing which sign-in to go and look at.

## What is not here

Any confinement. The process runs as the operator, in their repository, with
their credentials and their network, and it may commit, push and open pull
requests. That was asked for explicitly. The boundary is the directory the
operator chose and the fact that git can undo what happens inside it. Nothing in
`coding/` should ever be described as a sandbox.

Nor is the spend. Both harnesses read their own auth, and a job's cost does not
appear in this app's usage table because this app did not spend it. What a job
reports back is what the harness says it cost, which on a subscription is the
equivalent API price rather than money that moved. Both programs price it that
way, so the two numbers agree and neither claims to be more than what was said.

## Testing it

`tests/coding.rs`. The offline half puts real stand-in executables on `PATH`,
because the thing being tested is a process: a fake in front of the fold would be
a test of the fold, which already has one beside it in each submodule. Each
stand-in records the argument vector it was handed, so the suite can assert that
a repository set to Claude Code starts `claude` with Claude Code's vector, which
is the seam nothing else can see. Drop the column read in `Runtime::start_job`
and every other suite in this repo still passes.

The four `shell` tests in the same file need no stand-in, because the program
they run is `bash` and the repository is a real one `git init` made in the temp
directory. They are the only place the two doors are checked against each other:
that a line runs in the linked directory and answers inside the turn, that the
gate stops the same commands through both and that a denial runs no part of the
line, that an ordinary line is not stopped, and that a work tree with a job in
it still answers one. `shell.rs` has the rest beside it, against a real shell
for the reason the stand-ins exist: what a child prints, whether it is still
running and what happens when it fills a pipe are only observable from outside
it.

The `#[ignore]`d half asks the real programs whether they still accept those
vectors and still answer in the shape this build reads. That is the failure no
offline test can see, and it is the same live half `subscription.rs`,
`plugins.rs` and `account.rs` each keep for the same reason. It spends the
operator's own plan.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test coding
cargo test --manifest-path src-tauri/Cargo.toml --test coding -- --ignored
```

## Codex runs through its official CLI

`Harness::Codex` starts `codex app-server --listen stdio://` in the same
prepared worktree as the other harnesses. Existing repositories keep their
harness, assignments and gate. Codex owns model selection and authentication.
On the backend, as the daemon's user, run `codex login --device-auth`. This
sign-in is separate from Guaca's ChatGPT provider sign-in. The image pins the
CLI version; its configuration, credentials and sessions live in the persistent
home volume. The control contract is measured against Codex 0.153.3; the UI
requires 0.153 or newer for steering and approvals.

The runner initializes one thread, starts one turn, and records its id, tool
activity, answer and failures. It does not turn token counts into a dollar
cost. A stream ending before `turn/completed` is an interrupted job, even if
it printed narration. Both output pipes are drained concurrently under the
job's deadline. Completion kills and reaps the listening app-server before
the runtime releases the worktree.

A correction uses `turn/steer` with the active turn id. Guaca reports it sent
only after Codex acknowledges that id. A completed turn rejects a late
correction visibly; Guaca never silently starts another turn. Steering guides
the model's next decision and does not undo a command already executed. Stop
remains available while acknowledgment is pending. An unanswered control
request stops the job after thirty seconds; partial changes may remain.

**Ask me before pushing** selects Codex's `untrusted` approval policy and user
reviewer, verified in the thread response before any work starts. Command
approval callbacks use the same script inspection and operator decision as
Claude's hooks and the `shell` tool. Each decision applies to that request;
Guaca never grants a session exemption. The protocol keeps reading corrections
while an approval waits. As with the existing gate, this covers ordinary shell
commands, not confinement or arbitrary API calls. The CLI retains its own
[configured command rules](https://developers.openai.com/codex/rules), so an operator's explicit allow rule can bypass its
approval callback. Keep outward commands out of those allow rules when relying
on this gate.

The official Claude Code CLI can also run on an operator's own backend. Its
local `claude auth login` and Codex's `codex login status` are CLI operations;
Guaca offers setup instructions and reads only sign-in status, never credential
files. No model call is needed to check either CLI. This does not introduce a
Guaca Claude.ai login flow or authorize a managed service to route consumer
subscriptions. See the provider's current terms before offering such a service.

## A GitHub App authenticates both repository doors

[GitHub App access](GITHUB.md) supplies repository-scoped, renewable credentials
through Git's helper and a `gh` wrapper. Both coding harnesses and `shell` get
the wrapper for a connected repository; personal-token and SSH repositories
retain their existing configuration. The push gate is unchanged: authentication
answers whether GitHub permits an operation, and the gate answers whether the
operator allowed this job to perform it.


## Commit attribution is independent of access

The repository's Git identity supplies the human author and committer defaults
for all harnesses. The remote clone form and Git access panel let the operator
set that name and email. GitHub App authentication does not replace it with a
bot identity. Missing backend identity requires operator configuration before
committing; Guaca never guesses the installation owner's identity. See
[GitHub access](GITHUB.md#commit-authorship) for contribution attribution and
the separate identity GitHub shows on App-created pull requests.
