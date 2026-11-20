import type { IconName } from "../components/Icon";
import { BINDINGS, formatCombo, keyOf } from "./keybinds";
import type { Prefs, SurfaceMode, Typeface } from "./prefs";
import { stepped } from "./rail";
import type { SearchAction, SearchResult } from "./search";
import { SECTION_LABELS, SECTIONS, type Section } from "./sections";
import type { Activity, AgentCard, AgentId, Approval, Decision, Escalation } from "./types";

export const WRITE_WORDS = 3;

export type MenuSection = "Waiting on you" | "Agents" | "Actions" | "Settings" | "Search results";

export const MENU_SECTIONS: readonly MenuSection[] = [
  "Waiting on you",
  "Agents",
  "Actions",
  "Settings",
  "Search results",
];

export type Place =
  | "forYou"
  | "calendar"
  | "artifacts"
  | "groups"
  | "newAgent"
  | "newGroup"
  | "cafeteria";

export const PLACE_LABELS: Record<Place, string> = {
  forYou: "For you",
  calendar: "Calendar",
  artifacts: "Artifacts",
  groups: "Groups and agents",
  newAgent: "New agent",
  newGroup: "New group",
  cafeteria: "Open the cafeteria",
};

export const APP_SETTINGS = "App settings";

export function groupsLabel(count: number): string {
  return count > 1 ? PLACE_LABELS.groups : "Agents";
}

export type MenuAction =
  | { do: "write"; agentId: AgentId; text: string }
  | { do: "decide"; request: Approval; decision: Decision }
  | { do: "answer"; request: Approval; text: string }
  | { do: "open"; agentId: AgentId }
  | { do: "place"; place: Place }
  | { do: "section"; section: Section }
  | { do: "look"; patch: Partial<Prefs> }
  | { do: "result"; action: SearchAction };

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
  face?: { avatar: string; color: string; seed: string };
  meta?: string;
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
  groupCount: number;
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

const ACTIONS: { place: Place; icon: IconName; words: string }[] = [
  { place: "newAgent", icon: "plus", words: "create hire add agent" },
  { place: "newGroup", icon: "group", words: "create add group team" },
  {
    place: "forYou",
    icon: "for-you",
    words: "inbox questions results decisions requests",
  },
  { place: "calendar", icon: "calendar", words: "open dates schedule" },
  { place: "artifacts", icon: "artifact", words: "open pages documents" },
  { place: "groups", icon: "group", words: "groups agents sheet manage list" },
  { place: "cafeteria", icon: "agent", words: "hire browse" },
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
    face: { avatar: agent.avatar, color: agent.color, seed: agent.id },
    words: `agent open conversation ${agent.skills.join(" ")} ${agent.systemPrompt}`,
    action: { do: "open" as const, agentId: agent.id },
  }));
}

function actionRows(input: MenuInput): MenuRow[] {
  const rows: MenuRow[] = [];
  for (const by of [1, -1] as const) {
    const target = stepped(input.agents, input.open, by);
    if (!target || input.agents.length < 2) continue;
    const previous = by === -1;
    const combo = BINDINGS.find((binding) => binding.id === "step")?.to?.combo;
    rows.push({
      id: previous ? "agent:previous" : "agent:next",
      section: "Actions",
      title: previous ? "Previous agent" : "Next agent",
      detail: target.name,
      key: previous && combo ? formatCombo(combo) : keyOf("step"),
      words: "switch open conversation",
      action: { do: "open", agentId: target.id },
    });
  }
  for (const action of ACTIONS)
    rows.push({
      id: `place:${action.place}`,
      section: "Actions",
      title: action.place === "groups" ? groupsLabel(input.groupCount) : PLACE_LABELS[action.place],
      icon: action.icon,
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
      title: section === "general" ? APP_SETTINGS : `Settings: ${SECTION_LABELS[section]}`,
      icon: "gear",
      key:
        section === "shortcuts"
          ? formatCombo({ key: "/", mod: true })
          : section === "general"
            ? formatCombo({ key: ",", mod: true })
            : undefined,
      words: `settings preferences ${SECTION_LABELS[section]} ${SECTION_WORDS[section] ?? ""}`,
      action: { do: "section", section },
    });
  return rows;
}

function hits(row: MenuRow, words: string[]): boolean {
  const hay = `${row.title} ${row.detail ?? ""} ${row.words}`.toLowerCase();
  return words.every((word) => hay.includes(word));
}

const RESULT_ICONS: Record<SearchResult["kind"], IconName> = {
  messages: "message",
  agents: "agent",
  groups: "group",
  files: "file",
  links: "external",
  routines: "clock",
  artifacts: "artifact",
  actions: "gear",
};

export function menuRows(input: MenuInput, matches: SearchResult[] = []): MenuRow[] {
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
            face: { ...input.recipient, seed: input.recipient.id },
            words: "",
            action: { do: "write", agentId: input.recipient.id, text },
          },
        ]
      : [];
  const results: MenuRow[] = matches
    .filter((result) => result.kind !== "agents")
    .map((result) => ({
      id: result.key,
      section:
        result.kind === "actions" || result.kind === "groups" ? "Settings" : "Search results",
      title: result.title,
      detail: result.detail,
      meta: result.meta,
      face: result.face,
      icon: RESULT_ICONS[result.kind],
      words: "",
      action: { do: "result", action: result.action },
    }));
  const actions = found.filter((row) => row.section === "Actions");
  return [
    ...write,
    ...found.filter((row) => row.section === "Waiting on you" || row.section === "Agents"),
    ...actions,
    ...found.filter((row) => row.section === "Settings"),
    ...results.filter((row) => row.section === "Settings"),
    ...results.filter((row) => row.section === "Search results"),
  ];
}

export function firstChoice(rows: MenuRow[], query: string): string | null {
  const words = wordsIn(query);
  const write = rows.find((row) => row.id === "write");
  if (words.length >= WRITE_WORDS && write) return write.id;
  const others = rows.filter((row) => row.id !== "write" && row.section !== "Search results");
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
