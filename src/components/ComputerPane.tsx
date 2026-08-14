import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { type AgentCard, type Computer, errorMessage } from "../lib/types";

interface Props {
  agent: AgentCard;
}

/**
 * An agent's computer, in the corner of its channel.
 *
 * Two sizes, and the difference is not just scale. Minimised it is a live but
 * read-only picture: the operator watches what the agent is doing without their
 * pointer landing in the middle of it. Maximised, the same desktop accepts
 * input and the operator can take over.
 *
 * The frame is Daytona's own noVNC client, but it is not loaded from Daytona
 * directly. Daytona puts an interstitial in front of every preview request, so
 * loading it straight returned the warning page in place of noVNC's stylesheet
 * and scripts. Both views therefore come through `guaccomputer://`, which Rust
 * forwards with the header that suppresses it.
 */
export function ComputerPane({ agent }: Props) {
  const [computer, setComputer] = useState<Computer | null>(null);
  const [view, setView] = useState<"screen" | "terminal">("screen");
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [checked, setChecked] = useState(false);

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
    void look();
  }, [look]);

  // A sandbox reports `creating` or `starting` before it can serve a desktop,
  // so the pane polls itself up rather than leaving a dead frame on screen.
  useEffect(() => {
    if (!computer || computer.state === "started" || computer.state === "stopped") return;
    const timer = setTimeout(() => void look(), 2000);
    return () => clearTimeout(timer);
  }, [computer, look]);

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

  if (!checked) return null;

  const running = computer?.state === "started";
  const url = view === "screen" ? computer?.vncUrl : computer?.terminalUrl;

  return (
    <>
      <div className="computer" data-open={open ? "true" : undefined}>
        <div className="computer__bar">
          <span className="computer__title">Computer</span>
          {computer && (
            <span className="computer__state" data-state={computer.state}>
              {computer.state}
            </span>
          )}

          {running && (
            <>
              <button
                type="button"
                className="computer__tab"
                aria-pressed={view === "screen"}
                onClick={() => setView("screen")}
              >
                Screen
              </button>
              <button
                type="button"
                className="computer__tab"
                aria-pressed={view === "terminal"}
                onClick={() => setView("terminal")}
              >
                Terminal
              </button>
              <button
                type="button"
                className="computer__tab"
                onClick={() => setOpen((o) => !o)}
                title={open ? "Shrink to a preview" : "Take control"}
              >
                {open ? "Minimise" : "Take control"}
              </button>
            </>
          )}
        </div>

        {running && url ? (
          <div className="computer__screen">
            <iframe
              // Remounting on the mode switch is deliberate: noVNC decides
              // whether it listens for input when it connects, so flipping
              // view_only on a live connection would do nothing.
              key={`${computer?.sandboxId}:${view}:${open}`}
              title={`${agent.name}'s computer`}
              src={view === "screen" ? `${url}&view_only=${open ? 0 : 1}` : url}
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
                {computer
                  ? computer.state === "stopped"
                    ? "Asleep. Its disk is kept."
                    : `${computer.state}…`
                  : "No computer yet."}
              </p>
            )}
            <div className="computer__actions">
              <button
                type="button"
                className="btn btn--primary"
                disabled={busy}
                onClick={() => void act(() => api.startAgentComputer(agent.id))}
              >
                {busy ? "Working…" : computer ? "Wake" : "Give one"}
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

      {running && (
        <div className="computer__foot">
          <button
            type="button"
            className="btn btn--ghost"
            disabled={busy}
            onClick={() => void act(() => api.stopAgentComputer(agent.id))}
          >
            Put to sleep
          </button>
        </div>
      )}
    </>
  );
}
