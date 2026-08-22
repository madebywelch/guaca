# The workspace

What the operator sees, and the decisions the webview makes on its own. The
runtime half is in `ARCHITECTURE.md`; `src/lib/transcript.ts` is the file to
read first.

## A channel names nobody, and that is not a missing feature

It has two participants: the agent it is named after, at the top of the pane,
and the person reading it. A name and a clock over every message is two lines of
chrome carrying one fact, and four replies written inside the same minute drew
four of them. The portrait says which agent and the side of the column says
whether the words are yours. `named` on `MessageItem` is where the two views
part: a channel passes `false`, and the pair's own thread takes the default,
because there both participants are agents and neither is the reader. The clock
went with them: it is a hover on the row, and `transcriptRows` draws one line
where the silence ran past half an hour, which is the only place a time ever
changed what the operator understood. That line also ends whatever burst was
open, because two exchanges three hours apart are two things that happened.

What a channel folds and what it must never fold is in *A channel says an
exchange happened; the pair's thread is what it said*, in `ARCHITECTURE.md`.

## A turn's own work is chips, not a line per call

The third thing in a channel, after the operator's conversation and the peer
traffic. It had the least design and by volume it was the most of it: a turn may
make two dozen tool calls, because the round limit is twenty-four and a browsing
turn legitimately spends most of it, and every one of those was a line reading
`Chef used browse`. That is the same burial peer traffic was collapsed to fix,
arriving from the other direction, and it sat between the operator's question
and the answer to it.

So a run of calls is one row of chips, one per kind of work, and the calls
themselves open underneath it. `lib/trail.ts` holds the rules as pure functions
over a list, the way `lib/rail.ts` does for the rail, so what folds can be read
and tested without a DOM.

Grouped by tool across the run rather than only where calls are adjacent. The
order a model happens to interleave `browse` and `run_command` in is not
something anybody asked about; "4 steps on cnn.com, ran 2 commands" is.

**Two things never fold, and they are the two the row exists to be right
about.** A call the runtime refused or that failed keeps its own chip with its
reason on it, for the reason a refused send never joins a burst: it is the
runtime stopping something rather than something happening. And a command that
spent one of the operator's credentials keeps its own chip with the credential
named on it, because that is their audit trail for their own tokens and it is
not a thing to put behind a click. The value is not there and there is no field
it could arrive in; the name and the variable are, which is what tells two
tokens apart. `readSpent` mirrors `credentials_named_in` in `runtime/mod.rs` and
the two have to agree: `lib/trail.test.ts` holds this side to that wording, so
a change there is noticed here rather than quietly costing the operator the
trail.

**One call still says exactly what it was.** `Opened cnn.com` and `Ran a
command` are what the operator wanted to know; `used browse` is the name of a
function in a file they do not have. A count only replaces that where there are
several of a kind, and it names what it can even then: a browsing turn that
stayed on one site says which site, for the reason a burst draws one chip per
peer rather than "5 messages with 2 agents".

**What came back is on the chip in two cases and no others.** Most of these
summaries are the line above read back in the runtime's words: `browse` answers
`read in the browser`, `update_notes` answers with a character count printed
directly over the characters. Nine of those turned a row into a paragraph of
grey monospace. So the summary earns its place when the call went wrong, or when
the call has nothing else to show and the summary is the whole of what came
back: `2 agents: Chef, Scribe` is the answer to a directory lookup, not a
restatement of it. Everything else is one click away, where an exit code is
worth reading and a byte count can be ignored in peace.

**A chip that opens nothing is not a button.** A directory lookup is one call
whose whole content is the sentence already on it. A control that does nothing
is one the operator stops trusting the rest of.

Nobody is named on any of it. There is exactly one agent whose own work this can
be, its portrait and name are at the top of the pane, and the rows this replaced
put that name in front of every line. Same argument as *A channel names nobody*
above, applied to the last place it had not reached.

## A transcript is a log, and says one thing out loud

Waiting for a reply is the shape of using this app, and a reader who cannot see
the column arrive has no way of knowing one did. So `.pane__scroll` is
`role="log"` and a message addressed to the operator is announced once, when it
settles.

The announcing is a region of its own rather than the log's own politeness, and
each half of that is deliberate. A live transcript would read three hundred rows
aloud when a channel is opened, which is the whole history recited for the crime
of clicking a name. And it would read a bubble being typed a dozen times before
it was finished, so the streams are `aria-live="off"` and `WorkingNote` already
says the turn is alive.

What is announced is exactly what is drawn as a full bubble, and that symmetry
is the rule. Peer traffic and tool trails are collapsed on screen precisely
because reading them line by line buries the conversation; read aloud they would
bury it more thoroughly, because there is no glancing past a sentence being
spoken. A permission request goes ahead of the text, as it does everywhere else:
it is the one thing in a transcript the operator is expected to act on.

The region sits outside the log. Inside it, the sentence is a second copy of the
newest message for anybody reading the transcript itself, which is the cost of
announcing it to everybody else.

## A transcript follows the end for whoever is at the end, and nobody else

Watching an agent work means watching the bottom of a channel, so a transcript
that did not keep up would have the operator chasing it. Reading back through a
cascade means being somewhere else in the same channel, and moving the page
under a reader is worse than a scrollbar that does not move: a turn writing four
hundred tokens is four hundred chances to do it.

Both wants are real, so the only question is which of the two the operator is
doing, and that question has a wrong answer people reach for. **A scroll event
does not say where the operator is.** It is delivered after the fact, and a
token committing in between arrives first: a wheel tick up, a token, then the
event, in that order. Anything waiting to be told has already put the transcript
back on the floor, and the next tick starts the same race. Under streaming text
a trackpad could not climb out of a channel at all.

So `lib/follow.ts` compares the offset instead of listening for it. It remembers
the offset it wrote, and before writing again it checks the box is still there.
Above it, somebody else moved the box, and the only somebody is the operator: it
lets go, and writes nothing. The check and the write are one statement, so there
is no window for a token to land in, and no threshold to tune: one pixel up is a
decision, because from the operator's side it was one.

Scroll events keep the half of the job that has no race in it, which is noticing
the operator come back. Nothing is being written by then, so a late event costs
nothing, and this end is worth being generous about: stopping a few pixels short
of the end while coming down the page is arriving at the end. A transcript that
then refused to follow would read as broken in the other direction.

Two things override where the operator was, and both are their own doing.
Opening a channel lands at the newest message, because the transcript was
unmounted and there is no position to come back to. And sending a message goes
to the end: typing into the box is a decision to be at the end, and their own
message landing off screen with nothing following it is the same complaint
pointing the other way.

The listener is bound by a ref callback rather than an effect, and that is
load-bearing. A transcript is unmounted whenever the pane shows something else,
a pair thread or the activity board, and comes back as a new node with the same
class. An effect can only re-bind on a dependency it was given, and the node it
holds is not one: a channel opened from the activity board spent the session
listening to a node that had been thrown away, which is the shape the report
arrived in.

## The rail is arranged by hand, and lends the top of a section out

Two orders, not one. `railOrder` on the card is the arrangement: the operator
drags a row into place and it stays there, which is what makes reaching for one
a thing you can learn. Activity is a loan on top of it. An agent that is working
is lifted to the top of its section for exactly as long as it is working, and the
place goes back the moment it stops. `awaitingApproval` outranks working, because
it is the one state the operator is the fix for; `paused` scores nothing, because
it is not work in progress but a row that will not move until somebody moves it,
and lifting it would hold the top indefinitely. A pinned row never lifts at all:
being findable in one glance is what a pin is for, and a row that moves when its
agent gets busy is the thing a pin exists to stop.

Before this, the rail was ordered by who spoke last and by nothing else. That is
an order nobody chose and one that moves under the hand reaching for it: every
reply rewrote it, so no arrangement could survive a conversation. Recency did not
go away, it just stopped being the whole answer: it is what separates two lifted
rows, and it is still the text in the right-hand column.

`lib/rail.ts` holds all of it, and holds it as pure functions over a list, so the
same rules order a section, decide where a drop lands, and decide what "one row
up" means. The rail draws the arrangement itself while a drag is in progress,
with nobody lifted. Dragging is arranging: a row dropped below a peer that is
only near the top because it happens to be mid-turn would land somewhere nobody
aimed at, and the rail would look like it had ignored the gesture the moment that
turn ended.

## A drop is one call, because a drag is one gesture

`move_agent` takes the group and the row to land in front of, and `None` for the
end of that group. One command rather than two: a drag can change the crew and
the place at the same time, and two writes leave a state where the agent has
joined the group but not landed in it. The runtime renumbers every live row
densely inside one transaction, because a workspace holds tens of agents and a
scheme with gaps has a state where the gap is used up that this one does not.

The anchor is a row, not an index. The operator dropped onto something they could
see, so the position is expressed as the thing they saw; a stale view cannot
corrupt an arrangement, it can only lose the half of the intent that no longer
applies. A row that is gone, or has moved to another group, is ignored and the
agent lands at the end of the group it was dropped into. A row dropped on itself
asks for nothing and is refused in both halves, because the fallback for an
anchor that is not there is the end of the group, and a null gesture that reached
it would move the row to the bottom of the rail.

Which side of the target it lands on is read off two positions rather than
measured against the pointer. A row's midpoint is geometry a test cannot see and
a hand cannot aim at; the direction the row travelled says which side it belongs
on, and dragging down past something and dragging up onto it are exactly what
those two look like while they are happening.

**Pointer events, not HTML5 drag and drop.** `dragDropEnabled` is what lets a
dropped document reach Rust without its bytes entering the renderer, and it is
the same setting that stops `dragstart` firing inside the webview on some
platforms. A rail that only rearranges on macOS is not a feature. A press
becomes a drag after five pixels, because a row is a button first.

Everything a drag does is also in the agent's menu: move up, move down, and move
to a named group. A rail that can only be arranged by dragging cannot be arranged
from a keyboard at all.

## A group is a place you can be inside

Two views of one rail. In the overview every group has a heading and its members
under it, which is what it always was. Clicking a group's circle takes the rail
inside it: one crew, its name and controls given a line of their own, and the
pins folded to the head of the single list rather than kept in a section of their
own, because everybody drawn there is in that crew already.

The circles are faces, not names. A crew is recognised by who is in it long
before its name is read, and four of them have to fit across a rail that is
15.5rem wide. Four faces tiled in a square rather than a row of overlaps: the
circle is 38px across and three 1.35rem avatars in a row measure 51px even
leaning on each other, so a row either overflowed the ring or hid the faces it
was drawn to show.

Each circle does two jobs, which is why it is a circle and not an item in a menu.
Clicking it opens the group. Dropping an agent on it puts the agent in the group,
so the shortest gesture for moving somebody between crews is the one that also
says which crew, and it is on screen for the whole of the drag. The circle also
carries whether anyone inside is working and whether anyone is waiting on the
operator: once the rail is inside one group, the strip is the only place the
other crews are visible at all.

The strip is absent while there is one group, which is the state most workspaces
are in. A choice of one is chrome that never changes and a drop target that
cannot move anybody anywhere.

The group being looked inside is in the store rather than in the sidebar, because
it and the open channel invalidate each other, and both are in the store. A
search hit or a click on the flow board can land on an agent in another crew, and
a rail still showing the group you were in has the open channel nowhere on it, so
`select` lets the focus go. Going inside a crew is the same mismatch from the
other end, so `focusGroup` closes the channel and the pane falls back to the
activity feed, which belongs to no crew and is never closed. Whichever of the two
the operator just asked for is the one that survives.

Closing it is not tidiness. Two crews can hold two agents with the same name and
the same face, and a rail that is not drawing the row leaves nothing on screen
saying which crew the pane belongs to: a channel left open from the group you
came from reads as a member of the group you are looking at, working while nobody
here is. The agent goes on working either way. Closing a channel is not a control
on the conversation, only on what you are looking at.

Going back out to the overview keeps whatever was open, because the overview
draws everybody. Neither does a move close anything: dragging somebody into
another crew is not a change of view, and the operator who just made the gesture
is the one person who does not need telling where that row went.

The pinned section is a flag drawn as a place. It spans groups, so a row dropped
among the pins keeps its own crew: the alternative is a gesture that says "keep
this where I can see it" and quietly moves the agent to a different set of peers.

## A group's settings are the app's, with the crew's answer on top

Everything in the group editor except the name is an override, and blank means
inherit. That is why the boxes carry placeholders rather than values: an operator
has to be able to tell "this crew uses the app's model" apart from "this crew
pins that exact model", and an empty box that means two different things is a
setting nobody can read.

It is sectioned on the Settings shell for the same reason Settings is: a group
now decides who pays for its turns, which model answers them, how long a call may
take and how far a conversation may run, and one scroll put the name and the
delete button a page apart. The state lives in the shell, so changing section
cannot discard a half-typed endpoint. Accounts is disabled until the group
exists, because a credential has to belong to something.

Three rows say who pays, and the first is "follow the app settings", which is
where every group starts. The second is the ChatGPT subscription, and the group
editor cannot sign in: there is one sign-in on this machine, it is performed in
Settings, and what a group chooses is whether to spend it. The rest is the same
preset list Settings draws, from the same file, because an endpoint that is off
by a path segment fails the same way whoever typed it.

Two model fields, not one, and only the one belonging to the resolved provider
is on screen at a time. A model belongs to a provider — the two have disjoint
names and neither accepts the other's — so a crew that tries the subscription for
an hour and moves back has to find its endpoint model where it left it. Test
connection is here for the same reason it is in Settings, and it sends what is on
screen resolved over the app settings, which is what the next turn would do.

## Pinning is where a row is drawn and nothing else

It does not bump the card version, because the version is how a peer notices a
card changed under it and nothing a peer can read has. `railOrder` is the same
kind of fact and is kept the same way, and both live on the agent rather than in
a preferences blob because they have to die with it: a name is free to reuse the
moment an agent is deleted, and whoever takes it next must not inherit a pin or a
place. A pinned agent is lifted out of its group in the rail and still counted in
it, because it is still in it: same wall, same bill, same peers. Two rows for one
agent would be two nodes in the sidebar's `rowRefs`, and the wire would have to
pick one to throw a message at.

## The cafeteria is a copy machine, not a registry

Sixteen agents written out once, well, so that a new workspace is a few clicks
rather than an hour of typing. They are named after jobs rather than functions:
"Chief of Staff" and "Paralegal", not "Manager" and "Reviewer". A role carries
duties and refusals a function label does not, so the operator does not have to
supply them in the prompt, which is the work this removes. Titles are capped at
three words because peers resolve each other by whole name and the composer's
`@` typeahead gives up after two spaces, so a longer title is an agent nobody
can delegate to.

A hire copies the preset's fields into an ordinary `AgentDraft` and forgets
where they came from: there is no preset id on
the card, nothing joins back to `lib/cafeteria.ts`, and an agent hired yesterday
does not change when the catalog does. That is what stops a UI file from
becoming a schema the database has to agree with, and it is the reason there is
no "update from preset" anywhere.

A preset's model is deliberately blank, which means inherit. Writing the app
default in at hire time is the obvious thing and it is wrong: it pins every
hired agent to the app model and silently ignores a group that chose its own
endpoint, which is exactly what a group-level model exists to express.

A hired crew lands under the arrangement, not inside it, in the order it was
picked. The batch reads the bottom of the rail once inside its transaction and
hands out consecutive slots: asking per agent would give all of them the same
answer, because none is written until the commit, and the rail would then order
a crew by the tiebreak rather than by what the operator chose. See *The rail is
arranged by hand* above for what those slots are.

`hire_agents` takes the batch rather than the UI looping `create_agent`, for
two reasons that both bite at four agents and up. Every create emits
`AgentsChanged` and the rail answers each by re-reading the whole roster. And
names are unique per group, so a batch has to be settled against the roster
*and* against itself: two `Researcher`s picked in one go are both free until the
first one is written. `domain::agent::hire_names` does that in the same place
`copy_name` already lived, so the app has one rule for a name somebody else is
holding instead of two that can disagree.

The catalog is content with a test, `lib/cafeteria.test.ts`, holding it to the
avatar and accent catalogs and to what `AgentDraft::validate` will accept. The
rule that is not mechanically checkable is the one that matters most: every
preset prompt states a stopping condition. A prompt without one makes a crew
that talks to itself, no automated suite can see it, and the evals are what
catch it. Run `./scripts/evals.sh` after touching a preset.

## A duplicate copies the card and nothing an agent went and did

Look, model, skills and instructions; not the sandbox, the memory, the schedule,
the accounts or the transcript. Two agents holding one sandbox id is two agents
on one machine, and a copy that inherited a routine would double a standing
commitment nobody asked to double.

## Search happens in two places and is ranked in one

The workspace is held in two places, so it is matched in two: messages, files,
links and routines are in SQLite and are matched there, while agents and groups
are already in the webview's store to draw the rail and actions are not stored
anywhere at all. Reading the transcript into the renderer to search it would
copy the database across IPC on every keystroke; going to IPC for two agent
names would make the commonest search the slow one. What must not be split is
the ordering: both halves arrive in `lib/search.ts` as raw matches and are
scored by one function, because a list where an agent and a message are ordered
by different rules is a list you have to read twice. A file and a link are the
same rows as the messages read from a different angle, which is why one scan
produces all three.

## A search hit that opens the wrong part of a channel is a search that failed

A transcript is read as "the newest three hundred", and a hit from last month is
not in that window. `channel_messages` takes a `through` so the window reaches
back to the message being opened, bounded at a thousand; past that the operator
lands in the right channel at its newest end. Anything that jumps to a message
goes through `openMessage` rather than `select`.

## Settings is eight places, because it stopped being one subject

An endpoint, a set of limits, a machine's credentials, how large the window
draws and what is allowed to interrupt you are five different questions, and one
scroll made the operator read all five to change one. So it is a nav and a pane,
on the Cafeteria's shape: a panel that owns its own height, a head and a foot
pinned to it, one scrolling half.

Two of those eight are defaults rather than orders. Whatever Provider and Limits
say is what a group falls back to, and a group that answers for itself is not
affected by either. What stays app-wide is what is genuinely one of: the
operator's name, the machine and browser accounts, and everything about how the
app looks and when it may interrupt.

Every value lives in the shell rather than in the pane that draws it, and that is
not tidiness. The shell is unmounted when the dialog closes, so a pane holding
its own state would discard it on every section change: typing an endpoint,
glancing at Limits and coming back would silently lose the endpoint. Save and
Test stay in the foot for the same reason they used to be at the bottom of the
one column — they act on the whole panel, not on the section that happens to be
open — and Save still does not close, because the point of Test is to press it
next.

Two things open onto a named section rather than onto the top. The banner that
says there is no API key opens Provider, because landing on General with the key
two sections away was a step nobody wanted, and the shortcuts key opens
Shortcuts. The palette's own row opens the surface rather than a section: it
lists what is in there, so the row is found by searching for "notifications" or
"appearance", and the nav is the first thing under the cursor once it is open.

The provider list is presets, not a registry. Guaca speaks one protocol and the
field is a text box, so the failure it exists to prevent is an endpoint that is
off by a path segment and fails on every turn of every agent with an error from
somebody else's server. A local endpoint is marked as local because that changes
what the key field means: an empty key beside a warning about a missing key is a
state an operator will try to fix forever.

## The reading column has two surfaces; the rail has one

`styles.css` used to say the surface never follows the OS, and the argument was
sound as far as it went: a chat log is read for minutes at a time and white wins
that in a lit room. It does not win it in a dark one. So the reading column is a
token block behind `data-surface` — the same seven neutrals, four accents, a
scrim and a shadow, and nothing else.

The rail is not part of the question. It is dark in both surfaces, which is what
makes the column read as a surface rather than as another panel, and it pins the
two accents it draws with on `.rail` itself. That last part is load-bearing: the
rail reads `--flesh` in twelve rules and `--flesh-soft` in two, so a dark value
for either would have repainted it silently and no test in this repo would have
caught it. Pinning them on the one element every rail rule descends from turns
the rail into a colour scope instead of a naming convention.

`system` is resolved before it reaches the document, so `data-surface` is only
ever `light` or `dark`. A rule keyed on `system` would have to duplicate the one
keyed on `dark`, and CSS has no way to share the two.

Scale is the same idea from the other side: every length in the stylesheet is
already a `rem`, so the whole interface is one root font size. It is anchored at
16px because that is what a `rem` already resolved to — nothing had ever set a
root font size — and `body` moved from `15px` to `0.9375rem` so that everything
inheriting from it grows too. The rail and the inspector are capped against the
viewport as well as scaled: both grow with the scale and the window's own minimum
does not, so at the largest scale in the smallest window they would otherwise
leave the reading column narrower than a message can draw.

Neither preference goes anywhere near the runtime. They are `localStorage`, the
way the inspector's open-or-closed already is, because the runtime would carry
them across IPC only to hand them straight back.

## An interruption has to earn it

Guaca keeps working while nobody is looking, which is the only time a
notification is worth anything: routines fire on a schedule, a parked turn waits
ten minutes for an answer, a cascade settles long after the message that started
it. All of that matters when you are elsewhere and none of it is worth a badge
while you watch it happen.

So "away" is not one condition, because the four kinds are not the same news.

- **A permission request** blocks a turn until it is answered. It reaches the
  operator when the window is not in front of them, *and* when it is but the
  request belongs to a channel they are not looking at. Nobody finds a parked
  turn by noticing a row change colour three screens up the rail.
- **A routine firing** was addressed to nobody and implies no channel: it goes
  where it was pointed, which is almost never where the operator is. It reaches
  them whenever they are away, with no channel to match against.
- **A conversation finishing, or failing**, is the end of something they started.
  It reaches them only when they are away *and* it is the channel they were last
  looking at, because a busy runtime settles runs in channels nobody has ever
  opened and one badge each would make the whole mechanism worth turning off.

Two more refusals, both of which would otherwise read as a bug. Nothing fires for
the first few seconds after a launch: a routine whose slot passed while the app
was closed is overdue and fires on the first tick, which is correct, but starting
Guaca after a weekend away should not announce a weekend of schedule at once. And
the same kind about the same channel is said once a second at most, so two agents
failing together are two notifications while one agent failing twice is one.

There is a button that sends a test notification, which is the only way to tell a
refused operating-system permission from a working one.

## The menu bar is Guaca with the window shut

A notification is a thing that happened. The menu bar is the state of things,
standing, for as long as the window is not what you are looking at. Those are
different questions and neither answers the other: a banner about a parked turn
is gone in four seconds, and the turn is still parked.

So there is a presence in the menu bar, and it has four channels doing four
jobs. `menubar.rs` decides all of it and `tray.rs` draws it, which is what makes
every judgement below arguable in a test rather than by squinting at a corner of
the screen.

- **The glyph** is state without being looked at. An outline when nothing is
  running, filled when something is, and warm red when an agent is blocked on
  you. That last one is the only glyph that is not a template image, and giving
  up the menu bar's own light-and-dark tinting is the price of it: macOS tints a
  template image to match the bar, so a template glyph cannot have a colour.
  Worth paying exactly once, for the one state that must not be missed.
- **The title** is the count of turns waiting on you, and nothing else. Menu bar
  width is shared with every other app on the machine. A number that is always
  there becomes furniture and stops being read; one that appears only when
  something is parked is information. The spend is deliberately not here.
- **The tooltip** is one line on hover: what is happening, and what the session
  has cost. The glance that costs no click and no width.
- **The menu** is the whole picture. What is waiting, who is working, what has
  been spent, and the two things worth doing from here.

**A permission request is answered from the strip.** That is the point of the
whole feature. A parked turn is the one thing in Guaca that stops until you deal
with it, and the flow-preserving move is to answer it where you noticed it rather
than to go and find the channel. The request's own fields are under it, because a
decision made without them is a decision made blind, and every one of them is
`label: value` with the label being Guaca's word: a value crafted to read like an
answer then sits behind a heading the runtime wrote rather than loose in a menu of
answers. The same refusal as the card in the transcript applies, for the same
reason: `actOnBehalf` is never offered an "always".

**Spend is two lines, session and all time.** A price with no places is a working
crew reading `$0.00` for its first hour, so the precision follows the number,
exactly as the group meters do. The token count is always there and the price
joins it only when there is one worth the width, under the same floor as the
meters: a local server and a subscription plan price nothing at all, and a free
model prices every call at a real zero, so `$0.0000` is what a strip shared with
every other app would otherwise spend seven characters saying. No price is not a
price of zero either, which is why a workspace with one local crew and one hosted
one reports the hosted one's bill rather than the average of a number and a
silence.

That floor is written twice, once in Rust and once in TypeScript, and
`ipc.contract.test.ts` compares them. Two readings of one number that disagree
give the operator no way to tell which is lying.

**Closing the window puts Guaca in the menu bar instead of ending it.** Tauri
exits when the last window closes, and for this app that is the wrong default:
agents keep their own appointments, so quitting on a close means a routine set
for every morning stops firing the first time somebody tidies their screen, with
nothing said. A hidden window is not a closed one, so preventing the close is the
whole mechanism and no exit handling goes with it. Command-Q and the strip's own
Quit still quit.

That is conditional on the strip having built, and the condition is the point
rather than caution: an app with no window and no menu bar icon is one the
operator cannot see, cannot reach and cannot stop. If the tray did not build,
closing the window quits exactly as it used to.

**"Stop everything running" is the counterpart to that change.** A window that is
gone is not a workspace that has stopped, and the cost of finding that out late
is money. It sits beside the spend it is about, appears only when there is
something to stop, and is a run-level stop like the one in the working line: what
the operator wants to end reached however many agents it reached.

Two implementation decisions are load-bearing and read as fussiness.

The presence is **read, not accumulated**. Every number but the session total is
a fresh read of whatever already holds the truth: the roster, the activity map,
the pending requests, the usage table. A presence assembled by adding up events
drifts the moment one is missed, and the thing that would drift is the number
being used to decide whether to go and look. The reads are local, coalesced, and
happen only while something is happening.

The menu is **edited in place when it can be**. Replacing a menu closes one the
operator is reading, and the spend on it moves every few seconds while a crew
works, so a strip that rebuilt on every change would be unreadable exactly when
it was worth reading. `menubar::plan` compares the shape of the rows: same shapes
in the same order is the same menu saying different numbers, which is a text
edit, and anything else is a rebuild.

What is deliberately not here: a second window. A popover would be a whole
webview to position, blur and keep in step for the sake of drawing a sparkline
nobody asked for, and none of the four questions this answers needs one.

## Every key the app answers to is in one list

The app answered to a dozen keys and said so nowhere, which is the same as not
having them. The Shortcuts pane is that list.

It is a reference rather than a rebinding system, deliberately. Nine of these
keys are handled by the component that owns the surface they act on — Escape by
whatever is open, Enter and the arrows by the composer and the palette — so
rebinding would mean routing all nine through one dispatcher, and a setting that
appears to rebind a key while only moving one of them is worse than no setting.
The fixed ones are listed as fixed, and the three that are genuinely global are
matched from the same table the pane draws, so a key listed there is a key that
works.

`mod` means Command or Control on every platform, both accepted. Only the label
follows the platform: a label naming a key the keyboard does not have is worse
than none.

## Stop is a control on the conversation, not on the agent

It sits in the working line, always visible rather than on hover, because a
control that appears on hover is a control nobody knows is there and this is the
one thing on screen that costs money to leave alone.

What it stops is the run. A message that reached four agents is one conversation,
and stopping only the one on screen would leave the other three working on the
operator's bill. The button knows which run to name because the runtime already
said so: a placeholder opening in an agent's channel is the runtime's own
statement that this agent is working on that run, and the entry is dropped when
the run settles. Not read from what `sendMessage` returned, which only knows
about conversations the operator started — a routine's work and a peer's request
are exactly as worth stopping.
