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
import { FORM } from "./form";

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

  /* One species means the eyes carry as much of an identity as the outline
     does, so no two characters may have the same pair in the same place. */
  it("gives every character a distinguishable set of eyes", () => {
    const seen = CHARACTERS.map((c) =>
      [c.eye.one ? 1 : 2, c.eye.spread, c.eye.r, c.eye.x ?? 0, c.eye.y ?? 0].join(":"),
    );
    expect(new Set(seen).size, "two characters wear the same face").toBe(CHARACTERS.length);
  });

  it("keeps every character round", () => {
    for (const c of CHARACTERS) {
      // The species is a ball. A lump that stretched past this would read as a
      // second kind of creature standing in the same rail.
      expect(Math.abs(c.ax - 1), c.key).toBeLessThanOrEqual(0.12);
      expect(Math.abs(c.ay - 1), c.key).toBeLessThanOrEqual(0.12);
      const lumpy = c.sig.reduce((sum, lobe) => sum + Math.abs(lobe.amp), 0);
      expect(lumpy, c.key).toBeLessThanOrEqual(0.09);
    }
  });

  it("seats both eyes inside the body", () => {
    for (const c of CHARACTERS) {
      const reach = Math.abs(c.eye.x ?? 0) + c.eye.spread + c.eye.r * (c.eye.one ? 1.5 : 1);
      expect(Math.hypot(reach, Math.abs(c.eye.y ?? 0)), c.key).toBeLessThan(FORM.radius * 0.75);
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
