import type { ReactNode } from "react";

/**
 * Agent avatars.
 *
 * Every agent is the same egg. Personality comes from three swappable parts —
 * eyes, mouth, accessory — over one shared silhouette, which is what makes a
 * crew read as a set instead of a bag of unrelated drawings. The previous pass
 * gave each agent its own outline and the sidebar looked like clip art.
 *
 * Agents store the preset `key`. No drawing is persisted, so any of this can be
 * redrawn without touching the database.
 */

const FILL = "var(--egg-fill)";
const SHADE = "var(--egg-shade)";
const DEEP = "var(--egg-deep)";
const LIGHT = "var(--egg-light)";
const INK = "var(--egg-ink)";
const WHITE = "var(--egg-white)";

/** Shared geometry. Every part is positioned against these. */
export const EGG = {
  eyeY: 31,
  eyeLeft: 24.5,
  eyeRight: 39.5,
  mouthY: 42,
  headTop: 10,
  neckY: 50,
} as const;

// ---- eyes -----------------------------------------------------------------

/** Wrapped so the whole eye squashes together when it blinks. */
function eyeball(x: number, r: number, pupil: number) {
  return (
    <>
      <circle cx={x} cy={EGG.eyeY} r={r} fill={WHITE} />
      <circle className="egg__pupil" cx={x} cy={EGG.eyeY} r={pupil} fill={INK} />
      <circle cx={x - r * 0.34} cy={EGG.eyeY - r * 0.38} r={r * 0.22} fill="#fff" opacity="0.95" />
    </>
  );
}

/** A closed, happy arc. */
function arc(x: number) {
  return (
    <path
      d={`M${x - 4.4} ${EGG.eyeY + 1.6}q4.4-6 8.8 0`}
      stroke={INK}
      strokeWidth="2.2"
      fill="none"
      strokeLinecap="round"
    />
  );
}

const EYES: Record<string, ReactNode> = {
  round: (
    <>
      <g className="egg__eye">{eyeball(EGG.eyeLeft, 5, 2.7)}</g>
      <g className="egg__eye">{eyeball(EGG.eyeRight, 5, 2.7)}</g>
    </>
  ),
  wide: (
    <>
      <g className="egg__eye">{eyeball(EGG.eyeLeft, 6.3, 3.3)}</g>
      <g className="egg__eye">{eyeball(EGG.eyeRight, 6.3, 3.3)}</g>
    </>
  ),
  dot: (
    <>
      <g className="egg__eye">
        <circle cx={EGG.eyeLeft} cy={EGG.eyeY} r="2.9" fill={INK} />
      </g>
      <g className="egg__eye">
        <circle cx={EGG.eyeRight} cy={EGG.eyeY} r="2.9" fill={INK} />
      </g>
    </>
  ),
  tall: (
    <>
      <g className="egg__eye">
        <ellipse cx={EGG.eyeLeft} cy={EGG.eyeY} rx="3.9" ry="5.6" fill={WHITE} />
        <ellipse className="egg__pupil" cx={EGG.eyeLeft} cy={EGG.eyeY} rx="2.1" ry="3" fill={INK} />
      </g>
      <g className="egg__eye">
        <ellipse cx={EGG.eyeRight} cy={EGG.eyeY} rx="3.9" ry="5.6" fill={WHITE} />
        <ellipse
          className="egg__pupil"
          cx={EGG.eyeRight}
          cy={EGG.eyeY}
          rx="2.1"
          ry="3"
          fill={INK}
        />
      </g>
    </>
  ),
  happy: (
    <>
      {arc(EGG.eyeLeft)}
      {arc(EGG.eyeRight)}
    </>
  ),
  wink: (
    <>
      <g className="egg__eye">{eyeball(EGG.eyeLeft, 5, 2.7)}</g>
      {arc(EGG.eyeRight)}
    </>
  ),
  sleepy: (
    <>
      <g className="egg__eye">
        {eyeball(EGG.eyeLeft, 5, 2.7)}
        <path d={`M${EGG.eyeLeft - 5.4} ${EGG.eyeY - 0.6}a5.4 5.4 0 0110.8 0z`} fill={FILL} />
      </g>
      <g className="egg__eye">
        {eyeball(EGG.eyeRight, 5, 2.7)}
        <path d={`M${EGG.eyeRight - 5.4} ${EGG.eyeY - 0.6}a5.4 5.4 0 0110.8 0z`} fill={FILL} />
      </g>
    </>
  ),
  sparkle: (
    <>
      <g className="egg__eye">
        {eyeball(EGG.eyeLeft, 5.6, 3)}
        <path
          d={`M${EGG.eyeLeft + 2.6} ${EGG.eyeY - 4.2}l0.7 1.6 1.6 0.7-1.6 0.7-0.7 1.6-0.7-1.6-1.6-0.7 1.6-0.7z`}
          fill="#fff"
        />
      </g>
      <g className="egg__eye">{eyeball(EGG.eyeRight, 5.6, 3)}</g>
    </>
  ),
  stern: (
    <>
      <g className="egg__eye">{eyeball(EGG.eyeLeft, 4.6, 2.5)}</g>
      <g className="egg__eye">{eyeball(EGG.eyeRight, 4.6, 2.5)}</g>
      <path
        d={`M${EGG.eyeLeft - 5} ${EGG.eyeY - 7}l9 2.4M${EGG.eyeRight + 5} ${EGG.eyeY - 7}l-9 2.4`}
        stroke={INK}
        strokeWidth="1.9"
        strokeLinecap="round"
      />
    </>
  ),
};

// ---- mouths ---------------------------------------------------------------

const stroke = {
  stroke: INK,
  strokeWidth: 2.1,
  fill: "none",
  strokeLinecap: "round",
} as const;

const MOUTHS: Record<string, ReactNode> = {
  smile: <path d={`M26 ${EGG.mouthY - 2}q6 6 12 0`} {...stroke} />,
  grin: <path d={`M25 ${EGG.mouthY - 3}h14a7 7 0 01-14 0z`} fill={INK} />,
  flat: <path d={`M27 ${EGG.mouthY}h10`} {...stroke} />,
  smirk: <path d={`M27 ${EGG.mouthY}q5 4 9-2`} {...stroke} />,
  o: <ellipse cx="32" cy={EGG.mouthY} rx="3" ry="3.6" fill={INK} />,
  cat: <path d={`M26 ${EGG.mouthY - 2}q3 3.4 6 0q3 3.4 6 0`} {...stroke} strokeWidth={1.9} />,
  tiny: <path d={`M29.6 ${EGG.mouthY}q2.4 2.4 4.8 0`} {...stroke} strokeWidth={1.9} />,
  wobble: <path d={`M26 ${EGG.mouthY}q3-3 6 0q3 3 6 0`} {...stroke} strokeWidth={1.9} />,
};

// ---- accessories ----------------------------------------------------------

/** Drawn over the egg. Hats sit on the head, ties at the neck. */
const ACCESSORIES: Record<string, ReactNode> = {
  none: null,
  tophat: (
    <>
      <rect x="22" y="1.5" width="20" height="11" rx="1.8" fill={DEEP} />
      <rect x="22" y="8" width="20" height="3.4" fill={SHADE} />
      <rect x="16" y="11.5" width="32" height="3.2" rx="1.6" fill={DEEP} />
    </>
  ),
  cap: (
    <>
      <path d="M20 13a12 12 0 0124 0z" fill={DEEP} />
      <path d="M43 10h9a2.6 2.6 0 010 5.2h-9z" fill={SHADE} />
      <circle cx="32" cy="2.4" r="1.9" fill={SHADE} />
    </>
  ),
  crown: (
    <path
      d="M18 13l1.6-9.5 5.6 4.6L32 1l6.8 7.1 5.6-4.6L46 13z"
      fill={SHADE}
      stroke={DEEP}
      strokeWidth="1.2"
      strokeLinejoin="round"
    />
  ),
  sprout: (
    <>
      <path d={`M32 ${EGG.headTop}V2.5`} stroke={DEEP} strokeWidth="2.2" strokeLinecap="round" />
      <path d="M32 6.5c-7-1-9.5-4.5-9.5-4.5s4.5-2.5 9.5 4.5z" fill={DEEP} />
      <path d="M32 9.5c6-1.6 8.5-5 8.5-5s-3 7.6-8.5 5z" fill={SHADE} />
    </>
  ),
  antenna: (
    <>
      <path d="M32 9V4" stroke={DEEP} strokeWidth="2.2" strokeLinecap="round" />
      <circle className="egg__blip" cx="32" cy="2.6" r="2.8" fill={SHADE} />
    </>
  ),
  headphones: (
    <>
      <path
        d="M15 33v-6a17 17 0 0134 0v6"
        stroke={DEEP}
        strokeWidth="3.2"
        fill="none"
        strokeLinecap="round"
      />
      <rect x="10.5" y="29" width="8" height="12" rx="4" fill={SHADE} />
      <rect x="45.5" y="29" width="8" height="12" rx="4" fill={SHADE} />
    </>
  ),
  glasses: (
    <>
      <circle cx={EGG.eyeLeft} cy={EGG.eyeY} r="7.4" fill="none" stroke={DEEP} strokeWidth="1.9" />
      <circle cx={EGG.eyeRight} cy={EGG.eyeY} r="7.4" fill="none" stroke={DEEP} strokeWidth="1.9" />
      <path
        d={`M${EGG.eyeLeft + 7.4} ${EGG.eyeY}h${EGG.eyeRight - EGG.eyeLeft - 14.8}`}
        stroke={DEEP}
        strokeWidth="1.9"
      />
    </>
  ),
  monocle: (
    <>
      <circle cx={EGG.eyeRight} cy={EGG.eyeY} r="7.4" fill="none" stroke={DEEP} strokeWidth="1.9" />
      <path
        d={`M${EGG.eyeRight} ${EGG.eyeY + 7.4}v5`}
        stroke={DEEP}
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </>
  ),
  bowtie: (
    <>
      <path d={`M32 ${EGG.neckY}l-8-4v8z`} fill={SHADE} />
      <path d={`M32 ${EGG.neckY}l8-4v8z`} fill={SHADE} />
      <circle cx="32" cy={EGG.neckY} r="2.2" fill={DEEP} />
    </>
  ),
  necktie: (
    <>
      <path d={`M28.6 ${EGG.neckY - 4}h6.8l-1.8 3.4h-3.2z`} fill={DEEP} />
      <path d={`M30.2 ${EGG.neckY}h3.6l1.4 7-3.2 3.4-3.2-3.4z`} fill={SHADE} />
    </>
  ),
  scarf: (
    <>
      <path d={`M20 ${EGG.neckY - 1}q12 5 24 0v5q-12 5-24 0z`} fill={SHADE} />
      <path d={`M40 ${EGG.neckY + 3}l5 8-4.5 1.4-3-7z`} fill={DEEP} />
    </>
  ),
};

// ---- presets --------------------------------------------------------------

export type EggGroup = "Faces" | "Dressed" | "Gear";

export interface EggPreset {
  key: string;
  label: string;
  group: EggGroup;
  eyes: keyof typeof EYES;
  mouth: keyof typeof MOUTHS;
  accessory: keyof typeof ACCESSORIES;
}

export const EGGS: EggPreset[] = [
  {
    key: "plain",
    label: "Plain",
    group: "Faces",
    eyes: "round",
    mouth: "smile",
    accessory: "none",
  },
  {
    key: "cheerful",
    label: "Cheerful",
    group: "Faces",
    eyes: "happy",
    mouth: "grin",
    accessory: "none",
  },
  { key: "curious", label: "Curious", group: "Faces", eyes: "wide", mouth: "o", accessory: "none" },
  { key: "wink", label: "Wink", group: "Faces", eyes: "wink", mouth: "smirk", accessory: "none" },
  {
    key: "sleepy",
    label: "Sleepy",
    group: "Faces",
    eyes: "sleepy",
    mouth: "tiny",
    accessory: "none",
  },
  { key: "stern", label: "Stern", group: "Faces", eyes: "stern", mouth: "flat", accessory: "none" },
  {
    key: "bright",
    label: "Bright",
    group: "Faces",
    eyes: "sparkle",
    mouth: "grin",
    accessory: "none",
  },
  { key: "blank", label: "Blank", group: "Faces", eyes: "dot", mouth: "wobble", accessory: "none" },
  { key: "cat", label: "Whiskers", group: "Faces", eyes: "round", mouth: "cat", accessory: "none" },

  {
    key: "tophat",
    label: "Top hat",
    group: "Dressed",
    eyes: "round",
    mouth: "smile",
    accessory: "tophat",
  },
  { key: "cap", label: "Cap", group: "Dressed", eyes: "round", mouth: "smirk", accessory: "cap" },
  {
    key: "crown",
    label: "Crown",
    group: "Dressed",
    eyes: "wide",
    mouth: "smile",
    accessory: "crown",
  },
  {
    key: "bowtie",
    label: "Bow tie",
    group: "Dressed",
    eyes: "round",
    mouth: "grin",
    accessory: "bowtie",
  },
  {
    key: "necktie",
    label: "Necktie",
    group: "Dressed",
    eyes: "stern",
    mouth: "flat",
    accessory: "necktie",
  },
  {
    key: "scarf",
    label: "Scarf",
    group: "Dressed",
    eyes: "happy",
    mouth: "smile",
    accessory: "scarf",
  },

  {
    key: "glasses",
    label: "Glasses",
    group: "Gear",
    eyes: "round",
    mouth: "tiny",
    accessory: "glasses",
  },
  {
    key: "monocle",
    label: "Monocle",
    group: "Gear",
    eyes: "round",
    mouth: "smirk",
    accessory: "monocle",
  },
  {
    key: "headphones",
    label: "Headphones",
    group: "Gear",
    eyes: "round",
    mouth: "cat",
    accessory: "headphones",
  },
  {
    key: "antenna",
    label: "Antenna",
    group: "Gear",
    eyes: "tall",
    mouth: "flat",
    accessory: "antenna",
  },
  {
    key: "sprout",
    label: "Sprout",
    group: "Gear",
    eyes: "happy",
    mouth: "tiny",
    accessory: "sprout",
  },
];

export const EGG_GROUPS: EggGroup[] = ["Faces", "Dressed", "Gear"];

const BY_KEY = new Map(EGGS.map((e) => [e.key, e]));

export const DEFAULT_EGG = "plain";

/** Keys from earlier avatar sets, so existing agents keep a sensible face. */
const ALIASES: Record<string, string> = {
  // hand-drawn creature set
  avocado: "plain",
  chilli: "cheerful",
  onion: "blank",
  bean: "sleepy",
  fox: "curious",
  owl: "glasses",
  crab: "stern",
  bird: "bright",
  bug: "antenna",
  slime: "blank",
  bot: "headphones",
  gear: "antenna",
  ghost: "blank",
  moon: "sleepy",
  star: "bright",
  cloud: "plain",
  // original emoji set
  robot: "headphones",
  brain: "glasses",
  penguin: "bowtie",
  butterfly: "sprout",
  bee: "antenna",
  rocket: "bright",
  sun: "cheerful",
  tomato: "cheerful",
  garlic: "blank",
  lime: "sleepy",
  corn: "sprout",
  salt: "plain",
  taco: "curious",
  pepper: "cheerful",
  octopus: "curious",
  frog: "wink",
  snail: "sleepy",
  comet: "bright",
  fire: "cheerful",
  bolt: "bright",
  satellite: "antenna",
};

function hashToIndex(key: string): number {
  let hash = 0;
  for (let i = 0; i < key.length; i++) hash = (hash * 31 + key.charCodeAt(i)) >>> 0;
  return hash % EGGS.length;
}

/**
 * Never returns undefined. An agent saved by any past or future build still has
 * to render today, so an unknown key gets a stable stand-in.
 */
export function lookupEgg(key: string): EggPreset {
  const direct = BY_KEY.get(key);
  if (direct) return direct;

  const aliased = ALIASES[key];
  if (aliased) {
    const found = BY_KEY.get(aliased);
    if (found) return found;
  }
  return EGGS[hashToIndex(key)] ?? BY_KEY.get(DEFAULT_EGG)!;
}

export function eggParts(preset: EggPreset) {
  return {
    eyes: EYES[preset.eyes],
    mouth: MOUTHS[preset.mouth],
    accessory: ACCESSORIES[preset.accessory],
  };
}

/** The shared silhouette. Identical for every agent; only the colour changes. */
export function EggBody() {
  return (
    <>
      <path
        d="M32 5c-11.6 0-20.5 13.4-20.5 27.6C11.5 45.4 20.6 55 32 55s20.5-9.6 20.5-22.4C52.5 18.4 43.6 5 32 5z"
        fill={FILL}
      />
      {/* A soft highlight gives the shell a little volume without an outline. */}
      <ellipse cx="24" cy="21" rx="7" ry="9" fill={LIGHT} opacity="0.28" />
    </>
  );
}

/**
 * Accent colours. Drawn from the same palette as the app chrome so a random
 * pick never clashes with the surface behind it.
 */
export const ACCENTS: { name: string; value: string }[] = [
  { name: "Flesh", value: "#c7d96b" },
  { name: "Rind", value: "#6faa5c" },
  { name: "Pit", value: "#b0784a" },
  { name: "Mint", value: "#7fd1a3" },
  { name: "Chilli", value: "#e2674a" },
  { name: "Tomato", value: "#d9534f" },
  { name: "Corn", value: "#e8b84b" },
  { name: "Sky", value: "#6aa9d9" },
  { name: "Iris", value: "#9b8ad4" },
  { name: "Rose", value: "#d97ea8" },
  { name: "Clay", value: "#c2926b" },
  { name: "Slate", value: "#8aa0a6" },
];

export const DEFAULT_ACCENT = ACCENTS[0]!.value;

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

/** Picks an unused preset so two new agents do not look identical. */
export function suggestEgg(taken: string[]): string {
  const used = new Set(taken);
  return (EGGS.find((e) => !used.has(e.key)) ?? EGGS[0]!).key;
}
