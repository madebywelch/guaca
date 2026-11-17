import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import {
  adoptInvitation,
  hosted,
  setToken,
  settleInvitation,
  token,
  UNAUTHORIZED_EVENT,
} from "../lib/transport";
import { asSentence, errorMessage } from "../lib/types";

function refusedForToken(error: unknown): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    (error as { kind?: unknown }).kind === "unauthorized"
  );
}

export function TokenEntry({ children }: { children: ReactNode }) {
  const [admitted, setAdmitted] = useState(() => !hosted || token() !== "");
  const [settling, setSettling] = useState(() => hosted && token() === "");
  const [value, setValue] = useState("");
  const [refused, setRefused] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const wasAdmitted = useRef(false);
  const generation = useRef(0);
  const submitting = useRef<number | null>(null);

  useEffect(() => {
    if (!hosted) return;
    adoptInvitation();
    if (token() !== "") {
      setSettling(false);
      return;
    }
    let cancelled = false;
    const current = ++generation.current;
    const valid = () => !cancelled && generation.current === current;
    const settle = async () => {
      if (await settleInvitation()) {
        if (valid()) {
          setAdmitted(true);
          setSettling(false);
        }
        return;
      }
      try {
        await api.capabilities();
        if (valid()) setAdmitted(true);
      } catch (error) {
        if (valid()) setAdmitted(!refusedForToken(error));
      } finally {
        if (valid()) setSettling(false);
      }
    };
    void settle();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!hosted) return;
    const onHash = () => {
      if (!adoptInvitation()) return;
      const current = ++generation.current;
      submitting.current = null;
      setChecking(false);
      setSettling(true);
      void settleInvitation().then((settled) => {
        if (generation.current !== current) return;
        setSettling(false);
        if (settled) setAdmitted(true);
      });
    };
    window.addEventListener("hashchange", onHash);
    return () => {
      generation.current++;
      window.removeEventListener("hashchange", onHash);
    };
  }, []);

  useLayoutEffect(() => {
    if (admitted) wasAdmitted.current = true;
  }, [admitted]);

  useEffect(() => {
    if (!hosted) return;
    const turnedAway = () => {
      if (submitting.current !== null) return;
      generation.current++;
      setAdmitted(false);
      setSettling(false);
      if (wasAdmitted.current) {
        setRefused("This host stopped accepting this browser's session.");
      }
    };
    window.addEventListener(UNAUTHORIZED_EVENT, turnedAway);
    return () => window.removeEventListener(UNAUTHORIZED_EVENT, turnedAway);
  }, []);

  if (admitted) return <>{children}</>;

  if (settling) {
    return (
      <main className="threshold">
        <p className="empty__body" role="status">
          Opening Guaca…
        </p>
      </main>
    );
  }

  const submit = async () => {
    const pasted = value.trim();
    if (!pasted || checking) return;
    const current = ++generation.current;
    submitting.current = current;
    setChecking(true);
    setRefused(null);
    try {
      await setToken(pasted);
      await api.capabilities();
      if (generation.current !== current) return;
      setValue("");
      setAdmitted(true);
    } catch (error) {
      if (generation.current === current) setRefused(errorMessage(error));
    } finally {
      if (submitting.current === current) submitting.current = null;
      if (generation.current === current) setChecking(false);
    }
  };

  return (
    <main className="threshold">
      <form
        className="empty"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <h1 className="empty__title">This host needs its token</h1>
        <p className="empty__body">
          The box printed one when it started. It is in its logs, and in the <code>token</code> file
          beside its settings. Paste it here, or open the invitation link it printed.
        </p>
        <label className="field">
          <span className="field__label">Host token</span>
          <input
            className="input input--mono"
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={value}
            disabled={checking}
            onChange={(event) => setValue(event.target.value)}
          />
        </label>
        {refused && (
          <div className="banner banner--error" role="alert">
            <span>{asSentence(refused)}</span>
          </div>
        )}
        <button type="submit" className="btn btn--primary" disabled={checking || !value.trim()}>
          {checking ? "Checking…" : "Open Guaca"}
        </button>
      </form>
    </main>
  );
}
