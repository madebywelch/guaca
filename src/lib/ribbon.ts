/**
 * Where everything on the ribbon sits: the strip across the top of the window
 * that puts each agent at a moment of today and the rest of the week after it.
 *
 * Pure, and with no clock of its own. The window passes `now` in, so the same
 * input lays out the same strip, and a test can stand at 9:12 on a Monday.
 *
 * Positions are fractions of the timeline's width, 0 at its left edge and 1 at
 * its right. The scale is bent rather than linear: the stretch before the last
 * hour or so is compressed, the hours either side of now are wide, the rest of
 * today is narrow, and each coming day is a narrow column of its own. A linear
 * day would spend most of the strip on hours where nothing happened.
 */

import { clockLabel, midnight, whenLabel } from "./time";
import type {
  Activity,
  AgentCard,
  AgentId,
  Approval,
  ApprovalId,
  Occasion,
  Outcome,
  UpcomingRun,
} from "./types";

export interface RibbonInput {
  now: number;
  /** The agents to draw, deleted ones already left out. */
  agents: AgentCard[];
  activity: Record<AgentId, Activity | undefined>;
  outcomes: Outcome[];
  /** Approvals still waiting on the operator. */
  approvals: Approval[];
  upcoming: UpcomingRun[];
  occasions: Occasion[];
  /**
   * How much of the timeline one face takes, as a fraction of its width:
   * `RIBBON_FACE_PX` over the width `Ribbon` reports. Faces nearer than this
   * are pushed apart.
   */
  faceSpan: number;
}

/**
 * Why a face is where it is. The first four pull it to a live or unseen
 * moment and carry a mark; `rest` is its last moment today, unmarked.
 */
export type FaceState = "needsYou" | "working" | "finished" | "failed" | "rest";

export interface RibbonFace {
  agentId: AgentId;
  state: FaceState;
  /** The moment it belongs to. */
  at: number;
  /** Where it is drawn, after being pushed clear of its neighbors. */
  x: number;
  /** What a screen reader says: "Dara, needs you, asked 9:05 AM". */
  label: string;
}

export interface RibbonScheduled {
  key: string;
  /** A routine shows its owner's face; a calendar occasion shows a glyph. */
  agentId: AgentId | null;
  at: number;
  x: number;
  what: string;
  /** Under the name: "10:00 AM", "8 to 10 AM". The day is the column's. */
  when: string;
  /** The whole of it, day included: "Changelog type check, Tue 9:00 AM". */
  label: string;
  /**
   * The distance to the nearest thing beside it on the strip, as a share of
   * the strip. Its words get that much width and no more, and none at all
   * when it is less than a face: a crowd of names written over each other
   * says nothing, and the label above is still read out.
   */
  room: number;
  /** Whether there is room for its words beside it at all: two faces' width. */
  words: boolean;
}

export interface RibbonLapse {
  approvalId: ApprovalId;
  agentId: AgentId;
  at: number;
  x: number;
  /** "Dara stops waiting". */
  label: string;
  when: string;
}

export interface RibbonTick {
  at: number;
  x: number;
  label: string;
  /** An hour is centered on its moment; a day labels the column it starts. */
  kind: "hour" | "day";
}

/** One bend in the scale: the moment `at` is drawn at `x`. */
export interface Knot {
  at: number;
  x: number;
}

export interface Ribbon {
  now: { at: number; x: number; label: string };
  /** The faint gray behind what happened since the operator last looked. Runs to now. */
  band: { from: number; x: number; label: string } | null;
  /** Agents with nothing to show, for the pile at the left edge. */
  pile: AgentId[];
  faces: RibbonFace[];
  scheduled: RibbonScheduled[];
  lapses: RibbonLapse[];
  ticks: RibbonTick[];
  scale: Knot[];
}

/** A face on the ribbon and the air beside it, in CSS pixels at 100%. */
export const RIBBON_FACE_PX = 52;

/** How many faces' width a scheduled item needs free beside it to say what it is. */
const WORDS_SPANS = 2;

export const BAND_LABEL = "Since you last looked";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;

/** Where today ends and the week's columns begin. */
const TODAY_END = 0.58;
/** How today's width is shared: before the near hours, the near hours, after. */
const SHARE = { before: 0.45, near: 0.45, after: 0.1 };
const NEAR_BEFORE = 75 * MINUTE;
const NEAR_AFTER = 105 * MINUTE;
/** Air before the earliest thing drawn today, so it is not on the edge. */
const LEAD = 30 * MINUTE;
/** A day column spends its inside on 6 AM to 9 PM, and its edges on the night. */
const DAY_FROM = 6 * HOUR;
const DAY_TO = 21 * HOUR;
const DAY_INSET = 0.14;
const MAX_DAYS = 4;
/** How close two hour labels may sit, and how close one may come to the now pill. */
const TICK_GAP = 0.12;
const TICK_CLEAR_NOW = 0.05;
/** The band's label takes this much of the strip after where it starts. */
const BAND_LABEL_SPAN = 0.14;

/** Local midnight `days` after the one `at` falls in, honest across a clock change. */
function dayStart(at: number, days: number): number {
  const day = new Date(midnight(at));
  day.setDate(day.getDate() + days);
  return day.getTime();
}

/**
 * The coming days the strip has columns for: through Sunday, at most four, and
 * on a Sunday the four after it, so there is always a week ahead to look at.
 */
function comingDays(now: number): number {
  const weekday = new Date(now).getDay();
  return weekday === 0 ? MAX_DAYS : Math.min(MAX_DAYS, 7 - weekday);
}

/** The moment `at` drawn on a scale, clamped to its ends. */
export function xAt(scale: Knot[], at: number): number {
  const first = scale[0];
  const last = scale[scale.length - 1];
  if (!first || !last) return 0;
  if (at <= first.at) return first.x;
  if (at >= last.at) return last.x;
  for (let i = 1; i < scale.length; i++) {
    const a = scale[i - 1] as Knot;
    const b = scale[i] as Knot;
    if (at <= b.at) return a.x + ((at - a.at) / (b.at - a.at)) * (b.x - a.x);
  }
  return last.x;
}

function buildScale(start: number, now: number, days: number): Knot[] {
  const tomorrow = dayStart(now, 1);
  const nearFrom = Math.max(start, now - NEAR_BEFORE);
  const nearTo = Math.min(tomorrow, now + NEAR_AFTER);
  const before = nearFrom > start ? SHARE.before : 0;
  const after = nearTo < tomorrow ? SHARE.after : 0;
  const unit = TODAY_END / (before + SHARE.near + after);

  const knots: Knot[] = [{ at: start, x: 0 }];
  if (before) knots.push({ at: nearFrom, x: before * unit });
  knots.push({ at: nearTo, x: (before + SHARE.near) * unit });
  if (after) knots.push({ at: tomorrow, x: TODAY_END });

  const width = (1 - TODAY_END) / days;
  for (let i = 0; i < days; i++) {
    const day = dayStart(now, i + 1);
    const left = TODAY_END + width * i;
    knots.push({ at: day + DAY_FROM, x: left + width * DAY_INSET });
    knots.push({ at: day + DAY_TO, x: left + width * (1 - DAY_INSET) });
  }
  knots.push({ at: dayStart(now, days + 1), x: 1 });
  return knots;
}

/**
 * Spreads sorted positions so no two are nearer than `gap`, moving each as
 * little as it can.
 *
 * A run of positions that collide is laid out as one cluster at `gap` steps,
 * centered on where its members wanted to be, and clusters that then collide
 * merge. Pushing each one right of the last would drag a crowd past the moment
 * it belongs to, all to one side.
 */
export function spread(xs: number[], gap: number): number[] {
  const clusters: { first: number; n: number; left: number }[] = [];
  xs.forEach((x, i) => {
    let next = { first: i, n: 1, left: x };
    for (;;) {
      const prev = clusters[clusters.length - 1];
      if (!prev || prev.left + prev.n * gap <= next.left + 1e-9) break;
      clusters.pop();
      const n = prev.n + next.n;
      // The left edge every member agrees on best: the mean of where each
      // would put it, which is its own x less its place in the cluster.
      let sum = 0;
      for (let k = 0; k < n; k++) sum += (xs[prev.first + k] as number) - k * gap;
      next = { first: prev.first, n, left: sum / n };
    }
    clusters.push(next);
  });

  const out = new Array<number>(xs.length);
  for (const c of clusters) {
    for (let k = 0; k < c.n; k++) out[c.first + k] = c.left + k * gap;
  }
  // Kept on the strip. A crowd too big for it overlaps rather than leaving it.
  const half = gap / 2;
  for (let i = 0; i < out.length; i++) {
    const floor = i === 0 ? half : (out[i - 1] as number) + gap;
    out[i] = Math.max(out[i] as number, floor);
  }
  for (let i = out.length - 1; i >= 0; i--) {
    const ceiling = i === out.length - 1 ? 1 - half : (out[i + 1] as number) - gap;
    out[i] = Math.max(half, Math.min(out[i] as number, ceiling));
  }
  return out;
}

/** "2:56 AM" today, "yesterday 11:50 PM", "Saturday 4:00 PM" further back. */
function stamp(at: number, now: number): string {
  return whenLabel(at, now)
    .replace(/^Today /, "")
    .replace(/^Yesterday/, "yesterday");
}

/** A clock without its half of the day, for the pill on the now line: "9:12". */
function bareClock(at: number): string {
  return clockLabel(at).replace(/\s\S+$/, "");
}

function hourLabel(at: number): string {
  const hour = new Date(at).getHours();
  if (hour === 12) return "Noon";
  if (hour === 0) return "Midnight";
  return new Date(at).toLocaleTimeString([], { hour: "numeric" });
}

/** "8 AM" on the hour, "8:30 AM" otherwise. */
function shortClock(at: number): string {
  return new Date(at).getMinutes()
    ? clockLabel(at)
    : new Date(at).toLocaleTimeString([], { hour: "numeric" });
}

/** "8 to 10 AM" when both ends share a half of the day, "11 AM to 1 PM" when not. */
function span(from: number, to: number): string {
  const start = shortClock(from);
  const end = shortClock(to);
  const half = (text: string) => text.split(/\s/).pop();
  return `${half(start) === half(end) ? start.replace(/\s\S+$/, "") : start} to ${end}`;
}

/** The face an agent gets, or null for the pile. */
function placeFace(
  agent: AgentCard,
  input: RibbonInput,
  today: number,
): Omit<RibbonFace, "x" | "label"> | null {
  const asked = input.approvals
    .filter((one) => one.agentId === agent.id)
    .sort((a, b) => a.createdAt - b.createdAt)[0];
  if (asked) return { agentId: agent.id, state: "needsYou", at: asked.createdAt };

  const doing = input.activity[agent.id]?.state;
  if (doing === "thinking" || doing === "queued") {
    return { agentId: agent.id, state: "working", at: input.now };
  }

  const own = input.outcomes.filter((one) => one.agentId === agent.id);
  const unseen = own
    .filter((one) => one.seenAt === null)
    .sort((a, b) => b.finishedAt - a.finishedAt)[0];
  if (unseen) {
    const state = unseen.status === "failed" ? "failed" : "finished";
    return { agentId: agent.id, state, at: unseen.finishedAt };
  }

  const last = own
    .flatMap((one) => [one.startedAt, one.finishedAt])
    .filter((at) => at >= today && at <= input.now)
    .sort((a, b) => b - a)[0];
  return last === undefined ? null : { agentId: agent.id, state: "rest", at: last };
}

function faceLabel(name: string, state: FaceState, at: number, now: number): string {
  switch (state) {
    case "needsYou":
      return `${name}, needs you, asked ${stamp(at, now)}`;
    case "working":
      return `${name}, working now`;
    case "finished":
      return `${name}, finished ${stamp(at, now)}`;
    case "failed":
      return `${name}, failed ${stamp(at, now)}`;
    case "rest":
      return `${name}, last active ${stamp(at, now)}`;
  }
}

/**
 * Where the band starts: the last time anything was marked seen, or with
 * nothing ever seen, when the oldest unseen run began. Null when there is
 * nothing unseen to put in it.
 */
function bandFrom(outcomes: Outcome[]): number | null {
  const unseen = outcomes.filter((one) => one.seenAt === null);
  if (!unseen.length) return null;
  const seen = outcomes.flatMap((one) => (one.seenAt === null ? [] : [one.seenAt]));
  if (seen.length) return Math.max(...seen);
  return Math.min(...unseen.map((one) => one.startedAt));
}

function hourTicks(scale: Knot[], start: number, now: number, band: number | null): RibbonTick[] {
  // From the first local hour, since not every zone is a whole hour off UTC.
  const first = new Date(start);
  first.setMinutes(0, 0, 0);
  const candidates: number[] = [];
  for (let at = first.getTime(); at < dayStart(now, 1); at += HOUR) {
    if (at >= start) candidates.push(at);
  }
  // Round hours win a crowded stretch: noon and midnight, then every six,
  // every three, then any.
  const rank = (at: number) => {
    const hour = new Date(at).getHours();
    return hour % 12 === 0 ? 0 : hour % 6 === 0 ? 1 : hour % 3 === 0 ? 2 : 3;
  };
  const nowX = xAt(scale, now);
  const bandX = band === null ? null : xAt(scale, band);
  const kept: RibbonTick[] = [];
  for (const at of candidates.sort((a, b) => rank(a) - rank(b) || a - b)) {
    const x = xAt(scale, at);
    if (x > TODAY_END - TICK_GAP / 2) continue;
    if (Math.abs(x - nowX) < TICK_CLEAR_NOW) continue;
    if (bandX !== null && x > bandX - TICK_CLEAR_NOW && x < bandX + BAND_LABEL_SPAN) continue;
    if (kept.some((tick) => Math.abs(tick.x - x) < TICK_GAP)) continue;
    kept.push({ at, x, label: hourLabel(at), kind: "hour" });
  }
  return kept.sort((a, b) => a.at - b.at);
}

/**
 * Lays the strip out.
 *
 * Each agent is one face, placed by the first of these that holds: it needs
 * the operator (at the oldest pending approval), it is working (at now), it
 * finished a run nobody has seen (at that run's end, failed or not), or it did
 * anything today (at the last of it). An agent none of those hold for goes to
 * the pile. What is scheduled is drawn apart from the faces, small.
 */
export function layoutRibbon(input: RibbonInput): Ribbon {
  const { now } = input;
  const today = midnight(now);
  const days = comingDays(now);
  const weekEnd = dayStart(now, days + 1);
  const names = new Map(input.agents.map((agent) => [agent.id, agent.name]));

  const placed = input.agents.map((agent) => ({ agent, face: placeFace(agent, input, today) }));
  const pile = placed.filter(({ face }) => !face).map(({ agent }) => agent.id);
  const wanted = placed.flatMap(({ face }) => (face ? [face] : []));

  const band = bandFrom(input.outcomes);
  // As far back as anything drawn, but no further than a day: a run left
  // unseen for a week is drawn at the left edge rather than a week away.
  const earliest = Math.min(now - NEAR_BEFORE, band ?? now, ...wanted.map((face) => face.at));
  const start = Math.max(now - 24 * HOUR, Math.floor((earliest - LEAD) / HOUR) * HOUR);
  const scale = buildScale(start, now, days);

  const dayName = (at: number) =>
    at < dayStart(now, 1) ? "today" : new Date(at).toLocaleDateString([], { weekday: "short" });
  // The same routine at the same moment for several agents is one item: nine
  // copies of it side by side would push the last of them days away from
  // the moment all nine are at.
  const together = new Map<string, UpcomingRun[]>();
  for (const run of input.upcoming) {
    if (!names.has(run.agentId) || run.nextRunAt < now || run.nextRunAt >= weekEnd) continue;
    const key = `${run.name} at ${Math.floor(run.nextRunAt / MINUTE)}`;
    together.set(key, [...(together.get(key) ?? []), run]);
  }
  const runs = [...together.values()].map((same) => {
    const run = same[0] as UpcomingRun;
    const agents = new Set(same.map((one) => one.agentId)).size;
    const who = agents > 1 ? ` for ${agents} agents` : "";
    return {
      key: `run:${run.routineId}`,
      agentId: run.agentId as AgentId | null,
      at: run.nextRunAt,
      what: run.name,
      when: clockLabel(run.nextRunAt),
      label: `${run.name}${who}, ${dayName(run.nextRunAt)} ${clockLabel(run.nextRunAt)}`,
    };
  });
  const occasions = input.occasions
    .filter((one) => (one.allDay ? one.startsAt > today : one.startsAt >= now))
    .filter((one) => one.startsAt < weekEnd)
    .map((one) => {
      // A day with no time on it sits in the middle of its day.
      const at = one.allDay ? one.startsAt + 12 * HOUR : one.startsAt;
      const when = one.allDay
        ? "All day"
        : one.minutes
          ? span(one.startsAt, one.startsAt + one.minutes * MINUTE)
          : clockLabel(one.startsAt);
      const day = dayName(at);
      return {
        key: `occasion:${one.id}`,
        agentId: null,
        at,
        what: one.title,
        when,
        label: one.allDay ? `${one.title}, ${day}, all day` : `${one.title}, ${day} ${when}`,
      };
    });
  // Faces and what is scheduled share one row, so they are spread as one:
  // spread apart, a run due at eight would sit on top of a face working now.
  // When the strip cannot hold every face and every item a face apart, the
  // schedule gives way: who needs you and who is working come first.
  const fits = (wanted.length + runs.length + occasions.length) * input.faceSpan <= 1;
  const row = [
    ...wanted.map((face) => ({ face, item: null, at: face.at, x: xAt(scale, face.at) })),
    ...(fits ? [...runs, ...occasions] : []).map((item) => ({
      face: null,
      item,
      at: item.at,
      x: xAt(scale, item.at),
    })),
  ].sort((a, b) => a.x - b.x || a.at - b.at);
  const xs = spread(
    row.map((one) => one.x),
    input.faceSpan,
  );
  const faces: RibbonFace[] = [];
  const scheduled: RibbonScheduled[] = [];
  row.forEach((one, i) => {
    const x = xs[i] as number;
    if (one.face) {
      const label = faceLabel(names.get(one.face.agentId) ?? "", one.face.state, one.face.at, now);
      faces.push({ ...one.face, x, label });
      return;
    }
    if (!one.item) return;
    const before = i > 0 ? x - (xs[i - 1] as number) : 2 * x;
    const after = i < xs.length - 1 ? (xs[i + 1] as number) - x : 2 * (1 - x);
    const room = Math.min(before, after);
    scheduled.push({ ...one.item, x, room, words: room >= WORDS_SPANS * input.faceSpan });
  });

  const lapses = input.approvals.flatMap((one) =>
    one.lapsesAt === null
      ? []
      : [
          {
            approvalId: one.id,
            agentId: one.agentId,
            at: one.lapsesAt,
            x: xAt(scale, one.lapsesAt),
            label: `${names.get(one.agentId) ?? "It"} stops waiting`,
            when: stamp(one.lapsesAt, now),
          },
        ],
  );

  const width = (1 - TODAY_END) / days;
  const dayTicks = Array.from({ length: days }, (_, i): RibbonTick => {
    const at = dayStart(now, i + 1);
    return {
      at,
      x: TODAY_END + width * i,
      label: new Date(at).toLocaleDateString([], { weekday: "short" }),
      kind: "day",
    };
  });

  return {
    now: { at: now, x: xAt(scale, now), label: bareClock(now) },
    band: band === null ? null : { from: band, x: xAt(scale, band), label: BAND_LABEL },
    pile,
    faces,
    scheduled,
    lapses,
    ticks: [...hourTicks(scale, start, now, band), ...dayTicks],
    scale,
  };
}
