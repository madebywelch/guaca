import { describe, expect, it } from "vitest";
import { anAgent, anApproval, DEFAULT_GROUP } from "../test-fixtures";
import { nextQuestion, openingConversation } from "./frame";
import type { Running } from "./types";

const order = ["owner", "colleague", "writer"].map((id) => anAgent(id));
const active = (agentId: string, runId: string, startedAt: number): Running => ({
  agentId,
  runId,
  startedAt,
  groupId: DEFAULT_GROUP,
  note: null,
});

describe("which conversation opens", () => {
  it("opens the owner of the soonest-due question rather than the colleague who asked it", () => {
    const early = anApproval({
      id: "early",
      agentId: "colleague",
      runId: "delegated",
      lapsesAt: 200,
    });
    const late = anApproval({ id: "late", agentId: "writer", runId: "direct", lapsesAt: 400 });
    const running = [active("owner", "delegated", 10), active("writer", "direct", 20)];
    expect(nextQuestion([late, early], [])?.id).toBe("early");
    expect(openingConversation(order, [late, early], [], running, { writer: 100 }, "writer")).toBe(
      "owner",
    );
  });

  it("uses the asker when its task owner is gone and ignores questions with no live conversation", () => {
    const gone = anApproval({ agentId: "gone", runId: "closed", lapsesAt: 100 });
    const live = anApproval({ agentId: "writer", runId: "delegated", lapsesAt: null });
    const running = [active("gone", "delegated", 1)];
    expect(openingConversation(order, [gone, live], [], running, {}, "owner")).toBe("writer");
  });

  it("uses the most active running conversation before falling back to the last open agent", () => {
    const running = [active("owner", "one", 20), active("colleague", "two", 10)];
    expect(openingConversation(order, [], [], running, { colleague: 30 }, "writer")).toBe(
      "colleague",
    );
    expect(openingConversation(order, [], [], [], {}, "writer")).toBe("writer");
    expect(openingConversation(order, [], [], [], {}, "gone")).toBe("owner");
    expect(openingConversation([], [], [], [], {}, "gone")).toBeUndefined();
  });
});
