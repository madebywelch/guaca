import { useState } from "react";
import { api, openExternal } from "../lib/ipc";
import {
  type RepositoryConnection as Connection,
  errorMessage,
  type RepositoryId,
} from "../lib/types";

/** Git access belongs to the repository, independently of the coding harness. */
export function RepositoryConnection({ id }: { id: RepositoryId }) {
  const [open, setOpen] = useState(false);
  const [connection, setConnection] = useState<Connection | null>(null);
  const [username, setUsername] = useState("");
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [checked, setChecked] = useState<string | null>(null);

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    setChecked(null);
    try {
      await action();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div>
      <button
        type="button"
        className="btn btn--ghost btn--small"
        aria-expanded={open}
        disabled={busy}
        onClick={() => {
          setOpen(!open);
          setToken("");
          if (!open) void run(async () => setConnection(await api.repositoryConnection(id)));
        }}
      >
        Git access
      </button>
      {open && (
        <>
          {connection && (
            <>
              <p className="field__hint">Origin: {connection.remote ?? "No origin configured"}</p>
              {connection.pushRemote !== connection.remote && (
                <p className="field__hint">
                  Push remote: {connection.pushRemote}. A token saved here applies to origin only.
                </p>
              )}
              <p className="field__hint">
                {connection.githubApp
                  ? "GitHub App access is connected. Git and pull-request commands obtain short-lived tokens automatically."
                  : connection.managedCredential
                    ? "A repository token is saved on the backend."
                    : "No repository token is saved. Git uses the backend's configured access."}{" "}
                Codex and Claude sign-ins do not grant Git access.
              </p>
              {connection.githubAvailable && !connection.githubApp && (
                <button
                  type="button"
                  className="btn btn--small"
                  disabled={busy}
                  onClick={() =>
                    void run(async () => setConnection(await api.setRepositoryGithub(id)))
                  }
                >
                  Connect GitHub App
                </button>
              )}
              {connection.acceptsToken && !connection.githubApp && (
                <>
                  <label className="field">
                    <span className="field__label">Git username</span>
                    <input
                      className="input"
                      autoComplete="off"
                      value={username}
                      placeholder="git (or the username your service requires)"
                      onChange={(event) => setUsername(event.target.value)}
                    />
                  </label>
                  <label className="field">
                    <span className="field__label">Repository access token</span>
                    <input
                      className="input"
                      type="password"
                      autoComplete="off"
                      spellCheck={false}
                      value={token}
                      onChange={(event) => setToken(event.target.value)}
                    />
                  </label>
                  <p className="field__hint">
                    Create a token with read and write access to this repository in your Git
                    service. Saving it replaces the previous token; it is never read back.
                    {connection.remote?.startsWith("https://github.com/") && (
                      <>
                        {" "}
                        <button
                          type="button"
                          className="btn btn--ghost btn--small"
                          onClick={() =>
                            void run(async () => {
                              await openExternal(
                                "https://github.com/settings/personal-access-tokens/new",
                              );
                            })
                          }
                        >
                          Create GitHub token
                        </button>{" "}
                        Choose this repository and Contents: read and write.
                      </>
                    )}
                  </p>
                  <button
                    type="button"
                    className="btn btn--small"
                    disabled={busy || !token.trim()}
                    onClick={() =>
                      void run(async () => {
                        const value = token;
                        setToken("");
                        setConnection(await api.setRepositoryCredential(id, username, value));
                      })
                    }
                  >
                    Save token
                  </button>
                </>
              )}
              {!connection.acceptsToken && connection.remote && (
                <p className="field__hint">
                  Configure SSH keys or the credential helper under the backend user for this
                  remote.
                </p>
              )}
              {(connection.managedCredential || connection.githubApp) && (
                <button
                  type="button"
                  className="btn btn--ghost btn--small"
                  disabled={busy}
                  onClick={() =>
                    void run(async () => {
                      setToken("");
                      setConnection(await api.clearRepositoryCredential(id));
                    })
                  }
                >
                  {connection.githubApp ? "Disconnect GitHub App" : "Remove saved token"}
                </button>
              )}
              <button
                type="button"
                className="btn btn--small"
                disabled={busy || !connection.remote}
                onClick={() =>
                  void run(async () => setChecked(await api.checkRepositoryConnection(id)))
                }
              >
                Check read and push access
              </button>
            </>
          )}
          {busy && (
            <p className="field__hint" role="status">
              Checking Git access…
            </p>
          )}
          {checked && (
            <p className="field__hint" role="status">
              {checked}
            </p>
          )}
          {error && (
            <p className="field__hint" role="alert">
              {error}
            </p>
          )}
        </>
      )}
    </div>
  );
}
