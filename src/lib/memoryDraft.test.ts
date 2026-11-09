import { describe, expect, it } from "vitest";

import { memoryDraft } from "./memoryDraft";

describe("memory draft ownership", () => {
  it("retains the original text when it is the draft in conflict", () => {
    const draft = memoryDraft("conflict-original");
    draft.dispatch({ type: "read", content: "old", version: "old-version" });
    draft.dispatch({ type: "edit" });
    draft.dispatch({ type: "change", value: "sent" });
    draft.dispatch({ type: "save", sent: "sent" });
    draft.dispatch({ type: "change", value: "old" });
    draft.dispatch({ type: "conflict", current: "agent", version: "agent-version" });
    expect(memoryDraft("conflict-original")).toBe(draft);
    expect(draft.getSnapshot()).toMatchObject({
      held: { draft: "old", incoming: "agent", incomingVersion: "agent-version" },
    });
    draft.dispatch({ type: "cancel" });
  });
  it("retains a pending save and later edits across remounts, then releases a settled draft", () => {
    const first = memoryDraft("draft-test");
    first.dispatch({ type: "read", content: "old" });
    first.dispatch({ type: "edit" });
    first.dispatch({ type: "change", value: "sent" });
    first.dispatch({ type: "save", sent: "sent" });
    first.dispatch({ type: "change", value: "later" });
    expect(memoryDraft("draft-test")).toBe(first);
    first.dispatch({ type: "saved", saved: "sent" });
    expect(memoryDraft("draft-test").getSnapshot()).toMatchObject({
      phase: "editing",
      held: { stored: "sent", draft: "later" },
    });
    first.dispatch({ type: "save", sent: "later" });
    first.dispatch({ type: "saved", saved: "later" });
    expect(memoryDraft("draft-test")).not.toBe(first);
  });

  it("isolates agents, notifies subscribers, and releases canceled drafts", () => {
    const first = memoryDraft("one");
    const second = memoryDraft("two");
    let calls = 0;
    const unsubscribe = first.subscribe(() => {
      calls++;
    });
    first.dispatch({ type: "read", content: "old" });
    first.dispatch({ type: "edit" });
    first.dispatch({ type: "change", value: "mine" });
    expect(calls).toBe(3);
    expect(second.getSnapshot().phase).toBe("loading");
    expect(memoryDraft("one")).toBe(first);
    unsubscribe();
    first.dispatch({ type: "cancel" });
    expect(calls).toBe(3);
    expect(memoryDraft("one")).not.toBe(first);
  });
});
