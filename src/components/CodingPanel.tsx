import { useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { type AgentId, errorMessage } from "../lib/types";

interface Props {
  agent: AgentId;
}

/**
 * What a coding agent is doing right now, in the channel of the agent that
 * asked for it.
 *
 * ## Why this exists at all
 *
 * `code` returns as soon as the harness is up and the turn ends, so the channel
 * goes silent while a coding agent works in the repository for twenty minutes.
 * The operator's own words for it: the chat is quiet and the only evidence
 * anything is happening is pull requests appearing on GitHub. The rail says
 * `building` beside the repository, which answers *whether*; this answers
 * *what*.
 *
 * ## Why it is a filtered line and not the stream
 *
 * `pi --mode json` emits tens of thousands of events for a real job, almost all
 * of them text deltas and a cumulative usage total repeated on each one.
 * Forwarding that into the webview would be a re-render per token for a panel
 * nobody could read fast enough. The runtime keeps the two things a person
 * watching over a shoulder wants — the tool it reached for, and what it says as
 * it goes — and drops the rest.
 *
 * ## Why nothing here is kept
 *
 * This is the same discipline as a turn's thinking. The record of what a job
 * did is the message it delivers when it finishes, which is in the transcript
 * and in the prompt and in search. This is only what that record looks like
 * before it exists, so it is held in memory, bounded, and dropped when the job
 * ends. A second durable copy would eventually disagree with the first.
 */
export function CodingPanel({ agent }: Props) {
  const lines = useStore((s) => s.coding[agent]);
  const building = useStore((s) => s.building);
  const repositories = useStore((s) => s.repositories);
  const floor = useRef<HTMLDivElement>(null);
  const [correction, setCorrection] = useState("");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  // Armed before it fires, like the two destructive items in `AgentMenu`. This
  // ends forty minutes of work that cannot be resumed, and the confirmation is
  // drawn where the click happened rather than somewhere the operator has to
  // go and find it.
  const [confirming, setConfirming] = useState(false);

  // Follows its own end unconditionally, unlike the transcript. The panel is
  // short, it is only ever on screen while something is happening in it, and
  // nobody scrolls back through a job that has not finished: the finished
  // account arrives in the channel underneath.
  useEffect(() => {
    floor.current?.scrollIntoView({ block: "end" });
  }, []);

  const repositoryId = building[agent];
  if (!repositoryId) return null;

  const repository = repositories.find((r) => r.id === repositoryId);
  const held = lines ?? [];

  const send = async () => {
    const message = correction.trim();
    if (!message) return;
    setBusy(true);
    try {
      await api.messageCodingJob(agent, message);
      setCorrection("");
      // Said rather than left to the transcript. What the operator typed goes
      // to the harness and never becomes a message anywhere, so without this
      // the only evidence it arrived is the job changing course minutes later.
      setNote("Sent. It reaches the job at its next step.");
    } catch (err) {
      setNote(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    setBusy(true);
    try {
      await api.stopCodingJob(agent);
    } catch (err) {
      setNote(errorMessage(err));
    } finally {
      setBusy(false);
      setConfirming(false);
    }
  };

  return (
    <section className="coding" aria-label="Coding job in progress">
      <div className="coding__head">
        <span className="coding__pulse" aria-hidden="true" />
        <span className="coding__title">Writing code in {repository?.name ?? "a repository"}</span>
        {confirming ? (
          <>
            <button
              type="button"
              className="btn btn--small btn--danger"
              disabled={busy}
              onClick={() => void stop()}
            >
              Stop it
            </button>
            <button
              type="button"
              className="btn btn--small btn--ghost"
              onClick={() => setConfirming(false)}
            >
              Keep going
            </button>
          </>
        ) : (
          <button
            type="button"
            className="btn btn--small btn--ghost"
            disabled={busy}
            onClick={() => setConfirming(true)}
          >
            Stop
          </button>
        )}
      </div>

      <div className="coding__tail">
        {held.length === 0 ? (
          // The gap between starting the harness and its first tool call is
          // several seconds of model call. Silence there reads as a panel that is
          // broken rather than a job that has not got going.
          <p className="coding__waiting">Starting the coding agent…</p>
        ) : (
          <ol className="coding__lines">
            {held.map((line, at) => (
              // Indexed on purpose: these are positions in a tail, not entities.
              // Two identical `bash: npm test` lines are two runs of the tests
              // and both belong on screen.
              // biome-ignore lint/suspicious/noArrayIndexKey: a tail has no ids
              <li className="coding__line" key={at}>
                {line.tool ? (
                  <>
                    <span className="coding__tool">{line.tool}</span>
                    <span className="coding__detail">{line.detail}</span>
                  </>
                ) : (
                  <span className="coding__said">{line.detail}</span>
                )}
              </li>
            ))}
          </ol>
        )}
        <div ref={floor} />
      </div>

      {confirming && (
        // What survives is the half an operator cannot see from here, and it is
        // the half that decides whether they press it.
        <p className="coding__waiting">
          Stopping kills the program where it stands. Whatever it has committed stays; whatever it
          was in the middle of does not. It cannot be resumed from here.
        </p>
      )}

      <div className="coding__say">
        <input
          className="input input--slim"
          placeholder="change course: use the other endpoint, stop after the tests…"
          value={correction}
          disabled={busy}
          aria-label="Send a correction to the running coding job"
          onChange={(event) => {
            setCorrection(event.target.value);
            setNote(null);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter" && correction.trim()) void send();
          }}
        />
        <button
          type="button"
          className="btn btn--small"
          disabled={busy || !correction.trim()}
          onClick={() => void send()}
        >
          Send
        </button>
      </div>
      {note && <p className="coding__waiting">{note}</p>}
    </section>
  );
}
