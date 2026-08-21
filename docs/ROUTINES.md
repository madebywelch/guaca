# Routines

A routine is an instruction an agent gives itself when a trigger comes due.
`domain/routine.rs` holds the shape, the scheduler is in `runtime/mod.rs`, and
`RoutineList` and `RoutineDetail` draw them.

## A repeat is a shape, not a number of seconds

`every weekday` and `every month` cannot be gaps, and `every day` should not be
one: a day is 23 or 25 hours twice a year, so a daily nine o'clock routine
stored as 86400 seconds drifts to eight and stays there. `domain/routine.rs`
holds the shape and `next_run_at` holds the hour, and the next slot is computed
in local time from the slot it was due at rather than from the moment it ran, so
a machine asleep through three of them fires once on waking instead of three
times. A gap still exists, because an agent scheduling itself works in seconds
and nothing shorter than a day has an hour to keep.

## A trigger is one string in both places it lives

The column is text and so is the wire form, which is why `Trigger` has a
hand-written `Serialize`: a derived one would hand the webview a tagged object
and SQLite `weekdays` for the same fact, and only one of the two would be read
by the frontend. Text also means the trigger after these, a connector event, is
a new value rather than a new column.

## Switching a routine off and editing one are different actions

`active` has its own command, acts on the click, and does not move
`next_run_at`: a routine turned back on is due at the slot it was already
holding, and the scheduler fires an overdue slot once. Parking it behind a Save
the operator has not pressed means a routine they think they stopped still runs.

## A test run is the scheduler's own path with the schedule left alone

Same delivery, same fresh run, so what the button shows is what Tuesday will do.
It deliberately does not move `next_run_at` or delete a one-shot, because trying
a routine out must not spend the only firing it had. It is refused while the
draft is dirty: firing the saved version while the operator is looking at an
edited one answers a different question and reads as the edit having done
nothing. Both kinds are recorded in `routine_runs` and the test is marked,
because in the transcript the two are identical.

## A routine's row is one line and its instruction is not in it

The instruction is written to be acted on with no other context, which is
several sentences, and drawing it as the title made one routine fill the panel.
The row is a name and a cadence; opening it gives the panel over to that
routine. An agent naming its own routine is optional, so `routineTitle` cuts the
instruction down when nobody named it, on a word boundary and after the first
sentence.

## What a routine delivers is work, and the runtime is what sent it

It arrives from neither the operator nor a peer, which is the case the reply
mode originally had no arm for: every schedule an agent kept was answered by an
agent that had just been told nothing was being asked of it. Anything carrying
work is `ReplyMode::Assigned`, whoever sent it. See *Cascades terminate because
of one asymmetry* in `ARCHITECTURE.md`.
