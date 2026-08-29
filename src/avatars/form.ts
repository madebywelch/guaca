/**
 * The body of an agent, as a function rather than a drawing.
 *
 * A creature is a closed curve through `FORM.samples` radii around one center.
 * Every expression is a term in that radius, or a scale applied to the point
 * after it. No transform is ever put on the drawing itself: a character that
 * slides around inside its own box reads as a sprite being moved, and one whose
 * outline changes reads as a thing that is alive. That distinction is the whole
 * design, and it is why there is no path data anywhere in this directory.
 *
 * The amplitudes here are deliberately small. The body breathes, leans and
 * settles; it does not act. What acts is `eyes.ts`, because a body that emotes
 * as hard as a face is a body nobody can read a face on.
 */

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
  /** Radii in the outline. Enough that Catmull-Rom reads as a curve. */
  samples: 28,
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
  /** Standing stretch. Kept near 1: the species is round. */
  ax: number;
  ay: number;
  /** The resting silhouette. */
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
 * How hard the mass follows a look.
 *
 * `swell` is the bulge on the side the eyes went to and `flatten` is what the
 * side they left gives up. Both are multiplied by how far the gaze has gone, so
 * a creature looking straight ahead is exactly its resting shape.
 */
const PULL = { swell: 0.8, flatten: 0.32, lean: 0.1, width: 0.7 };

/** One frame of one creature, as points. `t` is seconds, `gaze` in body radii. */
export function bodyPoints(lump: Lump, shape: Shape, t: number, gaze?: Point) {
  const gx = gaze ? gaze[0] : 0;
  const gy = gaze ? gaze[1] : 0;
  const reach = Math.hypot(gx, gy);
  const R = FORM.radius;

  /* The mass leans after the eyes. Not far: what sells it is that the lean and
     the swell are the same displacement, so the body looks pulled rather than
     moved. */
  const cx = FORM.center + gx * R * PULL.lean;
  const cy = FORM.center + (shape.rise ?? 0) + gy * R * PULL.lean;

  const knead = shape.knead ? pulse(t, shape.knead.hz, shape.knead.sharp) * shape.knead.amp : 0;
  /* Volume preserving: wide when short and the other way round, which is the
     whole of why clay reads as clay rather than as a balloon. */
  const ax = lump.ax * (shape.aspect ? shape.aspect[0] : 1) * (1 + knead * 0.9);
  const ay = lump.ay * (shape.aspect ? shape.aspect[1] : 1) * (1 - knead);

  const heave = shape.heave
    ? Math.max(0, Math.sin(t * shape.heave.hz * TAU)) ** 3 * shape.heave.amp
    : 0;

  const towards = reach > 0.004 ? Math.atan2(gy, gx) / TAU : 0;
  const pts: Point[] = [];
  for (let i = 0; i < FORM.samples; i++) {
    const a = (i / FORM.samples) * TAU;
    let rr = 1;
    for (const lobe of lump.sig) rr += lobe.amp * Math.sin(lobe.k * a + lobe.phase);
    if (shape.wob) for (const w of shape.wob) rr += w.amp * Math.sin(w.k * a + w.spd * t);
    if (reach > 0.004) {
      rr += press(a, towards, PULL.width, reach * PULL.swell);
      rr += press(a, towards + 0.5, PULL.width + 0.1, -reach * PULL.flatten);
    }
    if (shape.press) {
      for (const p of shape.press) {
        const amp = p.beat ? p.amp * (0.55 + 0.45 * Math.sin(t * p.beat)) : p.amp;
        rr += press(a, p.th + (p.spin ? p.spin * t : 0), p.w, amp);
      }
    }
    rr += heave * Math.max(0, -Math.sin(a));

    let x = cx + Math.cos(a) * R * rr * ax;
    let y = cy + Math.sin(a) * R * rr * (ay + heave * 0.5);

    /* Gravity, applied after the outline. The underside is the part that knows
       about the table, and a puddle is not an ellipse. */
    if (shape.sag) {
      const below = Math.max(0, (y - cy) / R);
      y += shape.sag * below * below;
      x = cx + (x - cx) * (1 + (shape.spread ?? 0) * below);
    }
    pts.push([x, y]);
  }
  return { pts, knead };
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
