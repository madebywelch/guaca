import { readableSize } from "./files";
import { describeTrigger, routineTitle } from "./routine";
import { relativeTime } from "./time";
import type {
  AgentCard,
  AgentId,
  ArtifactId,
  Group,
  GroupId,
  MessageHit,
  MessageId,
  Participant,
  SearchHits,
} from "./types";

export const SCOPES = [
  "all",
  "messages",
  "agents",
  "groups",
  "files",
  "links",
  "routines",
  "artifacts",
  "actions",
] as const;

export type Scope = (typeof SCOPES)[number];
export type ResultKind = Exclude<Scope, "all">;

export type SearchAction =
  | { do: "openChannel"; agentId: AgentId }
  | { do: "openMessage"; channelId: AgentId; messageId: MessageId }
  | { do: "openLink"; url: string }
  | { do: "openArtifact"; artifactId: ArtifactId }
  | { do: "editAgent"; agentId: AgentId }
  | { do: "editGroup"; groupId: GroupId }
  | { do: "openSettings" }
  | { do: "newAgent" }
  | { do: "newGroup" }
  | { do: "openCafeteria" };

export interface SearchResult {
  key: string;
  kind: ResultKind;
  title: string;
  detail: string;
  meta: string;
  score: number;
  action: SearchAction;
  face?: { avatar: string; color: string; seed: string };
}

export interface SearchInput {
  query: string;
  agents: AgentCard[];
  groups: Group[];
  hits: SearchHits | null;
  lastActive: Record<AgentId, number>;
  now: number;
}

const NEUTRAL = 10;

const PRIORITY: Record<ResultKind, number> = {
  agents: 8,
  groups: 7,
  artifacts: 6,
  actions: 5,
  messages: 4,
  files: 3,
  routines: 2,
  links: 1,
};

export function score(text: string, query: string): number {
  const needle = query.trim().toLowerCase();
  if (!needle) return NEUTRAL;

  const hay = text.toLowerCase();
  const at = hay.indexOf(needle);
  if (at < 0) return 0;

  const base =
    hay === needle ? 100 : at === 0 ? 80 : /[\s\-_/.,:]/.test(hay[at - 1] ?? "") ? 60 : 40;

  return base + Math.round(9 * (needle.length / Math.max(needle.length, hay.length)));
}

function bestOf(query: string, fields: [text: string, weight: number][]): number {
  return fields.reduce((best, [text, weight]) => Math.max(best, score(text, query) * weight), 0);
}

function firstLine(text: string, cap = 120): string {
  const line = text.replace(/\s+/g, " ").trim();
  return line.length > cap ? `${line.slice(0, cap - 1)}…` : line;
}

export function shortUrl(url: string): string {
  return url.replace(/^https?:\/\/(www\.)?/, "").replace(/\/$/, "");
}

export function searchResults(input: SearchInput): SearchResult[] {
  const { query, agents, groups, hits, lastActive, now } = input;

  const nameOf = (who: Participant): string => {
    if (who.kind === "human") return "You";
    if (who.kind === "system") return "Guaca";
    return agents.find((a) => a.id === who.id)?.name ?? "Deleted agent";
  };
  const cardOf = (id: AgentId) => agents.find((a) => a.id === id);

  const results: (SearchResult & { at: number })[] = [];

  for (const agent of agents) {
    if (agent.lifecycle === "terminated") continue;
    const value = bestOf(query, [
      [agent.name, 1],
      [agent.skills.join(" "), 0.6],
      [agent.systemPrompt, 0.3],
    ]);
    if (value <= 0) continue;
    results.push({
      key: `agent:${agent.id}`,
      kind: "agents",
      title: agent.name,
      detail: firstLine(agent.systemPrompt),
      meta: agent.lifecycle === "paused" ? "Paused" : "Agent",
      score: value,
      at: lastActive[agent.id] ?? agent.createdAt,
      action: { do: "openChannel", agentId: agent.id },
      face: { avatar: agent.avatar, color: agent.color, seed: agent.id },
    });
  }

  for (const group of groups) {
    const value = score(group.name, query);
    if (value <= 0) continue;
    results.push({
      key: `group:${group.id}`,
      kind: "groups",
      title: group.name,
      detail: `${group.agentCount} ${group.agentCount === 1 ? "agent" : "agents"}`,
      meta: "Group",
      score: value,
      at: group.createdAt,
      action: { do: "editGroup", groupId: group.id },
    });
  }

  for (const door of actionsFor(agents, groups)) {
    const value = score(door.title, query);
    if (value <= 0) continue;
    results.push({ ...door, score: value, at: 0 });
  }

  for (const hit of hits?.messages ?? []) {
    results.push({
      key: `message:${hit.id}`,
      kind: "messages",
      title: `${nameOf(hit.from)} → ${nameOf(hit.to)}`,
      detail: hit.excerpt,
      meta: relativeTime(hit.createdAt, now),
      score: score(hit.excerpt, query),
      at: hit.createdAt,
      action: { do: "openMessage", channelId: hit.channelId, messageId: hit.id },
      face: faceFor(hit, cardOf),
    });
  }

  for (const hit of hits?.files ?? []) {
    results.push({
      key: `file:${hit.file.digest}`,
      kind: "files",
      title: hit.file.name,
      detail: `from ${nameOf(hit.from)}`,
      meta: readableSize(hit.file.bytes),
      score: score(hit.file.name, query),
      at: hit.createdAt,
      action: { do: "openMessage", channelId: hit.channelId, messageId: hit.messageId },
    });
  }

  for (const hit of hits?.links ?? []) {
    results.push({
      key: `link:${hit.url}`,
      kind: "links",
      title: shortUrl(hit.url),
      detail: "",
      meta: relativeTime(hit.createdAt, now),
      score: score(hit.url, query),
      at: hit.createdAt,
      action: { do: "openLink", url: hit.url },
    });
  }

  for (const routine of hits?.routines ?? []) {
    results.push({
      key: `routine:${routine.id}`,
      kind: "routines",
      title: routineTitle(routine),
      detail: cardOf(routine.agentId)?.name ?? "Deleted agent",
      meta: describeTrigger(routine.trigger, routine.nextRunAt),
      score: Math.max(score(routine.name, query), score(routine.what, query)),
      at: routine.nextRunAt ?? routine.createdAt,
      action: { do: "openChannel", agentId: routine.agentId },
    });
  }

  for (const artifact of hits?.artifacts ?? []) {
    results.push({
      key: `artifact:${artifact.id}`,
      kind: "artifacts",
      title: artifact.title,
      detail: groups.find((group) => group.id === artifact.groupId)?.name ?? "",
      meta: relativeTime(artifact.updatedAt, now),
      score: score(artifact.title, query),
      at: artifact.updatedAt,
      action: { do: "openArtifact", artifactId: artifact.id },
    });
  }

  return results
    .sort(
      (a, b) =>
        b.score - a.score ||
        PRIORITY[b.kind] - PRIORITY[a.kind] ||
        b.at - a.at ||
        a.title.localeCompare(b.title),
    )
    .map(({ at: _at, ...result }) => result);
}

function faceFor(
  hit: MessageHit,
  cardOf: (id: AgentId) => AgentCard | undefined,
): SearchResult["face"] {
  const id = hit.from.kind === "agent" ? hit.from.id : hit.channelId;
  const card = cardOf(id);
  return card ? { avatar: card.avatar, color: card.color, seed: card.id } : undefined;
}

function actionsFor(agents: AgentCard[], groups: Group[]): Omit<SearchResult, "score">[] {
  const out: Omit<SearchResult, "score">[] = [];

  for (const agent of agents) {
    if (agent.lifecycle === "terminated") continue;
    out.push({
      key: `action:agent-settings:${agent.id}`,
      kind: "actions",
      title: `${agent.name} settings`,
      detail: "Prompt, model, skills, memory, routines",
      meta: "Action",
      action: { do: "editAgent", agentId: agent.id },
      face: { avatar: agent.avatar, color: agent.color, seed: agent.id },
    });
  }

  for (const group of groups) {
    out.push({
      key: `action:group-settings:${group.id}`,
      kind: "actions",
      title: `${group.name} settings`,
      detail: "Endpoint, model, limits, and who gets which connector",
      meta: "Action",
      action: { do: "editGroup", groupId: group.id },
    });
  }

  out.push(
    {
      key: "action:app-settings",
      kind: "actions",
      title: "App settings",
      detail: "API key, model, limits, computers, appearance, notifications, shortcuts",
      meta: "Action",
      action: { do: "openSettings" },
    },
    {
      key: "action:new-agent",
      kind: "actions",
      title: "New agent",
      detail: "Add an agent",
      meta: "Action",
      action: { do: "newAgent" },
    },
    {
      key: "action:new-group",
      kind: "actions",
      title: "New group",
      detail: "A group that cannot see the others",
      meta: "Action",
      action: { do: "newGroup" },
    },
    {
      key: "action:cafeteria",
      kind: "actions",
      title: "Cafeteria",
      detail: "Hire agents that are already set up",
      meta: "Action",
      action: { do: "openCafeteria" },
    },
  );

  return out;
}

export function inScope(results: SearchResult[], scope: Scope): SearchResult[] {
  return scope === "all" ? results : results.filter((r) => r.kind === scope);
}

export function scopeLabel(scope: Scope): string {
  return scope === "all" ? "All" : scope[0]!.toUpperCase() + scope.slice(1);
}
