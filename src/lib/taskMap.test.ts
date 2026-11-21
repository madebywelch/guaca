import { describe, expect, it } from "vitest";
import { taskMaps } from "./taskMap";
import type { Approval, Envelope } from "./types";

const lead = { kind: "agent", id: "lead" } as const;
const researcher = { kind: "agent", id: "researcher" } as const;
const analyst = { kind: "agent", id: "analyst" } as const;
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
