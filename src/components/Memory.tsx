import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";

import { api } from "../lib/ipc";
import { memoryDraft } from "../lib/memoryDraft";
import { useStore } from "../lib/store";
import { type AgentId, errorMessage } from "../lib/types";

export { arrived, type MemoryState, memoryReducer } from "../lib/memoryDraft";

/**
 * The cap the runtime enforces, mirrored here so the operator is warned before
 * they lose anything rather than after.
 *
 * Pinned to `MAX_MEMORY` by the suite, which reads both sources and compares
 * them, exactly as `ipc.contract.test.ts` does across the same seam. It was
 * written as advisory on the reasoning that this panel puts back whatever came
 * home from the write, so a drifted number cannot cost the operator their
 * text. True, and not the failure: a low copy tells an operator "the end is
 * cut on save" about a page the runtime stores whole. They then edit down a
 * memory that was never in danger, which is worse than no warning at all,
 * because a warning is read as a fact about what the runtime is going to do.
 * The number is only worth drawing while it is the runtime's number.
 */
export const CAP = 16_000;

/** How near the cap is near enough to say so. */
const ROOM = 400;

/** Characters as the runtime counts them, which is not what `length` counts. */
function characters(text: string): number {
  return [...text].length;
}

/**
 * How full the memory is, said only once it is nearly full.
 *
 * A running count under every agent's memory is a number nobody reads. What is
 * worth knowing is the one state that has a consequence: past the cap the
 * runtime cuts the end off, and an agent that has reached it is already
 * throwing things away to make room for what it learns next.
 *
 * `over` is the difference between a fact and a loss, and it is what the two
 * are drawn apart by. Room running out is worth a glance; room gone means the
 * end of what is on screen is not going to be stored.
 */
export function crowding(text: string): { text: string; over: boolean } | null {
  const length = characters(text);
  if (length > CAP) {
    return {
      text: `${(length - CAP).toLocaleString()} characters over. The end is cut on save.`,
      over: true,
    };
  }
  if (length > CAP - ROOM) {
    return { text: `${(CAP - length).toLocaleString()} characters left.`, over: false };
  }
  return null;
}

interface Props {
  agentId: AgentId;
}

/**
 * An agent's memory, in the column beside it.
 *
 * It used to be a field two thirds of the way down the profile dialog, which
 * put the one thing about an agent that changes on its own behind a modal you
 * open to change the things that do not. It is read constantly and written
 * rarely, which is the panel's half of the split the dialog holds the other
 * side of, and it is the only section here that every agent has something in.
 *
 * The stored page is read first. Editing is a deliberate step, while the
 * textarea still shows the markdown characters the agent will actually read.
 * A rewrite arriving under that draft is held until Save or Cancel decides it.
 */
export function Memory({ agentId }: Props) {
  return <MemoryPage key={agentId} agentId={agentId} />;
}

function MemoryPage({ agentId }: Props) {
  const [draft] = useState(() => memoryDraft(agentId));
  const state = useSyncExternalStore(draft.subscribe, draft.getSnapshot);
  const { dispatch } = draft;
  const box = useRef<HTMLTextAreaElement>(null);
  // The agent rewrote it. Same counter as the schedule's, one panel over.
  const changed = useStore((state) => state.memoryVersion[agentId] ?? 0);

  const load = useCallback(async () => {
    const request = ++draft.generation;
    try {
      let page: { content: string; version?: string };
      try {
        page = await api.agentMemoryVersioned(agentId);
      } catch (caught) {
        if (
          typeof caught !== "object" ||
          caught === null ||
          !("kind" in caught) ||
          caught.kind !== "unknownCommand"
        )
          throw caught;
        page = { content: await api.agentMemory(agentId) };
      }
      if (request === draft.generation) dispatch({ type: "read", ...page });
    } catch (caught) {
      if (request === draft.generation)
        dispatch({ type: "readFailed", error: errorMessage(caught) });
    }
  }, [agentId, draft, dispatch]);

  useEffect(() => {
    void load();
    return () => {
      draft.generation++;
    };
  }, [load, changed, draft]);

  const held = "held" in state ? state.held : null;
  const loading = state.phase === "loading";
  const editing =
    state.phase === "editing" || state.phase === "saving" || (loading && state.editing);
  const busy = state.phase === "saving";
  const error = "error" in state ? state.error : null;
  const cut = "cut" in state && state.cut;
  const value = held === null ? "" : (held.draft ?? held.stored);
  const shown = loading ? "Loading…" : value;
  const dirty =
    held !== null && held.draft !== null && held.draft !== (held.incoming ?? held.stored);
  const overwritten = held?.incoming != null;
  const room = crowding(value);

  // Grow with the content instead of scrolling a six-line box, capped in CSS.
  useEffect(() => {
    const node = box.current;
    if (!node) return;
    node.style.height = "auto";
    node.style.height = `${node.scrollHeight}px`;
  }, [shown, editing]);

  const save = async () => {
    if (held?.draft == null) return;
    const sent = held.draft;
    dispatch({ type: "save", sent });
    try {
      // What comes back is what was actually stored: the runtime trims it and
      // cuts anything over the cap, so leaving what was typed on screen would
      // show the operator a page their agent is never going to be given.
      const expected = held.incoming !== null ? held.incomingVersion : held.version;
      const saved = await api.setAgentMemory(agentId, sent, expected);
      draft.generation++;
      dispatch({ type: "saved", saved });
      if (expected !== undefined) {
        void api.agentMemoryVersioned(agentId).then(
          (page) => dispatch({ type: "version", ...page }),
          () => {},
        );
      }
    } catch (caught) {
      if (
        typeof caught === "object" &&
        caught !== null &&
        "kind" in caught &&
        caught.kind === "memoryConflict" &&
        "detail" in caught &&
        typeof caught.detail === "string"
      ) {
        try {
          const detail: unknown = JSON.parse(caught.detail);
          if (
            typeof detail === "object" &&
            detail !== null &&
            "current" in detail &&
            typeof detail.current === "string" &&
            "version" in detail &&
            typeof detail.version === "string" &&
            "draft" in detail &&
            detail.draft === sent
          ) {
            draft.generation++;
            dispatch({ type: "conflict", current: detail.current, version: detail.version });
            return;
          }
        } catch {}
      }
      dispatch({ type: "saveFailed", error: errorMessage(caught) });
    }
  };

  return (
    <section className="detail memory">
      <div className="detail__head">
        <h3
          className="detail__title"
          title="What the agent keeps across conversations and is shown at the start of every turn. It rewrites this itself; you can edit it too."
        >
          Memory
        </h3>
        {editing && room && (
          <span className="memory__room" data-over={room.over || undefined}>
            {room.text}
          </span>
        )}
        {!editing && (
          <button
            type="button"
            className="btn btn--small btn--ghost"
            disabled={state.phase === "failed"}
            onClick={() => dispatch({ type: "edit" })}
          >
            Edit
          </button>
        )}
      </div>

      {editing ? (
        <textarea
          ref={box}
          className="memory__text"
          aria-label="Memory"
          rows={2}
          disabled={loading}
          value={shown}
          placeholder="Nothing yet."
          onChange={(event) => {
            const next = event.target.value;
            dispatch({ type: "change", value: next });
          }}
        />
      ) : state.phase === "failed" ? null : shown ? (
        <p className="memory__text memory__read">{shown}</p>
      ) : (
        <p className="detail__empty">Nothing yet.</p>
      )}

      {editing && (
        <div className="memory__actions">
          <button
            type="button"
            className="btn btn--small btn--primary"
            disabled={busy || !dirty}
            onClick={() => void save()}
          >
            Save
          </button>
          <button
            type="button"
            className="btn btn--small btn--ghost"
            disabled={busy}
            onClick={() => dispatch({ type: "cancel" })}
          >
            Cancel
          </button>
        </div>
      )}

      {overwritten && (
        <p className="memory__note">
          The agent rewrote this while you were typing. Save keeps yours and throws away what it
          wrote; Cancel takes what it wrote.
        </p>
      )}

      {cut && (
        <p className="memory__note">
          Saved, but it was too long and the end was cut. What is above is what the agent has.
        </p>
      )}

      {error && (
        <div className="banner banner--error" style={{ margin: "0.5rem 0 0" }}>
          <span>{error}</span>
          {state.phase === "failed" && (
            <button type="button" className="btn btn--small btn--ghost" onClick={() => void load()}>
              Retry
            </button>
          )}
        </div>
      )}
    </section>
  );
}
