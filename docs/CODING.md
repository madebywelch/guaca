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

The `#[ignore]`d half asks the real programs whether they still accept those
vectors and still answer in the shape this build reads. That is the failure no
offline test can see, and it is the same live half `subscription.rs`,
`plugins.rs` and `account.rs` each keep for the same reason. It spends the
operator's own plan.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test coding
cargo test --manifest-path src-tauri/Cargo.toml --test coding -- --ignored
```
