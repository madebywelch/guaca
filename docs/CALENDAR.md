# The calendar

Dates a crew is answerable for. `domain/occasion.rs` holds the shape, the
`calendar` tool and `Runtime::keep_calendar` are how an agent writes it,
`lib/calendar.ts` decides what the view shows, and `Calendar.tsx` draws it.

## It exists because three stores were being reached for and all three are wrong

An agent that learns a filing is due on the 15th, that a customer moved a call,
or that a contract lapses at month end has to put it somewhere. Before this it
had three choices.

Memory is what the agent *knows*, rewritten whole and read every turn. A date
put there survives the date: `Rewriting` is the cheapest moment to copy a stale
line forward and the most expensive moment to decide it is stale, so a meeting
that happened in March is still in the file in June. A working note expires on
its own, which is right, but it carries no moment and is one line about *right
now*: "waiting on the board call" is a note, and "the board call is on the 14th
at three" is not one. And a routine is work the agent will do, on a clock that
fires. Written as a routine, a deadline wakes an agent up at midnight to be told
about itself.

None of the three can be read as "what is coming", which is the one question a
calendar answers and the one an operator asks. So an occasion is one fact:
something is happening, at a time, and somebody here is answerable for it.

## It is not Google Calendar, and the distinction is drawn in three places

Guaca has a Google plugin, and its calendar tools read and write the operator's
real calendar, with real invitations on it. This is not that. Nothing here
leaves the machine, nothing is booked, nobody is invited, and there is no
attendee list because there is nobody to attend.

That has to be said out loud three times, because the word "calendar" carries
the assumption on its own. It is in the tool description (*a record, not an
arrangement*), in the prompt section (*nothing here books anything*), and once
at the bottom of the panel. The failure it prevents is specific and was
observed: told it had a calendar, a model wrote "Call Priya, Tue 3pm" onto it
and reported to the operator that the call was booked.

## It fires nothing, and that is the difference from a routine

The two are the lists an agent keeps and they are one letter apart in a model's
reading of them. A routine fires: it reaches the agent as a message in a fresh
run with a fresh budget. An occasion sits there being true.

Folding them together is the obvious simplification and it is wrong in both
directions. Give the calendar a trigger and every note about a customer's
schedule becomes an agent running at 3am, on the operator's money, about
something that needs nothing from it. Take the calendar away and tell agents to
use routines, and every deadline becomes a wake-up.

So they are two tables, two tools and two prompt sections, and each section says
what the other is for. An agent that has to prepare for something on the
calendar writes both: the occasion here, the routine there.

## A crew's calendar, and one crew cannot touch another's

The wall is the same one the message bus keeps and it is enforced the same way:
in the store, in the WHERE clause, from the group on the calling agent's own
card. Nothing an agent sends is ever the group.

It has to be enforced there rather than trusted, and the reason is narrower than
"defense in depth". An id is a thing a model can invent, and it invents them
under exactly the pressure that makes an invented one plausible: asked to move a
meeting it cannot find, it will try an id it half-remembers. An invented id that
happened to land on another crew's board call would move it.

**A refusal is "not found", never "not yours".** `Store::update_occasion` and
`Store::delete_occasion` match on the group and come back empty, and
`keep_calendar` says *your crew has no occasion with the id X*. The second
sentence — "that belongs to another crew" — confirms the row exists and hints at
whose it is, which is the leak the wall is for. Same reasoning as
`no agent named Chef` in the directory.

The operator stands above that wall. `commands::calendar` with no `groupId` is
the one read in the app that crosses every crew at once, and the three write
commands take an id with no group on it. This is not an inconsistency: the wall
is between crews so that one crew's agents cannot reach another's, not between
the operator and a workspace they own.

An occasion's crew never changes. There is no call that moves one, and the
editor's crew field is disabled on anything that already exists: moving an
occasion between crews would move it out from under the agents that keep it,
with nothing on screen saying so.

## One instant column, two kinds of date

A deadline has a day and no time on it. A call has both. Stored as two shapes
they need two columns, two indexes and a branch in every `ORDER BY`; stored as
one instant they collapse, and that is what `all_day` buys.

An all-day occasion holds **local midnight of its day** with `all_day` set.
Everything sorts, ranges and indexes on one `starts_at`, and a deadline still
sorts above the nine o'clock on the same day. The flag is what stops midnight
being read as a time: it is a real time somebody might have chosen, and a filing
drawn as `12:00 AM` is a filing nobody reads as a deadline.

`minutes` is the other half. `NULL` is a moment with no stated end, which is
most of what lands here — a contract lapsing has no duration and inventing one
would draw a bar for a thing that is not a meeting. On an all-day occasion it is
always `NULL`, and that invariant is settled in exactly one place,
`occasion::Clean::new`, which is the only way to build a row for writing. A
model that sends both means the day, so the length is dropped rather than
refused: a round trip to be told something nobody would disagree with is a round
trip wasted.

## A model has no clock, so the prompt tells it the date

`## Your crew's calendar` opens with today's local date, and it is the only
place in the whole prompt that says what day it is. That is deliberate rather
than an oversight: it is stated in the one section that needs it, beside the
dates it is used to compute. An agent asked to put "the board meeting next
Thursday" on the calendar has to know what today is, and a guess produces a date
nobody catches.

Reading is the other half, and it is handled by not requiring arithmetic at all.
Every line carries both the wall clock and the distance: `Mon 14 Sep 2026, 3:00
PM — in 2 days`. The absolute half is what a person acts on and the relative
half is what a model reasons with, and a model given only the first computes the
second and gets it wrong. Same argument `worknote::how_long_ago` makes, pointed
the other way.

## Every write is answered with the date that was stored

`Occasion::describe` renders the answer to `add` and `update`, the lines in
`list`, and the lines in the prompt. One rendering, and that is load-bearing.

A date is the one argument a model gets wrong *silently*. `2026-09-14` when it
meant the 15th, `15:00Z` when it meant three in the afternoon local: nothing
about either looks like an error, and no refusal can catch them because both are
valid. Told back the local wall clock that was stored, the model reads its own
mistake in the same turn it made it. That is also why a stated timezone is
honored rather than refused — the parser cannot be the safety mechanism, so it
may as well accept what is unambiguous.

`parse_when` accepts what people and models actually write: `2026-09-14 15:00`,
the ISO `T`, optional seconds, `3:00 PM`, `9am`, a bare date for a whole day,
and an explicit offset or `Z`. What it refuses is anything it would have to
guess at, including an impossible date: `2026-09-31` is an error rather than the
1st of October, because a silently moved date is one nobody checks.

## What the operator sees is every crew, a month at a time

One surface, reached from the rail's footer, drawn like the cafeteria and the
compost because it is the same kind of thing: somewhere you go, look at the
whole workspace, and come back from. A pane would have to live inside a crew,
which is the framing this exists to escape.

**Every crew by default, filterable to one.** The crew chips are built from the
groups rather than from the occasions, so a crew with an empty calendar is still
something you can pick: a chip that appeared only once somebody wrote a date
would be a filter you cannot use until you no longer need it. Each row says
which crew it belongs to whether or not the list is filtered, because the crew
is why one crew's agents cannot touch it.

**Days, not a list.** A flat list sorted by date is what the store hands back
and reads as a feed: nothing in it says where Tuesday ends. `daysIn` groups into
local days, walking `Date` rather than adding 86,400,000 so the two days a year
that are not 24 hours long neither double nor vanish, and only the busy days are
drawn. Within a day, all-day occasions come first: they are what the day *is*,
and sorted against timed ones by `startsAt` a deadline at local midnight reads
as the first appointment of the morning.

**A month, not a rolling window.** "Show me October" has an answer and "the next
thirty days" slides under the operator every morning. It opens on this month
*and the rest of the next*, which is what stops a calendar opened on the 29th
from being a calendar showing two days.

Nothing is hidden once it has happened. `isPast` draws a row back rather than
dropping it: something that happened this morning is still what today was, and a
calendar that emptied itself as the day went on would be a calendar that lies
about the day.

## What is not here

**No recurrence.** "Every Monday at ten" is four words to say and a field that
turns one row into a series with exceptions, a rewrite rule, and a question
about what `cancel` means. Nothing has needed it, and the honest way to keep a
standing meeting on this calendar today is a routine that adds the next one.

**No invitations, no attendees, no reply state, no reminders.** The first three
have nobody to be about. The fourth is `schedule`, which already exists and
already does it better than a flag on a row would.
