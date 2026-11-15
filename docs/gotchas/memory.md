# Memory and working notes

Two stores, and the asymmetry between them is the design. *An agent's memory is
what it knows, and its working notes are what it is doing* in `docs/WORKSPACE.md`
is the argument; `workspace.rs`, `domain/worknote.rs`, `components/Memory.tsx`
and `components/WorkingNotes.tsx` are the code.

- **Memory and working notes are two stores, and the asymmetry between them is
  the design.** Memory is a file the agent rewrites whole; a working note is a
  row it appends and can never revise. One store cannot hold both lifetimes: an
  agent given only memory puts its progress there, correctly, because the
  alternative to writing down what it is waiting on is forgetting it, and a page
  rewritten every turn is one where copying a stale line forward is cheaper than
  deciding it is stale. Consolidating after every interaction is also the regime
  that measurably degrades a memory, so the small store is reconciled rarely and
  the perishable one is never consolidated at all. `docs/WORKSPACE.md`.
- **Nothing offers an agent a way to edit or delete a working note.** The oldest
  fall off past `KEPT` and that is the whole of forgetting. A stale note steers
  the next turn toward work already done, and deciding what to drop is the
  operation these models are worst at: this is the one store that never asks.
  Adding a `revise` or a `clear` to the tool surface hands that decision straight
  back.
- **A line an agent already holds is not stored twice, and its stamp does not
  move.** The only brake a store that never asks the agent to prune can have.
  *Still waiting on the legal read* on four turns is four of sixteen slots on
  one fact, and told "noted" the agent learns that restating is how you say
  something is still true. The repeat comes back with the age of the note that
  already says it, which is what it was reaching for; refreshing the stamp
  instead would hide staleness at the moment the age is worth reading. It is
  still not a revision: nothing is edited and nothing is dropped, the second
  write never happens. The same reasoning is why the tool and the prompt ask
  whether the *next* turn would go wrong without the note, rather than saying
  "note freely", which is what they said first and what filled the lists.
- **Working notes are a table because an append is a read-modify-write.** An
  agent's stores are written from every thread it holds, so appending to a file
  loses notes under exactly the concurrency this app has. The insert and the
  trim share one transaction for the same reason: between them the agent is over
  its own bound, and a read landing there gets a list one longer than the design
  says exists.
- **A working note is stamped, and the age is what makes the list worth
  reading.** Undated, a list of notes says an agent is working; the same list
  marked *6d ago* says the thing it waits for is not coming. The age is rendered
  in the coarsest unit that is still true, on both sides, because a model handed
  an ISO timestamp has to do date arithmetic against a clock nobody gave it.
- **The memory tool is `update_memory` and `update_notes` still parses.**
  `notes` meant memory everywhere the code named it, which is the one ambiguity
  a second store cannot survive. The old name stays as an alias because a model
  that learned Guaca from an older transcript reaches for it, and what is
  recorded is the current name either way. `lib/trail.ts` answers to both,
  because rows written before the rename cannot be migrated into a new spelling.
- **Two sections answer "what am I waiting on", and they are arrived at from
  opposite ends.** *What you are waiting on* is derived from the agent's own
  unanswered sent messages and cannot go stale; the working notes are written
  and cover everything off that path. Each names the other, because nothing else
  would catch them collapsing: both would render, both suites would pass, and
  the cost is an agent spending a bounded list restating one it gets for free.
- **The panel's cap is pinned to `MAX_MEMORY` by a test that reads both
  sources.** It was advisory, on the reasoning that the runtime is what cuts and
  a drifted mirror cannot cost the operator their text. It drifted to 4,000
  against 16,000 and told operators their memory was about to be cut by a runtime
  storing it whole. A warning is read as a fact about what will happen, so the
  number is only worth drawing while it is the runtime's number.
- **A memory read never replaces what the operator is in the middle of
  typing.** The panel refreshes because the agent rewrote the file mid-turn,
  which is exactly when somebody is most likely to be editing it by hand. A
  version that lands under a draft is held to one side, the panel says so, and
  the two ways out are already on screen: Save keeps what you wrote, Discard
  takes what the agent wrote. `arrived` decides that against what is held when
  the read lands, not what was held when it started. Only the runtime's write
  emits `MemoryChanged`: the operator's own comes back from `set_agent_notes`
  as what was stored, and what came back is what goes on screen, because
  `Workspace::write` trims and cuts and the typed version is a page the agent
  is never going to be given.
- **An agent's rewrite is checked against the page it was shown, and loses to
  the operator's save.** A rewrite is the page from the top of the turn with a
  change made to it, and the operator can save the same file from the panel
  while the turn runs. Stored unconditionally, it put back whatever the operator
  had just corrected and told nobody. The turn carries the version it read
  (`Reading::memory` in `runtime/mod.rs`) into `Workspace::write_versioned`;
  on a mismatch nothing is stored and `update_memory` answers with the
  operator's page whole, since the prompt still shows the old one. The version
  then moves to that page, and after a stored write to what was stored, so a
  second rewrite in one turn is checked against the turn's own first one and
  not refused for it. The panel's conflict handling is the mirror of this: it
  protects a draft, and this protects a save.
- **The memory file is replaced by a rename, never written in place.** It is
  the only copy of what an agent knows, and `fs::write` empties the file before
  refilling it, so a crash or a full disk in between left a page cut short.
  `workspace::replace` writes beside it and renames over it, the way the
  notebook and skills already did. The temporary name ends in `.md.tmp`, which
  the id lookup does not match, so a file a crash left behind is never read as
  memory.
- **The history window counts messages the prompt draws, not rows.** Every
  turn that called a tool files an activity record in the agent's channel, and
  one that holds only calls and notices renders as nothing. Counted as rows they
  took about a third of the forty places, so a tool-using agent saw 26 messages.
  `Store::agent_history` now skips what `prompt::is_empty` would skip, in SQL,
  before the limit; a record carrying the agent's commentary is drawn and still
  counts. The two tests of emptiness have to agree, and a store test holds them
  to it part by part.
- **What an agent has already reported is its outcomes read back, not a third
  store.** When a conversation's first message falls out of the history window,
  the agent is shown the line that conversation ended with: its own runs in its
  current group that started before the oldest message the turn draws, history
  or batch, ten at most, newest first (`Store::reported`,
  `prompt::reported_section`). No model call writes them; they are the outcome
  headlines already recorded, so the section costs no more than its own tokens,
  about 400 at its largest. A recurring check that found nothing is left out and
  is not counted toward the ten: it reported nothing, and an hourly one would
  push out every line that matters. A run with nothing left in the agent's
  channel is left out too, because clearing a channel is the operator taking a
  conversation out of what the agent reads back, and its line would put it
  back. Only failed, stopped and interrupted carry a word. A run still waiting
  on the operator reads as done, since its open decision is in the decision
  index the turn is given and its escalation in *What you have already
  escalated*. The section is in the volatile block, last before *Your reply*,
  because its ages move.
- **What a memory rewrite replaced is absent, empty or a page, and the three
  mean different things.** Absent is a call that replaced nothing, which is
  every other tool and every write recorded before the field existed: there is
  nothing to compare and the content is drawn as it always was. Empty is an
  agent's first memory, which replaced nothing because there was nothing, and
  draws as a page that is all new. A truthiness check collapses the first two
  and loses the only write where the whole page is the news. `Part::ToolCall`
  in `domain/envelope.rs`, then `trailStep`.
- **The notebook's index is appended after assembly, and only when it has
  something in it.** `prompt::add_notebook` follows `add_skills`; an empty
  notebook adds nothing, so a group that never uses one sees no prompt change.
- **A notebook path is checked for links, not only for `..`.** `clean_path`
  refuses names that could leave the folder, and `Notebooks::locate` refuses a
  path that runs through a link somebody left there. An agent cannot make a
  link; an operator's stray one must not become a way out.
- **The overnight tidy writes no outcome, and that is what keeps it from
  feeding itself.** Its lines are the agent's outcomes since its last tidy. A
  tidy with an outcome would be a line in the next night's tidy, and a night
  with nothing else would still owe one. It also keeps a tidy out of For you,
  the unseen counts and the outbox without a rule in any of them. *An overnight
  tidy is a turn the clock gives and nobody reads* in `docs/ARCHITECTURE.md`.
- **Two marks on the agent, and they move at different times.**
  `last_tidy_at` is written when the clock hands a tidy over, so a stopped or
  crashed tidy is not given twice in a night. `tidied_through` moves only when
  a tidy finishes or stores a rewrite, so one that failed hands its lines on.
  A tidy whose last save the size limit refused, with nothing stored, has not
  finished.
  Collapsing them into one either repeats a tidy the same night or loses the
  lines a failed one never read.
- **A tidy may not grow a long page, and that is a limit in code, not a
  sentence.** In a tidy `update_memory` refuses a rewrite longer than the page
  the tidy found, or 2,000 characters if that is more. Asked in words to keep
  the page short, a small model still added to it every week; refused, it cut.
  The floor leaves a short page room for the line a tidy came to add. An
  operator's save during the tidy is answered as the conflict it is, before the
  limit, and a normal turn is never limited.
- **The tidy message is not history.** `Store::agent_history` leaves it out
  and so does the prompt's history loop, and the oracle test that holds the two
  together has a tidy among its samples. Drawn later, it would put a stale copy
  of the agent's own notes in front of the fresh ones.
- **A tidy's three tools are checked by what the call is, not by its name.**
  `tools::tidies` matches the parsed invocation, so `update_notes`, the old
  name, is the same tool, and an alias of anything else is still refused.
