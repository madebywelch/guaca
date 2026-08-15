import { useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { errorMessage, type Group, type GroupDraft } from "../lib/types";

interface Props {
  /** Absent means create. */
  group?: Group;
  onClose: () => void;
}

/**
 * A group's name, its wall, and the inference settings its agents inherit.
 *
 * Every field except the name is an override. Left blank, it inherits the app
 * default, which is why the placeholders show what would be used instead of a
 * value: an operator has to be able to tell "this group uses the app model"
 * apart from "this group pins that exact model".
 */
export function GroupEditor({ group, onClose }: Props) {
  const refreshAgents = useStore((s) => s.refreshAgents);
  const settings = useStore((s) => s.settings);

  const [draft, setDraft] = useState<GroupDraft>({
    name: group?.name ?? "",
    baseUrl: group?.baseUrl ?? "",
    defaultModel: group?.defaultModel ?? "",
  });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);
  const [cleared, setCleared] = useState<number | null>(null);
  const nameRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    nameRef.current?.focus();
    nameRef.current?.select();
  }, []);

  const patch = (next: Partial<GroupDraft>) => setDraft((d) => ({ ...d, ...next }));

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      // `apiKey` is only sent when the operator typed one. Sending the redacted
      // hint back would overwrite the real key with its own placeholder.
      if (group) await api.updateGroup(group.id, draft);
      else await api.createGroup(draft);
      await refreshAgents();
      onClose();
    } catch (caught) {
      setError(errorMessage(caught));
      setBusy(false);
    }
  };

  /** Start fresh: the crew stays, everything it said goes. */
  const clear = async () => {
    if (!group) return;
    setBusy(true);
    setError(null);
    try {
      const gone = await api.clearGroup(group.id);
      setCleared(gone);
      setConfirmClear(false);
      await refreshAgents();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
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
      // The usual failure is a group that still holds agents. The message from
      // Rust already says how many and what to do, so it is shown as written.
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
        aria-label={group ? "Group settings" : "New group"}
      >
        <h2 className="dialog__title">{group ? draft.name || group.name : "New group"}</h2>
        <p className="dialog__lede" style={{ marginTop: 0 }}>
          Agents in different groups cannot see or message each other. Everything below is inherited
          from the app settings unless this group sets it.
        </p>

        <label className="field">
          <span className="field__label">Name</span>
          <input
            className="input input--mono"
            ref={nameRef}
            value={draft.name}
            maxLength={48}
            placeholder="Research"
            onChange={(event) => patch({ name: event.target.value })}
            onKeyDown={(event) => {
              if (event.key === "Enter" && draft.name.trim()) void save();
            }}
          />
        </label>

        <label className="field">
          <span className="field__label">Model</span>
          <input
            className="input input--mono"
            value={draft.defaultModel ?? ""}
            placeholder={settings?.defaultModel || "inherit"}
            onChange={(event) => patch({ defaultModel: event.target.value })}
          />
          <span className="field__hint">
            Used by every agent in this group that does not name its own model.
          </span>
        </label>

        <label className="field">
          <span className="field__label">Inference endpoint</span>
          <input
            className="input input--mono"
            value={draft.baseUrl ?? ""}
            placeholder={settings?.baseUrl || "inherit"}
            onChange={(event) => patch({ baseUrl: event.target.value })}
          />
          <span className="field__hint">
            Any OpenAI-compatible base URL. One group can run against a local server while another
            uses a hosted one.
          </span>
        </label>

        <label className="field">
          <span className="field__label">API key</span>
          <input
            className="input input--mono"
            type="password"
            value={draft.apiKey ?? ""}
            placeholder={group?.apiKeySet ? `set · ${group.apiKeyHint}` : "inherit"}
            onChange={(event) => patch({ apiKey: event.target.value })}
          />
          <span className="field__hint">
            Only needed when this group's endpoint uses a different key. Leave blank to keep the
            stored one.
          </span>
        </label>

        {error && (
          <div className="banner banner--error" style={{ margin: "0.2rem 0 0.9rem" }}>
            <span>{error}</span>
          </div>
        )}

        <div style={{ display: "flex", gap: "0.5rem", alignItems: "center" }}>
          {group &&
            (confirmDelete ? (
              <>
                <button
                  type="button"
                  className="btn btn--danger"
                  disabled={busy}
                  onClick={() => void remove()}
                >
                  Delete {group.name}
                </button>
                <button
                  type="button"
                  className="btn btn--ghost"
                  onClick={() => setConfirmDelete(false)}
                >
                  Keep
                </button>
              </>
            ) : (
              <button
                type="button"
                className="btn btn--danger"
                onClick={() => setConfirmDelete(true)}
              >
                Delete
              </button>
            ))}
          {group &&
            !confirmDelete &&
            (confirmClear ? (
              <>
                <button
                  type="button"
                  className="btn btn--danger"
                  disabled={busy}
                  onClick={() => void clear()}
                >
                  Clear every chat
                </button>
                <button
                  type="button"
                  className="btn btn--ghost"
                  onClick={() => setConfirmClear(false)}
                >
                  Keep them
                </button>
              </>
            ) : (
              <button
                type="button"
                className="btn btn--ghost"
                disabled={busy}
                onClick={() => setConfirmClear(true)}
                title="Empties every channel in this group. The agents, their notes and their computers stay."
              >
                {cleared === null ? "Start fresh" : `Cleared ${cleared}`}
              </button>
            ))}
          <span style={{ flex: 1 }} />
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn--primary"
            disabled={busy || !draft.name.trim()}
            onClick={() => void save()}
          >
            {group ? "Save" : "Create"}
          </button>
        </div>
      </div>
    </div>
  );
}
