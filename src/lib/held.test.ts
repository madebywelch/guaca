import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { anApproval } from "../test-fixtures";
import { HOLD_MS, shown, useForYouCount, useHeld } from "./held";
import { useStore } from "./store";
import type { Activity, Escalation } from "./types";

const stuck = { id: "esc-1", agentId: "scout" } as Escalation;

describe("what a held answer leaves on screen", () => {
  it("shows nothing at all when nothing is held", () => {
    const pending = [anApproval({ agentId: "pip" })];
    const activity: Record<string, Activity> = { pip: { state: "awaitingApproval" } };
    const view = shown(pending, [stuck], activity, []);
    expect(view).toEqual({ pending, stuck: [stuck], activity, sending: [] });
  });

  it("drops the held question, and the agent waits on nobody while its answer is being sent", () => {
    const asked = anApproval({ agentId: "pip" });
    const view = shown([asked], [], { pip: { state: "awaitingApproval" } }, [asked.id]);
    expect(view.pending).toEqual([]);
    expect(view.activity.pip).toEqual({ state: "thinking" });
    expect(view.sending).toEqual(["pip"]);
  });

  it("keeps an agent waiting that still has another question open", () => {
    const first = anApproval({ agentId: "pip", id: "req-a" as never });
    const second = anApproval({ agentId: "pip", id: "req-b" as never });
    const view = shown([first, second], [], { pip: { state: "awaitingApproval" } }, [first.id]);
    expect(view.pending).toEqual([second]);
    expect(view.activity.pip).toEqual({ state: "awaitingApproval" });
    expect(view.sending).toEqual([]);
  });

  it("drops a stuck agent whose clear is held, and leaves everyone else as they are", () => {
    const view = shown([], [stuck], { scout: { state: "idle" } }, [stuck.id]);
    expect(view.stuck).toEqual([]);
    expect(view.activity).toEqual({ scout: { state: "idle" } });
  });
});

describe("the For you count while an answer is held and then sent", () => {
  const asking = anApproval({ agentId: "pip" });

  beforeEach(() => {
    vi.useFakeTimers();
    useHeld.setState({ held: null, inFlight: [], notice: null });
    useStore.setState({ pending: [asking], stuck: [], decisions: [] });
  });

  afterEach(() => {
    vi.useRealTimers();
    useStore.setState({ pending: [] });
  });

  it("keeps the question answered after Undo runs out, until the host has taken the answer", async () => {
    let land = () => {};
    const send = () => new Promise<void>((resolve) => (land = resolve));
    const { result } = renderHook(() => useForYouCount());
    expect(result.current).toBe(1);
    act(() => useHeld.getState().hold(asking.id, "Allowed what Pip asked.", send));
    expect(result.current).toBe(0);
    act(() => vi.advanceTimersByTime(HOLD_MS));
    expect(result.current).toBe(0);
    await act(async () => land());
    expect(result.current).toBe(1);
  });

  it("subtracts nothing for a held answer whose question is already gone", () => {
    const { result } = renderHook(() => useForYouCount());
    act(() => useHeld.getState().hold("req-gone", "Allowed what Pip asked.", async () => {}));
    expect(result.current).toBe(1);
  });
});
