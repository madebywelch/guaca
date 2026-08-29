# Routines

Schedules, triggers, and what a firing looks like when it lands on an agent that
is already working. `docs/ROUTINES.md`, then `domain/routine.rs` and
`Runtime::sweep_schedule`.

- **A fired routine carries `Part::Routine`, not text.** The instruction reaches
  the model either way, because `as_plain_text` returns it. The part is what
  keeps the transcript from drawing a schedule firing as Guaca talking to the
  operator: `docs/ROUTINES.md`.
- **`Routine::next_run_at` is an `Option` because some triggers have no next
  run.** A sentinel date would be shown to the operator, and it is one bad
  comparison away from firing something that was waiting on a connector.
- **A skipped firing advances the slot, and is refused on anything that does not
  repeat.** Both follow from skipping being a drop rather than a deferral: an
  hourly sweep held back until the agent goes quiet fires the moment it does,
  which is the pile-up in a bunch instead of one at a time. Advancing is also
  why the pair is refused on a one-off, whose slot is the only one it has: it
  would be deleted having done nothing, and the operator would find an empty
  list where their alarm was. The panel hides the tick there rather than relying
  on the refusal, because a control nobody can see is a save that fails for no
  visible reason. `validate`, then *A firing can be skipped* in
  `docs/ROUTINES.md`.
- **`routine_runs.run_id` is nullable, and a skip is a row in that table.** Two
  failures read alike and have different fixes. A firing that leaves no trace is
  a gap, and a gap is what a scheduler that has stopped working looks like; a
  skip filed under an invented run id reads back as a delivery that bought no
  model calls, which is the case that history was built to surface. Only the row
  saying which it was tells them apart, so the id is absent rather than made up.
- **An agent's standing routines are in its prompt, and that is not a duplicate
  of `schedule` with `list`.** A list behind a tool call arrives after the model
  has decided what to do, so an agent asked to change something it already keeps
  wrote a second routine beside the first and reported the change as made. Both
  fired. `docs/ROUTINES.md` before you take the section out, and note that
  `update` is what the ids in it are for.
