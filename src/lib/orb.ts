/**
 * Where a crew's faces sit inside its circle.
 *
 * A group is recognized by who is in it, and at the 38px the strip draws that
 * recognition is mostly shape: how many faces there are and how they stand.
 * Tiling every crew into the same square threw that away, so a strip of crews
 * read as one badge repeated.
 *
 * Here the arrangement is the count. One face is centered, two stand side by
 * side, and three to six stand on a ring, so the size of a crew is legible
 * before any single face in it is, and two crews of different sizes cannot draw
 * the same badge.
 *
 * Geometry only, in fractions of the ring's own box, so the ring can be resized
 * in the stylesheet without a number in here having to be changed to agree.
 */

/** A face's place in the ring. `x`, `y` and `size` are fractions of the ring. */
export interface Seat {
  /** Center of the face, 0 at the ring's left and top edge, 1 at its right and bottom. */
  x: number;
  y: number;
  /** Width of the face's box. */
  size: number;
  /** Degrees of lean, so a crew looks arranged by hand rather than tiled. */
  tilt: number;
}

/** How many faces a ring seats before it starts counting instead. */
export const SEATS = 6;

/**
 * The ring each crew size stands on, and how big its faces are.
 *
 * Sized against the ink rather than the box, and against two numbers rather
 * than one: a creature is round, so `FORM.reach` is what has to stay inside the
 * rim on both axes, and `FORM.radius` is what two of them are spaced on. Every
 * row here leaves about a pixel of daylight at 2.4rem, and `orb.test.ts` holds
 * them to it against the geometry itself rather than against the numbers here.
 *
 * `from` is where the first face stands, in degrees clockwise from the right,
 * so -90 is the top. Everything but the pair starts there: a shape with a face
 * above a base under it reads as standing rather than as spilled.
 */
const RINGS: Record<number, { radius: number; size: number; from: number }> = {
  1: { radius: 0, size: 0.7, from: 0 },
  2: { radius: 0.2, size: 0.55, from: 180 },
  3: { radius: 0.235, size: 0.48, from: -90 },
  4: { radius: 0.265, size: 0.42, from: -90 },
  5: { radius: 0.285, size: 0.38, from: -90 },
  6: { radius: 0.3, size: 0.35, from: -90 },
};

/** Furthest a face leans, in degrees. Enough to notice, not enough to topple. */
const LEAN = 6;

/**
 * Deterministic 0..1 from a string, so a crew's arrangement never moves.
 *
 * FNV-1a with a mixing step on the end, rather than the rolling multiply-add
 * used elsewhere for animation phase. In that one the last character of the
 * seed reaches the result unmixed, so two seeds differing only there come out a
 * thousandth apart and lean the same way. An agent id is a UUID and would
 * survive either, but a seed that is anything shorter would not, and the
 * failure is a crew drawn as one face repeated with nothing to say why.
 */
function hash(seed: string): number {
  let at = 2166136261;
  for (let i = 0; i < seed.length; i++) {
    at = Math.imul(at ^ seed.charCodeAt(i), 16777619);
  }
  at = Math.imul(at ^ (at >>> 15), 2246822507);
  return ((at ^ (at >>> 13)) >>> 0) / 2 ** 32;
}

/** Keeps a seat's arithmetic from reaching the stylesheet at full precision. */
function round(value: number): number {
  return Math.round(value * 1e4) / 1e4;
}

/**
 * Seats as many of these members as the ring holds, and counts the rest.
 *
 * Each seat carries the member it was cut for, so nothing downstream has to
 * line two arrays up by index. Only the id is read: the lean has to survive a
 * rename and a recolor, or a crew rearranges itself when somebody is edited.
 */
export function cluster<T extends { id: string }>(
  members: T[],
): { seats: (Seat & { of: T })[]; rest: number } {
  const seated = members.slice(0, SEATS);
  const ring = RINGS[seated.length];
  if (!ring) return { seats: [], rest: 0 };

  const step = 360 / seated.length;
  const seats = seated.map((of, i) => {
    const angle = ((ring.from + i * step) * Math.PI) / 180;
    return {
      of,
      x: round(0.5 + ring.radius * Math.cos(angle)),
      y: round(0.5 + ring.radius * Math.sin(angle)),
      size: ring.size,
      // A crew of one has nobody to be arranged with, and a lone face leaning
      // reads as a mistake rather than as an arrangement.
      tilt: seated.length === 1 ? 0 : Math.round((hash(of.id) - 0.5) * 2 * LEAN * 10) / 10,
    };
  });

  return { seats, rest: members.length - seated.length };
}
