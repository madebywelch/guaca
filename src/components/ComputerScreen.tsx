import { useCallback, useEffect, useLayoutEffect, useReducer, useRef, useState } from "react";

import { useDismiss } from "../lib/dismiss";
import { screenUrl } from "../lib/files";
import { api } from "../lib/ipc";
import { prefersReducedMotion } from "../lib/motion";
import { useStore } from "../lib/store";
import { hosted } from "../lib/transport";
import { type AgentCard, type Computer, errorMessage } from "../lib/types";
import { Grant } from "./Grant";

interface Props {
  agent: AgentCard;
}

type ReadState = { kind: "checking" } | { kind: "checked"; computer: Computer | null };
type ReadEvent =
  | { type: "reset" }
  | { type: "resolved"; computer: Computer | null }
  | { type: "failed" };

export function computerRead(state: ReadState, event: ReadEvent): ReadState {
  switch (event.type) {
    case "reset":
      return { kind: "checking" };
    case "resolved":
      return { kind: "checked", computer: event.computer };
    case "failed":
      return state.kind === "checked" ? state : { kind: "checked", computer: null };
  }
}

export function ComputerScreen({ agent }: Props) {
  const settings = useStore((s) => s.settings);
  const [read, dispatchRead] = useReducer(computerRead, { kind: "checking" });
  const computer = read.kind === "checked" ? read.computer : null;
  const checked = read.kind === "checked";
  const [full, setFull] = useState(false);
  const [busy, setBusy] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<"sleep" | "destroy" | null>(null);
  const showing = useRef(agent.id);
  const generation = useRef(0);
  const requests = useRef(0);

  const stage = useRef<HTMLDivElement>(null);
  const frame = useRef<HTMLIFrameElement>(null);
  const veil = useRef<HTMLButtonElement>(null);
  const returnFocus = useRef(false);
  const cameFrom = useRef<DOMRect | null>(null);

  const grow = useCallback(() => {
    cameFrom.current = stage.current?.getBoundingClientRect() ?? null;
    returnFocus.current = true;
    setFull(true);
    frame.current?.focus();
  }, []);

  const configured = settings?.e2bKeySet === true;
  const given = agent.hasComputer;

  const look = useCallback(async () => {
    const asked = agent.id;
    const epoch = generation.current;
    const request = ++requests.current;
    try {
      const found = await api.agentComputer(asked);
      if (generation.current !== epoch || requests.current !== request) return;
      dispatchRead({ type: "resolved", computer: found });
      setError(null);
    } catch (caught) {
      if (generation.current !== epoch || requests.current !== request) return;
      setError(errorMessage(caught));
      dispatchRead({ type: "failed" });
    }
  }, [agent.id]);

  useEffect(() => {
    showing.current = agent.id;
    generation.current++;
    dispatchRead({ type: "reset" });
    setFull(false);
    setBusy(false);
    setExpanded(false);
    setError(null);
    setConfirming(null);
    if (configured && given) void look();
    else dispatchRead({ type: "resolved", computer: null });
    return () => {
      generation.current++;
    };
  }, [agent.id, look, configured, given]);

  useEffect(() => {
    if (!configured || !given) return;
    const timer = setInterval(() => void look(), 15000);
    return () => clearInterval(timer);
  }, [configured, given, look]);

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
    const asked = agent.id;
    const epoch = generation.current;
    setBusy(true);
    setError(null);
    try {
      await run();
    } catch (caught) {
      if (showing.current === asked && generation.current === epoch) setError(errorMessage(caught));
    } finally {
      if (showing.current === asked && generation.current === epoch) setBusy(false);
    }
  };

  const act = async (run: () => Promise<Computer | null>) => {
    const asked = agent.id;
    const epoch = generation.current;
    setBusy(true);
    setError(null);
    try {
      const next = await run();
      if (showing.current !== asked || generation.current !== epoch) return;
      requests.current++;
      dispatchRead({ type: "resolved", computer: next });
      setConfirming(null);
    } catch (caught) {
      if (showing.current === asked && generation.current === epoch) setError(errorMessage(caught));
    } finally {
      if (showing.current === asked && generation.current === epoch) setBusy(false);
    }
  };

  if (!configured) return null;

  const running = given && computer?.state === "running";
  const asleep = given && computer?.state === "asleep";
  const live = running && computer?.vncUrl;

  const asDialog = full
    ? { role: "dialog", "aria-modal": true, "aria-label": `${agent.name}'s computer` }
    : {};

  return (
    <section className="detail place" data-given={given} aria-label={`${agent.name}'s computer`}>
      <Grant
        name="Computer"
        expanded={expanded}
        onExpand={() => setExpanded((open) => !open)}
        given={given}
        busy={busy}
        state={given ? computer?.state : undefined}
        about={
          given
            ? "Take it back. The machine sleeps, and its disk is kept."
            : `Lets ${agent.name} use a desktop in a cloud sandbox, started when it first needs one.`
        }
        onChange={(next) =>
          void decide(() =>
            next ? api.giveAgentComputer(agent.id) : api.takeAgentComputer(agent.id),
          )
        }
      />

      {given && checked && (live || expanded || full) && (
        <div className="screen" data-full={full ? "true" : undefined}>
          <div className="screen__stage" ref={stage} {...asDialog}>
            {full && (
              <div className="screen__bar">
                <span className="screen__title">{agent.name}'s computer</span>
                <span className="screen__state" data-state={computer?.state}>
                  {computer?.state}
                </span>
                <span style={{ flex: 1 }} />

                {confirming === "sleep" ? (
                  <>
                    <button
                      type="button"
                      className="btn btn--compact btn--danger"
                      disabled={busy}
                      onClick={() => void act(() => api.stopAgentComputer(agent.id))}
                    >
                      Sleep it
                    </button>
                    <button
                      type="button"
                      className="btn btn--compact btn--quiet"
                      onClick={() => setConfirming(null)}
                    >
                      Keep awake
                    </button>
                  </>
                ) : confirming === "destroy" ? (
                  <>
                    <button
                      type="button"
                      className="btn btn--compact btn--danger"
                      disabled={busy}
                      onClick={() =>
                        void act(async () => {
                          await api.deleteAgentComputer(agent.id);
                          return null;
                        })
                      }
                    >
                      Destroy it and its disk
                    </button>
                    <button
                      type="button"
                      className="btn btn--compact btn--quiet"
                      onClick={() => setConfirming(null)}
                    >
                      Keep
                    </button>
                  </>
                ) : (
                  <>
                    <button
                      type="button"
                      className="btn btn--compact btn--quiet"
                      disabled={busy}
                      onClick={() => setConfirming("sleep")}
                      title="Sleep. The disk is kept, so it wakes signed in."
                    >
                      Sleep
                    </button>
                    <button
                      type="button"
                      className="btn btn--compact btn--quiet"
                      disabled={busy}
                      onClick={() => setConfirming("destroy")}
                    >
                      Destroy
                    </button>
                  </>
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
                  key={computer.sandboxId}
                  ref={frame}
                  sandbox={hosted ? "allow-scripts" : undefined}
                  referrerPolicy="no-referrer"
                  title={`${agent.name}'s computer`}
                  src={computer.vncUrl ? screenUrl(computer.vncUrl) : ""}
                />
                {!full && (
                  <button
                    type="button"
                    className="screen__veil"
                    ref={veil}
                    onClick={grow}
                    title={`Open ${agent.name}'s screen and take over`}
                    aria-label={`Open ${agent.name}'s screen and take over`}
                  />
                )}
              </div>
            ) : (
              <div className="screen__frame screen__frame--empty">
                <p className="screen__note">
                  {error ??
                    (busy
                      ? "Working on it. This takes a few seconds."
                      : asleep
                        ? "Asleep. Its disk is kept, so it wakes where it left off."
                        : running
                          ? "Running. The desktop is not up yet."
                          : "Starts when it needs one. Start it yourself to sign this agent in to something.")}
                </p>
                <button
                  type="button"
                  className="btn btn--compact btn--primary"
                  disabled={busy}
                  onClick={() => void act(() => api.startAgentComputer(agent.id))}
                >
                  {busy ? "Working…" : asleep ? "Wake" : "Start"}
                </button>
              </div>
            )}
          </div>
        </div>
      )}

      {!given && error && (
        <p className="field__error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
