/**
 * The corner of the rail: the icon's two creatures, and the name beside them
 * for as long as it is worth reading.
 *
 * The rail used to open on `GUACA` in tracked capitals on a row of its own,
 * which is the section heads' type in the section heads' tracking: it read as
 * one more label in the chrome, and it spent 45 pixels of the column on a word
 * the operator stops reading on the second day. So the name is on screen for
 * the few seconds after the app opens and whenever the pointer asks for it,
 * and what stays is the pair from the app's icon, in the strip beside the
 * window's own buttons that was empty anyway.
 *
 * On launch they arrive, look at each other, one throws something and the
 * other catches it, the name writes in, and both turn to the operator. That is
 * the product in four seconds, which is the only argument for a performance at
 * all. Afterwards they are ordinary creatures: they breathe, blink and glance
 * on the avatars' own clock, find each other every eleven seconds, and trade
 * something every third time.
 *
 * They report one thing, and only one: a turn parked on the operator turns the
 * first of them `blocked`, which is the face every agent in the rail makes for
 * the same reason and carries the amber mark that means exactly that. Work in
 * flight, a message landing and a quiet workspace all have a place to be seen
 * already, and a corner that echoed them would be a second tally.
 *
 * A pure function of the seconds since launch, so a scene is the same at the
 * same second however it was reached, and a test can stand at any moment of it.
 * `next` is when the scene next changes, which is all the component needs to
 * know to render at the beats and at nothing in between.
 */

import type { Point } from "../avatars/form";
import type { Gesture } from "../avatars/frame";
import type { Mood } from "../avatars/moods";

/** Seconds the name is held after launch before it folds away. */
export const HOLD = 6.5;

/** When the performance is over and the pair is left to itself. */
export const SETTLED = 3.8;

/** How long a throw or a catch is held, which is how long the knock lasts. */
const KNOCK = 0.9;

/** How long the pair is pleased to see the pointer. */
const GREETED = 1.4;

/** The pair finds each other once a cycle, for this long, and trades something every third time. */
const CYCLE = { every: 11, from: 4, until: 5.6, send: 4.3, catch: 4.55, trade: 3 } as const;

/* Where to look, in body radii. At the operator is straight out of the
   screen, which on a face drawn head-on is the pupils centered and a hair low.
   Across is the idle mood's own range, so no look here is further than a
   look any creature already takes on its own. */
const FACE: Point = [0, 0.12];
const RIGHT: Point = [0.36, 0.02];
const LEFT: Point = [-0.36, 0.02];

/** How far a look at the pointer goes, across and up and down. `AIM`'s own limits. */
const TOWARD = { across: 0.38, up: 0.25, down: 0.3 } as const;

/** What one of the two is doing. `pointer` is resolved by whoever knows where it is. */
export interface Cue {
  shown: boolean;
  mood: Mood;
  gaze: Point | "pointer" | null;
  gesture: Gesture;
}

export interface Scene {
  left: Cue;
  right: Cue;
  /** Whether the name is out. */
  named: boolean;
  /** Seconds since launch at which this scene next changes. Infinite when only an event can change it. */
  next: number;
}

export interface Situation {
  /** Seconds since launch the pointer arrived, or null while it is elsewhere. */
  hoverAt: number | null;
  /** A turn is parked on the operator. */
  waiting: boolean;
  /** Reduced motion: no performance, no glances, the name only on a hover. */
  still: boolean;
}

function step<T>(t: number, steps: [number, T][]): T {
  let out = (steps[0] as [number, T])[1];
  for (const [at, value] of steps) if (t >= at) out = value;
  return out;
}

/* Every moment the launch changes something, in one list, so `next` cannot
   disagree with the scene it is predicting. */
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

/** A throw or a catch, held for as long as its knock, and null either side so each one is a change. */
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

  /* The performance is not interrupted: a turn that parks during it is shown
     the moment it ends, and a pointer that arrives during it is greeted after. */
  const performing = !s.still && t < SETTLED;
  if (!performing && s.waiting) left = { ...left, mood: "blocked", gaze: null };
  if (!performing && hovering) {
    const greeted = t - (s.hoverAt as number) < GREETED;
    for (const cue of [left, right]) {
      cue.gaze = "pointer";
      /* Still, a greeting would be a face held until something else changed. */
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

/**
 * Where a creature looks to meet the pointer, in body radii.
 *
 * A pointer on the face is met straight on, and one a hand away gets the whole
 * of the look; past that only the direction changes, because a creature that
 * strained further the further away the pointer went would be looking off its
 * own face.
 */
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
