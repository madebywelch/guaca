import { describe, expect, it } from "vitest";

import { type DiffLine, diffSummary, diffTally, lineDiff } from "./diff";

/** A diff as a patch would write it, which is the shortest way to assert one. */
function marked(diff: DiffLine[]): string[] {
  const mark = { same: " ", added: "+", removed: "-" };
  return diff.map((line) => `${mark[line.kind]}${line.text}`);
}

describe("an empty side", () => {
  it("is no lines rather than one blank one", () => {
    // `"".split("\n")` is `[""]`. Left alone, an agent's first memory reads as
    // having replaced a blank line that was never there.
    expect(marked(lineDiff("", "Smith verifies."))).toEqual(["+Smith verifies."]);
    expect(marked(lineDiff("Smith verifies.", ""))).toEqual(["-Smith verifies."]);
    expect(lineDiff("", "")).toEqual([]);
  });
});

describe("what changed", () => {
  it("keeps the lines that did not, so the page is still readable", () => {
    // Not a patch. The operator opening this wants what the agent now believes
    // as much as they want what it just decided.
    const diff = lineDiff("one\ntwo\nthree", "one\ntwo\nthree\nfour");
    expect(marked(diff)).toEqual([" one", " two", " three", "+four"]);
  });

  it("finds a line taken out of the middle without moving the rest", () => {
    const diff = lineDiff("one\ntwo\nthree", "one\nthree");
    expect(marked(diff)).toEqual([" one", "-two", " three"]);
  });

  it("reads a rewritten line as the old one and then the new one", () => {
    // Adjacent, so the two sentences can be compared. Anything else puts the
    // replacement below everything else the rewrite touched.
    const diff = lineDiff("one\ntwo\nthree", "one\nTWO\nthree");
    expect(marked(diff)).toEqual([" one", "-two", "+TWO", " three"]);
  });

  it("holds an insertion apart from the line it was inserted before", () => {
    const diff = lineDiff("a\nb", "a\nnew\nb");
    expect(marked(diff)).toEqual([" a", "+new", " b"]);
  });

  it("says nothing changed when an agent rewrites what it already had", () => {
    // A real turn: asked to remember something it already remembers, an agent
    // writes the file again. Drawn as a page of additions that is a page of
    // additions in a diff, it reads as having thrown its memory away.
    const same = "Smith verifies.\nJones signs off.";
    const diff = lineDiff(same, same);
    expect(diffTally(diff)).toEqual({ added: 0, removed: 0 });
    expect(diffSummary(diff)).toBe("rewritten unchanged");
  });

  it("keeps every line of both versions, whatever it decided about them", () => {
    // The invariant that stops a line being lost between the two: a removal
    // nobody drew is a fact about an agent the operator never gets told.
    const before = "a\nb\nc\nd\ne";
    const after = "a\nx\nc\ny\ne\nf";
    const diff = lineDiff(before, after);
    const kept = (kinds: string[]) =>
      diff.filter((line) => kinds.includes(line.kind)).map((line) => line.text);

    expect(kept(["same", "removed"])).toEqual(before.split("\n"));
    expect(kept(["same", "added"])).toEqual(after.split("\n"));
  });
});

describe("a document too large to compare line by line", () => {
  it("is shown as a wholesale replacement rather than left working it out", () => {
    // Well past what memory can hold, so this is the shape of a bug rather than
    // of a page. It has to end, and it has to end without dropping a line.
    const before = Array.from({ length: 700 }, (_, i) => `old ${i}`).join("\n");
    const after = Array.from({ length: 700 }, (_, i) => `new ${i}`).join("\n");

    const diff = lineDiff(before, after);
    expect(diffTally(diff)).toEqual({ added: 700, removed: 700 });
    expect(diff.slice(0, 700).every((line) => line.kind === "removed")).toBe(true);
  });

  it("still matches the ends first, which is what makes a real page cheap", () => {
    // A page rewritten with one line changed is two short middles, however long
    // the page is: the table is never built for the part that did not move.
    const body = Array.from({ length: 700 }, (_, i) => `line ${i}`);
    const after = [...body];
    after[350] = "changed";

    const diff = lineDiff(body.join("\n"), after.join("\n"));
    expect(diffTally(diff)).toEqual({ added: 1, removed: 1 });
  });
});

describe("the tally", () => {
  it("counts in words, since it is the one part of a diff that is read aloud", () => {
    expect(diffSummary(lineDiff("a", "a\nb"))).toBe("1 line added");
    expect(diffSummary(lineDiff("a\nb", "a"))).toBe("1 line removed");
    expect(diffSummary(lineDiff("a\nb", "a\nc\nd"))).toBe("2 lines added, 1 removed");
  });
});
