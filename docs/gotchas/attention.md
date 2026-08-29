# Attention

Everything an agent stops to ask a person, and the one thing it raises without
stopping. `docs/ATTENTION.md`, then `domain/approval.rs`,
`domain/escalation.rs` and `Runtime::park`.

- **An escalation is not an approval with the parking taken out, and folding
  it into one would break the half that matters.** Both mean the operator, and
  everything else about them is opposite. A request stops a turn mid-flight to
  get something back, holds a run booking while it waits, and lapses after ten
  minutes because holding one costs money. An escalation is a turn that has run
  out of road saying so on its way out: nothing parks, so nothing expires, so a
  row can sit for the two days it has actually been true. Given ten minutes it
  would lapse before the operator got back from lunch, having spent a booking
  proving that a broken tool chain was still broken. `docs/ATTENTION.md`.
- **`raised_at` never moves and `times` only goes up.** An agent that hits the
  same wall on six turns raises six times and holds one row. Refreshing the
  stamp is the obvious implementation and it destroys the only thing the row is
  worth: *stuck since Tuesday, six turns into it* is a fact nothing else in the
  app can see, and *stuck just now* is what the message in the channel already
  said five times. Same argument a working note's stamp is built on, one store
  over, and the same reason nothing lets an agent withdraw its own escalation:
  it would take the record of a lost fortnight with it at the moment it decided
  things were fine.
- **`Store::raise_escalation` takes an immediate transaction and a deferred one
  would look identical until it did not.** The read and the insert are one
  decision, and an agent writes from every thread it holds. Deferred, two turns
  both read "nothing open", both try to insert, and the second is refused with
  `SQLITE_BUSY_SNAPSHOT`, which no busy timeout can wait out because the
  snapshot it read from is already stale. The failure lands inside a turn that
  had already given up on getting anywhere, which is the worst place in the app
  for one. The test is the eight-thread one beside it.
- **A question and a permission are two of everything, and folding them into
  one shape breaks the turn quietly.** Two `Part` variants, two cards, two
  commands, two `ApprovalState`s. The line is what a yes does: a permission
  authorizes something the agent could not otherwise do, and a question hands
  back a value that authorizes nothing and passes through every guard the agent
  already had. That is the whole reason a question may draw the model's own
  words on a button, which happens nowhere else in this app. It is also why a
  verdict on a question is refused before the row moves: `ask_question` reads
  the answer back off the row, so an Allow would settle it with nothing in it
  and the turn would resume having been told nothing at all. Underneath they
  are one row, one waker and one timeout, in `Runtime::park`.
- **`Part::Approval` was not widened to carry the question, and that is not
  timidity.** Parts are stored as JSON, so renaming a field on one breaks every
  historical transcript that contains it: the message fails to parse and the
  channel is gone, for a request answered a year ago. A new variant costs
  nothing, because no old row can contain one.
- **The `approvals` table has no `kind` column.** `action` already
  discriminates: a question stores the literal `question` there, which is not
  one of the two protected actions and which `ProtectedAction::parse` refuses. A
  second column would be a value that has to agree with the first with nothing
  keeping them in step. Both halves are decoded in `row_to_approval` and
  encoded in `create_approval`, and nowhere else.
- **The desk's queue is a read, not a list events are added to and removed
  from.** Same argument as the menu bar's presence, same failure if it is
  ignored: a queue assembled from `approvalRequested` and `approvalSettled` is
  one dropped event away from offering a decision that reaches nobody, and a
  stale card looks exactly like a live one. Both events invalidate it and the
  answer comes back from `pending_approvals`. The consequence is the feature:
  nothing can appear on the desk that is not a row somewhere.
