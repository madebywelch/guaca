import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { type AgentCard, type AgentId, type Computer, errorMessage } from "../lib/types";

interface Props {
  agent: AgentCard;
}

/**
 * Whether the operator has stowed this agent's pane before.
 *
 * Kept per agent and across restarts, because it is a statement about that
 * agent rather than about this session: an agent that is never going to want a
 * computer is never going to want one tomorrow either.
 */
function remembered(id: AgentId): "stowed" | "shown" | null {
  try {
    const held = localStorage.getItem(`guac.computer.${id}`);
    return held === "stowed" || held === "shown" ? held : null;
  } catch {
    // Private browsing modes and hardened webviews can refuse storage. A
    // forgotten preference is a much smaller problem than a blank channel.
    return null;
  }
}

function remember(id: AgentId, choice: "stowed" | "shown") {
  try {
    localStorage.setItem(`guac.computer.${id}`, choice);
  } catch {
    // As above: not worth telling the operator about.
  }
}

/**
 * An agent's computer, in the corner of its channel.
 *
 * One view, three sizes. Stowed it is a chip and nothing else, which is what
 * an agent that is never given a computer is worth; as a preview it is a live
 * but read-only picture behind a transparent veil, so a stray click cannot
 * land in the agent's desktop; expanded it accepts input and the operator can
 * take over.
 *
 * It starts stowed unless the agent has a machine, and remembers being stowed,
 * because an agent that will never want a computer will not want one tomorrow
 * either and should not hold the corner of its transcript in the meantime.
 *
 * There is deliberately no terminal here. A shell is how the agent works, not
 * how an operator watches it, and a second way in only invited the two to
 * disagree about what was on the machine.
 */
export function ComputerPane({ agent }: Props) {
  const settings = useStore((s) => s.settings);
  const [computer, setComputer] = useState<Computer | null>(null);
  const [open, setOpen] = useState(false);
  // The operator's explicit choice, if they have made one. Null follows the
  // agent: a machine is worth a preview, and an agent that will never have one
  // should not spend the corner of the transcript saying so.
  const [chosen, setChosen] = useState<"stowed" | "shown" | null>(() => remembered(agent.id));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [checked, setChecked] = useState(false);
  // Sleep sits beside Expand and Destroy throws work away, so neither happens
  // on one click. Held here rather than in each button so switching agent or
  // finishing an action always clears a half-pressed one.
  const [confirming, setConfirming] = useState<"sleep" | "destroy" | null>(null);
  const paneRef = useRef<HTMLDivElement>(null);
  // Which agent the pane is currently about. A lookup started for one agent
  // used to land after the operator had switched to another and paint that
  // agent's machine into the new pane, so an agent with no computer showed the
  // previous one's screen.
  const showing = useRef(agent.id);

  // Nothing at all until there is a key. Offering to give an agent a computer
  // that cannot be made is worse than not mentioning computers.
  const configured = settings?.e2bKeySet === true;

  // The pane floats over the transcript, and a wheel over it was scrolling the
  // conversation behind it: the desktop does not scroll, so the browser passes
  // the gesture up to the nearest thing that does. Attached by hand rather than
  // through onWheel because React's is passive, and a passive listener cannot
  // refuse the scroll.
  useEffect(() => {
    const node = paneRef.current;
    if (!node) return;
    const swallow = (event: WheelEvent) => event.preventDefault();
    node.addEventListener("wheel", swallow, { passive: false });
    return () => node.removeEventListener("wheel", swallow);
  }, [checked]);

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
      // was never set up, and the pane says so instead.
      setError(errorMessage(caught));
    } finally {
      if (showing.current === asked) setChecked(true);
    }
  }, [agent.id]);

  useEffect(() => {
    showing.current = agent.id;
    setComputer(null);
    setChecked(false);
    setOpen(false);
    // Reset too. A call still in flight when the operator switched agents left
    // this true for the new pane, which disabled its only button permanently
    // and made the terminal swallow every command silently.
    setBusy(false);
    setError(null);
    setConfirming(null);
    setChosen(remembered(agent.id));
    if (configured) void look();
  }, [agent.id, look, configured]);

  // Sandboxes expire on their own, so a pane left open goes stale: it kept
  // showing a desktop that had been reclaimed, and clicking it did nothing.
  useEffect(() => {
    if (!configured) return;
    const timer = setInterval(() => void look(), 15000);
    return () => clearInterval(timer);
  }, [configured, look]);

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
  const stowed = chosen === null ? computer === null : chosen === "stowed";

  const stow = (next: "stowed" | "shown") => {
    setChosen(next);
    remember(agent.id, next);
    if (next === "stowed") setOpen(false);
  };

  if (stowed) {
    return (
      <div className="computer computer--stowed">
        <button
          type="button"
          className="computer__chip"
          onClick={() => stow("shown")}
          title={
            computer
              ? `${agent.name}'s computer is ${computer.state}. Click to watch it.`
              : `${agent.name} has no computer. Click to give it one.`
          }
        >
          <span className="computer__chip-dot" data-state={computer?.state ?? "none"} />
          Computer
        </button>
      </div>
    );
  }

  return (
    <div className="computer" data-open={open ? "true" : undefined} ref={paneRef}>
      <div className="computer__panel">
        <div className="computer__bar">
          <span className="computer__title">Computer</span>
          {computer && (
            <span className="computer__state" data-state={computer.state}>
              {computer.state}
            </span>
          )}

          {running && (
            <>
              {confirming === "sleep" ? (
                <>
                  <button
                    type="button"
                    className="computer__tab computer__tab--danger"
                    disabled={busy}
                    onClick={() => void act(() => api.stopAgentComputer(agent.id))}
                  >
                    Sleep it
                  </button>
                  <button
                    type="button"
                    className="computer__tab"
                    onClick={() => setConfirming(null)}
                  >
                    Keep awake
                  </button>
                </>
              ) : (
                <button
                  type="button"
                  className="computer__tab"
                  disabled={busy}
                  onClick={() => setConfirming("sleep")}
                  title="Sleep. The disk is kept, so it wakes signed in."
                >
                  Sleep
                </button>
              )}
              <button
                type="button"
                className="computer__tab"
                onClick={() => setOpen((o) => !o)}
                title={open ? "Shrink to a preview" : "Make it bigger"}
              >
                {open ? "Minimise" : "Expand"}
              </button>
            </>
          )}
          <button
            type="button"
            className="computer__tab"
            onClick={() => stow("stowed")}
            title="Out of the way. The machine is not touched."
          >
            Hide
          </button>
        </div>

        {running && computer?.vncUrl ? (
          <div className="computer__screen">
            <iframe
              // Keyed on the machine alone, never on the size. Resizing is a CSS
              // change and the connection survives it, so expanding no longer
              // drops the desktop and reconnects to it.
              //
              // Which is also why noVNC's own `view_only` is not used: it is read
              // once when the connection opens, so switching it would mean
              // reconnecting. The veil below does that job instead, and does it
              // without touching the connection.
              key={computer.sandboxId}
              title={`${agent.name}'s computer`}
              src={computer.vncUrl}
            />
            {!open && (
              // Swallows clicks aimed at the desktop while it is only meant to be
              // watched, which is what makes noVNC's own read-only mode
              // unnecessary and the connection worth keeping.
              <button
                type="button"
                className="computer__veil"
                onClick={() => setOpen(true)}
                aria-label="Take control of this computer"
              />
            )}
          </div>
        ) : (
          <div className="computer__empty">
            {error ? (
              <p className="computer__note">{error}</p>
            ) : (
              <p className="computer__note">
                {busy
                  ? "Working on it. This takes a few seconds."
                  : asleep
                    ? `Asleep. Its disk is kept, so it wakes up where it left off, still signed
                     into anything it was signed into. It sleeps again after
                     ${settings?.computerIdleMinutes ?? 15} idle minutes.`
                    : running
                      ? "Running, but the desktop is not up yet."
                      : "No computer yet. Agents get one the first time they use it."}
              </p>
            )}
            <div className="computer__actions">
              <button
                type="button"
                className="btn btn--primary"
                disabled={busy}
                onClick={() => void act(() => api.startAgentComputer(agent.id))}
              >
                {busy ? "Working…" : asleep ? "Wake" : running ? "Start the desktop" : "Give one"}
              </button>
              {computer &&
                (confirming === "destroy" ? (
                  <>
                    <button
                      type="button"
                      className="btn btn--danger"
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
                      className="btn btn--ghost"
                      onClick={() => setConfirming(null)}
                    >
                      Keep
                    </button>
                  </>
                ) : (
                  <button
                    type="button"
                    className="btn btn--ghost"
                    disabled={busy}
                    onClick={() => setConfirming("destroy")}
                  >
                    Destroy
                  </button>
                ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
