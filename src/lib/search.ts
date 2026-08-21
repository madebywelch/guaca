/**
 * One ordered list out of two halves.
 *
 * The workspace is searched in two places, because it is held in two places.
 * Messages, files, links and routines live in SQLite and are matched there;
 * agents and groups are already in this store to draw the rail, and actions are
 * not stored anywhere at all. What must not be split is the ranking: a list
 * where an agent and a message are ordered by different rules is a list the
 * operator has to read twice. So both halves arrive here as raw matches and
 * everything is scored by the same function.
 */

import { readableSize } from "./files";
import { describeTrigger, routineTitle } from "./routine";
import { relativeTime } from "./time";
import type {
  AgentCard,
  AgentId,
  Group,
  GroupId,
  MessageHit,
  MessageId,
  Participant,
  SearchHits,
} from "./types";

/** The tabs, in the order they are drawn. */
export const SCOPES = [
  "all",
  "messages",
  "agents",
  "groups",
  "files",
  "links",
  "routines",
  "actions",
] as const;

export type Scope = (typeof SCOPES)[number];
export type ResultKind = Exclude<Scope, "all">;

/** What choosing a result does. Carried out by the palette, not here. */
export type SearchAction =
  | { do: "openChannel"; agentId: AgentId }
  | { do: "openMessage"; channelId: AgentId; messageId: MessageId }
  | { do: "openLink"; url: string }
  | { do: "editAgent"; agentId: AgentId }
  | { do: "editGroup"; groupId: GroupId }
  | { do: "openSettings" }
  | { do: "newAgent" }
  | { do: "newGroup" }
  | { do: "openCafeteria" };

export interface SearchResult {
  /** Unique within a result set: the React key, and what selection tracks. */
  key: string;
  kind: ResultKind;
  title: string;
  /** The line under the title. Empty when there is nothing worth saying. */
  detail: string;
  /** The right-hand label: a time, a size, a cadence. */
  meta: string;
  score: number;
  action: SearchAction;
  /** Colour and avatar, for the rows that have a face. */
  face?: { avatar: string; color: string; seed: string };
}

export interface SearchInput {
  query: string;
  /** Every agent, including terminated ones: they still name old messages. */
  agents: AgentCard[];
  groups: Group[];
  /** Null until the first answer comes back. */
  hits: SearchHits | null;
  lastActive: Record<AgentId, number>;
  now: number;
}

/**
 * What an empty query scores.
 *
 * Not zero: the palette opens before anybody types, and a list that is empty
 * until the first keystroke reads as a workspace with nothing in it. Everything
 * matches equally, so the order falls through to the category and then to
 * recency, which is the order the rail is already in.
 */
const NEUTRAL = 10;

/**
 * Which kind wins when two results are equally good matches.
 *
 * An agent is what an operator is most often looking for and the cheapest thing
 * to be wrong about: opening the wrong channel costs a click. A link is last
 * because it is the one result that leaves the app.
 */
const PRIORITY: Record<ResultKind, number> = {
  agents: 7,
  groups: 6,
  actions: 5,
  messages: 4,
  files: 3,
  routines: 2,
  links: 1,
};

/**
 * How well a piece of text answers a query, from 0 (not at all) to 100.
 *
 * Ranked by where the match falls rather than by edit distance: somebody typing
 * "man" wants Manager, not the message that happens to contain "command". The
 * whole-word rule is what separates those two, and a shorter haystack breaks
 * the remaining ties, so a name beats a paragraph that contains the same word.
 */
export function score(text: string, query: string): number {
  const needle = query.trim().toLowerCase();
  if (!needle) return NEUTRAL;

  const hay = text.toLowerCase();
  const at = hay.indexOf(needle);
  if (at < 0) return 0;

  const base =
    hay === needle
      ? 100
      : at === 0
        ? 80
        : // A match that starts a word, which is what an abbreviation is.
          /[\s\-_/.,:]/.test(hay[at - 1] ?? "")
          ? 60
          : 40;

  // Up to 9 points for concision, which never lifts a result into the band
  // above it: the bands are what decides the order, this only settles ties.
  return base + Math.round(9 * (needle.length / Math.max(needle.length, hay.length)));
}

/** The best of several fields, each worth a little less than the last. */
function bestOf(query: string, fields: [text: string, weight: number][]): number {
  return fields.reduce((best, [text, weight]) => Math.max(best, score(text, query) * weight), 0);
}

/** The first line of a longer body, for a one-line row. */
function firstLine(text: string, cap = 120): string {
  const line = text.replace(/\s+/g, " ").trim();
  return line.length > cap ? `${line.slice(0, cap - 1)}…` : line;
}

/**
 * A URL with the noise taken out, so a list of links is scannable.
 *
 * The scheme and a leading `www.` say nothing an operator is choosing between.
 */
export function shortUrl(url: string): string {
  return url.replace(/^https?:\/\/(www\.)?/, "").replace(/\/$/, "");
}

/**
 * Everything matching, best first.
 *
 * Ordered by score, then by category, then by recency. Scoring dominates so
 * that a name typed in full wins wherever it lives; the category only settles
 * results that are equally good answers to what was typed.
 */
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
    // Terminated agents still name the messages they sent, which is why they
    // are in the list above, but they are not somewhere you can go.
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
    // Nothing to be recent about, so equally-scored actions fall through the
    // sort to their own names.
    results.push({ ...door, score: value, at: 0 });
  }

  // The four below are pushed without a score test. The store already decided
  // they matched, and dropping one here because this side scores it zero would
  // be the search disagreeing with itself in front of the operator.
  for (const hit of hits?.messages ?? []) {
    results.push({
      key: `message:${hit.id}`,
      kind: "messages",
      title: `${nameOf(hit.from)} → ${nameOf(hit.to)}`,
      detail: hit.excerpt,
      meta: relativeTime(hit.createdAt, now),
      // Scored on the excerpt, which is the part that matched and the part
      // that is on screen. The full body is not on this side to score.
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
      // Titled and described by the same functions the schedule panel uses, so
      // a routine reads the same here as it does where it is edited.
      title: routineTitle(routine),
      detail: cardOf(routine.agentId)?.name ?? "Deleted agent",
      meta: describeTrigger(routine.trigger, routine.nextRunAt),
      // Either column is what somebody would type, and the store matches both.
      score: Math.max(score(routine.name, query), score(routine.what, query)),
      at: routine.nextRunAt,
      // The channel, not the profile: a schedule sits in the panel beside the
      // conversation, and the profile dialog no longer has it.
      action: { do: "openChannel", agentId: routine.agentId },
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

/** The avatar of whichever end of a message is an agent. */
function faceFor(
  hit: MessageHit,
  cardOf: (id: AgentId) => AgentCard | undefined,
): SearchResult["face"] {
  const id = hit.from.kind === "agent" ? hit.from.id : hit.channelId;
  const card = cardOf(id);
  return card ? { avatar: card.avatar, color: card.color, seed: card.id } : undefined;
}

/**
 * The things a search can do rather than open.
 *
 * Every one of them is a door: settings for an agent, settings for a group,
 * settings for the app, and the two ways to add to the workspace. Nothing here
 * destroys anything, because a list you arrow through and a list you delete
 * from should not be the same list.
 */
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
      detail: "Endpoint, model, credentials for the whole group",
      meta: "Action",
      action: { do: "editGroup", groupId: group.id },
    });
  }

  out.push(
    {
      key: "action:app-settings",
      kind: "actions",
      title: "App settings",
      detail: "API key, model, limits, computers",
      meta: "Action",
      action: { do: "openSettings" },
    },
    {
      key: "action:new-agent",
      kind: "actions",
      title: "New agent",
      detail: "Add somebody to the workspace",
      meta: "Action",
      action: { do: "newAgent" },
    },
    {
      key: "action:new-group",
      kind: "actions",
      title: "New group",
      detail: "A crew that cannot see the others",
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

/** What one tab shows. */
export function inScope(results: SearchResult[], scope: Scope): SearchResult[] {
  return scope === "all" ? results : results.filter((r) => r.kind === scope);
}

/** The label on a tab. */
export function scopeLabel(scope: Scope): string {
  return scope === "all" ? "All" : scope[0]!.toUpperCase() + scope.slice(1);
}
