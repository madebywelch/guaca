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
opening a channel can invalidate it. A search hit or a click on the flow board
can land on an agent in another crew, and a rail still showing the group you were
in has the open channel nowhere on it, so `select` lets the focus go.

The pinned section is a flag drawn as a place. It spans groups, so a row dropped
among the pins keeps its own crew: the alternative is a gesture that says "keep
this where I can see it" and quietly moves the agent to a different set of peers.

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
