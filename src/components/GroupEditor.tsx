import { useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { errorMessage, type Group } from "../lib/types";

interface Props {
  /** Absent means create. */
  group?: Group;
  onClose: () => void;
}

/**
 * Create, rename or delete a group.
 *
 * Deliberately thin: a group is a name and a wall. Everything interesting about
 * it — who is inside, who they can reach — is decided by the agents' own group
 * field and enforced in the Rust runtime.
 */
export function GroupEditor({ group, onClose }: Props) {
  const refreshAgents = useStore((s) => s.refreshAgents);
  const [name, setName] = useState(group?.name ?? "");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    nameRef.current?.focus();
    nameRef.current?.select();
  }, []);

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      if (group) await api.renameGroup(group.id, name);
      else await api.createGroup(name);
      await refreshAgents();
      onClose();
    } catch (caught) {
      setError(errorMessage(caught));
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!group) return;
    setBusy(true);
    setError(null);
    try {
      await api.deleteGroup(group.id);
      await refreshAgents();
      onClose();
    } catch (caught) {
      // The common failure is a group that still holds agents, and the message
      // from Rust already says how many and what to do, so it is shown as-is.
      setError(errorMessage(caught));
      setBusy(false);
    }
  };

  return (
    <div className="scrim">
      <button type="button" className="scrim__close" aria-label="Close dialog" onClick={onClose} />
      <div
        className="dialog"
        role="dialog"
        aria-modal="true"
        aria-label={group ? "Edit group" : "New group"}
      >
        <h2 className="dialog__title">{group ? "Edit group" : "New group"}</h2>
        <p className="dialog__lede" style={{ marginTop: 0 }}>
          Agents in different groups cannot see or message each other. Moving an agent between
          groups changes who it can reach.
        </p>

        <label className="field">
          <span className="field__label">Name</span>
          <input
            className="input input--mono"
            ref={nameRef}
            value={name}
            maxLength={48}
            placeholder="Research"
            onChange={(event) => setName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && name.trim()) void save();
            }}
          />
        </label>

        {error && (
          <div className="banner banner--error" style={{ margin: "0.2rem 0 0.9rem" }}>
            <span>{error}</span>
          </div>
        )}

        <div style={{ display: "flex", gap: "0.5rem", alignItems: "center" }}>
          {group && (
            <button
              type="button"
              className="btn btn--danger"
              disabled={busy}
              onClick={() => void remove()}
            >
              Delete
            </button>
          )}
          <span style={{ flex: 1 }} />
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn--primary"
            disabled={busy || !name.trim()}
            onClick={() => void save()}
          >
            {group ? "Save" : "Create"}
          </button>
        </div>
      </div>
    </div>
  );
}
