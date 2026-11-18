import { useMemo, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { COMPOST_DAYS, composted, goingSoon, timeLeft } from "../lib/compost";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { type AgentCard, type AgentId, errorMessage } from "../lib/types";

export function Compost() {
  const agents = useStore((s) => s.agents);
  const groups = useStore((s) => s.groups);
  const select = useStore((s) => s.select);
  const refreshAgents = useStore((s) => s.refreshAgents);

  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<AgentId | null>(null);
  const [confirming, setConfirming] = useState<AgentId | null>(null);

  const [now] = useState(() => Date.now());

  const rows = useMemo(() => composted(agents), [agents]);

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
    await select(back.id);
  };

  const row = (agent: AgentCard) => {
    const crew = groups.find((group) => group.id === agent.groupId);
    const soon = goingSoon(agent, now);
    const working = busy === agent.id;

    return (
      <li key={agent.id} className="compost__row">
        <span className="compost__face">
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
                className="btn btn--quiet"
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
                className="btn btn--quiet"
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
