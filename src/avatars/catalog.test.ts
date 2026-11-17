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

const SHIPPED = [
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

  it("gives every character a distinguishable set of eyes", () => {
    const seen = CHARACTERS.map((c) =>
      [c.eye.one ? 1 : 2, c.eye.spread, c.eye.r, c.eye.x ?? 0, c.eye.y ?? 0].join(":"),
    );
    expect(new Set(seen).size, "two characters wear the same face").toBe(CHARACTERS.length);
  });

  it("uses every silhouette there is", () => {
    const cut = new Set(CHARACTERS.map((c) => c.form));
    for (const key of Object.keys(SILHOUETTES) as Silhouette[]) {
      expect(cut.has(key), `nothing in the cast is a ${key}`).toBe(true);
    }
  });

  it("keeps every character to the silhouette it was cut from", () => {
    for (const c of CHARACTERS) {
      expect(Math.abs(c.ax - 1), c.key).toBeLessThanOrEqual(0.12);
      expect(Math.abs(c.ay - 1), c.key).toBeLessThanOrEqual(0.12);
      const lumpy = c.sig.reduce((sum, lobe) => sum + Math.abs(lobe.amp), 0);
      expect(lumpy, c.key).toBeLessThanOrEqual(0.09);
    }
  });

  it("leaves the moods the room they need", () => {
    for (const c of CHARACTERS) {
      let most = 0;
      for (const [x, y] of bodyPoints(c, {}, 0).pts) {
        most = Math.max(most, Math.hypot(x - FORM.center, y - FORM.center));
      }
      expect(most / FORM.radius, `${c.key} rests too far out`).toBeLessThanOrEqual(CREST + LUMP);
    }
  });

  it("seats both eyes inside the body", () => {
    for (const c of CHARACTERS) {
      const out = Math.abs(c.eye.x ?? 0) + c.eye.spread + c.eye.r;
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
