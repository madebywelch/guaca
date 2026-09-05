# The runtime

What a turn may read while it runs, what a stop does, what lands on a run's
bill, and the shapes the store keeps all of it in. `docs/ARCHITECTURE.md` is the
long version; `runtime/guard.rs`, `runtime/mod.rs` and `domain/group.rs` are the
code.

- **A group's settings are two blocks, and each is all-or-nothing.** A draft
  that mentions `inference` or `limits` replaces every override in that block,
  and one that leaves it out changes none of them. Per-field "absent means leave
  alone" would let a caller half-write a crew's settings, and would make a UI
  that renders what it read back clear a field by forgetting to mention it. The
  API key is the one exception and has the opposite rule (absent keeps the
  stored key), because it is the one setting that cannot be read back.
- **Every setting a group can hold is `None` when it is inherited, all the way
  down.** In the draft, in the column, and in the resolved overrides. An empty
  string is a field an operator blanked, which also means inherit, and it is
  normalized to `None` on the way in so the two can never disagree.
- **A thread between two agents is `pair_messages`.** A send is filed under the
  recipient and the answer under the sender, so a thread assembled from one
  channel's rows is missing messages nobody can account for.
- **An agent's prompt reads `agent_history`, not its channel.** Automatic
  replies are filed under the recipient; reading only the sender's channel
  made it forget what it had answered. Its own completed work remains visible
  even if the next question queued before that work finished. Incoming history
  is cut before its size is bounded, so a busy inbox cannot evict older context.
- **Reserve the next model call before taking in new work.** Intake consumes
  the envelope and releases its run. An exhausted turn that takes it first
  marks work handled without ever showing it to the model. It must stay queued
  and run against its own budget instead.
- **`ToolStarted` is emitted before the call, and that ordering is the
  feature.** A `run_command` can sit for a minute. A call reported only once it
  comes back is silence for exactly as long as the wait it was meant to explain,
  which is the state that reads as a hang.
- **`ToolFinished` carries the whole `Part::ToolCall` and not its outcome.** The
  chip a turn draws while it runs and the chip the transcript draws afterward
  are then one function over one value, rather than two that agree on the day
  they were written. The outcome alone was enough until `replaced` arrived, at
  which point a live memory rewrite silently stopped opening as a diff while the
  recorded one still did.
- **A turn ends when the model stops calling tools, so a closing promise is
  silence.** "Checking both properly" was the last thing that happened in a turn
  with a working plugin, two calls left to make and rounds and budget to make
  them in: the loop broke on an empty `tool_calls`, the message was filed, and
  the operator waited for a check that was never going to run. Three things
  answer it and none of them is enough alone. The prompt states the mechanism
  rather than a rule, because "do not announce work" is something a model talks
  itself out of and "nothing of yours runs after this message" is not. The
  runtime gives such a turn one more round, once, claimed from the same budget.
  `eval` counts it afterward as `PromisedAndStopped`, which is the only half CI
  can fail on, and the only reason a prompt change here is measurable at all.
  The rule itself is `domain::promise`, in one place because a copy in the
  runtime and a copy in the eval drift into a prompt fix that measures clean
  while the runtime goes on nudging.
- **The nudge is not gated on an empty tool trail, and is gated on `code`.** The
  turn this was written for had already made two calls and still closed on a
  promise about two more: what backs a sentence is a call made *before* it, not
  anywhere in the turn. The one exemption is a started `code` job, which
  genuinely outlives the message announcing it, and which `## Your repository`
  explicitly tells an agent to announce. Without that exemption the nudge fires
  on the one announcement the app asks for and every coding job buys a wasted
  model call.
- **A turn reads its inbox between rounds, and reads nothing that would change
  where its answer goes.** Those are one decision, not a rule with an exception
  bolted on. The mode, the reply target, the placeholder's channel and `cause`
  are all fixed before the first token and the UI has been drawing them since,
  so what a running turn may take in is exactly what it could already answer
  from where it stands: the operator and Guaca, into a turn that is writing to
  this agent's own channel. `ToPeer` takes in nothing, because that turn's
  answer is addressed to the agent that asked and an operator message read there
  would be read and never answered. A peer's message waits for the same reason
  from the other end: it is a hop with a reply owed, and answering it as a
  footnote to somebody else's turn is not an answer. `Runtime::take_in`.
- **Text a `ToPeer` turn trails after answering its peer belongs to the operator
  or to nobody, and which one is `Store::operator_addressed`.** Dropping all of
  it was right for the case it was written for and wrong for the manager. Seven
  workers each writing "I replied to the Chief of Staff" is mail from a
  conversation the operator was never in; the same turn shape on the one agent
  they *did* write to is the report, and that agent has no other way to send
  one. Its first turn is `ToOperator` and every turn after it is woken by a
  peer, so `ToPeer` is the only mode it is ever in again, `send_message`
  resolves agent names only, and a file attached afterward routes to the peer
  too. Measured on a real crew of four: twenty-seven minutes, 499 model calls,
  ten reports written and addressed to the operator by name, none delivered, and
  the operator's own message in the middle of it asking why nobody was working.
  The rule is per run in both directions, so a manager is not owed a report on a
  run the operator started somewhere else. `emit_reply`, and the pair of
  cascade tests either side of the carve-out.
- **A turn remembers what its own prompt already says, and intake renders
  against that.** `deliver` writes to the store before the inbox, so a message
  queued behind the one being answered is in the history the turn just read and
  in the inbox at the same time. It is consumed, released and answered by this
  turn either way; without the set it is also written into the prompt twice, and
  an instruction a model reads twice is one it was told twice.
- **`code` not blocking is what made a running turn need to read its inbox at
  all.** The job's answer comes back as a fresh envelope on a run of its own, an
  actor only examines the envelope it is holding, and the turn that asked was
  therefore the one thing that could not receive it. `RepositoryBusy` then told
  that agent to wait for the message its own turn was holding up, which is an
  instruction nothing can follow: measured on a real crew, forty-five minutes
  and forty-five model calls of a turn filling time, with three finished jobs
  and an operator correction stacked behind it. Neither half was wrong on its
  own, which is why it survived review twice.
- **An envelope booked against a run is released by whatever consumes it.** A
  path that takes one without turning it into a turn leaves the run outstanding
  for the life of the process.
- **A stop marks a run and releases nothing.** `track_inflight` reads a negative
  delta against a run it is no longer counting as that run reaching zero, and
  emits a second `RunSettled` for it. So a stop that helpfully released the
  envelopes it was ending would report the run finished twice, which fails the
  trajectory suite and double-counts its spend. Marking is the whole mechanism:
  each of the three boundaries releases through `finish_turn`, exactly as an
  ordinary turn does.
- **The stop check sits before `reserve_step`, not after it.** One line later and
  a run reports a model call that a stop prevented, for the rest of its life.
- **The checks inside the round loop do not cover the way out of it.** A turn
  whose last call returns text and no tool calls leaves by the break at the
  bottom, so there is a fourth check after the loop and before the reply is
  decided. Without it a stop that landed during a single-round reply turn — the
  whole turn, in that shape — still wrote to the peer that was waiting.
- **An actor only ever examines the envelope it is holding.** A paused agent
  parked on one run therefore cannot see that another run's work is queued behind
  it, which is why the pause park drains the queue whenever anything is stopped.
  Survivors go to a holding queue and stay counted in `depth`.
