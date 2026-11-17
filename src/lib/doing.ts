import { railMeta } from "./outcomes";
import { relativeTime } from "./time";
import { type LiveCall, machineInUse } from "./trail";
import type { Activity, AgentId, Escalation, Outcome } from "./types";

export interface Doing {
  text: string;
  kind: "thinking" | "queued" | "asking" | "paused" | "finished" | undefined;
  title?: string;
}

export interface DoingInputs {
  activity: Record<AgentId, Activity>;
  stuck: Escalation[];
  trail: Record<AgentId, LiveCall[] | undefined>;
  lastActive: Record<AgentId, number>;
  finished: Record<AgentId, Outcome | undefined>;
}

export function doing(id: AgentId, inputs: DoingInputs, now: number): Doing {
  const state = inputs.activity[id];

  if (state?.state !== "awaitingApproval") {
    const open = inputs.stuck.find((one) => one.agentId === id);
    if (open) {
      return { text: `stuck ${relativeTime(open.raisedAt, now)}`, kind: "asking" };
    }
  }

  switch (state?.state) {
    case "thinking":
      switch (machineInUse(inputs.trail[id])) {
        case "computer":
          return { text: "on its computer", kind: "thinking" };
        case "browser":
          return { text: "in its browser", kind: "thinking" };
        default:
          return { text: "typing", kind: "thinking" };
      }
    case "queued":
      return { text: `${state.depth} queued`, kind: "queued" };
    case "awaitingApproval":
      return { text: "needs you", kind: "asking" };
    case "paused":
      return { text: "paused", kind: "paused" };
    default: {
      const news = inputs.finished[id];
      if (news) return { text: railMeta(news, now), kind: "finished", title: news.headline };
      const at = inputs.lastActive[id];
      return { text: at ? relativeTime(at, now) : "", kind: undefined };
    }
  }
}
