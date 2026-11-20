import type { Activity, AgentId, RunId, Running } from "./types";

export function together(names: readonly string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  return `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`;
}

export const LEAD_NAMES = 3;

export function workingLead(names: readonly string[]): string {
  const distinct = [...new Set(names)];
  if (distinct.length > LEAD_NAMES) return `${distinct.length} agents are working.`;
  return `${together(distinct)} ${distinct.length === 1 ? "is" : "are"} working.`;
}

export function colleaguesOn(
  run: Pick<Running, "runId" | "agentId">,
  activeRun: Record<AgentId, RunId | undefined>,
  nameOf: (id: AgentId) => string | undefined,
): string[] {
  return Object.entries(activeRun)
    .filter(([agent, on]) => on === run.runId && agent !== run.agentId)
    .flatMap(([agent]) => nameOf(agent as AgentId) ?? [])
    .sort((a, b) => a.localeCompare(b));
}

export function runningNote(
  run: Pick<Running, "note">,
  owner: Activity | undefined,
  colleagues: readonly string[],
  sending = false,
): string {
  if (sending) return "Sending your answer";
  if (run.note) return run.note;
  if (owner?.state === "awaitingApproval") return "Waiting for your answer";
  if (owner?.state === "paused") return "Paused";
  if (colleagues.length > 0) return `Waiting for ${together(colleagues)}`;
  if (owner?.state === "queued") return "Waiting its turn";
  return "Working";
}
