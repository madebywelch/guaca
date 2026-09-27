/**
 * What every design on the page shares: the clock, the moods, the saccade, and
 * the one loop that paints every creature on screen.
 *
 * A design is handed a frame and owns everything else, including its own state
 * between frames (a soft body keeps its points, an inked one keeps its last
 * drawing). The harness decides only what is true: which mood, since when,
 * where the operator's pointer is, whether a message just landed.
 */

import { CHARACTERS, type Character, lookupCharacter } from "../../src/avatars/catalog";
import { gaitOf } from "../../src/avatars/clock";
import type { Mood } from "../../src/avatars/moods";

export type { Character, Mood };
export type Point = [number, number];
export type Look = "up" | "down" | null;
export type Gesture = "send" | "receive" | null;

export const MOOD_LIST: Mood[] = [
  "idle",
  "listening",
  "thinking",
  "working",
  "frustrated",
  "blocked",
  "pleased",
  "paused",
  "stuck",
  "surprised",
];

export const MOOD_SIGNAL: Record<Mood, string> = {
  idle: "active, nothing in flight",
  listening: "a message queued",
  thinking: "between rounds",
  working: "a tool call in flight",
  frustrated: "last call refused or failed",
  blocked: "parked on a person",
  pleased: "reply just landed",
  paused: "paused or composted",
  stuck: "its escalation is open",
  surprised: "just handed a message",
};

/** How long taking a message looks surprised. Same number as `moods.ts`. */
export const STRUCK = 0.9;

export const C = 32;
export const R = 20;
export const REACH = 30;
export const TAU = Math.PI * 2;
export const INK = "#252824";

export interface Frame {
  /** The creature's own clock, gait applied. Only cycles run on it. */
  t: number;
  /** Shared seconds. Every age is measured on these. */
  now: number;
  dt: number;
  live: boolean;
  mood: Mood;
  from: Mood;
  /** Seconds since `from` began becoming `mood`. */
  since: number;
  look: Look;
  gesture: Gesture;
  /** Seconds since the gesture began. */
  age: number;
  /** The operator's pointer, in body radii from the center, when this creature is watching it. */
  point: Point | null;
  /** Rendered size in CSS pixels. What stroke hinting reads. */
  px: number;
}

export interface Creature {
  paint(f: Frame): void;
}

export interface Design {
  key: string;
  name: string;
  create(svg: SVGSVGElement, who: Character, color: string, seed: string): Creature;
}

/* --- numbers ------------------------------------------------------------- */

/** Deterministic noise, the same function `eyes.ts` uses. */
export function rnd(i: number): number {
  const x = Math.sin(i * 127.1 + 197.3) * 43758.5453;
  return x - Math.floor(x);
}

export function clamp(x: number, lo = 0, hi = 1): number {
  return Math.max(lo, Math.min(hi, x));
}

export function smooth(u: number): number {
  const e = clamp(u);
  return e * e * e * (e * (e * 6 - 15) + 10);
}

export function lerp(a: number, b: number, u: number): number {
  return a + (b - a) * u;
}

/** A 0..1..0 bump over `dur` seconds starting at 0. */
export function bump(x: number, dur: number): number {
  return x >= 0 && x < dur ? Math.sin((x / dur) * Math.PI) : 0;
}

/** Stroke weight grows as the creature shrinks, so a closed eye survives 24px. */
export function hint(px: number, at = 44, most = 1.8): number {
  return clamp(at / px, 1, most);
}

/* --- color --------------------------------------------------------------- */

function rgb(hex: string): [number, number, number] {
  const h = hex.replace("#", "");
  return [0, 2, 4].map((i) => Number.parseInt(h.slice(i, i + 2), 16)) as [number, number, number];
}

export function mix(a: string, b: string, u: number): string {
  const p = rgb(a);
  const q = rgb(b);
  return `#${p
    .map((v, i) =>
      Math.round(v + ((q[i] as number) - v) * u)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

/* --- the look ------------------------------------------------------------ */

export interface GazeSpec {
  range?: number;
  hz?: number;
  cross?: number;
  bias?: Point;
  script?: [number, number, number][];
  far?: number;
  near?: number;
}

export interface Glance {
  at: Point;
  /** Where the current jump is going. */
  to: Point;
  /** Seconds since the current jump began. */
  jump: number;
  /** How far the current jump goes, in body radii. */
  size: number;
  /** Which jump this is, for anything that wants to roll against it. */
  slot: number;
}

function ease(u: number): number {
  return smooth(u);
}

function span(from: Point, to: Point, cross: number): number {
  const far = Math.hypot(to[0] - from[0], to[1] - from[1]);
  return cross * Math.max(1, Math.min(2, far / 0.25));
}

/**
 * `gazeAt` from `eyes.ts`, with the jump it is in reported alongside. A design
 * that blinks into a saccade has to know when one began.
 */
export function saccade(t: number, g?: GazeSpec): Glance {
  if (!g) return { at: [0, 0], to: [0, 0], jump: 99, size: 0, slot: 0 };
  const cross = g.cross ?? 0.26;
  if (g.script) {
    const steps = g.script;
    const total = steps.reduce((s, step) => s + step[2], 0);
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
    const k = ease(u / span(from, to, cross));
    return {
      at: [lerp(from[0], to[0], k), lerp(from[1], to[1], k)],
      to,
      jump: u,
      size: Math.hypot(to[0] - from[0], to[1] - from[1]),
      slot: lap * steps.length + i,
    };
  }
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
  const k = ease(jump / span(from, to, cross));
  return {
    at: [lerp(from[0], to[0], k), lerp(from[1], to[1], k)],
    to,
    jump,
    size: Math.hypot(to[0] - from[0], to[1] - from[1]),
    slot: i,
  };
}

/** `blinkAmount` from `eyes.ts`: slots of about four seconds, one in four doubled. */
export function blinkAt(t: number): number {
  const P = 4.6;
  const DUR = 0.18;
  let out = 0;
  for (let k = -1; k <= 0; k++) {
    const i = Math.floor(t / P) + k;
    const at = i * P + rnd(i) * (P - 0.8);
    out = Math.max(out, bump(t - at, DUR));
    if (rnd(i + 0.5) > 0.7) out = Math.max(out, bump(t - at - 0.25, DUR));
  }
  return out;
}

/** Critically damped follow, in place. The `settle` in `eyes.ts`. */
export function follow(pos: Point, vel: Point, target: Point, dt: number, settle = 0.12) {
  const w = 2.2 / settle;
  for (const i of [0, 1] as const) {
    const accel = w * w * (target[i] - pos[i]) - 2 * w * vel[i];
    vel[i] += accel * dt;
    pos[i] += vel[i] * dt;
  }
}

/** Where the operator's pointer is as a look, in body radii: near is a glance, far is a stare. */
export function lookToward(dx: number, dy: number): Point {
  const d = Math.hypot(dx, dy);
  if (d < 1e-6) return [0, 0];
  const reach = 0.42 * (1 - Math.exp(-d / 2.4));
  return [(dx / d) * reach, (dy / d) * reach * 0.8];
}

/* --- svg ----------------------------------------------------------------- */

const NS = "http://www.w3.org/2000/svg";

export function el<K extends keyof SVGElementTagNameMap>(
  tag: K,
  attrs: Record<string, string | number> = {},
  parent?: Element,
): SVGElementTagNameMap[K] {
  const node = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, String(v));
  parent?.appendChild(node);
  return node;
}

let ids = 0;
export function uid(prefix: string): string {
  ids += 1;
  return `${prefix}${ids}`;
}

export function poly(pts: Point[], close = true): string {
  if (pts.length === 0) return "";
  const head = pts[0] as Point;
  let d = `M${head[0].toFixed(2)} ${head[1].toFixed(2)}`;
  for (let i = 1; i < pts.length; i++) {
    const p = pts[i] as Point;
    d += `L${p[0].toFixed(2)} ${p[1].toFixed(2)}`;
  }
  return close ? `${d}Z` : d;
}

/** Catmull-Rom through a closed loop, resampled `per` times a segment. */
export function resample(pts: Point[], per: number): Point[] {
  const out: Point[] = [];
  const n = pts.length;
  for (let i = 0; i < n; i++) {
    const p0 = pts[(i - 1 + n) % n] as Point;
    const p1 = pts[i] as Point;
    const p2 = pts[(i + 1) % n] as Point;
    const p3 = pts[(i + 2) % n] as Point;
    for (let j = 0; j < per; j++) {
      const s = j / per;
      const s2 = s * s;
      const s3 = s2 * s;
      const f = (a: number, b: number, c: number, d: number) =>
        0.5 * (2 * b + (-a + c) * s + (2 * a - 5 * b + 4 * c - d) * s2 + (-a + 3 * b - 3 * c + d) * s3);
      out.push([f(p0[0], p1[0], p2[0], p3[0]), f(p0[1], p1[1], p2[1], p3[1])]);
    }
  }
  return out;
}

/* --- instances and the one loop ------------------------------------------ */

export interface Handle {
  el: HTMLElement;
  setMood(m: Mood): void;
  setLook(l: Look): void;
  gesture(g: "send" | "receive"): void;
  setFollow(on: boolean): void;
  destroy(): void;
  readonly mood: Mood;
}

interface Instance {
  host: HTMLElement;
  creature: Creature;
  gait: { phase: number; rate: number };
  base: Mood;
  shown: Mood;
  from: Mood;
  at: number;
  look: Look;
  gesture: Gesture;
  gestureAt: number;
  struck: number;
  follow: boolean;
  last: number;
  px: number;
}

const live = new Set<Instance>();
const onScreen = new WeakSet<Element>();
const observer = new IntersectionObserver(
  (rows) => {
    for (const row of rows) {
      if (row.isIntersecting) onScreen.add(row.target);
      else onScreen.delete(row.target);
    }
  },
  { rootMargin: "200px" },
);

export const world = {
  still: false,
  followAll: true,
  pointer: null as Point | null,
  movedAt: -99,
};

function clock(): number {
  return performance.now() / 1000;
}

export function mount(
  host: HTMLElement,
  design: Design,
  opts: { who: string; color: string; mood?: Mood; px: number; seed?: string; follow?: boolean },
): Handle {
  host.classList.add("creature", `creature--${design.key}`);
  host.style.width = `${opts.px}px`;
  host.style.height = `${opts.px}px`;
  host.style.setProperty("--accent", opts.color);
  const svg = el("svg", { viewBox: "0 0 64 64", class: "creature__svg", "aria-hidden": "true" });
  host.appendChild(svg);
  const who = lookupCharacter(opts.who);
  const seed = opts.seed ?? `${opts.who}-${opts.color}`;
  const gait = gaitOf(seed);
  host.style.setProperty("--gait", `${gait.phase.toFixed(2)}s`);
  const now = clock();
  const mood = opts.mood ?? "idle";
  const inst: Instance = {
    host,
    creature: design.create(svg, who, opts.color, seed),
    gait,
    base: mood,
    shown: mood,
    from: mood,
    at: now - 5,
    look: null,
    gesture: null,
    gestureAt: -99,
    struck: -99,
    follow: opts.follow ?? false,
    last: now,
    px: opts.px,
  };
  live.add(inst);
  observer.observe(host);
  paintOne(inst, now, 0);
  return {
    el: host,
    setMood(m) {
      inst.base = m;
    },
    setLook(l) {
      inst.look = l;
    },
    gesture(g) {
      const t = clock();
      inst.gesture = g;
      inst.gestureAt = t;
      if (g === "receive") inst.struck = t;
    },
    setFollow(on) {
      inst.follow = on;
    },
    destroy() {
      live.delete(inst);
      observer.unobserve(host);
      host.replaceChildren();
    },
    get mood() {
      return inst.base;
    },
  };
}

function paintOne(inst: Instance, now: number, dt: number) {
  const want = now - inst.struck < STRUCK ? "surprised" : inst.base;
  if (want !== inst.shown) {
    inst.from = inst.shown;
    inst.shown = want;
    inst.at = now;
    inst.host.dataset.mood = want;
  }
  if (inst.gesture && now - inst.gestureAt > 1.2) inst.gesture = null;
  let point: Point | null = null;
  if (inst.follow && world.followAll && world.pointer && now - world.movedAt < 2.6) {
    const box = inst.host.getBoundingClientRect();
    const r = (box.width * R) / 64;
    point = lookToward(
      (world.pointer[0] - (box.left + box.width / 2)) / r,
      (world.pointer[1] - (box.top + box.height / 2)) / r,
    );
  }
  inst.creature.paint({
    t: now * inst.gait.rate + inst.gait.phase,
    now,
    dt,
    live: !world.still,
    mood: inst.shown,
    from: inst.from,
    since: world.still ? 99 : now - inst.at,
    look: inst.look,
    gesture: inst.gesture,
    age: now - inst.gestureAt,
    point,
    px: inst.px,
  });
}

const tickers = new Set<(now: number, dt: number) => void>();
export function onTick(fn: (now: number, dt: number) => void) {
  tickers.add(fn);
}

let prev = clock();
function tick() {
  requestAnimationFrame(tick);
  const now = clock();
  const dt = Math.min(0.05, Math.max(0, now - prev));
  prev = now;
  for (const fn of tickers) fn(now, dt);
  for (const inst of live) {
    if (!onScreen.has(inst.host)) continue;
    const step = Math.min(0.05, Math.max(0, now - inst.last));
    inst.last = now;
    paintOne(inst, now, step);
  }
}
requestAnimationFrame(tick);

addEventListener("pointermove", (e) => {
  world.pointer = [e.clientX, e.clientY];
  world.movedAt = clock();
});
document.addEventListener("pointerleave", () => {
  world.pointer = null;
});

export { CHARACTERS };
