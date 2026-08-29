/**
 * The five outlines a body is cut from.
 *
 * A silhouette is one function: the resting radius at an angle, as a fraction
 * of `FORM.radius`. Everything else about a creature is still a term added to
 * that radius, so a square breathes, leans, kneads and settles with exactly the
 * code a circle does, and none of `form.ts`, `eyes.ts` or `moods.ts` knows how
 * many shapes there are.
 *
 * Angles are the ones the outline is walked in: 0 is to the right and they
 * increase clockwise, because the viewBox's y points down. Up is -PI/2.
 *
 * **Every silhouette encloses the same area.** Sizing five shapes by hand is
 * how a cast ends up with one member that reads as the small one, so the raw
 * functions below are written at whatever scale is easiest to think in and
 * `SIZE` scales each to the circle's area at load. A sixth shape is a function
 * and nothing else.
 *
 * **And none of them reaches past `CREST`.** The moods spend nearly all of the
 * room between `FORM.radius` and `FORM.reach` on the swell that follows a look,
 * so a shape with a long point or a flat underside, which are both ways of
 * putting the same area further out, gives area back rather than taking that
 * room. `silhouette.test.ts` holds both halves: the areas against each other,
 * and every crest against the budget.
 */

/** Which of the five a creature is cut from. */
export type Silhouette = "circle" | "octagon" | "square" | "drop" | "cloud";

/**
 * The furthest any silhouette rests from its center, in `FORM.radius`.
 *
 * Not a taste decision. `FORM.reach` is half again `FORM.radius`, and the moods
 * spend almost all of that on the swell that follows a look: the worst frame in
 * the whole space landed two hundredths of a pixel under the limit, and it did
 * that back when every creature was a circle. What is left over is this, minus
 * the allowance `catalog.test.ts` leaves a character's own lump on top of it.
 */
export const CREST = 1.06;

const TAU = Math.PI * 2;

/** Distance from the center to a circle, along one direction. */
function ball(a: number, cx: number, cy: number, r: number): number {
  const dx = Math.cos(a);
  const dy = Math.sin(a);
  const along = cx * dx + cy * dy;
  const off = cx * dy - cy * dx;
  const under = r * r - off * off;
  return under <= 0 ? 0 : Math.max(0, along + Math.sqrt(under));
}

/** Signed angle from straight up, in (-PI, PI]. */
function fromUp(a: number): number {
  let w = a + Math.PI / 2;
  while (w > Math.PI) w -= TAU;
  while (w <= -Math.PI) w += TAU;
  return w;
}

/**
 * A teardrop: a ball with the cone that is tangent to it standing on top.
 *
 * `aspect` is how tall it is against how wide, and it is the only choice here.
 * Everything else follows from wanting the apex and the underside the same
 * distance from the center, which is what keeps the crest as low as a point
 * this sharp can be, and a sharper point costs crest whichever way it is drawn.
 */
function drop(aspect: number): (a: number) => number {
  const b = 1;
  const k = (aspect - 1) * b;
  const h = k + b;
  const sin = b / (h + k);
  const cone = Math.asin(sin);
  const side = Math.sqrt((h + k) * (h + k) - b * b);
  /* Where the cone stops and the ball takes over, as an angle from up. */
  const seam = Math.atan2(side * sin, h - side * Math.cos(cone));
  return (a) => {
    const w = fromUp(a);
    if (Math.abs(w) <= seam) return (h * sin) / Math.sin(Math.abs(w) + cone);
    return -k * Math.cos(w) + Math.sqrt(b * b - k * k * Math.sin(w) * Math.sin(w));
  };
}

/**
 * Puffs over a flat underside.
 *
 * A cloud is the one shape here that is not convex, and the notches between the
 * puffs are the whole of what says cloud rather than lump, so it is a union of
 * balls cut off at a line rather than a wobble added to an ellipse. The wobble
 * was tried: at an amplitude deep enough to notch, the middle puff came to a
 * spike.
 */
function cloud(puffs: [number, number, number][], floor: number): (a: number) => number {
  return (a) => {
    let r = 0;
    for (const [cx, cy, br] of puffs) r = Math.max(r, ball(a, cx, cy, br));
    const down = Math.sin(a);
    return down > 1e-6 ? Math.min(r, floor / down) : r;
  };
}

/**
 * The same outline, narrower and taller.
 *
 * Puffs wide enough apart to notch make a cloud wider than the crest budget
 * pays for, and squeezing the finished outline keeps the notches where they
 * were: moving the puffs together instead closes them.
 */
function squeeze(shape: (a: number) => number, kx: number, ky: number): (a: number) => number {
  return (a) => {
    const ux = Math.cos(a) / kx;
    const uy = Math.sin(a) / ky;
    return shape(Math.atan2(uy, ux)) / Math.hypot(ux, uy);
  };
}

/** Written at any scale. `SIZE` sizes them against each other. */
const RAW: Record<Silhouette, (a: number) => number> = {
  circle: () => 1,

  /* A stop sign: flat to the right, left, top and bottom, corners between. */
  octagon: (a) => {
    const seg = Math.PI / 4;
    return Math.cos(seg / 2) / Math.cos(a - Math.round(a / seg) * seg);
  },

  /* A superellipse rather than four lines: the corner radius is a number here
     rather than a consequence of how many samples the outline is walked in. */
  square: (a) => {
    const n = 6;
    return (Math.abs(Math.cos(a)) ** n + Math.abs(Math.sin(a)) ** n) ** (-1 / n);
  },

  drop: drop(1.24),

  /* The puffs already carry the shift that stands the shape in the middle of
     its own box, so the tallest puff and the underside sit the same distance
     out. `silhouette.test.ts` is what says so, and a shape drawn off center
     leans in every rail it appears in with nothing else to catch it. */
  cloud: squeeze(
    cloud(
      [
        [0, -0.17, 0.46],
        [-0.48, 0.13, 0.42],
        [0.48, 0.13, 0.42],
        [0, 0.21, 0.6],
      ],
      0.63,
    ),
    0.94,
    1.064,
  ),
};

/** Enough steps that a notch or a corner cannot fall between two of them. */
const STEPS = 2048;

/** Area enclosed by one profile, by the polar integral. */
function encloses(shape: (a: number) => number): number {
  let sum = 0;
  for (let i = 0; i < STEPS; i++) {
    const r = shape((i / STEPS) * TAU);
    sum += 0.5 * r * r * (TAU / STEPS);
  }
  return sum;
}

/** Furthest one profile rests from the center. */
function crestOf(shape: (a: number) => number): number {
  let most = 0;
  for (let i = 0; i < STEPS; i++) most = Math.max(most, shape((i / STEPS) * TAU));
  return most;
}

/**
 * The circle's area for everyone, and the crest budget over the top of it.
 *
 * A shape that would have to reach past `CREST` to carry the circle's area is
 * scaled back until it fits, which spends area rather than the room the moods
 * need. The drop and the cloud both pay: a point and a flat underside are both
 * ways of putting the same area further out.
 */
function sizeOf(shape: (a: number) => number): number {
  const even = Math.sqrt(Math.PI / encloses(shape));
  return Math.min(even, CREST / crestOf(shape));
}

const SIZE = Object.fromEntries(
  Object.entries(RAW).map(([key, shape]) => [key, sizeOf(shape)]),
) as Record<Silhouette, number>;

/** The resting radius of each shape at an angle, in `FORM.radius`. */
export const SILHOUETTES = Object.fromEntries(
  Object.entries(RAW).map(([key, shape]) => {
    const scale = SIZE[key as Silhouette];
    return [key, (a: number) => shape(a) * scale];
  }),
) as Record<Silhouette, (a: number) => number>;
