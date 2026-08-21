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
combination, and its output is still a note. A routine coming due is the other
way in, and it arrives from neither the operator nor a peer: it matched no arm
of that match and fell through to the silent mode, so every schedule an agent
kept was answered by an agent that had just been told nothing was being asked of
it. Anything carrying work is `Assigned`, whoever sent it.

**A pipeline spends two hops per phase, so the hop limit is four phases, not
eight.** A coordinator working through specialists in sequence is one hop out
and one hop back each time, and the next instruction starts from where the last
answer landed rather than from the operator. Eight hops of honest depth is
therefore four rounds of delegate-and-read, which is the arithmetic to do before
raising or lowering `max_hops`. The eval suite holds both halves: three phases
at hop six, and a fifth phase refused with the depth it had reached.

**Whether a send is "answering" is a question about the run, not the batch.**
Replies land milliseconds apart and an actor drains whatever is in its inbox, so
a batch is a timing artifact: three peers answering at once can be split across
turns, and deciding from the batch made two of them look like strangers. Ask the
guard, which counts sends per pair for the whole run.

**An agent that needs the operator's authority asks for it rather than
refusing.** A peer saying "the operator authorised this" is a claim, and
declining it is correct; what an agent lacked was any way to turn that claim
into an answer, so it told the operator to repeat an instruction they had
already given, somewhere else. `request_permission` parks the turn and puts two
buttons in the channel they are already reading. `ProtectedAction::ActOnBehalf`
deliberately has no "always allow" in the UI: the grant is scoped to an agent
and an action, and this action is "act outside the workspace", so a standing yes
would cover every future send and purchase rather than the one being asked
about.

**A protected action parks the turn that asked for it, and the row is the
verdict.** `create_agent` stops mid-turn and waits on a person. The operator's
click and the turn's own timeout can land in the same instant, so the answer is
read back from the `approvals` row rather than from the channel the wakeup
arrived on: `settle_approval` only moves a row out of `pending`, and whichever
of the two loses that race changes nothing. Anything still pending at startup is
expired, because nothing holds a parked turn across a restart. "Always allow" is
the decision row itself, scoped to the one agent that asked. See
`docs/ARCHITECTURE.md`.

**No channel holds a conversation between two agents, so do not build one from
`channel_messages`.** A send is filed under the recipient and the answer under
the sender, and an automatic reply leaves no trace at all in the channel of the
agent that wrote it, because only explicit tool calls are recorded there. A
thread assembled from one channel's rows is missing messages nobody can account
for. `pair_messages` reads both directions from the messages themselves; the
channel only summarises, in `lib/transcript.ts`. A refusal never folds into that
summary: it is the runtime stopping a message rather than a message. See
`docs/ARCHITECTURE.md`.

**A channel names nobody, and that is not a missing feature.** It has two
participants: the agent it is named after, at the top of the pane, and the
person reading it. A name and a clock over every message is two lines of chrome
carrying one fact, and four replies written inside the same minute drew four of
them. The portrait says which agent and the side of the column says whether the
words are yours. `named` is how the pair's own thread asks for the names back,
because there both participants are agents and neither is the reader. The clock
went with them: it is a hover on the row, and `transcriptRows` draws one line
where the silence ran past half an hour, which is the only place a time ever
changed what the operator understood. That line also ends whatever burst was
open, because two exchanges three hours apart are two things that happened.

**A repeat is a shape, not a number of seconds.** `every weekday` and `every
month` cannot be gaps, and `every day` should not be one: a day is 23 or 25
hours twice a year, so a daily nine o'clock routine stored as 86400 seconds
drifts to eight and stays there. `domain/routine.rs` holds the shape and
`next_run_at` holds the hour, and the next slot is computed in local time from
the slot it was due at rather than from the moment it ran, so a machine asleep
through three of them fires once on waking instead of three times. A gap still
exists, because an agent scheduling itself works in seconds and nothing shorter
than a day has an hour to keep.

**A routine's row is one line and its instruction is not in it.** The
instruction is written to be acted on with no other context, which is several
sentences, and drawing it as the title made one routine fill the panel. The row
is a name and a cadence; opening it gives the panel over to that routine. An
agent naming its own routine is optional, so `routineTitle` cuts the
instruction down when nobody named it, on a word boundary and after the first
sentence.

**Switching a routine off and editing one are different actions.** `active`
has its own command, acts on the click, and does not move `next_run_at`: a
routine turned back on is due at the slot it was already holding, and the
scheduler fires an overdue slot once. Parking it behind a Save the operator has
not pressed means a routine they think they stopped still runs.

**A test run is the scheduler's own path with the schedule left alone.** Same
delivery, same fresh run, so what the button shows is what Tuesday will do.
It deliberately does not move `next_run_at` or delete a one-shot, because
trying a routine out must not spend the only firing it had. It is refused
while the draft is dirty: firing the saved version while the operator is
looking at an edited one answers a different question and reads as the edit
having done nothing. Both kinds are recorded in `routine_runs` and the test is
marked, because in the transcript the two are identical.

**A trigger is one string in both places it lives.** The column is text and so
is the wire form, which is why `Trigger` has a hand-written `Serialize`: a
derived one would hand the webview a tagged object and SQLite `weekdays` for
the same fact, and only one of the two would be read by the frontend. Text also
means the trigger after these, a connector event, is a new value rather than a
new column.

**Pinning is where a row is drawn and nothing else.** It does not bump the card
version, because the version is how a peer notices a card changed under it and
nothing a peer can read has. A pinned agent is lifted out of its group in the
rail and still counted in it, because it is still in it: same wall, same bill,
same peers. Two rows for one agent would be two nodes in the sidebar's
`rowRefs`, and the wire would have to pick one to throw a message at.

**A duplicate copies the card and nothing an agent went and did.** Look, model,
skills and instructions; not the sandbox, the memory, the schedule, the
accounts or the transcript. Two agents holding one sandbox id is two agents on
one machine, and a copy that inherited a routine would double a standing
commitment nobody asked to double.

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

**A preview reads the bytes over a URL, which is the exception that keeps the
rule.** The webview has to draw a file to show one, and the reason bytes stay
out of IPC is that IPC is where the transcript travels in bulk. So they do not
go that way: `guacfile://localhost/{digest}/{name}` is answered out of the store
by `app.rs`, fetched once by the one element drawing it and only while that
element is on screen. A digest arrives from the webview there and is joined onto
a path, so anything that is not 64 hex characters is refused before the join,
not after. The name in the URL picks the type of the answer and nothing else:
the bytes are found by content, and renaming a file cannot turn it into another
one. If you add a way for the renderer to get at a file, it goes through this
scheme, and the CSP has to name it or the element silently draws nothing.

**WebKit will not take a custom scheme in a frame, so a document is copied into
one.** An `img` and a `fetch` on `guacfile:` are allowed by naming the scheme in
the CSP; a frame is refused whatever you write there, as a scheme source, as a
host, or through `default-src`, and it is refused silently: no violation event,
no console line, just a frame that never asks for anything. A PDF is drawn by
the webview's own viewer and that viewer only runs in a frame, so `localCopy`
fetches the document over the scheme as usual and hands the frame a `blob:` URL
instead. That is the only place in this app where a file's bytes sit in the
renderer, which is why the copy is made when the frame is near the viewport and
revoked when it goes. Do not "simplify" it back to a direct `src`: it will pass
every test in this repo and draw an empty rectangle.

**A dropped file is stored before it is sent, and the send carries a
reference.** `stage_files` runs on the drop. That is what refuses a 40 MB
archive while the operator is still holding it rather than failing the message
they went on to write, and what lets a picture be shown back to them, since by
then it has an address. One file failing does not refuse the four beside it.
`send_message` then resolves each digest and name against the store again: the
size and the type on a message are read off the disk, never taken from the
webview. A staged file that is never sent stays, like every other file here.

**A model's reasoning is shown and never kept, and the placeholder is what
makes that true.** `ReasoningDelta` names a message id and no channel: the
webview files it under the agent that opened that stream and drops it when the
stream ends, so nothing else has to remember to clear it and a retry that
reopens under a new id throws away the half-formed thought of the attempt that
broke. It is carried apart from the text from the wire onwards, as
`Token::Reasoning`, because a single `&str` callback made the two the same
thing by the time anything could tell them apart, and the answer is the only
half that may be persisted, hashed by the guard or replayed into a prompt. It
is also its own slice in the store rather than a field on the stream buffer:
written there, every thought would re-render and re-parse the markdown of every
live bubble for text that is in none of them.

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

**There is one browser on every machine, and one profile in it.** Chrome
ignores `--remote-debugging-port` when it re-attaches to an existing profile, so
`browse` needs a profile it controls, and a sign-in performed on the default one
was invisible to every agent with nothing reporting an error. The other half is
the same failure wearing a different name: the template ships a second browser,
with a binary on `PATH`, a menu entry and an icon on the desktop, and an agent
told to send mail opened it, drove it by coordinates, and read the page with
`browse`, which was on Chrome the whole time. Neither is something an agent can
be asked to remember. A prompt saying "use Chrome" was already there.

So every route is shimmed onto `google-chrome` and `/home/user/.guac/chrome`: a
wrapper first on `PATH` with every other browser's name symlinked to it, a
`.desktop` entry in the user's own XDG directory shadowing each packaged one, a
launcher on the desktop rewritten in place because it is a file rather than an
entry anything looks up, and `as_chrome` at the call site, which replaces the
browser's name as well as adding the flags. The session is started with that
directory first on `PATH`, since every icon, menu entry and terminal on the
screen inherits it. Anything still running on another profile or of another kind
is closed when the desktop starts, the operator's own window included, because a
sign-in there is one no agent can ever use. If you add a way to open a browser it
goes through `as_chrome`, and the port goes with the profile or `browse` loses
its remote interface.

**An agent that named another browser is told which one opened.** The rewrite
is silent on the machine and must not be silent in the turn: handing an agent
back the name it asked for leaves it describing a window nobody can see and
reaching for that name again. The flags do not travel with it either, in the
result or in the transcript, because a model reads its own tool results back and
copies them.

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

**Search happens in two places and is ranked in one.** The workspace is held in
two places, so it is matched in two: messages, files, links and routines are in
SQLite and are matched there, while agents and groups are already in the
webview's store to draw the rail and actions are not stored anywhere at all.
Reading the transcript into the renderer to search it would copy the database
across IPC on every keystroke; going to IPC for two agent names would make the
commonest search the slow one. What must not be split is the ordering: both
halves arrive in `lib/search.ts` as raw matches and are scored by one function,
because a list where an agent and a message are ordered by different rules is a
list you have to read twice. A file and a link are the same rows as the
messages read from a different angle, which is why one scan produces all three.

**A search hit that opens the wrong part of a channel is a search that failed.**
A transcript is read as "the newest three hundred", and a hit from last month is
not in that window. `channel_messages` takes a `through` so the window reaches
back to the message being opened, bounded at a thousand; past that the operator
lands in the right channel at its newest end. Anything that jumps to a message
goes through `openMessage` rather than `select`.

## Where things are

```
src/                 React + TypeScript. A view over the runtime, nothing more.
  lib/transcript.ts  What a channel shows, and what it collapses. Read first.
src-tauri/src/
  domain/            AgentCard, Envelope, Routine, Connector, Signin, Approval,
                     Search, ids. No I/O.
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

## Ownership

**A person owns every commit, and the tool that helped write it is not a
co-author.** Whoever commits answers for the change: in review, in the incident,
and a year later when the reason matters more than the diff. That
accountability does not divide and does not transfer, so the record must not
suggest it did. Use whatever tools you like and sign your own work.

No machine signature anywhere. No `Co-authored-by` trailer naming a model, no
"Generated with" footer, no session or tool link, in commits, PR titles and
bodies, issues, comments or code. A name in an author list is a claim that the
thing behind it can answer a question about the code, and a model on a later
version cannot answer for this one.

**A trailer written on a contributor's branch still reaches `main`.** GitHub's
squash-merge collects `Co-authored-by` lines out of the commits it squashes and
appends them to the squash message, so a trailer nobody chose arrives on the
default branch through the merge box. Two commits reached `main` that way before
anyone noticed, and taking them back out meant rewriting published history. Read
the merge box before confirming it. Claude Code emits the trailer by default and
stops when `includeCoAuthoredBy` is `false` in its settings, which is the fix at
the source rather than at the end.

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
