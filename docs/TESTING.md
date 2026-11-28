# Testing

Which suite answers which question, and what each one costs to run. The three
that run against every change are in *Verify* below, with the commands
for them. This file is the rest: one per subsystem with a wire, a process or a
provider behind it, most of them holding an `#[ignore]`d half that asks the live
thing whether what this build believes about it is still true.

Live model tests accept `OPENROUTER_API_KEY` from the shell when the configured
endpoint is `https://openrouter.ai`. It overrides the saved key in the test
process only; the app's settings and other providers are unchanged.
`GUACA_TEST_MODEL` likewise selects a model for tests on a compatible endpoint,
so a retired model can be replaced for a run without editing the app's settings.

None of the three can be pointed at a whole team, because a real model given a
real instruction does something slightly different every time and one run of it
proves nothing either way. `tests/crew.rs` is that question: eight roles, one
directive to the Chief of Staff, run as many times as you ask for, and the
answer is a recording rather than an assertion. Every run writes its events, its
messages, a readable transcript and its numbers to `runs/<timestamp>/`, and the
comparison beside them says what was different between runs that were given
identical instructions. It asserts only what is not a matter of taste: every run
settled, no run left the machinery in a state `trajectory.rs` calls broken, and
somebody answered the operator. Run it after anything that changes how a group
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
./scripts/subscription.sh                                    # a real call against your own ChatGPT plan
GUAC_SUBSCRIPTION_MODEL=gpt-6-sol ./scripts/subscription.sh  # the same, on a model the catalog just added
```

`tests/subscription_catalog.rs` checks the authenticated model list, including
hidden entries, priority, malformed responses, one refresh after a 401, and that
no model is left out for needing a newer Codex CLI than the version Guaca names.
Its live check reads the catalog without spending model quota:

```sh
GUAC_SUBSCRIPTION_JSON=/path/to/subscription.json cargo test \
  --manifest-path src-tauri/Cargo.toml --test subscription_catalog -- --ignored --nocapture
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

`tests/skills_sh.rs` is the same shape for skills.sh: a scripted directory that
answers with the cases that have to be handled rather than drawn (a ranking in
a shape this build does not know, a skill whose files climb out of it, a skill
that changes between being read and being added), and the real client writing
to a temporary disk. Its `#[ignore]`d half asks the live skills.sh, whose
endpoints this build reads are not its documented API, and adds the top
addable skill to a temporary directory. It writes nothing anywhere else.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test skills_sh -- --ignored
```

`tests/server.rs` drives the daemon, which is where every workspace runs: a workspace opened under a temporary directory, bound to a free port,
and spoken to over HTTP and the event socket with `reqwest`, entirely offline.
It asks the questions only that host can fail: that the commands the window
calls answer over a POST, that a server refuses what it cannot do and
says what to do instead, that nothing but `/health` answers
without the token, that an event reaches a client that is not a webview, and
that a stored file is reachable by its digest. It is compiled only under the
daemon step in `ci.sh`, because that is the only build without Tauri in it, and
a break in it does not show anywhere else.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features server --test server
```

`tests/unreachable.rs` is the same host asked what it promises an operator
whose account service is not there. Every `guaca.bot` route is pointed at a
port that refuses connections — `GUACA_ACCOUNT_ORIGIN`, the seam `boot.rs`
already reads — and the suite then creates a group and an agent, takes a turn
against the scripted model, fires a routine, reads the settings pane, and
asserts that a connector whose grant lives at that service refuses with a
sentence naming what to fix. Every call has a deadline, so a boot or a turn
that waits on the absent service fails as this suite rather than as a harness
timeout. A third case plants an account file whose token has expired, which is
the operator who signed in once and whose service has since gone away. It is
the guarantee `docs/HOSTING.md#what-self-hosting-guarantees` states, held as a
test; `ci.sh` names it on its own line in the daemon phase.

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

`tests/errands.rs` drives errands against the same scripted server the cascade
suite uses, which answers an errand as "on an errand for" its agent because that
is how its prompt opens. It holds what is not wording: an errand is offered only
where the operator switched it on, a fourth is refused, the three withheld tools
are refused under any name, the last two steps stay with the sending turn,
and a stop reaches an errand mid-call. Remove the kept steps and the matching
test fails.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test errands
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
  commands::catalog::tests::every_use_case -- --ignored --nocapture
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

`tests/recall.rs` is the score for recall: each case from the memory
investigation played as a conversation on the scripted model, ending in a
question, and passing when the fact the question needs is in what the model was
sent (the case in another group passes when it is not). Every case is played
twice, once with the recall cards deleted before the question, which is what a
turn saw before recall existed, and the table it prints shows both. The cases
with different words for the same thing are reported and never fail the gate,
because their count is what decides whether search by meaning is worth adding.
Its `#[ignore]`d half plays the same cases against a real model and checks the
answer text, with the history written straight to the store so that only the
questions are paid for; it stops at a dollar.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test recall -- --nocapture
GUACA_TEST_MODEL=openai/gpt-4.1-nano cargo test --manifest-path src-tauri/Cargo.toml \
  --test recall live_the_score -- --ignored --nocapture
```

## Verify

```sh
./scripts/ci.sh          # lint, typecheck, build, every test suite, the icons, the scenarios
./scripts/ci.sh rust     # Rust only: desktop, then daemon
./scripts/ci.sh e2e      # scripts/verify.py --build, then scripts/hosting-browser.mjs
./scripts/image.sh       # the daemon's image: build it, run it, prove it answers
scripts/probe.py         # on a Mac: what only a Mac does, see docs/PROBE.md
GUAC_LOG=guac=debug pnpm app
```

Getting a change on screen has its own file: `.claude/skills/run-guaca`. It
holds the harness that draws a component in seconds without a Rust build, a key
or any spend; what to do when the app refuses to start; and the rule about the
operator's own workspace, which every workspace on the machine shares.

**User-visible behavior ships with a scenario.** A change a person can see or
do in the window, or one that changes what a Host does with their work (a
reply, a request, For you, a restart, a routine), is not done until a scenario
in `scripts/e2e/scenarios.py` drives it from an empty workspace the way a
person does, and asserts its failure path too. `scripts/verify.py` runs the
real daemon and web bundle with the model scripted on loopback, so a scenario
costs nothing, needs no key and reaches no network, and its result depends only
on the commit. Break the behavior on purpose and watch the scenario fail before
trusting it, and run it `--runs 20` before adding it. `.claude/skills/verify`
is how to drive, read the evidence and add one; `scripts/verify.py --list` is
the journeys covered and the claims each must pass. What only the native shell does (the window,
banners, the menu bar) is `scripts/verify.py mac`, which is the probe below.

Three suites ask three different questions, and a change can pass one while
failing the next. *Three test suites, asking different questions* in
`docs/ARCHITECTURE.md` is the long version.

- **Cascade tests**, inside the Rust suite, ask whether the runtime did as it
  was told. They drive the real runtime against a scripted OpenAI-compatible
  server. If you change messaging, they are the ones that will catch you.
- **Evals** ask whether the resulting traffic is something an operator would
  want to watch. Every cascade defect this app has had passed the first suite
  and failed this one. CI cannot see a prompt that makes agents chattier, so if
  you change a prompt, run the live half: `./scripts/evals.sh`, which costs
  money.
- **Trajectory** asks whether the machinery behaved: every placeholder closed,
  every parked turn released, every model call on the run's bill, nothing filed
  against a run already reported finished. If you touch streaming, settle
  detection, retries or the budget, run it.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test trajectory
```

The other suites, one per subsystem with something live behind it, are in
this file: the group recording, the subscription, the account, the
plugins, the two machine providers, and the two reads off OpenRouter's catalog.
Run the one you touched.

## Continuous integration

`.github/workflows/ci.yml` runs the gate on every pull request and every push
to `main`. It runs `./scripts/ci.sh` itself, one part per job, so a job that
fails here fails on a laptop with the same command, and the jobs together run
exactly what `./scripts/ci.sh` with no argument runs.

| Job | Runs | Needs |
|---|---|---|
| grants (comments, suppressions, prose files) | `ci.sh grants`: the grant lint's tests, then the lint over the change; the only job that fetches full history, which it needs to find the merge base and the ledger's commit | Node 22, pnpm and Python 3 |
| web, scripts, assets, connector | `ci.sh assets`, `scripts`, `web`, `connector`, one step each | Node 22 and pnpm |
| rust desktop | `ci.sh desktop`: format, Clippy and every test with the desktop features | the Tauri headers, Rust, a built `dist/` |
| rust daemon (no Tauri, no frontend) | `ci.sh daemon`: Clippy and every test built as the daemon | Rust only, so a dependency on Tauri or `dist/` fails here |
| formal (TLA+ model check) | `ci.sh formal`: the conversation recovery model | Java, and the TLA+ tools jar, cached by its pinned version |
| images (Dockerfile.ci and the daemon image) | builds `Dockerfile.ci`, then `./scripts/image.sh` | Docker |
| end-to-end scenarios (guacad, headless Chromium, stand-in model) | `ci.sh e2e`: builds `dist/` and `guacad`, then drives every scenario in `scripts/e2e/scenarios.py` once and runs `scripts/hosting-browser.mjs` | Rust, Node 22 and pnpm, the runner's Chrome |
| gate | passes only when every job above passed | nothing |

Each job starts on a fresh GitHub-hosted Ubuntu 24.04 machine, never on one of
the repository's own: a pull request from a fork runs its own code in these
jobs. The workflow can read the repository and nothing more, holds no secret,
and uses `pull_request`, so a fork's run cannot write anywhere. Actions are
pinned by commit. The cargo registry and target and the pnpm store are cached
between runs; a push to a pull request cancels that pull request's older run,
and every commit on `main` keeps its own. Debug info is off in CI
(`CARGO_PROFILE_DEV_DEBUG=0`), which changes nothing a test asserts and keeps
the target small enough to cache.

**Reading a failure.** Open the red job, then the red step: its name is the
`ci.sh` part, and each `==>` line inside it is one step of that part. A
failing Rust test prints `test <path> ... FAILED` and its panic under
`failures:`, and Cargo stops at the first test binary that fails, so the
binaries after it have not run; a failing Vitest test prints `FAIL` and the
file. Rerun the same
part locally, `./scripts/ci.sh desktop` for example, before anything else. A
job that is red only in CI is a finding about the test, not a reason to retry
until it passes.

**Adding a job.** Add one block under `jobs:` with its own `runs-on:
ubuntu-24.04` and `timeout-minutes`, and add its id to `gate`'s `needs`. A
part of the gate belongs in `ci.sh` first, as a new argument that `all` runs
too, so a laptop and CI keep running the same thing:
`scripts/check-ci-parts.py`, in `ci.sh scripts`, fails while a part `ci.sh`
runs has no job here. Never use a macOS runner here. Require `gate` and only
`gate` in branch protection, so a new job never needs a settings change.

## What looks like a simplification and is not

- **A stub that branches on what was said must not read the system prompt.**
  `anyone_said` skips it. Every scripted eval keyed on a word is really asking
  "does this appear anywhere in the request", and the request opens with two
  thousand words of instructions: adding the working-notes section, which says
  "when something you noted stops being true", made every stub keyed on `noted`
  fire on the first call and read as a group that would not stop repeating itself.
