import { describe, expect, it } from "vitest";

import { ACCENTS, CHARACTERS, lookupCharacter } from "../avatars/catalog";
import { HIREABLE, pick, STARTER_CREW, STATIONS, toDraft } from "./cafeteria";
import { matchMentions, mentionAt } from "./mentions";

/**
 * The cafeteria is data, so this is where its rules live.
 *
 * Nothing here needs a running app: a preset that names a character nobody
 * draws, or a colour the editor cannot show, is wrong the moment it is written
 * and should fail the build rather than a first run.
 */

const MAX_NAME_LEN = 48; // domain::agent::MAX_NAME_LEN

describe("the cafeteria catalog", () => {
  it("has presets to hire", () => {
    // Without this the rest of the file passes vacuously on an empty array.
    expect(HIREABLE.length).toBeGreaterThan(5);
  });

  it("gives every preset an id nothing else uses", () => {
    // Selection is keyed by id. Two presets sharing one means picking either
    // hires both, which reads as the app inventing an agent.
    const ids = HIREABLE.map((preset) => preset.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("gives every preset a name nothing else in the catalog uses", () => {
    // Names are unique per group in the database. Two presets sharing one is
    // legal, and it is also a batch that quietly hires "Editor copy" for a
    // preset the operator picked by its own name.
    const names = HIREABLE.map((preset) => preset.name.toLowerCase());
    expect(new Set(names).size).toBe(names.length);
  });

  it("draws every preset with a character the catalog actually has", () => {
    // `lookupCharacter` never fails: an unknown key falls back to a hash of
    // itself, so a typo here is a face nobody chose rather than a blank one,
    // and it would go unnoticed forever. The whole promise of the cafeteria is
    // that the look was chosen.
    const drawn = new Set(CHARACTERS.map((character) => character.key));
    const strays = HIREABLE.filter((preset) => !drawn.has(preset.avatar));
    expect(strays.map((preset) => `${preset.id}: ${preset.avatar}`)).toEqual([]);
  });

  it("colours every preset with an accent the editor offers", () => {
    // So that opening a hired agent shows its swatch already selected, rather
    // than a colour the operator cannot get back if they click away.
    const offered = new Set(ACCENTS.map((accent) => accent.value));
    const strays = HIREABLE.filter((preset) => !offered.has(preset.color));
    expect(strays.map((preset) => `${preset.id}: ${preset.color}`)).toEqual([]);
  });

  it("gives each station a distinct silhouette", () => {
    // Two agents side by side at one counter have to be tellable apart at the
    // 22px the rail draws them at, and colour is not what does that.
    for (const station of STATIONS) {
      const here = HIREABLE.filter((preset) => preset.station === station);
      const faces = here.map((preset) => lookupCharacter(preset.avatar).key);
      expect(new Set(faces).size, `${station} draws two agents the same way`).toBe(here.length);
    }
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
    // The same rules `AgentDraft::validate` enforces on the Rust side. A preset
    // that fails them is a card that cannot be hired, and the operator finds
    // out by clicking the button.
    for (const preset of HIREABLE) {
      expect(preset.name.trim(), preset.id).not.toBe("");
      expect([...preset.name].length, `${preset.id} name is too long`).toBeLessThanOrEqual(
        MAX_NAME_LEN,
      );
      expect(preset.color, `${preset.id} colour`).toMatch(/^#[0-9a-f]{6}$/);
      expect(preset.avatar.trim(), preset.id).not.toBe("");
    }
  });

  it("tells peers what every preset is for", () => {
    // Skills are what another agent reads when it decides who to ask, so a
    // preset without them is one the crew cannot route work to.
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
    // Peers resolve each other by whole name, and the composer's `@` typeahead
    // gives up after two spaces. A four-word job title is an agent nobody can
    // mention and nobody can delegate to, which is most of the point of it.
    for (const preset of HIREABLE) {
      const spaces = preset.name.split(" ").length - 1;
      expect(spaces, `${preset.id} has too many words to mention`).toBeLessThanOrEqual(2);

      // The real path, not a reimplementation of the rule: type the whole name
      // after an `@` and check the menu still offers it.
      const typed = `@${preset.name}`;
      const query = mentionAt(typed, typed.length);
      expect(query, `@${preset.name} does not read as a mention`).not.toBeNull();
      expect(matchMentions([preset.name], query!.term)).toEqual([preset.name]);
    }
  });

  it("names presets after jobs rather than after functions", () => {
    // A guard on the thing that makes this catalog worth having. Bare function
    // labels carry no duties and no refusals, so the operator ends up writing
    // the role into the prompt anyway.
    const generic = ["manager", "editor", "reviewer", "writer", "analyst", "assistant"];
    const bare = HIREABLE.filter((preset) => generic.includes(preset.name.toLowerCase()));
    expect(
      bare.map((preset) => preset.name),
      "too generic to be a job title",
    ).toEqual([]);
  });

  it("keeps every prompt short enough that somebody read it", () => {
    // Not a rule about models, a rule about maintenance. A preset prompt long
    // enough to hide a stopping rule inside is one nobody will check again,
    // and a prompt with no stopping rule is the defect the evals exist to
    // catch. Kept where a reviewer can hold the whole thing in their head.
    for (const preset of HIREABLE) {
      expect(preset.systemPrompt.trim(), preset.id).not.toBe("");
      expect(preset.systemPrompt.length, `${preset.id} prompt is too long to review`).toBeLessThan(
        500,
      );
    }
  });

  it("never ties a browser to a computer, because they are two places", () => {
    // A preset prompt is injected above the sections that describe what an
    // agent has, so a preset that names a surface outranks the runtime's own
    // account of it. The Market Researcher said "using your computer's
    // browser", which in this app names the Chrome on the machine's screen:
    // the agent worked the desktop through screenshots instead of asking the
    // page, and every screenshot is an image the operator's model may not
    // accept. A preset describes the work. Which surface it lands on is the
    // runtime's to say, in `prompt.rs` and the tool descriptions.
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
    // Blank means inherit. A preset that pinned a model would override a group
    // that deliberately chose its own endpoint and model.
    expect(draft.model).toBe("");
    expect(draft.name).toBe(HIREABLE[0]!.name);
    expect(draft.systemPrompt).toBe(HIREABLE[0]!.systemPrompt);
    expect(draft.skills).toEqual(HIREABLE[0]!.skills);
  });

  it("sends no group on the draft, because the command carries it", () => {
    // `hire_agents` takes one group and overrides whatever the drafts hold, so
    // a group here would be a value that silently does nothing.
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
    // This is what an empty workspace hires on the first click. An id that
    // stopped matching would make that button hire fewer agents than it says,
    // or none.
    expect(pick(STARTER_CREW)).toHaveLength(STARTER_CREW.length);
  });

  it("starts a crew with somebody to delegate and somebody to delegate to", () => {
    const crew = pick(STARTER_CREW);
    expect(crew.length).toBeGreaterThan(1);
    expect(crew.some((preset) => preset.skills.includes("delegation"))).toBe(true);
  });
});
