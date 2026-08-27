import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { type AgentId, errorMessage } from "../lib/types";

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
}

/**
 * A version read off disk, against what the panel is already holding.
 *
 * The operator's unsaved text is never replaced by it. An agent rewrites this
 * file in the middle of a turn, and the operator is most likely to be editing
 * it exactly then, so a read that applied itself would take a sentence away as
 * it was being typed. It is held to one side instead and the panel says so:
 * keeping what you wrote is Save, and taking what the agent wrote is Discard.
 *
 * A draft typed back to what is on disk is not an edit, and holding it as one
 * would leave the panel on a page the agent has since replaced.
 */
export function arrived(held: Held | null, content: string): Held {
  if (held === null || held.draft === null || held.draft === held.stored) {
    return { stored: content, draft: null, incoming: null };
  }
  return { ...held, incoming: content };
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
 * One box, always editable, rather than a rendered page with an edit mode.
 * What is stored is markdown, and an operator seeding a persona wants to see
 * the characters the agent will actually be shown. A mode would buy formatting
 * nobody asked for and cost a click on the one thing this panel makes cheap.
 */
export function Memory({ agentId }: Props) {
  const [held, setHeld] = useState<Held | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** Set when the runtime stored something other than what was sent. */
  const [cut, setCut] = useState(false);
  const box = useRef<HTMLTextAreaElement>(null);
  // The agent rewrote it. Same counter as the schedule's, one panel over.
  const changed = useStore((state) => state.memoryVersion[agentId] ?? 0);

  const load = useCallback(async () => {
    try {
      const content = await api.agentMemory(agentId);
      // Through `arrived` rather than straight onto the state, so what happens
      // to an operator mid-sentence is decided against what is held right now
      // rather than against what was held when this read was started.
      setHeld((current) => arrived(current, content));
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught));
      setHeld((current) => current ?? { stored: "", draft: null, incoming: null });
    }
  }, [agentId]);

  useEffect(() => {
    void load();
  }, [load, changed]);

  const loading = held === null;
  const value = held === null ? "" : (held.draft ?? held.stored);
  const shown = loading ? "Loading…" : value;
  const dirty = held !== null && held.draft !== null && held.draft !== held.stored;
  const overwritten = held?.incoming != null;
  const room = crowding(value);

  // Grow with the content instead of scrolling a six-line box, capped in CSS.
  useEffect(() => {
    const node = box.current;
    if (!node) return;
    node.style.height = "auto";
    node.style.height = `${node.scrollHeight}px`;
  }, [shown]);

  const save = async () => {
    if (held?.draft == null) return;
    const sent = held.draft;
    setBusy(true);
    setError(null);
    try {
      // What comes back is what was actually stored: the runtime trims it and
      // cuts anything over the cap, so leaving what was typed on screen would
      // show the operator a page their agent is never going to be given.
      const saved = await api.setAgentMemory(agentId, sent);
      setHeld({ stored: saved, draft: null, incoming: null });
      setCut(saved !== sent.trim());
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="memory">
      <div className="memory__head">
        <h3 className="memory__title">Memory</h3>
        {room && (
          <span className="memory__room" data-over={room.over || undefined}>
            {room.text}
          </span>
        )}
      </div>

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
          setCut(false);
          setHeld((current) => current && { ...current, draft: next });
        }}
      />

      {dirty && (
        <div className="memory__actions">
          <button
            type="button"
            className="btn btn--small btn--primary"
            disabled={busy}
            onClick={() => void save()}
          >
            Save
          </button>
          <button
            type="button"
            className="btn btn--small btn--ghost"
            disabled={busy}
            onClick={() =>
              setHeld(
                (current) =>
                  current && {
                    stored: current.incoming ?? current.stored,
                    draft: null,
                    incoming: null,
                  },
              )
            }
          >
            Discard
          </button>
        </div>
      )}

      {overwritten && (
        <p className="memory__note">
          The agent rewrote this while you were typing. Save keeps yours and throws away what it
          wrote; Discard takes what it wrote.
        </p>
      )}

      {cut && (
        <p className="memory__note">
          Saved, but it was too long and the end was cut. What is above is what the agent has.
        </p>
      )}

      {/* Only where there is nothing to read, which is the one time the box
          cannot explain itself. On an agent that has written something, the
          same three sentences are a paragraph under every glance. */}
      {!loading && value === "" && (
        <p className="memory__note">
          The agent writes here with <code>update_memory</code> when it learns something durable,
          and is shown it at the start of every turn. Where its work stands goes in the working
          notes above instead. Seed a persona if you like.
        </p>
      )}

      {error && (
        <div className="banner banner--error" style={{ margin: "0.5rem 0 0" }}>
          <span>{error}</span>
        </div>
      )}
    </section>
  );
}
