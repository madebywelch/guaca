import { beforeEach, describe, expect, it } from "vitest";
import { type DraftState, readDraftState, reduceDraft, saveDraftState } from "./draft";

const file = (digest: string) => ({ digest, name: `${digest}.txt`, mime: "text/plain", bytes: 2 });

describe("draft ownership", () => {
  beforeEach(() => sessionStorage.clear());

  it("persists the operation key and preserves a later draft during retry", () => {
    const initial = reduceDraft(readDraftState("keyed"), { type: "text", text: "first" });
    const sending = reduceDraft(initial, { type: "send", key: "operation" });
    const later = reduceDraft(sending, { type: "text", text: "later" });
    saveDraftState("keyed", later);
    const restored = readDraftState("keyed");
    expect(restored.delivery).toEqual({
      kind: "unknown",
      payload: initial.draft,
      key: "operation",
      restored: false,
    });
    const retry = reduceDraft(restored, { type: "retry" });
    expect(retry.draft).toEqual(later.draft);
    expect(retry.delivery).toEqual({ kind: "sending", payload: initial.draft, key: "operation" });
  });

  it("clears only the restored payload on retry, preserving subsequent edits", () => {
    const initial = reduceDraft(readDraftState("retry"), { type: "text", text: "first" });
    const sending = reduceDraft(initial, { type: "send", key: "operation" });
    const unknown = reduceDraft(sending, { type: "unknown" });
    const retry = reduceDraft(unknown, { type: "retry" });
    expect(retry.draft).toEqual({ text: "", files: [] });
    expect(reduceDraft(retry, { type: "unknown" }).draft).toEqual(initial.draft);
    const edited = reduceDraft(unknown, { type: "text", text: "changed" });
    expect(reduceDraft(edited, { type: "retry" }).draft.text).toBe("changed");
  });

  it("reads the previous draft format", () => {
    sessionStorage.setItem("old", JSON.stringify({ text: "old text", files: [] }));
    expect(readDraftState("old")).toEqual({
      draft: { text: "old text", files: [] },
      delivery: { kind: "idle" },
      staging: 0,
      held: [],
    });
  });

  it("reads an interrupted send as unknown without replaying it", () => {
    const initial = readDraftState("new");
    const first = reduceDraft(initial, { type: "text", text: "first" });
    const sending = reduceDraft(first, { type: "send" });
    const later = reduceDraft(sending, { type: "text", text: "next" });
    saveDraftState("new", later);
    expect(readDraftState("new")).toEqual({
      draft: { text: "next", files: [] },
      delivery: { kind: "unknown", payload: { text: "first", files: [] }, restored: false },
      staging: 0,
      held: [],
    });
  });

  it("reduces text, files, staging, send, and success without touching held drafts", () => {
    const a = file("a".repeat(64));
    const b = file("b".repeat(64));
    const initial = readDraftState("transitions");
    const text = reduceDraft(initial, { type: "text", text: "hello" });
    const files = reduceDraft(text, { type: "files", files: [a] });
    const staging = reduceDraft(reduceDraft(files, { type: "stage" }), { type: "stage" });
    expect(staging.staging).toBe(2);
    const staged = reduceDraft(staging, { type: "staged", files: [a, b, b] });
    expect(staged.staging).toBe(1);
    expect(staged.draft.files).toEqual([a, b]);
    const finished = reduceDraft(staged, { type: "staged", files: [] });
    expect(finished.staging).toBe(0);
    const sending = reduceDraft(finished, { type: "send" });
    expect(sending.delivery).toEqual({ kind: "sending", payload: finished.draft });
    expect(sending.draft).toEqual({ text: "", files: [] });
    const later = reduceDraft(sending, { type: "text", text: "next" });
    expect(reduceDraft(later, { type: "success" })).toEqual({
      ...later,
      delivery: { kind: "idle" },
    });
  });

  it("preserves an unconfirmed payload separately when a later draft exists", () => {
    const initial = readDraftState("failure");
    const first = reduceDraft(initial, { type: "text", text: "first" });
    const sending = reduceDraft(first, { type: "send" });
    const next = reduceDraft(sending, { type: "text", text: "next" });
    const unknown = reduceDraft(next, { type: "unknown" });
    expect(unknown.delivery).toEqual({ kind: "unknown", payload: first.draft, restored: false });
    const checked = reduceDraft(unknown, { type: "dismiss" });
    expect(checked.draft).toEqual(next.draft);
    expect(checked.held).toEqual([first.draft]);
    saveDraftState("failure", checked);
    expect(readDraftState("failure")).toEqual(checked);
    const swapped = reduceDraft(checked, { type: "switch", index: 0 });
    expect(swapped.draft).toEqual(first.draft);
    expect(swapped.held).toEqual([next.draft]);
    expect(reduceDraft(swapped, { type: "switch", index: 0 })).toEqual(checked);
  });

  it("restores a rejected payload into an empty draft without duplicating it", () => {
    const initial = readDraftState("empty");
    const first = reduceDraft(initial, { type: "text", text: "first" });
    const unknown = reduceDraft(reduceDraft(first, { type: "send" }), { type: "unknown" });
    expect(unknown.draft).toEqual(first.draft);
    expect(unknown.delivery).toEqual({ kind: "unknown", payload: first.draft, restored: true });
    const checked = reduceDraft(unknown, { type: "dismiss" });
    expect(checked.draft).toEqual(first.draft);
    expect(checked.held).toEqual([]);
    expect(reduceDraft(checked, { type: "unknown" })).toEqual(checked);
    expect(reduceDraft(checked, { type: "switch", index: 0 })).toEqual(checked);
  });

  it("retains identical but separately composed messages", () => {
    const initial = readDraftState("identical");
    const first = reduceDraft(initial, { type: "text", text: "same" });
    const second = reduceDraft(reduceDraft(first, { type: "send" }), {
      type: "text",
      text: "same",
    });
    expect(reduceDraft(reduceDraft(second, { type: "unknown" }), { type: "dismiss" }).held).toEqual(
      [first.draft],
    );
  });

  it("retains an edited restored payload and supports switching from an empty draft", () => {
    const initial = readDraftState("edited");
    const first = reduceDraft(initial, { type: "text", text: "first" });
    const unknown = reduceDraft(reduceDraft(first, { type: "send" }), { type: "unknown" });
    const edited = reduceDraft(unknown, { type: "text", text: "edited" });
    const checked = reduceDraft(edited, { type: "dismiss" });
    expect(checked.held).toEqual([first.draft]);
    const empty: DraftState = { ...checked, draft: { text: "", files: [] } };
    expect(reduceDraft(empty, { type: "switch", index: 0 }).held).toEqual([]);
  });

  it("refuses invalid delivery and staging transitions", () => {
    const idle = readDraftState("invalid");
    expect(reduceDraft(idle, { type: "success" })).toEqual(idle);
    expect(reduceDraft(idle, { type: "dismiss" })).toEqual(idle);
    expect(reduceDraft(idle, { type: "staged", files: [] })).toEqual(idle);
    const staging = reduceDraft(idle, { type: "stage" });
    expect(reduceDraft(staging, { type: "send" })).toEqual(staging);
    const sending = reduceDraft(reduceDraft(idle, { type: "text", text: "first" }), {
      type: "send",
    });
    expect(reduceDraft(sending, { type: "send" })).toEqual(sending);
    expect(reduceDraft(sending, { type: "dismiss" })).toEqual(sending);
    const unknown = reduceDraft(sending, { type: "unknown" });
    expect(reduceDraft(unknown, { type: "unknown" })).toEqual(unknown);
    expect(reduceDraft(unknown, { type: "success" })).toEqual(unknown);
  });

  it("does not switch a held draft during a send or attachment staging", () => {
    const idle = { ...readDraftState("switch-guard"), held: [{ text: "saved", files: [] }] };
    const staging = reduceDraft(idle, { type: "stage" });
    expect(reduceDraft(staging, { type: "switch", index: 0 })).toEqual(staging);
    const sending = reduceDraft(idle, { type: "send" });
    expect(reduceDraft(sending, { type: "switch", index: 0 })).toEqual(sending);
  });
});
