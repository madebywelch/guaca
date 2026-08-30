/**
 * The eyes, which are where everything an agent has to say is said.
 *
 * An eye is one stroke with a round cap and four numbers on it: `w` half its
 * length, `h` its weight, `c` how far it bows and `a` how far it tilts, all in
 * eye radii. A dot is that stroke with no length. A blink is the same dot
 * moulded into a line. Upset is the line tilted in. Nothing is ever swapped for
 * anything, which is what lets a face sit halfway between two moods and lets a
 * mood change be an interpolation rather than a cut.
 *
 * On top of the shape sits behavior, and it is the behavior that reads as alive:
 * a blink that is sometimes a double, and a gaze that flicks somewhere and
 * holds. Eyes do not slide. Sliding one around on a sine is the single thing
 * that makes a face read as a screensaver; holding still between jumps is what
 * makes it read as attention.
 */

import { FORM, type Lump, type Point } from "./form";

const TAU = Math.PI * 2;

/** The shape of an eye, in eye radii. */
export interface Eye {
  /** Half the length of the stroke. 0 is a dot. */
  w: number;
  /** Weight of the stroke, so `h` of 2 with `w` of 0 is a circle. */
  h: number;
  /** Bow. Negative curves up, which is the only smile there is. */
  c?: number;
  /** Degrees, mirrored between the two. Positive drops the inner ends. */
  a?: number;
  /** Widens or narrows the pair, in eye radii. */
  sep?: number;
  /** Fixed offset from the middle of the body, in eye radii. */
  dx?: number;
  dy?: number;
}

/** Where a mood looks, and how it holds itself while it looks. */
export interface Watch {
  /** False never blinks, "slow" blinks at about half the rate. */
  blink?: boolean | "slow";
  gaze?: Gaze;
  /** A shiver on the eyes only, in eye radii. */
  jitter?: number;
  /** A slow swell on the weight and the bow, so a held face is not a still one. */
  breath?: { amp: number; hz: number };
  /**
   * How the eye changes as it looks up, blended in by how far up it has got.
   * A mood that looks at something and minds what it sees needs no second face.
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
}

/**
 * How long the clay takes to catch up with the eyes, in seconds. The whole of
 * what makes a look read as pulling the body rather than as two things being
 * animated at once. Spent in `AgentAvatar`, which follows it with a critically
 * damped spring so that a look, a peer being addressed and a message landing
 * all arrive through the same filter.
 */
export const SETTLE = 0.44;

/**
 * Where an aimed look points, in body radii, and what it does to the stroke.
 *
 * The two halves are both needed. `AIM` is spent as a gaze, so the mass leans
 * and swells after it exactly as it does for a mood's own wandering; but two
 * marks sliding a few units down a face do not read as looking down, because
 * nothing about the eye itself changed. What reads is the lid coming with
 * them. So an aimed look also moulds the stroke, toward a line and thinner as
 * the look drops, back toward a dot as it lifts, and carries the pair a little
 * further through `dy`, which is in eye radii and costs the outline nothing.
 * All of it is added to whatever the mood made the eye, so a frustrated
 * creature aiming downward is still frustrated.
 *
 * The asymmetry is measured, not chosen. Every one of these bodies hangs its
 * mass below its eyes, so there is depth under them and very little over them:
 * the down look has almost three units of the outline to spare and the up look
 * has a third of one, at the character the suite in `form.test.ts` binds on.
 * That is also why the up look rounds the stroke rather than fattening it --
 * weight is the term that eats what is left of the room -- and why the widest
 * eyes on the table, `surprised`, still fit while looking up at whoever just
 * threw something at them.
 */
export const AIM = { down: 0.3, up: 0.22 } as const;

const AIMED = {
  down: { lid: 0.34, bow: 0.2, sep: -0.08, dy: 0.45 },
  up: { lid: -0.4, bow: -0.06, sep: -0.06, dy: 0 },
} as const;

/** One mood's eye, with an aimed look moulded into it. */
export function aimedEye(eye: Eye, at: "up" | "down"): Eye {
  const { lid, bow, sep, dy } = AIMED[at];
  return {
    ...eye,
    /* Eased toward a line rather than swapped for one, as a blink is, so an eye
       that is already a dash does not grow a second length of its own; and back
       toward the dot it was cut from when the look lifts instead. */
    w: lid > 0 ? eye.w + (1.3 - eye.w) * lid : eye.w * (1 + lid),
    h: eye.h * (1 - 0.62 * Math.max(lid, 0)),
    c: (eye.c ?? 0) + bow,
    sep: (eye.sep ?? 0) + sep,
    dy: (eye.dy ?? 0) + dy,
  };
}

/** Deterministic noise. The same creature blinks the same way every reload. */
function rnd(i: number): number {
  const x = Math.sin(i * 127.1 + 197.3) * 43758.5453;
  return x - Math.floor(x);
}

/** 0 open to 1 shut, in slots of about four seconds, one in four doubled. */
export function blinkAmount(t: number): number {
  const P = 4.6;
  const DUR = 0.18;
  let out = 0;
  for (let k = -1; k <= 0; k++) {
    const i = Math.floor(t / P) + k;
    const at = i * P + rnd(i) * (P - 0.8);
    const d = t - at;
    if (d >= 0 && d < DUR) out = Math.max(out, Math.sin((d / DUR) * Math.PI));
    if (rnd(i + 0.5) > 0.7) {
      const again = t - (at + 0.25);
      if (again >= 0 && again < DUR) out = Math.max(out, Math.sin((again / DUR) * Math.PI));
    }
  }
  return out;
}

/** Quintic. It leaves and arrives with no corner on it. */
function ease(u: number): number {
  const e = Math.max(0, Math.min(1, u));
  return e * e * e * (e * (e * 6 - 15) + 10);
}

/** A gaze that was written down: `[x, y, hold]` steps, cycled. */
function scripted(t: number, g: Gaze): Point {
  const steps = g.script as [number, number, number][];
  const total = steps.reduce((sum, step) => sum + step[2], 0);
  let u = t % total;
  if (u < 0) u += total;
  let i = 0;
  while (u >= (steps[i] as [number, number, number])[2]) {
    u -= (steps[i] as [number, number, number])[2];
    i = (i + 1) % steps.length;
  }
  const from = steps[(i - 1 + steps.length) % steps.length] as [number, number, number];
  const to = steps[i] as [number, number, number];
  const k = ease(u / (g.cross ?? 0.26));
  return [from[0] + (to[0] - from[0]) * k, from[1] + (to[1] - from[1]) * k];
}

/**
 * A saccade. `hz` is how often it happens and `cross` is how long the move
 * takes, which are two separate decisions: a creature that looks around rarely
 * does not also move its eyes slowly. Tying them together made every mood feel
 * hurried.
 */
export function gazeAt(t: number, g?: Gaze): Point {
  if (!g) return [0, 0];
  if (g.script) return scripted(t, g);
  const range = g.range ?? 0;
  const P = 1 / (g.hz ?? 0.5);
  const i = Math.floor(t / P);
  let u = (t / P) % 1;
  if (u < 0) u += 1;
  const at = (k: number): Point => [
    (rnd(k) * 2 - 1) * range + (g.bias ? g.bias[0] : 0),
    (rnd(k + 0.37) * 2 - 1) * range * 0.62 + (g.bias ? g.bias[1] : 0),
  ];
  const from = at(i - 1);
  const to = at(i);
  const k = ease((u * P) / (g.cross ?? 0.26));
  return [from[0] + (to[0] - from[0]) * k, from[1] + (to[1] - from[1]) * k];
}

/** One eye, resolved to numbers a path can be written from. */
export interface Drawn {
  x: number;
  y: number;
  /** Half length, weight and bow in viewBox units, and the tilt in radians. */
  w: number;
  h: number;
  c: number;
  ang: number;
}

function geometry(
  e: Eye,
  r: number,
  side: number,
  blink: number,
  breath: number,
  squint: number,
  squintTo: Partial<Eye>,
) {
  let w = e.w;
  let h = e.h;
  let c = e.c ?? 0;
  let ang = (e.a ?? 0) * side;
  if (squint > 0) {
    /* Looking somewhere can change the shape of the eye, which is what stops a
       mood being a fixed face with a moving body under it. */
    w += squint * (squintTo.w ?? 0);
    h += squint * (squintTo.h ?? 0);
    ang += squint * (squintTo.a ?? 0) * side;
  }
  if (breath) {
    h *= 1 + breath * 0.16;
    c *= 1 + breath * 0.3;
  }
  if (blink > 0) {
    /* A blink is a mould, not a mask: the eye is squashed into its own line. */
    w = w + (1.3 - w) * blink;
    h = h * (1 - 0.88 * blink);
    c = c * (1 - blink);
  }
  return {
    w: Math.max(w, 0.012) * r,
    h: Math.max(h, 0.1) * r,
    c: c * r,
    ang: (ang * Math.PI) / 180,
  };
}

/** Both eyes, this instant. `gaze` is in body radii. */
export function eyesAt(
  lump: Lump,
  eye: Eye,
  watch: Watch | undefined,
  t: number,
  live: boolean,
  gaze: Point,
): Drawn[] {
  const anim = watch ?? {};
  const r = lump.eye.r;
  const [gx, gy] = gaze;
  /* Looking hard to one side foreshortens the pair, which is what stops a big
     excursion reading as two stickers sliding across a ball. */
  const sep = (lump.eye.spread + (eye.sep ?? 0) * r) * (1 - Math.abs(gx) * 0.5);
  const blink =
    live && anim.blink !== false ? blinkAmount(t * (anim.blink === "slow" ? 0.55 : 1)) : 0;
  const jitter = live && anim.jitter ? anim.jitter : 0;
  const jx = jitter ? Math.sin(t * 41) * jitter : 0;
  const jy = jitter ? Math.sin(t * 33 + 1.7) * jitter * 0.7 : 0;
  const breath = live && anim.breath ? Math.sin(t * anim.breath.hz * TAU) * anim.breath.amp : 0;
  const squintTo = anim.squint ?? {};
  const squint = anim.squint ? Math.max(0, Math.min(1, -gy / anim.squint.at)) : 0;

  const x = FORM.center + (lump.eye.x ?? 0) + gx * FORM.radius + (jx + (eye.dx ?? 0)) * r;
  const y = FORM.center + (lump.eye.y ?? 0) + gy * FORM.radius + (jy + (eye.dy ?? 0)) * r;

  if (lump.eye.one) {
    return [{ x, y, ...geometry(eye, r * 1.5, 1, blink, breath, squint, squintTo) }];
  }
  return [
    { x: x - sep, y, ...geometry(eye, r, 1, blink, breath, squint, squintTo) },
    { x: x + sep, y, ...geometry(eye, r, -1, blink, breath, squint, squintTo) },
  ];
}

/** The stroke, as a path. Weight and cap are set on the element. */
export function eyePath(e: Drawn): string {
  const dx = Math.cos(e.ang) * e.w;
  const dy = Math.sin(e.ang) * e.w;
  const px = -Math.sin(e.ang) * e.c * 2;
  const py = Math.cos(e.ang) * e.c * 2;
  return `M${(e.x - dx).toFixed(2)} ${(e.y - dy).toFixed(2)}Q${(e.x + px).toFixed(2)} ${(e.y + py).toFixed(2)} ${(e.x + dx).toFixed(2)} ${(e.y + dy).toFixed(2)}`;
}

/** Eye shapes lerp exactly as the body does. Nothing switches. */
export function blendEyes(a: Eye, b: Eye, u: number): Eye {
  const at = (k: keyof Eye, fallback: number) =>
    (a[k] ?? fallback) * (1 - u) + (b[k] ?? fallback) * u;
  return {
    w: at("w", 0),
    h: at("h", 1),
    c: at("c", 0),
    a: at("a", 0),
    sep: at("sep", 0),
    dx: at("dx", 0),
    dy: at("dy", 0),
  };
}
