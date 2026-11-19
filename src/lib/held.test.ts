import { describe, expect, it } from "vitest";

import { anApproval } from "../test-fixtures";
import { shown } from "./held";
import type { Activity, Escalation } from "./types";

const stuck = { id: "esc-1", agentId: "scout" } as Escalation;

describe("what a held answer leaves on screen", () => {
  it("shows nothing at all when nothing is held", () => {
    const pending = [anApproval({ agentId: "pip" })];
    const activity: Record<string, Activity> = { pip: { state: "awaitingApproval" } };
    const view = shown(pending, [stuck], activity, undefined);
    expect(view).toEqual({ pending, stuck: [stuck], activity, sending: null });
  });

  it("drops the held question, and the agent waits on nobody while its answer is being sent", () => {
    const asked = anApproval({ agentId: "pip" });
    const view = shown([asked], [], { pip: { state: "awaitingApproval" } }, asked.id);
    expect(view.pending).toEqual([]);
    expect(view.activity.pip).toEqual({ state: "thinking" });
    expect(view.sending).toBe("pip");
  });

  it("keeps an agent waiting that still has another question open", () => {
    const first = anApproval({ agentId: "pip", id: "req-a" as never });
    const second = anApproval({ agentId: "pip", id: "req-b" as never });
    const view = shown([first, second], [], { pip: { state: "awaitingApproval" } }, first.id);
    expect(view.pending).toEqual([second]);
    expect(view.activity.pip).toEqual({ state: "awaitingApproval" });
    expect(view.sending).toBeNull();
  });

  it("drops a stuck agent whose clear is held, and leaves everyone else as they are", () => {
    const view = shown([], [stuck], { scout: { state: "idle" } }, stuck.id);
    expect(view.stuck).toEqual([]);
    expect(view.activity).toEqual({ scout: { state: "idle" } });
  });
});
