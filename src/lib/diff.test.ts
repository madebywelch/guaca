import { describe, expect, it } from "vitest";

import { type DiffKind, type DiffLine, diffSummary, diffTally, lineDiff } from "./diff";

function marked(diff: DiffLine[]): string[] {
  const mark = { same: " ", added: "+", removed: "-" };
  return diff.map((line) => `${mark[line.kind]}${line.text}`);
}

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
    expect(marked(lineDiff("", "Smith verifies."))).toEqual(["+Smith verifies."]);
    expect(marked(lineDiff("Smith verifies.", ""))).toEqual(["-Smith verifies."]);
    expect(lineDiff("", "")).toEqual([]);
  });
});

describe("what changed", () => {
  it("keeps the lines that did not, so the page is still readable", () => {
    const diff = lineDiff("one\ntwo\nthree", "one\ntwo\nthree\nfour");
    expect(marked(diff)).toEqual([" one", " two", " three", "+four"]);
  });

  it("finds a line taken out of the middle without moving the rest", () => {
    const diff = lineDiff("one\ntwo\nthree", "one\nthree");
    expect(marked(diff)).toEqual([" one", "-two", " three"]);
  });

  it("reads a rewritten line as the old one and then the new one", () => {
    const diff = lineDiff("one\ntwo\nthree", "one\nTWO\nthree");
    expect(marked(diff)).toEqual([" one", "-two", "+TWO", " three"]);
  });

  it("holds an insertion apart from the line it was inserted before", () => {
    const diff = lineDiff("a\nb", "a\nnew\nb");
    expect(marked(diff)).toEqual([" a", "+new", " b"]);
  });

  it("says nothing changed when an agent rewrites what it already had", () => {
    const same = "Smith verifies.\nJones signs off.";
    const diff = lineDiff(same, same);
    expect(diffTally(diff)).toEqual({ added: 0, removed: 0 });
    expect(diffSummary(diff)).toBe("rewritten unchanged");
  });

  it("keeps every line of both versions, whatever it decided about them", () => {
    const diff = lineDiff("a\nb\nc\nd\ne", "a\nx\nc\ny\ne\nf");

    expect(reconstruct(diff)).toEqual({
      before: ["a", "b", "c", "d", "e"],
      after: ["a", "x", "c", "y", "e", "f"],
    });
  });
});

describe("a document too large to compare line by line", () => {
  it("is shown as a wholesale replacement rather than left working it out", () => {
    const before = Array.from({ length: 700 }, (_, i) => `old ${i}`).join("\n");
    const after = Array.from({ length: 700 }, (_, i) => `new ${i}`).join("\n");

    const diff = lineDiff(before, after);
    expect(diffTally(diff)).toEqual({ added: 700, removed: 700 });
    expect(diff.slice(0, 700).every((line) => line.kind === "removed")).toBe(true);
  });

  it("still matches the ends first, which is what makes a real page cheap", () => {
    const body = Array.from({ length: 700 }, (_, i) => `line ${i}`);
    const after = [...body];
    after[350] = "changed";

    const diff = lineDiff(body.join("\n"), after.join("\n"));
    expect(diffTally(diff)).toEqual({ added: 1, removed: 1 });
  });
});

describe("newlines, blanks and repeated lines", () => {
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
    for (const [before, after] of [
      ["a\r\n\r\nb\r\n", "a\n\nb\n"],
      ["a\n\nb\n", "a\r\n\r\nb\r\n"],
    ] as const) {
      expect(lineDiff(before, after).every((line) => line.kind === "same")).toBe(true);
    }
  });
});

describe("the size cutoff", () => {
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
    const { beforeLines, afterLines } = wrapped(53, 4717, 26, 2358);
    const diff = lineDiff(beforeLines.join("\n"), afterLines.join("\n"));

    expect(diff.filter((line) => line.kind === "same").map((line) => line.text)).toEqual([
      "prefix",
      "suffix",
    ]);
    expect(reconstruct(diff)).toEqual({ before: beforeLines, after: afterLines });
    expect(diffTally(diff)).toEqual({ added: 4717, removed: 53 });

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
