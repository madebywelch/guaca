import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { type AgentCard, errorMessage, type Repository } from "../lib/types";

interface Props {
  agent: AgentCard;
}

/**
 * Which codebase this agent works in. The designation, such as it is.
 *
 * ## There is no engineer switch, and this is why there does not need to be one
 *
 * An agent in no repository cannot write code, and this panel says so in the
 * same place somebody would have gone looking for the switch. An agent in one
 * is an engineer on that codebase, which no boolean could have said.
 *
 * ## One, and the reason is coordination rather than permissions
 *
 * Two agents on one repository settle a change between themselves, in the crew
 * they already share, in messages the operator can read. One agent quietly
 * holding two codebases is a change whose shape nobody can see until it lands
 * in both. So this is a choice, not a set of ticks, and choosing a second one
 * moves the agent rather than adding to it.
 *
 * ## Here and in the rail, deliberately twice
 *
 * The rail is where the change is usually made, because dragging is the
 * shortest gesture and the tree is where the question is usually asked. This is
 * where an agent is *read*: it sits beside the model and the instructions, which
 * is what somebody is looking at when they are deciding what an agent is for.
 * Both write through the same call.
 */
export function AgentRepositories({ agent }: Props) {
  const [repositories, setRepositories] = useState<Repository[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      setRepositories(await api.groupRepositories(agent.groupId));
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught));
      setRepositories([]);
    }
  }, [agent.groupId]);

  useEffect(() => {
    void load();
  }, [load]);

  const put = async (into: Repository | null) => {
    setBusy(true);
    setError(null);
    try {
      // Choosing the one it is already in takes it out. A set of buttons where
      // one is always pressed has no other way back to none, and "none" is a
      // state the operator has to be able to reach.
      const next = into && into.id !== agent.repositoryId ? into.id : null;
      await api.setAgentRepository(agent.id, next);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  if (repositories === null) return null;

  const held = repositories.find((repository) => repository.id === agent.repositoryId);

  return (
    <div className="access">
      <div className="routines__head">
        <span className="field__label">Repository</span>
      </div>

      {repositories.length === 0 ? (
        // Drawn even with nothing to offer, unlike the standing grants above.
        // Somebody looking for the way to make this agent an engineer looks
        // here, and an absent panel reads as a feature that is not there rather
        // than as a crew that has linked nothing yet.
        <p className="field__hint">
          This crew has no repositories linked. Link one in the group's settings, then put this
          agent in it. Until then it cannot write code anywhere.
        </p>
      ) : (
        <>
          <div className="choices">
            {repositories.map((repository) => (
              <button
                key={repository.id}
                type="button"
                className="choice"
                aria-pressed={repository.id === agent.repositoryId}
                disabled={busy}
                onClick={() => void put(repository)}
                title={repository.path}
              >
                {repository.name}
              </button>
            ))}
          </div>
          <p className="field__hint">
            {held
              ? `Works in ${held.name}, and nowhere else. Choosing another moves it; choosing ${held.name} again takes it out.`
              : "In no repository, so this agent cannot write code. An agent works in one at a time: two agents on one codebase coordinate in this crew, which is something you can read."}
          </p>
        </>
      )}

      {error && (
        <div className="banner banner--error" style={{ margin: "0.4rem 0 0" }}>
          <span>{error}</span>
        </div>
      )}
    </div>
  );
}
