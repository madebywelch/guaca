import { describe, expect, it } from "vitest";

import {
  ACCENTS,
  DEFAULT_EGG,
  EGG,
  EGG_GROUPS,
  EGGS,
  eggParts,
  lookupEgg,
  suggestAccent,
  suggestEgg,
} from "./catalog";

describe("egg catalog", () => {
  it("has unique keys", () => {
    const keys = EGGS.map((e) => e.key);
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("only uses groups that are listed", () => {
    for (const egg of EGGS) {
      expect(EGG_GROUPS).toContain(egg.group);
    }
  });

  it("gives every group something to choose from", () => {
    for (const group of EGG_GROUPS) {
      expect(EGGS.filter((e) => e.group === group).length).toBeGreaterThanOrEqual(3);
    }
  });

  it("resolves every preset to real parts", () => {
    // A typo in a preset would silently render an egg with no eyes.
    for (const egg of EGGS) {
      const parts = eggParts(egg);
      expect(parts.eyes, `${egg.key} eyes`).toBeTruthy();
      expect(parts.mouth, `${egg.key} mouth`).toBeTruthy();
      if (egg.accessory !== "none") {
        expect(parts.accessory, `${egg.key} accessory`).toBeTruthy();
      }
    }
  });

  it("varies the parts rather than shipping the same face twice", () => {
    const combos = EGGS.map((e) => `${e.eyes}/${e.mouth}/${e.accessory}`);
    expect(new Set(combos).size).toBe(combos.length);
  });

  it("keeps the shared face geometry inside the drawing", () => {
    // Every egg reuses these, so one bad number moves every face at once.
    expect(EGG.eyeLeft).toBeGreaterThan(12);
    expect(EGG.eyeRight).toBeLessThan(52);
    expect(EGG.mouthY).toBeGreaterThan(EGG.eyeY);
    expect(EGG.mouthY).toBeLessThan(55);
  });

  it("falls back rather than returning undefined for an unknown key", () => {
    const found = lookupEgg("not-a-real-key");
    expect(found).toBeDefined();
    expect(EGGS).toContain(found);
  });

  it("is stable for the same unknown key", () => {
    // An agent must not change face between launches.
    expect(lookupEgg("mystery").key).toBe(lookupEgg("mystery").key);
  });

  it("keeps a face for agents created by earlier avatar sets", () => {
    // Both the creature set and the original emoji set are still in databases.
    for (const legacy of ["avocado", "owl", "chilli", "star", "robot", "penguin", "slime"]) {
      expect(EGGS).toContain(lookupEgg(legacy));
    }
    expect(lookupEgg("owl").key).toBe("glasses");
    expect(lookupEgg("robot").key).toBe("headphones");
  });

  it("gives the starter crew four distinct faces", () => {
    // They sit together in the sidebar, so aliasing them onto one preset would
    // make the crew unreadable at a glance.
    const crew = ["avocado", "owl", "chilli", "star"].map((k) => lookupEgg(k).key);
    expect(new Set(crew).size).toBe(4);
  });

  it("resolves a known key exactly", () => {
    expect(lookupEgg("tophat").key).toBe("tophat");
    expect(lookupEgg(DEFAULT_EGG).key).toBe(DEFAULT_EGG);
  });
});

describe("suggestions", () => {
  it("picks a preset that is not already in use", () => {
    const taken = EGGS.slice(0, 5).map((e) => e.key);
    expect(taken).not.toContain(suggestEgg(taken));
  });

  it("falls back to the first preset once every one is taken", () => {
    expect(suggestEgg(EGGS.map((e) => e.key))).toBe(EGGS[0]!.key);
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
