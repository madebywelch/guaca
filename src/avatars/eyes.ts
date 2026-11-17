import { FORM, grip, type Lump, type Point, PULL } from "./form";

export interface Eye {
  open: number;
  tilt: number;
  arch: number;
  low: number;
  smile: number;
  pupil: number;
  size: number;
  skew: number;
}

export interface Watch {
  blink?: boolean | "slow";
  gaze?: Gaze;
  jitter?: number;
  squint?: { at: number } & Partial<Eye>;
}

export interface Gaze {
  range?: number;
  hz?: number;
  cross?: number;
  bias?: Point;
  script?: [number, number, number][];
  far?: number;
  near?: number;
}

export const SETTLE = 0.12;

const OMEGA = 2.2 / SETTLE;

export interface Mass {
  gaze: Point;
  vel: Point;
}

type GazeStep = { kind: "hold" } | { kind: "advance"; dt: number };

function advance(mass: Mass, target: Point, step: GazeStep): void {
  if (step.kind === "hold") return;
  const decay = Math.exp(-OMEGA * step.dt);
  for (const i of [0, 1] as const) {
    const offset = mass.gaze[i] - target[i];
    const impulse = mass.vel[i] + OMEGA * offset;
    mass.gaze[i] = target[i] + (offset + impulse * step.dt) * decay;
    mass.vel[i] = (mass.vel[i] - OMEGA * impulse * step.dt) * decay;
  }
}

export function settle(mass: Mass, target: Point, dt: number): void {
  advance(mass, target, dt <= 0 ? { kind: "hold" } : { kind: "advance", dt });
}

export const AIM = { down: 0.3, up: 0.25 } as const;

const TRAVEL = { across: 0.42, down: 0.3 } as const;

const PEEK = 0.25;

const PUPIL_REACH = 0.42;

export const LASH_REACH = 1.14;

const SHADE = 0.22;

const BLINK = 0.18;

function rnd(i: number): number {
  const x = Math.sin(i * 127.1 + 197.3) * 43758.5453;
  return x - Math.floor(x);
}

function clamp(x: number, lo = 0, hi = 1): number {
  return Math.max(lo, Math.min(hi, x));
}

function bump(x: number, dur: number): number {
  return x >= 0 && x < dur ? Math.sin((x / dur) * Math.PI) : 0;
}

export function blinkAmount(t: number): number {
  const P = 4.6;
  let out = 0;
  for (let k = -1; k <= 0; k++) {
    const i = Math.floor(t / P) + k;
    const at = i * P + rnd(i) * (P - 0.8);
    out = Math.max(out, bump(t - at, BLINK));
    if (rnd(i + 0.5) > 0.7) out = Math.max(out, bump(t - at - 0.25, BLINK));
  }
  return out;
}

export function cueBlink(age: number): number {
  return bump(age, BLINK);
}

function ease(u: number): number {
  const e = clamp(u);
  return e * e * e * (e * (e * 6 - 15) + 10);
}

function span(from: Point, to: Point, cross: number): number {
  const far = Math.hypot(to[0] - from[0], to[1] - from[1]);
  return cross * Math.max(1, Math.min(2, far / 0.25));
}

export interface Glance {
  at: Point;
  jump: number;
  size: number;
  slot: number;
}

function scripted(t: number, g: Gaze): Glance {
  const steps = g.script as [number, number, number][];
  const total = steps.reduce((sum, step) => sum + step[2], 0);
  const lap = Math.floor(t / total);
  let u = t - lap * total;
  let i = 0;
  while (u >= (steps[i] as [number, number, number])[2]) {
    u -= (steps[i] as [number, number, number])[2];
    i = (i + 1) % steps.length;
  }
  const a = steps[(i - 1 + steps.length) % steps.length] as [number, number, number];
  const b = steps[i] as [number, number, number];
  const from: Point = [a[0], a[1]];
  const to: Point = [b[0], b[1]];
  const k = ease(u / span(from, to, g.cross ?? 0.26));
  return {
    at: [from[0] + (to[0] - from[0]) * k, from[1] + (to[1] - from[1]) * k],
    jump: u,
    size: Math.hypot(to[0] - from[0], to[1] - from[1]),
    slot: lap * steps.length + i,
  };
}

export function glance(t: number, g?: Gaze): Glance {
  if (!g) return { at: [0, 0], jump: Number.POSITIVE_INFINITY, size: 0, slot: 0 };
  if (g.script) return scripted(t, g);
  const range = g.range ?? 0;
  const P = 1 / (g.hz ?? 0.5);
  const starts = (k: number) => k * P + rnd(k + 0.83) * P * 0.5;
  let i = Math.floor(t / P);
  if (t < starts(i)) i -= 1;
  const at = (k: number): Point => {
    const bx = g.bias ? g.bias[0] : 0;
    const by = g.bias ? g.bias[1] : 0;
    if (g.far && rnd(k + 0.61) < g.far) {
      return [(rnd(k) < 0.5 ? -1 : 1) * range + bx, (rnd(k + 0.37) * 2 - 1) * range * 0.2 + by];
    }
    const within = g.far ? (g.near ?? 0.6) : 1;
    return [
      (rnd(k) * 2 - 1) * range * within + bx,
      (rnd(k + 0.37) * 2 - 1) * range * 0.62 * within + by,
    ];
  };
  const from = at(i - 1);
  const to = at(i);
  const jump = t - starts(i);
  const k = ease(jump / span(from, to, g.cross ?? 0.26));
  return {
    at: [from[0] + (to[0] - from[0]) * k, from[1] + (to[1] - from[1]) * k],
    jump,
    size: Math.hypot(to[0] - from[0], to[1] - from[1]),
    slot: i,
  };
}

export function gazeAt(t: number, g?: Gaze): Point {
  return glance(t, g).at;
}

export function saccadeBlink(t: number, g?: Gaze): number {
  const now = glance(t, g);
  if (now.size <= 0.2 || rnd(now.slot + 0.19) >= 0.7) return 0;
  return bump(now.jump + 0.05, BLINK * 0.95);
}

export interface Moment {
  t: number;
  live: boolean;
  gaze: Point;
  cue?: number;
  weight?: number;
}

export interface Drawn {
  x: number;
  y: number;
  r: number;
  opening: Point[];
  pupil: { x: number; y: number; r: number };
  shade: Point[];
  lash: Point[];
  lashWidth: number;
  under: Point[];
  underWidth: number;
  underOpacity: number;
}

const SAMPLES = 25;

function lidded(
  x: number,
  y: number,
  r: number,
  side: number,
  eye: Eye,
  look: Point,
  weight: number,
): Drawn {
  const shut = 0.22 * r;
  const L = r * 1.3;
  const slope = Math.tan((eye.tilt * Math.PI) / 180);
  const across = (lx: number) => (side === 0 ? L * 0.5 - Math.abs(lx) : -side * lx);
  const bow = (lx: number) => 1 - (lx / L) ** 2;
  const lowerAt = (lx: number) =>
    r * 1.02 - (r * 1.02 - shut) * eye.low + slope * 0.25 * across(lx) - eye.smile * r * bow(lx);
  const upperAt = (lx: number) =>
    Math.min(
      -r * 1.04 -
        eye.arch * r * bow(lx) +
        (lowerAt(lx) + r * 1.04 + eye.arch * r * bow(lx)) * eye.open +
        slope * across(lx),
      lowerAt(lx),
    );

  const top: Point[] = [];
  const bottom: Point[] = [];
  const lash: Point[] = [];
  const under: Point[] = [];
  for (let i = 0; i < SAMPLES; i++) {
    const s = i / (SAMPLES - 1);
    const ex = -r + 2 * r * s;
    const h = r * Math.sqrt(Math.max(0, 1 - (ex / r) ** 2));
    let t = Math.max(-h, upperAt(ex));
    let d = Math.min(h, lowerAt(ex));
    if (t > d) {
      const mid = clamp((t + d) / 2, -h, h);
      t = mid;
      d = mid;
    }
    top.push([x + ex, y + t]);
    bottom.push([x + ex, y + d]);
    const lx = -L + 2 * L * s;
    lash.push([x + lx, y + upperAt(lx)]);
    under.push([x + lx, y + lowerAt(lx)]);
  }

  const P = eye.pupil * (r / eye.size);
  const room = Math.max(0.01, r - P * 0.8);
  let ox = (look[0] / PUPIL_REACH) * room;
  let oy = (look[1] / PUPIL_REACH) * room;
  const out = Math.hypot(ox, oy) / room;
  if (out > 1) {
    ox /= out;
    oy /= out;
  }

  const closed = clamp((eye.open - 0.55) / 0.45);
  const lashWidth = 0.6 * weight * (1 + 0.9 * closed);
  const smiling = clamp((eye.low - 0.4) * 2);
  return {
    x,
    y,
    r,
    opening: [...top, ...bottom.reverse()],
    pupil: { x: x + ox, y: y + oy, r: P },
    shade: [
      ...lash.map(([a, b]): Point => [a, b - r]),
      ...lash.map(([a, b]): Point => [a, b + r * SHADE]).reverse(),
    ],
    lash,
    lashWidth,
    under,
    underWidth: lashWidth * (0.5 + 0.5 * smiling),
    underOpacity: clamp((eye.low - 0.35) * 2),
  };
}

export function eyesAt(lump: Lump, eye: Eye, watch: Watch | undefined, at: Moment): Drawn[] {
  const anim = watch ?? {};
  const [gx, gy] = at.gaze;
  const E = lump.eye.r;

  const up = clamp(-gy / 0.28);
  const down = clamp(gy / 0.28);
  let lids = { ...eye };
  if (anim.squint) {
    const s = anim.squint;
    const k = clamp(-gy / s.at);
    for (const key of ["open", "tilt", "arch", "low", "smile", "pupil", "size", "skew"] as const) {
      lids[key] += k * (s[key] ?? 0);
    }
  }
  lids = { ...lids, open: lids.open + down * 0.32 - up * 0.1, low: lids.low + up * 0.06 };

  let blink = at.cue ?? 0;
  if (at.live && anim.blink !== false) {
    blink = Math.max(blink, blinkAmount(at.t * (anim.blink === "slow" ? 0.5 : 0.85)));
  }
  if (!at.live) blink = 0;

  const jitter = at.live && anim.jitter ? anim.jitter * E : 0;
  const jx = jitter ? Math.sin(at.t * 41) * jitter : 0;
  const jy = jitter ? Math.sin(at.t * 33 + 1.7) * jitter * 0.7 : 0;

  const seen = Math.hypot(gx, gy);
  const lead = seen > 0.004 ? (PULL.lead * grip(seen)) / seen : 0;
  const x = FORM.center + (lump.eye.x ?? 0) + gx * FORM.radius * (TRAVEL.across + lead) + jx;
  const y = FORM.center + (lump.eye.y ?? 0) + gy * FORM.radius * TRAVEL.down + jy;
  const weight = at.weight ?? 1;

  const one = (side: number): Drawn => {
    const own = { ...lids, open: lids.open + (side > 0 ? lids.skew : 0) };
    own.open += (1 - own.open) * blink;
    own.low += Math.max(0, 0.4 - own.low) * blink;
    const r = E * own.size * (1 + side * gx * PEEK);
    const sep = lump.eye.spread * (1 - Math.abs(gx) * 0.35);
    return lidded(x + side * sep, y, r, side, own, at.gaze, weight);
  };
  return lump.eye.one ? [one(0)] : [one(-1), one(1)];
}

export function pathOf(pts: Point[], closed: boolean): string {
  if (pts.length === 0) return "";
  let d = "";
  for (let i = 0; i < pts.length; i++) {
    const p = pts[i] as Point;
    d += `${i === 0 ? "M" : "L"}${p[0].toFixed(2)} ${p[1].toFixed(2)}`;
  }
  return closed ? `${d}Z` : d;
}

export function blendEyes(a: Eye, b: Eye, u: number): Eye {
  const out = { ...a };
  for (const key of Object.keys(a) as (keyof Eye)[]) out[key] = a[key] + (b[key] - a[key]) * u;
  return out;
}
