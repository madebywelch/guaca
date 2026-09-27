import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import {
  type AgentCard,
  errorMessage,
  type Gate,
  HARNESSES,
  type Harness,
  type HarnessOnMachine,
} from "../lib/types";

interface Props {
  agent: AgentCard;
}

/** What an operator is shown for a harness, including one this build predates. */
function labelOf(harness: Harness): string {
  return HARNESSES.find((known) => known.id === harness)?.label ?? harness;
}

/**
 * An agent's terminal, in its panel: whether it has one, which program writes
 * its code, and whether its pushes ask first.
 *
 * Given and taken back like a computer, and for the same reason: it is a
 * decision about this agent, and what it can reach follows from it. The two
 * answers under it are written the moment they are clicked, which is what a
 * `.choice` means everywhere else in this app; staged under a Save button, a
 * harness switch made because a plan just ran out is the change most likely to
 * be lost.
 *
 * Taking it back keeps the directory and both answers. The work in it is the
 * agent's, and a change of mind about access is not a reason to delete it.
 */
export function TerminalPanel({ agent }: Props) {
  const [machine, setMachine] = useState<HarnessOnMachine[] | null>(null);
  const [path, setPath] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Which agent the panel is currently about. A lookup started for one agent
  // landing after the operator switched would paint its path under another.
  const showing = useRef(agent.id);
  const given = agent.hasTerminal;

  const look = useCallback(async () => {
    const asked = agent.id;
    try {
      const [terminal, harnesses] = await Promise.all([
        api.agentTerminal(asked),
        api.codingHarnesses(),
      ]);
      if (showing.current !== asked) return;
      setPath(terminal.path);
      setMachine(harnesses);
    } catch (caught) {
      if (showing.current === asked) setError(errorMessage(caught));
    }
  }, [agent.id]);

  useEffect(() => {
    showing.current = agent.id;
    setPath(null);
    setMachine(null);
    setBusy(false);
    setError(null);
    if (given) void look();
  }, [agent.id, given, look]);

  /**
   * Every change here is a change to the card, and the roster refresh that
   * follows is what redraws this panel. Nothing is patched locally.
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

  // Null is the check still running, and it must not disable every harness.
  const has = (harness: Harness) => {
    const row = machine?.find((known) => known.harness === harness);
    return row === undefined || (row.installed && !row.withheld);
  };
  const chosen = machine?.find((row) => row.harness === agent.harness);

  return (
    <section className="worknotes" aria-label={`${agent.name}'s terminal`}>
      <div className="worknotes__head">
        <h3 className="worknotes__title">Terminal</h3>
      </div>

      {!given ? (
        <>
          <p className="worknotes__empty">
            {agent.name} has no terminal. Give it one and it gets a directory of its own on the
            machine Guaca runs on: it can run commands there, clone repositories, change files, and
            hand bigger changes to a coding agent.
          </p>
          <div className="screen__offer">
            <button
              type="button"
              className="btn btn--small btn--primary"
              disabled={busy}
              onClick={() => void decide(() => api.giveAgentTerminal(agent.id))}
            >
              {busy ? "Working…" : "Give one"}
            </button>
          </div>
        </>
      ) : (
        <>
          <p className="field__hint">
            {path ? (
              <>
                Its directory is <code>{path}</code>. Commands run there as the backend's user, with
                that machine's git and GitHub sign-ins. This is not a sandbox.
              </>
            ) : (
              "Finding its directory…"
            )}
          </p>

          <div className="field">
            <span className="field__label">Writes code with</span>
            <div className="choices">
              {HARNESSES.map((harness) => (
                <button
                  key={harness.id}
                  type="button"
                  className="choice choice--tight"
                  aria-label={`Coding harness: ${harness.label}`}
                  aria-pressed={harness.id === agent.harness}
                  disabled={busy || !has(harness.id)}
                  onClick={() =>
                    void decide(() => api.setAgentCoding(agent.id, harness.id, agent.gate))
                  }
                >
                  {harness.label}
                </button>
              ))}
            </div>
            <span className="field__hint">
              {!chosen ? (
                `${labelOf(agent.harness)}: status not checked yet.`
              ) : chosen.withheld ? (
                `${labelOf(agent.harness)}: ${chosen.withheld}.`
              ) : !chosen.installed ? (
                <>
                  {labelOf(agent.harness)} is not installed. On the backend, run{" "}
                  <code>{chosen.install}</code>.
                </>
              ) : chosen.signedIn === false ? (
                <>
                  {labelOf(agent.harness)} is not signed in on the backend. Run{" "}
                  <code>{chosen.signIn}</code> as the backend's user. Guaca's own sign-in does not
                  sign in the coding tool.
                </>
              ) : (
                `${labelOf(agent.harness)} uses its own account and model settings on the backend.`
              )}
            </span>
          </div>

          <label className="field field--row">
            <input
              type="checkbox"
              checked={agent.gate === "askBeforePushing"}
              disabled={busy}
              onChange={(event) => {
                const gate: Gate = event.target.checked ? "askBeforePushing" : "open";
                void decide(() => api.setAgentCoding(agent.id, agent.harness, gate));
              }}
            />
            <span>
              <span className="field__label">Ask me before pushing</span>
              <span className="field__hint">
                A push, pull request, merge or release waits on your desk first, whether{" "}
                {agent.name} runs it or its coding agent does. Everything else runs without asking.
                This is an approval step, not a sandbox.
              </span>
            </span>
          </label>

          <div className="screen__offer">
            <button
              type="button"
              className="btn btn--small btn--ghost"
              disabled={busy}
              onClick={() => void decide(() => api.takeAgentTerminal(agent.id))}
              title="Take it back. A running coding job stops, and the directory is kept."
            >
              Take it back
            </button>
          </div>
        </>
      )}

      {error && (
        <p className="field__error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
