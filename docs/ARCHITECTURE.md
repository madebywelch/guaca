# Architecture

Decisions worth explaining, and the reasoning that would otherwise be lost.

## The runtime lives in Rust, not the webview

The requirement is that agents message each other asynchronously and without
blocking. That could be done in the renderer, and it would be wrong:

- A page reload would kill every in-flight conversation.
- The API key would have to live in the webview.
- "Non-blocking" would mean an event loop shared with rendering, so five agents
  streaming at once would fight the UI for frames.

Instead each agent is a `tokio` task with an unbounded `mpsc` inbox. Sending is
enqueue-and-return. The frontend is a view: it renders state and forwards
intent, and holds nothing durable.

The cost is an IPC boundary and a duplicated type definition on each side. The
`ipc.contract.test.ts` test compares the two sources directly so drift is caught
at build time rather than when a user clicks something.

## One channel per agent, and one rule for routing

Every envelope is filed under exactly one agent's channel, decided at write time
by `channel_for`:

- human → agent, and agent → human, file under that agent.
- agent A → agent B files under **B**.

That single rule produces the behaviour you want without any special-casing:
asking Manager to introduce itself fills every other agent's channel with the
introduction, and fills Manager's channel with the replies as they come back.
`#activity` is a query over the same table, not a second copy.

Deciding this at write time rather than inferring it at read time means the
transcript cannot disagree with itself later.

## A channel says an exchange happened; the pair's thread is what it said

The same rule that makes a channel readable means no channel holds a
conversation between two agents. A → B is filed under B and B's answer under A,
and an automatic reply leaves no trace at all in the channel of the agent that
wrote it: only explicit tool calls are recorded there. So the thread is its own
query, `pair_messages`, read from the messages themselves in both directions.
Anything assembled from one channel's rows would be missing messages nobody
could account for.

What a channel shows of peer traffic is therefore a summary, and a deliberately
lossy one. `transcriptRows` collapses a burst (a fan-out, and the answers
landing milliseconds apart) into one centred line per peer, counting what that
channel holds. The thread behind the line can hold more, which is the right way
round: clicking reveals more, never less. Two things never fold in. A refusal is
the runtime stopping a message rather than a message, so it keeps its own line
with its reason; and anything that is not peer traffic ends the burst, because a
tool trail between two exchanges is a break in what was happening.

## Cascades terminate because of one asymmetry

The loop guard (`runtime/guard.rs`) bounds the worst case, but bounding is not
the same as terminating well. What actually makes a cascade converge is
`expects_reply`:

- A human message, or a message to a peer that has not written to this agent in
  this run, expects an answer.
- A message back to someone who has already written expects nothing: it is a
  continuation, not an approach.

An agent that receives a non-reply-expecting message still takes a turn to read
it, but its output is filed as a note in its own channel rather than delivered
onward. So a broadcast settles in three levels: Manager sends, peers reply,
Manager reads. Without this, two agents being polite at each other would grind
against the hop limit every time, wasting the entire budget to reach the same
end state.

**"Has already written" is a question about the run, not about the batch.** This
is the subtlety that took several attempts. Replies land milliseconds apart and
an actor drains whatever is in its inbox, so a batch is a timing artifact: three
peers answering one broadcast can be split across two turns, and deciding from
the batch made the late two look like agents this one had never spoken to. Their
messages then demanded answers and the cascade restarted. The guard already
counts sends per pair for the whole run, and that answer does not change with
arrival order.

On top of the asymmetry, one rule: when nothing an agent woke to asked it for
anything, and the peer it wants to write to has already had its say, a courtesy
is refused. Both sides are finished, and a thank-you here is the beginning of a
crew talking to itself.

**Which it is, the sender declares.** `send_message` carries an `intent` of
`work` or `courtesy`, and only the courtesy is turned away. Deciding it from the
shape of the exchange was tried first and was wrong: a second instruction and a
thank-you are identical on the wire, so the rule refused both. A real session
lost an authorised external send that way. The operator authorised it, the
coordinator relayed the authorisation, read the answer, and was refused when it
tried to instruct again, because by then nobody was waiting on it. Every
delegation that takes two rounds died at exactly that point.

**Being asked for an answer and being given work are different questions.**
`expects_reply` is the first and `intent` is the second, and they came apart the
moment an agent could instruct a peer that had already answered: such a message
carries work and expects no reply. The runtime had only `expects_reply` to go
on, so that turn ran in the mode whose prompt says nothing is being asked of it
and that silence is usually right. The agent read an explicit instruction to
send an email, spent a model call, said nothing, and to the operator had simply
stopped. `intent` is now on the envelope and `ReplyMode::Assigned` is the
combination of work with nobody waiting: do it, then file what you did as a note
in your own channel. The asymmetry that terminates cascades is untouched, because
what changed is what the recipient is told, not where its answer goes.

The field is trusted, and that is deliberate. A model can label a courtesy as
work, and then the run pays for one extra turn and hits the same per-pair,
hop and budget limits as before. The alternative was to keep guessing, and the
guess was already refusing real work. `courtesy` is the default when nothing is
declared, so a model that ignores the field gets the old, stricter behaviour
rather than a door quietly opened.

Messages that do not expect a reply are batched: an agent waking to four replies
reads all four in one turn. Because real replies arrive seconds apart rather
than together, an agent will also wait briefly for replies it is still owed,
counted as peers it has written to that have not written back, before reading
what it already has. Waiting instead on "is anyone in this run still busy" was
tried and was wrong: it made an agent sit through peers that had already
answered and were finishing their own notes.

## The five limits, and what each is for

The guard is the backstop, not the mechanism. Each limit catches a different
shape of runaway, so weakening one is not a local change. All are adjustable in
Settings.

| Limit | Default | Stops |
|---|---|---|
| Model calls per run | 60 | Runaway spend, whatever the shape |
| Relay depth | 8 | Long delegation chains |
| Messages between any two agents | 6 | Two agents ping-ponging |
| Recipients per send | 8 | One message blasting the whole roster |
| Identical message to the same peer | 1 | An agent restating itself |

When a limit is hit the agent is told why, in words it can act on, and the
reason appears on the transcript chip. Nothing is dropped silently.

## The budget counts model calls, not turns

An early version reserved one unit of budget per agent turn. A turn can make
several model calls as it works through tool results, so a 12-unit budget
permitted up to 48 billable calls. The unit is now the model call, because that
is the thing that costs money. A cascade test caught this; the fix is one line
and the test that found it is still there.

## A failed model call is retried before the operator hears about it

`LlmError::is_transient` decides. Rate limits, timeouts, transport failures and
5xx are worth another attempt; a rejected key or an unknown model is not,
because it answers the same way every time and retrying only delays the message
the operator needs to read. Three attempts, one and three seconds apart, or the
provider's own `Retry-After` capped at twenty seconds.

Two rules keep it honest:

- **The stream is reopened on each attempt.** Text the operator has already
  watched arrive belongs to the attempt that broke, and the replacement starts
  from the beginning. Appending the second attempt to the first reads as a
  sentence starting over halfway through.
- **No attempt reserves its own step.** A call is one call however many times
  the network dropped it. Billing per attempt would let a flaky connection spend
  a run's budget without producing anything.

What survives all three becomes a notice carrying the `cause` of the turn, which
is what the operator's "Try again" sends again, as a new run at the original hop.

## A thought is shown and never kept

A turn can spend a minute working through tool results before it writes a word.
For that minute the operator had a pulsing avatar and the sentence "Manager is
working", which says a turn is alive and nothing about what it is doing. Where
the provider publishes the model's own working, that is what the line above the
composer now shows: the line it is on, replaced as it writes.

Three things keep it from becoming a fourth kind of message.

- **It is carried apart from the text, from the wire onwards.** `Token::Text`
  and `Token::Reasoning` reach the runtime as separate fragments, and only the
  first is accumulated. Reasoning is never persisted, never included in the
  content hash the loop guard compares, and never sent back to a model. Nothing
  downstream of `stream_chat` holds it, so there is no path by which it could
  be. Two spellings are read (`reasoning`, `reasoning_content`) because two
  conventions exist, and a frame carrying both is one thought, not two.
- **It is addressed to the placeholder, which is what makes it ephemeral for
  free.** `ReasoningDelta` names a message id and no channel. The webview files
  it under the agent that opened that stream and drops it when the stream ends,
  so a thought cannot outlive the turn that had it, and a retry that reopens
  under a new id discards the half-formed thought of the attempt that broke.
  The agent is the right key rather than the channel: a turn writing to a peer
  streams into the peer's channel, while the operator watching it work is
  reading its own.
- **It is buffered on the same clock as the text, and stored beside it rather
  than in it.** Reasoning arrives as fast as an answer and costs the same IPC
  hop and render, so `Pen` coalesces both to 16ms. In the store it is its own
  slice: written into the stream buffer, every token would re-render and
  re-parse the markdown of every live bubble for text that is in none of them.
  `ChannelView.perf.test.tsx` counts that.

Only the newest line is kept, because there is nowhere to scroll back to. Models
that publish nothing (Anthropic's, over OpenRouter, unless thinking is asked for)
leave the line exactly as it was.

## Trust is a property of the envelope, and it is restated in words

The survey's "tool poisoning" (MCP) and "task injection" (A2A) describe the same
failure: text that arrived over the wire being read as an instruction from the
principal. Guaca handles it in two places:

1. `Trust` on the envelope: `Operator`, `Peer`, or `System`.
2. The system prompt says what a peer may not do, and every incoming message is
   prefixed with its true origin (`[OPERATOR]`, `[AGENT "Chef"]`, `[SYSTEM]`),
   as the first thing the model reads. An agent that writes `[OPERATOR]` into
   its own message still arrives labelled as an agent. There is a test for that.

The directory deliberately excludes `system_prompt`, so one agent cannot read
another's instructions by listing peers. There is a test for that too.

## A protected action parks the turn that asked for it

Agents can add agents. Every other tool acts on the asking agent's own machine,
memory or peers; this one changes the workspace and adds something the operator
pays to run, so it stops and asks.

The turn does not return and retry later. It parks: `create_agent` writes a
request row, puts a card in the channel it is talking in, and awaits a channel
under a ten-minute timeout while the actor holds its place. That costs one idle
task and keeps the whole thing linear, so the tool result the model reads is the
real outcome rather than a promise. It also means the run genuinely has not
settled while a person is thinking, which is the truth.

Three things follow, and each is load-bearing:

- **The row is the verdict, not the channel.** The operator's click and the
  turn's timeout can land in the same instant. `settle_approval` only moves a
  row out of `pending`, so whichever arrives second changes nothing, and the
  parked turn reads its answer back from the row afterwards. A button that
  visibly said "allowed" therefore allowed it.
- **A restart expires everything pending.** Nothing holds a parked turn across
  one, so a `pending` row after a restart is a question that can no longer reach
  anybody. `expire_pending_approvals` runs at startup, before the window opens.
- **"Always allow" is one agent being let off one question.** The grant is the
  decision row itself (`state = 'alwaysAllow'` for that agent and that action),
  not a second table that could disagree with it. It is scoped per agent because
  that is what the operator was asked: allowing the Manager to create agents
  says nothing about anyone else. Deleting an agent deletes its grants, since a
  freed name must not inherit them.

The wording the operator reads is composed by the runtime from the validated
draft, never by the model. An agent that could write its own request could
describe creating an agent as tidying up.

**The second protected action is acting in the operator's name**, and it exists
because refusing was the only other move. An agent told by a peer that the
operator authorised an email is being told a claim; declining it is right, and
the app's answer to that was for the agent to ask the operator to repeat the
instruction in another channel. The operator had already decided, and was being
asked to do the routing by hand. `request_permission` parks the turn and puts
the question where they are already looking.

Two differences from `create_agent`. The heading is the runtime's but the
sentence being decided is the agent's own, quoted under it, because only the
agent can describe what it is about to do. And there is no "always allow": a
grant is scoped to an agent and an action, and when the action is "act outside
the workspace" a standing yes covers every future send, submission and purchase.
Creating an agent is narrow enough to be worth not asking twice; this is not.

## Files are references, and what a model gets depends on what they are

Agents exchange documents, and the operator drops them into a channel. Three
decisions carry that:

**The bytes are never in the envelope.** A message carries a `Part::File`: the
name, the type, the size, and the SHA-256 that addresses the contents. The bytes
live once, content-addressed, under the app's data directory. A transcript is
read forty messages at a time into every prompt and hundreds at a time into the
activity view, so a proposal inlined into a message would be dragged through
both, every time. Content addressing also makes the common case free: the same
document sent to four agents and forwarded once is one file.

**What reaches the model depends on the file.** A picture goes as a picture,
down the same path as a screenshot from `use_screen`, because that is the one
thing a model cannot be told about in words. Text is read into the prompt, cut
at a limit that says it was cut. Anything else, a proposal in Word or a
spreadsheet, is written to `~/inbox` on the agent's own machine and the agent is
told the path. The host does not learn to parse PDF or docx: the agent has a
Linux box and can install what it needs, which is the premise the rest of the
app already rests on.

**A file that could not be delivered is admitted.** Placing needs a machine, and
starting one can fail. The agent is told, in words, that the file is out of
reach and not to describe something it has not read. The same holds for a file
an agent asks to send and does not have. Silence here is the worst available
outcome, because both ends believe the document arrived.

Files an agent attaches are resolved first against its own channel, which is
forwarding and needs no machine at all, and otherwise as a path on its computer,
which is where an agent that *produced* a document has it. The operator's end is
the same pipe: `dragDropEnabled` hands Rust the dropped paths, so the bytes are
read on the Rust side and never enter the webview.

**A drop is taken into the store before anything is sent.** `stage_files` runs
on the drop, which is what lets the app refuse a 40 MB archive while the
operator is still holding it rather than failing the message they went on to
write, and lets it show a picture back to them, since by then it has an address.
One file failing does not refuse the rest of the drop. What the send then
carries is a digest and a name, and the runtime resolves both against its own
store: the size and the type on the message are read off the disk, not taken
from the webview.

**The webview reads a file over a URL, not over IPC.** `guacfile://localhost/
{digest}/{name}` is answered out of the file store by `app.rs`, so a preview is
fetched once, by the one element drawing it, only while that element is on
screen, and the webview caches and ranges it. Handing the same bytes back over
IPC would give up exactly what content-addressing them bought: IPC is where the
transcript travels, in bulk. The scheme is also narrower than Tauri's asset
protocol, which opens a scoped part of the disk; nothing is addressable here but
a digest this app stored, and a digest that is not 64 hex characters is refused
before it is ever joined onto a path. The name in the URL decides the type of
the answer and nothing else.

A transcript draws what it can of a file rather than naming it: a picture, a
document's first page in the webview's own viewer, the first lines of anything
textual, and for the rest a row saying what it is. Each opens a full view, and
each offers a copy into the downloads folder, whose path is said out loud
because a file saved somewhere the operator has to go looking for has not really
been saved.

One exception, and it is WebKit's. A custom scheme is allowed in an `img` and a
`fetch` if the CSP names it, and refused in a frame however it is named, with no
violation event and no console line to say so. A PDF is drawn by the webview's
own viewer and that viewer only runs in a frame, so a document is fetched over
the scheme like everything else and handed to the frame as a `blob:` URL. That
is the one place a file's bytes sit in the renderer: the copy is made when the
frame comes near the viewport and revoked when it leaves.

Two limits worth knowing: 25 MB in, and 8 MB onto a machine, because bytes reach
a sandbox as base64 inside a shell command. A real upload endpoint is the fix
for the second.

## Storage

SQLite, plain SQL, forward-only numbered migrations. Eight tables: agents,
groups, messages, usage, approvals, routines, signins and connectors. No ORM:
every query is a few lines of SQL, and hiding them behind a builder would add a
dependency and remove the ability to read what hits the disk.

Two lessons are encoded in `Store::open`:

- `journal_mode` is a property of the file and switching it needs an exclusive
  lock, so it happens once on a lone connection before the pool exists. Letting
  eight pooled connections race for it logs "database is locked" and silently
  leaves some on the rollback journal. A busy timeout does not help: SQLite
  treats a shared/exclusive conflict inside one process as a deadlock and fails
  immediately.
- Migrations run inside an *immediate* transaction and re-read `user_version`
  after taking the write lock. Reading the version first lets two callers both
  see 0 and both try to create the tables.

Agents are ordered by `rowid`. `created_at` is not a total order at millisecond
resolution, and falling back to the UUID sorts the sidebar randomly.

## Deleting an agent is a soft delete

Hard deletion would punch holes in transcripts belonging to other agents, which
had nothing to do with this one. Instead the agent is marked terminated: it
leaves the rail and the directory, its actor stops, it can never be messaged
again, and its name is freed for reuse by a partial unique index over live rows.
What it already said stays readable.

Deleting one mid-run releases whatever it was holding. A run settles when
nothing is outstanding, and the turn that reads an envelope is what releases it,
so an agent deleted with work in its inbox used to take those bookings with it:
the run never settled, its spend was never reconciled against the store, and the
entry sat in the in-flight table for the life of the process. The booking is now
made where an envelope is queued and released wherever one is dropped, which is
the only pair of places that can be kept in step. The trajectory suite found
this.

## Why there is no Docker image for the app

The engineering default here is to containerize services. A GUI desktop binary
is not a service, and containerizing one produces ceremony rather than
reproducibility. The reproducibility that matters for this project is in CI,
where the Rust core, the frontend, and the test suites build in a clean
environment. That is where the container belongs.

## The blank-window failure, and why it can't recur silently

The first release bundle opened to a solid green window with no elements.
`useLiveAgents()` returned a fresh array from `filter()` on every render, and
`Sidebar` had that array in a `useLayoutEffect` dependency list alongside a
`setState`. Effect ran, set state, re-rendered, produced a new array reference,
ran the effect again. React aborts after roughly fifty nested updates by
unmounting the whole tree, so the window painted its background and stopped.
Nothing threw where anyone could see it, and reloading reproduced it exactly.

Three things came out of it, and all three are worth keeping:

1. **Selectors that derive a value return a stable reference.** `useLiveAgents`
   memoizes and `useAgentLookup` is a `useCallback`. This is correctness, not
   optimization.
2. **`ErrorBoundary` plus a `window.onerror` fallback** paint the failure into
   the page. A blank window is the one failure mode with no diagnostic value at
   all; anything is better.
3. **`App.test.tsx` mounts the real component tree.** The whole Rust suite was
   green while the window was blank, because nothing rendered a component. A
   render smoke test is the cheapest possible guard against shipping that again.

The lesson generalizes: "the process started and logged ready" is not evidence
that the app works.

## Three test suites, asking different questions

The cascade suite answers "does the runtime do what it was told". The evals
answer a different question: given an instruction someone would actually type,
is the resulting traffic reasonable? Every cascade defect this app has had
passed the first and failed the second, because each individual message was fine
and the shape was not.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test evals   # scripted, in CI
./scripts/evals.sh                                             # live, costs money
```

The scripted ones run a stub model playing a specific bad habit on purpose and
check the runtime contains it. The live ones run the real prompts against your
configured model and print the whole conversation, which is the only way to see
that a prompt change made agents chattier.

`src-tauri/src/eval.rs` is the analyser: it reads a run's envelopes and names
what went wrong, and every fault it reports is decidable from the messages
rather than judged.

Both of those read the messages, and there is a class of defect neither can
see, because it leaves the messages intact. A placeholder that opens and never
closes is a bubble that stays half-arrived until the window is closed. A settle
that fires while an agent is still thinking stops the spinner and then keeps
talking. A budget that counts turns rather than model calls bills a bounded run
several times over. The third suite reads the event stream the UI is drawn from
and asks whether the machinery behaved.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test trajectory
```

`src-tauri/src/trajectory.rs` is that analyser. A run's events become an ordered
ledger (asked, thinking, placeholder opened, model called, tool used, message
persisted, settled) and the anomalies are properties of that ledger: a stream
left open, text after a stream ended, a parked turn nobody released, a step
count that does not match the calls, anything filed against a run already
reported finished. Nothing is timed. A wall clock in an assertion is a flake,
and "the run took too long" is what the settle timeout already says.

The event stream is therefore a test contract as well as a UI feed. An event
that stops being emitted, or one emitted twice, fails here rather than showing
up as a spinner nobody can explain.

## Known limitations

Stated plainly rather than discovered later.

- **The API key is stored in plaintext**, mode `0600`, in the app config
  directory. See the README. The honest fix is the OS keychain.
- **History window is fixed at 40 messages** per prompt, with no summarization.
  A very long conversation loses its early context.
- **Undelivered messages to an agent deleted mid-run are dropped**, not returned
  to the sender, and the sender is not notified. The run they belonged to ends
  rather than hanging, but nobody is told what was lost.
- **No search.** Finding an old message means scrolling.
- **A peer's answer to a follow-up instruction goes to its own channel, not back
  to the agent that instructed it.** `expects_reply` stays false for a message
  to a peer that has already written, because that asymmetry is what makes
  cascades terminate. So a coordinator can now instruct twice in one run, but
  reads the outcome where the operator does rather than being handed it. Routing
  it back means recording the declared intent on the envelope, which is a
  schema change.
- **Prompt instructions are guidance, not guarantees.** Several behaviours here
  are steered by wording in `runtime/prompt.rs`: staying quiet when there is
  nothing to add, and not narrating that silence. A model can ignore any of it, so
  anything that must hold is enforced in the runtime and the rest is measured by
  the evals.
