/**
 * Three performances of the same idea: the name is on screen for the first few
 * seconds of a launch and then gives the corner back, leaving only the cast.
 *
 * Each direction is a pure function of the seconds since launch and a handful
 * of flags, so a scene is the same scene at the same second however it was
 * reached, and replaying one is setting a number back to zero. Precedence, in
 * every one of them: the launch, then the pointer, then whatever the app is
 * doing, then rest.
 */

import { ACCENTS, lookupCharacter } from "../../src/avatars/catalog";
import type { Lump, Point } from "../../src/avatars/form";
import type { Mood } from "../../src/avatars/moods";
import type { Cue, Look } from "./actor";

export type Happening = "none" | "message" | "working" | "waiting" | "asleep";

export interface Flags {
  /** Scene seconds at which the mark condensed. Infinite until it has. */
  condensedAt: number;
  hover: boolean;
  /** Scene seconds at which the pointer last arrived. */
  hoverAt: number;
  /** How many times the pointer has arrived. The shapeshifter spends it. */
  visits: number;
  happening: Happening;
  happeningAt: number;
}

export interface Scene {
  cues: Cue[];
  /** How much of the set name is shown, 0 to 1. */
  name: number;
  /** A line said in a bubble, and when it was said; a new time is a new bubble. */
  says: { text: string; at: number } | null;
}

export interface Direction {
  key: string;
  name: string;
  title: string;
  why: string;
  beats: [string, string][];
  cast: { lump: Lump; color: string; seed: string }[];
  /** Set beside the cast, or said by it. */
  voice: "set" | "said";
  /** How wide the condensed mark is at 1x, in CSS pixels. */
  footprint: number;
  at(t: number, f: Flags): Scene;
}

/** Seconds after launch the full mark is held before it condenses. A number to tune, not a decision. */
export const HOLD = 6.5;
/** How long a message landing holds a creature's attention. */
const STRUCK = 2.4;

export const PIGMENT = Object.fromEntries(ACCENTS.map((a) => [a.name, a.value])) as Record<string, string>;
const INK = "#2e2f2a";

/* Where to look, in body radii. At the operator is straight out of the screen,
   which on a face drawn head-on is the pupils centered and a hair low. */
const FACE: Point = [0, 0.12];
const RIGHT: Point = [0.36, 0.02];
const LEFT: Point = [-0.36, 0.02];

function step<T>(t: number, steps: [number, T][]): T {
  let out = (steps[0] as [number, T])[1];
  for (const [at, value] of steps) if (t >= at) out = value;
  return out;
}

const smooth = (x: number) => (x <= 0 ? 0 : x >= 1 ? 1 : x * x * (3 - 2 * x));
const ramp = (t: number, at: number, over: number) => smooth((t - at) / over);

/** The name once the launch is over: out while the pointer is on the mark, in once it has condensed. */
function settled(t: number, f: Flags): number {
  if (f.hover) return 1;
  return t >= f.condensedAt ? 0 : 1;
}

/**
 * What the app is doing, as a face. Only the loud states are worth the
 * corner: a message landing, work in flight, a turn parked on the operator,
 * and nothing at all for a long time. `bang`, the amber mark, is the parked
 * turn and nothing else, which is the one place amber may appear here.
 */
function happening(t: number, f: Flags, who: number, of: number): Partial<Cue> | null {
  const age = t - f.happeningAt;
  switch (f.happening) {
    case "message":
      if (age > STRUCK) return null;
      if (who === of - 1) {
        return {
          mood: age < 0.6 ? "surprised" : "idle",
          gesture: { kind: "receive", at: f.happeningAt },
          look: of > 1 && age > 0.6 ? LEFT : null,
        };
      }
      return { look: age > 0.5 ? RIGHT : null };
    case "working":
      return { mood: who % 2 === 0 ? "thinking" : "working", look: null };
    case "waiting":
      return who === 0 ? { mood: "blocked", look: FACE } : { mood: "idle", look: LEFT };
    case "asleep":
      return { mood: "paused", look: null };
    default:
      return null;
  }
}

function rested(t: number, f: Flags, cue: Cue, who: number, of: number): Cue {
  if (f.hover) {
    return {
      ...cue,
      look: "pointer",
      mood: t - f.hoverAt < 1.4 ? "pleased" : "idle",
    };
  }
  const now = happening(t, f, who, of);
  return now ? { ...cue, ...now } : cue;
}

/* --- Pair ------------------------------------------------------------------ */

export const pair: Direction = {
  key: "pair",
  name: "Pair",
  title: "The icon, alive.",
  why:
    "The shipped icon's two creatures, in the corner where the name was. On launch they arrive, " +
    "one throws something, the other catches it, and both turn to you while the name writes in " +
    "beside them. Then the name folds away and the pair stays in the title bar, glancing at each " +
    "other now and then and at the pointer when it comes near.",
  beats: [
    ["0.0", "The first arrives, surprised, and looks at you."],
    ["0.4", "The second arrives."],
    ["0.9", "They look at each other."],
    ["1.3", "One throws; the other catches it."],
    ["2.0", "The name writes in."],
    ["2.5", "Both turn to you, pleased."],
    ["6.5", "The name folds away. The pair stays."],
  ],
  cast: [
    { lump: lookupCharacter("slab"), color: PIGMENT.Terracotta as string, seed: "pair-left" },
    { lump: lookupCharacter("slab"), color: INK, seed: "pair-right" },
  ],
  voice: "set",
  footprint: 50,
  at(t, f) {
    const launched = t >= 3.8;
    let a: Cue = {
      shown: t >= 0.05,
      mood: step<Mood>(t, [[0, "surprised"], [0.8, "idle"], [2.5, "pleased"], [3.8, "idle"]]),
      look: step<Look>(t, [[0, FACE], [0.8, RIGHT], [2.5, FACE], [3.8, null]]),
      gesture: t >= 1.25 ? { kind: "send", at: 1.25 } : null,
    };
    let b: Cue = {
      shown: t >= 0.4,
      mood: step<Mood>(t, [[0, "surprised"], [1.0, "idle"], [1.5, "surprised"], [1.95, "pleased"], [3.8, "idle"]]),
      look: step<Look>(t, [[0, FACE], [0.9, LEFT], [2.55, FACE], [3.8, null]]),
      gesture: t >= 1.5 ? { kind: "receive", at: 1.5 } : null,
    };
    if (launched) {
      /* Every eleven seconds they find each other for a moment, and every
         third time one of them has something to say. */
      const cycle = Math.floor((t - 3.8) / 11);
      const phase = (t - 3.8) % 11;
      const at = 3.8 + cycle * 11;
      if (phase >= 4 && phase < 5.6) {
        a = { ...a, look: RIGHT };
        b = { ...b, look: LEFT };
        if (cycle % 3 === 2) {
          a = { ...a, gesture: { kind: "send", at: at + 4.3 } };
          if (phase >= 4.55) b = { ...b, gesture: { kind: "receive", at: at + 4.55 } };
        }
      }
      a = rested(t, f, a, 0, 2);
      b = rested(t, f, b, 1, 2);
    }
    return { cues: [a, b], name: launched ? settled(t, f) : ramp(t, 1.95, 0.5), says: null };
  },
};

/* --- Namesake -------------------------------------------------------------- */

export const namesake: Direction = {
  key: "namesake",
  name: "Namesake",
  title: "The name is something it says.",
  why:
    "No wordmark at all. One creature, avocado-shaped and avocado-colored, says its name in the " +
    "bubble the app already uses for a shout, and that bubble is the only place the name is ever " +
    "drawn. Afterward it is one face in the title bar, and pointing at it makes it say its name again.",
  beats: [
    ["0.0", "It arrives, surprised, and looks at you."],
    ["0.7", "It looks around."],
    ["1.0", "It says its name."],
    ["2.4", "It looks back at you, pleased."],
    ["3.6", "It settles. It never condenses, because it never had a wordmark to fold away."],
  ],
  cast: [{ lump: lookupCharacter("egg"), color: PIGMENT.Olive as string, seed: "namesake" }],
  voice: "said",
  footprint: 24,
  at(t, f) {
    const launched = t >= 3.6;
    let cue: Cue = {
      shown: t >= 0.05,
      mood: step<Mood>(t, [[0, "surprised"], [0.7, "idle"], [1.0, "pleased"], [3.2, "idle"]]),
      look: step<Look>(t, [[0, FACE], [0.7, null], [1.0, [0.3, -0.06]], [2.4, FACE], [3.6, null]]),
      gesture: t >= 1.0 ? { kind: "send", at: 1.0 } : null,
    };
    let says: Scene["says"] = t >= 1.0 ? { text: "Guaca", at: 1.0 } : null;
    if (launched) {
      cue = rested(t, f, cue, 0, 1);
      if (f.hoverAt > 3.6) {
        says = { text: "Guaca", at: f.hoverAt };
        if (f.hover) cue = { ...cue, gesture: { kind: "send", at: f.hoverAt } };
      }
    }
    return { cues: [cue], name: 0, says };
  },
};

/* --- Shapeshifter ---------------------------------------------------------- */

/** One member of the cast per silhouette, in the order the name is written. */
const MEMBERS: { key: string; color: string; mood: Mood }[] = [
  { key: "orb", color: PIGMENT.Olive as string, mood: "surprised" },
  { key: "puck", color: PIGMENT.Indigo as string, mood: "pleased" },
  { key: "slab", color: PIGMENT.Slate as string, mood: "listening" },
  { key: "drop", color: PIGMENT.Madder as string, mood: "surprised" },
  { key: "lobe", color: PIGMENT.Verdigris as string, mood: "pleased" },
];
/* It lands on the character and the pigment every new agent starts with. */
const HOME = { key: "orb", color: PIGMENT.Ochre as string, mood: "idle" as Mood };
const EVERYONE = [...MEMBERS, HOME];
const WRITE = { first: 0.35, each: 0.42, morph: 0.3 };

function member(i: number) {
  return EVERYONE[((i % EVERYONE.length) + EVERYONE.length) % EVERYONE.length] as (typeof EVERYONE)[number];
}

export const shapeshifter: Direction = {
  key: "shapeshifter",
  name: "Shapeshifter",
  title: "One creature, the whole cast.",
  why:
    "It writes the name one letter per silhouette, circle to cloud, becoming each member of the " +
    "cast as it goes, and lands on the character and pigment every new agent starts with. " +
    "Condensed, it is that one creature. Point at it and it becomes somebody else: the corner " +
    "is never the same face two visits running, which is the whole claim the app makes about agents.",
  beats: [
    ["0.0", "A circle arrives and writes the G."],
    ["0.5", "It becomes an octagon for the u, a square for the a."],
    ["1.4", "A drop for the c, a cloud for the last a."],
    ["2.2", "It lands on the default agent, orb in ochre."],
    ["2.8", "It looks at you, pleased."],
    ["6.5", "The name folds away. Each visit of the pointer makes it someone else."],
  ],
  cast: [{ lump: lookupCharacter("orb"), color: HOME.color, seed: "shapeshifter" }],
  voice: "set",
  footprint: 24,
  at(t, f) {
    const launched = t >= 3.9;
    /* Which member it is becoming, and how far. The launch walks the five and
       lands home; after that, every visit of the pointer is one step on. */
    let from = 0;
    let to = 0;
    let u = 1;
    if (!launched) {
      for (let i = 1; i < EVERYONE.length; i++) {
        const at = WRITE.first + 0.2 + (i - 1) * WRITE.each;
        if (t >= at) {
          from = i - 1;
          to = i;
          u = ramp(t, at, WRITE.morph);
        }
      }
    } else {
      const home = EVERYONE.length - 1;
      from = home + Math.max(0, f.visits - 1);
      to = home + f.visits;
      u = f.visits === 0 ? 1 : ramp(t, f.hoverAt, 0.4);
    }
    const a = member(from);
    const b = member(to);
    let cue: Cue = {
      shown: t >= 0.05,
      mood: launched ? "idle" : t >= 2.8 ? "pleased" : b.mood,
      look: launched ? null : t >= 2.8 ? FACE : RIGHT,
      gesture: null,
      morph: {
        from: lookupCharacter(a.key),
        to: lookupCharacter(b.key),
        u,
        colors: [a.color, b.color],
      },
    };
    if (launched) cue = { ...rested(t, f, cue, 0, 1), morph: cue.morph };
    /* One letter per shape, each written as the shape before it settles. */
    let letters = 0;
    for (let i = 0; i < 5; i++) letters += ramp(t, WRITE.first + i * WRITE.each, 0.16);
    return { cues: [cue], name: launched ? settled(t, f) : letters / 5, says: null };
  },
};

export const DIRECTIONS: Direction[] = [pair, namesake, shapeshifter];
