import { describe, expect, it } from "vitest";

import { ACCENTS, CHARACTERS, lookupCharacter } from "../avatars/catalog";
import { HIREABLE, pick, STARTER_CREW, STATIONS, toDraft } from "./cafeteria";
import { matchMentions, mentionAt } from "./mentions";
import constants from "./mirrors/constants.json";

const MAX_NAME_LEN = constants.maxAgentNameLength;

describe("the cafeteria catalog", () => {
  it("has presets to hire", () => {
    expect(HIREABLE.length).toBeGreaterThan(5);
  });

  it("gives every preset an id nothing else uses", () => {
    const ids = HIREABLE.map((preset) => preset.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("gives every preset a name nothing else in the catalog uses", () => {
    const names = HIREABLE.map((preset) => preset.name.toLowerCase());
    expect(new Set(names).size).toBe(names.length);
  });

  it("draws every preset with a character the catalog actually has", () => {
    const drawn = new Set(CHARACTERS.map((character) => character.key));
    const strays = HIREABLE.filter((preset) => !drawn.has(preset.avatar));
    expect(strays.map((preset) => `${preset.id}: ${preset.avatar}`)).toEqual([]);
  });

  it("colors every preset with an accent the editor offers", () => {
    const offered = new Set(ACCENTS.map((accent) => accent.value));
    const strays = HIREABLE.filter((preset) => !offered.has(preset.color));
    expect(strays.map((preset) => `${preset.id}: ${preset.color}`)).toEqual([]);
  });

  it("gives every preset a character nothing else in the catalog has", () => {
    const faces = HIREABLE.map((preset) => lookupCharacter(preset.avatar).key);
    const twice = faces.filter((face, i) => faces.indexOf(face) !== i);
    expect([...new Set(twice)], "two presets draw the same way").toEqual([]);
  });

  it("puts every preset at a station that is drawn", () => {
    const strays = HIREABLE.filter((preset) => !STATIONS.includes(preset.station));
    expect(
      strays.map((preset) => preset.id),
      "would not appear anywhere",
    ).toEqual([]);
  });

  it("leaves no station empty", () => {
    const empty = STATIONS.filter((station) => !HIREABLE.some((p) => p.station === station));
    expect(empty, "a heading with nothing under it").toEqual([]);
  });

  it("writes every preset so the runtime would accept it", () => {
    for (const preset of HIREABLE) {
      expect(preset.name.trim(), preset.id).not.toBe("");
      expect([...preset.name].length, `${preset.id} name is too long`).toBeLessThanOrEqual(
        MAX_NAME_LEN,
      );
      expect(preset.color, `${preset.id} color`).toMatch(/^#[0-9a-f]{6}$/);
      expect(preset.avatar.trim(), preset.id).not.toBe("");
    }
  });

  it("tells peers what every preset is for", () => {
    for (const preset of HIREABLE) {
      expect(preset.skills.length, `${preset.id} has no skills`).toBeGreaterThan(0);
      expect(
        preset.skills.every((skill) => skill.trim().length > 0),
        preset.id,
      ).toBe(true);
      expect(preset.tagline.trim(), `${preset.id} has nothing to read`).not.toBe("");
    }
  });

  it("gives every preset a title a peer can actually address", () => {
    for (const preset of HIREABLE) {
      const spaces = preset.name.split(" ").length - 1;
      expect(spaces, `${preset.id} has too many words to mention`).toBeLessThanOrEqual(2);

      const typed = `@${preset.name}`;
      const query = mentionAt(typed, typed.length);
      expect(query, `@${preset.name} does not read as a mention`).not.toBeNull();
      expect(matchMentions([preset.name], query!.term)).toEqual([preset.name]);
    }
  });

  it("names presets after jobs rather than after functions", () => {
    const generic = ["manager", "editor", "reviewer", "writer", "analyst", "assistant"];
    const bare = HIREABLE.filter((preset) => generic.includes(preset.name.toLowerCase()));
    expect(
      bare.map((preset) => preset.name),
      "too generic to be a job title",
    ).toEqual([]);
  });

  it("keeps every prompt short enough that somebody read it", () => {
    for (const preset of HIREABLE) {
      expect(preset.systemPrompt.trim(), preset.id).not.toBe("");
      expect(preset.systemPrompt.length, `${preset.id} prompt is too long to review`).toBeLessThan(
        500,
      );
    }
  });

  it("never ties a browser to a computer, because they are two places", () => {
    const conflates = [
      "computer's browser",
      "browser on your computer",
      "browser on your machine",
      "browser on your screen",
      "screenshot",
    ];
    for (const preset of HIREABLE) {
      const prompt = preset.systemPrompt.toLowerCase();
      for (const phrase of conflates) {
        expect(prompt, `${preset.id} sends its browsing to the wrong surface`).not.toContain(
          phrase,
        );
      }
    }
  });
});

describe("hiring", () => {
  it("hands the runtime a draft that inherits its model", () => {
    const draft = toDraft(HIREABLE[0]!);
    expect(draft.model).toBe("");
    expect(draft.name).toBe(HIREABLE[0]!.name);
    expect(draft.systemPrompt).toBe(HIREABLE[0]!.systemPrompt);
    expect(draft.skills).toEqual(HIREABLE[0]!.skills);
  });

  it("sends no group on the draft, because the command carries it", () => {
    expect(toDraft(HIREABLE[0]!).groupId).toBeUndefined();
  });

  it("picks presets in catalog order, whatever order they were selected in", () => {
    const ids = [HIREABLE[3]!.id, HIREABLE[0]!.id];
    expect(pick(ids).map((preset) => preset.id)).toEqual([HIREABLE[0]!.id, HIREABLE[3]!.id]);
  });

  it("drops an id nothing matches rather than inventing an agent", () => {
    expect(pick(["nobody-by-that-name"])).toEqual([]);
  });

  it("names a starter crew that is actually in the catalog", () => {
    expect(pick(STARTER_CREW)).toHaveLength(STARTER_CREW.length);
  });

  it("starts a crew with somebody to delegate and somebody to delegate to", () => {
    const crew = pick(STARTER_CREW);
    expect(crew.length).toBeGreaterThan(1);
    expect(crew.some((preset) => preset.skills.includes("delegation"))).toBe(true);
  });
});
