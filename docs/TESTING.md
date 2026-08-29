# Testing

Which suite answers which question, and what each one costs to run. The three
that run against every change are in *Verify* in `AGENTS.md`, with the commands
for them. This file is the rest: one per subsystem with a wire, a process or a
provider behind it, most of them holding an `#[ignore]`d half that asks the live
thing whether what this build believes about it is still true.

None of the three can be pointed at a whole team, because a real model given a
real instruction does something slightly different every time and one run of it
proves nothing either way. `tests/crew.rs` is that question: eight roles, one
directive to the Chief of Staff, run as many times as you ask for, and the
answer is a recording rather than an assertion. Every run writes its events, its
messages, a readable transcript and its numbers to `runs/<timestamp>/`, and the
comparison beside them says what was different between runs that were given
identical instructions. It asserts only what is not a matter of taste: every run
settled, no run left the machinery in a state `trajectory.rs` calls broken, and
somebody answered the operator. Run it after anything that changes how a crew
divides work, and read the transcripts rather than the exit code.

```sh
./scripts/crew.sh                 # one run, a few cents
GUACA_RUNS=5 ./scripts/crew.sh    # five, to see what varies
```

A narrower one exists for the subscription. `tests/subscription.rs` runs
the real runtime against a scripted *Responses* server, which is a protocol the
other three never touch, and it holds one `#[ignore]`d live test. Run that after
changing `llm/codex.rs`, or when a sign-in that worked stops working: everything
offline is a stub agreeing with what this app believes the protocol is, and the
failure worth catching is that belief going stale.

```sh
./scripts/subscription.sh    # a real call against your own ChatGPT plan
```

`tests/account.rs` is the same shape again for the guaca.bot sign-in: a scripted
authorization server, and the real `Account` driven through discovery, the
loopback listener, the PKCE exchange and the first call the token is spent on.
Its stub checks that the verifier presented at the token endpoint actually
hashes to the challenge that was sent, because a sign-in that stops proving that
still works. Its `#[ignore]`d half asks whether the live service still publishes
what this build reads, and `GUACA_ACCOUNT_ORIGIN` points it at a Worker on this
machine instead. It authorizes nothing and stores nothing.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test account -- --ignored
```

Another, `tests/plugins.rs`, does the same job for MCP: a scripted server that
publishes the four metadata documents an OAuth sign-in needs, and one runtime
turn that calls a plugin tool end to end. Its `deployments` module is a second
scripted server and deliberately not the same one: that one is the five vendors'
shape, and this one is a box in somebody's own network — the older transport, a
gate wanting headers, a key taped to it. Its live half runs `oauth::discover`
against every vendor on the list and asks whether each still publishes what this
build expects, which is the failure no offline test can see. It reaches the internet, authorizes nothing and spends nothing.

```sh
./scripts/plugins.sh
```

`tests/coding.rs` does the same job for the coding harnesses, and its offline
half puts real stand-in executables on `PATH` rather than mocking: the thing
being tested is a process, and each stand-in records the argument vector it was
handed. That is what makes "a repository set to Claude Code starts `claude`" an
assertion rather than a code read; drop the column read in `Runtime::start_job`
and every other suite in this repo still passes. Its `#[ignore]`d half asks the
real programs whether they still accept those vectors and still answer in the
shape this build reads, and it spends the operator's own plan.

The stand-ins are also what makes *where* a job ran an assertion. Each one
records into whatever directory it was started in, so a repository that gives
each agent a work tree of its own is checked by there being no recording in the
operator's own checkout and one in the worktree, and two agents in one codebase
are checked by there being two. Nothing else in the build can see that: the
directory a harness is handed is one argument, and a wrong one is a job that
works perfectly in the wrong tree.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test coding
cargo test --manifest-path src-tauri/Cargo.toml --test coding -- --ignored
```

And `tests/machines.rs` is the same shape for the two providers: scripted
control planes for Kernel and E2B, and the real `Runtime` provisioning against
them. It is entirely offline and costs nothing. Nothing else in the build
reaches a provider, so without it every suite passes with a turn renting a
machine on every tool call. Run it after touching `ensure_computer`,
`ensure_browser` or either client.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test machines
```

The model suggestions beside an agent's model field have the same shape again,
without a script because it is one test. It asks the live OpenRouter whether it
still ranks models for all twelve of the use cases this build believes in, which
is the one failure the offline suite cannot see: a category renamed there answers
200 with an empty list. It reaches the internet, authorizes nothing and spends
nothing.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib \
  llm::catalog::tests::every_use_case -- --ignored --nocapture
```

Whether a model can be shown a picture is read off the same vendor's catalog and
has the same blind spot, so it has the same test: `architecture.input_modalities`
renamed or dropped on OpenRouter's side turns every model into one nothing was
published about, which looks exactly like the day before any of this existed and
fails nothing offline. Also free.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib \
  llm::modality::tests::openrouter_still -- --ignored --nocapture
```

## What looks like a simplification and is not

- **A stub that branches on what was said must not read the system prompt.**
  `anyone_said` skips it. Every scripted eval keyed on a word is really asking
  "does this appear anywhere in the request", and the request opens with two
  thousand words of instructions: adding the working-notes section, which says
  "when something you noted stops being true", made every stub keyed on `noted`
  fire on the first call and read as a crew that would not stop repeating itself.
