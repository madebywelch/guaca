import { useMemo } from "react";
import { create } from "zustand";

import { forYouCount, useStore } from "./store";
import type { Activity, AgentId, Approval, Escalation } from "./types";

export const HOLD_MS = 5000;

export type Notice =
  | { kind: "held"; id: string; text: string }
  | { kind: "sent"; agentId: AgentId; text: string };

interface Held {
  id: string;
  send: () => Promise<void>;
}

interface HeldState {
  held: Held | null;
  notice: Notice | null;
  hold: (id: string, text: string, send: () => Promise<void>) => void;
  undo: () => boolean;
  release: () => void;
  tell: (notice: Notice) => void;
  dismiss: () => void;
}

let timer: ReturnType<typeof setTimeout> | undefined;

function restart(then: () => void) {
  clearTimeout(timer);
  timer = setTimeout(then, HOLD_MS);
}

export const useHeld = create<HeldState>((set, get) => ({
  held: null,
  notice: null,

  hold(id, text, send) {
    get().release();
    set({ held: { id, send }, notice: { kind: "held", id, text } });
    restart(() => get().release());
  },

  undo() {
    if (!get().held) return false;
    clearTimeout(timer);
    set({ held: null, notice: null });
    return true;
  },

  release() {
    const held = get().held;
    if (!held) return;
    clearTimeout(timer);
    set({ held: null, notice: null });
    void held.send();
  },

  tell(notice) {
    get().release();
    set({ notice });
    restart(() => set({ notice: null }));
  },

  dismiss() {
    if (get().held) return get().release();
    clearTimeout(timer);
    set({ notice: null });
  },
}));

export interface Shown {
  pending: Approval[];
  stuck: Escalation[];
  activity: Record<string, Activity>;
  sending: AgentId | null;
}

export function shown(
  pending: Approval[],
  stuck: Escalation[],
  activity: Record<string, Activity>,
  heldId: string | undefined,
): Shown {
  const open = pending.filter((one) => one.id !== heldId);
  const sending = pending.find((one) => one.id === heldId)?.agentId ?? null;
  const still = sending !== null && open.some((one) => one.agentId === sending);
  const answered = sending !== null && !still && activity[sending]?.state === "awaitingApproval";
  return {
    pending: open,
    stuck: stuck.filter((one) => one.id !== heldId),
    activity: answered ? { ...activity, [sending]: { state: "thinking" } } : activity,
    sending: answered ? sending : null,
  };
}

export function useShown(): Shown {
  const heldId = useHeld((s) => s.held?.id);
  const pending = useStore((s) => s.pending);
  const stuck = useStore((s) => s.stuck);
  const activity = useStore((s) => s.activity);
  return useMemo(() => shown(pending, stuck, activity, heldId), [pending, stuck, activity, heldId]);
}

export function useForYouCount(): number {
  const heldId = useHeld((s) => s.held?.id);
  return useStore((state) => {
    const held =
      heldId !== undefined &&
      (state.pending.some((one) => one.id === heldId) ||
        state.stuck.some((one) => one.id === heldId));
    return forYouCount(state) - (held ? 1 : 0);
  });
}

if (typeof window !== "undefined") {
  window.addEventListener("pagehide", () => useHeld.getState().release());
}
