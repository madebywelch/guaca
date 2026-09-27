# Characters

An agent is a creature in one of five shapes, and everything it has to say it
says by changing shape: the outline of its body, and the lids over its eyes.
There are two casts of the same creatures, one cut from pigmented paper and one
drawn with a brush, and the operator chooses which (*Two casts* below).

This is the fourth cast. The three before it were emoji, then a hand-drawn set
of creatures, then an egg with props, then sixteen vegetables, and every one of
them was the same idea: somebody draws a picture per agent and the app picks
between the pictures. The vegetables were the best of them and they still failed
the same way. They were charming at six agents and childish at sixty, every
character was a bezier somebody had to draw to a written spec, and a test had to
check the drawing afterwards because the spec could not enforce itself.

Nothing is drawn now. A character is a silhouette and a row of numbers, an
expression is another row of numbers, and the geometry is a function of the
three. A character cannot leave its box, cannot sit at a different optical
weight and cannot take light from a different direction, because none of those
is a thing a character supplies.

**There is more than one shape because one was not enough.** The first version
of this cast was a single round species varying by a few percent of stretch and
where the eyes sat, and at that amplitude a rail of sixty is a rail you read by
color. The outline has to carry some of an identity or the eyes carry all of it.
So it is two decisions now: which of five shapes, and then the lump on top.

## Where it lives

| File | What is in it |
|---|---|
| `src/avatars/silhouette.ts` | The five shapes, as one radius function each, and the two numbers they are sized against. |
| `src/avatars/form.ts` | The body. `FORM`, the types, and the maths that turns a character and a mood into points. |
| `src/avatars/Skin.tsx` | The paper relief: one outline, a shallow shadow and a lifted edge. Shared by the app and the README. |
| `src/avatars/eyes.ts` | The gaze both casts share, the blinks, and the cut cast's eye: a ball, a pupil, two lids. |
| `src/avatars/catalog.ts` | The cast, the accents, and the alias table that keeps every key an older build wrote still meaning something. |
| `src/avatars/moods.ts` | The ten expressions, the marks drawn beside a head, and `moodFor`, which is the only place a runtime signal becomes a face. |
| `src/avatars/drawn.ts` | The drawn cast: its faces, the brush, the mark on each head and the marks beside it. No DOM. |
| `src/avatars/clock.ts` | The clock every creature shares, and the one each of them keeps. |
| `src/avatars/frame.ts` | The one frame `AgentAvatar` decides and either cast draws. |
| `src/avatars/CutArt.tsx`, `DrawnArt.tsx` | Each cast's elements, and the writes it makes to them. |
| `src/avatars/AgentAvatar.tsx` | What is true right now: the mood, the look and the knock, handed to whichever cast is drawing. |
| `scripts/make-crew.ts` | The strip on the README's front page, drawn from the files above rather than redrawn. |

**The README's strip is generated, not drawn.** `./scripts/make-crew.sh` holds
one frame of eight creatures in eight moods and writes `docs/img/crew.svg`, so a
redesign updates the front page by re-running it. It has to be re-run, and it
was not: the strip was still showing a cast of vegetables three casts after they
were deleted.

## The body is a function, not a path

A creature is a closed curve through 32 radii around one center, smoothed into
cubics. The first term of every radius is its silhouette; everything a mood does
is another term added to it, or a scale applied to the point after it.

**Nothing below `silhouette.ts` knows how many shapes there are.** A cloud
kneads, leans, sags and settles through the code a circle does, because a shape
is a resting radius and a mood is what gets added. A sixth shape is a function
in one file, a row in the cast, and nothing else: no component, no stylesheet,
no branch in `form.ts`.

**The count of radii is divisible by eight, and that is load-bearing.** A
square's corners are at 45 degrees and an octagon's at 22.5, and a corner that
falls between two samples is a corner that gets chamfered off. At the 28 this
started with, the octagons drew as lumpy circles and every test still passed.
`silhouette.test.ts` holds the count and checks both shapes keep their corners.

**Motion changes the outline, never its position.** A character that slides around
inside its own box reads as a sprite being moved; one whose outline changes
reads as a thing that is alive. This is the load-bearing decision and it is why
there is no animated transform or keyframe in the drawing path. When a creature
leans, the mass leans: one side thickens and the other
thins.

**The cut cast is paper relief.** `Skin` draws three uses of the same path:
a shadow offset by (1.1, 2.3) at 18% opacity, the agent's pigment, and a white
edge offset by (-0.4, -0.5), 0.55 units wide at 45% opacity. These are fixed
material offsets in the 64-unit viewBox, so they scale with the avatar. A frame
still writes one outline, and all three layers follow it. Each instance owns
its SVG references; one creature cannot borrow another's shape.

The relief is clipped to `FORM.reach` because its offset is not permission to
draw outside the space the crew's circle allows. Eyes and attention marks are
outside that clip. The face uses dark ink; marks beside it use the reading
surface's text color so they remain visible in a dark room. `make-crew.ts`
renders the same `Skin` component for the README strip.

**The amplitudes are small on purpose.** The body breathes, leans and settles.
It does not act. An early version had the body doing the acting — a puddle for
stuck, a jagged boil for frustrated — and it read as ten different creatures
rather than as one creature in ten states. A body that emotes as hard as a face
is a body nobody can read a face on.

**`FORM.reach` is a number the rest of the app sizes against.** Nothing is ever
drawn outside it, at any character, in any mood, at any point of any cycle, and
`form.test.ts` samples the whole space and holds the geometry to it. `orb.test.ts`
seats a crew inside its group's circle against the same number, so a mood that
grew could not quietly push a face through a rim with nothing noticing.
`FORM.radius` is the resting body and is what two faces are *spaced* on, because
spacing them on the worst case would push a pair apart for a bulge that happens
a fraction of the time and touches nothing when it does.

## Five shapes, one weight

Circle, octagon, square, water drop, cloud. Each is a function from an angle to
a radius, written at whatever scale was easiest to think in, and then sized by
two rules that between them are why nobody has to balance a cast by eye.

**Every silhouette encloses the same area as the circle.** Sizing five shapes by
hand is how a cast ends up with one member that reads as the small one, so the
scaling is computed at load rather than typed in. A sixth shape is a function
and nothing else.

**And none of them rests past `CREST`.** This is the part that costs something.
The moods spend nearly all of the room between `FORM.radius` and `FORM.reach` on
the swell that follows a look: the worst frame in the whole space landed two
hundredths of a pixel under the limit, and it did that back when every creature
was a circle. So a shape with a long point or a flat underside, which are both
ways of putting the same area further out, gives area back rather than taking
room the moods need. The square gives up an eighth of its area, the drop and the
cloud a fifth each, and all three end up taller or wider than the circle rather
than bigger than it.

The two rules pull against each other on purpose. A shape that has to give up
more than a quarter of its area to fit the crest is a shape to redraw rounder,
not one to scale down, and `silhouette.test.ts` fails on it.

**The cloud is a union of balls cut off at a line.** It is the one shape here
that is not convex, and the notches between its puffs are the whole of what says
cloud rather than lump. A wobble added to an ellipse was tried first: at an
amplitude deep enough to notch, the middle puff came to a spike.

**A character varies its silhouette; it never replaces one.** The stretch and
the lobes in the cast are bounded, and separately every character's resting
outline is held under `CREST` plus a small allowance, because everything past
that belongs to the moods. Without that second bound a character that rested too
far out would not fail in `catalog.test.ts`, where somebody typed the number: it
would fail in `form.test.ts`, in one frame of one mood, months later.

## The eye is a ball, a pupil and two lids

This is the cut cast's eye. The drawn cast has its own, which is the stroke this
one replaced; *Two casts* has it.

An eye used to be one stroke with four numbers on it, and it was being asked to
carry everything. With no pupil a look was the whole eye sliding across the face,
so every glance moved the face and every stare spent outline; with no lid,
looking down at a peer had to be faked by molding the stroke toward a line, and
it read as a squint. At 24px, working, frustrated, paused and stuck were four
dashes told apart by the tilt of a line two pixels long.

So an eye is what an animator draws. **The pupil takes the glance and the lids
take the emotion.** Eight numbers in `Eye`, all lerped, none switched:

- `open` is the upper lid, from clear of the ball to shut onto the lower lid.
  Negative retracts it, which is shock.
- `tilt` drops the inner end of both lids (cross) or lifts it (worry); `arch`
  bows the upper lid.
- `low` raises the lower lid and `smile` bows it up in the middle: the cheeks
  pushing, which is a smile with no mouth in it.
- `pupil` is its radius. Pinned is shock, open is interest.
- `size` scales the ball, and `skew` lowers the lid on the viewer's right.

**What is drawn is the opening, not the lids.** The white between the two lid
lines, cut to the ball, with the pupil and the lid's shadow clipped to it and a
line along the upper lid. Lids painted in the body's color over a white ball left
a hairline of white round every shut eye wherever the two edges antialiased; the
body is already there to be the lid. The upper lid closes onto the lower one,
wherever that is, so a shut eye is one line and never a line with white under it.

**The lid line is hinted.** At 24px a line drawn at its own weight is a
hairline, and a shut eye is then nothing at all. `CutArt` draws it up to 2.2
times heavier as the creature shrinks, from the size it was asked for rather
than the size it measured, and `styles.test.ts` holds `AVATAR_PX` to the sizes
in `styles.css` so the two cannot drift.

**A mirror can be calm, cross or afraid, but never doubtful.** Every expression
above is the same lids on both sides, and one whole family of faces is the two
sides disagreeing. `skew` is the disagreement: one lid lower than the other.
`thinking` wears it while it looks up and away, which is what turned a mild pair
into a creature weighing something up, and `blocked` puts it on as it squints up
at its badge.

**The far eye is smaller.** A hard look to one side turns the head, and on a
turned head the eye that went round the curve is foreshortened: `PEEK` grows the
near eye and shrinks the far one by the same share of the look.

**The eyes cross the face by a fraction of a glance and all of a stare.** The
pupil travels inside the white, so the eyes themselves move a little under half
the look across the face (`TRAVEL`), and the snout the body pulls out for a
stare carries them further (`PULL.lead`). That is the bargain `grip` makes for
the body, made for the face: `form.test.ts` measures both per unit of look.

**The upper lid rides the pupil down.** A look down lowers the lid after the
pupil, and a look up lifts it a little and brings the lower lid up after it.
That, and not a second molding, is what makes a look at somebody below read as
one, and it is true of every look, so the aimed one needs nothing of its own.

**Eyes flick, they never slide.** A gaze picks a target, crosses to it over a
number of seconds that has nothing to do with how often it happens, and holds.
Sliding an eye around on a sine is the single thing that makes a face read as a
screensaver; holding still between jumps is what makes it read as attention.
`hz` and `cross` are separate for that reason: a creature that looks around
rarely does not also move its eyes slowly, and tying the two together made every
mood feel hurried.

**And they do not flick on the beat.** Each slot's jump lands somewhere in the
first half of it rather than at its start, so no two holds are the same length.
A hold that never varies is a metronome, which is the thing a slide is, arrived
at from the other side.

**Half of blinking is not on a timer.** A face blinks into a large saccade, lids
leading by a few frames, and blinks as it turns to look at somebody and as it
turns away. `saccadeBlink` is the first, two large jumps in three and never a
small one; `cueBlink` is the second, and `AgentAvatar` hands it the age of the
aimed look. The blink on a timer is still there, a little rarer, because a face
that only blinks for a reason is a face that stares.

**Most looks are glances and some are looks.** `gaze.far` is the share of
saccades that go the whole way to one side and level; the rest stay inside the
middle of the range. Without it every target is anywhere in the box and a
creature never quite commits to looking at anything. `idle` sets it to 0.3, and
the far look is the one thing that moves an idle body at all, which is the next
section.

**A gaze can be written down.** Random saccades are right for idle and thinking
and wrong for a mood that is looking at one particular thing. `gaze.script` is
`[x, y, hold]` steps, cycled through the same crossing and the same easing.
`blocked` uses one: it looks up at its own badge, narrows an eye at it, and comes
back to you. `working` uses another, and it is reading: short steps to the right
along a line, one long return, the next line a little lower.

**A look can change the face.** `watch.squint` blends into the lids by how far
up the gaze has gone, so blocked narrows and cocks a lid as it looks at the badge
and opens again when it looks back. A mood that acts needs no second drawing to
switch to.

## The gaze moves the body

This is the part worth protecting. One gaze vector, smoothed once through
`settle`, is read by the eyes and the body at the same instant. As the eyes go
the body is pulled into a pear pointed after them: the front is drawn out and
narrowed with the eyes leading it, the back is left round, the top cranes over
on a planted base, and the creature stands up a shade.

**Together, not after.** The body used to follow the eyes on a spring almost
half a second behind, on the argument that a bulge read late reads as a
consequence of the look. That was true of a bulge and false of a pear: the
eyes went, and then something else happened to the body, which read as two
animations rather than one creature. So the smoother sits in front of both.
It is short, so a flick is still a flick, and it is there at all because an
aimed look arrives as a step and a step through nothing is a cut, for the eyes
as much as for the body. It is critically damped, because the eyes read it and
an eye that overshoots is a wobble.

**A long look is slower than a glance.** The crossing time a mood sets is for
an ordinary glance; a jump the whole way to one side takes up to twice it,
scaled by how far it goes. The body comes with the eyes now, and a body that
becomes a pear in a quarter of a second is a body that snapped.

**The body answers a stare, not a glance.** The look the mass follows is `grip`
of the look the eyes took: nothing under `PULL.quiet`, all of it past
`PULL.wide`, a smooth ramp between. The linear version moved a body as much for
a glance as for a stare, scaled, and an idle creature glancing about was a
creature that would not sit still. Now an idle body is still until an eye goes
to the edge, and then it goes further than anything did before.

**A look pulls a pear, not a bulge.** The first version added a bump to the
radius on the side the eyes went to, and a bump is a lump: a bigger one read as
a growth rather than as the eyes taking the body with them. So the pull is done
on the point rather than on the radius, in the frame of the look: the front is
drawn out by `stretch` and narrowed by `taper`, both rising from nothing at the
back of the body to everything at the tip, so the back keeps its shape and the
front becomes a snout. Across, the eyes travel up to `lead` further as the body
answers, so they sit at the narrow end rather than in the middle. Not up and
down: up has a third of a unit to spare and down already has the lid. On top
of it a shear about a pivot under the center leans the top and not the base,
and a crane on height stands the creature up, which is the free direction since
every one of these bodies has room over its head and none at its sides.

**An idle body is still.** It had a breath and a wobble, and beside a face that
blinks and looks about, a body that also pulsed read as a second animation
running rather than as a creature at rest. The only body that breathes at rest
now is `paused`, which is asleep. Everything else that moves a body is either a
look or work.

**The outline is bounded whatever it is handed.** A message landing is added
to the look on top of whatever the eyes were doing, so the gaze the body is
handed can be further than any gaze a mood produces, and the bound on the
outline cannot be a bound on the gaze. It is two things: `PULL.hold` caps the
look, and the stretch is then cut to the room the outline actually has left,
measured on the outline itself every frame. Before that cut a puddle (`stuck`,
`paused`) or a tall face (`surprised`) aimed downward at a peer drew past
`FORM.reach` by up to two units, because a quadratic sag grows faster than the
swell feeding it, and nothing sampled the combination. `form.test.ts` now
drives every creature in every mood a good deal past the cap in sixteen
directions and expects the outline to stop where it says.

A throw and a catch go through the same channel: a decaying displacement added
to the body's gaze, so a message landing deforms the creature rather than
translating it. It is added *away* from whoever the creature is looking at, so
a parcel thrown from above presses its recipient down and a throw recoils
against itself, which means the direction of a hit is read off the look and not
off the gesture. That is why the look outlasts the landing rather than being
released by it: `roleOf` used to drop it the moment the parcel arrived, which is
the one frame anybody is certainly watching, and a message thrown downward
knocked its recipient upward.

## An aimed look

A creature aimed at a peer is the only gaze that does not come out of `gazeAt`,
and it is the furthest any of them goes. `AIM` is spent as a gaze and nothing
else: the mass leans and swells after it, the pupil goes to the edge of the
white, and the upper lid comes down after the pupil. The stroke this replaced
needed `aimedEye`, a second molding on top of the look, because two marks
sliding down a face do not read as looking anywhere. A pupil going down under a
lid that comes with it does.

**The two directions are not the same size, and the numbers are measured.**
Every one of these bodies hangs its mass below its eyes, so there is depth under
them and very little over them, and the widest eyes on the table, `surprised`,
have to fit while looking up at whoever just threw something at them.
`form.test.ts` measures the ink against the outline itself rather than against a
radius at an angle, because these bodies are not star-shaped and a cloud's outer
corner sits over a dip between two lobes, where a radial bound is wrong in both
directions at once. It is the gate: move either number, or any eye in the
catalog, and it says which creature loses its eyes. The square is the one it
binds on, looking up and to the right at its badge.

## Moods

Ten expressions, in one table. Adding one is a row and nothing else: no
component learns about it, no stylesheet gains a rule.

| Mood | What it is | What the app reads it from |
|---|---|---|
| idle | Still, blinking, looking about, now and then all the way to one side | Active with nothing in flight |
| listening | Lids back and pupils open, held on you | A message queued |
| thinking | One lid lower than the other, looking up and away | A turn between rounds |
| working | Level lids half down, reading, kneading on a beat | A tool call in flight |
| frustrated | Inner ends of the lids down hard, trembling, glaring off to a side | The last call back was refused or failed |
| blocked | Looks up at its badge, narrows an eye at it, then back at you | A turn parked on a person |
| pleased | Lower lids pushed up into a crescent, quietly satisfied | Its reply landed in the last few seconds |
| paused | Shut, slow, sitting down, grey | Lifecycle paused, or composted |
| stuck | Low, worried, inner ends up, eyes darting where the body cannot go | An escalation of its own is open |
| surprised | Lids retracted, pupils pinned | It has just been handed a message |

`moodFor` is the only place a runtime signal becomes an expression, and
`moods.test.ts` proves every mood in the table is reachable from a real signal.
Ten drawings is ten things to keep working, and one no signal can reach is one
nobody would notice going wrong.

Amber is spent on exactly one mood, and the test says so. `blocked` is the state
where a turn is parked on a person; spend the color anywhere else and the rail
stops meaning anything.

**Two moods are transient and neither needs a timer.** `pleased` and `surprised`
expire on a stamp, and `moodFor` takes the clock as an argument, so the decision
is made inside the render loop. A rail of a dozen agents reacting to each other
costs React nothing at all.

## Two casts

The same creatures, drawn two ways, and the operator picks which in Settings,
Appearance. `cut` is everything above: paper relief and an eye with lids.
`drawn` is a brush. Both were prototyped side by side with a third, a simulated
gel, and the gel had the best motion and the wrong material for a column of text;
these two are the ones worth having, and they are different enough that one does
not stand in for the other.

**It is the operator's, not the agent's.** An agent stores which character it
is and nothing about how it is drawn, so the choice lives in `lib/prefs.ts` with
the surface and the scale, is read by every avatar through the store, and
changing it redraws every creature on screen at once. `AgentAvatar` takes a
`cast` for a preview, which is how each choice in Settings shows its own cast
before it is picked.

**Everything that is a decision is shared.** The body is `bodyPoints` for both,
the moods and `moodFor` are one table, the gaze and its smoothing are one, and
the knock is one. `AgentAvatar` decides all of it once a frame and hands the
result to the cast as a `Frame`; the cast owns only its own elements. So a mood
added to the table is a mood both casts must draw, and the type says so: `DRAWN`
is a `Record<Mood, Face>`.

**The drawn outline is one brush stroke.** It is the same outline, drawn as a
filled shape that presses heavier underneath, starts at the top left and runs a
little past where it began, which is what a hand drawing a closed shape does.
The pigment under it is printed a unit out of register. The whole body is
clipped to `FORM.reach` the way the paper relief is, because a brush has width
on both sides of the line the reach was measured on.

**It is drawn on twos.** Twelve drawings a second, held between: a look is a
snap, a hold is dead still, and a large look is hidden behind a blink the way an
animator cuts a head turn. A change of mood is five drawings, one of them past
the new pose (`POSES`), which is the overshoot a smoothstep can never have. The
one thing simulated between drawings is the mark on each head, a curl, a leaf, a
tick or nothing, which lags the body on a spring: the only secondary motion in
either cast, and a second thing that tells two creatures of one silhouette apart
at 24px.

**The line is an emotional channel.** Calm moods hold a clean line and it does
not move between drawings, which `drawn.test.ts` holds, because a rail of idle
creatures whose outlines crawl is the screensaver this whole design exists to
avoid. Work boils it on every drawing, frustration scratches it, stuck wavers
slowly, and a paused creature's line runs dry.

**The drawn face has brows and a mouth, and neither can fall out of register.**
Both were refused for the cut cast: a brow is a second object that has to stay
over the eye under it, and a mouth at 22px is a smudge. Here the eye is a dot or
a dash, the brow is a stroke computed from it, and the mouth is not drawn at all
under `MOUTH_PX`. Every stroke of the face is also kept inside the outline point
by point, measured on the outline itself: a brow over a cloud's notch bends down
into it rather than floating in the air above it, and a mouth on a squat body
rides up off the floor. `drawn.test.ts` holds every face inside every body, in
every mood, wherever the eyes have gone.

**Beside the head, comics' own marks.** A scribble is cross, a drop of sweat is
worry, lines round the head are shock and a star is pleased, drawn on twos in
the page's text color like the cut cast's marks. `bang` is the same amber badge
in both casts and is still only `blocked`.

## What one loop buys

`clock.ts` holds one `requestAnimationFrame` for every creature on screen. A
rail of a dozen agents is otherwise a dozen loops and a dozen chances for one to
be left running after its row is gone. What is off screen is not computed:
a transcript with sixty faces in it would otherwise be sixty outlines a frame
for the sake of the eight you can see.

**Correctness does not depend on the loop.** Every avatar paints itself once on
mount and again whenever its props change, so an operator who asked for reduced
motion, a hidden window and a row scrolled out of view all draw the right thing.
The loop only makes it move.

**The drawn cast writes a fifth as often.** It is called every frame like the
cut one and returns at once unless a new drawing is due, so a rail of sixty
drawn faces is twelve writes a second each rather than sixty.

The next frame is scheduled before the painting, so one painter throwing cannot
stop every other face in the app for the rest of the session.

## No two of them keep time together

One clock for every creature is also the thing that makes a rail read as one
animal. A mood is a table of rates every agent in it shares, so eight idle
agents handed the same seconds breathe at 0.22Hz together, blink on the same
4.6-second slots and glance at the same moment. Nothing about that is wrong per
frame, and all of it is wrong per rail: a row of creatures on one beat is
choreography, and choreography is the one thing a creature must not look like.

**A phase offset does not fix it, because a phase offset never changes.** An
offset moves where a creature is in its cycle and leaves its tempo alone, so two
of them hold whatever gap they started with for the life of the session. That
gap is what an operator actually sees: eight blobs pulsing at one rate in fixed
formation reads as one animation played eight times, which is what it is.

So a seed buys two numbers. `gaitOf` in `clock.ts` turns an agent id into a
phase and a tempo, the tempo is the one that does the work, and the spread is
±22%: a pair drifts half a breath apart inside about fifteen seconds of idling
and keeps drifting, so the formation never re-forms. Wider and the same mood
starts reading as two different amounts of urgency, which `moods.ts` is supposed
to be the only source of.

**Only cycles are on that clock.** A mood becoming another, a message landing, a
turn finishing: every age is measured on the shared seconds, or how fast a face
reacts to being spoken to would be a property of its id. `AgentAvatar` keeps the
two apart on purpose, and the transient moods are decided against `Date.now()`
regardless.

The marks beside a head loop in CSS rather than in the frame loop, and CSS has
only the document's clock, which is the same for everybody. Every avatar on
screen is written in one frame and a crew told one thing starts thinking in one
frame, so the marks come out in lockstep unless something says otherwise: the
phase is handed down as `--gait` and each loop is pulled back by it.

## Identity

The silhouette carries the first half and the eyes carry the rest: how far apart
they are set, how big they are, how high they sit, and whether there is one of
them. `r` is the whole ball, before any lid covers it; the drawn cast's dot is
two thirds of it. `catalog.test.ts` holds every character to a distinguishable pair, because
four characters share a shape and the pair is all that is left to tell them
apart. It also holds every one of the five to being used by somebody: a shape
nobody is cut from is a shape that could break with nothing on screen to say so.

Where the eyes sit is a per-shape decision rather than a global one. A drop is
narrower at the sides than a circle and its mass is low, so its characters look
out of the ball rather than out of the middle of the box, and the seating test
measures against the character's own silhouette rather than against a circle.

The cast cannot be smaller than the cafeteria's preset list. A crew is whatever
subset the operator ticked, so two presets sharing a character are two agents
that look the same in one rail. `lib/cafeteria.test.ts` is where that is
enforced.

**Agents store the key and nothing else.** No drawing is persisted, so any of
this can be redrawn without touching the database. `ALIASES` maps every key that
has ever shipped onto a current character by hand, and `catalog.test.ts` holds
the table to the list: the fallback hash would answer for all of them, and would
re-roll every existing agent's face on the day the cast changed size, which is
the one thing the table is for.
