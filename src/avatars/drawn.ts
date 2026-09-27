/**
 * The drawn cast: the same creatures, drawn rather than cut.
 *
 * The body is `bodyPoints`, exactly as the cut cast has it, and the gaze and
 * the moods are the ones every creature shares. What is different is all in
 * how it is put on screen. The outline is one brush stroke that presses at the
 * bottom and lifts off where it began, over a pigment printed a unit out of
 * register. It is drawn on twos: twelve drawings a second, held between, so a
 * look is a snap and a hold is dead still. And the line is an emotional channel
 * of its own: calm moods hold it clean, work boils it, frustration scratches
 * it, and a paused creature's line runs dry.
 *
 * The face has two things the cut cast does not, brows and a mouth. The cut
 * cast refused both, a brow because it has to stay in register with the eye
 * under it and a mouth because it is a smudge at 22px. Here both are strokes
 * computed from the eye they belong to, so register is not something anyone
 * can get wrong, and the mouth is not drawn at all under `MOUTH_PX`.
 *
 * Nothing here touches the DOM. It is geometry in, path data out, so the suite
 * beside it can hold every drawing to the same reach the cut cast is held to.
 */

import { blinkAmount, type Gaze, glance } from "./eyes";
import { FORM, grip, type Lump, type Point } from "./form";
import type { Mood } from "./moods";

const TAU = Math.PI * 2;

/** Drawings a second. On twos, at the frame rate animation has always used. */
export const FPS = 12;

/**
 * Pose to pose. A change of mood is five drawings: the old pose, one halfway,
 * one past the new pose, one just short of it, and the new pose.
 */
export const POSES = [0, 0.5, 1.14, 0.97, 1] as const;

/** How far into a change of mood the drawing is, from the seconds since it began. */
export function poseAt(since: number): number {
  const k = Math.floor(Math.max(0, since) * FPS);
  return POSES[Math.min(k, POSES.length - 1)] as number;
}

/** Under this many CSS pixels a creature has no mouth. */
export const MOUTH_PX = 40;

/** An ink eye is a pupil and not a ball: this much of the eyeball's radius. */
const DOT = 2 / 3;

/** How far inside the outline a brow or a mouth is kept, in viewBox units. */
const ROOM = 0.5;

/** An eye, as one stroke. The stroke the cut cast used to draw, before it had lids. */
export interface Mark {
  /** Half the length of the stroke, in eye radii. 0 is a dot. */
  w: number;
  /** Weight, so `h` of 2 with `w` of 0 is a dot of radius 1. */
  h: number;
  /** Bow. Negative curves up. */
  c: number;
  /** Degrees, mirrored. Positive drops the inner ends. */
  a: number;
  /** Widens the pair, and moves it down, in eye radii. */
  sep: number;
  dy: number;
  /** The eye on the viewer's right higher, and narrower, in eye radii. */
  skew: number;
  lop: number;
}

export interface Brow {
  /** 0 is no brow. In eye radii. */
  weight: number;
  /** Above the eye's middle, in eye radii. */
  lift: number;
  /** Degrees. Positive drops the inner end. */
  tilt: number;
  len: number;
  bow: number;
  /** The brow on the viewer's right higher by this, and turned by `turn`. */
  skew: number;
  turn: number;
}

export interface Mouth {
  /** 0 is no mouth. In eye radii. */
  weight: number;
  w: number;
  /** Positive smiles. */
  curve: number;
  wave: number;
  /** An open "o" past a half. */
  o: number;
  /** Off to one side, in eye radii. */
  x: number;
}

export interface Line {
  /** In viewBox units, before hinting. */
  weight: number;
  /** How far the line wanders, and how many new drawings of it a second. */
  boil: number;
  rate: number;
  scratch: number;
  /** Share of the stroke that has run dry. */
  dry: number;
}

/**
 * What comics draws beside a head, and has a word for: emanata. A scribble is
 * cross, a drop of sweat is worry, lines round a head are a shock, a star is
 * pleased. `bang` is the one that is amber, and it is still only `blocked`.
 */
export type Emanata = "dots" | "bang" | "z" | "plewd" | "scribble" | "shock" | "sparkle";

export interface Face {
  eye: Mark;
  brow: Brow;
  mouth: Mouth;
  line: Line;
  blink?: boolean | "slow";
  jitter?: number;
  /** As the look goes up: the brow on the right cocks, the eye narrows. */
  squint?: { at: number; skew: number; turn: number; lop: number };
  mark?: Emanata;
  /** Where the mark on its head wants to point, in degrees, and how stiff it is. */
  tuft: { rest: number; stiff: number };
}

const DOT_EYE: Mark = { w: 0.02, h: 2, c: 0, a: 0, sep: 0, dy: 0, skew: 0, lop: 0 };
const NO_BROW: Brow = { weight: 0, lift: 1.9, tilt: 0, len: 1.1, bow: 0.2, skew: 0, turn: 0 };
const NO_MOUTH: Mouth = { weight: 0, w: 1, curve: 0, wave: 0, o: 0, x: 0 };
const CLEAN: Line = { weight: 1.5, boil: 0, rate: 0, scratch: 0, dry: 0 };

export const DRAWN: Record<Mood, Face> = {
  idle: {
    eye: DOT_EYE,
    brow: NO_BROW,
    mouth: NO_MOUTH,
    line: CLEAN,
    blink: true,
    tuft: { rest: 0, stiff: 1 },
  },
  listening: {
    eye: { ...DOT_EYE, h: 2.35, dy: -0.2 },
    brow: { ...NO_BROW, weight: 0.5, lift: 2.35, tilt: -6, bow: 0.25 },
    mouth: NO_MOUTH,
    line: { ...CLEAN, weight: 1.65 },
    blink: true,
    tuft: { rest: 0, stiff: 1.4 },
  },
  thinking: {
    eye: { ...DOT_EYE, w: 0.3, h: 1.7, a: -8, skew: 0.3, lop: 0.3 },
    brow: { ...NO_BROW, weight: 0.65, lift: 1.9, tilt: -4, skew: 0.75, turn: -16 },
    mouth: { ...NO_MOUTH, weight: 0.45, w: 0.75, x: 0.9 },
    line: { ...CLEAN, boil: 0.28, rate: 3 },
    blink: true,
    mark: "dots",
    tuft: { rest: -8, stiff: 0.8 },
  },
  /* Narrowed and reading. No brow: two dashes under two dashes is a face
     nobody can read at 24px, and the boiling line already says busy. */
  working: {
    eye: { ...DOT_EYE, w: 1, h: 1.05, a: 8, dy: 0.35 },
    brow: NO_BROW,
    mouth: NO_MOUTH,
    line: { ...CLEAN, weight: 1.6, boil: 0.42, rate: 12 },
    blink: true,
    tuft: { rest: 0, stiff: 1 },
  },
  frustrated: {
    eye: { ...DOT_EYE, w: 0.15, h: 1.7, dy: 0.2 },
    brow: { ...NO_BROW, weight: 1.05, lift: 1.3, tilt: 30, len: 1.25, bow: -0.1 },
    mouth: { ...NO_MOUTH, weight: 0.5, w: 1.2, wave: 0.55 },
    line: { weight: 1.85, boil: 0.55, rate: 12, scratch: 0.45, dry: 0 },
    blink: "slow",
    jitter: 0.06,
    mark: "scribble",
    tuft: { rest: 0, stiff: 3 },
  },
  blocked: {
    eye: { ...DOT_EYE, h: 2.3 },
    brow: { ...NO_BROW, weight: 0.6, lift: 2, tilt: -10, bow: 0.25 },
    mouth: { ...NO_MOUTH, weight: 0.45, w: 0.7 },
    line: { ...CLEAN, weight: 1.6, boil: 0.15, rate: 2 },
    squint: { at: 0.28, skew: 0.85, turn: 20, lop: 0.35 },
    blink: "slow",
    mark: "bang",
    tuft: { rest: 10, stiff: 1 },
  },
  pleased: {
    eye: { ...DOT_EYE, w: 1, h: 0.9, c: -0.85, dy: -0.1 },
    brow: NO_BROW,
    mouth: { ...NO_MOUTH, weight: 0.5, w: 1.3, curve: 1 },
    line: { ...CLEAN, weight: 1.75 },
    blink: false,
    mark: "sparkle",
    tuft: { rest: 0, stiff: 0.7 },
  },
  paused: {
    eye: { ...DOT_EYE, w: 1.25, h: 0.55, c: 0.25 },
    brow: NO_BROW,
    mouth: NO_MOUTH,
    line: { ...CLEAN, weight: 1.15, dry: 0.6 },
    blink: false,
    mark: "z",
    tuft: { rest: 38, stiff: 0.5 },
  },
  stuck: {
    eye: { ...DOT_EYE, w: 0.05, h: 1.8, dy: 0.35 },
    brow: { ...NO_BROW, weight: 0.85, lift: 1.6, tilt: -28, len: 1.15, bow: 0.1 },
    mouth: { ...NO_MOUTH, weight: 0.45, w: 0.9, curve: -0.35, wave: 0.3 },
    line: { ...CLEAN, boil: 0.3, rate: 4 },
    blink: "slow",
    jitter: 0.03,
    mark: "plewd",
    tuft: { rest: 24, stiff: 0.6 },
  },
  surprised: {
    eye: { ...DOT_EYE, h: 2.8, sep: 0.5 },
    brow: { ...NO_BROW, weight: 0.7, lift: 2.8, tilt: -4, bow: 0.35 },
    mouth: { ...NO_MOUTH, weight: 0.5, w: 0.9, o: 1 },
    line: { ...CLEAN, weight: 2.05 },
    blink: false,
    mark: "shock",
    tuft: { rest: 0, stiff: 4 },
  },
};

/** Every number on one of these lerps. Past 1 is the overshoot drawing. */
export function mixFace<T extends object>(a: T, b: T, u: number): T {
  const out = { ...a } as Record<string, number>;
  const x = a as Record<string, number>;
  const y = b as Record<string, number>;
  for (const k of Object.keys(a)) out[k] = (x[k] ?? 0) + ((y[k] ?? 0) - (x[k] ?? 0)) * u;
  return out as T;
}

/** The size a line is drawn at its own weight, in CSS pixels. Smaller draws it heavier. */
const HINT_PX = 40;

/** How much heavier a line is drawn on a small creature, from its size in CSS pixels. */
export function hint(px: number, most: number): number {
  return Math.max(1, Math.min(most, HINT_PX / px));
}

/* --- strokes --------------------------------------------------------------- */

function pathOf(pts: Point[], closed: boolean): string {
  let d = "";
  for (let i = 0; i < pts.length; i++) {
    const p = pts[i] as Point;
    d += `${i === 0 ? "M" : "L"}${p[0].toFixed(2)} ${p[1].toFixed(2)}`;
  }
  return closed && d ? `${d}Z` : d;
}

/** A filled stroke along a centerline, with its own width at every point and round ends. */
export function stroke(center: Point[], widths: number[]): string {
  const n = center.length;
  if (n < 2) return "";
  const tangent = (i: number): Point => {
    const a = center[Math.max(0, i - 1)] as Point;
    const b = center[Math.min(n - 1, i + 1)] as Point;
    const l = Math.hypot(b[0] - a[0], b[1] - a[1]) || 1;
    return [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
  };
  const left: Point[] = [];
  const right: Point[] = [];
  for (let i = 0; i < n; i++) {
    const [tx, ty] = tangent(i);
    const w = (widths[i] ?? 0) / 2;
    const p = center[i] as Point;
    left.push([p[0] - ty * w, p[1] + tx * w]);
    right.push([p[0] + ty * w, p[1] - tx * w]);
  }
  /* Round ends as points rather than arc commands: an arc's sweep flag is a
     second thing to get right, and this is the same cap either way round. */
  const cap = (i: number, forward: boolean): Point[] => {
    const [tx, ty] = tangent(i);
    const d = forward ? 1 : -1;
    const w = (widths[i] ?? 0) / 2;
    const p = center[i] as Point;
    const nx = forward ? -ty : ty;
    const ny = forward ? tx : -tx;
    const out: Point[] = [];
    for (let k = 1; k < 6; k++) {
      const phi = (k / 6) * Math.PI;
      out.push([
        p[0] + (nx * Math.cos(phi) + tx * d * Math.sin(phi)) * w,
        p[1] + (ny * Math.cos(phi) + ty * d * Math.sin(phi)) * w,
      ]);
    }
    return out;
  };
  return pathOf([...left, ...cap(n - 1, true), ...right.reverse(), ...cap(0, false)], true);
}

/** A quadratic from a to b bowed by `bow` (positive bows up the screen), sampled. */
function arc(a: Point, b: Point, bow: number, n = 10): Point[] {
  const lx = b[0] - a[0];
  const ly = b[1] - a[1];
  const l = Math.hypot(lx, ly) || 1;
  const cx = (a[0] + b[0]) / 2 + (ly / l) * bow * 2;
  const cy = (a[1] + b[1]) / 2 - (lx / l) * bow * 2;
  const out: Point[] = [];
  for (let i = 0; i <= n; i++) {
    const s = i / n;
    const q = 1 - s;
    out.push([
      q * q * a[0] + 2 * q * s * cx + s * s * b[0],
      q * q * a[1] + 2 * q * s * cy + s * s * b[1],
    ]);
  }
  return out;
}

/** Catmull-Rom through a loop (`closed`) or a run of points, `per` samples a segment. */
function spline(pts: Point[], per: number, closed: boolean): Point[] {
  const n = pts.length;
  const at = (i: number) =>
    (closed ? pts[(i + n) % n] : pts[Math.max(0, Math.min(n - 1, i))]) as Point;
  const out: Point[] = [];
  const segments = closed ? n : n - 1;
  for (let i = 0; i < segments; i++) {
    const p0 = at(i - 1);
    const p1 = at(i);
    const p2 = at(i + 1);
    const p3 = at(i + 2);
    for (let j = 0; j < per; j++) {
      const s = j / per;
      const s2 = s * s;
      const s3 = s2 * s;
      const f = (a: number, b: number, c: number, d: number) =>
        0.5 *
        (2 * b + (-a + c) * s + (2 * a - 5 * b + 4 * c - d) * s2 + (-a + 3 * b - 3 * c + d) * s3);
      out.push([f(p0[0], p1[0], p2[0], p3[0]), f(p0[1], p1[1], p2[1], p3[1])]);
    }
  }
  if (!closed) out.push(pts[n - 1] as Point);
  return out;
}

/** Deterministic noise. The same creature boils the same way every reload. */
function rnd(i: number): number {
  const x = Math.sin(i * 127.1 + 197.3) * 43758.5453;
  return x - Math.floor(x);
}

/** A number from a string, for anything a character keeps for life. */
export function seedOf(key: string): number {
  let h = 0;
  for (let i = 0; i < key.length; i++) h = (h * 31 + key.charCodeAt(i)) >>> 0;
  return (h % 10007) / 10007;
}

/** The pigment: the outline, printed a unit down and to the right of its line. */
export function pigment(pts: Point[]): string {
  return pathOf(
    spline(pts, 3, true).map(([x, y]): Point => [x + 1.3, y + 1.1]),
    true,
  );
}

/**
 * The body's line: one brush stroke round the outline, heavier underneath,
 * starting at the top left and running a little past where it began, which is
 * what a hand drawing a closed shape does. `step` is which drawing of the boil
 * this is; the same step is the same line.
 */
export function brush(
  pts: Point[],
  line: Line,
  step: number,
  seed: number,
  weight: number,
): string {
  const ring = spline(pts, 4, true);
  const n = ring.length;
  let start = 0;
  let best = Number.NEGATIVE_INFINITY;
  for (let i = 0; i < n; i++) {
    const [x, y] = ring[i] as Point;
    const score = -(x - FORM.center) - (y - FORM.center) * 1.4;
    if (score > best) {
      best = score;
      start = i;
    }
  }
  const phase = (k: number) => rnd(step * 3.1 + k * 1.7 + seed * 17) * TAU;
  const [p1, p2, p3, p4] = [phase(1), phase(2), phase(3), phase(4)];
  const total = Math.round(n * 1.05);
  const center: Point[] = [];
  const widths: number[] = [];
  for (let j = 0; j <= total; j++) {
    const i = (start + j) % n;
    const [x, y] = ring[i] as Point;
    const th = (i / n) * TAU;
    const s = j / total;
    const wander =
      line.boil *
        (0.5 * Math.sin(2 * th + p1) + 0.3 * Math.sin(3 * th + p2) + 0.2 * Math.sin(5 * th + p3)) +
      line.scratch * Math.sin(13 * th + p4) * Math.sin(7 * th + p1);
    const dx = x - FORM.center;
    const dy = y - FORM.center;
    const l = Math.hypot(dx, dy) || 1;
    center.push([x + (dx / l) * wander, y + (dy / l) * wander]);
    const down = (1 + dy / l) / 2;
    const taper = Math.min(1, 0.4 + s / 0.06, (1 - s) / 0.12);
    const dry = line.dry > 0 && Math.sin(s * TAU * 9 + seed * 85) > 1 - line.dry * 1.6 ? 0.08 : 1;
    const press = 1 + line.boil * 0.35 * Math.sin(9 * th + p2);
    widths.push(Math.max(0, line.weight * weight * (0.62 + 0.55 * down) * taper * dry * press));
  }
  return stroke(center, widths);
}

/* --- what is on its head --------------------------------------------------- */

export type Tuft = "curl" | "leaf" | "tick" | "none";

/**
 * Which mark a character wears, for life. A second thing that tells two
 * creatures cut from one silhouette apart at 24px, and the one piece of
 * secondary motion in the set: it lags the body.
 */
export function tuftOf(key: string): Tuft {
  const kinds: Tuft[] = ["curl", "tick", "none", "leaf", "tick", "curl", "none"];
  return kinds[Math.floor(seedOf(`${key}:tuft`) * kinds.length)] as Tuft;
}

const TUFT_LINES: Record<Tuft, Point[][]> = {
  curl: [
    [
      [0, 0],
      [0.3, -1.8],
      [0.25, -3.4],
      [-0.6, -4.4],
      [-1.6, -4.1],
      [-1.7, -3.2],
    ],
  ],
  tick: [
    [
      [0, 0],
      [0.4, -2.4],
      [0.9, -4.2],
    ],
  ],
  leaf: [
    [
      [0, 0],
      [0.1, -2.2],
    ],
    [
      [0.1, -2.2],
      [1.6, -3.6],
      [3, -3.4],
      [1.7, -2.3],
      [0.1, -2.2],
    ],
  ],
  none: [],
};

/** The mark on its head, standing on the crown of the outline and turned by `deg`. */
export function tuftPath(kind: Tuft, pts: Point[], deg: number, width: number): string {
  if (kind === "none") return "";
  let crown = 0;
  for (let i = 1; i < pts.length; i++) {
    if ((pts[i] as Point)[1] < (pts[crown] as Point)[1]) crown = i;
  }
  const [kx, ky] = pts[crown] as Point;
  const a = (deg * Math.PI) / 180;
  return TUFT_LINES[kind]
    .map((seg) => {
      const placed = seg.map(
        ([x, y]): Point => [
          kx + x * Math.cos(a) - y * Math.sin(a),
          ky + 0.6 + x * Math.sin(a) + y * Math.cos(a),
        ],
      );
      const run = placed.length > 2 ? spline(placed, 4, false) : placed;
      return stroke(
        run,
        run.map((_, i) => width * (1 - (i / run.length) * 0.55)),
      );
    })
    .join("");
}

/* --- the face ------------------------------------------------------------------ */

export interface FaceAt {
  t: number;
  live: boolean;
  /** Where it is looking, smoothed, in body radii. */
  gaze: Point;
  /** The mood's own gaze, for the blink a large look is hidden behind. None while aimed. */
  watch?: Gaze;
  px: number;
}

export interface DrawnFace {
  /** Whether the eyes are shut this drawing: a blink, or a large look being hidden. */
  closed: boolean;
  eyes: string[];
  brows: string[];
  mouth: string;
  /** The "o", when the mouth is one: center and radii. */
  o: { x: number; y: number; rx: number; ry: number; w: number } | null;
  /** Everything above, as points and a pad, for the suite that holds it inside the body. */
  ink: { pts: Point[]; pad: number }[];
}

/** How far a point is inside the outline; negative is outside. Ray cast, so a notch counts. */
function depth(pts: Point[], x: number, y: number): number {
  let near = Number.POSITIVE_INFINITY;
  let within = false;
  for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
    const [xi, yi] = pts[i] as Point;
    const [xj, yj] = pts[j] as Point;
    if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) within = !within;
    const ex = xj - xi;
    const ey = yj - yi;
    const len = ex * ex + ey * ey;
    const u = len === 0 ? 0 : Math.max(0, Math.min(1, ((x - xi) * ex + (y - yi) * ey) / len));
    near = Math.min(near, Math.hypot(x - (xi + ex * u), y - (yi + ey * u)));
  }
  return within ? near : -near;
}

/**
 * A stroke kept inside the outline, point by point: any point with less than
 * `pad` of body round it is drawn in toward `anchor` until it has that much.
 * A brow over a cloud's notch bends down into it rather than floating in the
 * air above it, which is what a brow on a brow ridge does anyway, and a mouth
 * on a squat body rides up off the floor. Measured on the outline itself, as
 * the cut cast's stretch is, because a bound on the numbers that made a
 * drawing is not a bound on the drawing.
 */
function inside(pts: Point[], run: Point[], pad: number, anchor: Point): Point[] {
  return run.map((p): Point => {
    if (depth(pts, p[0], p[1]) >= pad) return p;
    let lo = 0;
    let hi = 1;
    for (let k = 0; k < 14; k++) {
      const mid = (lo + hi) / 2;
      const x = p[0] + (anchor[0] - p[0]) * mid;
      const y = p[1] + (anchor[1] - p[1]) * mid;
      if (depth(pts, x, y) >= pad) hi = mid;
      else lo = mid;
    }
    return [p[0] + (anchor[0] - p[0]) * hi, p[1] + (anchor[1] - p[1]) * hi];
  });
}

function dot(x: number, y: number, r: number): string {
  const f = (v: number) => v.toFixed(2);
  return `M${f(x - r)} ${f(y)}a${f(r)} ${f(r)} 0 1 0 ${f(2 * r)} 0a${f(r)} ${f(r)} 0 1 0 ${f(-2 * r)} 0Z`;
}

/** One frame of one face, on the outline it is drawn inside. */
export function faceAt(lump: Lump, face: Face, at: FaceAt, pts: Point[]): DrawnFace {
  let eye = face.eye;
  let brow = face.brow;
  const [gx, gy] = at.gaze;
  const up = Math.max(0, Math.min(1, -gy / (face.squint?.at ?? 0.28)));
  if (face.squint && up > 0) {
    brow = {
      ...brow,
      skew: brow.skew + face.squint.skew * up,
      turn: brow.turn + face.squint.turn * up,
    };
    eye = { ...eye, lop: eye.lop + face.squint.lop * up };
  }
  /* A look down narrows the eye toward a line, the one lid a dot has. */
  const down = Math.max(0, Math.min(1, gy / 0.3));
  if (down > 0) {
    eye = {
      ...eye,
      w: eye.w + (1 - eye.w) * 0.45 * down,
      h: eye.h * (1 - 0.4 * down),
      c: eye.c + 0.2 * down,
      dy: eye.dy + 0.4 * down,
    };
  }

  /* Shut on a blink, and shut through a large look: the look is a cut hidden
     by a blink, as an animator hides a head turn. */
  let closed = false;
  if (at.live && face.blink !== false) {
    closed = blinkAmount(at.t * (face.blink === "slow" ? 0.5 : 0.85)) > 0.4;
  }
  if (at.live && at.watch) {
    const g = glance(at.t, at.watch);
    if (g.size > 0.2 && g.jump < 2 / FPS) closed = true;
  }
  if (closed) eye = { ...eye, w: 1.2, h: 0.5, c: 0.1 };

  const r = lump.eye.r * DOT;
  const one = Boolean(lump.eye.one);
  const jitter = at.live && face.jitter ? face.jitter * r : 0;
  const seen = Math.hypot(gx, gy);
  const lead = seen > 0.004 ? grip(seen) / seen : 0;
  const ex =
    FORM.center +
    (lump.eye.x ?? 0) +
    gx * FORM.radius * (1 + 0.6 * lead) +
    Math.sin(at.t * 41) * jitter;
  const ey =
    FORM.center +
    (lump.eye.y ?? 0) +
    gy * FORM.radius +
    eye.dy * r +
    Math.sin(at.t * 33) * jitter * 0.7;
  const sep = (lump.eye.spread + eye.sep * r) * (1 - Math.abs(gx) * 0.5);
  const heavy = hint(at.px, 1.5);

  const eyes: string[] = [];
  const brows: string[] = [];
  const ink: { pts: Point[]; pad: number }[] = [];
  for (const side of one ? [0] : [-1, 1]) {
    const er = r * (1 + side * gx * 0.3);
    const lop = eye.lop * side;
    const w = Math.max(0.02, eye.w + lop) * er;
    const h = Math.max(0.12, eye.h * (1 - lop * 0.45)) * er * heavy;
    const x = ex + side * sep;
    const y = ey + (side > 0 ? -eye.skew * er : 0);
    if (w < h * 0.3) {
      eyes.push(dot(x, y, h / 2));
      ink.push({ pts: [[x, y]], pad: h / 2 });
    } else {
      const tilt = side === 0 ? 0 : (eye.a * side * Math.PI) / 180;
      const ax = Math.cos(tilt) * w;
      const ay = Math.sin(tilt) * w * -side;
      const center = arc([x - ax, y - ay], [x + ax, y + ay], -eye.c * er);
      eyes.push(
        stroke(
          center,
          center.map((_, k) => h * (0.72 + 0.28 * Math.sin((k / (center.length - 1)) * Math.PI))),
        ),
      );
      ink.push({ pts: center, pad: h / 2 });
    }

    if (brow.weight > 0.04) {
      const mine = side > 0 ? brow.skew : 0;
      const deg = brow.tilt + (side > 0 ? brow.turn : 0);
      /* A single eye has no inner end: its brow bends in the middle instead. */
      const t = side === 0 ? 0 : (deg * Math.PI) / 180;
      const bend = side === 0 ? -Math.sin((deg * Math.PI) / 180) * brow.len * er * 0.8 : 0;
      const by = ey - (brow.lift + mine) * er;
      const half = brow.len * er;
      const inner = side === 0 ? 1 : -side;
      const a: Point = [x + inner * Math.cos(t) * half, by + Math.sin(t) * half];
      const b: Point = [x - inner * Math.cos(t) * half, by - Math.sin(t) * half];
      const innerFirst = a[0] < b[0];
      const bw = brow.weight * er * 0.9 * heavy;
      const center = inside(
        pts,
        arc(innerFirst ? a : b, innerFirst ? b : a, brow.bow * er + bend),
        bw / 2 + ROOM,
        [x, y],
      );
      /* Inner end first: that is where a brush presses. */
      brows.push(
        stroke(
          center,
          center.map((_, k) => {
            const s = k / (center.length - 1);
            return bw * (1 - 0.55 * (innerFirst ? s : 1 - s));
          }),
        ),
      );
      ink.push({ pts: center, pad: bw / 2 });
    }
  }

  let mouth = "";
  let o: DrawnFace["o"] = null;
  const mo = face.mouth;
  if (mo.weight > 0.05 && at.px >= MOUTH_PX && !closed) {
    const my = ey + (one ? 1.9 : 2.5) * r;
    const mx = ex + mo.x * r;
    const mw = mo.w * r;
    const weight = mo.weight * r * 0.95;
    if (mo.o > 0.5) {
      const ry = mw * 0.72;
      /* Kept whole: an "o" pulled in point by point would not be an o. */
      const [[, bottom]] = inside(pts, [[mx, my + 0.4 + ry]], weight / 2 + ROOM, [ex, ey]) as [
        Point,
      ];
      const oy = Math.min(my + 0.4, bottom - ry);
      o = { x: mx, y: oy, rx: mw * 0.55, ry, w: weight };
      ink.push({
        pts: [
          [mx - mw * 0.55, oy],
          [mx + mw * 0.55, oy],
          [mx, oy + ry],
          [mx, oy - ry],
        ],
        pad: weight / 2,
      });
    } else {
      const center: Point[] = [];
      for (let k = 0; k <= 12; k++) {
        const s = k / 12;
        const bow = 1 - (2 * s - 1) ** 2;
        center.push([
          mx - mw + 2 * mw * s,
          my + mo.curve * bow * r * 0.55 + mo.wave * Math.sin(s * TAU * 1.5) * r * 0.3,
        ]);
      }
      const kept = inside(pts, center, weight / 2 + ROOM, [ex, ey]);
      mouth = stroke(
        kept,
        kept.map(() => weight),
      );
      ink.push({ pts: kept, pad: weight / 2 });
    }
  }
  return { closed, eyes, brows, mouth, o, ink };
}

/* --- beside the head ------------------------------------------------------------ */

/**
 * The marks beside a head, drawn on twos like everything else, as markup for
 * the mark group. `frame` is which drawing this is and `since` is how long the
 * mood has been held, because two of these are only there for a moment. Ink
 * is `var(--eye)`, which the mark group maps to the page's text color so a
 * mark stays visible in a dark room. `bang` is not here: it is `markFor`'s,
 * amber, and the same in both casts.
 */
export function emanata(
  kind: Emanata | undefined,
  at: { frame: number; since: number; live: boolean; px: number },
  pts: Point[],
  seed: number,
): string {
  const w = (1.4 * hint(at.px, 1.6)).toFixed(2);
  const ink = `stroke="var(--eye)" stroke-width="${w}" fill="none" stroke-linecap="round" stroke-linejoin="round"`;
  const f = (v: number) => v.toFixed(2);
  switch (kind) {
    case "dots": {
      const shown = at.live ? Math.floor(at.frame / 4) % 4 : 3;
      const dots: [number, number, number][] = [
        [48, 12, 1.5],
        [52.6, 9.4, 1.8],
        [57.4, 6.2, 2.1],
      ];
      return dots
        .slice(0, shown)
        .map(
          ([x, y, r]) => `<circle cx="${x}" cy="${y}" r="${r}" fill="var(--eye)" opacity="0.75"/>`,
        )
        .join("");
    }
    case "z": {
      const k = at.live ? Math.floor(at.frame / 3) % 6 : 2;
      const x = 45 + k * 0.7;
      const y = 15 - k * 1.3;
      const s = 0.75 + k * 0.07;
      return `<path d="M${f(x - 3.2 * s)} ${f(y - 3.2 * s)}h${f(6.4 * s)}l${f(-6.4 * s)} ${f(6.4 * s)}h${f(6.4 * s)}" ${ink} opacity="${f(0.75 - k * 0.1)}"/>`;
    }
    case "plewd": {
      /* Up and to the left of the head, which is where a drop of sweat goes. */
      let left = 0;
      let most = Number.NEGATIVE_INFINITY;
      for (let i = 0; i < pts.length; i++) {
        const [x, y] = pts[i] as Point;
        const score = -(x - FORM.center) * 0.8 - (y - FORM.center) * 0.6;
        if (score > most) {
          most = score;
          left = i;
        }
      }
      const [px, py] = pts[left] as Point;
      const x = px - 2.2;
      const y = py - 1 + (at.live ? (Math.floor(at.frame / 3) % 5) * 0.5 : 0);
      return `<path d="M${f(x)} ${f(y - 3.2)}C${f(x + 1.9)} ${f(y - 0.4)} ${f(x + 1.9)} ${f(y + 1.6)} ${f(x)} ${f(y + 1.6)}S${f(x - 1.9)} ${f(y - 0.4)} ${f(x)} ${f(y - 3.2)}Z" stroke="var(--eye)" stroke-width="${w}" fill="var(--sclera)" stroke-linejoin="round"/>`;
    }
    case "scribble": {
      const step = at.live ? at.frame : 0;
      const loop: Point[] = [];
      for (let i = 0; i < 9; i++) {
        const a = i * 2.3 + rnd(step * 1.3 + i + seed) * 1.2;
        const rr = 2.2 + rnd(step * 0.7 + i * 3.1 + seed) * 2.2;
        loop.push([52 + Math.cos(a) * rr, 10 + Math.sin(a) * rr * 0.75]);
      }
      return `<path d="${pathOf(spline(loop, 4, false), false)}" ${ink}/>`;
    }
    case "shock": {
      if (at.live && at.since > 0.75) return "";
      const k = at.live ? Math.min(2, Math.floor(at.since * FPS)) : 1;
      const len = 2 + k * 0.9;
      return [-0.62, -0.5, -0.38]
        .map((turn) => {
          const a = turn * TAU;
          const r0 = 23 + k * 0.4;
          const x0 = FORM.center + Math.cos(a) * r0;
          const y0 = FORM.center + 2 + Math.sin(a) * r0;
          return `<path d="M${f(x0)} ${f(y0)}l${f(Math.cos(a) * len)} ${f(Math.sin(a) * len)}" ${ink}/>`;
        })
        .join("");
    }
    case "sparkle": {
      if (at.live && at.since > 1.8) return "";
      const k = at.live ? Math.floor(at.since * FPS) % 4 : 1;
      const s = [1.9, 3, 2.4, 3.2][k] as number;
      const x = 52;
      const y = 11;
      return `<path d="M${x} ${f(y - s)}Q${x} ${y} ${f(x + s)} ${y}Q${x} ${y} ${x} ${f(y + s)}Q${x} ${y} ${f(x - s)} ${y}Q${x} ${y} ${x} ${f(y - s)}Z" fill="var(--eye)"/>`;
    }
    default:
      return "";
  }
}
