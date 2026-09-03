# The calendar

A crew's own dates, and the wall between crews' calendars. `docs/CALENDAR.md`,
then `domain/occasion.rs` and `Runtime::keep_calendar`.

- **The group comes from the agent's card, never from the tool call.** Every
  store call in `keep_calendar` is scoped to `card.group_id`, and an id that
  belongs to another crew comes back as `None`. An id is a thing a model can
  invent, and it invents them under exactly the pressure that makes an invented
  one plausible. A read that trusted an id would let one crew move another's
  board call.
- **A refusal is "no occasion with that id", never "not yours".** The second
  sentence confirms the row exists and hints at whose it is, which is the leak
  the wall exists to stop. Same rule as `no agent named Chef`.
- **The operator's reads and writes have no group on them, and that is not the
  same bug.** `commands::calendar` with no `groupId` crosses every crew. The
  wall stands between crews, not between the operator and their own workspace.
- **An all-day occasion is local midnight plus a flag, and `minutes` is always
  NULL on one.** Midnight alone cannot carry it: it is a real time somebody
  might have chosen, and a filing drawn as `12:00 AM` is a filing nobody reads
  as a deadline. The invariant is settled in `occasion::Clean::new` and nowhere
  else, so a second constructor is a second place for it to be wrong.
- **A length sent with an all-day date is dropped, not refused.** A model that
  sends both means the day, and a round trip to be told so is a round trip
  wasted. A length that is absurd (`90000` for "an hour and a half in seconds")
  *is* refused, because a dropped one reads back as an occasion with no length
  and the model never learns.
- **Every write is answered with `Occasion::describe`, and that is the safety
  mechanism on a date rather than the parser.** A wrong date is the one argument
  a model gets wrong silently: nothing about `2026-09-14` when it meant the 15th
  looks like an error. Told back the local wall clock that was stored, the model
  catches it in the same turn. This is also why a stated timezone is honored
  rather than refused.
- **`## Your crew's calendar` is the only place the prompt says what day it
  is.** A model has no clock, and an agent asked to put "next Thursday" on a
  calendar guesses without one. Do not take the date out of that section without
  putting it somewhere the calendar can still reach.
- **Every calendar line carries the wall clock *and* the distance.** A model
  given only `Mon 14 Sep 2026` computes "in how long" and gets it wrong; an
  operator given only "in 2 days" cannot act on it. Both, always.
- **Nothing on this calendar fires, and folding it into `routines` is wrong in
  both directions.** Give an occasion a trigger and every note about a
  customer's schedule becomes an agent running at 3am; drop the calendar and
  every deadline becomes a wake-up. Two tables, two tools, two prompt sections,
  each naming the other.
- **The word "calendar" carries the assumption that writing on it arranges
  something.** Observed: a model wrote an occasion and told the operator the
  call was booked. *Books nothing, invites nobody* is in the tool description,
  the prompt section and the panel's footer, and all three are load-bearing.
- **An occasion's crew never changes.** There is no call that moves one and the
  editor's crew field is disabled on anything that exists. Moving one would move
  it out from under the agents that keep it with nothing on screen saying so.
- **`Store::delete_group` deletes occasions explicitly.** The foreign key is a
  plain `REFERENCES groups(id)` like every other group-scoped table here, and
  these keys are enforced: a row left behind refuses the delete rather than
  dangling.
- **A deleted agent's occasions stay.** Its memory, schedule and working notes
  go with it because those are the agent's own. A board call does not stop
  happening because the agent that heard about it was let go, and the calendar
  it is on belongs to the crew. `Runtime::purge_agent` names every store it
  clears and deliberately does not name this one.
- **`daysIn` walks a `Date`, and `monthOf` sets the day before the month.**
  Stepping by 86,400,000 loses the 23-hour day and doubles the 25-hour one, so
  March comes out at 30 days. Setting the month first walks the 31st of January
  into the 3rd of March, which is how a "next month" button skips one.
