import { workspaceOrigin } from "./transport";

export const FIRST_TASKS = [
  {
    id: "research",
    title: "Research a decision",
    description: "Compare options and make the tradeoffs clear.",
    agent: "market-researcher",
    brief:
      "Help me compare my options for a decision. Ask me what I am choosing between and what matters most, then help me weigh the tradeoffs.",
  },
  {
    id: "plan",
    title: "Plan a project",
    description: "Turn an idea into a manageable first step.",
    agent: "product-manager",
    brief:
      "Help me turn a project idea into a practical plan. Ask me about the goal, the deadline, and the constraints, then suggest the smallest useful first step.",
  },
  {
    id: "write",
    title: "Draft content",
    description: "Find the words for something you need to say.",
    agent: "content-marketer",
    brief:
      "Help me write a first draft. Ask me who it is for, what I want to say, and the tone I am aiming for before you start writing.",
  },
  {
    id: "own",
    title: "Something else",
    description: "Start with a task in your own words.",
    agent: "executive-assistant",
    brief: "",
  },
] as const;

export interface FirstTask {
  task: (typeof FIRST_TASKS)[number]["id"];
  brief: string;
  agentId: string | null;
  sent: boolean;
}

export function readFirstTask(): FirstTask {
  const empty: FirstTask = { task: "own", brief: "", agentId: null, sent: false };
  try {
    const value = JSON.parse(
      localStorage.getItem(`guaca.firstTask.${workspaceOrigin()}`) ?? "null",
    );
    if (
      !value ||
      !FIRST_TASKS.some((task) => task.id === value.task) ||
      typeof value.brief !== "string"
    )
      return empty;
    return {
      task: value.task,
      brief: value.brief,
      agentId: typeof value.agentId === "string" ? value.agentId : null,
      sent: value.sent === true,
    };
  } catch {
    return empty;
  }
}

export function saveFirstTask(value: FirstTask | null): void {
  const key = `guaca.firstTask.${workspaceOrigin()}`;
  if (value) localStorage.setItem(key, JSON.stringify(value));
  else localStorage.removeItem(key);
}
