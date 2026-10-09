import { describe, expect, it } from "vitest";

import { type DiffKind, type DiffLine, diffSummary, diffTally, lineDiff } from "./diff";

/** A diff as a patch would write it, which is the shortest way to assert one. */
function marked(diff: DiffLine[]): string[] {
  const mark = { same: " ", added: "+", removed: "-" };
  return diff.map((line) => `${mark[line.kind]}${line.text}`);
}

/**
 * The two documents a diff is of, recovered row by row.
 *
 * The before side is every kept line and every removal; the after side is
 * every kept line and every addition. Written out rather than joined, sorted or
 * deduplicated, because a reconstruction only checks anything if it preserves
 * each line and its position. The expected arrays passed to it are literals,
 * never the strings fed to `lineDiff` split back apart.
 */
function reconstruct(diff: DiffLine[]): { before: string[]; after: string[] } {
  const keep = (kinds: DiffKind[]) =>
    diff.filter((line) => kinds.includes(line.kind)).map((line) => line.text);
  return {
    before: keep(["same", "removed"]),
    after: keep(["same", "added"]),
  };
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
    const diff = lineDiff("a\nb\nc\nd\ne", "a\nx\nc\ny\ne\nf");

    expect(reconstruct(diff)).toEqual({
      before: ["a", "b", "c", "d", "e"],
      after: ["a", "x", "c", "y", "e", "f"],
    });
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

describe("newlines, blanks and repeated lines", () => {
  // The expected line arrays are written out here rather than derived from the
  // inputs, so a change to how the inputs are split cannot move the goalposts
  // with it. Each row names the raw document on each side and the two sequences
  // it must reconstruct to, plus a hand-counted tally.
  const cases: {
    name: string;
    before: string;
    after: string;
    beforeLines: string[];
    afterLines: string[];
    added: number;
    removed: number;
  }[] = [
    {
      name: "reads the same document however its lines are ended",
      before: "a\r\n\r\nb\r\n",
      after: "a\n\nb\n",
      beforeLines: ["a", "", "b", ""],
      afterLines: ["a", "", "b", ""],
      added: 0,
      removed: 0,
    },
    {
      name: "reads it the same the other way round too",
      // A regression that normalized only one side, or only the first side
      // handed to it, leaves carriage returns in the payload it drew.
      before: "a\n\nb\n",
      after: "a\r\n\r\nb\r\n",
      beforeLines: ["a", "", "b", ""],
      afterLines: ["a", "", "b", ""],
      added: 0,
      removed: 0,
    },
    {
      name: "finds a real edit among mixed line endings",
      before: "top\r\nkeep\nold\r\nend",
      after: "top\nkeep\r\nnew\nend",
      beforeLines: ["top", "keep", "old", "end"],
      afterLines: ["top", "keep", "new", "end"],
      added: 1,
      removed: 1,
    },
    {
      name: "writes a first memory and keeps the entry after its newline",
      // Split on LF alone gives `["one"]`, dropping the blank line the trailing
      // newline stands for; trimming at the end gives `["one", ""]` too, but
      // for the wrong reason, so the two are told apart by the rows below.
      before: "",
      after: "one\n",
      beforeLines: [],
      afterLines: ["one", ""],
      added: 2,
      removed: 0,
    },
    {
      name: "reads a final newline as its own blank line",
      before: "one",
      after: "one\n",
      beforeLines: ["one"],
      afterLines: ["one", ""],
      added: 1,
      removed: 0,
    },
    {
      name: "reads the loss of a final newline as a change",
      before: "one\n",
      after: "one",
      beforeLines: ["one", ""],
      afterLines: ["one"],
      added: 0,
      removed: 1,
    },
    {
      name: "keeps a second trailing newline as a second blank line",
      before: "one\n",
      after: "one\n\n",
      beforeLines: ["one", ""],
      afterLines: ["one", "", ""],
      added: 1,
      removed: 0,
    },
    {
      name: "removes one of two trailing newlines",
      before: "one\n\n",
      after: "one\n",
      beforeLines: ["one", "", ""],
      afterLines: ["one", ""],
      added: 0,
      removed: 1,
    },
    {
      name: "counts a blank-only page as a document with blank lines",
      // `"".split("\n")` is `[""]`, the empty document is zero lines, and
      // `"\n".split("\n")` is two blanks. Collapsing the two would read a page
      // of whitespace as nothing at all.
      before: "",
      after: "\n",
      beforeLines: [],
      afterLines: ["", ""],
      added: 2,
      removed: 0,
    },
    {
      name: "clears a blank-only page rather than rewriting one blank line",
      before: "\n",
      after: "",
      beforeLines: ["", ""],
      afterLines: [],
      added: 0,
      removed: 2,
    },
    {
      name: "keeps interior blanks in order and at their count",
      before: "head\n\nbody\n\nfoot",
      after: "head\nbody\n\n\nfoot",
      beforeLines: ["head", "", "body", "", "foot"],
      afterLines: ["head", "body", "", "", "foot"],
      added: 1,
      removed: 1,
    },
    {
      name: "removes one occurrence and leaves the other where it was",
      before: "start\nrepeat\nrepeat\nend",
      after: "start\nrepeat\nend",
      beforeLines: ["start", "repeat", "repeat", "end"],
      afterLines: ["start", "repeat", "end"],
      added: 0,
      removed: 1,
    },
    {
      name: "adds a second occurrence rather than treating one as enough",
      before: "start\nrepeat\nend",
      after: "start\nrepeat\nrepeat\nend",
      beforeLines: ["start", "repeat", "end"],
      afterLines: ["start", "repeat", "repeat", "end"],
      added: 1,
      removed: 0,
    },
    {
      name: "keeps both versions when several alignments are equally short",
      // `a,b,a` against `b,a,b` has two two-line common subsequences, `a,b` and
      // `b,a`. Either is minimal, so nothing here says which one wins; a test
      // that did would fail on a correct change of tie-break. What is fixed is
      // the one removal and one addition, and that both sequences come back.
      before: "left\na\nb\na\nright",
      after: "left\nb\na\nb\nright",
      beforeLines: ["left", "a", "b", "a", "right"],
      afterLines: ["left", "b", "a", "b", "right"],
      added: 1,
      removed: 1,
    },
    {
      name: "returns every line when edits sit on both sides of a match",
      before: "old-first\nkeep\nold-last",
      after: "new-first\nkeep\nnew-last\nnew-tail",
      beforeLines: ["old-first", "keep", "old-last"],
      afterLines: ["new-first", "keep", "new-last", "new-tail"],
      added: 3,
      removed: 2,
    },
  ];

  for (const row of cases) {
    it(row.name, () => {
      const diff = lineDiff(row.before, row.after);
      expect(reconstruct(diff)).toEqual({ before: row.beforeLines, after: row.afterLines });
      expect(diffTally(diff)).toEqual({ added: row.added, removed: row.removed });
    });
  }

  it("reads equivalent documents as unchanged line for line", () => {
    // The stronger claim behind the first two rows: no row is even marked as a
    // change, so no carriage return survives into a payload as a phantom edit.
    for (const [before, after] of [
      ["a\r\n\r\nb\r\n", "a\n\nb\n"],
      ["a\n\nb\n", "a\r\n\r\nb\r\n"],
    ] as const) {
      expect(lineDiff(before, after).every((line) => line.kind === "same")).toBe(true);
    }
  });
});

describe("the size cutoff", () => {
  /**
   * A middle whose lines cannot match across the two sides, with one shared
   * sentinel line and the same prefix and suffix around both.
   *
   * The sentinel is the point: it is the one pair the table can keep, so a
   * branch that builds the table and a branch that replaces the middle wholesale
   * draw different rows. All-different middles would look identical whichever
   * branch ran, and would pass even if the guard were wrong.
   */
  function wrapped(middle: number, other: number, sentinelOld: number, sentinelNew: number) {
    const oldMiddle = Array.from({ length: middle }, (_, i) => `old ${i}`);
    const newMiddle = Array.from({ length: other }, (_, i) => `new ${i}`);
    oldMiddle[sentinelOld] = "shared middle";
    newMiddle[sentinelNew] = "shared middle";
    const beforeLines = ["prefix", ...oldMiddle, "suffix"];
    const afterLines = ["prefix", ...newMiddle, "suffix"];
    return { beforeLines, afterLines };
  }

  it("still matches the lines a full middle holds at exactly the cutoff", () => {
    // The trimmed middle is 500 x 500, exactly 250,000 cells, which the table
    // is still built for. The full documents are 502 lines against 502, above
    // the cutoff, so this also catches a guard that counts the whole documents
    // instead of the unmatched middle the table is actually for.
    const { beforeLines, afterLines } = wrapped(500, 500, 250, 250);
    const diff = lineDiff(beforeLines.join("\n"), afterLines.join("\n"));

    expect(diff.filter((line) => line.kind === "same").map((line) => line.text)).toEqual([
      "prefix",
      "shared middle",
      "suffix",
    ]);
    expect(reconstruct(diff)).toEqual({ before: beforeLines, after: afterLines });
    expect(diffTally(diff)).toEqual({ added: 499, removed: 499 });
  });

  it("replaces the middle wholesale one cell above the cutoff", () => {
    // 53 x 4,717 is 250,001 cells, one cell past the limit, so the table is not
    // built: every middle line is a removal followed by every addition, and
    // the shared sentinel appears on both sides rather than as a kept line.
    // An off-by-one in the guard, counting allocated cells as 54 x 4,718, or
    // raising the limit would each keep the sentinel instead.
    const { beforeLines, afterLines } = wrapped(53, 4717, 26, 2358);
    const diff = lineDiff(beforeLines.join("\n"), afterLines.join("\n"));

    expect(diff.filter((line) => line.kind === "same").map((line) => line.text)).toEqual([
      "prefix",
      "suffix",
    ]);
    expect(reconstruct(diff)).toEqual({ before: beforeLines, after: afterLines });
    expect(diffTally(diff)).toEqual({ added: 4717, removed: 53 });

    // The retained ends stay at the ends, the removals come before the
    // additions, and the two blocks meet in the middle with nothing between:
    // prefix, 53 removals, 4,717 additions, suffix.
    expect(diff[0]!.kind).toBe("same");
    expect(diff.slice(1, 54).every((line) => line.kind === "removed")).toBe(true);
    expect(diff.slice(54, 54 + 4717).every((line) => line.kind === "added")).toBe(true);
    expect(diff[54 + 4717]!.kind).toBe("same");
    expect(diff).toHaveLength(4772);
  });
});

describe("the tally", () => {
  it("counts in words, since it is the one part of a diff that is read aloud", () => {
    expect(diffSummary(lineDiff("a", "a\nb"))).toBe("1 line added");
    expect(diffSummary(lineDiff("a\nb", "a"))).toBe("1 line removed");
    expect(diffSummary(lineDiff("a\nb", "a\nc\nd"))).toBe("2 lines added, 1 removed");
  });
});
