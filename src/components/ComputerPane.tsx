import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { type AgentCard, type Computer, errorMessage } from "../lib/types";

interface Props {
  agent: AgentCard;
}

interface Line {
  /** The log is append-only, but an index key still breaks if it ever is not. */
  id: number;
  command: string;
  output: string;
  failed: boolean;
  /** Still waiting. The first command on a cold agent makes a sandbox first,
      which takes seconds, and a prompt that just says "running" reads as
      broken. */
  pending: boolean;
}

let lineSeq = 0;

/**
 * An agent's computer, in the corner of its channel.
 *
 * Two views of one machine. The screen is E2B's noVNC, embedded straight from
 * the sandbox's public URL: minimised it is a live but read-only picture behind
 * a transparent veil, so a stray click cannot land in the agent's desktop;
 * maximised it accepts input and the operator can take over.
 *
 * The terminal is not an embedded shell. It runs commands through exactly the
 * call the agent's own `run_command` tool makes, so the operator and the agent
 * are looking at one machine through one mechanism rather than two that can
 * disagree about what is on it.
 */
export function ComputerPane({ agent }: Props) {
  const settings = useStore((s) => s.settings);
  const [computer, setComputer] = useState<Computer | null>(null);
  const [view, setView] = useState<"screen" | "terminal">("screen");
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [checked, setChecked] = useState(false);
  const [lines, setLines] = useState<Line[]>([]);
  const [command, setCommand] = useState("");
  const logRef = useRef<HTMLDivElement>(null);

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
    setLines([]);
    if (configured) void look();
  }, [look, configured]);

  useEffect(() => {
    const node = logRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [lines]);

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

  const send = async () => {
    const next = command.trim();
    if (!next || busy) return;

    // Echoed before the call, not after it. Waiting in silence for a command
    // that takes seconds is indistinguishable from a terminal that is broken.
    const id = lineSeq++;
    setLines((prior) => [
      ...prior,
      { id, command: next, output: "", failed: false, pending: true },
    ]);
    setBusy(true);
    setCommand("");

    const settle = (output: string, failed: boolean) =>
      setLines((prior) =>
        prior.map((line) => (line.id === id ? { ...line, output, failed, pending: false } : line)),
      );

    try {
      const result = await api.runOnAgentComputer(agent.id, next);
      const body = [result.stdout, result.stderr && `stderr: ${result.stderr}`]
        .filter(Boolean)
        .join("\n")
        .trimEnd();
      settle(
        body || (result.exitCode === 0 ? "(no output)" : `exit ${result.exitCode}`),
        result.exitCode !== 0,
      );
    } catch (caught) {
      settle(errorMessage(caught), true);
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
              title={open ? "Shrink to a preview" : "Make it bigger"}
            >
              {open ? "Minimise" : "Expand"}
            </button>
          </>
        )}
      </div>

      {running && view === "terminal" ? (
        <div className="computer__term">
          <div className="computer__log" ref={logRef}>
            {lines.length === 0 && (
              <p className="computer__note">
                The same machine the agent uses. Anything you leave here, it will find.
              </p>
            )}
            {lines.map((line) => (
              <div key={line.id}>
                <div className="computer__cmd">
                  <span aria-hidden="true">$</span> {line.command}
                </div>
                {line.pending ? (
                  <pre className="computer__out computer__out--pending">working…</pre>
                ) : (
                  line.output && (
                    <pre className="computer__out" data-failed={line.failed ? "true" : undefined}>
                      {line.output}
                    </pre>
                  )
                )}
              </div>
            ))}
          </div>
          <div className="computer__prompt">
            <span aria-hidden="true">$</span>
            <input
              value={command}
              spellCheck={false}
              placeholder={busy ? "running…" : "curl -s wttr.in/Charleston?format=3"}
              onChange={(event) => setCommand(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") void send();
              }}
            />
          </div>
        </div>
      ) : running && computer?.vncUrl ? (
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
              {running
                ? "Running. Start the desktop to watch it, or use the terminal."
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
                    setLines([]);
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
