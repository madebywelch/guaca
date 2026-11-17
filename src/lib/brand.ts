import type { Point } from "../avatars/form";
import type { Gesture } from "../avatars/frame";
import type { Mood } from "../avatars/moods";

export const HOLD = 6.5;

export const SETTLED = 3.8;

const KNOCK = 0.9;

const GREETED = 1.4;

const CYCLE = { every: 11, from: 4, until: 5.6, send: 4.3, catch: 4.55, trade: 3 } as const;

const FACE: Point = [0, 0.12];
const RIGHT: Point = [0.36, 0.02];
const LEFT: Point = [-0.36, 0.02];

const TOWARD = { across: 0.38, up: 0.25, down: 0.3 } as const;

export interface Cue {
  shown: boolean;
  mood: Mood;
  gaze: Point | "pointer" | null;
  gesture: Gesture;
}

export interface Scene {
  left: Cue;
  right: Cue;
  named: boolean;
  next: number;
}

export interface Situation {
  hoverAt: number | null;
  waiting: boolean;
  still: boolean;
}

function step<T>(t: number, steps: [number, T][]): T {
  let out = (steps[0] as [number, T])[1];
  for (const [at, value] of steps) if (t >= at) out = value;
  return out;
}

const LEFT_MOOD: [number, Mood][] = [
  [0, "surprised"],
  [0.8, "idle"],
  [2.5, "pleased"],
  [SETTLED, "idle"],
];
const LEFT_GAZE: [number, Point | null][] = [
  [0, FACE],
  [0.8, RIGHT],
  [2.5, FACE],
  [SETTLED, null],
];
const RIGHT_MOOD: [number, Mood][] = [
  [0, "surprised"],
  [1.0, "idle"],
  [1.5, "surprised"],
  [1.95, "pleased"],
  [SETTLED, "idle"],
];
const RIGHT_GAZE: [number, Point | null][] = [
  [0, FACE],
  [0.9, LEFT],
  [2.55, FACE],
  [SETTLED, null],
];
const ARRIVE = { left: 0.05, right: 0.4 };
const THROW = 1.25;
const CATCH = 1.5;
const NAMED = 1.95;
const LAUNCH_EDGES = [
  ...new Set(
    [
      ARRIVE.left,
      ARRIVE.right,
      THROW,
      THROW + KNOCK,
      CATCH,
      CATCH + KNOCK,
      NAMED,
      ...[LEFT_MOOD, LEFT_GAZE, RIGHT_MOOD, RIGHT_GAZE].flatMap((steps) => steps.map(([at]) => at)),
    ].filter((at) => at > 0),
  ),
].sort((a, b) => a - b);

function gesture(t: number, at: number, kind: Exclude<Gesture, null>): Gesture {
  return t >= at && t < at + KNOCK ? kind : null;
}

export function brandAt(t: number, s: Situation): Scene {
  const hovering = s.hoverAt !== null;
  let left: Cue;
  let right: Cue;
  let edges: number[];

  if (s.still) {
    left = { shown: true, mood: "idle", gaze: null, gesture: null };
    right = { shown: true, mood: "idle", gaze: null, gesture: null };
    edges = [];
  } else if (t < SETTLED) {
    left = {
      shown: t >= ARRIVE.left,
      mood: step(t, LEFT_MOOD),
      gaze: step(t, LEFT_GAZE),
      gesture: gesture(t, THROW, "send"),
    };
    right = {
      shown: t >= ARRIVE.right,
      mood: step(t, RIGHT_MOOD),
      gaze: step(t, RIGHT_GAZE),
      gesture: gesture(t, CATCH, "receive"),
    };
    edges = LAUNCH_EDGES;
  } else {
    left = { shown: true, mood: "idle", gaze: null, gesture: null };
    right = { shown: true, mood: "idle", gaze: null, gesture: null };
    const cycle = Math.floor((t - SETTLED) / CYCLE.every);
    const base = SETTLED + cycle * CYCLE.every;
    const phase = t - base;
    if (phase >= CYCLE.from && phase < CYCLE.until) {
      left.gaze = RIGHT;
      right.gaze = LEFT;
    }
    if (cycle % CYCLE.trade === CYCLE.trade - 1) {
      left.gesture = gesture(t, base + CYCLE.send, "send");
      right.gesture = gesture(t, base + CYCLE.catch, "receive");
    }
    edges = [0, CYCLE.every].flatMap((offset) =>
      [
        CYCLE.from,
        CYCLE.send,
        CYCLE.send + KNOCK,
        CYCLE.catch,
        CYCLE.catch + KNOCK,
        CYCLE.until,
      ].map((at) => base + offset + at),
    );
  }

  const performing = !s.still && t < SETTLED;
  if (!performing && s.waiting) left = { ...left, mood: "blocked", gaze: null };
  if (!performing && hovering) {
    const greeted = t - (s.hoverAt as number) < GREETED;
    for (const cue of [left, right]) {
      cue.gaze = "pointer";
      if (cue.mood === "idle" && greeted && !s.still) cue.mood = "pleased";
    }
    if (!s.still) edges = [...edges, (s.hoverAt as number) + GREETED];
  }

  const named = (!performing && hovering) || (!s.still && t >= NAMED && t < HOLD);
  if (!s.still) edges = [...edges, HOLD];
  const next = edges
    .filter((at) => at > t)
    .reduce((a, b) => Math.min(a, b), Number.POSITIVE_INFINITY);
  return { left, right, named, next };
}

export function toward(
  face: { x: number; y: number; width: number; height: number },
  x: number,
  y: number,
): Point {
  const dx = x - (face.x + face.width / 2);
  const dy = y - (face.y + face.height / 2);
  const d = Math.hypot(dx, dy);
  if (d === 0) return [0, 0];
  const k = Math.min(1, d / (face.width * 2.5));
  return [
    (dx / d) * TOWARD.across * k,
    Math.max(-TOWARD.up, Math.min(TOWARD.down, (dy / d) * TOWARD.across * k)),
  ];
}
