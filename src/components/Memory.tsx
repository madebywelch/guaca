import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";

import { api } from "../lib/ipc";
import { memoryDraft } from "../lib/memoryDraft";
import { useStore } from "../lib/store";
import { type AgentId, errorMessage } from "../lib/types";

export { arrived, type MemoryState, memoryReducer } from "../lib/memoryDraft";

export const CAP = 16_000;

const ROOM = 400;

function characters(text: string): number {
  return [...text].length;
}

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

export function Memory({ agentId }: Props) {
  return <MemoryPage key={agentId} agentId={agentId} />;
}

function MemoryPage({ agentId }: Props) {
  const [draft] = useState(() => memoryDraft(agentId));
  const state = useSyncExternalStore(draft.subscribe, draft.getSnapshot);
  const { dispatch } = draft;
  const box = useRef<HTMLTextAreaElement>(null);
  const changed = useStore((state) => state.memoryVersion[agentId] ?? 0);
  const who = useStore((state) => state.agents.find((agent) => agent.id === agentId)?.name);

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
            className="btn btn--compact btn--quiet"
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
          placeholder={`What should ${who ?? "this agent"} always remember?`}
          onChange={(event) => {
            const next = event.target.value;
            dispatch({ type: "change", value: next });
          }}
        />
      ) : state.phase === "failed" ? null : shown ? (
        <p className="memory__text memory__read">{shown}</p>
      ) : (
        <p className="detail__empty">
          {who ?? "This agent"} keeps what it should remember between conversations here, and reads
          it before every reply. It adds to this as it learns, or you can start it with Edit.
        </p>
      )}

      {editing && (
        <div className="memory__actions">
          <button
            type="button"
            className="btn btn--compact btn--primary"
            disabled={busy || !dirty}
            onClick={() => void save()}
          >
            Save
          </button>
          <button
            type="button"
            className="btn btn--compact btn--quiet"
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
            <button
              type="button"
              className="btn btn--compact btn--quiet"
              onClick={() => void load()}
            >
              Retry
            </button>
          )}
        </div>
      )}
    </section>
  );
}
