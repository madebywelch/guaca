import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import { prefersReducedMotion } from "../lib/motion";
import { useStore } from "../lib/store";
import { type AgentCard, type Computer, errorMessage } from "../lib/types";

interface Props {
  agent: AgentCard;
}

/**
 * An agent's computer, at the top of its panel.
 *
 * Two sizes, one connection. In the panel it is a live but read-only picture
 * behind a transparent veil, so a stray click cannot land in the agent's
 * desktop and the thing stays something you glance at while reading. Full
 * screen it accepts input and the operator can take over.
 *
 * Growing is a CSS change and nothing more: the frame keeps its place in the
 * tree and the stage around it is promoted to fill the window, so opening and
 * closing it never drops the desktop and reconnects to it.
 *
 * Two things make that change something to watch rather than something to
 * flinch at. The stage is not the element holding the screen's place in the
 * panel, so the panel does not reflow around the gap it leaves; and the stage
 * is played out of the picture it grew from rather than appearing at full size
 * in one frame, which is what made a change of size read as a reconnect.
 *
 * There is deliberately no terminal here. A shell is how the agent works, not
 * how an operator watches it, and a second way in only invited the two to
 * disagree about what was on the machine.
 */
export function ComputerScreen({ agent }: Props) {
  const settings = useStore((s) => s.settings);
  const [computer, setComputer] = useState<Computer | null>(null);
  const [full, setFull] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [checked, setChecked] = useState(false);
  // Sleep sits beside Destroy and Destroy throws work away, so neither happens
  // on one click. Held here rather than in each button so switching agent or
  // finishing an action always clears a half-pressed one.
  const [confirming, setConfirming] = useState<"sleep" | "destroy" | null>(null);
  // Which agent the panel is currently about. A lookup started for one agent
  // used to land after the operator had switched to another and paint that
  // agent's machine into the new panel, so an agent with no computer showed
  // the previous one's screen.
  const showing = useRef(agent.id);

  const stage = useRef<HTMLDivElement>(null);
  // Where the picture was before it grew. Measured in the click, because by the
  // time anything can react to the change the stage is already in its new place.
  const cameFrom = useRef<DOMRect | null>(null);

  const grow = useCallback(() => {
    cameFrom.current = stage.current?.getBoundingClientRect() ?? null;
    setFull(true);
  }, []);

  // Nothing at all until there is a key. Offering to give an agent a computer
  // that cannot be made is worse than not mentioning computers.
  const configured = settings?.e2bKeySet === true;

  const look = useCallback(async () => {
    const asked = agent.id;
    try {
      const found = await api.agentComputer(asked);
      if (showing.current !== asked) return;
      setComputer(found);
      setError(null);
    } catch (caught) {
      if (showing.current !== asked) return;
      // A missing key is not a failure worth a red banner: it means the feature
      // was never set up, and the panel says so instead.
      setError(errorMessage(caught));
    } finally {
      if (showing.current === asked) setChecked(true);
    }
  }, [agent.id]);

  useEffect(() => {
    showing.current = agent.id;
    setComputer(null);
    setChecked(false);
    setFull(false);
    // Reset too. A call still in flight when the operator switched agents left
    // this true for the new panel, which disabled its only button permanently.
    setBusy(false);
    setError(null);
    setConfirming(null);
    if (configured) void look();
  }, [agent.id, look, configured]);

  // Sandboxes expire on their own, so a panel left open goes stale: it kept
  // showing a desktop that had been reclaimed, and clicking it did nothing.
  useEffect(() => {
    if (!configured) return;
    const timer = setInterval(() => void look(), 15000);
    return () => clearInterval(timer);
  }, [configured, look]);

  // Escape shrinks it again. On the window rather than on the frame, because
  // the desktop swallows key presses the moment it has focus, and the operator
  // should not have to find somewhere else to click first.
  useEffect(() => {
    if (!full) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setFull(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [full]);

  // FLIP: the stage is already covering the window, so it is put back over the
  // small picture it came from and let go. The desktop under it never reloads,
  // and a change of size that lands in a single frame reads as a reconnect that
  // did not happen.
  //
  // Only on the way up. Coming down, the stage is back inside the panel's
  // scroller, which clips anything still scaled to the size of the window, and
  // holding it out of the flow until a transition ended would risk leaving it
  // there.
  useLayoutEffect(() => {
    const node = stage.current;
    const before = cameFrom.current;
    cameFrom.current = null;
    if (!node) return;

    // Anything still playing belongs to the size the stage has just left, so it
    // stops here rather than finishing in the new one.
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

  const act = async (run: () => Promise<Computer | null>) => {
    const asked = agent.id;
    setBusy(true);
    setError(null);
    try {
      const next = await run();
      if (showing.current !== asked) return;
      setComputer(next);
      setConfirming(null);
    } catch (caught) {
      if (showing.current === asked) setError(errorMessage(caught));
    } finally {
      if (showing.current === asked) setBusy(false);
    }
  };

  if (!configured || !checked) return null;

  const running = computer?.state === "running";
  const asleep = computer?.state === "asleep";
  const live = running && computer?.vncUrl;

  // Announced as a dialog only while it covers the window; the rest of the
  // time it is one section of the panel. Grouped so the role and the two
  // properties that mean nothing without it cannot come apart.
  const asDialog = full
    ? { role: "dialog", "aria-modal": true, "aria-label": `${agent.name}'s computer` }
    : {};

  return (
    // Two elements, one connection. The outer one stays in the panel and holds
    // the space the screen had; the inner one is what covers the window. The
    // frame inside that is the same element in both sizes, which is what keeps
    // the desktop connected across the change.
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
                  className="btn btn--small btn--danger"
                  disabled={busy}
                  onClick={() => void act(() => api.stopAgentComputer(agent.id))}
                >
                  Sleep it
                </button>
                <button
                  type="button"
                  className="btn btn--small btn--ghost"
                  onClick={() => setConfirming(null)}
                >
                  Keep awake
                </button>
              </>
            ) : confirming === "destroy" ? (
              <>
                <button
                  type="button"
                  className="btn btn--small btn--danger"
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
                  className="btn btn--small btn--ghost"
                  onClick={() => setConfirming(null)}
                >
                  Keep
                </button>
              </>
            ) : (
              <>
                <button
                  type="button"
                  className="btn btn--small btn--ghost"
                  disabled={busy}
                  onClick={() => setConfirming("sleep")}
                  title="Sleep. The disk is kept, so it wakes signed in."
                >
                  Sleep
                </button>
                <button
                  type="button"
                  className="btn btn--small btn--ghost"
                  disabled={busy}
                  onClick={() => setConfirming("destroy")}
                >
                  Destroy
                </button>
              </>
            )}

            <button type="button" className="btn btn--small" onClick={() => setFull(false)}>
              Close
            </button>
          </div>
        )}

        {live ? (
          <div className="screen__frame">
            <iframe
              // Keyed on the machine alone, never on the size, so growing to
              // fill the window keeps the same connection.
              //
              // Which is also why noVNC's own `view_only` is not used: it is
              // read once when the connection opens, so switching it would mean
              // reconnecting. The veil below does that job instead, without
              // touching the connection.
              key={computer.sandboxId}
              title={`${agent.name}'s computer`}
              src={computer.vncUrl ?? ""}
            />
            {!full && (
              // Swallows clicks aimed at the desktop while it is only meant to
              // be watched, which is what makes noVNC's own read-only mode
              // unnecessary and the connection worth keeping.
              <button
                type="button"
                className="screen__veil"
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
                    ? `Asleep. Its disk is kept, so it wakes up where it left off, still signed into
                       anything it was signed into. It sleeps again after
                       ${settings?.computerIdleMinutes ?? 15} idle minutes.`
                    : running
                      ? "Running, but the desktop is not up yet."
                      : "No computer yet. Agents get one the first time they use it.")}
            </p>
            <button
              type="button"
              className="btn btn--small btn--primary"
              disabled={busy}
              onClick={() => void act(() => api.startAgentComputer(agent.id))}
            >
              {busy ? "Working…" : asleep ? "Wake" : running ? "Start the desktop" : "Give one"}
            </button>
          </div>
        )}
      </div>

      {/* Left in place while the stage covers the window: it is out of sight
          behind it, and it is part of the space the panel is holding open. */}
      <p className="screen__caption">
        <span>{agent.name}'s screen</span>
        {computer && (
          <span className="screen__state" data-state={computer.state}>
            {computer.state}
          </span>
        )}
      </p>
    </div>
  );
}
