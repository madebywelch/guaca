import { needsAnswer } from "./decisions";
import { workspaceOrigin } from "./transport";
import type { AgentCard, AgentId, Approval, Escalation, Running, WorkDecision } from "./types";

export type LiveQuestion = Approval | Escalation;

export function dueQuestions(decisions: readonly WorkDecision[], now: number) {
  return decisions.filter(
    (one) => needsAnswer(one) && (one.snoozedUntil === null || one.snoozedUntil <= now),
  );
}

export function lastConversation(): AgentId | null {
  try {
    return localStorage.getItem(`guaca.lastConversation:${workspaceOrigin()}`);
  } catch {
    return null;
  }
}

export function rememberConversation(agentId: AgentId) {
  try {
    localStorage.setItem(`guaca.lastConversation:${workspaceOrigin()}`, agentId);
  } catch {
    return;
  }
}

export function nextQuestion(
  pending: readonly Approval[],
  stuck: readonly Escalation[],
): LiveQuestion | undefined {
  return (
    [...pending].sort(
      (a, b) => (a.lapsesAt ?? Infinity) - (b.lapsesAt ?? Infinity) || a.createdAt - b.createdAt,
    )[0] ?? [...stuck].sort((a, b) => a.raisedAt - b.raisedAt)[0]
  );
}

export function questionConversation(
  question: LiveQuestion,
  order: readonly AgentCard[],
  running: readonly Running[],
): AgentId | undefined {
  const owner = running.find((run) => run.runId === question.runId)?.agentId;
  if (order.some((agent) => agent.id === owner)) return owner;
  if (order.some((agent) => agent.id === question.agentId)) return question.agentId;
  return undefined;
}

export function openingConversation(
  order: readonly AgentCard[],
  pending: readonly Approval[],
  stuck: readonly Escalation[],
  running: readonly Running[],
  lastActive: Readonly<Record<AgentId, number>>,
  lastOpen: AgentId | null,
): AgentId | undefined {
  const live = new Set(order.map((agent) => agent.id));
  const answerable = (one: LiveQuestion) => questionConversation(one, order, running) !== undefined;
  const question = nextQuestion(pending.filter(answerable), stuck.filter(answerable));
  if (question) return questionConversation(question, order, running);
  const recent = running
    .filter((run) => live.has(run.agentId))
    .sort(
      (a, b) =>
        Math.max(b.startedAt, lastActive[b.agentId] ?? 0) -
        Math.max(a.startedAt, lastActive[a.agentId] ?? 0),
    )[0];
  return recent?.agentId ?? (lastOpen && live.has(lastOpen) ? lastOpen : order[0]?.id);
}
