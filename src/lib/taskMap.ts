import type { AgentId, Approval, Envelope, RunId } from "./types";

export interface TaskBranch {
  agentId: AgentId;
  asked: Envelope;
  reply: Envelope | null;
  questions: Approval[];
  delegatedTo: AgentId[];
}

export interface TaskMap {
  runId: RunId;
  brief: Envelope;
  inFlight: boolean;
  branches: TaskBranch[];
}

export interface MapRows {
  room: number;
  root: number;
  beside: boolean;
  gap: number;
  branches: readonly number[];
}

export function fitRows({ room, root, beside, gap, branches }: MapRows) {
  const count = (space: number) => {
    let used = -gap;
    let shown = 0;
    for (const height of branches) {
      used += gap + height;
      if (used > space + 0.5) break;
      shown += 1;
    }
    return shown;
  };
  if (beside) return { shown: count(room), root: root <= room + 0.5 };
  const withRoot = count(room - root);
  const without = count(room);
  return without > withRoot ? { shown: without, root: false } : { shown: withRoot, root: true };
}

const due = (question: Approval) => question.lapsesAt ?? Infinity;

export function taskMaps(
  lead: AgentId,
  messages: readonly Envelope[],
  questions: readonly Approval[],
  running: ReadonlySet<RunId>,
): TaskMap[] {
  const runs = new Map<RunId, Envelope[]>();
  for (const message of messages) {
    const run = runs.get(message.runId);
    if (run) run.push(message);
    else runs.set(message.runId, [message]);
  }
  return [...runs]
    .flatMap(([runId, run]) => {
      run.sort((a, b) => a.createdAt - b.createdAt);
      const [brief] = run;
      if (brief?.from.kind !== "human" || brief.to.kind !== "agent" || brief.to.id !== lead)
        return [];
      const branches = new Map<AgentId, TaskBranch>();
      for (const message of run) {
        const { from, to } = message;
        if (from.kind !== "agent" || to.kind !== "agent") continue;
        if (from.id === lead && to.id !== lead && message.intent === "work") {
          if (!branches.has(to.id)) {
            branches.set(to.id, {
              agentId: to.id,
              asked: message,
              reply: null,
              questions: [],
              delegatedTo: [],
            });
          }
        } else {
          const branch = branches.get(from.id);
          if (!branch) continue;
          if (
            to.id === lead &&
            message.intent === "courtesy" &&
            !message.expectsReply &&
            message.parts.length > 0 &&
            message.parts.every((part) => part.type === "text" || part.type === "file")
          ) {
            branch.reply = message;
          } else if (
            to.id !== lead &&
            message.intent === "work" &&
            !branch.delegatedTo.includes(to.id)
          )
            branch.delegatedTo.push(to.id);
        }
      }
      if (!branches.size) return [];
      for (const question of questions) {
        if (
          question.runId !== runId ||
          question.state !== "pending" ||
          question.request.kind !== "question"
        )
          continue;
        const branch =
          branches.get(question.agentId) ??
          [...branches.values()].find((one) => one.delegatedTo.includes(question.agentId));
        branch?.questions.push(question);
      }
      for (const branch of branches.values()) {
        branch.questions.sort((a, b) => due(a) - due(b) || a.createdAt - b.createdAt);
      }
      return [
        {
          runId,
          brief,
          inFlight: running.has(runId),
          branches: [...branches.values()].sort((a, b) => {
            const x = a.questions[0];
            const y = b.questions[0];
            if (x && y) return due(x) - due(y) || x.createdAt - y.createdAt;
            if (x || y) return x ? -1 : 1;
            return Number(a.reply !== null) - Number(b.reply !== null);
          }),
        },
      ];
    })
    .sort((a, b) => b.brief.createdAt - a.brief.createdAt);
}
