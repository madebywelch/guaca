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
`read in the browser`, `update_memory` answers with a character count printed
directly over the characters. Nine of those turned a row into a paragraph of
gray monospace. So the summary earns its place when the call went wrong, or when
the call has nothing else to show and the summary is the whole of what came
back: `2 agents: Chef, Scribe` is the answer to a directory lookup, not a
restatement of it. Everything else is one click away, where an exit code is
worth reading and a byte count can be ignored in peace.

**A chip that opens nothing is not a button.** A directory lookup is one call
whose whole content is the sentence already on it. A control that does nothing
is one the operator stops trusting the rest of. A memory that was cleared is the
one call with no content and something to say anyway: what was thrown away is
the whole of what happened, so it opens on that.

**The two things worth a longer look share one slot.** The working the model
has published and the chips for what it has done are both disclosures on that
line, and they open into the same bounded panel above it rather than stacking.
Stacked, the transcript gives up twice the height for a question asked once and
the composer moves twice. `styles.test.ts` holds the two to one bound, because
they are one place on screen that draws two things.

**A memory rewrite opens as a diff, because the call is always the whole page.**
`update_memory` replaces the file rather than appending to it, which is the right
interface for an agent — it has to reconcile what it believed against what it
just learned — and the wrong one for whoever reads the result. Opening a rewrite
used to show a page of markdown, and "what did it decide to remember this time"
meant holding two near-identical pages in your head. So the call carries what it
overwrote, and `lib/diff.ts` draws the lines between the two.

That previous version is recorded by the runtime at the moment of the write and
cannot be worked out later by anybody. The same memory is written from an
agent's wall and from every thread it holds, and the operator can edit it by
hand from the agent's panel, so a previous version recovered from the transcript
this call happens to sit in is wrong exactly when something interesting
happened. It is on the call rather than in its arguments because the arguments
are the model's own JSON, verbatim, and a reader has to be able to tell what the
model asked for from what the runtime found. Calls recorded before any of this
have nothing to compare against and draw as they always did.

Every unchanged line is kept, unlike a patch. A page is short enough to show
whole, and an operator opening this has a second question the changed lines
alone do not answer: not only what the agent changed its mind about, but what it
now believes. The marker in the gutter is a character rather than only a color,
because red and green either side of a line are the one distinction a reader may
not have, and because it is what makes a diff copied out of the window still
read as one. How much moved is said in words, in the place the runtime's own
summary would have gone — that summary is a character count printed directly
above the characters it counts.

Nobody is named on any of it. There is exactly one agent whose own work this can
be, its portrait and name are at the top of the pane, and the rows this replaced
put that name in front of every line. Same argument as *A channel names nobody*
above, applied to the last place it had not reached.

**A chip is never cut to make room for what came back.** Both are on the same
line and only one of them yields, which is not the rule flex applies by
default: it shrinks in proportion to what each item asked for, so a refused
call whose reason ran to a paragraph asked for twenty times what its label did
and took the row on the way to being clipped itself. The chip drew `U… a coding
agent is already working in whizzworks-site, started by…`, which is one
character about which call went wrong. A weighting is the near-miss worth
naming and not the fix: at a hundred to one the label still gave up its last
letter, because proportional is proportional however lopsided. So the label
does not shrink at all, the answer takes whatever is left on the default every
flex item has, and the chip clips anything that still will not fit.
`styles.test.ts` holds it, because no DOM assertion sees a layout.

**A refusal opens onto its reason.** It is written to be acted on and it runs
to a paragraph, so the copy on the chip is a summary of something rather than
the thing itself, and it is drawn where a command is drawn: whole, wrapped, and
scrolled if it is long. Only where the call has nothing else to show — somebody
opening a failed `run_command` came for the command. Open, the chip stops
repeating the first line of what is directly underneath it.

**The same chips are drawn while the turn is still making them**, above the
composer rather than in the transcript, from `trail` in the store rather than
from a message. Same rules, same file, same fold, and the same value: the
runtime reports a finished call as the whole part the message will carry, so a
memory rewrite opens as the diff above while the turn is still running and not
only once it has ended. A ten-minute turn is otherwise ten minutes of a
pulsing avatar and a line of prose, and the transcript cannot help, because the
record it draws this from does not exist until the turn ends. It goes when the
turn does. *A turn's own work is watched while it happens* in
`docs/ARCHITECTURE.md` is why it is safe for it to be there at all.

**Behind a count, though, and not in front of one.** Open, the live copy is the
whole record of a long turn stacked between the transcript and the composer:
seven kinds of work wrapped across four rows of gray monospace, reflowing every
time a call comes back, with the box the operator is typing into moving
underneath it. That is the shape a channel was collapsed to get away from,
rebuilt in the one place nobody was looking at it. What it answers — "is this
doing something sensible" — is a question asked a few times in ten minutes, not
continuously, and the answer is permanent a minute later in the transcript. So
the line carries `12 steps`, and the chips are one click behind it.

Two things stay in front of the click, and each is on the line for a reason the
count cannot cover. **A failure is counted separately and says so**, because it
is the one part of the trail the operator may have to act on and a number that
folded it in would report a turn that refused half its work as a turn that did
it. **A credential is named**, not counted, for the reason it never joins a fold
anywhere: it is the operator's audit trail for their own tokens. The value is
not there and there is no field it could arrive in.

**A call that has not come back is not a chip.** It has no outcome to draw one
from, and it is not counted either: it is what the line itself is for.
`Running a command · 1m 14s`, in the present tense, because a command still
running is not a command that ran. That line is the one place `describe`'s
vocabulary is not reused, and `callInFlight` is deliberately coarser than it.
What is worth knowing while a call is in flight is which machine has not
answered yet.

It has to have been in flight for a second first. A `directory` lookup answers
in milliseconds, and a line that flashed its name for each of them would put
back the flicker the sentence rule in `lib/reasoning.ts` takes out. What the
line is for is the wait an operator notices, so a call has to become one before
it earns the line.

## An agent's memory is in the panel, and never quietly overwritten

The transcript above says what an agent changed about itself. What it currently
believes is a different question, and it used to be answered two thirds of the
way down the profile dialog, behind a right click. That put the one thing about
an agent that changes on its own behind a modal opened to change the things that
do not: the name, the look, the instructions. Memory is the opposite shape. It
is read constantly, written rarely by hand, and rewritten by the agent itself in
the middle of a turn. So it moved to the panel beside the transcript, and it is
first in it, because it is the only section there that every agent has something
in: a screen is usually an offer rather than a picture, and a new agent keeps no
routines.

It is one box, always editable, rather than a rendered page with an edit mode.
What is stored is markdown, and an operator seeding a persona wants to see the
characters the agent will actually be shown; a mode would buy formatting nobody
asked for and cost a click on the thing the panel exists to make cheap.

**A read never replaces what the operator is in the middle of typing.** The
whole reason the panel refreshes is `MemoryChanged`, which is the agent
rewriting the file mid-turn, and the operator is most likely to be editing it
exactly then. So a version that lands under a draft is held to one side rather
than applied, the panel says it happened, and the two ways out are already on
screen: Save keeps what you wrote, Discard takes what the agent wrote. `arrived`
is where that decision is made and it is made against what is held at the moment
the read lands, not what was held when it started. A draft typed back to what is
on disk is not a draft, and holding it as one would leave the panel sitting on a
page the agent has since replaced.

**Only the runtime's own write emits the event.** The operator's edit comes back
from `set_agent_memory` as what was actually stored, so the panel that made it
already has the answer and an event there is a refetch to learn what the reply
just said. What comes back is also what goes on screen, not what was typed:
`Workspace::write` trims and cuts at `MAX_MEMORY`, and leaving the typed version
up would show an operator a page their agent is never going to be given. A cut
is said out loud; a trim is not, because a trim on every save that ended in a
newline is crying wolf.

**How full it is, is said only near the cap.** A running character count under
every agent's memory is a number nobody reads. Past the cap the end is not
stored, and an agent at the cap has already started throwing things away to make
room, which are the two states worth a glance.

The number here mirrors `MAX_MEMORY`, and `Memory.test.tsx` reads the Rust and
compares, the way `ipc.contract.test.ts` does across the same seam. It was
written as advisory in the way `LIMITS` is advisory, on the reasoning that the
runtime is what actually cuts and the read-back is what the operator is shown,
so a drifted mirror cannot cost anybody their text. True, and not the failure
that happened. The runtime went to 16,000 and the panel stayed at 4,000, and a
5,003-character memory drew *1,003 characters over. The end is cut on save.*
about a page the runtime was storing whole. A warning is read as a fact about
what is going to happen, so an operator believing that one edits down a memory
that was never in danger. Both sides compiled, both suites passed, and the only
symptom was a sentence on screen that was not true. The number is only worth
drawing while it is the runtime's number.

## An agent's memory is what it knows, and its working notes are what it is doing

Two stores, because one cannot have both lifetimes.

Given only memory, an agent puts its progress in it, and it is right to: memory
is the only thing that survives the turn, and the alternative to writing down
what it is waiting on is forgetting it. What that costs is that a page rewritten
every turn is a page where copying a stale section forward is cheaper than
deciding it is stale, so the ratchet turns one way. Sixteen of one operator's
twenty-three agents had a *Waiting on* or *Status* heading, a fifth of every
memory file was task state that had stopped being true, and several agents had
invented headings called *Working notes* and *Working memory* inside the one
file they had. The distinction was already being drawn by hand, without support.

**Memory holds what you could not look up again.** Who the agent is, how it
works, standing preferences, decisions that hold across conversations, what it
has learned about the people around it. And pointers: if the agent could open
it, it does not copy it, it records where the thing is and when it is worth
opening. An assistant here spent 900 characters summarizing an engineering
report whose filename was three lines further up its own memory, and the summary
is the copy that goes stale.

**Working notes are not the same list as *What you are waiting on*.** That
section is derived: computed from the agent's own unanswered sent messages, so
it cannot go stale and costs the agent nothing to maintain. Working notes are
written, and cover everything off that path: what the operator owes the agent,
what it handed over, what it decided, what is still open. Each section says so,
because an agent reading both without being told would spend half a bounded list
restating one it is given for free, and the restatement is the copy that rots.

**Working notes hold where the work stands, and expire.** One line per note,
appended with `note_progress`, never revised. The agent is shown them with an
age against each, which is what turns *waiting on the legal read* into something
it can act on: the same line marked six days old says the thing is not coming.

**A note is written when the next turn would go wrong without it.** That test,
and not "note freely", which is what the tool and the prompt said first. It is
true about what one note costs and it was read as a reason to write one: agents
noted what they were about to do, what they had just said, and each step of a
task they finished inside the same turn, so a list of sixteen ran on a turn
narrating itself while what the agent was waiting on aged off the end of it.
Cheap is a fact about the shape of the write, one line against memory's whole
page, and it has to stay true or the impulse goes back to memory where it came
from. It is not an invitation. The rule is now a question about the *next* turn
— work you would repeat, something you would carry on waiting for, a decision
you would make differently — with the three cases that produced the volume named
as exclusions, because a model given only a positive rule reads every borderline
call as inside it.

**A line the agent already holds is not stored again.** The one mechanical
brake, and the only one this store can have: it cannot ask the agent to prune,
for the reason below. *Still waiting on the legal read*, noted on four turns, is
four of sixteen slots spent on one fact and three notes that record nothing. The
repeat is refused with the age of the note that already says it, which is both
what the agent was reaching for and what it needs in order to chase or give up.
The stamp is deliberately not moved forward: refreshing it would hide staleness
at exactly the moment the age is worth reading. This is not a revision either,
which is the rule the store does keep — nothing is edited and nothing is
dropped, the second write simply never happens. The match is exact, per agent,
and against the notes that survived the trim: a line that has aged out is one
the agent can no longer see, so writing it again is recording rather than
repeating.

**The write rules are deliberately not symmetric.** Consolidating a memory after
every interaction degrades it, and past a point below having no memory at all
(arXiv 2605.12978); localized maintenance holds up better than global
reorganization (arXiv 2606.24775). A full rewrite is global reorganization by
definition, so memory is small and written rarely and can afford it, and a
working note is never consolidated at all. An append is also the only write that
is safe here: an agent's stores are written from every thread it holds, so a
read-modify-write against a file loses notes under exactly this app's
concurrency. An `INSERT` does not, which is why this one is a table.

**Forgetting is the store's job, not the agent's.** The oldest notes fall off
once there are more than `KEPT` of them, and there is deliberately no tool to
edit or delete one. A stale note does not sit inert, it steers the next turn
toward work that is already done (arXiv 2505.16067), and deciding what to drop
is the operation these models are measurably worst at. This is the one store
that never asks.

**The panel draws the newest four and keeps the rest behind a button.** Its
bound and the store's answer different questions: sixteen is what an agent may
carry, four is what one section of a shared column can show. A note wraps to
three lines in a sidebar that narrow, so a full list is a screen of text with
the schedule and the memory below it, and the operator scrolls past what the
agent is doing now to reach anything else. The newest are what stay, because
the list reads oldest first and the tail is where the work got to; the button
carries the count of what it hides, since a list of six and a list of sixteen
are otherwise the same closed section.

**The panel below the memory is read-only apart from Clear.** That asymmetry is
the design and not an unfinished half of it. Memory is a page two parties
maintain, which is why it needs a draft, a held incoming version and two ways
out. The working notes are the agent's own account of its work: the operator
either believes it or declares the work done. Clearing exists because the one
failure this list has is an agent still waiting on something the operator
settled in person. Editing a single line does not, because a list the operator
half-rewrites is one neither of them can trust.

**A reset takes both stores, and it has to take both.** *Start fresh* on a
group deletes what the crew said, what it had scheduled and what it spent, and
the memories with them, because an agent that opens tomorrow believing what it
wrote about a conversation nobody can read has not started over. The working
notes are the same argument one step closer in: they are what each agent
thought it was in the middle of, and every one of them is about the
conversation that just went. Left behind, a reset crew's first act is to chase
work whose record it no longer has. `clear_group` takes both, reports each
separately, and the panels are sent back to read again on `ChannelsCleared`:
neither refreshes on its own, so the inspector would otherwise draw a memory
and a list of notes beside the empty channel that says they are gone.

**The tool is `update_memory`, and `update_notes` still parses.** `notes` meant
memory everywhere the code named it for a year, which is exactly the ambiguity a
second store cannot survive, so the internals took the operator's word. The old
name is kept as an alias because a model that learned Guaca from an older
transcript still reaches for it, and refusing it spends a whole turn on a rename
the agent had no way to hear about. What is *recorded* is the current name
either way, so the transcript does not fork from here on; `lib/trail.ts` answers
to both because rows written before the rename cannot be migrated into a new
spelling, which is the same reason `Part::Approval` was never widened.

## A reply can mark the one part that needs a person

An agent that has been working for ten minutes writes nine paragraphs, and one
of them is the sentence that needs the operator: a key only they can rotate, a
decision nobody else can take, a thing about to go out that should not. Written
as prose it is the fourth paragraph of nine, and it is read last or not at all.
So a quote a model opens with an alert marker is drawn as a box.

**The syntax is GitHub's, and that is the point.** Models write `> [!IMPORTANT]`
without being asked, so the box appears on a reply written before the prompt
mentioned it, on one from an agent skimming its instructions, and on every
transcript already in the database. A marker `lib/callout.ts` does not know
stays a quote with its own words in it, which is the rule `figure.ts` keeps for
a fence it cannot draw.

**A quote and not a fence.** Figures are fences because a chart spec is text.
What goes in a callout is prose: a list, a link, a mention, a table, a line of
code, and a fence could hold none of them. It is a remark plugin for the reason
mentions are one, and the element stops being a `blockquote` on the way out
because it is not a quote: `hName` makes it a `div`, so no rule written for a
quote reaches it and no landmark is opened in the middle of a message. The
label goes in as the first child, so a screen reader reads *Needs you* where a
sighted operator sees the box, with no ARIA holding the two together.

**Five markers, two registers, two words.** `IMPORTANT`, `WARNING` and
`CAUTION` draw the amber box; `NOTE` and `TIP` draw the quiet one. The amber is
the app's one accent and it means here exactly what it means in the rail, on
the desk and in the menu bar: a person has to do something. That is also why
the label is the app's word rather than the model's. An agent that writes
`[!CAUTION]` and one that writes `[!WARNING]` mean the same thing, and drawing
both of their words is an operator learning a vocabulary that decides nothing;
what they need off the box is whether it is for them, which is what *Needs you*
says, in the words the rail already uses for an agent that is waiting on
somebody.

The amber box is the only filled panel in the reading column. That is the token
block's own rule rather than a hole in it: hierarchy here is carried by size,
weight and air, and the single exception is the accent that means answer me. A
quiet callout gets the hairline and no fill, because a second tint would make
the first one furniture.

**And the prompt argues against it in the same breath.** An agent told it can
draw an amber box draws one around every paragraph, which is the same failure
the chart section is written to avoid and has the same fix: a reply with three
boxes in it is a reply with none.

## A reply can be a figure, and a figure is a fenced block

An operator asking for last quarter by region gets four numbers, and four
numbers written as `Enterprise: $810K` lines is a shape they have to hold in
their head. What they wanted was to see which one was biggest, which is a
picture. So a reply can carry one.

**A fence, not a tool call.** An agent has the numbers in hand at the moment it
writes the sentence about them, so a chart it asks for through a tool call is a
round trip spent sending back something it had already finished computing. A
fence costs nothing, streams in with the rest of the reply, and needs no change
to the runtime at all: `as_plain_text` already returns the text of a message, so
the record, the prompt, the dedup fingerprint, search and a peer's copy all keep
working exactly as they did. It also means the agent can read back what it drew,
and an agent that cannot see its own last chart draws it again.

Three tags are figures (`chart`, `graph`, `plot` for the first, `html` or
`artifact` for the second) and every other fence is source, which is the rule
that keeps ```python from turning into something nobody asked for.

**Guaca draws the chart, from a spec.** `lib/chart.ts` reads a model's JSON into
a value that cannot be wrong (a pie with two series is not a thing that file can
produce) and turns it into coordinates; `components/Chart.tsx` turns those into
elements. Neither half touches the DOM, which is the point: jsdom performs no
layout, so a chart that worked out its own geometry from a measured element
would be exercised by no test in this repo and checked by nobody but the
operator. Every geometry decision this app makes is a pure function with a test
on it.

The spec is deliberately the shape every plotting library uses: `type`, `labels`,
`series: [{ name, data }]`. That is not a lack of imagination. A model has seen
it ten thousand times and writes it right first try; a novel schema of our own
would be a thing every agent has to be taught in a prompt it is already
skimming.

**A figure that will not draw shows what was asked for and why.** Not instead of
it. The operator needs to see the request to know what their agent thought it
was showing them, and the agent needs a sentence it can act on next turn: every
refusal in `readChart` names the field and the fix, because "invalid chart"
costs a whole turn and teaches nothing. And a spec that has not finished
arriving is neither drawn nor refused. A reply lands a token at a time, so a
chart spends most of its life on screen as half an object, and called an error
that is a red box under every figure for a second, which teaches an operator the
feature is broken. `looksComplete` counts braces outside strings; until they
balance, the figure says it is still drawing.

## A chart's colors are the output of a check, not a decision

Every other color in `styles.css` is a judgment somebody can revise. The eight
series hues are not. What makes a chart readable to a red-green colorblind
operator is that *neighboring* slots stay far apart, because neighbors are
what touch in a stack, sit side by side in a group and cross in a line chart,
and no one can check that by looking.

So the order was not chosen by looking. The eight hexes are a documented set;
what this app chose is the order, by enumerating all 40,320 of them and keeping
the 160 that clear every gate on this app's own two surfaces. Guaca opens on
green because Guaca is green, and it cost nothing: of the orders that pass, this
is one of the best, and it beats the set's own default on the gate that scatter
needs.

`palette.test.ts` recomputes all of it from the hexes themselves: sRGB to
linear, Machado-Oliveira-Fernandes at full severity, distance in OKLab. The
figures in `palette.ts`'s comment cannot drift from the values they describe,
and a hex nudged because a screenshot looked slightly off fails the suite. That
is the intended experience.

**Nine hues is not an option.** A generated ninth is indistinguishable from one
of the eight under colorblindness, so a ninth series is refused with the fix in
the sentence: fold the tail into an "Other". A pie past six wedges folds itself,
because past six nobody is comparing them, they are reading the big ones and the
remainder.

**Every chart carries its own numbers, and that is not a fallback.** The Figures
table beside the source control is how a value is read by a screen reader, by an
operator who cannot separate two of the hues, and by anybody who wants the
figure rather than the shape. Three light-mode hues sit under 3:1 against the
surface, which is allowed only because the table and the direct labels are
there; a change that drops either has to change the palette too, and
`palette.test.ts` asserts the debt so it cannot be dropped quietly.

The drawing itself is one `role="img"` with one sentence on it, and nothing
inside it is focusable. That is deliberate: a `role="img"` subtree is invisible
to a screen reader by definition, so labels on the bands would be announced to
nobody, and tab stops would put twelve invisible rectangles between the message
above and the message below for a readout the table already holds in a form
somebody can actually read. The readout enhances; it never gates.

## A page an agent wrote runs somewhere else entirely

A chart is drawn by Guaca and needs nowhere to run. A page is different: asked
for a diagram, a layout or a small thing to click, an agent writes markup and
script, and the only honest way to show that is to run it.

Not in this document. The webview's policy is `script-src 'self'` and it stays
that way. A frame pointed at `srcdoc:`, `blob:` or `about:blank` inherits the
policy of whoever framed it, so the page would draw and its script would
silently never run: an empty rectangle that passes every test, which is the
worst thing this app could ship and is the same failure `FileCard` already has
a note about.

So a page gets an origin of its own: `artifact.rs`, a loopback server serving
one document by the SHA-256 of its own bytes, on a port already in the app's
`frame-src` because the computer viewer needed one first. It is deliberately not
part of `proxy.rs`, whose request parser is where the token that reaches a
running machine gets decided; a second unrelated route in front of that is a
branch nobody wants to reason about at three in the morning.

**What the page may do is the whole argument.** It is the least trustworthy
content in the app: written by a model that may have read a hostile web page
earlier in the same turn. It may compute and it may draw, and it may not talk to
anybody. `default-src 'none'` closes every fetch, socket, remote font and remote
image, and `<img src="https://…/?data=">` is the cheapest exfiltration there is.
`script-src 'unsafe-inline'` lets its own script run and nothing loaded from
elsewhere. `sandbox allow-scripts` on the header and on the frame gives it an
opaque origin with no same-origin access, and the two values must never gain
`allow-same-origin` beside `allow-scripts`: together they let a page take its own
sandbox off and reload out of it.

Nothing is persisted. The message that carried the document is the record; the
server holds a copy of the last two dozen while a transcript is drawing them,
and a restart re-registers whatever is on screen.

**The page reports its own height, because nothing outside it can measure it.**
A cross-origin frame cannot be read, so `artifact.rs` prepends a bridge that
posts its height on every change. Prepended rather than appended: a model's page
is exactly where an unclosed tag lives, and an unclosed tag swallows everything
after it. The parent trusts the message by the window that sent it and by
nothing else, because an opaque origin reports itself as `"null"` and checking
the origin would either reject every real message or accept every forged one. It
clamps what it is told, too: a page claiming a height of a hundred thousand has
made itself the whole channel.

**A page is framed once, whole, and not while the reply is still arriving.**
This is where the two figures come apart. A chart is a pure function from a spec
to coordinates, so redrawing it on every token is free and is what makes one
assemble itself on screen; that is the feature. A page is drawn by registering a
document and pointing a frame at the address that comes back, so the same
treatment is a reload per token: a round trip each, a fresh entry each in a store
that holds two dozen, and a frame that throws away whatever the operator had done
in it every sixteen milliseconds. So a `html` fence in a live bubble says
*Drawing…* until the reply settles. `live` is a prop on `Markdown`, set by the
one component that draws a body still being written.

## A page can hand one value back, and Guaca is what carries it

A page that can be worked and cannot answer is a dead end. The operator picks one
plan out of four, drags a range to $450K, ticks six of the nine rows, and none of
it reaches the agent that drew the page: they are left retyping in the composer
what they just expressed by clicking, which is the work the page was supposed to
save.

So the bridge defines one more thing, `guaca.answer(value)`, and it is not a hole
in the paragraph above. It reaches no network. It posts to the window that framed
it, which is the one channel an opaque origin has and the one the height reporter
has always used, and **what happens next is Guaca's decision rather than the
page's**: the renderer draws the value below the frame, in the app's own chrome,
and waits. The page fills a form in. A person sends it.

That ordering is the whole safety argument and it is not ceremony. A transcript
re-frames a page whenever it draws one, so a page that could send by itself would
send again every time it was scrolled past, and every send is a turn the operator
did not ask for and does pay for. It is also the rule this app applies everywhere
a model's words would travel under the operator's name: shown before they go,
drawn as text, never as markup. Same line `domain::approval` draws between a
permission and a question, one level out.

**The page hands back a value, never a sentence.** `answerMessage` is Guaca's
wording around the page's JSON, so nothing the page wrote can arrive as an
instruction in the operator's voice. What is sent is an ordinary operator
message: `Trust::Operator`, in the record, searchable, readable back by the agent
that drew the page. It is theirs because they read the value in the strip and
pressed the button, and what they are vouching for is a value they could see.

The value crosses as JSON *text* rather than as a structured-cloned object,
because the string is what is shown, what is capped and what is sent. A value
that will not survive `JSON.stringify` fails inside the page, where the page is
told about it by the `false` coming back, instead of arriving as something the
app has to decide about. It is capped at 4,000 characters, with the refusal
saying so: an answer is a choice, not a document.

**Answering replaces, and never queues.** A page that calls `guaca.answer` on
every drag of a slider is doing exactly the right thing, and what the operator
sends is what the page last handed back, which is what they are looking at.

**Nobody to answer means no strip at all.** `Answering` is a context that is null
everywhere and provided by one surface: the channel, where the operator is one of
the two participants and their next message has an obvious recipient. A page
behind a search hit, inside a pair's thread or in a document preview draws and
runs exactly as before; a Send button in those places would be a control that
cannot say who it sends to.

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

## A mention is one thing, in the box and in the message

An `@` that names an agent is drawn as a chip: a rounded tint behind the name,
so an operator can see at a glance who a message actually reaches. It happens in
two places, and the point is that they agree. What the box paints while a draft
is typed is what the transcript paints after it is sent.

**Resolution against the roster is the whole rule.** `@Critic` is a mention
because there is a Critic; `@lunch` is a word somebody wrote. Nothing else can
decide it, because `@` in front of a word is also a handle, a Python decorator,
a shell flag and half an email address, and a chip around one of those tells the
operator this app knows something it does not. `splitMentions` in
`lib/mentions.ts` is the one answer, and it opens on the same word-boundary rule
the typeahead does, so what lights up is exactly what could have been completed.
It takes the longest name that fits, or a crew holding both "Head" and
"Head Chef" gets a chip on the first word and the rest of a name reading as
prose. It stops at the end of the name rather than inside a longer word, which
is why `@Critical` is nobody.

Two rosters, and the difference is which question is being asked. The composer
completes against the live crew, because a mention is about to become a
delivery. A transcript resolves against everyone the store holds, retired agents
included, because it is history: an agent that has since been let go was still
an agent when somebody wrote to it, and dropping its name back into prose
rewrites what the message looked like the day it was sent.

**In a message, the chip is a remark plugin, not a pass over the rendered
output.** A mention turns up inside a bold run, a list item, a heading and a
table cell, and the mdast is the one place all four are the same node. It is
declared with `hName` and `hProperties`, so what `react-markdown` builds is
still plain hast and the rule about raw HTML is untouched: no `rehype-raw`, no
markup from a model reaching the document. Code and fences never reach the walk,
because they carry a value rather than text children, and links are skipped on
purpose. A chip inside an anchor is two things claiming one click, and
`remark-gfm` autolinks a bare email address, which is the one place an `@` is
guaranteed not to be a mention.

**In the composer, the chip is painted on a copy of the draft, underneath the
operator's own characters.** The textarea is still the text. A contenteditable
would make a mention a real element that backspace deletes whole, and it would
also cost the caret, the undo stack, an input method and every native thing a
box does. Making the textarea's own text transparent and reading the copy
instead is the other way, and it is worse where it matters: a selection over
transparent text is an empty highlight, and the box an operator drags across is
the one they are about to retype.

So the copy carries the pill and nothing else. That is a constraint on the chip
rather than a note about it: `.mention` may not declare a padding, a weight, a
letter-spacing or anything else that moves a glyph, because the copy would move
and the characters on top of it would not. The room around a name is a spread
shadow, which takes up none. `styles.test.ts` reads the two elements back
through the cascade and compares every property that decides where a character
lands, and it reads `.mention` out of the source to check it declares nothing
that shifts one. Neither is visible in review and neither renders in a window
with no layout, which is the whole reason they are assertions.

The copy is out of flow, so the textarea is still what gives the row its height
and still what grows as the draft does. Past the twelve-rem cap the box scrolls,
and the copy is scrolled with it from two places: the scroll event, and the
effect that resizes it. Typing at the bottom of a full box moves the view
without the operator having scrolled anything.

## The rail is arranged by hand, and lends the top of a section out

Two orders, not one. `railOrder` on the card is the arrangement: the operator
drags a row into place and it stays there, which is what makes reaching for one
a thing you can learn. Activity is a loan on top of it. An agent that is working
is lifted to the top of its section for exactly as long as it is working, and the
place goes back the moment it stops. `awaitingApproval` outranks working, because
it is the one state the operator is the fix for; `paused` scores nothing, because
it is not work in progress but a row that will not move until somebody moves it,
and lifting it would hold the top indefinitely. A pinned row is above all of
this: it heads its crew and never lifts at all, because being findable in one
glance is what a pin is for, and a row that moves when its agent gets busy is the
thing a pin exists to stop.

Before this, the rail was ordered by who spoke last and by nothing else. That is
an order nobody chose and one that moves under the hand reaching for it: every
reply rewrote it, so no arrangement could survive a conversation. Recency did not
go away, it just stopped being the whole answer: it is what separates two lifted
rows, and it is still the text in the right-hand column.

`lib/rail.ts` holds all of it, and holds it as pure functions over a list, so the
same rules order a section and decide where a drop lands. The rail draws the
arrangement itself while a drag is in progress, with nobody lifted. Dragging is arranging: a row dropped below a peer that is
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
a hand cannot aim at; the direction the row traveled says which side it belongs
on, and dragging down past something and dragging up onto it are exactly what
those two look like while they are happening.

**Pointer events, not HTML5 drag and drop.** `dragDropEnabled` is what lets a
dropped document reach Rust without its bytes entering the renderer, and it is
the same setting that stops `dragstart` firing inside the webview on some
platforms. A rail that only rearranges on macOS is not a feature. A press
becomes a drag after five pixels, because a row is a button first.

The one thing a drag does that a hand on a trackpad should not have to aim for
is crossing a crew boundary, so that is in the agent's menu, behind one row that
opens the crews beside it. A row per crew was there first and is what the submenu
replaced: the crews are the one part of that menu whose length nothing bounds, so
a workspace with eight of them pushed clearing a history and deleting an agent
off the bottom of the window, under the two items nobody wants to hunt for.
Ordering within a crew has no menu item at all. "Move up" and "move down" were
two rows spent on the gesture the rail already answers, in a menu where every
other row is something a drag cannot do.

## A group is a place you can be inside

Two views of one rail. In the overview every group has a heading and its members
under it, which is what it always was. Clicking a group's circle takes the rail
inside it: one crew, its name and controls given a line of their own, and the
same list in the same order, because a crew's pins are at the head of it in
either view. Neither view gives them a heading: everybody drawn under a crew is
in that crew already, and a heading over one or two rows would divide nothing.
The mark on the row is what says which rows are pinned.

The circles live in a column of their own on the far left, and that column is
the second change here. They were a strip inside the rail: a wrapping row of
circles that fit four across, went to a second line at five and a third at nine.
It did not overflow and it did not scroll. It ate the rail's own height, so a
workspace with a dozen crews spent the top third of its agent list on the
navigation for it, and four was a hard wall nobody had chosen. Upright, the axis
is the one there is room on, and the crews that do not fit scroll instead of
folding.

Two things follow from the column that the strip could not buy. The crews are on
screen wherever the operator is, including while the rail is inside one of them,
which is what lets a circle carry a permanent statement about the crews nobody is
looking at. And "which crew am I in" stops being something the rail has to
answer, because the answer is a lit circle in a fixed place.

The circles are faces, and the name is beside them. A crew is recognized by who
is in it long before its name is read, which is true and was never enough on its
own. The cafeteria is a copy machine with a fixed avatar and a fixed color per
preset, so two crews hired from the same counters draw the same faces in the
same colors, differing only by the few degrees of lean `lib/orb.ts` takes off an
agent id; and above six members every crew draws the same six and a count. The
operator's way out was to click through the column reading names, which is the
navigation the column exists to replace.

So pointing at a circle names it, and says the two marks in words underneath.
The name is still not drawn *under* the circle: the column is four rem wide and
a name cut to fit that is a name nobody can read. It is laid over the app beside
the circle instead, which costs the column no width, the layout no reflow, and
can hold a name long enough to wrap rather than ellipse. `src/components/OrbTag.tsx`.

This is what `title` was already trying to do and could not. The native tooltip
waits about a second, so sweeping a column of twelve shows nothing; it never
appears on a keyboard focus, so the column was reachable by pointer only; and it
is suppressed for the whole of a drag, which is the one moment the circle is most
load-bearing, because dropping an agent onto an unnamed circle is how somebody
ends up in the wrong crew. The tag opens on hover, on focus and during a drag.
`title` is kept for the operator who has stopped and is waiting for it.

The words under the name are `presenceNote`, which `presenceLabel` also composes
its sentence from. Two functions would be one circle described two ways, and a
tag saying "3 agents" under a label saying "working" is the version where only
one of them is right. The tag itself is out of the accessibility tree: the button
already says the same words as its own label, and drawn into the tree twice a
crew is announced twice, the second time as text nobody can reach.

Both rail headings carry the full name on a `title` too. They ellipse whatever
does not fit, and after clicking a circle the heading is the only place the name
is drawn at all, so a crew called "Customer research, EMEA" was two words and a
hyphen wherever the operator looked.

How the faces stand is the size of the crew. One is a portrait, two stand side
by side, and three to six stand on a ring, so a strip of crews is a strip of
different badges and the number of people in each is legible before any single
face is. Every crew of two or more used to draw the same square of four, which
made the circle read as the app's own mark rather than as these agents. The
seating is `src/lib/orb.ts`, in fractions of the ring rather than in pixels, and
it is sized against the ink instead of the box the ink is drawn in: a character
fills 62% of its box across and 83% of it down, so the tight axis is vertical
and `orb.test.ts` holds every ring to the catalog's own construction spec rather
than to a number copied out of it.

Six is where it stops. A seventh face is a smudge and it makes the six already
there smaller, so the rest are a count, hung off the rim rather than laid on it:
a ring with a seat in every quarter has nowhere inside it for an opaque chip
that is not somebody's face.

Each circle does two jobs, which is why it is a circle and not an item in a menu.
Clicking it opens the group. Dropping an agent on it puts the agent in the group,
so the shortest gesture for moving somebody between crews is the one that also
says which crew, and it is on screen for the whole of the drag. A column makes
that gesture better rather than worse: the target never scrolls out from under
the hand, and pointer events cross a column boundary for free where HTML5 drag
and drop would not.

**What the circle says, it says in two marks, and they are deliberately not
one.** A ring means somebody in this crew is working, with no number on it,
because how many agents are mid-turn is not a thing anybody acts on. A count in
the corner is how many turns in this crew are parked on the operator, and that
one is a number because three and one are different amounts of work and a dot
says neither. The same two states the menu bar's glyph already distinguishes:
that reports them for the workspace while the window is shut, and this reports
them per crew while it is open. One rule, two granularities, one fold, in
`src/lib/presence.ts`. It is counted off the activity map rather than off the
pending requests, and the two agree by construction: an agent runs one turn at a
time and a turn parks on one request.

What is deliberately not there is unread. Discord's rail carries a dot for
messages you have not seen, and Guaca does not know which those are, because
nothing records what the operator has read. A dot derived from "this channel is
not the open one" would be lit on every crew but one, forever. Unread is a
persisted per-channel marker and a migration, not a badge.

The column is still absent while there is one group, which is the state most
workspaces are in, and it is the same rule the strip had. A column offering a
choice of one is a drop target that cannot move anybody anywhere, and with one
crew the rail *is* that crew: every row of it is already on screen saying what
the badge would.

The group being looked inside is in the store rather than in the sidebar, because
it and the open channel have to agree, and both are in the store. One invariant
holds them together: the rail draws the row of whatever the pane is showing. A
search hit or a click on the flow board can land on an agent in another crew, and
`select` repairs that by following the agent into its crew. It used to drop out
to the overview instead, which was the only honest answer while the crews were a
strip inside the rail: the overview was the one view where every row was
drawable. With a column that is on screen whichever crew you are in, following
is both possible and less, because one lit circle moves instead of the whole
rail changing shape.

`focusGroup` repairs the same invariant from the other end, and lets go rather
than following. The asymmetry is the point: `select` is the operator naming an
agent, so taking them to that agent's crew is what they asked for, and
`focusGroup` is the operator naming a crew, so following the channel back out of
it would undo the click. The pane falls back to the activity feed, which belongs
to no crew and is never closed.

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

## Deleting a group deletes the crew, and the machines they were renting

One button, two calls. An empty group loses a row; a group with four agents in
it loses the four agents, their computers, their browsers and the profiles
holding those browsers' cookies. Which call a click makes is decided by the
roster, and the confirmation names the count before it is pressed.

It was two buttons, and the one for a populated group had exactly one outcome:
an error telling the operator to go and delete four agents by hand and come
back. That is not a refusal protecting anything. It is the work, described. An
operator winding a crew down wants the crew gone, and the half-finished state
that path produces is a group with one agent left in it and no reason to keep
that one either.

What a disband is not is a bigger *Start fresh*. A reset keeps the crew and
takes what it accumulated; this takes the crew. They sit next to each other and
only one of them is reversible in the sense that matters: a computer is rented
from a provider, and killing it is not a row in a table.

Each agent goes exactly as `delete_agent` sends one, through `retire_agent`,
which is why they are the same function. Transcripts stay readable, because a
delete at the scale of a crew is not a different rule about history: a message
survives its author here for the same reason it does there. What is gone is
everything the agent held privately, which is its memory, its schedule, its
sign-ins and any standing permission the operator gave it.

The refusal that remains is the first group, which cannot be deleted because
every agent has to be in one. `Store::group_for_removal` asks that question
separately from the emptiness check, and the command asks it *first*: a disband
that killed four computers and then found the group could not go would have
spent the irreversible half of the work on a call that fails. Terminated agents
are not in the crew it takes. Their machines are already destroyed, and handing
one back would mean a second kill against a sandbox that is not there, which the
operator would be shown as the failure of the disband.

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
cannot discard a half-typed endpoint. Plugins and Repositories are both disabled
until the group exists, because a sign-in, a credential and a linked directory
all have to belong to something.

Repositories is a section rather than a third block under Plugins, and the two
are near-neighbors on purpose rather than by accident: both are a thing given to
the crew and then handed to named agents, which is the only shape they share. A
plugin is a server somewhere that this crew signs in to. A repository is a
directory on this machine that it writes in, and it is the one place in the app
where the operator hands over their own source. Stacked under Plugins it was
reached by scrolling past two sign-in panels, and the panel had to draw its own
heading to be findable at all, which is how a section earns itself.

The plugins section holds two decisions, not one. Connecting is the crew's
sign-in; under each connected row is who in the crew may use it, which is every
agent until an operator says otherwise. It is written the moment it is clicked,
like the Connect and Disconnect buttons above it: a draft nobody submitted
would be a permission the operator believes they granted. The reasoning is in
`docs/PLUGINS.md`; what the panel owes the operator is the sentence saying who
a plugin is currently offered to, including the honest one for a plugin
narrowed to nobody, which is otherwise indistinguishable from a working row.

The foot follows from that. A Save under a panel that has already written what it
was told is a button offering to save work that is saved, beside a Cancel
implying it could be taken back, and both sit there at the moment the operator is
deciding whether the plugin they just connected went through. So the Save is
drawn while something is genuinely waiting for it — a name, a provider, a limit,
a key that was typed — and the button next to it says Close rather than Cancel
when there is nothing to cancel. It is read across the whole dialog rather than
the open section, for the reason the state lives in the shell: an endpoint typed
on Provider is still unsaved while the operator is on Plugins, and a Save that
went missing on the way past is how it would get lost. A group that does not
exist yet is always waiting, because there is nothing to compare it with and
Create is the only way out that leaves one behind.

Three rows say who pays, and the first is "follow the app settings", which is
where every group starts. The second is the ChatGPT subscription, and the group
editor cannot sign in: there is one sign-in on this machine, it is performed in
Settings, and what a group chooses is whether to spend it. The rest is the same
preset list Settings draws, from the same file, because an endpoint that is off
by a path segment fails the same way whoever typed it.

Two model fields, not one, and only the one belonging to the resolved provider
is on screen at a time. A model belongs to a provider, which has disjoint names
from the other and will not accept them, so a crew that tries the subscription for
an hour and moves back has to find its endpoint model where it left it. Test
connection is here for the same reason it is in Settings, and it sends what is on
screen resolved over the app settings, which is what the next turn would do.

## Pinning is the head of a crew, and nothing else

It does not bump the card version, because the version is how a peer notices a
card changed under it and nothing a peer can read has. `railOrder` is the same
kind of fact and is kept the same way, and both live on the agent rather than in
a preferences blob because they have to die with it: a name is free to reuse the
moment an agent is deleted, and whoever takes it next must not inherit a pin or a
place. A pinned agent is still in its group in every other sense: same wall, same
bill, same peers, and counted in the crew it heads.

The pins used to be a section of their own, above the groups and spanning them.
That made a pin the one arrangement that came undone on the way into the crew it
was arranging: inside a group the rail draws that group and nothing else, so the
section holding the row was not on screen, and pinning something while looking at
the list it was in moved nothing until the operator went back out to the
overview. A pin is a band at the head of a crew now, drawn wherever that crew is
drawn, which is both views and the same order in each.

The band is why the row that a drag lands on decides both things: the crew it
joins, and whether it is pinned. Dropping onto a pinned row pins the dragged
agent, dropping below the band unpins it, and dropping on a group says nothing
about the band and therefore changes nothing about it. A pin is a standing
instruction about one agent, and moving somebody between crews is not a decision
to drop it.

The mark on the row is load-bearing. A crew whose pin is also the row the
operator arranged at the top looks exactly like a pin that did nothing, and being
first is not a state anything on screen can say by itself.

An agent is drawn once, wherever it is drawn: two rows for one agent would be two
nodes in the sidebar's `rowRefs`, and the wire would have to pick one to throw a
message at.

## Making something is one plus, at the top of the rail

The rail's footer used to carry four rows: the cafeteria, a new agent, a new
group, and settings. Two of those are places you go and two are things you make,
and the two kinds sat together because the footer was where there was room. It
also grew by a row every time something new became makeable, which is a footer
that gets worse as the app gets better.

Now there is one plus, beside the wordmark at the top of the rail, and it lists
what can be made. The footer keeps the two places.

It spent a release at the end of the channel header instead, on the argument
that the rail is a list of agents and making one is about none of them. That is
the wrong half of the argument, and the report that ended it came from the
person who wrote it: an agent is a row in the rail and a group is a heading in
it, so the rail is where somebody looks to add one, and a plus glyph at the far
right of the reading column, beside the agent's own actions menu, is not
somewhere anybody looks at all. The channel header was also the one place the
plus could not be drawn on an empty workspace, because there is no channel open
to hang a header on, and that is the state where making an agent is the only
thing left to do.

It is the shared menu, so it closes on Escape, on a click that lands away from
it, and on anything that moves the button underneath it. The listeners are bound
by name rather than as inline arrows, which is not a style point: `removeEventListener`
matches by reference, so two arrows leave a listener behind on every open.

## The cafeteria is a copy machine, not a registry

Twenty-one agents written out once, well, so that a new workspace is a few
clicks rather than an afternoon of typing. They are named after jobs rather than
functions: "Chief of Staff" and "Paralegal", not "Manager" and "Reviewer". A
role carries duties and refusals a function label does not, so the operator does
not have to supply them in the prompt, which is the work this removes. Titles
are capped at three words because peers resolve each other by whole name and the
composer's `@` typeahead gives up after two spaces, so a longer title is an
agent nobody can delegate to.

The six counters are the departments of a small software company, and the set is
chosen against one test: a crew assembled from it can take a change the whole
way without the operator writing a prompt. Somebody decides what to build,
somebody draws it, somebody writes it, somebody reviews it, somebody ships it
and watches it, somebody documents it, somebody tells the market. A gap in that
chain is exactly the prompt an operator ends up writing by hand. That is why
design, reliability, security and growth are hires of their own rather than
duties folded into the engineer and the marketer: an agent doing five jobs has
no refusals, and the refusals are most of what a role is.

Product and engineering used to be one counter, and nine cards under one heading
is a wall rather than a menu. They are two now, because they answer different
questions: what to build is decided at the first and settled at the second.

The catalog cannot grow past the character cast, and that is a rule rather than
an accident. A crew is whatever subset the operator ticked, so two presets
sharing a silhouette are two agents that look the same in one rail, and the
silhouette is the only thing that separates them at 22px. A new preset is a new
ingredient in `avatars/catalog.tsx` or it is not a preset. `lib/cafeteria.test.ts`
is where that is enforced.

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

## Deleting an agent is a thirty-day hold, and the compost is where it waits

Deleting used to be one act. The click killed the computer, destroyed the
browser and the profile holding its cookies, removed the memory the agent had
spent months writing, deleted the schedule, the working notes, the sign-ins and
every standing permission the operator had given it, and marked the row
terminated. All of that behind one menu item, with nothing between the press and
the loss but a confirmation. There was no way back and nothing on screen said
there had been.

So the destructive half waits thirty days. What a delete does now is stamp the
row and stop the actor: the agent is out of the rail, out of the directory,
unreachable by any peer, and everything it held privately is exactly where it
was. `Runtime::discard_agent` is the first half, `Runtime::purge_agent` is the
second, and the thirty days between them are the feature.

**It is a column, not a fourth lifecycle.** An agent in the compost is
`Terminated`, which is what every other part of the store has always meant by
deleted, and `discarded_at` is what says it can still come back. That is not
timidity about the type: there are fifteen queries in this app that ask
`lifecycle <> 'terminated'` and every one of them is still asking exactly the
right question, including the partial unique index that frees the name. A
fourth state would have been fifteen places to remember, each failing silently
in a different way — a composted agent in a directory listing, in a crew count,
in a disband, in the roster a peer is told to ask. `NULL` in the column is both
ends of the wait: an agent nobody deleted and one whose thirty days are over.
The lifecycle tells those apart.

**A restore comes back paused, and may come back renamed.** Paused because the
wait is what makes it a different question from unpausing: an agent restored
three weeks on returns to a schedule that has been coming due every morning
without it and to peers that carried on, so starting it is a decision the
operator takes on a row that is already drawn as stopped. Renamed because
throwing an agent out frees its name at once — that is the whole point of the
partial index — so the crew may have hired somebody into it since, and
`copy_name` settles it exactly as a duplicate does. Refusing the restore instead
would show the operator a unique-index violation for a button whose job is to
succeed.

**The machines are released, not destroyed, and the sweep has to know that.** A
sandbox is put to sleep and keeps its disk, because that disk holds the accounts
the operator signed it in to and nothing else can sign them in again; a browser
is closed, which is what writes its cookies back to the profile. Both are what a
restore has to find. `claimed_sandboxes` therefore counts a composted agent as
holding its machine, which is the one exception to "only a live agent holds a
claim": swept on the old rule, that machine would be killed inside the minute
and a restore three weeks later would hand back an agent signed in to nothing.

**Nothing about the transcript changes.** What an agent said stays readable in
every channel it said it in, through the hold and after the sweep, for the
reason it always did: hard-deleting punches holes in transcripts that had
nothing to do with this agent. What goes at the end is what the agent held
privately.

**A disband does not use the compost.** Deleting a group takes the place a
restore would come back to: `delete_group` files whatever is left under the
default group, so a composted crew would be offered a restore into a crew that
no longer exists, holding sign-ins belonging to a group whose credentials went
with it. A disband purges each agent outright and its confirmation names the
count, which is what makes it the irreversible one on screen as well as in the
code. See *Deleting a group deletes the crew* above.

**The sweep is its own loop, and runs hourly.** Not a third statement in the
scheduler's: a routine is late by however long the tick is, so that one is paced
in seconds, and a thirty-day deadline is not made better by being met to the
second. It also makes provider calls when it finds something, which a schedule
sweep should never wait behind. Swept once before the first wait, so an app left
closed for a month empties on the next launch rather than an hour into it.

The panel is the cafeteria's shape at the cafeteria's opposite end, and the two
being alike is deliberate: one is hiring and one is letting go. Where the
cafeteria is a grid of tiles to browse, this is a list of decisions, each with
its clock and each saying what is still inside the agent — its memory, its
working notes, its schedule, its sign-ins. That sentence is the reason the panel
exists. None of it is visible at the moment somebody presses delete, and this is
the one screen where they are deciding whether they meant it.

The rail's footer draws the compost only while there is something in it. That is
what keeps the footer two rows for an operator who has never deleted anybody,
and it is also how the feature is discovered: the row turns up at the moment it
has a reason to, which is the moment somebody has just deleted an agent and
might want it back. The count is on the row because whether there is anything in
there at all is the question it is read for.

`COMPOST_DAYS` lives in `domain/agent.rs` and `Compost.test.tsx` reads it out of
the Rust rather than restating it. The panel draws a countdown to the moment an
operator's memory is deleted, nothing else in the build compares the two sides,
and a mirror that has drifted is a promise the app does not keep. Same rule, and
the same reason, as the memory cap.

## The model field suggests three, and is still a text box

An agent's model is any slug its endpoint accepts, and the endpoint is the
operator's to choose. That is right, and it also means somebody who has just
decided they want an agent that reads contracts is looking at a blank box with
no way to find out what to put in it. Under the box are three models, and
pressing one fills the box in. It is the same edit as typing: nothing is saved
by it, and nothing about the field changed.

Deriving what the agent is for is a keyword scan over the name, the skills and
the instructions, in `src/lib/roles.ts`. Not a model call: it runs on every
keystroke in three fields, it would cost money and a round trip to answer a
question a keyword answers, and it would answer it differently on two openings
of the same dialog. It is also the one place a wrong answer is invisible, since
an operator cannot tell a considered "marketing" from a guessed one. Keywords
are free, the same twice, and wrong in ways a person can see and ignore.

Saying nothing is the common answer and not a failure. OpenRouter classifies
traffic into twelve use cases and most agents are none of them: a Manager, a
Router, an Inbox. There is a floor under the evidence and a tie is not a winner,
because an agent described as equally legal and financial has no single best
model, and a confident wrong suggestion is what teaches an operator to ignore
the right ones. Sales is the one word bent to fit: OpenRouter ranks nothing for
it, and an agent called Sales is the second thing anybody builds here, so sales
vocabulary scores into marketing rather than into silence.

Nothing is offered unless OpenRouter is what pays for that agent's turns,
resolved crew over app the way the backend resolves it. A slug only means
something at the endpoint it was ranked at, and `anthropic/claude-opus-5` put
into a field pointed at `api.openai.com` is a refusal by name on the next turn,
an hour after the button was pressed and with nothing connecting the two.

The order is not the one OpenRouter returns by default, and that is the whole
design. Its default is how many tokens it routed to each model for that kind of
work, which is bulk traffic: the same cheap high-throughput model tops eleven of
the twelve, so a picker built on it suggests one model for every agent while
claiming a different reason each time. So the use case chooses the pool, the
models people actually send that work to, and capability orders the pool. Every
row carries its price, because capability ordering ignores price and the most
capable model in a pool is regularly the dearest thing in it: a one-click swap
that hid the number would be a one-click way to make every turn forty times
dearer. `llm/catalog.rs` has the rest, including why the twelve are checked
before a request is spent rather than after.

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

## Settings is nine places, because it stopped being one subject

An endpoint, a set of limits, a machine's credentials, how large the window
draws and what is allowed to interrupt you are five different questions, and one
scroll made the operator read all five to change one. So it is a nav and a pane,
on the Cafeteria's shape: a panel that owns its own height, a head and a foot
pinned to it, one scrolling half.

Two of those nine are defaults rather than orders. Whatever Provider and Limits
say is what a group falls back to, and a group that answers for itself is not
affected by either. What stays app-wide is what is genuinely one of: the
operator's name, the machine and browser accounts, and everything about how the
app looks and when it may interrupt.

One of them is optional in a way none of the others are. Account is the only
pane that talks to a service Guaca's author runs, and an install that never
opens it never sends that service a request: both of its reads happen when the
pane is opened rather than at startup. That is the shape the feature has to keep.
Most people will never sign in, everything else in the app works identically
either way, and the pane says so in its first sentence rather than leaving an
operator to work out whether they are missing something. What signing in buys is
one thing that genuinely cannot be done from here: an OAuth client for a service
that will only issue programmatic access to a registered application. A client
secret shipped inside an open-source download is not a secret, so the client has
to live somewhere else or not exist. `docs/ACCOUNT.md` is the long version.

Every value lives in the shell rather than in the pane that draws it, and that is
not tidiness. The shell is unmounted when the dialog closes, so a pane holding
its own state would discard it on every section change: typing an endpoint,
glancing at Limits and coming back would silently lose the endpoint. Save stays
in the foot for the same reason it used to be at the bottom of the one column —
it acts on the whole panel, not on the section that happens to be open — and it
still does not close, because the point of Test is to press it next. Test is not
beside it: it reads the endpoint, the key and the model, acts on none of the
other eight sections, and sits in the pane those three are in.

What the foot does not do is offer a Save when nothing is waiting for one. Five
of the nine panes stage nothing at all: Appearance and Notifications are kept the
moment they are clicked, Account acts at once, and Shortcuts and About are only
there to be read. A Save pinned there regardless spent most of its life under a
pane it could not affect, saying "not saved yet" over settings that were already
on disk. So it is drawn while the panel differs from what is stored, and the
whole panel is what is compared: an endpoint typed on Provider keeps the Save
reachable from Shortcuts, which is what stops the rule from turning "there is
nothing to save here" into a way to lose an edit.

A key is the one field that cannot take part in that comparison, because a stored
one is never read back. So a key box counts as an edit whenever it holds
anything, spaces excepted, which is the same rule that keeps it out of the patch.

Which is why a save puts every box that stages something back to blank. Blank is
their resting state: each one means "keep what is stored" and each has a
placeholder saying what that is, read from what the save returned. A key left on
screen afterward is indistinguishable from an edit nobody saved, which is how the
E2B key kept the Save up offering to send itself again. The three durations are
worse than that, because the runtime clamps them: a box still reading 2000 sits
under "Saved." claiming a machine sleeps after thirty-three hours when what was
stored was twenty-four. It is the same read-back the limits have always had,
finally applied to the fields beside them.

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

About says which commit it is, rather than a version. The number in
`package.json` and `tauri.conf.json` has not moved since the first commit and is
not going to: what ships here is a commit, so a version read off either file
tells an operator nothing and tells a bug report less. It used to be read
through Tauri's `getVersion`, which is exactly that placeholder arriving over
IPC. So `vite.config.ts` asks git for the short hash while the bundle is being
built and `src/lib/build.ts` is the one place it is read back from. Nothing is
asked at runtime, because the built app carries no repository to ask. A tree
with uncommitted edits on top of that commit gets `-dirty`, because an
unqualified hash sends whoever reads it to check out a commit that did not
produce the build in front of them. And a build made outside a repository at all
draws a dash: it is the same answer the pane already gave for a version it could
not read, and it is still a thing to say rather than a failure worth a banner.

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
the rail into a color scope instead of a naming convention.

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
  turn by noticing a row change color three screens up the rail.
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
every judgment below arguable in a test rather than by squinting at a corner of
the screen.

- **The glyph** is state without being looked at. An outline when nothing is
  running, filled when something is, and warm red when an agent is blocked on
  you. That last one is the only glyph that is not a template image, and giving
  up the menu bar's own light-and-dark tinting is the price of it: macOS tints a
  template image to match the bar, so a template glyph cannot have a color.
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
