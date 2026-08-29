# Characters

An agent is a round creature made of clay. There is one species, twenty-one of
it, and everything it has to say it says by changing shape.

This is the fourth cast. The three before it were emoji, then a hand-drawn set
of creatures, then an egg with props, then sixteen vegetables, and every one of
them was the same idea: somebody draws a picture per agent and the app picks
between the pictures. The vegetables were the best of them and they still failed
the same way. They were charming at six agents and childish at sixty, every
character was a bezier somebody had to draw to a written spec, and a test had to
check the drawing afterwards because the spec could not enforce itself.

Nothing is drawn now. A character is a row of numbers, an expression is another
row of numbers, and the geometry is a function of the two. A character cannot
leave its box, cannot sit at a different optical weight and cannot take light
from a different direction, because none of those is a thing a character
supplies.

## Where it lives

| File | What is in it |
|---|---|
| `src/avatars/form.ts` | The body. `FORM`, the types, and the maths that turns a character and a mood into points. |
| `src/avatars/eyes.ts` | The eye primitive, the blink, and the gaze. |
| `src/avatars/catalog.ts` | The cast, the accents, and the alias table that keeps every key an older build wrote still meaning something. |
| `src/avatars/moods.ts` | The ten expressions, and `moodFor`, which is the only place a runtime signal becomes a face. |
| `src/avatars/clock.ts` | One frame loop for every creature on screen. |
| `src/avatars/AgentAvatar.tsx` | What is true right now, written to three attributes a frame. |

## The body is a function, not a path

A creature is a closed curve through 28 radii around one center, smoothed into
cubics. Everything a mood does is a term in that radius, or a scale applied to
the point after it.

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

## Identity

One species means the outline carries less of an identity than it used to, so
the eyes carry the rest: how far apart they are set, how big they are, how high
they sit, and whether there is one of them. `catalog.test.ts` holds every
character to a distinguishable pair, and to being round — a lump that stretched
past a tenth would read as a second kind of creature standing in the same rail.

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
