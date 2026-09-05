import { useMemo, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { COMPOST_DAYS, composted, goingSoon, timeLeft } from "../lib/compost";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { type AgentCard, type AgentId, errorMessage } from "../lib/types";

/**
 * Where deleted agents go, and what it takes to get one back.
 *
 * A pane in app settings, with one decision and one clock per deleted agent.
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
export function Compost() {
  const agents = useStore((s) => s.agents);
  const groups = useStore((s) => s.groups);
  const select = useStore((s) => s.select);
  const refreshAgents = useStore((s) => s.refreshAgents);

  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<AgentId | null>(null);
  const [confirming, setConfirming] = useState<AgentId | null>(null);

  // Read once, when the panel opens. A clock that ticks would redraw every row
  // to move a number that changes once a day, and nothing here is worth
  // watching happen: the sweep runs hourly and the panel is open for seconds.
  const [now] = useState(() => Date.now());

  const rows = useMemo(() => composted(agents), [agents]);

  /**
   * One row's button, whichever it was.
   *
   * The roster is re-read here rather than left to the runtime's own event,
   * because both of these change which rows this panel draws and one of them
   * selects the restored agent. Returns `null` when the call was refused, so
   * a caller does not act on a failure the operator is currently reading.
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
    // Show its channel when settings closes. Keep this pane open so restoring
    // an agent does not discard settings edits waiting to be saved.
    await select(back.id);
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
    <>
      <h3 className="settings__title">Compost</h3>
      <p className="settings__lede">
        Deleted agents wait {COMPOST_DAYS} days here, still holding everything they knew: their
        memory, their working notes, their schedule and their sign-ins. Put one back and it returns
        paused. Leave it and all of that goes with it.
      </p>

      {rows.length === 0 ? (
        <p className="settings__lede">No deleted agents.</p>
      ) : (
        <ul className="compost__list">{rows.map(row)}</ul>
      )}

      {error && (
        <div className="banner banner--error">
          <span>{error}</span>
        </div>
      )}
    </>
  );
}
