import { useCallback, useEffect, useReducer, useRef, useState } from "react";

import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { type AgentId, errorMessage, type WorkingNote } from "../lib/types";

/**
 * How long ago, in the coarsest unit that is still true.
 *
 * The same rule `worknote::how_long_ago` follows on the Rust side, and the same
 * reason: an age is what makes this list worth reading. "Waiting on the legal
 * read" says an agent is working; the same line marked six days old says the
 * thing it is waiting for is not coming. A timestamp would make the operator do
 * that subtraction themselves, every time, for every row.
 *
 * Deliberately not shared with the runtime's copy. The two are read by different
 * audiences: this one may say "just now" while a model is told the same note is
 * "0m ago", and pinning them to each other would be pinning two sentences that
 * only look alike.
 */
export function ago(at: number, now: number): string {
  const ms = Math.max(0, now - at);
  const minutes = Math.floor(ms / 60_000);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);
  if (days >= 1) return `${days}d ago`;
  if (hours >= 1) return `${hours}h ago`;
  if (minutes >= 1) return `${minutes}m ago`;
  return "just now";
}

/**
 * How many notes the panel draws before it is asked for the rest.
 *
 * The list is bounded at `KEPT` and not at this, and the two answer different
 * questions. Sixteen is what an agent may carry; four is what the panel can
 * show without becoming the column. A note runs to three wrapped lines in a
 * sidebar this narrow, so a full list is a screen of text sitting above the
 * schedule and the memory, and the operator scrolls past it to reach anything
 * else. Four is where the section stops being a wall and still answers the
 * question it is for, which is what this agent is in the middle of right now.
 */
const SHOWN = 4;
const REMEMBERED = "guac.workingNotes";

function remembered(): boolean {
  try {
    return localStorage.getItem(REMEMBERED) === "open";
  } catch {
    return false;
  }
}

interface Props {
  agentId: AgentId;
}

type Read =
  | { kind: "loading" }
  | { kind: "failed"; error: string }
  | { kind: "ready"; notes: WorkingNote[]; error: string | null };

type State = { agentId: AgentId; generation: number; read: Read; busy: boolean };
type Action =
  | { type: "load"; agentId: AgentId; generation: number }
  | { type: "loaded"; agentId: AgentId; generation: number; notes: WorkingNote[] }
  | { type: "readFailed"; agentId: AgentId; generation: number; error: string }
  | { type: "clear"; agentId: AgentId }
  | { type: "cleared"; agentId: AgentId; generation: number }
  | { type: "clearFailed"; agentId: AgentId; error: string };

export function notesTransition(state: State, action: Action): State {
  if (action.type === "load") {
    return action.agentId === state.agentId
      ? { ...state, generation: action.generation }
      : {
          agentId: action.agentId,
          generation: action.generation,
          read: { kind: "loading" },
          busy: false,
        };
  }
  if (action.agentId !== state.agentId) return state;
  switch (action.type) {
    case "loaded":
      return action.generation === state.generation
        ? { ...state, read: { kind: "ready", notes: action.notes, error: null } }
        : state;
    case "readFailed":
      if (action.generation !== state.generation) return state;
      return {
        ...state,
        read:
          state.read.kind === "ready"
            ? { ...state.read, error: action.error }
            : { kind: "failed", error: action.error },
      };
    case "clear":
      return {
        ...state,
        busy: true,
        read: state.read.kind === "ready" ? { ...state.read, error: null } : state.read,
      };
    case "cleared":
      return {
        ...state,
        generation: action.generation,
        busy: false,
        read: { kind: "ready", notes: [], error: null },
      };
    case "clearFailed":
      return {
        ...state,
        busy: false,
        read:
          state.read.kind === "ready"
            ? { ...state.read, error: action.error }
            : { kind: "failed", error: action.error },
      };
  }
}

/**
 * What an agent is in the middle of, over what it knows.
 *
 * Read-only apart from one button, and that asymmetry against the memory panel
 * below it is the design rather than an unfinished half of it. Memory is a page
 * two parties maintain, which is why it needs a draft, a held incoming version
 * and two ways out. This is the agent's own account of its work: the operator
 * either believes it or declares the work done, and there is no third thing to
 * do to a line somebody else wrote about what they are waiting for.
 *
 * Clearing is offered because the one failure this list has is an agent still
 * waiting on something the operator settled in person. Editing a single note is
 * not, because a list the operator half-rewrites is a list neither of them can
 * trust, and the agent cannot see the edit as anything but its own past self.
 */
export function WorkingNotes({ agentId }: Props) {
  const [state, dispatch] = useReducer(notesTransition, {
    agentId,
    generation: 0,
    read: { kind: "loading" },
    busy: false,
  });
  const generation = useRef(0);
  const currentAgent = useRef({ agentId });
  if (currentAgent.current.agentId !== agentId) currentAgent.current = { agentId };
  const read = state.agentId === agentId ? state.read : { kind: "loading" as const };
  const notes = read.kind === "ready" ? read.notes : null;
  const error = read.kind === "loading" ? null : read.error;
  const busy = state.agentId === agentId && state.busy;
  // One choice across agents, like the inspector itself. The panel remounts
  // on an agent switch, but the operator's preferred density does not.
  const [all, setAll] = useState(false);
  const [expanded, setExpanded] = useState(remembered);
  useEffect(() => {
    try {
      localStorage.setItem(REMEMBERED, expanded ? "open" : "closed");
    } catch {
      // A refused preference must not keep the notes from opening.
    }
  }, [expanded]);
  // The agent appended one. Its own counter, because notes move far more often
  // than memory does and sharing one would refetch a page that has not changed.
  const changed = useStore((state) => state.workingNotesVersion[agentId] ?? 0);

  const load = useCallback(() => {
    const request = ++generation.current;
    dispatch({ type: "load", agentId, generation: request });
    void (async () => {
      try {
        const notes = await api.agentWorkingNotes(agentId);
        dispatch({ type: "loaded", agentId, generation: request, notes });
      } catch (caught) {
        dispatch({ type: "readFailed", agentId, generation: request, error: errorMessage(caught) });
      }
    })();
  }, [agentId]);

  useEffect(() => {
    load();
  }, [load, changed]);

  const clear = async () => {
    const owner = currentAgent.current;
    dispatch({ type: "clear", agentId });
    try {
      await api.clearAgentWorkingNotes(agentId);
      if (currentAgent.current === owner) {
        dispatch({ type: "cleared", agentId, generation: ++generation.current });
      }
    } catch (caught) {
      if (currentAgent.current === owner)
        dispatch({ type: "clearFailed", agentId, error: errorMessage(caught) });
    }
  };

  // One clock for the whole list, taken at render. Per-row `Date.now()` would
  // let two notes written in the same second render a minute apart.
  const now = Date.now();
  // The newest, and the older ones behind a button. Which end is cut is the
  // decision: the notes read oldest first, so the tail is where the work is
  // now, and a panel that showed the first four would answer with the state
  // the agent has already moved on from.
  const held = notes ?? [];
  const hidden = all ? 0 : Math.max(0, held.length - SHOWN);
  const drawn = hidden > 0 ? held.slice(hidden) : held;

  return (
    <section className="detail worknotes">
      <div className="detail__head">
        <h3
          className="detail__title"
          title="Where its work stands, one line per note, written by the agent as it goes. The oldest fall off on their own."
        >
          <button
            type="button"
            className="worknotes__toggle"
            aria-expanded={expanded}
            onClick={() => setExpanded((open) => !open)}
          >
            Working notes
            {notes !== null && <span className="detail__count">{` · ${notes.length}`}</span>}
            {notes?.length ? (
              <span className="detail__count">
                {` · newest ${ago(notes[notes.length - 1]!.at, now)}`}
              </span>
            ) : null}
          </button>
        </h3>
        {expanded && notes !== null && notes.length > 0 && (
          <button
            type="button"
            className="btn btn--small btn--ghost"
            disabled={busy}
            onClick={() => void clear()}
          >
            Clear
          </button>
        )}
      </div>

      {expanded &&
        (read.kind === "failed" ? null : notes === null ? (
          <p className="detail__empty">Loading…</p>
        ) : notes.length === 0 ? (
          <p className="detail__empty">Nothing in flight.</p>
        ) : (
          <ol className="worknotes__list">
            {held.length > SHOWN && (
              // Above the list because that is where the notes it hides are: they
              // are the older ones, and a control under the list would point the
              // operator the wrong way down it.
              <li className="worknotes__rest">
                <button
                  type="button"
                  className="worknotes__more"
                  aria-expanded={all}
                  onClick={() => setAll((open) => !open)}
                >
                  {all ? "Show fewer" : `Show ${hidden} older`}
                </button>
              </li>
            )}
            {drawn.map((note) => (
              // Keyed on the stamp because it is unique: the runtime's clock is
              // strictly increasing, so two notes cannot share one. An index key
              // would redraw every row each time the oldest falls off the end.
              <li className="worknotes__note" key={note.at}>
                <span className="worknotes__body">{note.body}</span>
                <span className="worknotes__age">{ago(note.at, now)}</span>
              </li>
            ))}
          </ol>
        ))}

      {error && (
        <div className="banner banner--error" style={{ margin: "0.5rem 0 0" }}>
          <span>{read.kind === "failed" ? `Could not read working notes. ${error}` : error}</span>
          {read.kind === "failed" && (
            <button
              type="button"
              className="btn btn--small btn--ghost"
              aria-label="Retry reading working notes"
              onClick={load}
            >
              Retry
            </button>
          )}
        </div>
      )}
    </section>
  );
}
