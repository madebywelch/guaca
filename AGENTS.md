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

**A courtesy to a peer that has already answered is refused; work is not.**
`send_message` carries an `intent`, and the sender declares it, because a second
instruction and a thank-you are the same shape on the wire and guessing from the
shape refused real work. Anything not declared `work`, including a value nobody
defined, is a courtesy: the permissive half of the parser must not be the half
that opens the door. `runtime/prompt.rs` says the same thing to the model in the
mode where it matters, and the two have to agree.

**`expects_reply` and `intent` answer different questions, and conflating them
stopped an agent mid-task.** The first is whether anybody is waiting on your
words, which is what terminates a cascade. The second is whether you were given
something to do. An instruction to a peer that has already answered carries work
and expects no reply, and reading the first as the second put that turn in the
mode that says nothing is being asked of it and silence is usually right. A real
send to the operator's own address died there: the agent spent a call, said
nothing, and looked like it had stopped. `ReplyMode::Assigned` is that
combination, and its output is still a note.

**Whether a send is "answering" is a question about the run, not the batch.**
Replies land milliseconds apart and an actor drains whatever is in its inbox, so
a batch is a timing artifact: three peers answering at once can be split across
turns, and deciding from the batch made two of them look like strangers. Ask the
guard, which counts sends per pair for the whole run.

**A protected action parks the turn that asked for it, and the row is the
verdict.** `create_agent` stops mid-turn and waits on a person. The operator's
click and the turn's own timeout can land in the same instant, so the answer is
read back from the `approvals` row rather than from the channel the wakeup
arrived on: `settle_approval` only moves a row out of `pending`, and whichever
of the two loses that race changes nothing. Anything still pending at startup is
expired, because nothing holds a parked turn across a restart. "Always allow" is
the decision row itself, scoped to the one agent that asked. See
`docs/ARCHITECTURE.md`.

**Migrations are forward-only and numbered.** One has already run against a real
database by the time you think of an improvement, and editing it leaves that
database at the same `user_version` with a different schema. Add another.

**Budget counts model calls, not agent turns.** One turn can make several calls
working through tool results. Counting turns lets a bounded run bill many times
over. There is a test named after this.

**A run settles when nothing is outstanding, and an envelope is what is
outstanding.** `deliver` books one against the run as it queues an envelope,
and the turn that reads it releases it. Any new path that takes an envelope and
does not turn it into a turn has to release it too: an agent deleted while
holding queued work used to take the booking with it, and that run never ended.
Nothing else decrements.

**A file's bytes never travel in an envelope, and never cross IPC.** A message
carries a `Part::File` naming the digest; the bytes sit once in `files.rs`,
addressed by content, and a drop hands Rust the *path* rather than the file.
Both follow from the same fact: a transcript is read in bulk, forty messages
into every prompt and hundreds into the activity view. What a model gets depends
on what the file is: a picture is shown, text is read out, and anything else is
written to `~/inbox` on the agent's own machine, because a Linux box knows more
file formats than this runtime ever will. When placing fails the model is told
so in words, since an agent that hears nothing describes a document it never
read.

**Every event is an IPC hop and a render, so tokens are coalesced before they
leave.** A model writes faster than a screen refreshes. One event per token
spent the operator's main thread on work no eye could resolve, and with five
agents answering at once it stopped painting at all, which reads as the window
freezing and the text arriving in a lump rather than streaming. `Pen` in
`runtime/mod.rs` buffers to 16ms and flushes when the call ends. On the other
side, only the component drawing the live bubbles subscribes to `streams`: with
that subscription in `ChannelView` a single token re-rendered every message in
the transcript. `ChannelView.perf.test.tsx` counts both.

**Anything crossing IPC is camelCase.** `rename_all` on a tagged enum renames
variants, not fields; you also need `rename_all_fields`. `ipc.contract.test.ts`
compares the Rust and TypeScript command lists directly, so a rename that only
lands on one side fails the build rather than at runtime.

**`Store::open` has two SQLite lessons encoded in comments.** Do not reorder the
pragmas or simplify the migration transaction without reading them.

**There is one Chrome profile on every machine, and keeping it that way is
deliberate.** Chrome ignores `--remote-debugging-port` when it re-attaches to an
existing profile, so `browse` needs a profile it controls. There used to be a
second, default one behind `open_on_desktop` and the desktop icon, and a
sign-in performed there was invisible to every agent with nothing reporting an
error. Now every route is shimmed onto `/home/user/.guac/chrome`: a wrapper
first on `PATH`, a `.desktop` entry in the user's own XDG directory, the flags
added again at the call site, and a browser found on any other profile is
closed when the desktop starts. If you add a way to open a browser, it goes
through `chrome_flags`, and the port goes with the profile or `browse` loses its
remote interface.

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

**A failed model call is retried before the operator hears about it, and the
row the retry reads is the transcript.** `stream_with_retries` re-attempts only
what `is_transient` admits to, reopens the stream so a second attempt cannot
append to a first one's half-written text, and never reserves a second step: a
call is one call however many times the network dropped it. What survives that
becomes a notice carrying the `cause` of the turn, which is what the operator's
"Try again" sends again, as a new run at the original hop.

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
  domain/            AgentCard, Envelope, Routine, Connector, Signin, Approval,
                     ids. No I/O.
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
  trajectory.rs      Reads a run's events and says whether the machinery did.
  files.rs           Attachments, addressed by the SHA-256 of their contents.
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
./scripts/ci.sh          # lint, typecheck, build, every test suite
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

The trajectory suite asks the third question, about the machinery rather than
the talk: every placeholder closed, every parked turn released, every model
call on the run's bill, nothing filed against a run already reported finished.
Both other suites read the messages, and a run whose messages are all correct
can still have left a half-arrived bubble on screen. If you touch streaming,
settle detection, retries or the budget, this is the one that will catch you.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test trajectory
```
