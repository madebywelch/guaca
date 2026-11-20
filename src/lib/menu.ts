import type { IconName } from "../components/Icon";
import { formatCombo } from "./keybinds";
import type { Prefs, SurfaceMode, Typeface } from "./prefs";
import { SECTION_LABELS, SECTIONS, type Section } from "./sections";
import type { Activity, AgentCard, AgentId, Approval, Decision, Escalation } from "./types";

export const WRITE_WORDS = 3;

export type MenuSection = "Waiting on you" | "Agents" | "Actions" | "Settings";

export const MENU_SECTIONS: readonly MenuSection[] = [
  "Waiting on you",
  "Agents",
  "Actions",
  "Settings",
];

export type Place =
  | "today"
  | "search"
  | "forYou"
  | "calendar"
  | "artifacts"
  | "groups"
  | "newAgent"
  | "newGroup"
  | "cafeteria";

export type MenuAction =
  | { do: "write"; agentId: AgentId; text: string }
  | { do: "decide"; request: Approval; decision: Decision }
  | { do: "answer"; request: Approval; text: string }
  | { do: "open"; agentId: AgentId }
  | { do: "place"; place: Place; query?: string }
  | { do: "section"; section: Section }
  | { do: "look"; patch: Partial<Prefs> };

export interface Asking {
  id: string;
  agent: AgentCard | undefined;
  who: string;
  text: string;
}

export interface MenuRow {
  id: string;
  section: MenuSection | null;
  asking?: Asking;
  title: string;
  detail?: string;
  key?: string;
  face?: AgentCard;
  icon?: IconName;
  words: string;
  request?: string;
  action: MenuAction;
}

export interface MenuInput {
  query: string;
  recipient: AgentCard | undefined;
  agents: AgentCard[];
  activity: Record<AgentId, Activity>;
  pending: Approval[];
  stuck: Escalation[];
  open: AgentId | null;
  prefs: Prefs;
}

export function wordsIn(query: string): string[] {
  return query.trim().toLowerCase().split(/\s+/).filter(Boolean);
}

export function stateWord(agent: AgentCard, activity: Activity | undefined): string {
  if (agent.lifecycle === "paused" || activity?.state === "paused") return "paused";
  switch (activity?.state) {
    case "awaitingApproval":
      return "needs you";
    case "thinking":
      return "working";
    case "queued":
      return "queued";
    default:
      return "idle";
  }
}

const SURFACE_WORDS: Record<SurfaceMode, [string, string]> = {
  light: ["Light", "day bright white"],
  dark: ["Dark", "night black dim"],
  system: ["Match system", "auto automatic os"],
};

const TYPEFACE_WORDS: Record<Typeface, [string, string]> = {
  inter: ["Inter", "sans"],
  system: ["System", "san francisco sf"],
  legible: ["Atkinson Hyperlegible", "readable legible dyslexia accessible"],
};

const SECTION_WORDS: Partial<Record<Section, string>> = {
  appearance: "theme look color colour dark light font typeface size",
  provider: "model api key openai anthropic openrouter endpoint",
  notifications: "alerts banner sound push phone quiet",
  shortcuts: "keys keyboard hotkeys bindings",
  permissions: "approve allow deny rules",
  limits: "budget spend cost hops",
  workspace: "host server daemon update",
};

const ACTIONS: { place: Place; title: string; icon: IconName; words: string; key?: string }[] = [
  { place: "newAgent", title: "New agent", icon: "plus", words: "create hire add agent" },
  { place: "newGroup", title: "New group", icon: "group", words: "create add group team" },
  {
    place: "search",
    title: "Search messages",
    icon: "search",
    words: "find history look up",
    key: formatCombo({ key: "f", mod: true }),
  },
  {
    place: "forYou",
    title: "Open For you",
    icon: "for-you",
    words: "inbox questions results decisions requests",
  },
  { place: "calendar", title: "Open the calendar", icon: "calendar", words: "dates schedule" },
  { place: "artifacts", title: "Open artifacts", icon: "artifact", words: "pages documents" },
  { place: "groups", title: "Groups and agents", icon: "group", words: "sheet manage list" },
  { place: "cafeteria", title: "Open the cafeteria", icon: "agent", words: "hire browse" },
];

function waitingRows(input: MenuInput): MenuRow[] {
  const named = (id: AgentId) =>
    input.agents.find((agent) => agent.id === id)?.name ?? "A deleted agent";
  const rows: MenuRow[] = [];
  const faceOf = (id: AgentId) => input.agents.find((agent) => agent.id === id);
  for (const request of input.pending) {
    const asker = named(request.agentId);
    const detail = `${asker} ${request.summary}`;
    const base = {
      section: "Waiting on you" as const,
      request: request.id,
      asking: { id: request.id, agent: faceOf(request.agentId), who: asker, text: request.summary },
    };
    if (request.request.kind === "permission") {
      rows.push({
        ...base,
        id: `${request.id}:allow`,
        title: "Allow",
        icon: "check",
        words: `allow approve yes accept ${detail}`,
        action: { do: "decide", request, decision: "allow" },
      });
      if (request.request.action === "createAgent")
        rows.push({
          ...base,
          id: `${request.id}:always`,
          title: "Always allow",
          icon: "check",
          words: `always allow approve ${detail}`,
          action: { do: "decide", request, decision: "alwaysAllow" },
        });
      rows.push({
        ...base,
        id: `${request.id}:deny`,
        title: "Deny",
        icon: "close",
        words: `deny reject decline no ${detail}`,
        action: { do: "decide", request, decision: "deny" },
      });
      continue;
    }
    if (request.request.options.length === 0) {
      rows.push({
        ...base,
        id: `${request.id}:open`,
        title: "Answer in the conversation",
        icon: "message",
        words: `answer reply question ${detail}`,
        action: { do: "open", agentId: request.agentId },
      });
      continue;
    }
    request.request.options.forEach((option, index) => {
      rows.push({
        ...base,
        id: `${request.id}:${index}`,
        title: option,
        icon: "message",
        words: `answer ${detail}`,
        action: { do: "answer", request, text: option },
      });
    });
  }
  for (const one of input.stuck) {
    const asker = named(one.agentId);
    rows.push({
      id: `${one.id}:open`,
      section: "Waiting on you",
      asking: { id: one.id, agent: faceOf(one.agentId), who: asker, text: one.summary },
      title: "Open the conversation",
      icon: "alert",
      words: `stuck help ${asker} ${one.summary}`,
      action: { do: "open", agentId: one.agentId },
    });
  }
  return rows;
}

function agentRows(input: MenuInput): MenuRow[] {
  return input.agents.map((agent, index) => ({
    id: `agent:${agent.id}`,
    section: "Agents" as const,
    title: agent.name,
    detail: stateWord(agent, input.activity[agent.id]),
    key: index < 9 ? formatCombo({ key: String(index + 1), mod: true }) : undefined,
    face: agent,
    words: "agent open conversation",
    action: { do: "open" as const, agentId: agent.id },
  }));
}

function actionRows(input: MenuInput): MenuRow[] {
  const rows: MenuRow[] = [];
  if (input.open !== null)
    rows.push({
      id: "place:today",
      section: "Actions",
      title: "Back to Today",
      icon: "chevron-left",
      words: "home today back",
      action: { do: "place", place: "today" },
    });
  for (const action of ACTIONS)
    rows.push({
      id: `place:${action.place}`,
      section: "Actions",
      title: action.title,
      icon: action.icon,
      key: action.key,
      words: action.words,
      action: { do: "place", place: action.place },
    });
  return rows;
}

function settingsRows(input: MenuInput): MenuRow[] {
  const rows: MenuRow[] = [];
  for (const [mode, [label, words]] of Object.entries(SURFACE_WORDS) as [
    SurfaceMode,
    [string, string],
  ][])
    rows.push({
      id: `surface:${mode}`,
      section: "Settings",
      title: `Reading surface: ${label}`,
      detail: input.prefs.surface === mode ? "Current" : undefined,
      icon: "gear",
      words: `appearance theme mode surface ${words}`,
      action: { do: "look", patch: { surface: mode } },
    });
  for (const [face, [label, words]] of Object.entries(TYPEFACE_WORDS) as [
    Typeface,
    [string, string],
  ][])
    rows.push({
      id: `typeface:${face}`,
      section: "Settings",
      title: `Typeface: ${label}`,
      detail: input.prefs.typeface === face ? "Current" : undefined,
      icon: "gear",
      words: `appearance font type ${words}`,
      action: { do: "look", patch: { typeface: face } },
    });
  for (const section of SECTIONS)
    rows.push({
      id: `section:${section}`,
      section: "Settings",
      title: `Settings: ${SECTION_LABELS[section]}`,
      icon: "gear",
      key:
        section === "shortcuts"
          ? formatCombo({ key: "/", mod: true })
          : section === "general"
            ? formatCombo({ key: ",", mod: true })
            : undefined,
      words: `settings preferences ${SECTION_WORDS[section] ?? ""}`,
      action: { do: "section", section },
    });
  return rows;
}

function hits(row: MenuRow, words: string[]): boolean {
  const hay = `${row.title} ${row.detail ?? ""} ${row.words}`.toLowerCase();
  return words.every((word) => hay.includes(word));
}

export function menuRows(input: MenuInput): MenuRow[] {
  const words = wordsIn(input.query);
  const text = input.query.trim();
  const found = [
    ...waitingRows(input),
    ...agentRows(input),
    ...actionRows(input),
    ...settingsRows(input),
  ].filter((row) => hits(row, words));
  const write: MenuRow[] =
    text && input.recipient
      ? [
          {
            id: "write",
            section: null,
            title: `Write to ${input.recipient.name}`,
            detail: text,
            face: input.recipient,
            words: "",
            action: { do: "write", agentId: input.recipient.id, text },
          },
        ]
      : [];
  const search: MenuRow[] = text
    ? [
        {
          id: "search",
          section: "Actions",
          title: `Search for “${text}”`,
          icon: "search",
          words: "",
          action: { do: "place", place: "search", query: text },
        },
      ]
    : [];
  const actions = found.filter((row) => row.section === "Actions");
  return [
    ...write,
    ...found.filter((row) => row.section === "Waiting on you" || row.section === "Agents"),
    ...actions,
    ...search,
    ...found.filter((row) => row.section === "Settings"),
  ];
}

export function firstChoice(rows: MenuRow[], query: string): string | null {
  const words = wordsIn(query);
  const write = rows.find((row) => row.id === "write");
  if (words.length >= WRITE_WORDS && write) return write.id;
  const others = rows.filter((row) => row.id !== "write" && row.id !== "search");
  if (words.length === 0) return others[0]?.id ?? rows[0]?.id ?? null;
  const named = others.find((row) => wordsIn(row.title).join(" ") === words.join(" "));
  const titled = others.find((row) =>
    words.every((word) => row.title.toLowerCase().includes(word)),
  );
  return (named ?? titled ?? others[0] ?? write ?? rows[0])?.id ?? null;
}

export function requestAt(rows: MenuRow[], selected: string | null): string | undefined {
  const chosen = rows.find((row) => row.id === selected)?.request;
  return chosen ?? rows.find((row) => row.request !== undefined)?.request;
}

export function nextRecipient(
  agents: AgentCard[],
  current: AgentCard | undefined,
  step: 1 | -1,
): AgentCard | undefined {
  if (agents.length === 0) return undefined;
  const at = current ? agents.findIndex((agent) => agent.id === current.id) : -1;
  return agents[(at + step + agents.length) % agents.length];
}
