import { describe, expect, it } from "vitest";

import { pathBounds } from "./bounds";
import {
  ACCENTS,
  CHARACTER_GROUPS,
  CHARACTERS,
  DEFAULT_CHARACTER,
  FACE,
  type Face,
  FORM,
  lookupCharacter,
  suggestAccent,
  suggestCharacter,
} from "./catalog";

/**
 * How far a face reaches, without drawing it. Derived from the same constants
 * the drawing uses, so a change to one is caught here rather than by eye.
 */
function faceBounds(face: Face) {
  const cx = face.cx ?? FORM.centreX;
  const reach = face.spread + face.r;
  return {
    x0: cx - reach,
    x1: cx + reach,
    y0: face.y - face.r * FACE.tallest,
    y1: face.y + face.r * (FACE.drop + FACE.width),
  };
}

describe("the construction spec", () => {
  // The whole premise of the set is that a silhouette carries the identity. That
  // only works if the silhouettes agree about how much room they take, so these
  // are the numbers that stop a new ingredient from being drawn freehand.
  it.each(CHARACTERS.map((c) => [c.key, c] as const))("%s stays inside the box", (_key, c) => {
    const b = pathBounds(c.body.d);
    expect(b.x0).toBeGreaterThanOrEqual(FORM.left);
    expect(b.x1).toBeLessThanOrEqual(FORM.right);
    expect(b.y0).toBeGreaterThanOrEqual(FORM.top);
    expect(b.y1).toBeLessThanOrEqual(FORM.bottom);
  });

  it.each(CHARACTERS.map((c) => [c.key, c] as const))("%s carries one weight", (_key, c) => {
    const b = pathBounds(c.body.d);
    // Wide and narrow ingredients are both allowed; a row of them sitting at
    // visibly different sizes is not. An earlier pass ran from 760 to 1684.
    expect(b.x1 - b.x0).toBeGreaterThanOrEqual(24);
    expect(b.x1 - b.x0).toBeLessThanOrEqual(40);
    expect(b.y1 - b.y0).toBeGreaterThanOrEqual(34);
    expect(b.y1 - b.y0).toBeLessThanOrEqual(47);
  });

  it.each(CHARACTERS.map((c) => [c.key, c] as const))("%s sits on the baseline", (_key, c) => {
    const b = pathBounds(c.body.d);
    expect(Math.abs((b.x0 + b.x1) / 2 - FORM.centreX)).toBeLessThanOrEqual(1.5);
    expect(Math.abs((b.y0 + b.y1) / 2 - FORM.centreY)).toBeLessThanOrEqual(3);
  });

  it.each(CHARACTERS.map((c) => [c.key, c] as const))("%s keeps its face on", (_key, c) => {
    // A face hanging off the rim is the single most visible way to break the
    // set, and it is invisible in a diff.
    const body = pathBounds(c.body.d);
    const face = faceBounds(c.face);
    expect(face.x0).toBeGreaterThan(body.x0);
    expect(face.x1).toBeLessThan(body.x1);
    expect(face.y0).toBeGreaterThan(body.y0);
    expect(face.y1).toBeLessThan(body.y1);
  });

  it("draws every silhouette with the shared sheen", () => {
    // One light, from one direction. A body without it reads as a flat region.
    for (const c of CHARACTERS) {
      expect(c.body.sheen, `${c.key} sheen`).toBeTruthy();
    }
  });
});

describe("the cast", () => {
  it("has unique keys", () => {
    const keys = CHARACTERS.map((c) => c.key);
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("gives every character its own silhouette", () => {
    // Two agents sharing an outline is the failure this whole set exists to fix.
    const shapes = CHARACTERS.map((c) => c.body.d);
    expect(new Set(shapes).size).toBe(shapes.length);
  });

  it("only uses groups that are listed", () => {
    for (const c of CHARACTERS) {
      expect(CHARACTER_GROUPS).toContain(c.group);
    }
  });

  it("gives every group something to choose from", () => {
    for (const group of CHARACTER_GROUPS) {
      expect(CHARACTERS.filter((c) => c.group === group).length).toBeGreaterThanOrEqual(3);
    }
  });

  it("keeps the default resolvable", () => {
    expect(lookupCharacter(DEFAULT_CHARACTER).key).toBe(DEFAULT_CHARACTER);
  });
});

describe("lookup", () => {
  it("falls back rather than returning undefined for an unknown key", () => {
    const found = lookupCharacter("not-a-real-key");
    expect(found).toBeDefined();
    expect(CHARACTERS).toContain(found);
  });

  it("is stable for the same unknown key", () => {
    // An agent must not change character between launches.
    expect(lookupCharacter("mystery").key).toBe(lookupCharacter("mystery").key);
  });

  it("keeps a character for agents created by every earlier set", () => {
    // The egg set, the creature set and the original emoji set are all still in
    // databases. None of their keys may fall through to the hash.
    const legacy = [
      "plain",
      "cheerful",
      "tophat",
      "monocle",
      "headphones",
      "sprout",
      "owl",
      "crab",
      "slime",
      "ghost",
      "moon",
      "star",
      "robot",
      "penguin",
      "taco",
      "bolt",
    ];
    for (const key of legacy) {
      expect(CHARACTERS, key).toContain(lookupCharacter(key));
    }
    expect(lookupCharacter("plain").key).toBe("avocado");
    expect(lookupCharacter("taco").key).toBe("chip");
    expect(lookupCharacter("ghost").key).toBe("garlic");
  });

  it("gives the starter crew four distinct characters", () => {
    // They sit together in the rail, so collapsing them onto one character would
    // make the crew unreadable at a glance.
    const crew = ["avocado", "owl", "chilli", "star"].map((k) => lookupCharacter(k).key);
    expect(new Set(crew).size).toBe(4);
  });

  it("resolves a known key exactly", () => {
    expect(lookupCharacter("tomato").key).toBe("tomato");
  });
});

describe("suggestions", () => {
  it("picks a character that is not already in use", () => {
    const taken = CHARACTERS.slice(0, 5).map((c) => c.key);
    expect(taken).not.toContain(suggestCharacter(taken));
  });

  it("falls back to the first character once every one is taken", () => {
    expect(suggestCharacter(CHARACTERS.map((c) => c.key))).toBe(CHARACTERS[0]!.key);
  });

  it("picks the least used accent so a new crew stays separable", () => {
    const [first, second] = [ACCENTS[0]!.value, ACCENTS[1]!.value];
    expect(suggestAccent([first])).not.toBe(first);
    expect(suggestAccent([first, second])).not.toBe(first);
    expect(suggestAccent([first, second])).not.toBe(second);
  });

  it("is case insensitive about taken colours", () => {
    const first = ACCENTS[0]!.value;
    expect(suggestAccent([first.toUpperCase()])).not.toBe(first);
  });
});
