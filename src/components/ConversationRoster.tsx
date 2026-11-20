import { useLayoutEffect, useRef } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { dueQuestions } from "../lib/frame";
import { useShown } from "../lib/held";
import { formatCombo } from "../lib/keybinds";
import { newestByAgent } from "../lib/outcomes";
import { numbered } from "../lib/rail";
import { useStore } from "../lib/store";
import { useNow } from "../lib/time";
import type { AgentCard, AgentId, Group } from "../lib/types";

export function ConversationRoster({
  onOpen,
  onQuestions,
}: {
  onOpen: (agentId: AgentId) => void;
  onQuestions: () => void;
}) {
  const agents = useStore((s) => s.agents);
  const groups = useStore((s) => s.groups);
  const selected = useStore((s) => s.selected);
  const outcomes = useStore((s) => s.outcomes);
  const running = useStore((s) => s.running);
  const scroll = useRef<HTMLDivElement>(null);
  const decisions = useStore((s) => s.decisions);
  const { pending, stuck, activity } = useShown();
  const filed = dueQuestions(decisions, useNow(30_000));
  const count = pending.length + stuck.length + filed.length;
  const asking = new Set([...pending, ...stuck, ...filed].map((one) => one.agentId));
  const working = new Set(running.map((one) => one.agentId));
  const newest = newestByAgent(outcomes);
  const order = numbered(agents, groups);
  const assigned = new Set(groups.map((group) => group.id));
  const sections = groups.map((group) => ({
    group,
    agents: order.filter((a) => a.groupId === group.id),
  }));
  const others = order.filter((a) => !assigned.has(a.groupId));
  const all: { group: Group | null; agents: AgentCard[] }[] = sections;
  if (others.length) all.push({ group: null, agents: others });

  useLayoutEffect(() => {
    scroll.current
      ?.querySelector('[aria-current="page"]')
      ?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [selected]);

  return (
    <nav className="roster" aria-label="Agent conversations">
      <button
        type="button"
        className="roster__questions"
        disabled={count === 0}
        onClick={onQuestions}
      >
        {count === 1 ? "1 question waiting" : `${count} questions waiting`}
      </button>
      <div className="roster__scroll" ref={scroll}>
        {all.map(({ group, agents: members }) =>
          members.length ? (
            <section key={group?.id ?? "other"} aria-label={group?.name ?? "Other agents"}>
              <h2 className="roster__group">{group?.name ?? "Other agents"}</h2>
              <ul className="roster__list">
                {members.map((agent) => {
                  const index = order.indexOf(agent);
                  const state = activity[agent.id]?.state;
                  const result = newest[agent.id]?.status;
                  const status = asking.has(agent.id)
                    ? "Has a question"
                    : working.has(agent.id) || state === "thinking" || state === "queued"
                      ? "Working"
                      : result === "failed" || result === "interrupted" || result === "stopped"
                        ? "Needs attention"
                        : result === "done"
                          ? "Has a result"
                          : "Idle";
                  return (
                    <li key={agent.id}>
                      <button
                        type="button"
                        className="roster__agent"
                        aria-label={`${agent.name}, ${status}`}
                        aria-current={agent.id === selected ? "page" : undefined}
                        aria-keyshortcuts={
                          index < 9 ? `Meta+${index + 1} Control+${index + 1}` : undefined
                        }
                        onClick={() => onOpen(agent.id)}
                        data-status={status}
                      >
                        <AgentAvatar
                          avatar={agent.avatar}
                          color={agent.color}
                          seed={agent.id}
                          size="sm"
                          title=""
                        />
                        <span className="roster__label">
                          <span className="roster__name">{agent.name}</span>
                          <span className="roster__status">{status}</span>
                        </span>
                        {index < 9 && (
                          <kbd>{formatCombo({ key: String(index + 1), mod: true })}</kbd>
                        )}
                      </button>
                    </li>
                  );
                })}
              </ul>
            </section>
          ) : null,
        )}
      </div>
    </nav>
  );
}
