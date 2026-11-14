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
 * On a desktop this renders its children and nothing else. The window keeps
 * the token for the host it was pointed at, and the runtime underneath reads
 * with it.
 *
 * The credential a browser keeps is an HttpOnly cookie, which is the point of
 * it: nothing on this page can read it back. So whether a session exists is a
 * question the daemon answers, and this screen asks it before deciding. The
 * form is what a refusal draws, never what the first paint guesses.
 */

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

/** Whether a refusal is this browser being told it needs a token. */
function refusedForToken(error: unknown): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    (error as { kind?: unknown }).kind === "unauthorized"
  );
}

export function TokenEntry({ children }: { children: ReactNode }) {
  // A window pointed at a box carries its token in the Remote record and the
  // app underneath reads with it. Only a browser has to be asked, and only
  // when it has nothing to present.
  const [admitted, setAdmitted] = useState(() => !hosted || token() !== "");
  // A browser has no script-readable token to look at, so on a reload the only
  // way to know whether it is signed in is to ask. Until the answer arrives
  // this holds the viewport rather than drawing the form, because a page that
  // flashed "this host needs its token" on its way to the workspace would be
  // asking for a credential the operator already gave it.
  const [settling, setSettling] = useState(() => hosted && token() === "");
  const [value, setValue] = useState("");
  const [refused, setRefused] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  // Whether this browser was ever let in. A refusal that arrives while the
  // first question is still being asked is not a session that expired, and
  // telling a first visitor their session stopped would name a problem they
  // never had.
  const wasAdmitted = useRef(false);
  const generation = useRef(0);
  const submitting = useRef<number | null>(null);

  // On mount, turn an invitation from the address bar or a token an older
  // build left in localStorage into a session cookie. The exchange is async
  // because the daemon sets an HttpOnly cookie, which only a request can do.
  // Nothing is kept in script-readable storage once the exchange succeeds.
  //
  // With nothing to exchange, the cookie may already be valid: a reload of a
  // signed-in page has no invitation and no stored token. Asking is the only
  // way to know, and `capabilities` is the cheapest question there is. Only a
  // refusal for the token means this browser has to be given one; a box that
  // is down or a build that is too old is the app's own banner to draw, and
  // holding its workspace back would only hide the sentence that says why.
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

  // An invitation pasted into a tab that is already open is a same-document
  // navigation: the fragment changes and nothing reloads, so the mount above
  // never sees it. The address bar is where an operator keeps the link, so
  // this is the case where clicking it again has to work.
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

  // Remembered for the listener below, which cannot see the render that
  // admitted this browser. A layout effect, so it holds from the commit that
  // draws the workspace: a passive one ran after the workspace's own effects,
  // and a refusal raised by anything they started read a browser that had
  // been let in as a first visitor.
  useLayoutEffect(() => {
    if (admitted) wasAdmitted.current = true;
  }, [admitted]);

  // A token the box stopped accepting is one event on the window, raised by
  // the transport the moment any call is turned away. The app underneath is
  // unmounted, because every read it would make is the same refusal, and it
  // boots again from nothing once a token is accepted.
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
      // Exchanges the pasted token for the HttpOnly session cookie. The token
      // crosses once and is not stored after this.
      await setToken(pasted);
      // The cheapest call there is, and the one the app reads first anyway.
      // A wrong token is refused here, on this screen, rather than forty
      // times by the reads the app would make.
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
          // The daemon writes for a log, and this is the one line read as
          // prose. Finished here rather than in Rust, for the reason every
          // banner is: `asSentence` is the app's one rule for it.
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
