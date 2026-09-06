import { useEffect, useState } from "react";
import { api, openExternal } from "../lib/ipc";
import {
  type RepositoryConnection as Connection,
  errorMessage,
  type RepositoryId,
} from "../lib/types";
import { GitAuthor } from "./GitAuthor";
import { RepositoryGithubUser } from "./RepositoryGithubUser";
import { RepositoryToken, type TokenChoice } from "./RepositoryToken";

/** Git access belongs to the repository, independently of the coding harness. */
export function RepositoryConnection({
  id,
  editing = false,
}: {
  id: RepositoryId;
  editing?: boolean;
}) {
  const [revision, setRevision] = useState(0);
  const [connection, setConnection] = useState<Connection | null>(null);
  const [author, setAuthor] = useState({ name: "", email: "" });
  const [tokenChoice, setTokenChoice] = useState<TokenChoice>({ kind: "loading" });
  const [changingToken, setChangingToken] = useState(false);
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

  useEffect(() => {
    let active = true;
    setConnection(null);
    setError(null);
    void (async () => {
      try {
        const next = await api.repositoryConnection(id);
        if (!active) return;
        setConnection(next);
        setAuthor(next.author ?? { name: "", email: "" });
      } catch (caught) {
        if (active) setError(errorMessage(caught));
      }
    })();
    return () => {
      active = false;
    };
  }, [id, revision]);

  useEffect(() => {
    if (!editing) {
      setChangingToken(false);
      setTokenChoice({ kind: "loading" });
      setAuthor(connection?.author ?? { name: "", email: "" });
    }
  }, [editing, connection]);

  return (
    <div className="field">
      <span className="field__label">Git access</span>
      {connection ? (
        <>
          <p className="field__hint">
            {connection.githubApp
              ? "GitHub App connected. Git renews its tokens automatically."
              : connection.managedCredential
                ? "Repository token saved. You do not need to enter it again."
                : "No token saved in Guaca. Git uses credentials configured on the backend."}
          </p>
          <p className="field__hint">
            {connection.author?.name &&
            connection.author?.email &&
            connection.author.email !== "guaca@localhost"
              ? `Commit author: ${connection.author.name} <${connection.author.email}>`
              : "Commit author is not configured on this backend. Set it in Edit before committing."}
          </p>
          {editing && (
            <>
              {connection.githubApp && (
                <RepositoryGithubUser
                  id={id}
                  onAuthorized={(identity) => {
                    setAuthor(identity);
                    setConnection((current) =>
                      current ? { ...current, author: identity } : current,
                    );
                    setChecked(
                      "GitHub connected. Future commits and pull requests use your account.",
                    );
                  }}
                />
              )}
              <GitAuthor author={author} disabled={busy} onChange={setAuthor} />
              {(!connection.author?.name ||
                !connection.author?.email ||
                connection.author.email === "guaca@localhost") && (
                <p className="field__hint">
                  Set your identity before asking an engineer to commit code.
                </p>
              )}
              <button
                type="button"
                className="btn btn--small"
                disabled={busy || !author.name.trim() || !author.email.trim()}
                onClick={() =>
                  void run(async () => {
                    const next = await api.setRepositoryAuthor(id, author);
                    setConnection(next);
                    setAuthor(next.author ?? author);
                    setChecked("Commit author saved. Existing commits are unchanged.");
                  })
                }
              >
                Save commit author
              </button>
              <p className="field__hint">Origin: {connection.remote ?? "No origin configured"}</p>
              {connection.pushRemote !== connection.remote && (
                <p className="field__hint">
                  Push remote: {connection.pushRemote}. A token saved here applies to origin only.
                </p>
              )}
              <p className="field__hint">
                {connection.githubApp
                  ? "GitHub App access is connected. Git obtains short-lived tokens automatically; pull requests require your GitHub sign-in."
                  : "Git access lets agents read and push this repository."}{" "}
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
              {connection.acceptsToken && !connection.githubApp && connection.managedCredential && (
                <button
                  type="button"
                  className="btn btn--small"
                  aria-expanded={changingToken}
                  disabled={busy}
                  onClick={() => {
                    setChangingToken(!changingToken);
                    setTokenChoice({ kind: "loading" });
                  }}
                >
                  {changingToken ? "Cancel token change" : "Change saved token"}
                </button>
              )}
              {connection.acceptsToken &&
                !connection.githubApp &&
                (!connection.managedCredential || changingToken) && (
                  <>
                    <RepositoryToken
                      remote={connection.remote ?? ""}
                      choice={tokenChoice}
                      onChange={setTokenChoice}
                      disabled={busy}
                      tokenLabel="Repository access token"
                      allowExisting={false}
                    />
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
                      disabled={
                        busy ||
                        tokenChoice.kind === "loading" ||
                        tokenChoice.kind === "existing" ||
                        (tokenChoice.kind === "new" && !tokenChoice.token.trim())
                      }
                      onClick={() =>
                        void run(async () => {
                          if (tokenChoice.kind === "saved") {
                            setConnection(await api.reuseRepositoryCredential(id, tokenChoice.id));
                          } else if (tokenChoice.kind === "new") {
                            const { username, token } = tokenChoice;
                            setTokenChoice({ ...tokenChoice, token: "" });
                            setConnection(await api.setRepositoryCredential(id, username, token));
                          }
                          setChangingToken(false);
                          setChecked(
                            "Repository token saved. Existing access on other repositories is unchanged.",
                          );
                        })
                      }
                    >
                      {tokenChoice.kind === "saved" ? "Use saved token" : "Save token"}
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
                      setTokenChoice({ kind: "new", username: "", token: "" });
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
        </>
      ) : !error ? (
        <p className="field__hint">Loading Git settings…</p>
      ) : null}
      {busy && (
        <p className="field__hint" role="status">
          Updating Git settings…
        </p>
      )}
      {checked && (
        <p className="field__hint" role="status">
          {checked}
        </p>
      )}
      {error && (
        <>
          <p className="field__hint" role="alert">
            {error}
          </p>
          {!connection && (
            <button
              type="button"
              className="btn btn--small"
              onClick={() => setRevision(revision + 1)}
            >
              Retry Git settings
            </button>
          )}
        </>
      )}
    </div>
  );
}
