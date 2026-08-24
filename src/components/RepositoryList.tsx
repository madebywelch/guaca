import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import {
  type AgentCard,
  errorMessage,
  type GroupId,
  type Repository,
  type RepositoryDraft,
  type RepositoryId,
} from "../lib/types";

interface Props {
  groupId: GroupId;
  crew: AgentCard[];
}

/** Who works in a repository, as a sentence. */
function worksIn(repository: Repository, crew: AgentCard[]): string {
  const named = crew.filter((agent) => agent.repositoryId === repository.id);
  if (named.length === 0) return "nobody yet";
  return named.map((agent) => agent.name).join(", ");
}

/**
 * The directories a crew may write code in, and who in it may.
 *
 * ## There is no engineer here, and that is the point
 *
 * The obvious control is a switch on an agent that says it is an engineer, or a
 * specialist, or whatever the tier gets called. It is not here, because a tier
 * carries no information this list does not. An agent with no repository is
 * offered nothing that reaches a working tree; an "engineer" with no repository
 * is the same agent with a badge. Two answers to one question is two places for
 * it to be wrong, and only one of them is the one the runtime reads.
 *
 * The part a tier feels like it would buy is already content and already ships.
 * The cafeteria hires a Software Engineer, a Code Reviewer and a QA Tester,
 * each with its own brief, and the model field suggests programming models for
 * an agent whose words read that way. Designating an engineer is hiring one and
 * giving it a directory. Nothing under here has to know the word.
 *
 * ## Linking is here; handing out is on the agent
 *
 * Two decisions, and they are split across two panels rather than stacked in
 * one, because they are asked at different moments. "What codebases does this
 * crew have" is a crew fact, set up once and edited when a directory moves.
 * "What can this agent work on" is asked every time somebody hires an engineer,
 * and it is answered where an agent is decided, beside its model and its
 * instructions. `AgentRepositories` is that panel.
 *
 * A newly linked repository therefore reaches nobody, and the line under each
 * row says so. That is a state to pass through rather than one to hide: a
 * crew's source handed to every agent at the moment it was linked is the
 * accident this feature most has to avoid.
 *
 * Who has what stays here as a read, and only as a read. Auditing is a real
 * question, and answering it should not mean opening six agents one at a time.
 * The same reason the plugin panel says who a sign-in is offered to.
 *
 * There is no "every agent" button here and there is one on the plugins above.
 * An agent hired next week must not inherit the operator's own source. Names,
 * always, and only on the agent.
 */
export function RepositoryList({ groupId, crew }: Props) {
  const [repositories, setRepositories] = useState<Repository[] | null>(null);
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState({ path: "", name: "", note: "" });
  const [editing, setEditing] = useState<RepositoryId | null>(null);
  const [edit, setEdit] = useState({ name: "", note: "" });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const pathRef = useRef<HTMLInputElement>(null);

  // The path is the only field that has to be filled in, so it takes the
  // cursor. Same reason the credential form focuses its first box.
  useEffect(() => {
    if (adding) pathRef.current?.focus();
  }, [adding]);

  const load = useCallback(async () => {
    try {
      setRepositories(await api.groupRepositories(groupId));
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught));
      setRepositories([]);
    }
  }, [groupId]);

  useEffect(() => {
    void load();
  }, [load]);

  const run = async (key: string, action: () => Promise<unknown>) => {
    setBusy(key);
    setError(null);
    try {
      await action();
      await load();
      return true;
    } catch (caught) {
      setError(errorMessage(caught));
      return false;
    } finally {
      setBusy(null);
    }
  };

  const reset = () => {
    setAdding(false);
    setDraft({ path: "", name: "", note: "" });
  };

  const add = () =>
    void run("add", () =>
      api.createRepository({
        groupId,
        name: draft.name,
        path: draft.path,
        note: draft.note,
      } satisfies RepositoryDraft),
    ).then((ok) => ok && reset());

  if (repositories === null) return <p className="field__hint">Loading repositories…</p>;

  return (
    <div className="access">
      <div className="routines__head">
        <span className="field__label">Repositories</span>
        {adding && (
          <button type="button" className="btn btn--ghost btn--small" onClick={reset}>
            Cancel
          </button>
        )}
      </div>

      {repositories.map((repository) => (
        <div className="access__item" key={repository.id}>
          <div className="access__row">
            <strong className="access__name">{repository.name}</strong>
            <span className="access__where">{repository.path}</span>
            <button
              type="button"
              className="btn btn--small btn--ghost"
              disabled={busy !== null}
              onClick={() => {
                setEditing(editing === repository.id ? null : repository.id);
                setEdit({ name: repository.name, note: repository.note });
              }}
            >
              {editing === repository.id ? "Done" : "Edit"}
            </button>
            <button
              type="button"
              className="btn btn--small btn--ghost"
              disabled={busy !== null}
              onClick={() => void run(repository.id, () => api.deleteRepository(repository.id))}
            >
              Unlink
            </button>
          </div>

          {editing === repository.id ? (
            <>
              <div className="access__row">
                <input
                  className="input input--slim"
                  placeholder="what you call it"
                  value={edit.name}
                  onChange={(event) => setEdit({ ...edit, name: event.target.value })}
                />
                <input
                  className="input input--slim"
                  placeholder="run ./scripts/ci.sh before you finish"
                  value={edit.note}
                  onChange={(event) => setEdit({ ...edit, note: event.target.value })}
                />
                <button
                  type="button"
                  className="btn btn--small btn--primary"
                  disabled={busy !== null || !edit.name.trim()}
                  onClick={() =>
                    void run(`${repository.id}-edit`, () =>
                      api.updateRepository(repository.id, edit.name, edit.note),
                    ).then((ok) => ok && setEditing(null))
                  }
                >
                  Save
                </button>
              </div>
              {/* Said where the field is, because the field is next to a path
                  and the two look like they change the same kind of thing. */}
              <p className="field__hint">
                The path is not editable. A different directory is a different repository: whoever
                you gave this one was given that directory, so moving it here would move their
                boundary without saying so.
              </p>
            </>
          ) : (
            repository.note && <p className="field__hint">{repository.note}</p>
          )}

          <p className="field__hint">
            Worked in by {worksIn(repository, crew)}. Put an agent in it by dragging it onto this
            repository in the rail.
          </p>
        </div>
      ))}

      {adding ? (
        <div className="access__item">
          <div className="access__row">
            <input
              className="input input--mono"
              placeholder="/Users/you/dev/your-project"
              ref={pathRef}
              value={draft.path}
              onChange={(event) => setDraft({ ...draft, path: event.target.value })}
              onKeyDown={(event) => {
                if (event.key === "Enter" && draft.path.trim()) add();
              }}
            />
            <button
              type="button"
              className="btn btn--small btn--primary"
              disabled={busy !== null || !draft.path.trim()}
              onClick={add}
            >
              Link
            </button>
          </div>
          <div className="access__row">
            <input
              className="input input--slim"
              placeholder="what you call it (optional)"
              value={draft.name}
              onChange={(event) => setDraft({ ...draft, name: event.target.value })}
            />
            <input
              className="input input--slim"
              placeholder="run ./scripts/ci.sh before you finish (optional)"
              value={draft.note}
              onChange={(event) => setDraft({ ...draft, note: event.target.value })}
            />
          </div>
          <p className="field__hint">
            The full path to the directory, which has to be the root of a git repository. Git is the
            undo: it is the reason an agent can be turned loose in there at all. The note is read by
            every agent that has it, on every turn.
          </p>
        </div>
      ) : (
        <>
          <p className="field__hint">
            A directory on this machine that this crew may write code in. Linking one gives it to
            nobody. You hand it to an agent from that agent's own panel, one at a time, and an agent
            you hire later does not inherit it.
          </p>
          <button type="button" className="btn btn--small" onClick={() => setAdding(true)}>
            Link a repository
          </button>
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
