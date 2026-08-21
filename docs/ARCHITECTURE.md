# Architecture

Decisions worth explaining, and the reasoning that would otherwise be lost.
Routines, machines and the workspace have files of their own beside this one,
and `AGENTS.md` routes between all four.

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

The mode follows the work rather than the sender, and the second incident is
why. The first version matched on who the message came from, and a routine
coming due comes from neither the operator nor a peer: it matched no arm of that
match and fell through to the silent mode, so every schedule an agent kept was
answered by an agent that had just been told nothing was being asked of it.
Anything carrying work is `Assigned`, whoever sent it. See `ROUTINES.md`.

The field is trusted, and that is deliberate. A model can label a courtesy as
work, and then the run pays for one extra turn and hits the same per-pair,
hop and budget limits as before. The alternative was to keep guessing, and the
guess was already refusing real work. `courtesy` is the default when nothing is
declared, so a model that ignores the field gets the old, stricter behaviour
rather than a door quietly opened. The rule lives in two places and they have to
agree: `runtime/prompt.rs` tells the model the same thing, in the mode where it
matters.

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

**A pipeline spends two hops per phase, so the depth limit is four phases, not
eight.** A coordinator working through specialists in sequence is one hop out
and one hop back each time, and the next instruction starts from where the last
answer landed rather than from the operator. That is the arithmetic to do before
raising or lowering `max_hops`. The eval suite holds both halves: three phases
reaching hop six, and a fifth phase refused with the depth it had reached.

## The budget counts model calls, not turns

An early version reserved one unit of budget per agent turn. A turn can make
several model calls as it works through tool results, so a 12-unit budget
permitted up to 48 billable calls. The unit is now the model call, because that
is the thing that costs money. A cascade test caught this; the fix is one line
and the test that found it is still there.

## A stop marks the run and releases nothing

The limits above decide when a conversation ends on its own. A stop is the
operator deciding, and the two are the same mechanism seen from different ends:
a run that has been called off is a run with no budget left, except that it is
the person paying who said so.

The mark is on the `RunId`, in the same structure that already decides
settlement, and there is no second generation of a run. `RunId` is minted once
per operator action and every envelope the action causes inherits it, so "this
conversation" is already a value the runtime can name; `retry_turn` is already
how the operator sends the same thing again, as a new run.

**A stop releases nothing.** This is the part that is easy to get wrong and hard
to notice. Every envelope booked against a run is released by whatever consumes
it, and `track_inflight` reads a negative delta against a run it is no longer
counting as that run reaching zero. So a stop that tidily released the envelopes
it was ending would emit a second `RunSettled`, report the run finished twice and
reconcile its spend twice. Marking and waking is the whole of what `stop_run`
does. Each boundary that notices the mark releases through `finish_turn`, the
same call an ordinary turn ends with.

There are four boundaries, and between them they cover every place a turn can
be:

- **After an envelope is dequeued, inside the pause park.** The only case
  `run_turn` cannot reach: an agent that is not accepting work never gets there,
  so its booking would be held until somebody resumed it. The check is inside
  the park rather than above it because an agent that was already asleep when
  the stop arrived has to see it on the wake-up, which is why `stop_run` also
  notifies every inbox.

  The actor only ever examines the envelope it is holding, which is why the same
  place also drains the queue behind it whenever anything at all is stopped. A
  paused agent parked on one conversation would otherwise never notice that a
  different one, sitting behind it in the same inbox, had been called off — and
  that run would wait on a turn that cannot happen until somebody resumes an
  agent the operator has already stopped. What survives the drain keeps its place
  in line in a holding queue, and stays counted in the depth the rail reads,
  because an envelope set aside is as queued as one still in the channel.
- **At the top of the turn**, before the prompt, the placeholder and the first
  call. This catches the whole queued half of a stopped cascade, so a fan-out
  that reached eight agents leaves eight channels each saying why nothing came
  back rather than eight messages nobody answered.
- **Inside the turn**, before each model call and between tool calls. Before the
  call and not after: a step reserved for a call that a stop then prevents would
  leave the run reporting a call it never made, for the rest of its life. Between
  tool calls is the finest boundary that exists, because one tool call is a
  single unbounded await into a sandbox or a browser.
- **After the turn's rounds, before its reply is decided.** The one that is easy
  to leave out, because the two checks above look like they cover the loop. They
  do not cover the way out of it: a turn whose last call comes back with text and
  no tool calls leaves by the ordinary break at the bottom of the round, and a
  stop that landed during that call — which for a single-round reply is the whole
  turn — would reach the reply with the mode it started in and write to the peer
  that was waiting. One lock read per turn closes it.

A stop does not interrupt the model call in flight. The streaming client has no
cancellation handle, so the turn that is talking finishes talking and stops
before it would have called again. That is also what keeps the accounting
honest: a call that was paid for is a call that completed. The same reasoning is
why a stop is not looked at inside the retry loop either, which is the one place
it costs something: a step is claimed for the whole call before the first
attempt, so abandoning it between attempts would leave the run reporting a step
against no call. A stop landing during a backoff waits it out.

A stopped turn keeps its words and sends them nowhere. It reports as a note, so
whatever it managed to say lands in its own channel where the operator can read
how far it got, and the peer that was waiting is not written to. Not sending on
is the whole of what a stop is for.

Two things a stop has to answer for that nothing else does. A turn parked on a
permission request is holding its envelope inside a ten-minute window, so the
stop closes those rows itself — expired rather than denied, because the operator
stopped a conversation and did not refuse an action, and that difference is what
a standing grant would be read out of later. The row moves before the turn is
woken, because the turn reads its verdict back off the row. And a stop of a run
that has already finished returns false and writes nothing: a line in the
transcript saying a conversation was stopped, in a conversation that ended on its
own, describes something that did not happen.

## A subscription is a second provider, not a second endpoint

An operator can pay for a turn two ways: an OpenAI-compatible endpoint with a key
they pasted, or a ChatGPT subscription they signed in to. `InferenceConfig`
carries a `Provider` rather than a flag, because almost nothing about the call is
shared. Different host, different wire protocol, different auth header, a model
list that is not the operator's to choose, and no price on the answer. Modelling
the subscription as "a base URL with a different key" would put all of that
behind a string in a text box, and the first symptom would be an agent failing on
a parameter nobody set.

**The two protocols meet in exactly one function.** `LlmClient::stream_chat`
dispatches on the provider, and `llm/codex.rs` translates. Above that line the
app has one shape of request and one shape of completion: `runtime/mod.rs`
assembles a single kind of call, `prompt.rs` writes one kind of history, tool
results come back one way, and the guard, the budget, the retry loop and the stop
boundaries did not change at all. That is the whole reason for translating rather
than teaching the runtime a second protocol. The cost is one file that has to be
right about both shapes, which is what its tests are for.

**What the Responses API disagrees with chat completions about.** Each of these
was learned from a live call refusing one, and each has a test that fails without
it:

- The system prompt is not a message. It is `instructions`, and the endpoint
  answers 400 without one.
- A tool result is not a role. It is a `function_call_output` item carrying a
  `call_id`, filed as a sibling of the `function_call` it answers, and there is
  no `role: "tool"` to send.
- A tool definition is flat. The nested `{type, function: {...}}` form is
  accepted and then the model is never offered the tool, which reads as an agent
  that has forgotten how to do its job.
- There is no temperature. The parameter is rejected outright rather than
  ignored, so a request carrying one fails in full. `probe` sets one, which is
  why the Test connection button is the path that would have found this.
- Nothing says `[DONE]`. The stream ends on `response.completed`, and the usage
  rides inside it rather than arriving in a frame of its own.

**A subscription call is unpriced, not free.** Tokens are counted and `cost` is
`None`, which is what a local model already reports and what every reader
downstream already handles. Zero would draw as a free call in the usage view.

**Reasoning is asked for and still never kept.** Summaries stream to the operator
and are dropped, exactly as on the other transport. The encrypted reasoning
blobs that would let a later round resume the model's own working are
deliberately not requested: keeping them would mean persisting reasoning and
sending it back, which is the one thing `Token::Reasoning` exists to prevent. A
multi-round turn therefore re-reasons from its tool results rather than
continuing, and that is the price of the promise.

**The sign-in is a device flow, and it lives beside the settings rather than in
them.** `subscription.rs` has both arguments. Briefly: the other half of OAuth's
browser dance is a redirect back, and catching one means either binding a
localhost port or claiming a URL scheme, both of which put the app in the path of
a credential arriving from a browser and both of which fail quietly when
something else got there first. The device flow has no redirect. And the token
set rotates on refresh, which is Guaca writing in the background, while
`config.json` is rewritten wholesale every time the operator presses Save; two
writers on one file lose a refreshed token to a stale in-memory copy, and the
symptom is a sign-in that works until an unrelated setting changes.

**A group that names its own endpoint or key leaves the subscription.**
`GroupInference::apply` flips the provider back to the endpoint, because an
endpoint is not where a subscription is spent. Without it such a group inherits
the app's subscription and has both of its overrides silently ignored: the
operator is looking at a URL and a key that nothing used, with no error to
explain it. Overriding only the model does not flip it, since that is a group
asking for a different model on whatever the app is already paying with. The flip
happens before the model is collapsed, so each case lands on a model its own
provider can run.

**Only OpenAI offers this.** Anthropic prohibits it. Consumer Claude OAuth tokens
are restricted to Claude Code and Claude.ai, enforced server-side, so a Claude
subscription cannot fund a third-party harness however the credential is
obtained. Claude models reach Guaca the same way they always did: an API key, or
OpenRouter, which is still the default. `docs/PROTOCOL.md` has the dates.

## A failed model call is retried before the operator hears about it

`stream_with_retries` is the loop and `LlmError::is_transient` decides. Rate
limits, timeouts, transport failures and 5xx are worth another attempt; a
rejected key or an unknown model is not, because it answers the same way every
time and retrying only delays the message the operator needs to read. Three
attempts, one and three seconds apart, or the provider's own `Retry-After`
capped at twenty seconds.

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
  first is accumulated. A single `&str` callback was tried and made the two the
  same thing by the time anything downstream could tell them apart. Reasoning is
  never persisted, never included in the content hash the loop guard compares,
  and never sent back to a model. Nothing downstream of `stream_chat` holds it,
  so there is no path by which it could be. Two spellings are read (`reasoning`,
  `reasoning_content`) because two conventions exist, and a frame carrying both
  is one thought, not two.
- **It is addressed to the placeholder, which is what makes it ephemeral for
  free.** `ReasoningDelta` names a message id and no channel. The webview files
  it under the agent that opened that stream and drops it when the stream ends,
  so a thought cannot outlive the turn that had it, and a retry that reopens
  under a new id discards the half-formed thought of the attempt that broke.
  The agent is the right key rather than the channel: a turn writing to a peer
  streams into the peer's channel, while the operator watching it work is
  reading its own.
- **It is buffered on the same clock as the text, and stored beside it rather
  than in it.** A model writes faster than a screen refreshes, and one event per
  token spent the operator's main thread on work no eye could resolve: with five
  agents answering at once the window stopped painting at all, which reads as
  freezing and the text arriving in a lump rather than streaming. `Pen` in
  `runtime/mod.rs` buffers text and reasoning alike to 16ms and flushes when the
  call ends. In the store reasoning is its own slice: written into the stream
  buffer, every token would re-render and re-parse the markdown of every live
  bubble for text that is in none of them. The same care applies one level out,
  where only the component drawing the live bubbles subscribes to `streams`:
  with that subscription any higher, a single token re-rendered every message in
  the transcript. `ChannelView.perf.test.tsx` counts both.

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
agent can describe what it is about to do. And `ProtectedAction::ActOnBehalf`
deliberately has no "always allow": a grant is scoped to an agent and an action,
and when the action is "act outside the workspace" a standing yes covers every
future send, submission and purchase. Creating an agent is narrow enough to be
worth not asking twice; this is not.

## A page that was read this turn cannot quietly press a button

The trust boundary below is words: `Trust` on the envelope, `[OPERATOR]` and
`[AGENT "x"]` labels, `WEB_LABEL` in front of every page, and the *Message
sources* section of the system prompt saying a page is data. All of it is
aimed at a model, and an injection is a piece of writing aimed at the same
model, arguing the opposite. Where the two disagree the app had no answer,
because nothing in the runtime stopped a turn that had just read a page from
acting on what it said.

`needs_consent` is the answer, and its whole design is in what it does *not*
gate. Three conditions have to hold together:

- the browser action changes something (`click`, `type`) rather than reading it,
- this turn has already taken in a page or a screenshot,
- the browser is standing on a domain this agent holds a session for.

The third one reads the *browser's* sessions rather than the agent's whole list.
An agent can hold a computer and a browser, they have unrelated cookie jars, and
the URL this is decided from came from the browser. A session the computer holds
is not something a `browse` click could spend, so gating on it would stop and
ask about an account the action cannot touch, which is how an operator learns to
click through the prompt without reading it.

Any one of them on its own refuses ordinary work. Gating reading means an agent
cannot report the attack it found. Gating an untainted turn means a dialog in
front of an agent doing exactly what the operator told it. Gating a site nobody
is signed in to means every form on the open web is a question. Together they
describe one situation, and it is the situation BrowseSafe says is worth the
attacker's time: the agent already holds the operator's account, so the payload
does not have to obtain access, only to be read.

What happens then is the machinery that already existed for `request_permission`:
the turn parks, the operator gets a card in the channel they are reading, and
the answer is read back from the `approvals` row. There is no "always allow",
for the reason `ActOnBehalf` never had one. A standing yes here would be granted
once, on one page, and would cover every page after it.

The rule is a pure function and the asking is not, so the rule can be read and
tested on its own. Two of its tests exist only to fail a careless version:
`notgmail.com` must not match a `gmail.com` session, and
`https://gmail.com@evil.com/` must resolve to `evil.com`. A gate that matched
either would hand an attacker's page the operator's account while looking like
it had thought about it.

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

**A file reaches the operator on the turn's own answer, through `attach_file`.**
For a while it could not reach them at all. `send_message` carries files to
another agent; a turn's final message carried text and nothing else, so an agent
asked for a brief wrote one, saved it, and ended its turn with the path. That
reads as success and is not: `/home/user/brief.md` is a location on a sandbox
the operator does not have, in an app with nothing to click. The tool resolves
its arguments through the same `resolve_files` a send uses, and `emit_reply`
puts what came back on the reply envelope rather than sending anything of its
own, so there is no second message, no extra hop and nothing new for the guard
to judge. Three consequences worth knowing:

- **The reply is delivered when it carries a file and no text.** Handing over a
  document with nothing typed is normal, and a reply judged empty by its text
  alone would drop the thing the whole turn was spent producing.
- **It attaches to whatever the reply is addressed to.** In every mode but
  `ToPeer` that is the operator's channel, including `Assigned`, where a
  delegated agent's answer is filed as a note. A peer turn attaches to the peer,
  which is the honest reading of "attach to your answer".
- **An agent's own attachments are named back to it.** `body_with_files` runs
  over own messages as well as incoming ones. Without it an agent reads its last
  turn back with no file in it, has no record of handing anything over, and
  attaches the same document again while telling the operator it is the first
  time.

The prompt states the mistake rather than the feature (*Handing over a
document*), because the tool schema alone was not enough: a model that has just
saved a file has no reason to go looking for a tool it does not know it needs.

**A drop is taken into the store before anything is sent.** `stage_files` runs
on the drop, which is what lets the app refuse a 40 MB archive while the
operator is still holding it rather than failing the message they went on to
write, and lets it show a picture back to them, since by then it has an address.
One file failing does not refuse the rest of the drop. What the send then
carries is a digest and a name, and the runtime resolves both against its own
store: the size and the type on the message are read off the disk, not taken
from the webview.

**The webview reads a file over a URL, not over IPC.** The scheme is
`guacfile://localhost/{digest}/{name}`, answered out of the file store by
`app.rs`, so a preview is fetched once, by the one element drawing it, only
while that element is on screen, and the webview caches and ranges it. Handing
the same bytes back over IPC would give up exactly what content-addressing
them bought: IPC is where the transcript travels, in bulk. The scheme is also
narrower than Tauri's asset protocol, which opens a scoped part of the disk;
nothing is addressable here but a digest this app stored, and a digest that is
not 64 hex characters is refused before it is ever joined onto a path. The
name in the URL decides the type of the answer and nothing else.

A transcript draws what it can of a file rather than naming it: a picture, a
document's first page in the webview's own viewer, a markdown file as the
document it is, the first lines of anything else textual, and for the rest a row
saying what it is. Each opens a full view, and each offers a copy into the
downloads folder, whose path is said out loud because a file saved somewhere the
operator has to go looking for has not really been saved.

Markdown is its own preview kind because it is what the agents write in. A brief
drawn as monospace `##` is a document the operator reads around rather than
through, and every message body in this app is already rendered as the prose it
is: a file is the same prose that happened to arrive as a file. It goes through
the same `Markdown` component, which means the same trust decision, and that is
the point rather than a coincidence. Raw HTML is off because `rehype-raw` is not
installed, and a document off an agent's machine is no more trustworthy than the
message that carried it. Anything else textual stays source: a log is not prose,
and markdown rules applied to one eat its punctuation.

Two exceptions, and both are WebKit's.

The first is CORS, and it cost every preview but the picture. A response on a
custom scheme is cross-origin to the page that asked for it, so a `fetch` of one
that does not name an allowed origin rejects with `TypeError: Load failed`
before the caller sees a status. An `img` is exempt, and is also the one preview
that does not go through `fetch`, so pictures drew while a markdown brief, a
PDF and a log all came up as a widget saying the file could not be read. The
refusals were unreadable for the same reason: the status of a 404 is no more
visible than the body, so the three sentences `whyNot` exists to tell apart all
arrived as the same one. `file_response` therefore answers everything, refusals
included, with `access-control-allow-origin`. It names the app's own origin
rather than allowing any, and refuses a page that is not this app's with a 403:
this webview also holds a cross-origin frame showing an agent's browser, and a
wildcard would let script in that frame read any file whose digest it could
name. `app_origin` has to keep agreeing with Tauri's `get_app_url`, because an
origin that is merely close fails exactly as a missing one does.

The second is frames. A custom scheme is allowed in an `img` and a `fetch` if
the CSP names it, and refused in a frame however it is named, as a scheme
source, as a host, or through `default-src`, with no violation event and no
console line to say so. A PDF is drawn by the webview's own viewer and that
viewer only runs in a frame, so `localCopy` fetches the document over the scheme
like everything else and hands the frame a `blob:` URL. That is the one place a
file's bytes sit in the renderer: the copy is made when the frame comes near the
viewport and revoked when it leaves. Do not simplify it back to a direct `src`.
It will pass every test in this repo and draw an empty rectangle. Any other way
the renderer is given to reach a file goes through the same scheme, and the CSP
has to name it or the element silently asks for nothing.

Neither of these is visible from the harness or from CI: a mocked `fetch` has no
origin to check and jsdom has no viewer. Both were found by pointing a real
WKWebView at a scheme handler and reading what came back, which is what to do
with the next one.

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
the only pair of places that can be kept in step. Any new path that takes an
envelope and does not turn it into a turn has to release it too; nothing else
decrements. The trajectory suite found this.

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

For a long time every fault it knew about pointed the same way. Answered too
often, said the same thing twice, thanked a thank-you, nagged a peer: all of
them are a crew talking too much, which is the failure this app was built
against and the one it is therefore most likely to overcorrect into. The
opposite failure had exactly one check, `Silent`, and it only fires when the
operator was told nothing by anybody for the whole run. Both times this app
shipped an agent that stopped early, somebody else in the run did answer the
operator, so `Silent` was satisfied and the agent that had actually been given
the job simply never appeared. `AssignedAndSaidNothing` is that gap: it reads
`intent` off the wire, and an agent that was handed work and produced no text
for anyone is named. A tool trail deliberately does not count, because the turn
that shipped the bug did call a tool and what the operator saw was a channel
with no words in it. The runtime half is in `emit_reply`: an `Assigned` turn
that produces nothing files a notice instead of returning quietly, since a turn
with no text produced no envelope at all and therefore left no trace of itself.

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

**A fourth suite asks the same three questions of the other protocol.**
`tests/subscription.rs` drives the real runtime against a scripted Responses
server, because the three above all speak chat completions and would pass with
the subscription path never dispatched, the credential never read and the model
resolved from the wrong field. Its stub asserts the three things a live call was
observed to refuse — no `temperature`, `instructions` present, tools flat — so a
regression is a failing test rather than a 400 nobody sees until they sign in.

It also holds one `#[ignore]`d live test, `./scripts/subscription.sh`. Everything
offline is a stub agreeing with what this app believes the protocol is; the live
one is the only thing that notices when that belief goes stale, which it will,
because the protocol belongs to somebody else.

## Known limitations

Stated plainly rather than discovered later.

- **The API key is stored in plaintext**, mode `0600`, in the app config
  directory, and the ChatGPT sign-in in `subscription.json` beside it. See the
  README. The honest fix is the OS keychain, and it matters more for the sign-in:
  a pasted key is scoped to inference, while that credential belongs to a ChatGPT
  account with more than Guaca behind it.
- **A subscription's remaining quota is not shown.** A plan is metered in hours
  per window by the vendor, Guaca counts tokens, and the two do not convert. So
  the usage view reports what a run spent in tokens with no price, and the first
  sign of a plan running out is the backend refusing a turn. The number lives
  behind an endpoint this app does not call.
- **A multi-round subscription turn re-reasons rather than continuing.** Resuming
  a model's own working across rounds means holding its encrypted reasoning and
  sending it back, which this app promises not to do. The cost is tokens, not
  correctness.
- **History window is fixed at 40 messages** per prompt, with no summarization.
  A very long conversation loses its early context.
- **Undelivered messages to an agent deleted mid-run are dropped**, not returned
  to the sender, and the sender is not notified. The run they belonged to ends
  rather than hanging, but nobody is told what was lost.
- **The consent gate is one tool wide.** `needs_consent` covers `browse`,
  because that is where an action is addressed to a domain and can be matched
  against a session. Two paths reach the same internet with the same credentials
  and are not gated: a `curl` through `run_command`, which is not addressed to a
  domain at all, and a `use_screen` click on a page the operator signed in to on
  the machine's own screen, because a screenshot carries no URL to match. Both
  predate the split into a computer and a browser and neither was made worse by
  it. Wording is still the only thing holding them.
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
