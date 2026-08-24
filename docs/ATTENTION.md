# Attention

What happens when an agent needs a person, and how a person finds out.

Guaca runs unattended. Routines fire, cascades run, and every so often a turn
stops and cannot go on until the operator deals with it. A workspace with one
crew of four made that easy: the row went amber and the card was in the channel
you already had open. A workspace with a dozen crews of six does not, and this
file is about the three surfaces that answer it, what each is for, and the
things every one of them refuses to do.

## Three tiers over one queue

They answer three different questions and none of them answers another's.

- **The count on a crew's circle** says *where*. It is on screen wherever the
  operator is, it is the only permanent statement in the app about the crews
  they are not looking at, and it cannot be answered from. *A group is a place
  you can be inside* in `docs/WORKSPACE.md`, then `src/lib/presence.ts`.
- **The desk** says *what*, and takes the answer. Bottom right, over whatever is
  there, absent almost all the time. `src/components/Desk.tsx`.
- **The card in the transcript** is the record, and the way into the
  conversation the request came out of. It is where a decision that needs its
  context is made. `ApprovalRequest.tsx` and `QuestionRequest.tsx`.

And a fourth that is not part of the queue and is often confused with it: the
menu bar, which is the state of things while the window is shut. *The menu bar
is Guaca with the window shut*. A notification is a thing that happened; the
menu bar is the state of things when you are away; the desk is the state of
things while you are looking. Three questions, and each surface answers exactly
one of them.

## What earns a place on the desk

One rule: **it holds stopped work, and only the operator can start it again.**

A parked turn qualifies by definition. That is the whole list today, and the
rule is what keeps it that way. A run that failed does not qualify, because
nothing is waiting on anybody and a failure is understood in the channel it
happened in. Neither does a run that finished, a routine that fired, a paused
agent, or a workspace with no API key: that last one is a precondition rather
than a request, it already has a banner at the top of the reading column where
somebody setting up is looking, and moving it into a panel that can be collapsed
would make first-run setup quieter for no gain.

Three more refusals, each of which is a thing this could easily become.

**It is usually absent.** No queue, no surface: not a small empty one, not a
collapsed bar. A panel that is always there is furniture within a week and
furniture is not read. Being gone almost all the time is what buys the corner of
the screen.

**It has no composer and no scrollback.** Every control on it is bounded, and
what has been answered is gone from it. The transcript is the record. A second
record that could be scrolled back through would eventually disagree with the
first, and nothing would say which was lying.

**It never takes focus.** A request can land while the operator is mid-sentence
in a composer. It announces itself once, politely, through the line that is
already on screen rather than through a second copy of it offscreen, and waits.

## The queue is read, not accumulated

`pending_approvals` is the truth and both approval events invalidate it. Nothing
is appended on `approvalRequested` and removed on `approvalSettled`, which is
the obvious implementation and the wrong one: it is one dropped event away from
offering a decision that reaches nobody, and a stale card is indistinguishable
from a live one. It is the same discipline `menubar::plan` is built on, for the
same reason, and it is why the desk cannot show anything that is not a row
somewhere.

The consequence is worth stating: anything that wants a place on the desk has to
become durable state first. That is a feature.

One request is live on two surfaces at once, so answering is one action in the
store rather than one call in each card. A refusal is the runtime saying it was
already answered or that it lapsed while it was being read, and the runtime's
copy is the truth: it is taken rather than argued with, and both readings of it
are corrected together. Nothing is believed until the read comes back, so a card
whose answer was refused is still there and still answerable.

## Two things an agent can stop to ask, and the line between them

Before this there was one, and it was "may I". An agent that needed a judgment
call had no way to ask for one: it wrote the question into a channel nobody was
watching and either guessed or stalled. In a workspace with a dozen crews that
is the common case, and it is invisible, because nothing parks and nothing is
counted. You find out when the work comes back wrong.

So there are two kinds of request, and the line between them is what a yes does.

**A permission authorizes.** The agent could not do the thing; the operator's
answer is what lets it. Every word on one of these is Guaca's, because an agent
that could write its own request could describe creating an agent as tidying up.
`request_permission`.

**A question informs.** The agent could have gone either way and does not know
which way is wanted. The answer is a value it carries into the rest of its turn,
and whatever it does with it passes through every guard it already had.
`ask_operator`.

That distinction is what makes it safe to draw a model's own words on a button,
which happens in exactly one place in this app and only for a question. Nothing
answered there grants anything, so no wording on it can talk an operator into
granting something. It is also why they are two `Part` variants, two cards, two
commands and two `ApprovalState`s rather than one shape with a flag: a card that
offered Allow and Deny for "which vendor" would settle the row saying nothing at
all, and the turn would resume having been told nothing.

Both park the same way. One table, one row, one waker, one stop check, one
timeout, one read back: `Runtime::park`, with `ask_permission` and
`ask_question` as the two ways in. A second copy of that machinery would be a
second place for a turn to be left parked forever.

**A question is counted in the menu bar and cannot be answered there.** Its
answer is a word the operator picks or writes and a menu item is a thing you
click; the shapes do not meet. So it appears as a row that opens the channel.
Left out entirely it would still be in the title's count, and the operator would
open the window looking for a request the menu had not mentioned.

## What bounds the asking

Nothing new, and that is deliberate. An agent physically cannot ask twice at
once, because the turn is parked inside the tool call. Asking repeatedly across
rounds costs a model call each time, so the run's step budget and the group's
`maxToolRounds` already bound it. A per-agent guard on top of those would be a
third limit on something two limits already hold.

What is not bounded is a crew of eight each asking once. That is eight cards,
and it is correct: eight agents genuinely need answers, and the count on their
crew's circle is what says so at a glance.

## The ten minutes

A request lapses after ten minutes and the window has not changed. It exists
because a parked turn holds a run booking and costs money to hold, and that
argument does not weaken because there is now a better place to notice the
request. For a present operator ten minutes is generous; for an absent one no
window is long enough.

What did change is what an unanswered question leaves behind. A permission that
nobody answers means nothing happened, which is the safe end and the right one.
A question that nobody answers must not mean nothing happened: the agent is told
so and told to take the most defensible option, do what that lets it do, and say
in its reply what it asked, that nobody answered, what it assumed instead, and
what would change if the assumption is wrong. An agent told only that something
failed reports the failure and stops, which leaves the operator with no work
done and a question they have already missed once.

## Testing

`presence.test.ts` holds the fold, and `presenceOf` names every activity state
rather than defaulting, so a variant added to the runtime and not weighed here
fails the typecheck rather than drawing a busy crew as idle. The other half of
that seam — a variant added in Rust and never written down in TypeScript at all
— is in `ipc.contract.test.ts`, which compares the two lists directly.

`Desk.test.tsx` and `QuestionRequest.test.tsx` hold the refusals: no verdict on
a question, no standing yes for anything done in the operator's name, no empty
answer, no card dropped before the runtime has confirmed it.

The cascade suite drives a real turn through a question end to end: it parks,
the operator answers, the answer reaches the turn. What no offline suite can
reach is the ten minute window, because that is a wall clock and the stub is a
real server, so the unanswered path is covered as far as the row and the release
and no further. Whether an agent does something sensible with "nobody answered"
is a model's behavior and belongs to the evals.

**Adding a tool changes how a crew behaves, and CI cannot see that.** Run
`./scripts/evals.sh` after anything in this file changes what an agent is
offered or how it is described.
