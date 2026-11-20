import { workspaceOrigin } from "./transport";

const ASK_ONLY_IF_STUCK = "Ask me a question only if you cannot go on without one.";

export const FIRST_TASKS = [
  {
    id: "research",
    title: "Research a decision",
    description: "Compare options and make the tradeoffs clear.",
    group: [
      {
        agent: "chief-of-staff",
        role: "Takes your brief, asks both colleagues at once, and brings back one recommendation.",
      },
      { agent: "market-researcher", role: "Reads what current sources say about each option." },
      { agent: "financial-analyst", role: "Works out the cost and what would change the answer." },
    ],
    brief: `Help me decide whether to buy or lease my next car. Ask Market Researcher and Financial Analyst at the same time: Market Researcher for what current sources say about each option, Financial Analyst for the cost over three years and what would change the answer. When both have answered, come back to me with one recommendation that says where they disagreed. ${ASK_ONLY_IF_STUCK}`,
  },
  {
    id: "plan",
    title: "Plan a project",
    description: "Turn an idea into a manageable first step.",
    group: [
      {
        agent: "chief-of-staff",
        role: "Takes your brief, asks both colleagues at once, and brings back one plan.",
      },
      {
        agent: "product-manager",
        role: "Finds the smallest plan that would work, and its first step.",
      },
      { agent: "qa-tester", role: "Looks for the ways that plan could go wrong." },
    ],
    brief: `Help me plan a neighborhood cleanup day for about 30 volunteers next month. Ask Product Manager and QA Tester at the same time: Product Manager for the smallest plan that would work and its first step, QA Tester for the ways that plan could go wrong. When both have answered, come back to me with one plan, its first step, and the risks worth fixing first. ${ASK_ONLY_IF_STUCK}`,
  },
  {
    id: "write",
    title: "Draft content",
    description: "Find the words for something you need to say.",
    group: [
      {
        agent: "chief-of-staff",
        role: "Takes your brief, asks both colleagues at once, and brings back one draft.",
      },
      { agent: "content-marketer", role: "Writes a warm first draft." },
      {
        agent: "technical-writer",
        role: "Checks it says what people need to know, and nothing unclear.",
      },
    ],
    brief: `Help me write a short note telling my team that our weekly meeting moves from Monday to Thursday morning, starting next week. Ask Content Marketer and Technical Writer at the same time: Content Marketer for a warm, friendly draft, Technical Writer for the facts people need and anything that reads as unclear. When both have answered, come back to me with one draft I can send. ${ASK_ONLY_IF_STUCK}`,
  },
  {
    id: "own",
    title: "Something else",
    description: "Start with a task in your own words.",
    group: [
      {
        agent: "chief-of-staff",
        role: "Takes your brief, splits it between both colleagues, and brings back one result.",
      },
      { agent: "market-researcher", role: "Finds out what is true, and says where it read it." },
      { agent: "product-manager", role: "Turns it into a plan with a first step." },
    ],
    brief: "",
  },
] as const;

export interface FirstTask {
  task: (typeof FIRST_TASKS)[number]["id"];
  brief: string;
  agentId: string | null;
  sent: boolean;
  key?: string;
}

export function readFirstTask(): FirstTask {
  const empty: FirstTask = { task: "own", brief: "", agentId: null, sent: false };
  try {
    const value = JSON.parse(
      localStorage.getItem(`guaca.firstGroup.${workspaceOrigin()}`) ?? "null",
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
      ...(typeof value.key === "string" && value.key ? { key: value.key } : {}),
    };
  } catch {
    return empty;
  }
}

export function saveFirstTask(value: FirstTask | null): void {
  const key = `guaca.firstGroup.${workspaceOrigin()}`;
  if (value) localStorage.setItem(key, JSON.stringify(value));
  else localStorage.removeItem(key);
}
