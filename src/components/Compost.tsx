import { useEffect, useMemo, useRef, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { COMPOST_DAYS, composted, goingSoon, timeLeft } from "../lib/compost";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { type AgentCard, type AgentId, errorMessage } from "../lib/types";

interface Props {
  onClose: () => void;
}

/**
 * Where deleted agents go, and what it takes to get one back.
 *
 * The counterpart of the cafeteria, and drawn like one on purpose: the two
 * surfaces are hiring and letting go, and an operator who has used one should
 * recognize the other. Everything else about them is opposite. The cafeteria is
 * a menu of agents nobody has met; this is a list of agents somebody worked
 * with, each with a clock on it.
 *
 * What a delete actually costs is said once, in the head, and not on every row.
 * It is the whole argument for the panel existing — a deleted agent used to
 * lose its memory, its schedule, its sign-ins and its machine on the click, and
 * none of that is visible at the moment of pressing delete — but it is the same
 * sentence about every agent in the list, and the same sentence three times is
 * wallpaper. Said once at the top it is read; repeated down the column it turns
 * the clock, which is the one thing that differs per row, into more of the
 * same gray text.
 *
 * Restoring is one click and deleting for good is two, which is the same
 * asymmetry the agent menu draws. This is the only surface in the app that
 * destroys a memory on a button, so the button says so before it does it.
 */
export function Compost({ onClose }: Props) {
  const agents = useStore((s) => s.agents);
  const groups = useStore((s) => s.groups);
  const select = useStore((s) => s.select);
  const refreshAgents = useStore((s) => s.refreshAgents);

  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<AgentId | null>(null);
  const [confirming, setConfirming] = useState<AgentId | null>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  // Read once, when the panel opens. A clock that ticks would redraw every row
  // to move a number that changes once a day, and nothing here is worth
  // watching happen: the sweep runs hourly and the panel is open for seconds.
  const [now] = useState(() => Date.now());

  useEffect(() => {
    panelRef.current?.focus();
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const rows = useMemo(() => composted(agents), [agents]);

  // Closed when the last one is dealt with, rather than left drawing an empty
  // panel the operator has to dismiss. Emptying the compost is the one reason
  // to be here, and finishing it is the answer.
  useEffect(() => {
    if (rows.length === 0) onClose();
  }, [rows.length, onClose]);

  /**
   * One row's button, whichever it was.
   *
   * The roster is re-read here rather than left to the runtime's own event,
   * because both of these change which rows this panel draws and one of them
   * closes it. Returns `null` when the call was refused, so a caller with
   * something to do afterwards does not do it on a failure the operator is
   * currently reading.
   */
  const act = async <T,>(agent: AgentCard, run: () => Promise<T>): Promise<T | null> => {
    setBusy(agent.id);
    setError(null);
    try {
      const done = await run();
      await refreshAgents();
      return done;
    } catch (caught) {
      setError(errorMessage(caught));
      return null;
    } finally {
      setBusy(null);
      setConfirming(null);
    }
  };

  const restore = async (agent: AgentCard) => {
    const back = await act(agent, () => api.restoreAgent(agent.id));
    if (!back) return;
    // Opening it is what makes the click look like it did something: the agent
    // comes back paused, so nothing it does will draw attention to itself, and
    // the name may have been settled on the way in.
    await select(back.id);
    onClose();
  };

  const row = (agent: AgentCard) => {
    const crew = groups.find((group) => group.id === agent.groupId);
    const soon = goingSoon(agent, now);
    const working = busy === agent.id;

    return (
      <li key={agent.id} className="compost__row">
        <span className="compost__face">
          {/* Drawn as what it is: an agent that is not running. The same mark
              the rail puts on a deleted row, so a face in here and a face in an
              old transcript agree with each other. */}
          <AgentAvatar
            avatar={agent.avatar}
            color={agent.color}
            size="md"
            seed={agent.id}
            lifecycle="terminated"
          />
        </span>

        <span className="compost__body">
          <span className="compost__name">
            {agent.name}
            {crew && <span className="compost__crew">{crew.name}</span>}
          </span>
          <span className="compost__clock" data-soon={soon ? "true" : undefined}>
            {timeLeft(agent, now)}
          </span>
        </span>

        <span className="compost__actions">
          {confirming === agent.id ? (
            <>
              <button
                type="button"
                className="btn btn--ghost"
                disabled={working}
                onClick={() => setConfirming(null)}
              >
                Keep it
              </button>
              <button
                type="button"
                className="btn btn--danger"
                disabled={working}
                onClick={() => void act(agent, () => api.purgeAgent(agent.id))}
              >
                {working ? "Deleting…" : "Delete the memory too"}
              </button>
            </>
          ) : (
            <>
              <button
                type="button"
                className="btn btn--ghost"
                disabled={working}
                onClick={() => setConfirming(agent.id)}
              >
                Delete now
              </button>
              <button
                type="button"
                className="btn btn--primary"
                disabled={working}
                onClick={() => void restore(agent)}
              >
                {working ? "Restoring…" : "Put back"}
              </button>
            </>
          )}
        </span>
      </li>
    );
  };

  return (
    <div className="scrim">
      <button type="button" className="scrim__close" aria-label="Close dialog" onClick={onClose} />
      <div
        className="dialog dialog--compost"
        role="dialog"
        aria-modal="true"
        aria-label="Compost"
        tabIndex={-1}
        ref={panelRef}
      >
        <div className="compost__head">
          <h2 className="dialog__title">Compost</h2>
          {/* The number comes from the constant the runtime enforces rather
              than from this sentence, because this sentence is a promise about
              when somebody's memory is deleted. */}
          <p className="dialog__lede" style={{ margin: 0 }}>
            Deleted agents wait {COMPOST_DAYS} days here, still holding everything they knew: their
            memory, their working notes, their schedule and their sign-ins. Put one back and it
            returns paused. Leave it and all of that goes with it.
          </p>
        </div>

        <ul className="compost__list">{rows.map(row)}</ul>

        {error && (
          <div className="banner banner--error" style={{ margin: "0 1.35rem" }}>
            <span>{error}</span>
          </div>
        )}

        <div className="compost__foot">
          <button type="button" className="btn" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
