# Routines

A routine is an instruction an agent gives itself when a trigger comes due.
`domain/routine.rs` holds the shape, the scheduler is in `runtime/mod.rs`, and
`RoutineList` and `RoutineDetail` draw them. `lib/routine.ts` is the webview's
half: the only file on that side that reads a trigger's stored text.

## A trigger is one string, in both places it lives, and two kinds of thing

The column is text and so is the wire form, which is why `Trigger` has a
hand-written `Serialize`: a derived one would hand the webview a tagged object
and SQLite `weekdays` for the same fact, and only one of the two would be read
by the frontend.

Underneath that one string are two families. A `Cadence` is a moment or a repeat
of them, and the scheduler owns it. An `EventTrigger` is something happening in
a service the group is connected to, written `event:stripe/invoice.payment_failed`,
and it owns nothing: it has no next moment, and no clock will ever produce one.
Splitting them in the type is what keeps `next_after`, `first_run` and `accepts`
off a value that has no answer for any of them.

**Nothing delivers an event yet.** What exists is every layer under it: the
value parses and round-trips, the row stores, the scheduler leaves it alone, the
panel and the list draw it, `Test run` fires it, and the history records it. What
is missing is the one part that cannot be written without a service to receive
from, a webhook or a poll that turns an arriving event into
`Runtime::send_from_routine`. It is deliberately not in the trigger picker
either: a routine an operator can set and then watch never fire is worse than
one they cannot set yet.

## A routine with no next firing has no next firing

`next_run_at` is `Option<i64>`, and NULL in SQLite since migration 21. The
alternative was a sentinel date, which is a date the operator eventually gets
shown, and one bad comparison away from firing something that was meant to wait
for Stripe. NULL also tells the scheduler for free: SQL compares NULL to
nothing, so `next_run_at <= now` skips these without `due_routines` knowing what
kinds of trigger exist.

Everything reading that column has to answer for the empty case rather than
invent a moment. `NextSlot` is the shape of that: `Due`, `Waiting` and `Done`
are three answers, not two, because "nothing on the clock" and "finished" mean
opposite things to the row and reading them off the same `None` deleted every
event routine the first time it fired. `ORDER BY next_run_at` needs the same
care: NULL sorts first in SQLite, so a routine waiting on an event drew above
one firing in ten minutes.

Migration 21 rebuilds the table, which is the only way SQLite will drop NOT
NULL. `migrations::run` turns foreign key enforcement off around the whole
sequence for it: with enforcement on, `DROP TABLE routines` performs an implicit
delete first and fires `routine_runs`' `ON DELETE CASCADE`, taking every
recorded firing with it.

## A repeat is a shape, not a number of seconds

`every weekday` and `every month` cannot be gaps, and `every day` should not be
one: a day is 23 or 25 hours twice a year, so a daily nine o'clock routine
stored as 86400 seconds drifts to eight and stays there. `Cadence` holds the
shape and `next_run_at` holds the hour, and the next slot is computed in local
time from the slot it was due at rather than from the moment it ran, so a
machine asleep through three of them fires once on waking instead of three
times. A gap still exists, because an agent scheduling itself works in seconds
and nothing shorter than a day has an hour to keep.

## The moment is the only record of which day a repeat keeps

A weekly routine keeps the weekday of its first firing and a monthly one keeps
the day of the month. Neither is stored anywhere else, and for a while neither
was askable: the panel offered a time of day, so "every week at nine" landed on
whichever day the operator happened to be at the keyboard, and nothing on screen
said so. `anchorFor` in `lib/routine.ts` says which part of a moment each trigger
actually keeps, and `firstRunDelay` turns what the operator picked into the one
number the backend takes.

Two rules in there are not obvious. A monthly 31st goes to the next month that
*has* a 31st rather than clamping to the end of a short one, because clamping
would anchor the routine on the 28th and every firing after it would inherit
that day: the walk backwards down the calendar `months_after` is careful to
avoid. And a moment already gone is refused rather than sent, because a negative
delay reaches the scheduler as a routine overdue and fires on the next tick,
which is not what picking a date means. A one-off takes a date for the same
reason: a time on its own can only ever mean the next 24 hours.

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

## A firing that spent nothing is a routine that did not run

`routine_runs` records that a firing happened. What the operator actually wants
to know is whether it worked, and a delivery to an agent that never took a turn
is indistinguishable, row for row, from one that did the job. So the history
joins `usage` by `run_id` and every firing carries what it bought: `calls: 0` is
the one worth seeing.

Joined at read time rather than stored on the row, because a firing's cost is
not known when it is recorded and keeps moving until the run settles. A column
would be a snapshot of a number that was still changing, and the model calls are
already filed under the run id.

## A routine's row is one line and its instruction is not in it

The instruction is written to be acted on with no other context, which is
several sentences, and drawing it as the title made one routine fill the panel.
The row is a name and a cadence; opening it gives the panel over to that
routine. An agent naming its own routine is optional, so `routineTitle` cuts the
instruction down when nobody named it, on a word boundary and after the first
sentence.

The end of that second line is where the row is honest about what it is looking
at. A switched-off routine must not claim a next firing, and one waiting on an
event has no next firing to claim, so both say when they last ran instead: on a
routine that is not about to do anything, that is the only news left.

## A firing is a line in the transcript, not a message

The instruction has to reach the model, and it does: the envelope carries
`Part::Routine`, `as_plain_text` returns the instruction, and prompt assembly,
dedup and the emptiness check are all unchanged. What the part buys is the
drawing. A firing used to arrive as a chat bubble from "Guaca" carrying all
several sentences of it, in the middle of the operator's own conversation with
their agent: the system prompting the agent, in the shape of somebody talking to
the reader. The reflex is to read it as addressed to you, and it never is.

So it is one chip naming the routine, and the click opens that routine in the
panel beside the transcript, where the instruction is the thing you came to
read. The part carries the routine's id and its name *at the time it fired*, so
a routine since renamed does not rewrite what the transcript said it was. The
two ends are in different columns of the window, so the click goes through
`openingRoutine` in the store rather than a prop threaded through every message,
which is the same arrangement `focused` already uses for search hits.

Old transcripts still hold text parts and still draw as bubbles. Migrations are
forward-only and a message is a record of what happened; rewriting one to change
how it looks is not worth being able to say the record was edited.

## What a routine delivers is work, and the runtime is what sent it

It arrives from neither the operator nor a peer, which is the case the reply
mode originally had no arm for: every schedule an agent kept was answered by an
agent that had just been told nothing was being asked of it. Anything carrying
work is `ReplyMode::Assigned`, whoever sent it. See *Cascades terminate because
of one asymmetry* in `ARCHITECTURE.md`.
