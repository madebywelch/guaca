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
- **What a memory rewrite replaced is absent, empty or a page, and the three
  mean different things.** Absent is a call that replaced nothing, which is
  every other tool and every write recorded before the field existed: there is
  nothing to compare and the content is drawn as it always was. Empty is an
  agent's first memory, which replaced nothing because there was nothing, and
  draws as a page that is all new. A truthiness check collapses the first two
  and loses the only write where the whole page is the news. `Part::ToolCall`
  in `domain/envelope.rs`, then `trailStep`.
