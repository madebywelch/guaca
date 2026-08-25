import { useEffect, useRef } from "react";

import { useStore } from "../lib/store";
import type { AgentId } from "../lib/types";

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

  // Follows its own end unconditionally, unlike the transcript. The panel is
  // short, it is only ever on screen while something is happening in it, and
  // nobody scrolls back through a job that has not finished: the finished
  // account arrives in the channel underneath.
  useEffect(() => {
    floor.current?.scrollIntoView({ block: "end" });
  }, []);

  const working = Object.entries(building).find(([, who]) => who === agent);
  if (!working) return null;

  const repository = repositories.find((r) => r.id === working[0]);
  const held = lines ?? [];

  return (
    <section className="coding" aria-label="Coding job in progress">
      <div className="coding__head">
        <span className="coding__pulse" aria-hidden="true" />
        <span className="coding__title">Writing code in {repository?.name ?? "a repository"}</span>
      </div>

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
    </section>
  );
}
