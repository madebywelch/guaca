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

On top of the asymmetry, one hard rule: when nothing an agent woke to asked it
for anything, and the peer it wants to write to has already had its say, the
send is refused. Both sides are finished, and the only thing left to send is an
acknowledgement of an acknowledgement.

Messages that do not expect a reply are batched: an agent waking to four replies
reads all four in one turn. Because real replies arrive seconds apart rather
than together, an agent will also wait briefly for replies it is still owed —
counted as peers it has written to that have not written back — before reading
what it already has. Waiting instead on "is anyone in this run still busy" was
tried and was wrong: it made an agent sit through peers that had already
answered and were finishing their own notes.

## The budget counts model calls, not turns

An early version reserved one unit of budget per agent turn. A turn can make
several model calls as it works through tool results, so a 12-unit budget
permitted up to 48 billable calls. The unit is now the model call, because that
is the thing that costs money. A cascade test caught this; the fix is one line
and the test that found it is still there.

## Trust is a property of the envelope, and it is restated in words

The survey's "tool poisoning" (MCP) and "task injection" (A2A) describe the same
failure: text that arrived over the wire being read as an instruction from the
principal. Guaca handles it in two places:

1. `Trust` on the envelope: `Operator`, `Peer`, or `System`.
2. The system prompt says what a peer may not do, and every incoming message is
   prefixed with its true origin — `[OPERATOR]`, `[AGENT "Chef"]`, `[SYSTEM]` —
   as the first thing the model reads. An agent that writes `[OPERATOR]` into
   its own message still arrives labelled as an agent. There is a test for that.

The directory deliberately excludes `system_prompt`, so one agent cannot read
another's instructions by listing peers. There is a test for that too.

## Storage

SQLite, two tables, plain SQL, forward-only coded migrations. No ORM: there are
eleven queries and hiding them behind a builder would add a dependency and
remove the ability to read what hits the disk.

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

## Known limitations

Stated plainly rather than discovered later.

- **The API key is stored in plaintext**, mode `0600`, in the app config
  directory. See the README. The honest fix is the OS keychain.
- **No retry on transient upstream failures.** `LlmError::is_transient()` exists
  and is tested, but nothing consumes it yet. A rate-limited agent reports the
  failure into its channel instead of backing off and retrying.
- **History window is fixed at 40 messages** per prompt, with no summarization.
  A very long conversation loses its early context.
- **Undelivered messages to a deleted agent are dropped**, not returned to the
  sender. The sender is not notified.
- **No search.** Finding an old message means scrolling.
- **An agent cannot follow up with a peer that already answered it inside the
  same run.** A genuine second question and a courtesy "thanks" are the same
  shape on the wire, and the refusal is aimed at the second. Making this
  distinction properly means letting the model declare its intent on
  `send_message` rather than inferring it, which is the obvious next change if
  multi-round delegation starts mattering.
- **Prompt instructions are guidance, not guarantees.** Several behaviours here
  are steered by wording in `runtime/prompt.rs` — staying quiet when there is
  nothing to add, not narrating that silence. A model can ignore any of it, so
  anything that must hold is enforced in the runtime and the rest is measured by
  the evals.
