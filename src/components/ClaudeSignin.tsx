import { useEffect, useRef, useState } from "react";
import { api, openExternal } from "../lib/ipc";
import type { ClaudeSignin as Session } from "../lib/types";

export function ClaudeSignin() {
  const [signedIn, setSignedIn] = useState(false);
  const [session, setSession] = useState<Session | null>(null);
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const generation = useRef(0);
  const active = useRef<string | null>(null);

  useEffect(() => {
    const current = generation.current;
    void api.claudeSigninStatus().then(
      (status) => {
        if (generation.current === current) setSignedIn(status.signedIn);
      },
      () => {
        if (generation.current === current) setError(true);
      },
    );
    return () => {
      generation.current += 1;
      if (active.current) void api.cancelClaudeSignin(active.current).catch(() => {});
      active.current = null;
    };
  }, []);

  const cancel = () => {
    generation.current += 1;
    if (active.current) void api.cancelClaudeSignin(active.current).catch(() => {});
    active.current = null;
    setSession(null);
    setCode("");
    setBusy(false);
    setError(false);
  };

  const begin = async () => {
    const current = ++generation.current;
    setBusy(true);
    setError(false);
    try {
      const next = await api.beginClaudeSignin();
      if (generation.current !== current) {
        void api.cancelClaudeSignin(next.id).catch(() => {});
        return;
      }
      active.current = next.id;
      setSession(next);
    } catch {
      if (generation.current === current) setError(true);
    } finally {
      if (generation.current === current) setBusy(false);
    }
  };

  const complete = async () => {
    if (!session || !code.trim() || busy) return;
    const current = generation.current;
    setBusy(true);
    setError(false);
    try {
      const result = await api.completeClaudeSignin(session.id, code.trim());
      if (generation.current !== current) return;
      if (!result.signedIn) throw new Error("Sign-in was not completed");
      active.current = null;
      setSession(null);
      setCode("");
      setSignedIn(true);
    } catch {
      if (generation.current === current) {
        void api.cancelClaudeSignin(session.id).catch(() => {});
        active.current = null;
        setSession(null);
        setCode("");
        setError(true);
      }
    } finally {
      if (generation.current === current) setBusy(false);
    }
  };

  return (
    <div>
      {signedIn ? (
        <p>Signed in to Claude on this host.</p>
      ) : session ? (
        <>
          <p>Open this address, sign in, then paste the code Claude gives you here:</p>
          <p>
            <a
              className="claude-signin__link"
              href={session.url}
              target="_blank"
              rel="noreferrer"
              onClick={(event) => {
                event.preventDefault();
                const current = generation.current;
                void openExternal(session.url).catch(() => {
                  if (generation.current === current) setError(true);
                });
              }}
            >
              {session.url}
            </a>
          </p>
          <label className="field">
            <span className="field__label">Code from browser</span>
            <input
              className="input input--mono"
              value={code}
              onChange={(event) => setCode(event.target.value)}
              autoComplete="off"
            />
          </label>
          <button
            type="button"
            className="btn"
            disabled={busy || !code.trim()}
            onClick={() => void complete()}
          >
            Submit code
          </button>
          <button type="button" className="btn" onClick={cancel}>
            Cancel sign-in
          </button>
        </>
      ) : (
        <>
          <button type="button" className="btn" disabled={busy} onClick={() => void begin()}>
            Sign in with Claude
          </button>
          {busy && (
            <button type="button" className="btn" onClick={cancel}>
              Cancel sign-in
            </button>
          )}
        </>
      )}
      {error && <p role="alert">Claude sign-in failed. Try again.</p>}
    </div>
  );
}
