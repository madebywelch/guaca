import { type ReactNode, useEffect, useRef, useState } from "react";
import { api, openExternal } from "../lib/ipc";
import type { ClaudeSignin as Session } from "../lib/types";

const genericError = "Claude sign-in didn't finish.";
const safeErrors = new Set([
  genericError,
  "Claude sign-in was cancelled.",
  "Claude sign-in timed out.",
  "Claude sign-in is no longer active.",
  "Enter the code from the Claude sign-in page.",
  "Claude couldn't read that code; copy the full code.",
]);

function signinError(value: unknown): string {
  if (
    typeof value === "object" &&
    value !== null &&
    "kind" in value &&
    value.kind === "claudeSignin" &&
    "message" in value &&
    typeof value.message === "string" &&
    safeErrors.has(value.message)
  )
    return value.message;
  return genericError;
}

type Request =
  | { kind: "idle" }
  | { kind: "starting" }
  | { kind: "session"; id: string }
  | { kind: "submitting"; id: string };

type RequestEvent =
  | { type: "start" }
  | { type: "opened"; id: string }
  | { type: "submit" }
  | { type: "reset" };

export function signinRequest(state: Request, event: RequestEvent): Request {
  switch (event.type) {
    case "start":
      return state.kind === "idle" ? { kind: "starting" } : state;
    case "opened":
      return state.kind === "starting" ? { kind: "session", id: event.id } : state;
    case "submit":
      return state.kind === "session" ? { kind: "submitting", id: state.id } : state;
    case "reset":
      return state.kind === "idle" ? state : { kind: "idle" };
  }
}

export function ClaudeSignin({
  onReady,
  row,
  enabled = true,
}: {
  onReady?: (ready: boolean) => void;
  row?: (action: ReactNode, signedIn: boolean) => ReactNode;
  enabled?: boolean;
}) {
  const [signedIn, setSignedIn] = useState(false);
  const [checking, setChecking] = useState(true);
  const [session, setSession] = useState<Session | null>(null);
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const generation = useRef(0);
  const active = useRef<Request>({ kind: "idle" });

  useEffect(() => {
    if (!enabled) return;
    const current = generation.current;
    void api.claudeSigninStatus().then(
      (status) => {
        if (generation.current === current) {
          setSignedIn(status.signedIn);
          setChecking(false);
        }
      },
      (failure) => {
        if (generation.current === current) {
          setError(signinError(failure));
          setChecking(false);
        }
      },
    );
    return () => {
      generation.current += 1;
      if ("id" in active.current) void api.cancelClaudeSignin(active.current.id).catch(() => {});
      active.current = signinRequest(active.current, { type: "reset" });
    };
  }, [enabled]);

  useEffect(() => {
    onReady?.(signedIn && !busy && !error && !checking);
  }, [signedIn, busy, error, checking, onReady]);

  const cancel = () => {
    generation.current += 1;
    if ("id" in active.current) void api.cancelClaudeSignin(active.current.id).catch(() => {});
    active.current = signinRequest(active.current, { type: "reset" });
    setSession(null);
    setCode("");
    setBusy(false);
    setError("");
  };

  const begin = async () => {
    const nextState = signinRequest(active.current, { type: "start" });
    if (nextState === active.current) return;
    active.current = nextState;
    const current = ++generation.current;
    setBusy(true);
    setError("");
    try {
      const next = await api.beginClaudeSignin();
      if (generation.current !== current) {
        void api.cancelClaudeSignin(next.id).catch(() => {});
        return;
      }
      active.current = signinRequest(active.current, { type: "opened", id: next.id });
      setSession(next);
    } catch (failure) {
      if (generation.current === current) {
        active.current = signinRequest(active.current, { type: "reset" });
        setError(signinError(failure));
      }
    } finally {
      if (generation.current === current) setBusy(false);
    }
  };

  const complete = async () => {
    if (!session || !code.trim()) return;
    const nextState = signinRequest(active.current, { type: "submit" });
    if (nextState === active.current) return;
    active.current = nextState;
    const current = generation.current;
    setBusy(true);
    setError("");
    try {
      const result = await api.completeClaudeSignin(session.id, code.trim());
      if (generation.current !== current) return;
      if (!result.signedIn) throw new Error("Sign-in was not completed");
      active.current = signinRequest(active.current, { type: "reset" });
      setSession(null);
      setCode("");
      setSignedIn(true);
    } catch (failure) {
      if (generation.current === current) {
        void api.cancelClaudeSignin(session.id).catch(() => {});
        active.current = signinRequest(active.current, { type: "reset" });
        setSession(null);
        setCode("");
        setError(signinError(failure));
      }
    } finally {
      if (generation.current === current) setBusy(false);
    }
  };

  const action = !signedIn && (
    <button
      type="button"
      className="btn btn--small"
      aria-label={!busy && !checking && !session && !error ? "Sign in with Claude" : undefined}
      disabled={busy || checking || Boolean(session)}
      onClick={() => void begin()}
    >
      {busy || checking || session
        ? "Signing in…"
        : error
          ? "Try again"
          : row
            ? "Sign in"
            : "Sign in with Claude"}
    </button>
  );

  const expanded = Boolean(session || busy || error);
  return (
    <div className="claude-signin" data-expanded={Boolean(row && expanded)}>
      {row?.(action, signedIn)}
      {(!row || expanded) && (
        <div className="claude-signin__panel">
          {signedIn ? (
            !row && <p>Signed in to Claude on this host.</p>
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
                      if (generation.current === current) {
                        setError("The browser didn't open. Copy the address above to continue.");
                      }
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
              <div className="claude-signin__actions">
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
              </div>
            </>
          ) : (
            <>
              {!row && action}
              {busy && (
                <button type="button" className="btn" onClick={cancel}>
                  Cancel sign-in
                </button>
              )}
            </>
          )}
          {error && <p role="alert">{error}</p>}
        </div>
      )}
    </div>
  );
}
