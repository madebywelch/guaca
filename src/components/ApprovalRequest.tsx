import { useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { type AgentCard, type Decision, errorMessage, type Part } from "../lib/types";

type ApprovalPart = Extract<Part, { type: "approval" }>;

interface Props {
  part: ApprovalPart;
  /** The agent that asked. Undefined once it has been deleted. */
  agent: AgentCard | undefined;
}

/** What the operator is told after the fact, per outcome. */
const SETTLED: Record<string, string> = {
  allow: "You allowed this.",
  alwaysAllow: "You allowed this, and said not to ask again.",
  deny: "You declined.",
  expired: "Nobody answered, so nothing happened.",
};

/**
 * An agent asking the operator for permission, in the channel it asked from.
 *
 * The agent is parked mid-turn while this is on screen, which is why it is a
 * card in the transcript rather than a dialog: the operator can read what led
 * to the request before answering it, and a request nobody answers lapses on
 * its own rather than blocking the window.
 *
 * Every value here is what a model asked for, so all of it is rendered as
 * text. An agent that could format its own request could draw a button.
 */
export function ApprovalRequest({ part, agent }: Props) {
  // A request the store has never heard of is older than the window it loads,
  // so it cannot still be live. Drawing live buttons for one would offer the
  // operator a decision that no longer reaches anybody.
  const state = useStore((s) => s.approvals[part.id]) ?? "expired";
  const refreshApprovals = useStore((s) => s.refreshApprovals);
  const setBanner = useStore((s) => s.setBanner);
  const [deciding, setDeciding] = useState<Decision | null>(null);

  const asker = agent?.name ?? "A deleted agent";

  const answer = (decision: Decision) => {
    setDeciding(decision);
    void api
      .decideApproval(part.id, decision)
      .catch((error) => {
        // Answered somewhere else, or lapsed while this was on screen. The
        // server's copy is the truth, so take it rather than arguing.
        setBanner({ tone: "error", text: errorMessage(error) });
        void refreshApprovals();
      })
      .finally(() => setDeciding(null));
  };

  return (
    <div className="ask" data-state={state}>
      <div className="ask__head">
        <AgentAvatar
          avatar={agent?.avatar ?? "blank"}
          color={agent?.color ?? "#8aa0a6"}
          size="sm"
          seed={agent?.id ?? part.id}
        />
        <div>
          <p className="ask__summary">{part.summary}</p>
          <p className="ask__note">
            {state === "pending"
              ? `${asker} is waiting on you. Nothing has been created yet.`
              : (SETTLED[state] ?? "This request is no longer live.")}
          </p>
        </div>
      </div>

      <dl className="ask__detail">
        {part.detail.map((field) => (
          <div key={`${part.id}:${field.label}`}>
            <dt>{field.label}</dt>
            <dd>{field.value}</dd>
          </div>
        ))}
      </dl>

      {state === "pending" && (
        <div className="ask__actions">
          <button
            type="button"
            className="btn btn--primary"
            disabled={deciding !== null}
            onClick={() => answer("allow")}
          >
            Allow
          </button>
          <button
            type="button"
            className="btn"
            disabled={deciding !== null}
            title={`Stop asking when ${asker} does this`}
            onClick={() => answer("alwaysAllow")}
          >
            Always allow
          </button>
          <button
            type="button"
            className="btn btn--ghost"
            disabled={deciding !== null}
            onClick={() => answer("deny")}
          >
            Deny
          </button>
          {/* The scope of the middle button, said in full. "Always" reads as a
              setting for the workspace, and it is not: it is one agent being
              let off one question. */}
          <span className="ask__scope">Always allow is for {asker} only.</span>
        </div>
      )}
    </div>
  );
}
