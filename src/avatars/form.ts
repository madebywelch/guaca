/**
 * The body of an agent, as a function rather than a drawing.
 *
 * A creature is a closed curve through `FORM.samples` radii around one center.
 * The first term of that radius is its silhouette, which is one of the five in
 * `silhouette.ts`; every expression is another term added to it, or a scale
 * applied to the point after it. No transform is ever put on the drawing
 * itself: a character that slides around inside its own box reads as a sprite
 * being moved, and one whose outline changes reads as a thing that is alive.
 * That distinction is the whole design, and it is why there is no path data
 * anywhere in this directory.
 *
 * Nothing else in this file knows how many silhouettes there are. A cloud
 * kneads, leans, sags and settles through the code a circle does, because a
 * shape is the resting radius and a mood is what gets added to it.
 *
 * The amplitudes here are deliberately small. The body breathes, leans and
 * settles; it does not act. What acts is `eyes.ts`, because a body that emotes
 * as hard as a face is a body nobody can read a face on. The one time the body
 * goes further is after a hard look, and then it goes as a consequence of the
 * eyes rather than on its own account: `PULL` is the whole of that bargain.
 */

import { SILHOUETTES, type Silhouette } from "./silhouette";

/** How far a look carries, what it costs the shape, and where the box is. */
export const FORM = {
  /** The viewBox every character is drawn in. */
  box: 64,
  center: 32,
  /** Resting radius of the body. What two creatures side by side are spaced on. */
  radius: 20,
  /**
   * Nothing is ever drawn outside this radius of the center, at any character,
   * in any mood, at any point of any cycle. `form.test.ts` samples the whole
   * space and holds the geometry to it, and `orb.test.ts` seats a crew against
   * it, so a mood that grew could not quietly push a face through the rim of
   * its group's circle.
   */
  reach: 30,
  /**
   * Radii in the outline.
   *
   * Divisible by eight, which is the part that matters: a square's corners sit
   * at 45 degrees and an octagon's at 22.5, and a corner that falls between two
   * samples is a corner that gets chamfered off. At the 28 this started with,
   * an octagon drew as a lumpy circle and nothing failed.
   */
  samples: 32,
} as const;

const TAU = Math.PI * 2;

/** A fixed lobe on the resting shape. Two or three of these are an identity. */
export interface Lobe {
  /** Harmonic. 1 leans, 2 ovals, 3 and up are corners. */
  k: number;
  amp: number;
  phase: number;
}

/** A travelling lobe. The same thing with the phase moving. */
export interface Ripple extends Omit<Lobe, "phase"> {
  /** Radians per second. */
  spd: number;
}

/** A thumb pressed into the outline at one angle. */
export interface Press {
  /** Turns clockwise from the right: 0 right, 0.25 down, 0.5 left, 0.75 up. */
  th: number;
  /** Angular width, in radians. */
  w: number;
  amp: number;
  /** Turns per second, if the press should travel. */
  spin?: number;
  /** Radians per second, if the press should come and go. */
  beat?: number;
}

/** Who a creature is, before anything happens to it. */
export interface Lump {
  key: string;
  label: string;
  /** Which of the five it is cut from. */
  form: Silhouette;
  /**
   * Standing stretch. Kept near 1: it varies a silhouette, it does not replace
   * one, and a square stretched far enough to read as a brick is a sixth shape
   * nobody declared.
   */
  ax: number;
  ay: number;
  /** Lobes on top of the silhouette. What tells two clouds apart. */
  sig: Lobe[];
  eye: {
    /** Half the gap between the eyes, in viewBox units. */
    spread: number;
    /** Eye radius. A dot is a stroke this thick with no length. */
    r: number;
    /** Offsets from the middle of the body, in viewBox units. */
    x?: number;
    y?: number;
    /** One eye instead of two. Drawn half again as large. */
    one?: boolean;
  };
}

/** What a mood does to the body. `moods.ts` holds the table of them. */
export interface Shape {
  aspect?: [number, number];
  /** The squeeze: amplitude, rate, and whether it snaps or breathes. */
  knead?: { amp: number; hz: number; sharp?: boolean };
  wob?: Ripple[];
  press?: Press[];
  /** How much the underside gives, and how far it puddles. */
  sag?: number;
  spread?: number;
  /** Where the center sits. Down is positive. */
  rise?: number;
  /** A slow effort that fails. The one thing a still image cannot say. */
  heave?: { amp: number; hz: number };
}

export type Point = [number, number];

/** A shaped pulse: squeeze fast, let go slowly. Sine is too polite for work. */
export function pulse(t: number, hz: number, sharp?: boolean): number {
  const u = (t * hz) % 1;
  if (!sharp) return Math.sin(u * TAU);
  return u < 0.3 ? Math.sin((u / 0.3) * Math.PI) : -0.34 * Math.sin(((u - 0.3) / 0.7) * Math.PI);
}

/**
 * How hard the mass follows a look, and how that grows with the look.
 *
 * A look pulls the body into a pear pointed at it: the front is drawn out by
 * `stretch`, narrowed by `taper` as it goes, and the back is left round, so a
 * circle looking hard to the right is a snout with the eyes in it and the mass
 * behind. `lean` carries the center, `shear` carries the top further than the
 * base, and `crane` stands the creature up a little. None of it is applied to
 * the gaze as the eyes took it. It is applied to `grip` of it, which is nothing
 * under `quiet`, everything past `wide`, and a smooth ramp between, so a glance
 * is the eyes alone and a look to the edge of the range takes the body with it.
 * The linear swell this replaced moved a body as much for a glance as for a
 * stare, scaled, and an idle creature glancing about was a creature that would
 * not sit still.
 *
 * `hold` is the most look the mass will answer, in body radii. A message
 * landing is added to the look on top of whatever the eyes were doing, so the
 * gaze the body is handed can be further than any gaze a mood produces; this
 * is the bound, and `form.test.ts` drives the body past it to prove the
 * outline stops here.
 */
export const PULL = {
  quiet: 0.06,
  wide: 0.26,
  hold: 0.38,
  /** How far the front is drawn out, in body radii per body radius of look. */
  stretch: 1.2,
  /** How much the front narrows as it goes, per body radius of look. */
  taper: 1.1,
  /** Where along the body the pull begins, in body radii behind the center. */
  back: 0.15,
  /**
   * How much further the eyes go, across, when the body answers: they lead the
   * snout rather than sit behind it. Across only, because up is the starved
   * direction (`eyes.ts` has the measurement) and down already has the lid.
   */
  lead: 0.6,
  lean: 0.1,
  shear: 0.2,
  /** Where the shear turns, below the center, in body radii. The base stays put. */
  pivot: 0.45,
  crane: 0.1,
};

/** Quintic step over 0..1. Leaves and arrives with no corner on it. */
function smooth(u: number): number {
  const e = Math.max(0, Math.min(1, u));
  return e * e * e * (e * (e * 6 - 15) + 10);
}

/** How much of a look, at this distance, the mass answers. */
export function grip(reach: number): number {
  const held = Math.min(reach, PULL.hold);
  return held * smooth((held - PULL.quiet) / (PULL.wide - PULL.quiet));
}

/** One frame of one creature, as points. `t` is seconds, `gaze` in body radii. */
export function bodyPoints(lump: Lump, shape: Shape, t: number, gaze?: Point) {
  const seen = Math.hypot(gaze ? gaze[0] : 0, gaze ? gaze[1] : 0);
  /* The look the body answers is the eyes' look through `grip`, in the same
     direction and shorter. Everything below reads this pair and never the raw
     gaze, so a glance costs the outline nothing. */
  const reach = seen > 0.004 ? grip(seen) : 0;
  const gx = gaze && seen > 0.004 ? (gaze[0] / seen) * reach : 0;
  const gy = gaze && seen > 0.004 ? (gaze[1] / seen) * reach : 0;
  const R = FORM.radius;

  /* The mass leans after the eyes. Not far: what sells it is that the lean and
     the swell are the same displacement, so the body looks pulled rather than
     moved. */
  const cx = FORM.center + gx * R * PULL.lean;
  const cy = FORM.center + (shape.rise ?? 0) + gy * R * PULL.lean;

  const knead = shape.knead ? pulse(t, shape.knead.hz, shape.knead.sharp) * shape.knead.amp : 0;
  /* Volume preserving: wide when short and the other way round, which is the
     whole of why clay reads as clay rather than as a balloon. A hard look also
     cranes: taller and a shade thinner, which is the body reaching after what
     the eyes found, and it is spent on height because height is the cheap
     direction, since every one of these bodies has room over its head and
     none to spare at its sides. */
  const crane = reach * PULL.crane;
  const ax = lump.ax * (shape.aspect ? shape.aspect[0] : 1) * (1 + knead * 0.9) * (1 - crane * 0.5);
  const ay = lump.ay * (shape.aspect ? shape.aspect[1] : 1) * (1 - knead) * (1 + crane);

  const heave = shape.heave
    ? Math.max(0, Math.sin(t * shape.heave.hz * TAU)) ** 3 * shape.heave.amp
    : 0;

  const ux = reach > 0.004 ? gx / reach : 1;
  const uy = reach > 0.004 ? gy / reach : 0;
  const silhouette = SILHOUETTES[lump.form];
  const walk = (swell: number): Point[] => {
    const pts: Point[] = [];
    for (let i = 0; i < FORM.samples; i++) {
      const a = (i / FORM.samples) * TAU;
      let rr = silhouette(a);
      for (const lobe of lump.sig) rr += lobe.amp * Math.sin(lobe.k * a + lobe.phase);
      if (shape.wob) for (const w of shape.wob) rr += w.amp * Math.sin(w.k * a + w.spd * t);
      if (shape.press) {
        for (const p of shape.press) {
          const amp = p.beat ? p.amp * (0.55 + 0.45 * Math.sin(t * p.beat)) : p.amp;
          rr += press(a, p.th + (p.spin ? p.spin * t : 0), p.w, amp);
        }
      }
      rr += heave * Math.max(0, -Math.sin(a));

      let x = cx + Math.cos(a) * R * rr * ax;
      let y = cy + Math.sin(a) * R * rr * (ay + heave * 0.5);

      /* The pear. In the frame of the look, `u` is along it and `v` across it,
         and `s` rises from nothing at the back of the body to one at the front,
         so the back keeps its shape while the front is drawn out and narrowed.
         Done on the point rather than the radius because a radius can only
         bulge: the narrowing is what says the eyes are pulling the body and not
         that the body has grown a lump. */
      if (reach > 0.004) {
        const dx = (x - cx) / R;
        const dy = (y - cy) / R;
        const u = dx * ux + dy * uy;
        const v = -dx * uy + dy * ux;
        const s = smooth((u + PULL.back) / (1 + PULL.back));
        const u2 = u + swell * s * s;
        const v2 = v * (1 - reach * PULL.taper * s * s);
        x = cx + (u2 * ux - v2 * uy) * R;
        y = cy + (u2 * uy + v2 * ux) * R;
      }

      /* The top goes further than the base. A shear about a point under the
         center, so a creature looking hard to one side cranes over toward it on
         a planted underside rather than sliding across its box, which is the
         difference between a body leaning and a sprite being moved. It is only
         ever sideways: up and down, the lean and the crane already say it. */
      x += (cy + PULL.pivot * R - y) * gx * PULL.shear;

      /* Gravity, applied after the outline. The underside is the part that knows
         about the table, and a puddle is not an ellipse. */
      if (shape.sag) {
        const below = Math.max(0, (y - cy) / R);
        y += shape.sag * below * below;
        x = cx + (x - cx) * (1 + (shape.spread ?? 0) * below);
      }
      pts.push([x, y]);
    }
    return pts;
  };

  /* The stretch spends the room that is left and no more. What is left depends
     on everything above: a creature that has sat down has spent the room under
     it on sitting, a tall one on being tall, and a puddle grows faster than
     the stretch that feeds it because the sag is quadratic. So the bound is
     taken on the outline rather than on the numbers that made it: if the full
     stretch puts a point past `FORM.reach`, it is cut back along the secant to
     the stretch that does not, and once more if the puddle bent the line. A
     tuned swell held this by two hundredths of a pixel and only in the frames
     somebody thought to sample. */
  let swell = reach * PULL.stretch;
  let pts = walk(swell);
  if (swell > 0) {
    let rest = Number.NaN;
    for (let pass = 0; pass < 3; pass++) {
      const far = furthest(pts);
      if (far <= FORM.reach) break;
      if (Number.isNaN(rest)) rest = furthest(walk(0));
      swell = rest >= FORM.reach ? 0 : swell * ((FORM.reach - rest) / (far - rest)) * 0.98;
      pts = walk(swell);
    }
  }
  return { pts, knead };
}

/** How far the furthest point of an outline is from the center of the box. */
function furthest(pts: Point[]): number {
  let most = 0;
  for (const [x, y] of pts) most = Math.max(most, Math.hypot(x - FORM.center, y - FORM.center));
  return most;
}

/** A gaussian thumbprint at one angle. */
function press(a: number, th: number, w: number, amp: number): number {
  let d = a - th * TAU;
  while (d > Math.PI) d -= TAU;
  while (d < -Math.PI) d += TAU;
  return amp * Math.exp(-(d * d) / (w * w));
}

/** Two shapes are the same radii, so one becomes the other by lerping. */
export function blend(a: Point[], b: Point[], u: number): Point[] {
  const out: Point[] = [];
  for (let i = 0; i < a.length; i++) {
    const p = a[i] as Point;
    const q = b[i] as Point;
    out.push([p[0] + (q[0] - p[0]) * u, p[1] + (q[1] - p[1]) * u]);
  }
  return out;
}

/** Catmull-Rom through a closed loop of points, as cubics. */
export function outline(pts: Point[]): string {
  const first = pts[0] as Point;
  const d = [`M${first[0].toFixed(2)} ${first[1].toFixed(2)}`];
  for (let i = 0; i < pts.length; i++) {
    const p0 = pts[(i - 1 + pts.length) % pts.length] as Point;
    const p1 = pts[i] as Point;
    const p2 = pts[(i + 1) % pts.length] as Point;
    const p3 = pts[(i + 2) % pts.length] as Point;
    const c1x = p1[0] + (p2[0] - p0[0]) / 6;
    const c1y = p1[1] + (p2[1] - p0[1]) / 6;
    const c2x = p2[0] - (p3[0] - p1[0]) / 6;
    const c2y = p2[1] - (p3[1] - p1[1]) / 6;
    d.push(
      `C${c1x.toFixed(2)} ${c1y.toFixed(2)} ${c2x.toFixed(2)} ${c2y.toFixed(2)} ${p2[0].toFixed(2)} ${p2[1].toFixed(2)}`,
    );
  }
  return d.join("");
}
