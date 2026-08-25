import { describe, expect, it } from "vitest";

import { applyMention, matchMentions, mentionAt, splitMentions } from "./mentions";

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

describe("splitMentions", () => {
  /** The runs that resolved, as they were typed. */
  const marked = (text: string, names = NAMES) =>
    splitMentions(text, names)
      .filter((run) => run.kind === "mention")
      .map((run) => run.text);

  it("marks a name the roster has and leaves one it does not", () => {
    // The whole rule. `@` and a word is also a handle, a decorator and half an
    // email address, and a chip around one of those claims a fact nobody has.
    expect(marked("ask @Critic about @lunch")).toEqual(["@Critic"]);
  });

  it("puts the prose back exactly as it arrived", () => {
    const text = "ask @Critic, then @Scribe.";
    expect(
      splitMentions(text, NAMES)
        .map((run) => run.text)
        .join(""),
    ).toBe(text);
  });

  it("takes the longest name, so a two-word one is not cut to its first word", () => {
    expect(marked("@Head Chef please")).toEqual(["@Head Chef"]);
    expect(marked("@Head Chef please", ["Head", "Head Chef"])).toEqual(["@Head Chef"]);
  });

  it("stops at the end of the name rather than inside a longer word", () => {
    expect(marked("@Critical thinking")).toEqual([]);
    expect(marked("@Critic's review")).toEqual(["@Critic"]);
    expect(marked("@Critic, @Scribe")).toEqual(["@Critic", "@Scribe"]);
  });

  it("ignores an @ inside a word, which is where an email address lives", () => {
    expect(marked("write to critic@Manager.com")).toEqual([]);
  });

  it("matches however it was typed and reports the name as the roster spells it", () => {
    const [run] = splitMentions("@critic", NAMES);
    expect(run).toEqual({ kind: "mention", at: 0, text: "@critic", name: "Critic" });
  });

  it("hands back one run of prose when nobody could be named", () => {
    expect(splitMentions("@Critic is not here", [])).toEqual([
      { kind: "text", at: 0, text: "@Critic is not here" },
    ]);
    expect(splitMentions("", NAMES)).toEqual([]);
  });

  it("offsets every run, because they are the keys the two surfaces draw with", () => {
    const runs = splitMentions("ask @Critic now", NAMES);
    expect(runs.map((run) => run.at)).toEqual([0, 4, 11]);
    expect(new Set(runs.map((run) => run.at)).size).toBe(runs.length);
  });
});
