import { type FormEvent, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { useStore } from "../lib/store";
import type { AgentCard, Part } from "../lib/types";

type QuestionPart = Extract<Part, { type: "question" }>;

interface Props {
  part: QuestionPart;
  /** The agent that asked. Undefined once it has been deleted. */
  agent: AgentCard | undefined;
}

/** What the operator is told afterward, per outcome. */
const SETTLED: Record<string, string> = {
  answered: "You answered this.",
  expired: "Nobody answered, so it went ahead without you.",
};

/**
 * An agent asking the operator what to do, in the channel it asked from.
 *
 * The other half of {@link ApprovalRequest}, and drawn as its own card for the
 * reason the two are separate parts: nothing answered here grants anything. The
 * agent could have gone either way and stopped because it does not know which
 * way is wanted, so the wording is a question rather than a request, and the
 * note under it says what happens if nobody answers instead of what has not
 * happened yet.
 *
 * The choices are the agent's own words. They are the only model-authored text
 * in this app that lands on a button, and that is safe exactly here: the value
 * goes back to the agent, and whatever it does with it passes every guard it
 * already had. Drawn as text, never as markdown, and already cut to a label's
 * length by the runtime.
 */
export function QuestionRequest({ part, agent }: Props) {
  // A request the store has never heard of is older than the window it loads,
  // so it cannot still be live. Drawing a live field for one would take an
  // answer that reaches nobody.
  const state = useStore((s) => s.approvals[part.id]) ?? "expired";
  const answer = useStore((s) => s.answerQuestion);
  const [sending, setSending] = useState(false);
  const [written, setWritten] = useState("");

  const asker = agent?.name ?? "A deleted agent";

  const send = (text: string) => {
    setSending(true);
    void answer(part.id, text).finally(() => setSending(false));
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const text = written.trim();
    if (!text) return;
    send(text);
  };

  return (
    <div className="ask ask--question" data-state={state}>
      <div className="ask__head">
        <AgentAvatar
          avatar={agent?.avatar ?? "blank"}
          color={agent?.color ?? "#8aa0a6"}
          size="sm"
          seed={agent?.id ?? part.id}
        />
        <div>
          <p className="ask__summary">{part.question}</p>
          <p className="ask__note">
            {state === "pending"
              ? `${asker} is waiting on you. Nothing is being decided for you here.`
              : (SETTLED[state] ?? "This question is no longer live.")}
          </p>
        </div>
      </div>

      {state === "pending" && (
        <div className="ask__actions">
          {part.options.length > 0 ? (
            part.options.map((option) => (
              <button
                key={option}
                type="button"
                className="btn"
                disabled={sending}
                onClick={() => send(option)}
              >
                {option}
              </button>
            ))
          ) : (
            <form className="ask__answer" onSubmit={submit}>
              <input
                type="text"
                className="ask__field"
                value={written}
                disabled={sending}
                placeholder="Your answer"
                onChange={(event) => setWritten(event.target.value)}
              />
              <button
                type="submit"
                className="btn btn--primary"
                disabled={sending || written.trim() === ""}
              >
                Send
              </button>
            </form>
          )}
          <span className="ask__scope">This answers {asker}. It permits nothing.</span>
        </div>
      )}
    </div>
  );
}
