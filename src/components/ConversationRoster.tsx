import { type ReactNode, useLayoutEffect, useRef } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { dueQuestions } from "../lib/frame";
import { useShown } from "../lib/held";
import { formatCombo } from "../lib/keybinds";
import { newestByAgent } from "../lib/outcomes";
import { numbered } from "../lib/rail";
import { useStore } from "../lib/store";
import { useNow } from "../lib/time";
import type { AgentId } from "../lib/types";
import { SeasonGreeting, SeasonScene } from "./Seasonal";

export function ConversationRoster({
  onOpen,
  onQuestions,
  foot,
}: {
  onOpen: (agentId: AgentId) => void;
  onQuestions: () => void;
  foot: ReactNode;
}) {
  const now = useNow(30_000);
  const agents = useStore((s) => s.agents);
  const groups = useStore((s) => s.groups);
  const selected = useStore((s) => s.selected);
  const outcomes = useStore((s) => s.outcomes);
  const running = useStore((s) => s.running);
  const scroll = useRef<HTMLDivElement>(null);
  const decisions = useStore((s) => s.decisions);
  const { pending, stuck, activity } = useShown();
  const filed = dueQuestions(decisions, now);
  const count = pending.length + stuck.length + filed.length;
  const asking = new Set([...pending, ...stuck, ...filed].map((one) => one.agentId));
  const working = new Set(running.map((one) => one.agentId));
  const newest = newestByAgent(outcomes);
  const order = numbered(agents, groups).map((agent, index) => {
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
            : agent.lifecycle === "paused"
              ? "Paused"
              : "Idle";
    const key = index < 9 ? formatCombo({ key: String(index + 1), mod: true }) : undefined;
    return { agent, index, status, key };
  });
  const sections = groups.map((group) => ({
    id: group.id,
    name: group.name,
    rows: order.filter(({ agent }) => agent.groupId === group.id),
  }));
  const assigned = new Set(groups.map((group) => group.id));
  const others = order.filter(({ agent }) => !assigned.has(agent.groupId));
  const all = [...sections, { id: "other", name: "Other agents", rows: others }].filter(
    ({ rows }) => rows.length > 0,
  );

  useLayoutEffect(() => {
    scroll.current
      ?.querySelector('[aria-current="page"]')
      ?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [selected]);

  return (
    <div className="roster">
      <nav className="roster__nav" aria-label="Agent conversations">
        <button
          type="button"
          className="roster__questions"
          disabled={count === 0}
          onClick={onQuestions}
        >
          {count === 1 ? "1 question waiting" : `${count} questions waiting`}
        </button>
        <select
          className="roster__compact"
          aria-label="Switch agent"
          value={selected ?? ""}
          onChange={(event) => onOpen(event.currentTarget.value)}
        >
          {all.map(({ id, name, rows }) => (
            <optgroup key={id} label={name}>
              {rows.map(({ agent, status, key }) => (
                <option key={agent.id} value={agent.id}>
                  {agent.name}, {status}
                  {key && ` (${key})`}
                </option>
              ))}
            </optgroup>
          ))}
        </select>
        <div className="roster__scroll" ref={scroll}>
          {all.map(({ id, name, rows }) => (
            <section key={id} aria-label={name}>
              <h2 className="roster__group">{name}</h2>
              <ul className="roster__list">
                {rows.map(({ agent, index, status, key }) => (
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
                        <span className="roster__name" title={agent.name}>
                          {agent.name}
                        </span>
                        <span className="roster__status">{status}</span>
                      </span>
                      {key && <kbd>{key}</kbd>}
                    </button>
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
        <SeasonScene />
      </nav>
      <footer className="roster__foot">
        {foot}
        <p className="roster__day">
          <span className="roster__day-long">
            {new Date(now).toLocaleDateString([], {
              weekday: "long",
              month: "long",
              day: "numeric",
            })}
          </span>
          <span className="roster__day-short">
            {new Date(now).toLocaleDateString([], { month: "short", day: "numeric" })}
          </span>
        </p>
        <SeasonGreeting />
      </footer>
    </div>
  );
}
