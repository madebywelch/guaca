import { describe, expect, it } from "vitest";
import { fitRows, taskMaps } from "./taskMap";
import type { Approval, Envelope } from "./types";

const lead = { kind: "agent", id: "lead" } as const;
const researcher = { kind: "agent", id: "researcher" } as const;
const analyst = { kind: "agent", id: "analyst" } as const;
const editor = { kind: "agent", id: "editor" } as const;
const person = { kind: "human" } as const;

function message(
  runId: string,
  createdAt: number,
  from: Envelope["from"],
  to: Envelope["to"],
  text: string,
  intent: Envelope["intent"] = "work",
): Envelope {
  return {
    id: `${runId}:${createdAt}`,
    runId,
    channelId: to.kind === "agent" ? to.id : lead.id,
    from,
    to,
    parts: [{ type: "text", text }],
    trust: "operator",
    hop: 0,
    expectsReply: intent === "work",
    intent,
    cause: null,
    createdAt,
  };
}

function question(runId: string, lapsesAt: number | null): Approval {
  return {
    id: `${runId}:question`,
    agentId: researcher.id,
    groupId: "group",
    runId,
    request: { kind: "question", options: ["This year", "Next year"] },
    summary: "Which year?",
    detail: [],
    state: "pending",
    answer: null,
    createdAt: 10,
    decidedAt: null,
    lapsesAt,
  };
}

describe("task map", () => {
  it("puts questions before working colleagues and finished replies last", () => {
    const [map] = taskMaps(
      lead.id,
      [
        message("one", 1, person, lead, "Check the lease"),
        message("one", 2, lead, editor, "Check the wording"),
        message("one", 3, lead, analyst, "Check the numbers"),
        message("one", 4, lead, researcher, "Check the sources"),
        message("one", 5, editor, lead, "The wording is correct", "courtesy"),
      ],
      [question("one", 50)],
      new Set(["one"]),
    );
    expect(map?.branches.map((branch) => branch.agentId)).toEqual([
      researcher.id,
      analyst.id,
      editor.id,
    ]);
  });

  it("does not draw a map for a direct conversation or courtesy message", () => {
    expect(
      taskMaps(
        lead.id,
        [
          message("one", 1, person, lead, "Check the lease"),
          message("one", 2, lead, researcher, "Thanks", "courtesy"),
        ],
        [],
        new Set(["one"]),
      ),
    ).toEqual([]);
  });

  it("attaches cause and results to the run, not the latest task in the conversation", () => {
    const first = message("one", 1, person, lead, "Check the lease");
    const second = message("two", 2, person, lead, "Check the budget");
    const reply = message("one", 5, researcher, lead, "Lease", "courtesy");
    const maps = taskMaps(
      lead.id,
      [
        first,
        second,
        message("one", 3, lead, researcher, "Research the lease"),
        message("two", 4, lead, researcher, "Check the budget"),
        reply,
      ],
      [question("two", 50)],
      new Set(["one", "two"]),
    );
    expect(maps.map((map) => [map.brief.id, map.branches[0]?.reply?.id])).toEqual([
      [second.id, undefined],
      [first.id, reply.id],
    ]);
    expect(maps[0]?.branches[0]?.questions[0]?.id).toBe("two:question");
  });

  it("names nested delegation without treating the nested agent as a direct branch", () => {
    const maps = taskMaps(
      lead.id,
      [
        message("one", 1, person, lead, "Check the lease"),
        message("one", 2, lead, researcher, "Research the lease"),
        message("one", 3, researcher, analyst, "Check the numbers"),
      ],
      [],
      new Set(["one"]),
    );
    const branches = maps[0]?.branches;
    expect(branches?.map((branch) => branch.agentId)).toEqual([researcher.id]);
    expect(branches?.[0]?.delegatedTo).toEqual([analyst.id]);
  });

  it("keeps a colleague's result and hand-off through follow-up requests", () => {
    const reply = message("one", 4, researcher, lead, "Revenue rose.", "courtesy");
    const branches = taskMaps(
      lead.id,
      [
        message("one", 1, person, lead, "Check the lease"),
        message("one", 2, lead, researcher, "Check the figures"),
        message("one", 3, researcher, analyst, "Verify the figures"),
        reply,
        message("one", 5, lead, researcher, "Check last year too"),
        message("one", 6, researcher, lead, "Send me the report"),
      ],
      [],
      new Set(["one"]),
    )[0]?.branches;
    expect(branches?.[0]?.reply).toEqual(reply);
    expect(branches?.[0]?.delegatedTo).toEqual([analyst.id]);
  });
});

describe("fitting a task map by rows", () => {
  it("folds a branch only when one-line branches still do not fit", () => {
    expect(
      fitRows({
        room: 114.5,
        root: 57,
        beside: false,
        gap: 6,
        branches: [88, 58],
        lines: [88, 32],
      }),
    ).toEqual({ shown: 1, root: false, collapsed: true });
  });

  it("folds the third branch beside the brief when one-line branches still overflow", () => {
    expect(
      fitRows({
        room: 100,
        root: 90,
        beside: true,
        gap: 6,
        branches: [68, 68, 68],
        lines: [36, 36, 36],
      }),
    ).toEqual({ shown: 2, root: true, collapsed: true });
  });

  it("drops the brief and lead beside the branches only when they alone overflow", () => {
    expect(
      fitRows({ room: 63, root: 138, beside: true, gap: 6, branches: [32], lines: [32] }),
    ).toEqual({ shown: 1, root: false, collapsed: false });
  });

  it("collapses two working colleagues to one line each before hiding either", () => {
    expect(
      fitRows({
        room: 114.5,
        root: 57,
        beside: false,
        gap: 6,
        branches: [58, 58],
        lines: [32, 32],
      }),
    ).toEqual({ shown: 2, root: false, collapsed: true });
  });

  it("keeps a question whole and collapses the working colleague beside it", () => {
    expect(
      fitRows({
        room: 150.5,
        root: 114,
        beside: true,
        gap: 6,
        branches: [92, 68],
        lines: [92, 36],
      }),
    ).toEqual({ shown: 2, root: true, collapsed: true });
  });

  it("keeps working colleagues in full beside the brief and lead when they fit", () => {
    expect(
      fitRows({ room: 150, root: 142, beside: true, gap: 6, branches: [68, 68], lines: [36, 36] }),
    ).toEqual({ shown: 2, root: true, collapsed: false });
  });

  it("keeps the brief and lead above the branches when dropping them shows no more", () => {
    expect(
      fitRows({
        room: 179.7,
        root: 57,
        beside: false,
        gap: 6,
        branches: [32, 32],
        lines: [32, 32],
      }),
    ).toEqual({ shown: 2, root: true, collapsed: false });
  });
});
