/**
 * The eyes, which are where everything an agent has to say is said.
 *
 * An eye is an eyeball, a pupil and two lids, and the lids are drawn as the
 * absence of eye: what is on screen is the white between the two lid lines,
 * cut to the ball, with the pupil inside it and a line along the upper lid.
 * The body is already there to be the lid. The pupil takes the glance, so a
 * look to one side costs the face almost nothing, and the lids take the
 * emotion, which is how an animator divides a face. Every number on a lid lerps
 * like every other, so a face can sit halfway between two moods and a mood
 * change is an interpolation rather than a cut.
 *
 * On top of the shape sits behavior, and it is the behavior that reads as alive:
 * a blink that is sometimes a double and is often led into a large saccade, a
 * gaze that flicks somewhere and holds, and an upper lid that rides the pupil
 * down. Eyes do not slide. Sliding one around on a sine is the single thing
 * that makes a face read as a screensaver; holding still between jumps is what
 * makes it read as attention.
 *
 * Nothing here draws. `eyesAt` hands back points, and the drawn style reads
 * the gaze from this file and none of the lids.
 */

import { FORM, grip, type Lump, type Point, PULL } from "./form";

/**
 * One eye's lids, in eye radii unless said otherwise. The same numbers on both
 * eyes, mirrored, except `skew`.
 */
export interface Eye {
  /** Upper lid: 0 clear of the eye, 1 shut onto the lower lid, negative retracted. */
  open: number;
  /** Degrees. Positive drops the inner end, which is cross; negative lifts it, which is worry. */
  tilt: number;
  /** How far the upper lid arches in its middle. */
  arch: number;
  /** Lower lid: 0 at rest, 1 up to where a shut eye meets. */
  low: number;
  /** How far the lower lid bows up in its middle. Cheeks pushing: a smile. */
  smile: number;
  /** Pupil radius. Small is shock, large is interest. */
  pupil: number;
  /** The eyeball's own scale. */
  size: number;
  /**
   * The upper lid of the eye on the viewer's right lower than the other, in
   * `open`. The pair is otherwise a mirror, and a mirror can be calm, cross or
   * afraid but never doubtful: a cocked brow is the one expression that needs
   * the two to disagree.
   */
  skew: number;
}

/** Where a mood looks, and how it holds itself while it looks. */
export interface Watch {
  /** False never blinks, "slow" blinks at about half the rate. */
  blink?: boolean | "slow";
  gaze?: Gaze;
  /** A shiver on the eyes only, in eye radii. */
  jitter?: number;
  /**
   * How the lids change as the look goes up, blended in by how far up it has
   * got. A mood that looks at something and minds what it sees needs no second
   * face.
   */
  squint?: { at: number } & Partial<Eye>;
}

/** Where a creature looks. Either rolled, or written down. */
export interface Gaze {
  /** How far, in body radii. */
  range?: number;
  /** How often a new target is chosen. */
  hz?: number;
  /** How long the move takes, in seconds. Independent of how often. */
  cross?: number;
  /** A standing offset, so a mood can look mostly upward. */
  bias?: Point;
  /** `[x, y, hold]` steps, cycled. For a mood that is looking at something. */
  script?: [number, number, number][];
  /**
   * What share of the saccades go the whole way to one side. With it set, the
   * rest stay inside the middle of the range, so a creature mostly glances and
   * now and then takes a look; without it every target is anywhere in the box,
   * which is a creature that never quite commits to looking at anything.
   */
  far?: number;
  /** How much of the range the ordinary glances use, when `far` is set. 0.6 by default. */
  near?: number;
}

/**
 * How long a change of where to look takes to be taken, in seconds, by the
 * eyes and the body together.
 *
 * This used to be the body's lag behind the eyes, at almost half a second, on
 * the argument that a bulge read late reads as a consequence of the look. With
 * the body pulled into a pear it read as the opposite: the eyes went, and then
 * something else happened to the body. So there is one gaze, both read it at
 * the same instant, and this is only the smoothing in front of both, short
 * enough that a flick is still a flick and long enough that an aimed look, which
 * arrives as a step, is not a cut. Spent in `AgentAvatar` through `settle`.
 */
export const SETTLE = 0.12;

const OMEGA = 2.2 / SETTLE;

/** Where the look has got to, and how fast, in body radii. */
export interface Mass {
  gaze: Point;
  vel: Point;
}

/**
 * One step of the look toward where it was asked to go, in place.
 *
 * Critically damped, so it never passes the target: the eyes read this too,
 * and an eye that overshoots reads as a wobble. A look, a peer being addressed
 * and a message landing all arrive through this one filter, so none of them
 * can snap. `dt` is seconds since the last step, already bounded by the caller:
 * a tab that was hidden for a minute is one big step, not a minute of them.
 */
export function settle(mass: Mass, target: Point, dt: number): void {
  for (const i of [0, 1] as const) {
    const x = mass.gaze[i];
    const v = mass.vel[i];
    const accel = OMEGA * OMEGA * (target[i] - x) - 2 * OMEGA * v;
    mass.vel[i] = v + accel * dt;
    mass.gaze[i] = x + mass.vel[i] * dt;
  }
}

/**
 * Where an aimed look points, in body radii.
 *
 * It is spent as a gaze and nothing else. The stroke this replaced needed a
 * second molding on top of the look to read as looking anywhere, because two
 * marks sliding down a face do not; a pupil going down under a lid that comes
 * down with it does, and that is true of every look, so the aimed one needs
 * nothing of its own.
 *
 * The asymmetry is measured, not chosen. Every one of these bodies hangs its
 * mass below its eyes, so there is depth under them and very little over them,
 * and the widest eyes on the table, `surprised`, have to fit while looking up
 * at whoever just threw something at them. `form.test.ts` is the gate.
 */
export const AIM = { down: 0.3, up: 0.25 } as const;

/**
 * How far the eyes themselves travel across the face for a look, in body radii
 * per body radius of look. The pupil takes the rest of it. Across, a look the
 * body answers carries the eyes further again, into the snout the look pulled
 * out (`PULL.lead`); up and down, the lid is what says it.
 */
const TRAVEL = { across: 0.42, down: 0.3 } as const;

/**
 * How much of a sideways look the two eyes disagree in size, per body radius of
 * look. Perspective on a turned head: the near eye grows by this and the far
 * one shrinks by it.
 */
const PEEK = 0.25;

/** How far a look takes the pupil to the edge of the white, in body radii. */
const PUPIL_REACH = 0.42;

/**
 * How far past the ball the lid line is drawn, as a share of its radius. The
 * line is clipped to this circle, so a shut eye is a line as wide as the eye
 * and a lid half down does not draw on the body beside it.
 */
export const LASH_REACH = 1.14;

/** How deep the lid's shadow on the ball is, in eye radii. */
const SHADE = 0.22;

/** How long a blink takes, in seconds. */
const BLINK = 0.18;

/** Deterministic noise. The same creature blinks the same way every reload. */
function rnd(i: number): number {
  const x = Math.sin(i * 127.1 + 197.3) * 43758.5453;
  return x - Math.floor(x);
}

function clamp(x: number, lo = 0, hi = 1): number {
  return Math.max(lo, Math.min(hi, x));
}

/** 0..1..0 over `dur` seconds from 0. */
function bump(x: number, dur: number): number {
  return x >= 0 && x < dur ? Math.sin((x / dur) * Math.PI) : 0;
}

/** 0 open to 1 shut, in slots of about four seconds, one in four doubled. */
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

/**
 * A blink the caller has a reason for, `age` seconds after the reason. An
 * aimed look starting or ending is one: a face blinks as it turns to somebody.
 */
export function cueBlink(age: number): number {
  return bump(age, BLINK);
}

/** Quintic. It leaves and arrives with no corner on it. */
function ease(u: number): number {
  const e = clamp(u);
  return e * e * e * (e * (e * 6 - 15) + 10);
}

/**
 * How long one jump takes, from how far it goes. `cross` is the time of an
 * ordinary glance; a look the whole way to one side takes up to twice that,
 * because the body comes with it now and a body that becomes a pear in a
 * quarter of a second is a body that snapped.
 */
function span(from: Point, to: Point, cross: number): number {
  const far = Math.hypot(to[0] - from[0], to[1] - from[1]);
  return cross * Math.max(1, Math.min(2, far / 0.25));
}

/** One moment of a gaze: where it is, and which jump it is part of. */
export interface Glance {
  at: Point;
  /** Seconds since the current jump began. */
  jump: number;
  /** How far the current jump goes, in body radii. */
  size: number;
  /** Which jump this is, for anything that rolls against it. */
  slot: number;
}

/** A gaze that was written down: `[x, y, hold]` steps, cycled. */
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

/**
 * A saccade, and which one. `hz` is how often it happens and `cross` is how
 * long the move takes, which are two separate decisions: a creature that looks
 * around rarely does not also move its eyes slowly. Tying them together made
 * every mood feel hurried.
 */
export function glance(t: number, g?: Gaze): Glance {
  if (!g) return { at: [0, 0], jump: Number.POSITIVE_INFINITY, size: 0, slot: 0 };
  if (g.script) return scripted(t, g);
  const range = g.range ?? 0;
  const P = 1 / (g.hz ?? 0.5);
  /* Each slot's jump lands somewhere in the first half of it rather than on
     its beat. On the beat, a creature holds every look for exactly as long as
     the last, and a hold that never varies reads as a metronome: the same
     thing a slide reads as, arrived at from the other side. */
  const starts = (k: number) => k * P + rnd(k + 0.83) * P * 0.5;
  let i = Math.floor(t / P);
  if (t < starts(i)) i -= 1;
  const at = (k: number): Point => {
    const bx = g.bias ? g.bias[0] : 0;
    const by = g.bias ? g.bias[1] : 0;
    if (g.far && rnd(k + 0.61) < g.far) {
      /* The whole way to one side, and level: a look, rather than a glance. */
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

/** Where a gaze is, in body radii. */
export function gazeAt(t: number, g?: Gaze): Point {
  return glance(t, g).at;
}

/**
 * The blink a face leads a large saccade with. Lids come down a few frames
 * before the eye moves and are up again as it lands, two jumps in three: a
 * blink on a timer ignores what the eyes are doing, and this is the half of
 * blinking that does not.
 */
export function saccadeBlink(t: number, g?: Gaze): number {
  const now = glance(t, g);
  if (now.size <= 0.2 || rnd(now.slot + 0.19) >= 0.7) return 0;
  return bump(now.jump + 0.05, BLINK * 0.95);
}

/** What a frame of eyes is drawn from, beyond the lids themselves. */
export interface Moment {
  /** The creature's own seconds. */
  t: number;
  /** False is reduced motion: no blink, no shiver. */
  live: boolean;
  /** Where it is looking, smoothed, in body radii. */
  gaze: Point;
  /** A blink the caller has a reason for, 0 open to 1 shut. */
  cue?: number;
  /**
   * How much heavier to draw the lid line. A creature drawn at 24px needs a
   * line nearly twice the weight of one at 44px for a shut eye to be there
   * at all.
   */
  weight?: number;
}

/** One eye, resolved to points. Everything is in viewBox units. */
export interface Drawn {
  /** The ball: its center and radius. The lid line is clipped to `r * LASH_REACH`. */
  x: number;
  y: number;
  r: number;
  /** The white between the lids, as a closed polygon. Empty of area when shut. */
  opening: Point[];
  pupil: { x: number; y: number; r: number };
  /** The lid's shadow on the ball, drawn inside the opening. */
  shade: Point[];
  /** The upper lid's line, and the lower lid's. */
  lash: Point[];
  lashWidth: number;
  under: Point[];
  underWidth: number;
  underOpacity: number;
}

/** Across the eye, how many places the lid lines are taken. */
const SAMPLES = 25;

/**
 * One eye. `side` is -1 for the eye on the viewer's left, 1 for the right and
 * 0 for a single eye. The inner end of a lid is the one toward the other eye,
 * which is what a tilt drops; a single eye has no inner end, so its middle is.
 */
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
  /* The upper lid closes onto the lower one, wherever that is, so a shut eye
     is one line and never a line with white under it. */
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

  /* The pupil, held inside the white by the ball it is drawn in. */
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

/** Both eyes, this instant. `at.gaze` is in body radii. */
export function eyesAt(lump: Lump, eye: Eye, watch: Watch | undefined, at: Moment): Drawn[] {
  const anim = watch ?? {};
  const [gx, gy] = at.gaze;
  const E = lump.eye.r;

  /* The lids ride the look. Down drops the upper lid after the pupil, which is
     what a look at somebody below is; up lifts it a little and brings the
     lower lid up after it. A mood that minds what it looks up at squints. */
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

  /* Across, the eyes go further as the body answers, so they lead the snout
     the look pulls out rather than sit in the middle of it. */
  const seen = Math.hypot(gx, gy);
  const lead = seen > 0.004 ? (PULL.lead * grip(seen)) / seen : 0;
  const x = FORM.center + (lump.eye.x ?? 0) + gx * FORM.radius * (TRAVEL.across + lead) + jx;
  const y = FORM.center + (lump.eye.y ?? 0) + gy * FORM.radius * TRAVEL.down + jy;
  const weight = at.weight ?? 1;

  const one = (side: number): Drawn => {
    const own = { ...lids, open: lids.open + (side > 0 ? lids.skew : 0) };
    own.open += (1 - own.open) * blink;
    own.low += Math.max(0, 0.4 - own.low) * blink;
    /* The far eye is smaller. A hard look to one side turns the head, and on
       a turned head the eye that went round the curve is foreshortened. */
    const r = E * own.size * (1 + side * gx * PEEK);
    const sep = lump.eye.spread * (1 - Math.abs(gx) * 0.35);
    return lidded(x + side * sep, y, r, side, own, at.gaze, weight);
  };
  return lump.eye.one ? [one(0)] : [one(-1), one(1)];
}

/** A run of points as path data. */
export function pathOf(pts: Point[], closed: boolean): string {
  if (pts.length === 0) return "";
  let d = "";
  for (let i = 0; i < pts.length; i++) {
    const p = pts[i] as Point;
    d += `${i === 0 ? "M" : "L"}${p[0].toFixed(2)} ${p[1].toFixed(2)}`;
  }
  return closed ? `${d}Z` : d;
}

/** Lids lerp exactly as the body does. Nothing switches. */
export function blendEyes(a: Eye, b: Eye, u: number): Eye {
  const out = { ...a };
  for (const key of Object.keys(a) as (keyof Eye)[]) out[key] = a[key] + (b[key] - a[key]) * u;
  return out;
}
