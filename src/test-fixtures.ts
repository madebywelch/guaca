import type {
  AgentCard,
  Approval,
  ApprovalId,
  Group,
  GroupId,
  Outcome,
  WorkDecision,
} from "./lib/types";

export const DEFAULT_GROUP: GroupId = "00000000-0000-4000-8000-000000000001";

export function aGroup(over: Partial<Group> = {}): Group {
  return {
    id: DEFAULT_GROUP,
    name: "Everyone",
    agentCount: 0,
    createdAt: 0,
    inference: {
      provider: null,
      baseUrl: null,
      defaultModel: null,
      subscriptionModel: null,
      reasoningEffort: null,
      requestTimeoutSecs: null,
    },
    apiKeySet: false,
    apiKeyHint: "",
    limits: {
      maxHops: null,
      maxStepsPerRun: null,
      maxFanoutPerCall: null,
      maxSendsPerPair: null,
      maxToolRounds: null,
    },
    tidy: { on: false, hour: 3 },
    ...over,
  };
}

export function aDecision(over: Partial<WorkDecision> = {}): WorkDecision {
  return {
    id: "decision-1",
    agentId: "assistant",
    groupId: DEFAULT_GROUP,
    topic: "mail/thread/time",
    request: {
      question: "10 AM or 11 AM?",
      context: "Alex asked about tomorrow.",
      recommendation: "11 AM leaves a break.",
      options: ["10 AM", "11 AM"],
      source: "Alex email",
    },
    status: "pending",
    answer: null,
    outcome: null,
    createdAt: 1000,
    updatedAt: 1000,
    dueAt: null,
    remindAt: 2000,
    snoozedUntil: null,
    deliveryRun: null,
    interrupted: false,
    ...over,
  };
}

export function anAgent(id: string, over: Partial<AgentCard> = {}): AgentCard {
  return {
    id,
    groupId: DEFAULT_GROUP,
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    browserConsent: "open",
    name: id.charAt(0).toUpperCase() + id.slice(1),
    avatar: "plain",
    color: "#7a6a58",
    model: "m",
    subscriptionModel: "",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    discardedAt: null,
    ...over,
  };
}

export function anOutcome(over: Partial<Outcome> & Pick<Outcome, "agentId">): Outcome {
  return {
    runId: `run-${over.agentId}`,
    groupId: DEFAULT_GROUP,
    trigger: "operator",
    triggerRef: null,
    status: "done",
    headline: "Done.",
    noteId: null,
    colleagues: [],
    needs: [],
    steps: 1,
    startedAt: 0,
    finishedAt: 0,
    seenAt: null,
    ...over,
  };
}

export function anApproval(over: Partial<Approval> & Pick<Approval, "agentId">): Approval {
  return {
    id: `req-${over.agentId}` as ApprovalId,
    groupId: DEFAULT_GROUP,
    runId: `run-${over.agentId}`,
    request: { kind: "permission", action: "actOnBehalf" },
    summary: "Send an email as you",
    detail: [],
    state: "pending",
    answer: null,
    createdAt: 0,
    decidedAt: null,
    lapsesAt: null,
    ...over,
  };
}
