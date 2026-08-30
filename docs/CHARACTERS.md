# Characters

An agent is a creature made of clay, cut from one of five shapes, and everything
it has to say it says by changing shape.

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
| `src/avatars/eyes.ts` | The eye primitive, the blink, and the gaze. |
| `src/avatars/catalog.ts` | The cast, the accents, and the alias table that keeps every key an older build wrote still meaning something. |
| `src/avatars/moods.ts` | The ten expressions, the marks drawn beside a head, and `moodFor`, which is the only place a runtime signal becomes a face. |
| `src/avatars/clock.ts` | The clock every creature shares, and the one each of them keeps. |
| `src/avatars/AgentAvatar.tsx` | What is true right now, written to three attributes a frame. |
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

**No transform is ever put on the drawing.** A character that slides around
inside its own box reads as a sprite being moved; one whose outline changes
reads as a thing that is alive. This is the load-bearing decision and it is why
there is no `translate`, no `rotate` and no keyframe anywhere in the drawing
path. When a creature leans, the mass leans: one side thickens and the other
thins.

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

## The eye is one stroke

An eye is an arc with a round cap and four numbers on it, in eye radii:

- `w` half its length. A dot is this at zero.
- `h` its weight. A dot with `w` of 0 and `h` of 2 is a circle of radius 1.
- `c` how far it bows. Negative curves up, which is the only smile there is.
- `a` how far it tilts, mirrored between the two. Positive drops the inner ends.

A blink is the dot moulded into a line. Upset is the line tilted in. Nothing is
ever swapped for anything, which is what lets a face sit halfway between two
moods and lets a mood change be an interpolation rather than a cut.

There are no eyebrows and there is no mouth. Both were tried. An eyebrow is a
second object that has to stay in register with the eye under it, and it was
doing work the tilt of the eye itself does better. A mouth at 22px is a smudge.

**Eyes flick, they never slide.** A gaze picks a target, crosses to it over a
number of seconds that has nothing to do with how often it happens, and holds.
Sliding an eye around on a sine is the single thing that makes a face read as a
screensaver; holding still between jumps is what makes it read as attention.
`hz` and `cross` are separate for that reason: a creature that looks around
rarely does not also move its eyes slowly, and tying the two together made every
mood feel hurried.

**A gaze can be written down.** Random saccades are right for idle and thinking
and wrong for a mood that is looking at one particular thing. `gaze.script` is
`[x, y, hold]` steps, cycled through the same crossing and the same easing.
`blocked` uses one: it looks up at its own badge, presses at it, and comes back
to you.

**A look can change the face.** `watch.squint` blends into the eye shape by how
far up the gaze has gone, so blocked narrows and tilts as it looks at the badge
and opens again when it looks back. A mood that acts needs no second drawing to
switch to.

## The gaze moves the body

This is the part worth protecting. The same gaze vector is read twice: sharp for
the eyes, and followed by the body on a critically damped spring tuned to
`SETTLE`. The eyes go, and about half a second later the side they went to
swells out, the side they left flattens, and the whole creature leans that way.

Read at the same instant, the bulge looks like part of the drawing. Read late
and smoothed, it looks like a consequence of it. One number in two places is why
it reads as being pulled rather than as two things being animated at once.

A throw and a catch go through the same channel: a decaying displacement added
to the body's gaze, so a message landing deforms the creature rather than
translating it.

## Moods

Ten expressions, in one table. Adding one is a row and nothing else: no
component learns about it, no stylesheet gains a rule.

| Mood | What it is | What the app reads it from |
|---|---|---|
| idle | Round, breathing, looking about | Active with nothing in flight |
| listening | Eyes open, held on you | A message queued |
| thinking | Flicking away and back | A turn between rounds |
| working | Narrowed, scanning, kneading on a beat | A tool call in flight |
| frustrated | Tilted in, trembling | The last call back was refused or failed |
| blocked | Looks up at its badge, then back at you | A turn parked on a person |
| pleased | Eased off, quietly satisfied | Its reply landed in the last few seconds |
| paused | Shut, slow, sitting down, grey | Lifecycle paused, or composted |
| stuck | Small, low, staring at nothing | An escalation of its own is open |
| surprised | Everything open at once | It has just been handed a message |

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
them. `catalog.test.ts` holds every character to a distinguishable pair, because
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
