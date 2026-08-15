import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { type AgentCard, type Computer, errorMessage } from "../lib/types";

interface Props {
  agent: AgentCard;
}

/**
 * An agent's computer, in the corner of its channel.
 *
 * One view, two sizes. Minimised it is a live but read-only picture behind a
 * transparent veil, so a stray click cannot land in the agent's desktop;
 * expanded it accepts input and the operator can take over.
 *
 * There is deliberately no terminal here. A shell is how the agent works, not
 * how an operator watches it, and a second way in only invited the two to
 * disagree about what was on the machine.
 */
export function ComputerPane({ agent }: Props) {
  const settings = useStore((s) => s.settings);
  const [computer, setComputer] = useState<Computer | null>(null);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [checked, setChecked] = useState(false);

  // Nothing at all until there is a key. Offering to give an agent a computer
  // that cannot be made is worse than not mentioning computers.
  const configured = settings?.e2bKeySet === true;

  const look = useCallback(async () => {
    try {
      setComputer(await api.agentComputer(agent.id));
      setError(null);
    } catch (caught) {
      // A missing key is not a failure worth a red banner: it means the feature
      // was never set up, and the pane says so instead.
      setError(errorMessage(caught));
    } finally {
      setChecked(true);
    }
  }, [agent.id]);

  useEffect(() => {
    setComputer(null);
    setChecked(false);
    setOpen(false);
    // Reset too. A call still in flight when the operator switched agents left
    // this true for the new pane, which disabled its only button permanently
    // and made the terminal swallow every command silently.
    setBusy(false);
    setError(null);
    if (configured) void look();
  }, [look, configured]);

  // Sandboxes expire on their own, so a pane left open goes stale: it kept
  // showing a desktop that had been reclaimed, and clicking it did nothing.
  useEffect(() => {
    if (!configured) return;
    const timer = setInterval(() => void look(), 15000);
    return () => clearInterval(timer);
  }, [configured, look]);

  const act = async (run: () => Promise<Computer | null>) => {
    setBusy(true);
    setError(null);
    try {
      setComputer(await run());
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  if (!configured || !checked) return null;

  const running = computer?.state === "running";

  return (
    <div className="computer" data-open={open ? "true" : undefined}>
      <div className="computer__bar">
        <span className="computer__title">Computer</span>
        {computer && (
          <span className="computer__state" data-state={computer.state}>
            {computer.state}
          </span>
        )}

        {running && (
          <button
            type="button"
            className="computer__tab"
            onClick={() => setOpen((o) => !o)}
            title={open ? "Shrink to a preview" : "Make it bigger"}
          >
            {open ? "Minimise" : "Expand"}
          </button>
        )}
      </div>

      {running && computer?.vncUrl ? (
        <div className="computer__screen">
          <iframe
            // Remounting on the mode switch is deliberate: noVNC decides whether
            // it listens for input when it connects, so flipping view_only on a
            // live connection would do nothing.
            key={`${computer.sandboxId}:${open}`}
            title={`${agent.name}'s computer`}
            src={`${computer.vncUrl}&view_only=${open ? 0 : 1}`}
          />
          {!open && (
            // Covers the frame so a stray click cannot type into the agent's
            // desktop while it is only meant to be watched.
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
                ? "Building a machine. This takes a few seconds."
                : running
                  ? "Running, but the desktop is not up. Start it to watch, or use the terminal."
                  : "No computer yet. Agents get one the first time they run a command."}
            </p>
          )}
          <div className="computer__actions">
            <button
              type="button"
              className="btn btn--primary"
              disabled={busy}
              onClick={() => void act(() => api.startAgentComputer(agent.id))}
            >
              {busy ? "Working…" : running ? "Start the desktop" : "Give one"}
            </button>
            {computer && (
              <button
                type="button"
                className="btn btn--ghost"
                disabled={busy}
                onClick={() =>
                  void act(async () => {
                    await api.deleteAgentComputer(agent.id);
                    return null;
                  })
                }
              >
                Destroy
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
