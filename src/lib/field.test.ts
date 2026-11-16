import { describe, expect, it } from "vitest";

import { anAgent, anApproval } from "../test-fixtures";
import { addressed, defaultTarget, waitingQuestion, writable } from "./field";

const dara = anAgent("dara", { name: "Dara", groupId: "studio" });
const ines = anAgent("ines", { name: "Ines", groupId: "studio" });
const inesMarie = anAgent("ines-marie", { name: "Ines Marie", groupId: "studio" });
const noor = anAgent("noor", { name: "Noor", groupId: "home" });
const question = { kind: "question" as const, options: [] };
const gone = anAgent("gone", { name: "Gone", lifecycle: "terminated" });

describe("who the field writes to", () => {
  it("never offers or picks a deleted agent", () => {
    expect(writable([gone, dara]).map((agent) => agent.id)).toEqual(["dara"]);
    expect(defaultTarget([gone], { gone: 5 }, null)).toBeUndefined();
    expect(defaultTarget([gone, dara], { gone: 9 }, null)?.id).toBe("dara");
  });

  it("picks whoever was talked to last in the group the operator is in", () => {
    const last = { dara: 3, ines: 2, noor: 9 };
    expect(defaultTarget([dara, ines, noor], last, "studio")?.id).toBe("dara");
  });

  it("falls back to whoever was talked to last anywhere, then to the first", () => {
    expect(defaultTarget([dara, noor], { noor: 9 }, "studio")?.id).toBe("noor");
    expect(defaultTarget([dara, noor], {}, null)?.id).toBe("dara");
    expect(defaultTarget([dara, noor], {}, "home")?.id).toBe("noor");
  });
});

describe("the question Tab answers", () => {
  it("is never a permission, which takes a verdict and not words", () => {
    const allow = anApproval({
      agentId: "agent-1",
      request: { kind: "permission", action: "createAgent" },
    });
    expect(waitingQuestion([allow])).toBeUndefined();
  });

  it("is never one that has been settled", () => {
    expect(
      waitingQuestion([anApproval({ agentId: "agent-1", request: question, state: "answered" })]),
    ).toBeUndefined();
  });

  it("is the oldest pending question", () => {
    const later = anApproval({ agentId: "agent-1", request: question, id: "later", createdAt: 5 });
    const first = anApproval({ agentId: "agent-1", request: question, id: "first", createdAt: 2 });
    expect(waitingQuestion([later, first])?.id).toBe("first");
  });
});

describe("an @ at the start of the field", () => {
  it("does nothing for an @ in the middle, or a name nobody has", () => {
    expect(addressed("ask @Ines about it", [ines])).toBeUndefined();
    expect(addressed("@Nobody draft it", [ines])).toBeUndefined();
  });

  it("does nothing for a name with no message after it, or a deleted agent", () => {
    expect(addressed("@Ines", [ines])).toBeUndefined();
    expect(addressed("@Ines   ", [ines])).toBeUndefined();
    expect(addressed("@Gone draft it", [gone])).toBeUndefined();
  });

  it("does not read a longer word that starts with a name as that name", () => {
    expect(addressed("@Inesta draft it", [ines])).toBeUndefined();
  });

  it("addresses that agent and sends the rest, whatever the case", () => {
    expect(addressed("@ines draft the note", [dara, ines])).toEqual({
      agent: ines,
      text: "draft the note",
    });
  });

  it("takes the longest name when one name starts another", () => {
    expect(addressed("@Ines Marie draft it", [ines, inesMarie])?.agent.id).toBe("ines-marie");
  });
});
