import { type FormEvent, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { ABSENT_ACCENT } from "../avatars/catalog";
import { useStore } from "../lib/store";
import type { AgentCard, Part } from "../lib/types";

type QuestionPart = Extract<Part, { type: "question" }>;

interface Props {
  part: QuestionPart;
  agent: AgentCard | undefined;
}

const SETTLED: Record<string, string> = {
  answered: "You answered this.",
  expired: "Nobody answered, so it went ahead without you.",
};

const FILED =
  "Nobody answered in ten minutes, so it went ahead without you. The question is waiting in For you.";

export function QuestionRequest({ part, agent }: Props) {
  const state = useStore((s) => s.approvals[part.id]) ?? "expired";
  const said = useStore((s) => s.answers[part.id]);
  const answer = useStore((s) => s.answerQuestion);
  const filed = useStore((s) => s.decisions.some((d) => d.topic === `approval:${part.id}`));
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
          color={agent?.color ?? ABSENT_ACCENT}
          size="sm"
          seed={agent?.id ?? part.id}
        />
        <div>
          <p className="ask__summary">{part.question}</p>
          <p className="ask__note">
            {state === "pending"
              ? `${asker} is waiting on you. Nothing is being decided for you here.`
              : state === "expired" && filed
                ? FILED
                : (SETTLED[state] ?? "This question is no longer live.")}
          </p>
        </div>
      </div>

      {state === "answered" && said && <p className="decision__answer ask__given">{said}</p>}

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
