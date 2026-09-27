import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { type AgentId, errorMessage, type NotebookEntry } from "../lib/types";
import { ago } from "./WorkingNotes";

interface Props {
  agentId: AgentId;
}

/**
 * The files an agent keeps for itself, for the operator to read.
 *
 * Read-only apart from deleting a file, for the reason working notes are: the
 * notebook is the agent's own account of its work, and a file the operator
 * half-rewrote is one neither of them can trust. A file that is wrong is taken
 * away, and the agent is told in the channel if it matters.
 */
export function Notebook({ agentId }: Props) {
  const [entries, setEntries] = useState<NotebookEntry[] | null>(null);
  const [open, setOpen] = useState<{ path: string; text: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const changed = useStore((state) => state.notebookVersion[agentId] ?? 0);

  const load = useCallback(async () => {
    try {
      setEntries(await api.agentNotebook(agentId));
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught));
      setEntries((current) => current ?? []);
    }
  }, [agentId]);

  useEffect(() => {
    void load();
  }, [load, changed]);

  const read = async (path: string) => {
    if (open?.path === path) {
      setOpen(null);
      return;
    }
    try {
      setOpen({ path, text: await api.readNotebook(agentId, path) });
    } catch (caught) {
      setError(errorMessage(caught));
    }
  };

  const remove = async (path: string) => {
    try {
      await api.deleteNotebookFile(agentId, path);
      if (open?.path === path) setOpen(null);
      await load();
    } catch (caught) {
      setError(errorMessage(caught));
    }
  };

  const now = Date.now();
  return (
    <section className="worknotes">
      <div className="worknotes__head">
        <h3 className="worknotes__title">Notebook</h3>
      </div>
      {entries === null ? (
        <p className="worknotes__empty">Loading…</p>
      ) : entries.length === 0 ? (
        <p className="worknotes__empty">
          Empty. The agent keeps files here with <code>notebook</code>: a log, a tracker, research
          it wants to come back to.
        </p>
      ) : (
        <ol className="worknotes__list">
          {entries.map((entry) => (
            <li className="worknotes__note" key={entry.path}>
              <button
                type="button"
                className="worknotes__more"
                aria-expanded={open?.path === entry.path}
                onClick={() => void read(entry.path)}
              >
                {entry.path}
              </button>
              <span className="worknotes__age">{ago(entry.updatedAt, now)}</span>
              <button
                type="button"
                className="btn btn--small btn--ghost"
                aria-label={`Delete ${entry.path}`}
                onClick={() => void remove(entry.path)}
              >
                Delete
              </button>
              {open?.path === entry.path && <pre className="skill__body">{open.text}</pre>}
            </li>
          ))}
        </ol>
      )}
      {error && (
        <p className="field__error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
