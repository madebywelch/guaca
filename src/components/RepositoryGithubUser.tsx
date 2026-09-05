import { useEffect, useRef, useState } from "react";
import { api, openExternal } from "../lib/ipc";
import {
  errorMessage,
  type GithubUserSignin,
  type GithubUserStatus,
  type GitIdentity,
  type RepositoryId,
} from "../lib/types";

export function RepositoryGithubUser({
  id,
  onAuthorized,
}: {
  id: RepositoryId;
  onAuthorized: (author: GitIdentity) => void;
}) {
  const [user, setUser] = useState<GithubUserStatus | null>(null);
  const [flow, setFlow] = useState<GithubUserSignin | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const latest = useRef(onAuthorized);
  latest.current = onAuthorized;

  useEffect(() => {
    let active = true;
    api
      .repositoryGithubUser(id)
      .then((value) => {
        if (active) setUser(value);
      })
      .catch((caught) => {
        if (active) setError(errorMessage(caught));
      });
    return () => {
      active = false;
    };
  }, [id]);

  useEffect(() => {
    if (!flow) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    const deadline = Date.now() + flow.expiresIn * 1000;
    const poll = async () => {
      try {
        if (Date.now() >= deadline) throw new Error("GitHub sign-in expired. Start again.");
        const value = await api.pollRepositoryGithubSignin(id, flow.flowId);
        if (!active) return;
        if (value.status === "authorized" && value.author) {
          setUser(value);
          setFlow(null);
          latest.current(value.author);
        } else {
          timer = setTimeout(poll, Math.max(5, value.interval ?? flow.interval) * 1000);
        }
      } catch (caught) {
        if (active) {
          setError(errorMessage(caught));
          setFlow(null);
        }
      }
    };
    timer = setTimeout(poll, Math.max(5, flow.interval) * 1000);
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [id, flow]);

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
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
      <p className="field__hint">
        {user?.status === "authorized"
          ? `Pull requests are opened as ${user.login}.`
          : "Sign in to GitHub to open pull requests under your account."}{" "}
        Signing in also sets your commit name and GitHub noreply email for this repository.
      </p>
      {!flow && (
        <button
          type="button"
          className="btn btn--small"
          disabled={busy}
          onClick={() => void run(async () => setFlow(await api.beginRepositoryGithubSignin(id)))}
        >
          {user?.status === "authorized" ? "Change GitHub account" : "Sign in to GitHub"}
        </button>
      )}
      {flow && (
        <>
          <p className="field__hint" role="status">
            Enter this code on GitHub: <code>{flow.userCode}</code>. Waiting for authorization…
          </p>
          <button
            type="button"
            className="btn btn--small"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                if (flow.verificationUri !== "https://github.com/login/device")
                  throw new Error("Unexpected GitHub verification address");
                await openExternal(flow.verificationUri);
              })
            }
          >
            Open GitHub
          </button>
          <button type="button" className="btn btn--ghost btn--small" onClick={() => setFlow(null)}>
            Cancel sign-in
          </button>
        </>
      )}
      {user?.status === "authorized" && !flow && (
        <button
          type="button"
          className="btn btn--ghost btn--small"
          disabled={busy}
          onClick={() => void run(async () => setUser(await api.signOutRepositoryGithubUser(id)))}
        >
          Sign out of GitHub
        </button>
      )}
      {error && (
        <p className="field__hint" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
