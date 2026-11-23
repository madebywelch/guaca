import { useCallback, useEffect, useLayoutEffect, useReducer, useRef, useState } from "react";

import { useDismiss } from "../lib/dismiss";
import { api } from "../lib/ipc";
import { prefersReducedMotion } from "../lib/motion";
import { useStore } from "../lib/store";
import { type AgentCard, type Browser, errorMessage, type LocalBrowser } from "../lib/types";
import { Grant } from "./Grant";

interface Props {
  agent: AgentCard;
}

type BrowserRead = { status: "loading" } | { status: "ready"; browser: Browser | null };
type BrowserReadAction =
  | { type: "reset" }
  | { type: "found"; browser: Browser | null }
  | { type: "failed" };

export function browserReadReducer(state: BrowserRead, action: BrowserReadAction): BrowserRead {
  switch (action.type) {
    case "reset":
      return { status: "loading" };
    case "found":
      return { status: "ready", browser: action.browser };
    case "failed":
      return state.status === "ready" ? state : { status: "ready", browser: null };
  }
}

const SETTING_UP_EVERY_MS = 5000;
const RUNNING_EVERY_MS = 20000;

export function BrowserScreen({ agent }: Props) {
  const settings = useStore((s) => s.settings);
  const localBrowser = useStore((s) => s.capabilities.localBrowser === true);
  const [setup, setSetup] = useState<LocalBrowser | null>(null);
  const [readState, dispatchRead] = useReducer(browserReadReducer, { status: "loading" });
  const browser = readState.status === "ready" ? readState.browser : null;
  const [full, setFull] = useState(false);
  const [busy, setBusy] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const visit = useRef(0);
  const reads = useRef(0);
  const frame = useRef<HTMLIFrameElement>(null);
  const veil = useRef<HTMLButtonElement>(null);
  const returnFocus = useRef(false);
  const stage = useRef<HTMLDivElement>(null);
  const cameFrom = useRef<DOMRect | null>(null);

  const hosted = settings?.kernelKeySet === true;
  const local = !hosted && localBrowser;
  const configured = hosted || local;
  const given = agent.hasBrowser;
  const settingUp = local && setup === "settingUp";

  const look = useCallback(async () => {
    const asked = agent.id;
    const currentVisit = visit.current;
    const read = ++reads.current;
    const current = () => visit.current === currentVisit && reads.current === read;
    try {
      if (local) {
        const state = await api.localBrowser();
        if (!current()) return;
        setSetup(state);
        if (state !== "ready") {
          dispatchRead({ type: "found", browser: null });
          setError(null);
          return;
        }
      }
      const found = await api.agentBrowser(asked);
      if (!current()) return;
      dispatchRead({ type: "found", browser: found });
      setError(null);
    } catch (caught) {
      if (!current()) return;
      dispatchRead({ type: "failed" });
      setError(errorMessage(caught));
    }
  }, [agent.id, local]);

  useEffect(() => {
    visit.current += 1;
    dispatchRead({ type: "reset" });
    setFull(false);
    setBusy(false);
    setExpanded(false);
    setError(null);
    setConfirming(false);
    if (configured && given) void look();
    else dispatchRead({ type: "found", browser: null });
    return () => {
      visit.current += 1;
    };
  }, [agent.id, look, configured, given]);

  useEffect(() => {
    if (!configured || !given) return;
    const timer = setInterval(
      () => void look(),
      settingUp ? SETTING_UP_EVERY_MS : RUNNING_EVERY_MS,
    );
    return () => clearInterval(timer);
  }, [configured, given, look, settingUp]);

  const grabKeyboard = useCallback(() => frame.current?.focus(), []);

  const grow = useCallback(() => {
    cameFrom.current = stage.current?.getBoundingClientRect() ?? null;
    returnFocus.current = true;
    setFull(true);
    grabKeyboard();
  }, [grabKeyboard]);

  useDismiss(() => setFull(false), full);

  useEffect(() => {
    if (!full && returnFocus.current) {
      returnFocus.current = false;
      veil.current?.focus();
    }
  }, [full]);

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

  const decide = async (run: () => Promise<unknown>) => {
    const currentVisit = visit.current;
    setBusy(true);
    setError(null);
    try {
      await run();
    } catch (caught) {
      if (visit.current === currentVisit) setError(errorMessage(caught));
    } finally {
      if (visit.current === currentVisit) setBusy(false);
    }
  };

  const act = async (run: () => Promise<Browser | null>) => {
    const currentVisit = visit.current;
    setBusy(true);
    setError(null);
    try {
      const next = await run();
      if (visit.current !== currentVisit) return;
      reads.current += 1;
      dispatchRead({ type: "found", browser: next });
      setConfirming(false);
      setFull(false);
    } catch (caught) {
      if (visit.current === currentVisit) setError(errorMessage(caught));
    } finally {
      if (visit.current === currentVisit) setBusy(false);
    }
  };

  if (!configured) return null;

  const live = given && browser?.state === "running" && browser?.liveViewUrl;

  const asDialog = full
    ? { role: "dialog", "aria-modal": true, "aria-label": `${agent.name}'s browser` }
    : {};

  return (
    <section className="detail place" data-given={given} aria-label={`${agent.name}'s browser`}>
      <Grant
        name="Browser"
        expanded={expanded}
        onExpand={() => setExpanded((open) => !open)}
        given={given}
        busy={busy}
        state={given ? (settingUp ? "setting up" : browser?.state) : undefined}
        about={
          local
            ? given
              ? "Take it back. Its page closes; nothing in it was kept."
              : `Lets ${agent.name} open and read public web pages in a browser on this Mac.`
            : given
              ? "Take it back. An open browser closes, and its sign-ins are saved."
              : `Lets ${agent.name} open and read web pages in a hosted browser.`
        }
        onChange={(next) =>
          void decide(() =>
            next ? api.giveAgentBrowser(agent.id) : api.takeAgentBrowser(agent.id),
          )
        }
      />

      {given && local && readState.status === "ready" && (
        <div className="screen">
          <div className="screen__frame screen__frame--empty">
            <p className="screen__note" role="status">
              {error ??
                (settingUp
                  ? `Setting up the browser on this Mac. The first time downloads it, which takes a minute or two; until it is ready, ${agent.name} is told to try again shortly.`
                  : `Public web only, and no live view. ${agent.name} reads pages in a browser on this Mac that is signed in to nothing and cannot reach your network. A Kernel key in Settings gives it a hosted browser you can watch and sign in to.`)}
            </p>
          </div>
        </div>
      )}

      {given && !local && readState.status === "ready" && (live || expanded || full) && (
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
                      className="btn btn--compact btn--danger"
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
                      className="btn btn--compact btn--quiet"
                      onClick={() => {
                        setConfirming(false);
                        grabKeyboard();
                      }}
                    >
                      Keep it open
                    </button>
                  </>
                ) : (
                  <button
                    type="button"
                    className="btn btn--compact btn--quiet"
                    disabled={busy}
                    onClick={() => setConfirming(true)}
                    title="Close it. What it is signed in to is saved, and the next one opens signed in."
                  >
                    Close
                  </button>
                )}

                <div className="screen__actions">
                  <button type="button" className="btn btn--compact" onClick={() => setFull(false)}>
                    Done
                  </button>
                </div>
              </div>
            )}

            {live ? (
              <div className="screen__frame">
                <iframe
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
                    ref={veil}
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
                      : browser?.unwatchable
                        ? `Open and working, but this window cannot show a live view from
                           ${browser.unwatchable}. ${agent.name} can still use the web. Update
                           Guaca, or allow that address in the window's CSP.`
                        : browser
                          ? "Closed. Its sign-ins are saved."
                          : "Opens when it needs the web. Open it yourself to sign this agent in to a site.")}
                </p>
                <button
                  type="button"
                  className="btn btn--compact btn--primary"
                  disabled={busy}
                  onClick={() => void act(() => api.startAgentBrowser(agent.id))}
                >
                  {busy ? "Working…" : "Open"}
                </button>
              </div>
            )}
          </div>
        </div>
      )}

      {!full && given && expanded && (
        <label className="field field--row">
          <input
            type="checkbox"
            checked={agent.browserConsent === "askBeforeActing"}
            disabled={busy}
            onChange={(event) =>
              void decide(() =>
                api.setAgentBrowserConsent(
                  agent.id,
                  event.target.checked ? "askBeforeActing" : "open",
                ),
              )
            }
          />
          <span>
            <span className="field__label">Ask me before it acts in my name</span>
            <span className="field__hint">
              Clicks and typing on sites it is signed in to wait for your approval.
            </span>
          </span>
        </label>
      )}

      {!given && error && (
        <p className="field__error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
