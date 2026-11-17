import { useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { ABSENT_ACCENT } from "../avatars/catalog";
import { useStore } from "../lib/store";
import type { AgentCard, Decision, Part, ProtectedAction } from "../lib/types";

type ApprovalPart = Extract<Part, { type: "approval" }>;

interface Props {
  part: ApprovalPart;
  agent: AgentCard | undefined;
}

const NOTHING_YET: Record<ProtectedAction, string> = {
  createAgent: "Nothing has been created yet.",
  actOnBehalf: "Nothing has been sent yet.",
  changeSettings: "Nothing has been changed yet.",
};

const SETTLED: Record<string, string> = {
  allow: "You allowed this.",
  alwaysAllow: "You allowed this, and said not to ask again.",
  deny: "You declined.",
  expired: "Nobody answered, so nothing happened.",
};

const FILED = "Nobody answered in ten minutes. It is waiting in For you.";

export function ApprovalRequest({ part, agent }: Props) {
  const state = useStore((s) => s.approvals[part.id]) ?? "expired";
  const decide = useStore((s) => s.decideApproval);
  const filed = useStore((s) => s.decisions.some((d) => d.topic === `approval:${part.id}`));
  const [deciding, setDeciding] = useState<Decision | null>(null);

  const asker = agent?.name ?? "A deleted agent";

  const answer = (decision: Decision) => {
    setDeciding(decision);
    void decide(part.id, decision).finally(() => setDeciding(null));
  };

  return (
    <div className="ask" data-state={state}>
      <div className="ask__head">
        <AgentAvatar
          avatar={agent?.avatar ?? "blank"}
          color={agent?.color ?? ABSENT_ACCENT}
          size="sm"
          seed={agent?.id ?? part.id}
        />
        <div>
          <p className="ask__summary">{part.summary}</p>
          <p className="ask__note">
            {state === "pending"
              ? `${asker} is waiting on you. ${NOTHING_YET[part.action]}`
              : state === "expired" && filed
                ? FILED
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
          {part.action === "createAgent" && (
            <button
              type="button"
              className="btn"
              disabled={deciding !== null}
              title={`Stop asking when ${asker} does this`}
              onClick={() => answer("alwaysAllow")}
            >
              Always allow
            </button>
          )}
          <button
            type="button"
            className="btn btn--ghost"
            disabled={deciding !== null}
            onClick={() => answer("deny")}
          >
            Deny
          </button>
          {part.action === "createAgent" ? (
            <span className="ask__scope">Always allow is for {asker} only.</span>
          ) : (
            <span className="ask__scope">Nothing here is remembered after this turn.</span>
          )}
        </div>
      )}
    </div>
  );
}
