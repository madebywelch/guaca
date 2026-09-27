/**
 * C. Ink. Drawn, not cut.
 *
 * The body is the app's own geometry (`bodyPoints`), drawn as one brush stroke
 * over a pigment printed a unit out of register. It is animated on twos: twelve
 * drawings a second, held between, so a look is a snap and a hold is dead
 * still. The line is an emotional channel of its own: calm moods hold it
 * clean, work boils it, frustration scratches it, a paused creature's line
 * dries out.
 *
 * The face gets back two things CHARACTERS.md took away, brows and a mouth.
 * Both are strokes computed from the eye they belong to, so neither can fall
 * out of register with it, and the mouth is only drawn at 40px and up.
 */

import { AIM } from "../../src/avatars/eyes";
import { blend, bodyPoints, grip } from "../../src/avatars/form";
import { markFor, MOODS, type Mood } from "../../src/avatars/moods";
import {
  blinkAt,
  C,
  clamp,
  type Design,
  el,
  follow,
  type GazeSpec,
  hint,
  lerp,
  type Point,
  poly,
  R,
  resample,
  rnd,
  saccade,
  TAU,
} from "./shared";

const FPS = 12;

interface Mark {
  w: number;
  h: number;
  c: number;
  a: number;
  sep: number;
  dy: number;
  skew: number;
  lop: number;
}

interface Brow {
  /** 0 is no brow. */
  weight: number;
  /** Above the eye's middle, in eye radii. */
  lift: number;
  /** Degrees. Positive drops the inner end. */
  tilt: number;
  len: number;
  bow: number;
  /** The creature's right brow higher by this, and turned by `turn`. */
  skew: number;
  turn: number;
}

interface Mouth {
  /** 0 is no mouth. */
  weight: number;
  w: number;
  /** Positive smiles. */
  curve: number;
  wave: number;
  /** An open "o". */
  o: number;
  /** Off to one side, in eye radii. */
  x: number;
}

interface Line {
  weight: number;
  /** How far the line wanders, and how many new drawings of it a second. */
  boil: number;
  rate: number;
  scratch: number;
  /** Share of the stroke that has run dry. */
  dry: number;
}

type Emanata = "dots" | "bang" | "z" | "plewd" | "scribble" | "shock" | "sparkle";

interface Ink {
  eye: Mark;
  brow: Brow;
  mouth: Mouth;
  line: Line;
  gaze?: GazeSpec;
  blink?: boolean | "slow";
  jitter?: number;
  /** When looking up: the brow the creature's right cocks, the eye it narrows. */
  squint?: { skew: number; turn: number; lop: number };
  mark?: Emanata;
  /** Where the mark on its head wants to point, in degrees, and how stiff it is. */
  tuft: { rest: number; stiff: number };
  dim?: boolean;
}

const DOT: Mark = { w: 0.02, h: 2, c: 0, a: 0, sep: 0, dy: 0, skew: 0, lop: 0 };
const NO_BROW: Brow = { weight: 0, lift: 1.9, tilt: 0, len: 1.1, bow: 0.2, skew: 0, turn: 0 };
const NO_MOUTH: Mouth = { weight: 0, w: 1, curve: 0, wave: 0, o: 0, x: 0 };
const CLEAN: Line = { weight: 1.5, boil: 0, rate: 0, scratch: 0, dry: 0 };

const READING: [number, number, number][] = [
  [-0.18, 0.12, 0.42],
  [-0.06, 0.12, 0.3],
  [0.06, 0.12, 0.34],
  [0.17, 0.13, 0.4],
  [-0.18, 0.16, 0.46],
  [-0.05, 0.16, 0.28],
  [0.08, 0.16, 0.36],
  [0.18, 0.17, 0.52],
];

const INKS: Record<Mood, Ink> = {
  idle: {
    eye: DOT,
    brow: NO_BROW,
    mouth: NO_MOUTH,
    line: CLEAN,
    gaze: { range: 0.4, hz: 0.32, far: 0.3 },
    tuft: { rest: 0, stiff: 1 },
  },
  listening: {
    eye: { ...DOT, h: 2.35, dy: -0.2 },
    brow: { ...NO_BROW, weight: 0.5, lift: 2.35, tilt: -6, bow: 0.25 },
    mouth: NO_MOUTH,
    line: { ...CLEAN, weight: 1.65 },
    gaze: { range: 0.05, hz: 0.5 },
    tuft: { rest: 0, stiff: 1.4 },
  },
  thinking: {
    eye: { ...DOT, w: 0.3, h: 1.7, a: -8, skew: 0.3, lop: 0.3 },
    brow: { ...NO_BROW, weight: 0.65, lift: 1.9, tilt: -4, skew: 0.75, turn: -16 },
    mouth: { ...NO_MOUTH, weight: 0.45, w: 0.75, x: 0.9 },
    line: { ...CLEAN, boil: 0.28, rate: 3 },
    gaze: { range: 0.2, hz: 0.6, bias: [-0.2, -0.2], far: 0.25 },
    mark: "dots",
    tuft: { rest: -8, stiff: 0.8 },
  },
  working: {
    eye: { ...DOT, w: 1, h: 1.05, a: 8, dy: 0.35 },
    brow: NO_BROW,
    mouth: NO_MOUTH,
    line: { ...CLEAN, weight: 1.6, boil: 0.42, rate: 12 },
    gaze: { cross: 0.05, script: READING },
    tuft: { rest: 0, stiff: 1 },
  },
  frustrated: {
    eye: { ...DOT, w: 0.15, h: 1.7, dy: 0.2 },
    brow: { ...NO_BROW, weight: 1.05, lift: 1.3, tilt: 30, len: 1.25, bow: -0.1 },
    mouth: { ...NO_MOUTH, weight: 0.5, w: 1.2, wave: 0.55 },
    line: { weight: 1.85, boil: 0.55, rate: 12, scratch: 0.45, dry: 0 },
    gaze: { range: 0.34, hz: 1.4, far: 0.22, near: 0.12 },
    blink: "slow",
    jitter: 0.06,
    mark: "scribble",
    tuft: { rest: 0, stiff: 3 },
  },
  blocked: {
    eye: { ...DOT, h: 2.3 },
    brow: { ...NO_BROW, weight: 0.6, lift: 2, tilt: -10, bow: 0.25 },
    mouth: { ...NO_MOUTH, weight: 0.45, w: 0.7 },
    line: { ...CLEAN, weight: 1.6, boil: 0.15, rate: 2 },
    squint: { skew: 0.85, turn: 20, lop: 0.35 },
    blink: "slow",
    gaze: {
      cross: 0.12,
      script: [
        [0, 0.02, 1.7],
        [0.24, -0.25, 1.2],
        [0.28, -0.3, 0.9],
        [0, 0.02, 1.5],
      ],
    },
    mark: "bang",
    tuft: { rest: 10, stiff: 1 },
  },
  pleased: {
    eye: { ...DOT, w: 1, h: 0.9, c: -0.85, dy: -0.1 },
    brow: NO_BROW,
    mouth: { ...NO_MOUTH, weight: 0.5, w: 1.3, curve: 1 },
    line: { ...CLEAN, weight: 1.75 },
    blink: false,
    gaze: { range: 0.08, hz: 0.3, bias: [0, -0.05] },
    mark: "sparkle",
    tuft: { rest: 0, stiff: 0.7 },
  },
  paused: {
    eye: { ...DOT, w: 1.25, h: 0.55, c: 0.25 },
    brow: NO_BROW,
    mouth: NO_MOUTH,
    line: { ...CLEAN, weight: 1.15, dry: 0.6 },
    blink: false,
    mark: "z",
    tuft: { rest: 38, stiff: 0.5 },
    dim: true,
  },
  stuck: {
    eye: { ...DOT, w: 0.05, h: 1.8, dy: 0.35 },
    brow: { ...NO_BROW, weight: 0.85, lift: 1.6, tilt: -28, len: 1.15, bow: 0.1 },
    mouth: { ...NO_MOUTH, weight: 0.45, w: 0.9, curve: -0.35, wave: 0.3 },
    line: { ...CLEAN, boil: 0.3, rate: 4 },
    blink: "slow",
    jitter: 0.03,
    gaze: { range: 0.14, hz: 1.2, bias: [0, 0.08] },
    mark: "plewd",
    tuft: { rest: 24, stiff: 0.6 },
  },
  surprised: {
    eye: { ...DOT, h: 2.8, sep: 0.5 },
    brow: { ...NO_BROW, weight: 0.7, lift: 2.8, tilt: -4, bow: 0.35 },
    mouth: { ...NO_MOUTH, weight: 0.5, w: 0.9, o: 1 },
    line: { ...CLEAN, weight: 2.05 },
    blink: false,
    gaze: { range: 0.015, hz: 3 },
    mark: "shock",
    tuft: { rest: 0, stiff: 4 },
  },
};

function mixAll<T extends object>(a: T, b: T, u: number): T {
  const out = { ...a } as Record<string, number>;
  for (const k of Object.keys(a)) {
    out[k] = lerp((a as Record<string, number>)[k] ?? 0, (b as Record<string, number>)[k] ?? 0, u);
  }
  return out as T;
}

/**
 * Pose to pose, on twos. A change of mood is five drawings: the old pose, one
 * halfway, one past the new pose, one just short of it, and the new pose.
 */
const POSES = [0, 0.5, 1.14, 0.97, 1];

/* --- strokes --------------------------------------------------------------- */

/** A filled stroke along a centerline, with its own width at every point and round ends. */
function stroke(center: Point[], widths: number[]): string {
  const n = center.length;
  if (n < 2) return "";
  const left: Point[] = [];
  const right: Point[] = [];
  const tangent = (i: number): Point => {
    const a = center[Math.max(0, i - 1)] as Point;
    const b = center[Math.min(n - 1, i + 1)] as Point;
    const l = Math.hypot(b[0] - a[0], b[1] - a[1]) || 1;
    return [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
  };
  for (let i = 0; i < n; i++) {
    const [tx, ty] = tangent(i);
    const w = (widths[i] ?? 0) / 2;
    const p = center[i] as Point;
    left.push([p[0] - ty * w, p[1] + tx * w]);
    right.push([p[0] + ty * w, p[1] - tx * w]);
  }
  const cap = (i: number, forward: boolean): Point[] => {
    const [tx, ty] = tangent(i);
    const d = forward ? 1 : -1;
    const w = (widths[i] ?? 0) / 2;
    const p = center[i] as Point;
    const out: Point[] = [];
    for (let k = 1; k < 6; k++) {
      const phi = (k / 6) * Math.PI;
      const nx = forward ? -ty : ty;
      const ny = forward ? tx : -tx;
      out.push([
        p[0] + (nx * Math.cos(phi) + tx * d * Math.sin(phi)) * w,
        p[1] + (ny * Math.cos(phi) + ty * d * Math.sin(phi)) * w,
      ]);
    }
    return out;
  };
  return poly([...left, ...cap(n - 1, true), ...right.reverse(), ...cap(0, false)]);
}

/** A quadratic from a to b bowed by `bow` (positive bows toward -y), sampled. */
function arc(a: Point, b: Point, bow: number, n = 10): Point[] {
  const mx = (a[0] + b[0]) / 2;
  const my = (a[1] + b[1]) / 2;
  const lx = b[0] - a[0];
  const ly = b[1] - a[1];
  const l = Math.hypot(lx, ly) || 1;
  /* The normal that points up the screen when a is left of b. */
  const nx = ly / l;
  const ny = -lx / l;
  const cx = mx + nx * bow * 2;
  const cy = my + ny * bow * 2;
  const out: Point[] = [];
  for (let i = 0; i <= n; i++) {
    const s = i / n;
    const q = 1 - s;
    out.push([q * q * a[0] + 2 * q * s * cx + s * s * b[0], q * q * a[1] + 2 * q * s * cy + s * s * b[1]]);
  }
  return out;
}

/** The body's line: one brush stroke round the outline, lifted off where it began. */
function brush(pts: Point[], line: Line, step: number, seed: number, px: number): string {
  const ring = resample(pts, 4);
  const n = ring.length;
  let start = 0;
  let best = -Infinity;
  for (let i = 0; i < n; i++) {
    const [x, y] = ring[i] as Point;
    const score = -(x - C) - (y - C) * 1.4;
    if (score > best) {
      best = score;
      start = i;
    }
  }
  const phase = (k: number) => rnd(step * 3.1 + k * 1.7 + seed) * TAU;
  const p1 = phase(1);
  const p2 = phase(2);
  const p3 = phase(3);
  const p4 = phase(4);
  const total = Math.round(n * 1.05);
  const center: Point[] = [];
  const widths: number[] = [];
  const weight = line.weight * hint(px, 40, 1.7);
  for (let j = 0; j <= total; j++) {
    const i = (start + j) % n;
    const [x, y] = ring[i] as Point;
    const th = (i / n) * TAU;
    const s = j / total;
    const wander =
      line.boil * (0.5 * Math.sin(2 * th + p1) + 0.3 * Math.sin(3 * th + p2) + 0.2 * Math.sin(5 * th + p3)) +
      line.scratch * Math.sin(13 * th + p4) * Math.sin(7 * th + p1);
    const dx = x - C;
    const dy = y - C;
    const l = Math.hypot(dx, dy) || 1;
    center.push([x + (dx / l) * wander, y + (dy / l) * wander]);
    /* Heavier underneath, where a brush going round a shape presses. */
    const down = (1 + dy / l) / 2;
    const taper = Math.min(1, 0.4 + s / 0.06, (1 - s) / 0.12);
    const dryness = line.dry > 0 && Math.sin(s * TAU * 9 + seed * 5) > 1 - line.dry * 1.6 ? 0.08 : 1;
    const noise = 1 + line.boil * 0.35 * Math.sin(9 * th + p2);
    widths.push(Math.max(0, weight * (0.62 + 0.55 * down) * taper * dryness * noise));
  }
  return stroke(center, widths);
}

/* --- what is on its head ------------------------------------------------------ */

type Tuft = "curl" | "leaf" | "tick" | "none";
const TUFTS: Tuft[] = ["curl", "tick", "none", "leaf", "tick", "curl", "none"];

function tuftLines(kind: Tuft): Point[][] {
  switch (kind) {
    case "curl":
      return [[[0, 0], [0.3, -1.8], [0.25, -3.4], [-0.6, -4.4], [-1.6, -4.1], [-1.7, -3.2]]];
    case "tick":
      return [[[0, 0], [0.4, -2.4], [0.9, -4.2]]];
    case "leaf":
      return [
        [[0, 0], [0.1, -2.2]],
        [[0.1, -2.2], [1.6, -3.6], [3, -3.4], [1.7, -2.3], [0.1, -2.2]],
      ];
    default:
      return [];
  }
}

/* --- the creature -------------------------------------------------------------- */

export const ink: Design = {
  key: "ink",
  name: "C. Ink",
  create(svg, who, _color, seed) {
    const seedNum = rnd(seed.length * 3.7 + (seed.charCodeAt(0) || 0));
    const kind = TUFTS[Math.floor(rnd(who.key.length * 5.1 + (who.key.charCodeAt(1) || 0)) * TUFTS.length)] as Tuft;
    const fill = el("path", { fill: "var(--accent)", class: "ink__fill" }, svg);
    /* The edge and the face are two inks: on a dark page the edge has to be
       the light one or the silhouette is only the misregistered pigment. */
    const tuft = el("path", { class: "ink__edge" }, svg);
    const line = el("path", { class: "ink__edge" }, svg);
    const face = el("g", { class: "ink__line" }, svg);
    const eyes = [el("path", {}, face), el("path", {}, face)];
    const brows = [el("path", {}, face), el("path", {}, face)];
    const mouth = el("path", {}, face);
    const mouthO = el("ellipse", { fill: "none", class: "ink__stroke" }, face);
    const mark = el("g", { class: "ink__mark" }, svg);
    const bang = el("g", { class: "avatar__mark" }, svg);

    const look: Point = [0, 0];
    const vel: Point = [0, 0];
    let tuftAngle = 0;
    let tuftVel = 0;
    let drawn = "";
    let marked: Mood | null = null;

    return {
      paint(f) {
        const to = INKS[f.mood];
        const from = INKS[f.from];

        let target: Point;
        let jumpClosed = false;
        if (f.look) target = [0, f.look === "up" ? -AIM.up : AIM.down];
        else if (!f.live) target = [0, 0];
        else if (f.point) target = f.point;
        else {
          const g = saccade(f.t, { ...to.gaze, cross: 0.05 });
          target = g.at;
          /* A big look is hidden behind a blink, as an animator cuts a head turn. */
          if (g.size > 0.2 && g.jump < 2 / FPS) {
            jumpClosed = true;
          }
        }
        if (f.live) follow(look, vel, target, f.dt, 0.07);
        else {
          look[0] = target[0];
          look[1] = target[1];
        }

        /* The mark on its head lags the body, which is the one piece of
           secondary motion in the set. Simulated every frame, drawn on twos. */
        if (f.live) {
          const want = to.tuft.rest - look[0] * 60 + (f.gesture === "receive" && f.age < 0.3 ? 30 : 0);
          const k = 90 * to.tuft.stiff;
          const acc = k * (want - tuftAngle) - 2 * Math.sqrt(k) * 0.35 * tuftVel;
          tuftVel += acc * f.dt;
          tuftAngle += tuftVel * f.dt;
          if (f.mood === "frustrated") tuftAngle += Math.sin(f.t * 60) * 1.4;
        } else tuftAngle = to.tuft.rest;

        const frame = f.live ? Math.floor(f.t * FPS) : 0;
        const pose = f.live ? Math.floor(f.since * FPS) : 99;
        const key = `${frame}|${pose < POSES.length ? pose : "held"}|${f.mood}|${f.from}|${f.look}|${f.px}`;
        if (key === drawn) return;
        drawn = key;
        const tq = frame / FPS;

        const u = POSES[Math.min(pose, POSES.length - 1)] as number;
        const settled = u === 1 && pose >= POSES.length - 1;

        if (marked !== f.mood) {
          marked = f.mood;
          bang.innerHTML = to.mark === "bang" ? markFor("blocked") : "";
          svg.classList.toggle("is-dim", Boolean(to.dim));
        }

        /* Body. */
        const bodyLook: Point = [look[0], look[1]];
        if (f.gesture && f.live && f.age < 0.8) {
          const away = f.look === "down" ? -1 : 1;
          const wave = f.gesture === "send" ? Math.sin(f.age * 9.4) : Math.cos(f.age * 13.8);
          bodyLook[1] += away * 0.3 * wave * Math.exp(-f.age / 0.28);
        }
        const next = bodyPoints(who, MOODS[f.mood].shape, tq, bodyLook).pts;
        const pts = settled ? next : blend(bodyPoints(who, MOODS[f.from].shape, tq, bodyLook).pts, next, u);
        /* The pigment is printed a unit out of register with the line. */
        fill.setAttribute("d", poly(resample(pts, 3).map(([x, y]) => [x + 1.3, y + 1.1] as Point)));

        const ln = settled ? to.line : mixAll(from.line, to.line, clamp(u));
        const boilStep = ln.rate > 0 ? Math.floor(f.t * ln.rate) : 0;
        line.setAttribute("d", brush(pts, ln, boilStep, seedNum * 17, f.px));

        /* Tuft, from the crown. */
        let crown = 0;
        for (let i = 1; i < pts.length; i++) if ((pts[i] as Point)[1] < (pts[crown] as Point)[1]) crown = i;
        const [kx, ky] = pts[crown] as Point;
        const ang = (tuftAngle * Math.PI) / 180;
        const tw = ln.weight * 0.85 * hint(f.px, 40, 1.6);
        tuft.setAttribute(
          "d",
          tuftLines(kind)
            .map((seg) => {
              const placed = seg.map(([x, y]): Point => [
                kx + x * Math.cos(ang) - y * Math.sin(ang),
                ky + 0.6 + x * Math.sin(ang) + y * Math.cos(ang),
              ]);
              const dense = placed.length > 2 ? openCurve(placed) : placed;
              return stroke(dense, dense.map((_, i) => tw * (1 - (i / dense.length) * 0.55)));
            })
            .join(""),
        );

        /* Face. */
        let eye = settled ? to.eye : mixAll(from.eye, to.eye, u);
        let brow = settled ? to.brow : mixAll(from.brow, to.brow, u);
        const mo = settled ? to.mouth : mixAll(from.mouth, to.mouth, u);
        const up = clamp(-look[1] / 0.28);
        if (to.squint && up > 0) {
          brow = { ...brow, skew: brow.skew + to.squint.skew * up, turn: brow.turn + to.squint.turn * up };
          eye = { ...eye, lop: eye.lop + to.squint.lop * up };
        }
        const watch = u > 0.5 ? to : from;
        let closed = jumpClosed;
        if (f.live && watch.blink !== false && blinkAt(tq * (watch.blink === "slow" ? 0.5 : 0.85)) > 0.4) closed = true;
        if (f.look === "down") eye = { ...eye, w: eye.w + (1 - eye.w) * 0.45, h: eye.h * 0.6, c: eye.c + 0.2, dy: eye.dy + 0.4 };
        if (closed) eye = { ...eye, w: 1.2, h: 0.5, c: 0.1 };

        const r = who.eye.r;
        const one = Boolean(who.eye.one);
        const jit = f.live && to.jitter ? to.jitter : 0;
        const seen = Math.hypot(look[0], look[1]);
        const lead = seen > 0.004 ? grip(seen) / seen : 0;
        const ex = C + (who.eye.x ?? 0) + look[0] * R * (1 + 0.6 * lead) + Math.sin(tq * 41) * jit * r;
        const ey = C + (who.eye.y ?? 0) + look[1] * R + (eye.dy + Math.sin(tq * 33) * jit * 0.7) * r;
        const sep = (who.eye.spread + eye.sep * r) * (1 - Math.abs(look[0]) * 0.5);
        const scale = one ? 1.5 : 1;
        const weightHint = hint(f.px, 40, 1.5);

        for (let i = 0; i < 2; i++) {
          const side = one ? 0 : i === 0 ? -1 : 1;
          const path = eyes[i] as SVGPathElement;
          const bp = brows[i] as SVGPathElement;
          if (one && i === 1) {
            path.setAttribute("d", "");
            bp.setAttribute("d", "");
            continue;
          }
          const er = r * scale * (1 + side * look[0] * 0.3);
          const skewY = side > 0 ? -eye.skew * er : 0;
          const lop = eye.lop * (side > 0 ? 1 : side < 0 ? -1 : 0);
          const w = Math.max(0.02, eye.w + lop) * er;
          const h = Math.max(0.12, eye.h * (1 - lop * 0.45)) * er * weightHint;
          const x = ex + side * sep;
          const y = ey + skewY;
          if (w < h * 0.3) {
            path.setAttribute("d", circle(x, y, h / 2));
          } else {
            const tilt = ((eye.a * (side || 1) * Math.PI) / 180) * (side === 0 ? 0 : 1);
            const ax = Math.cos(tilt) * w;
            const ay = Math.sin(tilt) * w * -side;
            const center = arc([x - ax, y - ay], [x + ax, y + ay], -eye.c * er);
            path.setAttribute("d", stroke(center, center.map((_, k) => h * (0.72 + 0.28 * Math.sin((k / (center.length - 1)) * Math.PI)))));
          }

          if (brow.weight > 0.04) {
            const mine = side > 0 ? brow.skew : 0;
            const tiltDeg = brow.tilt + (side > 0 ? brow.turn : 0);
            /* A single eye has no inner end: its brow bends in the middle instead. */
            const t = side === 0 ? 0 : (tiltDeg * Math.PI) / 180;
            const bend = side === 0 ? -Math.sin((tiltDeg * Math.PI) / 180) * brow.len * er * 0.8 : 0;
            const bx = x;
            const by = ey - (brow.lift + mine) * er;
            const half = brow.len * er;
            /* Inner end first: that is where a brush presses. */
            const inner = side === 0 ? 1 : -side;
            const a: Point = [bx + inner * Math.cos(t) * half, by + Math.sin(t) * half];
            const b: Point = [bx - inner * Math.cos(t) * half, by - Math.sin(t) * half];
            const left = a[0] < b[0] ? a : b;
            const right = a[0] < b[0] ? b : a;
            const center = arc(left, right, brow.bow * er + bend);
            const innerFirst = a[0] < b[0];
            const bw = brow.weight * er * 0.9 * weightHint;
            bp.setAttribute(
              "d",
              stroke(
                center,
                center.map((_, k) => {
                  const s = k / (center.length - 1);
                  const fromInner = innerFirst ? s : 1 - s;
                  return bw * (1 - 0.55 * fromInner);
                }),
              ),
            );
          } else bp.setAttribute("d", "");
        }

        if (mo.weight > 0.05 && f.px >= 40 && !closed) {
          const my = ey + 2.5 * r + (one ? 1.2 * r : 0);
          const mx = ex + mo.x * r;
          const mw = mo.w * r;
          const mweight = mo.weight * r * 0.95;
          if (mo.o > 0.5) {
            mouth.setAttribute("d", "");
            mouthO.setAttribute("cx", mx.toFixed(2));
            mouthO.setAttribute("cy", (my + 0.4).toFixed(2));
            mouthO.setAttribute("rx", (mw * 0.55).toFixed(2));
            mouthO.setAttribute("ry", (mw * 0.72).toFixed(2));
            mouthO.setAttribute("stroke-width", mweight.toFixed(2));
          } else {
            mouthO.setAttribute("rx", "0");
            mouthO.setAttribute("ry", "0");
            const center: Point[] = [];
            for (let k = 0; k <= 12; k++) {
              const s = k / 12;
              const lx = -mw + 2 * mw * s;
              const bow = 1 - (2 * s - 1) ** 2;
              center.push([mx + lx, my + mo.curve * bow * r * 0.55 + mo.wave * Math.sin(s * TAU * 1.5) * r * 0.3]);
            }
            mouth.setAttribute("d", stroke(center, center.map(() => mweight)));
          }
        } else {
          mouth.setAttribute("d", "");
          mouthO.setAttribute("rx", "0");
          mouthO.setAttribute("ry", "0");
        }

        mark.innerHTML = emanata(to.mark, f, frame, pts, seedNum);
      },
    };
  },
};

function circle(x: number, y: number, r: number): string {
  return `M${(x - r).toFixed(2)} ${y.toFixed(2)}a${r.toFixed(2)} ${r.toFixed(2)} 0 1 0 ${(2 * r).toFixed(2)} 0a${r.toFixed(2)} ${r.toFixed(2)} 0 1 0 ${(-2 * r).toFixed(2)} 0Z`;
}

/** Catmull-Rom through an open run of points. */
function openCurve(pts: Point[]): Point[] {
  const out: Point[] = [];
  const n = pts.length;
  for (let i = 0; i < n - 1; i++) {
    const p0 = pts[Math.max(0, i - 1)] as Point;
    const p1 = pts[i] as Point;
    const p2 = pts[i + 1] as Point;
    const p3 = pts[Math.min(n - 1, i + 2)] as Point;
    for (let j = 0; j < 4; j++) {
      const s = j / 4;
      const s2 = s * s;
      const s3 = s2 * s;
      const f = (a: number, b: number, c: number, d: number) =>
        0.5 * (2 * b + (-a + c) * s + (2 * a - 5 * b + 4 * c - d) * s2 + (-a + 3 * b - 3 * c + d) * s3);
      out.push([f(p0[0], p1[0], p2[0], p3[0]), f(p0[1], p1[1], p2[1], p3[1])]);
    }
  }
  out.push(pts[n - 1] as Point);
  return out;
}

/**
 * The marks beside a head, drawn on twos like everything else. Comics has a
 * word for these (emanata) and a vocabulary that reads at any size: a scribble
 * is cross, a drop of sweat is worry, lines round a head are a shock.
 */
function emanata(kind: Emanata | undefined, f: { since: number; live: boolean; px: number }, frame: number, pts: Point[], seed: number): string {
  const w = (1.4 * hint(f.px, 40, 1.6)).toFixed(2);
  const ink = `stroke="var(--text)" stroke-width="${w}" fill="none" stroke-linecap="round" stroke-linejoin="round"`;
  switch (kind) {
    case "dots": {
      const shown = f.live ? Math.floor(frame / 4) % 4 : 3;
      const dots: [number, number, number][] = [
        [48, 12, 1.5],
        [52.6, 9.4, 1.8],
        [57.4, 6.2, 2.1],
      ];
      return dots
        .slice(0, shown)
        .map(([x, y, r]) => `<circle cx="${x}" cy="${y}" r="${r}" fill="var(--text)" opacity="0.75"/>`)
        .join("");
    }
    case "z": {
      const k = f.live ? Math.floor(frame / 3) % 6 : 2;
      const x = 45 + k * 0.7;
      const y = 15 - k * 1.3;
      const s = 0.75 + k * 0.07;
      return `<path d="M${(x - 3.2 * s).toFixed(2)} ${(y - 3.2 * s).toFixed(2)}h${(6.4 * s).toFixed(2)}l${(-6.4 * s).toFixed(2)} ${(6.4 * s).toFixed(2)}h${(6.4 * s).toFixed(2)}" ${ink} opacity="${(0.75 - k * 0.1).toFixed(2)}"/>`;
    }
    case "plewd": {
      /* Up and to the left of the head, which is where a drop of sweat is drawn. */
      let left = 0;
      let most = -Infinity;
      for (let i = 0; i < pts.length; i++) {
        const [x, y] = pts[i] as Point;
        const score = -(x - C) * 0.8 - (y - C) * 0.6;
        if (score > most) {
          most = score;
          left = i;
        }
      }
      const [px, py] = pts[left] as Point;
      const drop = f.live ? (Math.floor(frame / 3) % 5) * 0.5 : 0;
      const x = px - 2.2;
      const y = py - 1 + drop;
      return `<path d="M${x.toFixed(2)} ${(y - 3.2).toFixed(2)}C${(x + 1.9).toFixed(2)} ${(y - 0.4).toFixed(2)} ${(x + 1.9).toFixed(2)} ${(y + 1.6).toFixed(2)} ${x.toFixed(2)} ${(y + 1.6).toFixed(2)}S${(x - 1.9).toFixed(2)} ${(y - 0.4).toFixed(2)} ${x.toFixed(2)} ${(y - 3.2).toFixed(2)}Z" stroke="var(--text)" stroke-width="${w}" fill="var(--ground)" stroke-linejoin="round"/>`;
    }
    case "scribble": {
      const loop: Point[] = [];
      const step = f.live ? frame : 0;
      for (let i = 0; i < 9; i++) {
        const a = i * 2.3 + rnd(step * 1.3 + i + seed) * 1.2;
        const rr = 2.2 + rnd(step * 0.7 + i * 3.1 + seed) * 2.2;
        loop.push([52 + Math.cos(a) * rr, 10 + Math.sin(a) * rr * 0.75]);
      }
      return `<path d="${poly(openCurve(loop), false)}" ${ink}/>`;
    }
    case "shock": {
      if (f.live && f.since > 0.75) return "";
      const k = f.live ? Math.min(2, Math.floor(f.since * FPS)) : 1;
      const len = 2 + k * 0.9;
      return [-0.62, -0.5, -0.38]
        .map((turn) => {
          const a = turn * TAU;
          const r0 = 23 + k * 0.4;
          const x0 = C + Math.cos(a) * r0;
          const y0 = C + 2 + Math.sin(a) * r0;
          return `<path d="M${x0.toFixed(2)} ${y0.toFixed(2)}l${(Math.cos(a) * len).toFixed(2)} ${(Math.sin(a) * len).toFixed(2)}" ${ink}/>`;
        })
        .join("");
    }
    case "sparkle": {
      if (f.live && f.since > 1.8) return "";
      const k = f.live ? Math.floor(f.since * FPS) % 4 : 1;
      const s = [1.9, 3, 2.4, 3.2][k] as number;
      const x = 52;
      const y = 11;
      return `<path d="M${x} ${y - s}Q${x} ${y} ${x + s} ${y}Q${x} ${y} ${x} ${y + s}Q${x} ${y} ${x - s} ${y}Q${x} ${y} ${x} ${y - s}Z" fill="var(--text)"/>`;
    }
    default:
      return "";
  }
}
