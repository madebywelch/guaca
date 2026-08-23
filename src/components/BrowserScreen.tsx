import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import { prefersReducedMotion } from "../lib/motion";
import { useStore } from "../lib/store";
import { type AgentCard, type Browser, errorMessage } from "../lib/types";

interface Props {
  agent: AgentCard;
}

/**
 * An agent's browser, below its computer in the panel.
 *
 * Deliberately the same shape as `ComputerScreen`: two sizes, one connection,
 * read-only behind a veil in the panel and interactive full screen. They are
 * different places and an operator has to be able to tell them apart at a
 * glance, but the way you watch one and take over is a thing worth learning
 * once.
 *
 * Taking over is not a nicety here, it is the only route in. Signing an agent
 * in is something only a person can do, and this frame is where they do it.
 *
 * There is no sleep button. A browser goes to standby seconds after the last
 * action, which keeps its state and stops the bill, and comes back the moment
 * anything drives it. Nothing about that is the operator's decision, so
 * offering it would be a switch that does nothing. Closing is offered, because
 * closing is what writes the cookies back to the profile: it is how a sign-in
 * just performed is made durable now rather than in an hour.
 */
export function BrowserScreen({ agent }: Props) {
  const settings = useStore((s) => s.settings);
  const [browser, setBrowser] = useState<Browser | null>(null);
  const [full, setFull] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [checked, setChecked] = useState(false);
  const [confirming, setConfirming] = useState(false);
  // Which agent the panel is currently about. A lookup started for one agent
  // landing after the operator switched would paint the previous agent's
  // browser into the new panel.
  const showing = useRef(agent.id);
  const frame = useRef<HTMLIFrameElement>(null);
  const stage = useRef<HTMLDivElement>(null);
  // Where the picture was before it grew. Measured in the click, because by the
  // time anything can react to the change the stage is already in its new place.
  const cameFrom = useRef<DOMRect | null>(null);

  // Nothing at all until there is a key. Offering to give an agent a browser
  // that cannot be made is worse than not mentioning browsers, and asked from
  // the settings rather than inferred from a failure: a message that has to be
  // matched on to be understood is one a reworded error breaks.
  const configured = settings?.kernelKeySet === true;
  // Whether this agent is one of the agents allowed the web. Read from the
  // card, never from whether a browser came back: every browser is deleted
  // minutes after it is used, and an agent whose browser timed out has not had
  // anything taken away from it.
  const given = agent.hasBrowser;

  const look = useCallback(async () => {
    const asked = agent.id;
    try {
      const found = await api.agentBrowser(asked);
      if (showing.current !== asked) return;
      setBrowser(found);
      setError(null);
    } catch (caught) {
      if (showing.current !== asked) return;
      setError(errorMessage(caught));
    } finally {
      if (showing.current === asked) setChecked(true);
    }
  }, [agent.id]);

  useEffect(() => {
    showing.current = agent.id;
    setBrowser(null);
    setChecked(false);
    setFull(false);
    setBusy(false);
    setError(null);
    setConfirming(false);
    if (configured && given) void look();
    else setChecked(true);
  }, [agent.id, look, configured, given]);

  // A browser that has timed out leaves a live view URL that is no longer
  // valid, and an iframe pointed at one is a blank rectangle rather than an
  // error. Polling is what turns that back into an offer to open another.
  useEffect(() => {
    if (!configured || !given) return;
    const timer = setInterval(() => void look(), 20000);
    return () => clearInterval(timer);
  }, [configured, given, look]);

  /**
   * Hands the keyboard to the live view.
   *
   * A cross-origin iframe receives key events only while it holds focus, and
   * nothing gives it focus on its own: the operator clicked the veil, which is
   * an element in this document, so the keyboard stayed here. The mouse worked
   * throughout, which is what made this look like a broken keyboard rather than
   * a focus problem.
   *
   * Called from inside the click handler rather than from an effect afterwards,
   * because this webview is WebKit and WebKit only honours a focus change that
   * is part of a user gesture. An effect running on the next render is not.
   *
   * Wrapped so its identity is stable: `grow` below depends on it, and a fresh
   * function every render would rebuild that callback for nothing. A ref does
   * not change, so there is nothing to list.
   */
  const grabKeyboard = useCallback(() => frame.current?.focus(), []);

  /**
   * Grows the pane to fill the window, and takes the keyboard with it.
   *
   * The same two steps as the computer's, and the same shape deliberately: the
   * two panes sit one above the other, and one animating while the other
   * snapped would read as a bug in whichever moved second.
   */
  const grow = useCallback(() => {
    cameFrom.current = stage.current?.getBoundingClientRect() ?? null;
    setFull(true);
    grabKeyboard();
  }, [grabKeyboard]);

  // Escape shrinks it again. On the window rather than the frame, because the
  // live view swallows key presses once it has focus.
  useEffect(() => {
    if (!full) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setFull(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [full]);

  // FLIP: the stage is already covering the window, so it is put back over the
  // small picture it came from and let go. The live view under it never
  // reloads, and a change of size that lands in a single frame reads as a
  // reconnect that did not happen. Only on the way up, for the reason given in
  // `ComputerScreen`.
  useLayoutEffect(() => {
    const node = stage.current;
    const before = cameFrom.current;
    cameFrom.current = null;
    if (!node) return;

    node.dataset.zooming = "false";
    node.style.transition = "none";
    node.style.transform = "";
    if (!before || prefersReducedMotion()) return;

    const after = node.getBoundingClientRect();
    if (!before.width || !before.height || !after.width || !after.height) return;

    node.style.transform =
      `translate(${before.left - after.left}px, ${before.top - after.top}px) ` +
      `scale(${before.width / after.width}, ${before.height / after.height})`;
    requestAnimationFrame(() => {
      node.dataset.zooming = "true";
      node.style.transition = "";
      node.style.transform = "";
    });
  }, [full]);

  /**
   * Giving a browser, and taking it back.
   *
   * Neither answers with a browser, for the reason `ComputerScreen.decide`
   * gives: what changes is the card, and the roster refresh that follows is
   * what sends the effect above to look again.
   */
  const decide = async (run: () => Promise<unknown>) => {
    const asked = agent.id;
    setBusy(true);
    setError(null);
    try {
      await run();
    } catch (caught) {
      if (showing.current === asked) setError(errorMessage(caught));
    } finally {
      if (showing.current === asked) setBusy(false);
    }
  };

  const act = async (run: () => Promise<Browser | null>) => {
    const asked = agent.id;
    setBusy(true);
    setError(null);
    try {
      const next = await run();
      if (showing.current !== asked) return;
      setBrowser(next);
      setConfirming(false);
      setFull(false);
    } catch (caught) {
      if (showing.current === asked) setError(errorMessage(caught));
    } finally {
      if (showing.current === asked) setBusy(false);
    }
  };

  if (!configured || !checked) return null;

  const live = given && browser?.state === "running" && browser?.liveViewUrl;

  const asDialog = full
    ? { role: "dialog", "aria-modal": true, "aria-label": `${agent.name}'s browser` }
    : {};

  return (
    // Two elements, one connection. The outer one stays in the panel and holds
    // the space the pane had; the inner one is what covers the window. The
    // frame inside that is the same element in both sizes, which is what keeps
    // the live view connected across the change.
    <div className="screen" data-full={full ? "true" : undefined}>
      <div className="screen__stage" ref={stage} {...asDialog}>
        {full && (
          <div className="screen__bar">
            <span className="screen__title">{agent.name}'s browser</span>
            <span className="screen__state" data-state={browser?.state}>
              {browser?.state}
            </span>
            <span style={{ flex: 1 }} />

            {confirming ? (
              <>
                <button
                  type="button"
                  className="btn btn--small btn--danger"
                  disabled={busy}
                  onClick={() =>
                    void act(async () => {
                      await api.stopAgentBrowser(agent.id);
                      return null;
                    })
                  }
                >
                  Close it and save the sign-ins
                </button>
                <button
                  type="button"
                  className="btn btn--small btn--ghost"
                  onClick={() => {
                    setConfirming(false);
                    grabKeyboard();
                  }}
                >
                  Keep it open
                </button>
              </>
            ) : (
              <>
                <button
                  type="button"
                  className="btn btn--small btn--ghost"
                  disabled={busy}
                  onClick={() => setConfirming(true)}
                  title="Close it. What it is signed in to is saved, and the next one opens signed in."
                >
                  Close
                </button>
                <button
                  type="button"
                  className="btn btn--small btn--ghost"
                  disabled={busy}
                  onClick={() => void decide(() => api.takeAgentBrowser(agent.id))}
                  title="Take it back. It closes, and what it is signed in to is saved."
                >
                  Take it back
                </button>
              </>
            )}

            <div className="screen__actions">
              <button type="button" className="btn btn--small" onClick={() => setFull(false)}>
                Done
              </button>
            </div>
          </div>
        )}

        {live ? (
          <div className="screen__frame">
            <iframe
              // Keyed on the session alone, never on the size, so growing to fill
              // the window keeps the same connection. Clipboard is allowed
              // because signing in means pasting a password out of a manager, and
              // without it the paste silently does nothing.
              key={browser.sessionId}
              ref={frame}
              title={`${agent.name}'s browser`}
              src={browser.liveViewUrl ?? ""}
              allow="autoplay; clipboard-read; clipboard-write"
            />
            {!full && (
              <button
                type="button"
                className="screen__veil"
                onClick={grow}
                title={`Open ${agent.name}'s browser and take over`}
                aria-label={`Open ${agent.name}'s browser and take over`}
              />
            )}
          </div>
        ) : (
          <div className="screen__frame screen__frame--empty">
            <p className="screen__note">
              {error ??
                (busy
                  ? "Working on it. This takes a moment."
                  : !given
                    ? `${agent.name} has no browser, so it cannot open a page or read one. Give it
                       one and it opens a browser the first time it uses the web.`
                    : browser?.unwatchable
                      ? `${agent.name}'s browser is open and working, and this build cannot show
                         it: Kernel is serving the live view from ${browser.unwatchable}, which
                         this window is not allowed to frame. The agent can still use the web.
                         Update Guaca, or allow that address in the window's CSP.`
                      : browser
                        ? `Closed. What it was signed in to is saved, so the next one opens signed
                           in to the same accounts.`
                        : `${agent.name} has a browser and none open. It opens one the first time
                           it uses the web. Open it yourself to sign this agent in to something:
                           that is the one thing an agent cannot do for itself.`)}
            </p>
            <div className="screen__offer">
              {given ? (
                <>
                  <button
                    type="button"
                    className="btn btn--small btn--primary"
                    disabled={busy}
                    onClick={() => void act(() => api.startAgentBrowser(agent.id))}
                  >
                    {busy ? "Working…" : browser ? "Open another" : "Open one"}
                  </button>
                  <button
                    type="button"
                    className="btn btn--small btn--ghost"
                    disabled={busy}
                    onClick={() => void decide(() => api.takeAgentBrowser(agent.id))}
                    title="Take it back. Any open browser closes, and its sign-ins are saved."
                  >
                    Take it back
                  </button>
                </>
              ) : (
                <button
                  type="button"
                  className="btn btn--small btn--primary"
                  disabled={busy}
                  onClick={() => void decide(() => api.giveAgentBrowser(agent.id))}
                >
                  {busy ? "Working…" : "Give one"}
                </button>
              )}
            </div>
          </div>
        )}
      </div>

      {!full && (
        <p className="screen__caption">
          <span>{agent.name}'s browser</span>
          {browser && (
            <span className="screen__state" data-state={browser.state}>
              {browser.state}
            </span>
          )}
        </p>
      )}
    </div>
  );
}
