import { workspaceOrigin } from "./transport";
import type { Provider, ReasoningEffort } from "./types";

const ASK_ONLY_IF_STUCK = "Ask me a question only if you cannot go on without one.";
const BOTTOM_LINE =
  'When the work is done, end with one last line that starts "Bottom line:" and gives the answer in one sentence.';

export const FIRST_TASKS = [
  {
    id: "research",
    title: "Research a decision",
    groupName: "Research",
    description: "Compare options and make the tradeoffs clear.",
    group: [
      {
        agent: "financial-analyst",
        role: "Works out what each option costs and comes back with one recommendation.",
      },
    ],
    brief: `Help me decide whether to buy or lease my next car. It costs $35,000 and should be worth about $20,000 in three years. I can borrow at 6% over five years with nothing down, or lease it for three years at $500 a month with $3,000 down. Work out what each costs me over the three years, leaving out taxes, fees and what I would pay either way, such as insurance. Show the arithmetic, state the loan still owed at the end of the three years, and count it against what the car is worth then. Tell me which you would pick on those numbers and why, and say what would change the answer. Where you need a figure I have not given, assume a typical one and say so. ${BOTTOM_LINE} ${ASK_ONLY_IF_STUCK}`,
    crew: {
      group: [
        {
          agent: "chief-of-staff",
          role: "Asks both colleagues at once and brings back one recommendation.",
        },
        { agent: "market-researcher", role: "Reads what current sources say about each option." },
      ],
      offer:
        "For a deeper pass, Chief of Staff can ask Market Researcher what current sources say, check it with Financial Analyst, and bring back one recommendation.",
      brief: `Take a deeper pass at what I asked Financial Analyst, which has answered once: "{brief}" Ask Financial Analyst and Market Researcher at the same time: Financial Analyst to test its answer and say what would change it, Market Researcher for what current sources say about each option. When both have answered, come back to me with one recommendation that says where they disagreed. ${ASK_ONLY_IF_STUCK}`,
    },
  },
  {
    id: "plan",
    title: "Plan a project",
    groupName: "Planning",
    description: "Turn an idea into a manageable first step.",
    group: [
      {
        agent: "product-manager",
        role: "Finds the smallest plan that would work, its first step, and its risks.",
      },
    ],
    brief: `Help me plan a neighborhood cleanup day for about 30 volunteers next month. Give me the smallest plan that would work, its first step, and the risks worth fixing first. Where you need a detail I have not given, such as the place or the date, assume a typical one and say so. ${BOTTOM_LINE} ${ASK_ONLY_IF_STUCK}`,
    crew: {
      group: [
        {
          agent: "chief-of-staff",
          role: "Asks both colleagues at once and brings back one plan.",
        },
        { agent: "qa-tester", role: "Looks for the ways that plan could go wrong." },
      ],
      offer:
        "For a deeper pass, Chief of Staff can ask QA Tester how this plan could go wrong, have Product Manager tighten it, and bring back one plan.",
      brief: `Take a deeper pass at what I asked Product Manager, which has answered once: "{brief}" Ask Product Manager and QA Tester at the same time: Product Manager to tighten its plan and first step, QA Tester for the ways that plan could go wrong. When both have answered, come back to me with one plan, its first step, and the risks worth fixing first. ${ASK_ONLY_IF_STUCK}`,
    },
  },
  {
    id: "write",
    title: "Draft content",
    groupName: "Writing",
    description: "Find the words for something you need to say.",
    group: [{ agent: "content-marketer", role: "Writes a warm draft you can send." }],
    brief: `Help me write a short note telling my team that our weekly meeting moves from Monday to Thursday morning, starting next week. Keep it warm and friendly, and give me one draft I can send. Say nothing I have not told you: for a time, a place, a reason or anything else I did not give, write a placeholder in brackets such as [time]. ${BOTTOM_LINE} ${ASK_ONLY_IF_STUCK}`,
    crew: {
      group: [
        {
          agent: "chief-of-staff",
          role: "Asks both colleagues at once and brings back one draft.",
        },
        {
          agent: "technical-writer",
          role: "Checks it says what people need to know, and nothing unclear.",
        },
      ],
      offer:
        "For a deeper pass, Chief of Staff can ask Technical Writer what reads as unclear, have Content Marketer polish the draft, and bring back one you can send.",
      brief: `Take a deeper pass at what I asked Content Marketer, which has answered once: "{brief}" Ask Content Marketer and Technical Writer at the same time: Content Marketer to polish its draft, Technical Writer for the facts people need and anything that reads as unclear. When both have answered, come back to me with one draft I can send. ${ASK_ONLY_IF_STUCK}`,
    },
  },
  {
    id: "own",
    title: "Something else",
    groupName: "My group",
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

export interface HeldProvider {
  provider: Provider;
  baseUrl: string;
  model: string;
  apiKey: string;
  subscriptionModel: string;
  effort: ReasoningEffort;
}

export interface HeldStart {
  task: FirstTask["task"];
  brief: string;
  step: "task" | "provider" | "start";
  provider: HeldProvider | null;
  chosen: boolean;
  queued: boolean;
  focus: { hold: string; at: number } | null;
}

let held: HeldStart | null = null;

export function heldStart(): HeldStart | null {
  return held;
}

export function holdStart(value: HeldStart | null): void {
  held = value;
}

export function waitsForHost(provider: Provider): boolean {
  return provider !== "compatible";
}

type Crew = Extract<(typeof FIRST_TASKS)[number], { crew: unknown }>["crew"];

export function crewFor(task: FirstTask["task"]): Crew | null {
  const entry = FIRST_TASKS.find((one) => one.id === task);
  return entry && "crew" in entry ? entry.crew : null;
}

export function named(text: string, names: ReadonlyMap<string, string>): string {
  let out = text;
  for (const [preset, name] of names) if (preset !== name) out = out.replaceAll(preset, name);
  return out;
}

export interface CrewOffer {
  task: FirstTask["task"];
  agentId: string;
  brief: string;
  key?: string;
}

export function readCrewOffer(): CrewOffer | null {
  try {
    const value = JSON.parse(
      localStorage.getItem(`guaca.crewOffer.${workspaceOrigin()}`) ?? "null",
    );
    if (
      !value ||
      !crewFor(value.task) ||
      typeof value.agentId !== "string" ||
      typeof value.brief !== "string"
    )
      return null;
    return {
      task: value.task,
      agentId: value.agentId,
      brief: value.brief,
      ...(typeof value.key === "string" && value.key ? { key: value.key } : {}),
    };
  } catch {
    return null;
  }
}

export function saveCrewOffer(value: CrewOffer | null): void {
  const key = `guaca.crewOffer.${workspaceOrigin()}`;
  if (value) localStorage.setItem(key, JSON.stringify(value));
  else localStorage.removeItem(key);
}
