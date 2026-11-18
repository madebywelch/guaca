import { create } from "zustand";

import type { AgentId } from "./types";

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

if (typeof window !== "undefined") {
  window.addEventListener("pagehide", () => useHeld.getState().release());
}
