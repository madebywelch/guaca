import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { type AgentCard, errorMessage, type Repository } from "../lib/types";

interface Props {
  agent: AgentCard;
}

/**
 * What this agent may write code in. The designation, such as it is.
 *
 * ## Why the grant is here and the list is on the group
 *
 * They answer two different questions, asked at two different moments by an
 * operator doing two different things.
 *
 * "What codebases does this crew have" is asked once, when a crew is set up and
 * whenever a directory moves, and it is answered in the group's settings beside
 * the crew's credentials and plugins. "What can this agent work on" is asked
 * every time somebody hires an engineer, and it is answered here, next to its
 * model and its instructions, because that is where an agent is decided.
 *
 * Putting the second one on the group panel does not scale and did not read
 * right. A crew with five repositories and six agents draws thirty toggles in a
 * settings page nobody opens twice, and it asks the operator to think
 * repository-first when what they are actually doing is making an engineer.
 * A read of who has what stays over there, because auditing is a real question
 * and the answer to it should not need six panels opened.
 *
 * ## This is what a specialist is
 *
 * There is no engineer switch anywhere in this app and this panel is why there
 * does not need to be one. An agent with nothing ticked here cannot write code,
 * and the panel says so in the same place you would have looked for the switch.
 * An agent with three ticked is an engineer on three codebases, which no boolean
 * could have said.
 *
 * ## More than one is the normal case
 *
 * Not one agent per repository. A change that adds an endpoint and the button
 * that calls it is one piece of work, and an agent holding only the API has to
 * hand half of it to a peer and wait. Give an agent whatever its job spans.
 * Unrelated products want separate crews rather than separate agents in one,
 * which is what groups are already for.
 */
export function AgentRepositories({ agent }: Props) {
  const [repositories, setRepositories] = useState<Repository[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

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

  const toggle = async (repository: Repository) => {
    setBusy(repository.id);
    setError(null);
    try {
      await api.setRepositoryAccess(repository.id, agent.id, !repository.reach.includes(agent.id));
      await load();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(null);
    }
  };

  if (repositories === null) return null;

  const held = repositories.filter((repository) => repository.reach.includes(agent.id));

  return (
    <div className="access">
      <div className="routines__head">
        <span className="field__label">Repositories</span>
      </div>

      {repositories.length === 0 ? (
        // Drawn even with nothing to offer, unlike the standing grants above.
        // An operator looking for the way to make this agent an engineer looks
        // here, and an absent panel reads as a feature that is not there rather
        // than as a crew with no repositories linked yet.
        <p className="field__hint">
          This crew has no repositories linked. Link one in the group's settings, then give it to
          this agent here. Until then it cannot write code anywhere.
        </p>
      ) : (
        <>
          <div className="choices">
            {repositories.map((repository) => (
              <button
                key={repository.id}
                type="button"
                className="choice"
                aria-pressed={repository.reach.includes(agent.id)}
                disabled={busy !== null}
                onClick={() => void toggle(repository)}
                title={repository.path}
              >
                {repository.name}
              </button>
            ))}
          </div>
          <p className="field__hint">
            {held.length === 0
              ? "Nothing ticked, so this agent cannot write code. Give it whatever its work actually spans: a change that touches two of these is one job, not two agents."
              : `Can write code in ${held.map((repository) => repository.name).join(", ")}, and nowhere else.`}
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
