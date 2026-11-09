import type { Attachment } from "./types";
export interface Draft {
  text: string;
  files: Attachment[];
}
export type Delivery =
  | { kind: "idle" }
  | { kind: "sending"; payload: Draft; key?: string }
  | { kind: "unknown"; payload: Draft; key?: string; restored: boolean };
export interface DraftState {
  draft: Draft;
  delivery: Delivery;
  staging: number;
  held: Draft[];
}
export type DraftAction =
  | { type: "text"; text: string }
  | { type: "files"; files: Attachment[] }
  | { type: "stage" }
  | { type: "staged"; files: Attachment[] }
  | { type: "send"; key?: string }
  | { type: "retry" }
  | { type: "success" }
  | { type: "unknown" }
  | { type: "dismiss" }
  | { type: "switch"; index: number };

export function reduceDraft(state: DraftState, action: DraftAction): DraftState {
  switch (action.type) {
    case "text":
      return { ...state, draft: { ...state.draft, text: action.text } };
    case "files":
      return { ...state, draft: { ...state.draft, files: action.files } };
    case "stage":
      return { ...state, staging: state.staging + 1 };
    case "staged": {
      if (!state.staging) return state;
      const seen = new Set(state.draft.files.map((file) => file.digest));
      const attached = action.files.filter((file) => {
        if (seen.has(file.digest)) return false;
        seen.add(file.digest);
        return true;
      });
      return {
        ...state,
        staging: state.staging - 1,
        draft: {
          ...state.draft,
          files: [...state.draft.files, ...attached],
        },
      };
    }
    case "send":
      if (state.delivery.kind !== "idle" || state.staging) return state;
      return {
        ...state,
        draft: { text: "", files: [] },
        delivery: {
          kind: "sending",
          payload: state.draft,
          ...(action.key ? { key: action.key } : {}),
        },
      };
    case "retry": {
      if (state.delivery.kind !== "unknown" || !state.delivery.key || state.staging) return state;
      const { payload, key, restored } = state.delivery;
      const same = restored && JSON.stringify(state.draft) === JSON.stringify(payload);
      return {
        ...state,
        draft: same ? { text: "", files: [] } : state.draft,
        delivery: { kind: "sending", payload, key },
      };
    }
    case "success":
      if (state.delivery.kind !== "sending") return state;
      return { ...state, delivery: { kind: "idle" } };
    case "unknown":
      return state.delivery.kind === "sending"
        ? {
            ...state,
            draft:
              !state.draft.text && !state.draft.files.length ? state.delivery.payload : state.draft,
            delivery: {
              ...state.delivery,
              kind: "unknown",
              restored: !state.draft.text && !state.draft.files.length,
            },
          }
        : state;
    case "dismiss": {
      if (state.delivery.kind !== "unknown") return state;
      const same =
        state.delivery.restored &&
        JSON.stringify(state.draft) === JSON.stringify(state.delivery.payload);
      return {
        ...state,
        delivery: { kind: "idle" },
        held: same ? state.held : [...state.held, state.delivery.payload],
      };
    }
    case "switch": {
      if (state.delivery.kind !== "idle" || state.staging || !state.held[action.index])
        return state;
      const held = [...state.held];
      const draft = held[action.index]!;
      if (state.draft.text || state.draft.files.length) held[action.index] = state.draft;
      else held.splice(action.index, 1);
      return { ...state, draft, held };
    }
  }
}

export function readDraftState(key: string): DraftState {
  const draft = readDraft(key);
  try {
    const value = JSON.parse(sessionStorage.getItem(key) ?? "null");
    const held: Draft[] = Array.isArray(value?.held) ? value.held.filter(validDraft) : [];
    if (value?.delivery?.payload) {
      const payload = value.delivery.payload;
      if (validDraft(payload))
        return {
          draft,
          held,
          staging: 0,
          delivery: {
            kind: "unknown",
            payload,
            ...(typeof value.delivery.key === "string" && value.delivery.key
              ? { key: value.delivery.key }
              : {}),
            restored: value.delivery.kind === "unknown" && value.delivery.restored === true,
          },
        };
    }
    return { draft, held, staging: 0, delivery: { kind: "idle" } };
  } catch {}
  return { draft, held: [], staging: 0, delivery: { kind: "idle" } };
}

export function saveDraftState(key: string, state: DraftState) {
  if (state.delivery.kind === "idle" && !state.held.length) return saveDraft(key, state.draft);
  sessionStorage.setItem(
    key,
    JSON.stringify({ ...state.draft, delivery: state.delivery, held: state.held }),
  );
}
function validDraft(value: unknown): value is Draft {
  if (!value || typeof value !== "object") return false;
  const draft = value as Draft;
  return (
    typeof draft.text === "string" &&
    Array.isArray(draft.files) &&
    draft.files.every(
      (file) =>
        file &&
        typeof file.digest === "string" &&
        /^[a-f0-9]{64}$/.test(file.digest) &&
        typeof file.name === "string" &&
        typeof file.mime === "string" &&
        Number.isSafeInteger(file.bytes) &&
        file.bytes >= 0,
    )
  );
}
export function readDraft(key: string): Draft {
  try {
    const value = JSON.parse(sessionStorage.getItem(key) ?? "null");
    if (validDraft(value)) return { text: value.text, files: value.files };
  } catch {
    /* An unavailable session store does not prevent composition. */
  }
  return { text: "", files: [] };
}
export function saveDraft(key: string, draft: Draft) {
  if (draft.text || draft.files.length) sessionStorage.setItem(key, JSON.stringify(draft));
  else sessionStorage.removeItem(key);
}
