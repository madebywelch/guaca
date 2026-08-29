import { describe, expect, it } from "vitest";

import {
  ACCENTS,
  ALIASES,
  CHARACTERS,
  DEFAULT_ACCENT,
  DEFAULT_CHARACTER,
  lookupCharacter,
  suggestAccent,
  suggestCharacter,
} from "./catalog";
import { bodyPoints, FORM } from "./form";
import { CREST, SILHOUETTES, type Silhouette } from "./silhouette";

/**
 * Keys that have been written into somebody's database by a shipped build.
 * None of them means anything to the current cast, so each has to be mapped by
 * hand or an existing crew is re-rolled by a hash the day this ships.
 */
const SHIPPED = [
  // the vegetables
  "avocado",
  "lime",
  "tomato",
  "onion",
  "garlic",
  "chilli",
  "cilantro",
  "salt",
  "corn",
  "pepper",
  "radish",
  "carrot",
  "mushroom",
  "squash",
  "eggplant",
  "chip",
  "pit",
  "mill",
  "molcajete",
  "jar",
  "spoon",
  // the egg with props
  "plain",
  "cheerful",
  "curious",
  "wink",
  "sleepy",
  "stern",
  "bright",
  "blank",
  "cat",
  "tophat",
  "cap",
  "crown",
  "bowtie",
  "necktie",
  "scarf",
  "glasses",
  "monocle",
  "headphones",
  "antenna",
  "sprout",
  // the creatures
  "bean",
  "fox",
  "owl",
  "crab",
  "bird",
  "bug",
  "slime",
  "bot",
  "gear",
  "ghost",
  "moon",
  "star",
  "cloud",
  // the emoji before any of it
  "robot",
  "brain",
  "penguin",
  "butterfly",
  "bee",
  "rocket",
  "sun",
  "taco",
  "octopus",
  "frog",
  "snail",
  "comet",
  "fire",
  "bolt",
  "satellite",
];

/** How far past its silhouette a character's own stretch and lobes may push. */
const LUMP = 0.05;

describe("the cast", () => {
  it("has a unique key and label for every character", () => {
    expect(new Set(CHARACTERS.map((c) => c.key)).size).toBe(CHARACTERS.length);
    expect(new Set(CHARACTERS.map((c) => c.label)).size).toBe(CHARACTERS.length);
  });

  it("draws something for a key from any build, past or future", () => {
    for (const key of [...SHIPPED, "", "not-a-character", "🙂"]) {
      expect(lookupCharacter(key), key).toBeDefined();
    }
    expect(lookupCharacter(DEFAULT_CHARACTER).key).toBe(DEFAULT_CHARACTER);
  });

  // A hash would answer too, and would re-roll every existing agent's face on
  // the day the cast changed size. The point of the alias table is that an
  // operator's crew looks the same tomorrow as it did today.
  it("maps every key that has ever shipped by hand", () => {
    const rolled = SHIPPED.filter(
      (key) => !CHARACTERS.some((c) => c.key === key) && ALIASES[key] === undefined,
    );
    expect(rolled, "these would fall through to the hash").toEqual([]);
    for (const [old, now] of Object.entries(ALIASES)) {
      expect(
        CHARACTERS.some((c) => c.key === now),
        `${old} points nowhere`,
      ).toBe(true);
    }
  });

  /* Shape carries some of an identity now and the eyes carry the rest, so two
     characters cut from the same silhouette may not also wear the same pair. */
  it("gives every character a distinguishable set of eyes", () => {
    const seen = CHARACTERS.map((c) =>
      [c.eye.one ? 1 : 2, c.eye.spread, c.eye.r, c.eye.x ?? 0, c.eye.y ?? 0].join(":"),
    );
    expect(new Set(seen).size, "two characters wear the same face").toBe(CHARACTERS.length);
  });

  /* Five shapes are five things to keep working, and one nobody is cut from is
     one that could break with nothing on screen to show it. */
  it("uses every silhouette there is", () => {
    const cut = new Set(CHARACTERS.map((c) => c.form));
    for (const key of Object.keys(SILHOUETTES) as Silhouette[]) {
      expect(cut.has(key), `nothing in the cast is a ${key}`).toBe(true);
    }
  });

  it("keeps every character to the silhouette it was cut from", () => {
    for (const c of CHARACTERS) {
      // A lump varies a shape, it does not replace one: a square stretched past
      // this is a brick, which is a sixth silhouette nobody declared.
      expect(Math.abs(c.ax - 1), c.key).toBeLessThanOrEqual(0.12);
      expect(Math.abs(c.ay - 1), c.key).toBeLessThanOrEqual(0.12);
      const lumpy = c.sig.reduce((sum, lobe) => sum + Math.abs(lobe.amp), 0);
      expect(lumpy, c.key).toBeLessThanOrEqual(0.09);
    }
  });

  /*
   * The one bound that is arithmetic rather than taste. `CREST` is what a shape
   * may rest at and `LUMP` is what the character on top of it may add, so this
   * is the only place the two are read together. Everything between here and
   * `FORM.reach` belongs to the moods, and `form.test.ts` is where they spend
   * it: a character over this line does not fail here, it fails there, in one
   * frame of one mood, months later.
   */
  it("leaves the moods the room they need", () => {
    for (const c of CHARACTERS) {
      let most = 0;
      for (const [x, y] of bodyPoints(c, {}, 0).pts) {
        most = Math.max(most, Math.hypot(x - FORM.center, y - FORM.center));
      }
      expect(most / FORM.radius, `${c.key} rests too far out`).toBeLessThanOrEqual(CREST + LUMP);
    }
  });

  /* Against the character's own silhouette rather than against a circle: a
     drop is narrower at the sides than at the top, so a pair of eyes that fits
     one shape is not a pair that fits all five. */
  it("seats both eyes inside the body", () => {
    for (const c of CHARACTERS) {
      const out = Math.abs(c.eye.x ?? 0) + c.eye.spread + c.eye.r * (c.eye.one ? 1.5 : 1);
      const down = c.eye.y ?? 0;
      const edge = SILHOUETTES[c.form](Math.atan2(down, out)) * FORM.radius;
      expect(Math.hypot(out, down), c.key).toBeLessThan(edge * 0.8);
    }
  });
});

describe("accents", () => {
  it("has one value per name, written the same way everywhere", () => {
    expect(new Set(ACCENTS.map((a) => a.value)).size).toBe(ACCENTS.length);
    for (const accent of ACCENTS) expect(accent.value).toMatch(/^#[0-9a-f]{6}$/);
    expect(ACCENTS.some((a) => a.value === DEFAULT_ACCENT)).toBe(true);
  });

  it("hands out what is least used, and keeps going once everything is", () => {
    expect(suggestAccent([])).toBe(DEFAULT_ACCENT);
    const first = suggestAccent([]);
    expect(suggestAccent([first])).not.toBe(first);
    expect(ACCENTS.some((a) => a.value === suggestAccent(ACCENTS.map((a) => a.value)))).toBe(true);
  });

  it("hands out an unused character until there are none", () => {
    expect(suggestCharacter([])).toBe(CHARACTERS[0]?.key);
    expect(suggestCharacter([CHARACTERS[0]?.key ?? ""])).toBe(CHARACTERS[1]?.key);
    const all = CHARACTERS.map((c) => c.key);
    expect(all).toContain(suggestCharacter(all));
  });
});
