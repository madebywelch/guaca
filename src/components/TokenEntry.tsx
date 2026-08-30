/**
 * The one screen a browser sees before the workspace, and only a browser.
 *
 * A hosted workspace holds inference keys, plugin refresh tokens and every
 * transcript the crew has written, so there is no anonymous mode: every call
 * carries the token the daemon printed when it started. This is where that
 * token comes from when the page does not have it yet, and where it comes
 * from again when the box stops accepting the one it had.
 *
 * Most operators never see the form. The daemon prints an invitation with the
 * token in the URL's fragment, and a click on it is adopted here before the
 * first render decides anything, so the ordinary path is one click and then
 * the app. The form is for the other cases: a token rotated on the box, a
 * browser whose storage was cleared, an address typed by hand.
 *
 * On a desktop this renders its children and nothing else. The runtime is
 * inside the process asking, and there is no token to have.
 */

import { type ReactNode, useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { adoptInvitation, hosted, setToken, token, UNAUTHORIZED_EVENT } from "../lib/transport";
import { errorMessage } from "../lib/types";

export function TokenEntry({ children }: { children: ReactNode }) {
  // Decided once, before the first paint. An invitation in the address bar is
  // taken in the same breath, so a clicked link never flashes the form.
  const [admitted, setAdmitted] = useState(() => !hosted || adoptInvitation() || token() !== "");
  const [value, setValue] = useState("");
  const [refused, setRefused] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);

  // A token the box stopped accepting is one event on the window, raised by
  // the transport the moment any call is turned away. The app underneath is
  // unmounted, because every read it would make is the same refusal, and it
  // boots again from nothing once a token is accepted.
  useEffect(() => {
    if (!hosted) return;
    const turnedAway = () => {
      setAdmitted(false);
      setRefused("This workspace stopped accepting the token this browser had.");
    };
    window.addEventListener(UNAUTHORIZED_EVENT, turnedAway);
    return () => window.removeEventListener(UNAUTHORIZED_EVENT, turnedAway);
  }, []);

  if (admitted) return <>{children}</>;

  const submit = async () => {
    const pasted = value.trim();
    if (!pasted || checking) return;
    setChecking(true);
    setRefused(null);
    setToken(pasted);
    try {
      // The cheapest call there is, and the one the app reads first anyway.
      // A wrong token is refused here, on this screen, rather than forty
      // times by the reads the app would make.
      await api.capabilities();
      setValue("");
      setAdmitted(true);
    } catch (error) {
      setToken("");
      setRefused(errorMessage(error));
    } finally {
      setChecking(false);
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
        <h1 className="empty__title">This workspace needs its token</h1>
        <p className="empty__body">
          The box printed one when it started. It is in its logs, and in the <code>token</code> file
          beside its settings. Paste it here, or open the invitation link it printed.
        </p>
        <label className="field">
          <span className="field__label">Workspace token</span>
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
            <span>{refused}</span>
          </div>
        )}
        <button type="submit" className="btn btn--primary" disabled={checking || !value.trim()}>
          {checking ? "Checking…" : "Open workspace"}
        </button>
      </form>
    </main>
  );
}
