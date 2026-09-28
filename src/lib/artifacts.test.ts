import { describe, expect, it } from "vitest";

import { detailsLine, logRows, ownerLabel, pageData, sentMessage, sourceLine } from "./artifacts";
import type { Artifact, ArtifactEntry } from "./types";

const rae = { kind: "agent", id: "rae", name: "Rae" } as const;
const milo = { kind: "agent", id: "milo", name: "Milo" } as const;
const juno = { kind: "agent", id: "juno", name: "Juno" } as const;

function entry(
  seq: number,
  change: ArtifactEntry["change"],
  by: ArtifactEntry["by"],
  version: number,
  owner: string | null,
  note = "",
): ArtifactEntry {
  return { seq, at: seq, change, by, version, owner, note };
}

describe("an artifact's history", () => {
  it("says who a take-over was from, which no single row records", () => {
    const rows = logRows([
      entry(1, "created", rae, 1, "Rae", "First cut"),
      entry(2, "edited", milo, 2, "Rae", "Added a Q4 column"),
      entry(3, "took", juno, 2, "Juno", "Rae moved to Ops"),
      entry(4, "handed", { kind: "operator" }, 2, "Milo"),
    ]);
    expect(rows.map((row) => `${row.who} ${row.what}`)).toEqual([
      "You handed it from Juno to Milo",
      "Juno took it over from Rae",
      "Milo edited it",
      "Rae created it",
    ]);
  });

  it("only offers to open the rows that made a version", () => {
    // A take-over has no page of its own. Offering to open one would open the
    // version before it and call it something else.
    const rows = logRows([
      entry(1, "created", rae, 1, "Rae"),
      entry(2, "took", milo, 1, "Milo", "Mine now"),
      entry(3, "restored", { kind: "operator" }, 2, "Milo", "Put back version 1."),
    ]);
    expect(rows.map((row) => [row.version, row.opens])).toEqual([
      [2, true],
      [1, false],
      [1, true],
    ]);
  });

  it("names nobody as nobody when there was no owner before", () => {
    const rows = logRows([entry(1, "took", milo, 1, "Milo", "Nobody had it")]);
    expect(rows[0]?.what).toBe("took it over from nobody");
  });
});

describe("who owns it", () => {
  it("still names an owner who left, and says so", () => {
    // Ownership moves only by a decision. An owner that left is not replaced
    // quietly, so the list has to say that nobody has decided yet.
    expect(ownerLabel({ id: "rae", name: "Rae", gone: true })).toBe("Rae, left the crew");
    expect(ownerLabel({ id: "rae", name: "Rae", gone: false })).toBe("Rae");
    expect(ownerLabel(null)).toBe("Nobody");
  });
});

describe("the line under a name", () => {
  const artifact: Artifact = {
    id: "a1",
    groupId: "g1",
    owner: { id: "rae", name: "Rae", gone: false },
    title: "Pipeline",
    version: 7,
    editedBy: { kind: "agent", id: "milo", name: "Milo" },
    createdAt: 0,
    updatedAt: new Date(2026, 8, 27, 14, 5).getTime(),
    sources: [],
    sourcesAllowed: false,
  };
  const now = new Date(2026, 8, 27, 18, 0).getTime();

  it("leads with the crew only while every crew is listed", () => {
    expect(detailsLine(artifact, "Sales", now)).toMatch(/^Sales · Owner Rae · v7 · Today /);
    expect(detailsLine(artifact, null, now)).toMatch(/^Owner Rae · v7 · Today /);
  });
});

describe("what a page is handed", () => {
  it("keys every read by its source's name, refused ones included", () => {
    // A page can always say which of its numbers is missing and why, rather
    // than failing on one it expected to find.
    expect(
      pageData([
        { name: "issues", data: [{ id: 1 }], text: '[{"id":1}]', error: null },
        { name: "deals", data: null, text: null, error: "Not allowed yet." },
      ]),
    ).toEqual({
      issues: { data: [{ id: 1 }], text: '[{"id":1}]', error: null },
      deals: { data: null, text: null, error: "Not allowed yet." },
    });
  });

  it("says a source as connector, tool and exactly what it is sent", () => {
    expect(
      sourceLine({ name: "issues", tool: "linear__list_issues", arguments: { team: "ENG" } }),
    ).toBe('linear · list_issues {"team":"ENG"}');
    expect(sourceLine({ name: "me", tool: "linear__viewer", arguments: {} })).toBe(
      "linear · viewer",
    );
  });

  it("fences what a page sent so it cannot end its own block", () => {
    const page: Artifact = {
      id: "a1",
      groupId: "g1",
      owner: null,
      title: "Code",
      version: 1,
      editedBy: { kind: "operator" },
      createdAt: 0,
      updatedAt: 0,
      sources: [],
      sourcesAllowed: false,
    };
    const text = sentMessage(page, '{"snippet":"```"}');
    expect(text).toContain('````json\n{"snippet":"```"}\n````');
  });
});
