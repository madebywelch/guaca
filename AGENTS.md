# Guaca

Local desktop app. You talk to LLM agents; the agents talk to each other. Tauri
v2, React + TypeScript front, Rust back.

## Non-obvious things worth knowing before you change something

**The agent runtime is Rust, not the webview.** Each agent is a `tokio` task
with an `mpsc` inbox. If you find yourself adding agent logic to `src/`, you are
in the wrong half of the repo. The frontend renders state and forwards intent.

**Read `runtime/guard.rs` before touching anything about messaging.** Agents
messaging each other does not terminate on its own. Five independent limits stop
it, and each catches a different shape of runaway. Weakening one is not a local
change.

**`expects_reply` is what makes cascades converge**, not the guard. The guard is
the backstop. See `docs/ARCHITECTURE.md`.

**Whether a send is "answering" is a question about the run, not the batch.**
Replies land milliseconds apart and an actor drains whatever is in its inbox, so
a batch is a timing artifact: three peers answering at once can be split across
turns, and deciding from the batch made two of them look like strangers. Ask the
guard, which counts sends per pair for the whole run.

**Migrations are forward-only and numbered.** One has already run against a real
database by the time you think of an improvement, and editing it leaves that
database at the same `user_version` with a different schema. Add another.

**Budget counts model calls, not agent turns.** One turn can make several calls
working through tool results. Counting turns lets a bounded run bill many times
over. There is a test named after this.

**Anything crossing IPC is camelCase.** `rename_all` on a tagged enum renames
variants, not fields; you also need `rename_all_fields`. `ipc.contract.test.ts`
compares the Rust and TypeScript command lists directly, so a rename that only
lands on one side fails the build rather than at runtime.

**`Store::open` has two SQLite lessons encoded in comments.** Do not reorder the
pragmas or simplify the migration transaction without reading them.

**There are two Chrome profiles on every machine, and only one of them counts.**
Chrome ignores `--remote-debugging-port` when it re-attaches to an existing
profile, so `browse` drives a profile of its own under `~/.guac/chrome`, while
`open_on_desktop google-chrome` opens the default one. Sign-in detection reads
the profile `browse` drives, so a session established in the other window is
invisible to every agent and nothing reports an error.

**Sign-ins are detected, never declared.** The browser is holding the cookies,
so `domain/signin.rs` asks it rather than asking the operator to keep a list.
The whole set for an agent is replaced on every scan: a row that outlives the
logout it should have noticed keeps the crew routing work to a machine that will
hit a login wall.

**A cookie's presence is not a login, and this is the trap.** A profile that has
browsed for an hour holds a thousand cookies across three hundred domains, most
of them durable and `httpOnly`. `google.com` sets `NID` on a browser that has
never seen an account, and `PHPSESSID` is handed to every anonymous visitor.
Both were real false positives from a live machine. Detection is therefore a
signature table plus a rule that needs the browser to have *visited* the site
and to hold a cookie implying an identity rather than a session. The tests carry
the real cookie names; do not loosen them without a fresh capture.

**A cookie value must never leave the sandbox.** `browser.py` drops it at the
only point in the system that sees one, and `CookieMark` has no field it could
arrive in.

**A credential's secret must never reach the model.** It goes from SQLite into
the `envs` of one sandbox command and stops there. Not into a prompt, not into
the transcript, not over IPC, and deliberately not into a dotfile on the sandbox
either, because that disk survives the sleep this app relies on.

**A session belongs to one agent; a credential belongs to the group.** That is
physical, not a policy: cookies are on one disk and a token is a string.

## Where things are

```
src/                 React + TypeScript. A view over the runtime, nothing more.
src-tauri/src/
  domain/            AgentCard, Envelope, Routine, Connector, Signin, ids. No I/O.
  runtime/
    guard.rs         The loop guard. Read this one first.
    mod.rs           Agent actors and the message bus.
    prompt.rs        Prompt assembly, including the trust boundary.
    events.rs        Events pushed to the UI.
  llm/               OpenAI-compatible client, SSE decoding, tool definitions.
  db/                SQLite. Plain SQL, numbered migrations.
  e2b.rs             Sandboxes: the machines agents work on.
  proxy.rs           Loopback viewer for those machines.
  eval.rs            Reads a run and says whether it communicated sensibly.
  commands.rs        The entire IPC surface.
  app.rs             The only file that knows Tauri exists.
```

The agent runtime lives in Rust, not the webview. Each agent is a `tokio` task
with its own inbox, so sending is enqueue-and-return and N agents genuinely run
concurrently. It also means your API key never crosses into the webview.

`docs/ARCHITECTURE.md` covers the design decisions. `docs/PROTOCOL.md` records
what the agent-interoperability literature contributed and what had to be
invented.

## Conventions

- Match the surrounding code. Comments explain why, never what.
- Every guard refusal and every error the operator can hit says what happened
  and what to do about it. Both are read by a model or a human under pressure.
- New behaviour needs a test that would fail without it. Failure paths first.
- No dead code, no speculative API surface. The contract test fails on a command
  nothing calls.
- Errors an agent reads mid-turn need a way forward, not just a reason. A
  refusal that only says no gets reworded and retried.

## Verify

```sh
./scripts/ci.sh          # lint, typecheck, build, both test suites
./scripts/ci.sh rust     # Rust only
GUAC_LOG=guac=debug pnpm app
```

The Rust suite includes cascade tests that drive the real runtime against a
scripted OpenAI-compatible server. If you change messaging, they are the ones
that will catch you.

The evals are a second suite asking a different question: not "did the runtime
do as it was told" but "is the resulting traffic something an operator would
want to watch". Every cascade defect this app has had passed the first suite and
failed the second. If you change a prompt, run the live half. CI cannot see a
prompt that makes agents chattier.

```sh
./scripts/evals.sh       # live, against the configured model, costs money
```
