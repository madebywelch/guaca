import { describe, expect, it } from "vitest";

import { applyMention, matchMentions, mentionAt } from "./mentions";

const NAMES = ["Manager", "Researcher", "Critic", "Scribe", "Head Chef"];

describe("mentionAt", () => {
  it("finds a mention being typed", () => {
    expect(mentionAt("ask @Cri", 8)).toEqual({ start: 4, end: 8, term: "Cri" });
  });

  it("opens on a bare @", () => {
    expect(mentionAt("@", 1)).toEqual({ start: 0, end: 1, term: "" });
  });

  it("ignores an @ inside a word", () => {
    // Otherwise typing an email address opens a menu.
    expect(mentionAt("mail me at bob@example.com", 26)).toBeNull();
  });

  it("opens after an opening bracket", () => {
    expect(mentionAt("(@Cri", 5)?.term).toBe("Cri");
  });

  it("closes once the mention is clearly abandoned", () => {
    expect(mentionAt("@Manager and then some more words", 33)).toBeNull();
    expect(mentionAt("@Manager\nnext line", 18)).toBeNull();
  });

  it("allows a couple of spaces, because agent names can contain them", () => {
    expect(mentionAt("@Head Ch", 8)?.term).toBe("Head Ch");
    expect(mentionAt("@Head Sous Ch", 13)?.term).toBe("Head Sous Ch");
  });

  it("uses the caret, not the end of the text", () => {
    // Editing mid-sentence must still offer completions.
    expect(mentionAt("ask @Cri to review", 8)).toEqual({ start: 4, end: 8, term: "Cri" });
  });

  it("returns nothing when there is no @ at all", () => {
    expect(mentionAt("plain text", 10)).toBeNull();
  });
});

describe("matchMentions", () => {
  it("lists everyone for a bare @", () => {
    expect(matchMentions(NAMES, "")).toHaveLength(5);
  });

  it("is case insensitive, and ranks a prefix match first", () => {
    // "Scribe" contains "cri" too, so this also pins the ordering.
    expect(matchMentions(NAMES, "cri")).toEqual(["Critic", "Scribe"]);
  });

  it("puts prefix matches before contains matches", () => {
    expect(matchMentions(["Chief Resource", "Researcher"], "res")).toEqual([
      "Researcher",
      "Chief Resource",
    ]);
  });

  it("caps the list", () => {
    const many = Array.from({ length: 30 }, (_, i) => `Agent${i}`);
    expect(matchMentions(many, "agent").length).toBeLessThanOrEqual(6);
  });

  it("returns nothing when nothing matches", () => {
    expect(matchMentions(NAMES, "zzz")).toEqual([]);
  });
});

describe("applyMention", () => {
  it("replaces the partial mention and leaves the caret after it", () => {
    const query = mentionAt("ask @Cri", 8)!;
    const result = applyMention("ask @Cri", query, "Critic");
    expect(result.text).toBe("ask @Critic ");
    expect(result.caret).toBe(result.text.length);
  });

  it("keeps whatever follows the caret", () => {
    const query = mentionAt("ask @Cri to review", 8)!;
    const result = applyMention("ask @Cri to review", query, "Critic");
    expect(result.text).toBe("ask @Critic  to review");
    expect(result.text.slice(result.caret)).toBe(" to review");
  });

  it("inserts a name containing a space intact", () => {
    // Exact names are the point: send_message resolves recipients by name.
    const query = mentionAt("@Head", 5)!;
    expect(applyMention("@Head", query, "Head Chef").text).toBe("@Head Chef ");
  });
});
