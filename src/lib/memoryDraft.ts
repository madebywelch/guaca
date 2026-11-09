/** The versions of one page this panel can be holding at once. */
export interface Held {
  /** What is on disk, as of the last read. */
  stored: string;
  /** What the operator has typed since, or null when they have not. */
  draft: string | null;
  /**
   * A newer version that landed while they were typing, held rather than
   * applied. Null when nothing has.
   */
  incoming: string | null;
  version?: string;
  incomingVersion?: string;
}

/**
 * A version read off disk, against what the panel is already holding.
 *
 * The operator's unsaved text is never replaced by it. An agent rewrites this
 * file in the middle of a turn, and the operator is most likely to be editing
 * it exactly then, so a read that applied itself would take a sentence away as
 * it was being typed. It is held to one side instead and the panel says so:
 * keeping what you wrote is Save, and taking what the agent wrote is Cancel.
 *
 * A draft typed back to what is on disk is not an edit, and holding it as one
 * would leave the panel on a page the agent has since replaced.
 */
export function arrived(held: Held | null, content: string, version?: string): Held {
  if (held === null || held.draft === null || held.draft === (held.incoming ?? held.stored)) {
    return { stored: content, draft: null, incoming: null, ...(version && { version }) };
  }
  if (content === held.stored)
    return { ...held, version, incoming: null, incomingVersion: undefined };
  return { ...held, incoming: content, incomingVersion: version };
}

type Page = { held: Held; error: string | null; cut: boolean };
export type MemoryState =
  | { phase: "loading"; editing: boolean }
  | { phase: "failed"; error: string }
  | ({ phase: "view" | "editing" } & Page)
  | ({ phase: "saving"; sent: string } & Page);

export type MemoryAction =
  | { type: "read"; content: string; version?: string }
  | { type: "readFailed"; error: string }
  | { type: "edit" }
  | { type: "change"; value: string }
  | { type: "cancel" }
  | { type: "save"; sent: string }
  | { type: "saved"; saved: string }
  | { type: "version"; content: string; version: string }
  | { type: "saveFailed"; error: string }
  | { type: "conflict"; current: string; version: string };

export function memoryReducer(state: MemoryState, action: MemoryAction): MemoryState {
  switch (action.type) {
    case "read":
      if (state.phase === "loading" || state.phase === "failed") {
        return {
          phase: state.phase === "loading" && state.editing ? "editing" : "view",
          held: arrived(null, action.content, action.version),
          error: null,
          cut: false,
        };
      }
      if (state.phase === "saving" && state.held.draft !== state.sent) {
        return {
          ...state,
          held: { ...state.held, incoming: action.content, incomingVersion: action.version },
          error: null,
        };
      }
      return { ...state, held: arrived(state.held, action.content, action.version), error: null };
    case "readFailed":
      if (state.phase === "loading" || state.phase === "failed")
        return { phase: "failed", error: action.error };
      return { ...state, error: action.error };
    case "edit":
      if (state.phase === "loading") return { ...state, editing: true };
      if (state.phase === "view") return { ...state, phase: "editing", cut: false };
      return state;
    case "change":
      if (state.phase !== "editing" && state.phase !== "saving") return state;
      return { ...state, held: { ...state.held, draft: action.value }, cut: false };
    case "cancel":
      if (state.phase !== "editing") return state;
      return {
        ...state,
        phase: "view",
        held: {
          stored: state.held.incoming ?? state.held.stored,
          draft: null,
          incoming: null,
          version: state.held.incoming !== null ? state.held.incomingVersion : state.held.version,
        },
        cut: false,
      };
    case "save":
      if (state.phase !== "editing") return state;
      return { ...state, phase: "saving", sent: action.sent, error: null };
    case "saved": {
      if (state.phase !== "saving") return state;
      const draft = state.held.draft === state.sent ? null : state.held.draft;
      return {
        phase: draft === null ? "view" : "editing",
        held: { stored: action.saved, draft, incoming: null, version: state.held.version },
        error: null,
        cut: action.saved !== state.sent.trim(),
      };
    }
    case "saveFailed":
      if (state.phase !== "saving") return state;
      return { ...state, phase: "editing", error: action.error };
    case "version":
      if (!("held" in state) || state.held.stored !== action.content) return state;
      return { ...state, held: { ...state.held, version: action.version } };
    case "conflict":
      if (state.phase !== "saving") return state;
      return {
        ...state,
        phase: "editing",
        held: { ...state.held, incoming: action.current, incomingVersion: action.version },
        error: null,
      };
  }
}

const drafts = new Map<string, Draft>();

type Draft = {
  getSnapshot: () => MemoryState;
  subscribe: (listener: () => void) => () => void;
  dispatch: (action: MemoryAction) => void;
  generation: number;
};

export function memoryDraft(agentId: string): Draft {
  const existing = drafts.get(agentId);
  if (existing) return existing;

  let state: MemoryState = { phase: "loading", editing: false };
  const listeners = new Set<() => void>();
  const draft: Draft = {
    generation: 0,
    getSnapshot: () => state,
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    dispatch: (action) => {
      state = memoryReducer(state, action);
      if (
        state.phase === "saving" ||
        (state.phase === "editing" &&
          state.held.draft !== null &&
          state.held.draft !== (state.held.incoming ?? state.held.stored))
      ) {
        drafts.set(agentId, draft);
      } else if (drafts.get(agentId) === draft) {
        drafts.delete(agentId);
      }
      for (const listener of listeners) listener();
    },
  };
  return draft;
}
