/**
 * The cast.
 *
 * Five silhouettes, twenty-one characters. What tells two of them apart is,
 * first, which shape they are cut from, and then the resting lump on top of it:
 * how it leans, where its lobes are, and how its eyes are set. Nobody draws an
 * outline, so nobody can draw a bad one, and a new character is a row of
 * numbers rather than a path somebody has to get right.
 *
 * There was one shape before this and it was not enough. A rail of sixty round
 * creatures is a rail you read by color, because the only thing varying across
 * it was a few percent of stretch and where the eyes sat; the shape has to
 * carry some of the identity or the eyes carry all of it. So the outline is now
 * two decisions rather than one, and `silhouette.ts` holds the first.
 *
 * This replaced a cast of sixteen hand-drawn vegetables. The vegetables were
 * charming at six agents and childish at sixty, and every one of them was a
 * bezier somebody had to draw to a written spec that a test then had to check.
 * The spec is now the code: a character cannot leave its box, cannot sit at a
 * different weight, and cannot take light from a different direction, because
 * none of those are things a character supplies.
 *
 * Agents store the `key` and nothing else. No drawing is persisted, so any of
 * this can be redrawn without touching the database, and `ALIASES` keeps every
 * agent made by every earlier cast rendering as something deliberate.
 */

import type { Lump } from "./form";

export type Character = Lump;

export const CHARACTERS: Character[] = [
  {
    key: "orb",
    label: "Orb",
    form: "circle",
    ax: 1,
    ay: 1,
    sig: [{ k: 2, amp: 0.02, phase: 0.4 }],
    eye: { spread: 6.6, r: 2.9 },
  },
  {
    key: "egg",
    label: "Egg",
    form: "drop",
    ax: 0.98,
    ay: 1.01,
    sig: [{ k: 2, amp: 0.02, phase: 1.4 }],
    eye: { spread: 5.9, r: 2.8, y: 2.2 },
  },
  {
    key: "pebble",
    label: "Pebble",
    form: "circle",
    ax: 1.07,
    ay: 0.94,
    sig: [
      { k: 2, amp: 0.026, phase: 0.2 },
      { k: 3, amp: 0.018, phase: 1.1 },
    ],
    eye: { spread: 7.4, r: 2.8 },
  },
  {
    key: "drop",
    label: "Drop",
    form: "drop",
    ax: 1,
    ay: 1,
    sig: [{ k: 3, amp: 0.018, phase: 0.9 }],
    eye: { spread: 6.2, r: 2.9, y: 3 },
  },
  {
    key: "bean",
    label: "Bean",
    form: "cloud",
    ax: 1.01,
    ay: 0.99,
    sig: [{ k: 1, amp: 0.028, phase: 0.6 }],
    eye: { spread: 6.4, r: 2.8, x: -1.4, y: 1 },
  },
  {
    key: "lobe",
    label: "Lobe",
    form: "cloud",
    ax: 1.02,
    ay: 1,
    sig: [{ k: 2, amp: 0.022, phase: -1.57 }],
    eye: { spread: 6.4, r: 3, y: 1 },
  },
  {
    key: "puck",
    label: "Puck",
    form: "octagon",
    ax: 1.04,
    ay: 0.95,
    sig: [{ k: 2, amp: 0.022, phase: 1.57 }],
    eye: { spread: 7.2, r: 2.7 },
  },
  {
    key: "cell",
    label: "Cell",
    form: "circle",
    ax: 1.02,
    ay: 1.02,
    sig: [{ k: 4, amp: 0.03, phase: 0.8 }],
    eye: { spread: 0, r: 3.6, one: true },
  },
  {
    key: "knot",
    label: "Knot",
    form: "square",
    ax: 1,
    ay: 1.01,
    sig: [{ k: 2, amp: 0.02, phase: 1.2 }],
    eye: { spread: 5.6, r: 2.7 },
  },
  {
    key: "moon",
    label: "Moon",
    form: "circle",
    ax: 1.02,
    ay: 1,
    sig: [
      { k: 1, amp: 0.05, phase: 3.14 },
      { k: 3, amp: 0.014, phase: 0.5 },
    ],
    eye: { spread: 6.6, r: 2.9, x: 1.6 },
  },
  {
    key: "wave",
    label: "Wave",
    form: "cloud",
    ax: 1.03,
    ay: 0.98,
    sig: [{ k: 3, amp: 0.018, phase: 0 }],
    eye: { spread: 6.2, r: 2.8, y: 1 },
  },
  {
    key: "mote",
    label: "Mote",
    form: "octagon",
    ax: 1.02,
    ay: 0.97,
    sig: [{ k: 2, amp: 0.022, phase: 2 }],
    eye: { spread: 8, r: 2.2 },
  },
  {
    key: "bead",
    label: "Bead",
    form: "circle",
    ax: 1,
    ay: 1,
    sig: [{ k: 2, amp: 0.015, phase: 0.9 }],
    eye: { spread: 4.6, r: 2.3 },
  },
  {
    key: "gourd",
    label: "Gourd",
    form: "drop",
    ax: 0.99,
    ay: 1,
    sig: [{ k: 2, amp: 0.024, phase: 0.3 }],
    eye: { spread: 6, r: 2.9, y: 3.6 },
  },
  {
    key: "slab",
    label: "Slab",
    form: "square",
    ax: 1.04,
    ay: 0.96,
    sig: [],
    eye: { spread: 7, r: 3.3 },
  },
  {
    key: "pip",
    label: "Pip",
    form: "drop",
    ax: 0.99,
    ay: 1.02,
    sig: [{ k: 4, amp: 0.016, phase: 1 }],
    eye: { spread: 5.4, r: 2.5, y: 1.6 },
  },
  {
    key: "husk",
    label: "Husk",
    form: "octagon",
    ax: 1.03,
    ay: 0.98,
    sig: [{ k: 1, amp: 0.026, phase: 1.3 }],
    eye: { spread: 7, r: 2.6 },
  },
  {
    key: "loop",
    label: "Loop",
    form: "octagon",
    ax: 1.01,
    ay: 1.01,
    sig: [{ k: 2, amp: 0.024, phase: -0.6 }],
    eye: { spread: 0, r: 2.8, one: true },
  },
  {
    key: "crumb",
    label: "Crumb",
    form: "square",
    ax: 1.02,
    ay: 1,
    sig: [{ k: 1, amp: 0.03, phase: 0 }],
    eye: { spread: 6, r: 2.7, x: 1, y: 1.4 },
  },
  {
    key: "tide",
    label: "Tide",
    form: "cloud",
    ax: 0.98,
    ay: 1.02,
    sig: [{ k: 2, amp: 0.026, phase: 0 }],
    eye: { spread: 5.8, r: 3.1, y: 1 },
  },
  {
    key: "cinder",
    label: "Cinder",
    form: "square",
    ax: 1.01,
    ay: 0.99,
    sig: [{ k: 3, amp: 0.02, phase: 0.5 }],
    eye: { spread: 6.8, r: 2.4 },
  },
];

const BY_KEY = new Map(CHARACTERS.map((c) => [c.key, c]));

export const DEFAULT_CHARACTER = "orb";

/**
 * Keys from every earlier cast, so an existing agent keeps a deliberate face
 * rather than whatever a hash lands on. Four sets have shipped: sixteen
 * vegetables, an egg with props, a set of hand-drawn creatures, and emoji
 * before any of it. The mapping is by feel, since none of the old keys means
 * anything here.
 *
 * Exported so `catalog.test.ts` can hold the table to the list of keys that
 * have actually reached a database. The fallback hash would answer for all of
 * them, and would re-roll every existing agent's face on the day the cast
 * changed size, which is the one thing this table is for.
 */
export const ALIASES: Record<string, string> = {
  // the vegetables
  avocado: "orb",
  lime: "pebble",
  tomato: "lobe",
  onion: "gourd",
  garlic: "bead",
  chilli: "crumb",
  cilantro: "wave",
  salt: "slab",
  corn: "egg",
  pepper: "drop",
  radish: "pip",
  carrot: "tide",
  mushroom: "puck",
  squash: "husk",
  eggplant: "knot",
  chip: "cinder",
  pit: "cell",
  mill: "mote",
  molcajete: "slab",
  jar: "tide",
  spoon: "loop",
  // the egg with props
  plain: "orb",
  cheerful: "lobe",
  curious: "pebble",
  wink: "drop",
  sleepy: "cell",
  stern: "mote",
  bright: "egg",
  blank: "gourd",
  cat: "puck",
  tophat: "mote",
  cap: "cinder",
  crown: "drop",
  bowtie: "cinder",
  necktie: "mote",
  scarf: "wave",
  glasses: "puck",
  monocle: "cell",
  headphones: "slab",
  antenna: "tide",
  sprout: "wave",
  // the creatures
  fox: "tide",
  owl: "puck",
  crab: "lobe",
  bird: "egg",
  bug: "pip",
  slime: "gourd",
  bot: "slab",
  gear: "mote",
  ghost: "bead",
  star: "egg",
  cloud: "wave",
  // the emoji before any of it
  robot: "slab",
  brain: "puck",
  penguin: "cell",
  butterfly: "wave",
  bee: "egg",
  rocket: "tide",
  sun: "lobe",
  taco: "cinder",
  octopus: "puck",
  frog: "drop",
  snail: "gourd",
  comet: "cinder",
  fire: "crumb",
  bolt: "cinder",
  satellite: "mote",
};

function hashToIndex(key: string): number {
  let hash = 0;
  for (let i = 0; i < key.length; i++) hash = (hash * 31 + key.charCodeAt(i)) >>> 0;
  return hash % CHARACTERS.length;
}

/**
 * Never returns undefined. An agent saved by any past or future build still has
 * to render today, so an unknown key gets a stable stand-in.
 */
export function lookupCharacter(key: string): Character {
  const direct = BY_KEY.get(key);
  if (direct) return direct;

  const aliased = ALIASES[key];
  if (aliased) {
    const found = BY_KEY.get(aliased);
    if (found) return found;
  }
  return CHARACTERS[hashToIndex(key)] ?? (BY_KEY.get(DEFAULT_CHARACTER) as Character);
}

/**
 * Accents. Pigments rather than candy: saturated primaries are half of what
 * reads as childish, and a mid-luminance body keeps the eyes as the darkest
 * thing on the figure, which is where the expression is. Nothing is mixed from
 * these any more, since the body has no rim to darken and no gradient to light,
 * so the value picked here is the value on the screen.
 */
export const ACCENTS: { name: string; value: string }[] = [
  { name: "Ochre", value: "#c28c31" },
  { name: "Terracotta", value: "#bf5f3c" },
  { name: "Madder", value: "#a8453a" },
  { name: "Clay", value: "#b3805c" },
  { name: "Olive", value: "#8b8f45" },
  { name: "Moss", value: "#5e8158" },
  { name: "Verdigris", value: "#4d8b83" },
  { name: "Slate", value: "#5a7d99" },
  { name: "Indigo", value: "#4f5f96" },
  { name: "Plum", value: "#7c5a8c" },
  { name: "Rose", value: "#b26f86" },
  { name: "Graphite", value: "#6c6f70" },
];

export const DEFAULT_ACCENT = ACCENTS[0]?.value as string;

/** Picks the accent least used so far, so a new crew is visually separable. */
export function suggestAccent(taken: string[]): string {
  const counts = new Map(ACCENTS.map((a) => [a.value, 0]));
  for (const color of taken) {
    const key = color.toLowerCase();
    if (counts.has(key)) counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  let best = DEFAULT_ACCENT;
  let lowest = Number.POSITIVE_INFINITY;
  for (const [value, count] of counts) {
    if (count < lowest) {
      lowest = count;
      best = value;
    }
  }
  return best;
}

/** Picks an unused character so two new agents do not look identical. */
export function suggestCharacter(taken: string[]): string {
  const used = new Set(taken);
  return (CHARACTERS.find((c) => !used.has(c.key)) ?? (CHARACTERS[0] as Character)).key;
}
