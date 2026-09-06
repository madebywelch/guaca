import { useEffect, useState } from "react";
import { api } from "../lib/ipc";
import { errorMessage, type SavedRepositoryCredential } from "../lib/types";

export type TokenChoice =
  | { kind: "loading" }
  | { kind: "saved"; id: string }
  | { kind: "new"; username: string; token: string }
  | { kind: "existing" };

/** Only opaque IDs are offered for reuse. A password is always a new input. */
export function RepositoryToken({
  remote,
  choice,
  onChange,
  disabled,
  tokenLabel = "Access token",
  allowExisting = true,
}: {
  remote: string;
  choice: TokenChoice;
  onChange: (choice: TokenChoice) => void;
  disabled: boolean;
  tokenLabel?: string;
  allowExisting?: boolean;
}) {
  const [saved, setSaved] = useState<SavedRepositoryCredential[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    onChange({ kind: "loading" });
    api.savedRepositoryCredentials(remote).then(
      (next) => {
        if (!active) return;
        setSaved(next);
        onChange(
          next[0] ? { kind: "saved", id: next[0].id } : { kind: "new", username: "", token: "" },
        );
      },
      (caught) => {
        if (!active) return;
        setError(errorMessage(caught));
        onChange({ kind: "new", username: "", token: "" });
      },
    );
    return () => {
      active = false;
    };
  }, [remote, onChange]);

  return (
    <>
      <label className="field">
        <span className="field__label">Git credential</span>
        <select
          className="input"
          disabled={disabled || choice.kind === "loading"}
          value={choice.kind === "saved" ? choice.id : choice.kind}
          onChange={(event) => {
            const value = event.target.value;
            onChange(
              value === "new"
                ? { kind: "new", username: "", token: "" }
                : value === "existing"
                  ? { kind: "existing" }
                  : { kind: "saved", id: value },
            );
          }}
        >
          {choice.kind === "loading" && (
            <option value="loading">Checking saved credentials…</option>
          )}
          {saved.map((entry) => (
            <option key={entry.id} value={entry.id}>
              Saved: {entry.username} · {entry.remote}
            </option>
          ))}
          <option value="new">Use a different token</option>
          {allowExisting && (
            <option value="existing">Use existing Git credentials or public access</option>
          )}
        </select>
      </label>
      {error && (
        <p className="field__hint" role="alert">
          Could not load saved credentials: {error}
        </p>
      )}
      {choice.kind === "saved" && (
        <p className="field__hint">
          Reuses this token for the selected repository on the same server. The token must grant
          access to it.
        </p>
      )}
      {choice.kind === "existing" && (
        <p className="field__hint">
          Uses Git credentials already configured on the backend. Public repositories need none.
        </p>
      )}
      {choice.kind === "new" && (
        <>
          <label className="field">
            <span className="field__label">{tokenLabel}</span>
            <input
              className="input"
              type="password"
              autoComplete="off"
              spellCheck={false}
              disabled={disabled}
              value={choice.token}
              onChange={(event) => onChange({ ...choice, token: event.target.value })}
            />
          </label>
          <label className="field">
            <span className="field__label">Git username</span>
            <input
              className="input"
              autoComplete="off"
              disabled={disabled}
              placeholder="git (or the username your service requires)"
              value={choice.username}
              onChange={(event) => onChange({ ...choice, username: event.target.value })}
            />
          </label>
        </>
      )}
    </>
  );
}
