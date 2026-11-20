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
  inFlight: string[];
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
  inFlight: [],
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
    set({ held: null, notice: null, inFlight: [...get().inFlight, held.id] });
    void held
      .send()
      .finally(() => set({ inFlight: get().inFlight.filter((id) => id !== held.id) }));
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
  sending: AgentId[];
}

export function shown(
  pending: Approval[],
  stuck: Escalation[],
  activity: Record<string, Activity>,
  hidden: readonly string[],
): Shown {
  const gone = new Set(hidden);
  const open = pending.filter((one) => !gone.has(one.id));
  const sending = [
    ...new Set(pending.filter((one) => gone.has(one.id)).map((one) => one.agentId)),
  ].filter(
    (agent) =>
      !open.some((one) => one.agentId === agent) && activity[agent]?.state === "awaitingApproval",
  );
  return {
    pending: open,
    stuck: stuck.filter((one) => !gone.has(one.id)),
    activity:
      sending.length === 0
        ? activity
        : {
            ...activity,
            ...Object.fromEntries(sending.map((agent) => [agent, { state: "thinking" }])),
          },
    sending,
  };
}

export function hiddenNow(): string[] {
  const { held, inFlight } = useHeld.getState();
  return held ? [...inFlight, held.id] : inFlight;
}

function useHidden(): readonly string[] {
  const heldId = useHeld((s) => s.held?.id);
  const inFlight = useHeld((s) => s.inFlight);
  return useMemo(
    () => (heldId === undefined ? inFlight : [...inFlight, heldId]),
    [heldId, inFlight],
  );
}

export function useShown(): Shown {
  const hidden = useHidden();
  const pending = useStore((s) => s.pending);
  const stuck = useStore((s) => s.stuck);
  const activity = useStore((s) => s.activity);
  return useMemo(() => shown(pending, stuck, activity, hidden), [pending, stuck, activity, hidden]);
}

export function useForYouCount(): number {
  const hidden = useHidden();
  return useStore((state) => {
    const gone = hidden.filter(
      (id) =>
        state.pending.some((one) => one.id === id) || state.stuck.some((one) => one.id === id),
    );
    return forYouCount(state) - gone.length;
  });
}

if (typeof window !== "undefined") {
  window.addEventListener("pagehide", () => useHeld.getState().release());
}
