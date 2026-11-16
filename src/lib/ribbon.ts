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
  /**
   * One character of a name or a time, as a fraction of the timeline's width:
   * `RIBBON_CHAR_PX` over the width `Ribbon` reports. What a label is reckoned
   * to take, so it can be kept off the now line and inside the strip. Left out,
   * every label is reckoned a face wide.
   */
  charSpan?: number;
}

/**
 * Where a label hangs from its mark. Centered under it, unless that would put
 * part of the label past an end of the strip: then it starts at the mark's
 * left edge, or ends at its right one.
 */
export type Align = "center" | "start" | "end";

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
  align: Align;
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
  /**
   * Whether there is room for what it is beside it: two faces' width. Its
   * time is drawn either way, so nothing coming is only a face.
   */
  words: boolean;
  align: Align;
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
  /** Its label is null when the band is too narrow to hold it clear of the now pill. */
  band: { from: number; x: number; label: string | null } | null;
  /** Agents with nothing to show, for the pile at the left edge. */
  pile: AgentId[];
  /** How many in the pile did something today and were moved there for room. */
  crowded: number;
  faces: RibbonFace[];
  scheduled: RibbonScheduled[];
  lapses: RibbonLapse[];
  ticks: RibbonTick[];
  scale: Knot[];
}

/** A face on the ribbon and the air beside it, in CSS pixels at 100%. */
export const RIBBON_FACE_PX = 52;

/**
 * One character of a label at the size the ribbon sets names, in CSS pixels at
 * 100%, rounded up: a label reckoned a little wide is kept a little further
 * from the now line, and one reckoned narrow is drawn across it.
 */
export const RIBBON_CHAR_PX = 8;

/** How many faces' width a scheduled item needs free beside it to say what it is. */
const WORDS_SPANS = 2;

export const BAND_LABEL = "Since you last looked";

export const UNNAMED_ROUTINE = "Routine";

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
/** What the band's label is reckoned to take when nothing says how wide a character is. */
const BAND_LABEL_SPAN = 0.14;
/** The band label's lead, and the now pill's padding on both sides, in characters. */
const BAND_LABEL_LEAD_CHARS = 1;
const PILL_PAD_CHARS = 2;

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
 * Spreads sorted positions so no two overlap, moving each as little as it can.
 * `widths` is what each one takes, or one width for all of them; two
 * neighbors stand half of each one's width apart.
 *
 * A run of positions that collide is laid out as one cluster, side by side,
 * centered on where its members wanted to be, and clusters that then collide
 * merge. Pushing each one right of the last would drag a crowd past the moment
 * it belongs to, all to one side.
 */
export function spread(xs: number[], widths: number | number[], lo = 0, hi = 1): number[] {
  const w = (i: number) => (typeof widths === "number" ? widths : (widths[i] as number));
  // Where each one's center sits from the left edge of a cluster it starts.
  const offset = new Array<number>(xs.length);
  xs.forEach((_, i) => {
    offset[i] = i === 0 ? w(0) / 2 : (offset[i - 1] as number) + (w(i - 1) + w(i)) / 2;
  });
  const at = (first: number, k: number) =>
    (offset[first + k] as number) - (offset[first] as number) + w(first) / 2;
  const clusters: { first: number; n: number; left: number }[] = [];
  xs.forEach((x, i) => {
    let next = { first: i, n: 1, left: x - w(i) / 2 };
    for (;;) {
      const prev = clusters[clusters.length - 1];
      if (!prev) break;
      const end = prev.left + at(prev.first, prev.n - 1) + w(prev.first + prev.n - 1) / 2;
      if (end <= next.left + 1e-9) break;
      clusters.pop();
      const n = prev.n + next.n;
      // The left edge every member agrees on best: the mean of where each
      // would put it, which is its own x less its place in the cluster.
      let sum = 0;
      for (let k = 0; k < n; k++) sum += (xs[prev.first + k] as number) - at(prev.first, k);
      next = { first: prev.first, n, left: sum / n };
    }
    clusters.push(next);
  });

  const out = new Array<number>(xs.length);
  for (const c of clusters) {
    for (let k = 0; k < c.n; k++) out[c.first + k] = c.left + at(c.first, k);
  }
  // Kept between `lo` and `hi`, the strip unless told otherwise. A crowd too
  // big for the room overlaps rather than leaving it.
  const apart = (i: number) => (w(i - 1) + w(i)) / 2;
  for (let i = 0; i < out.length; i++) {
    const floor = i === 0 ? lo + w(0) / 2 : (out[i - 1] as number) + apart(i);
    out[i] = Math.max(out[i] as number, floor);
  }
  for (let i = out.length - 1; i >= 0; i--) {
    const last = out.length - 1;
    const ceiling = i === last ? hi - w(last) / 2 : (out[i + 1] as number) - apart(i + 1);
    out[i] = Math.max(lo + w(i) / 2, Math.min(out[i] as number, ceiling));
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
): Omit<RibbonFace, "x" | "label" | "align"> | null {
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
 * Where the band starts: the last time anything was marked seen, or when the
 * oldest unseen run began if that is earlier. Null when there is nothing
 * unseen to put in it.
 *
 * The earlier of the two, because seeing is per run: opening one agent marks
 * its run seen now, and a band that started there would leave every older run
 * nobody has looked at outside the stretch that says it was missed.
 */
function bandFrom(outcomes: Outcome[]): number | null {
  const unseen = outcomes.filter((one) => one.seenAt === null);
  if (!unseen.length) return null;
  const seen = outcomes.flatMap((one) => (one.seenAt === null ? [] : [one.seenAt]));
  const oldest = Math.min(...unseen.map((one) => one.startedAt));
  return seen.length ? Math.min(Math.max(...seen), oldest) : oldest;
}

/**
 * `bandX` is where the band's words start, or null when it has none, and
 * `wordsSpan` how much of the strip they take.
 */
function hourTicks(
  scale: Knot[],
  start: number,
  now: number,
  bandX: number | null,
  wordsSpan: number,
): RibbonTick[] {
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
  const kept: RibbonTick[] = [];
  for (const at of candidates.sort((a, b) => rank(a) - rank(b) || a - b)) {
    const x = xAt(scale, at);
    if (x > TODAY_END - TICK_GAP / 2) continue;
    if (Math.abs(x - nowX) < TICK_CLEAR_NOW) continue;
    if (bandX !== null && x > bandX - TICK_CLEAR_NOW && x < bandX + wordsSpan) continue;
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
  const drowsy = placed.filter(({ face }) => !face).map(({ agent }) => agent.id);
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
    // A routine saved without a name still says what kind of thing it is.
    const what = run.name.trim() || UNNAMED_ROUTINE;
    return {
      key: `run:${run.routineId}`,
      agentId: run.agentId as AgentId | null,
      at: run.nextRunAt,
      what,
      when: clockLabel(run.nextRunAt),
      label: `${what}${who}, ${dayName(run.nextRunAt)} ${clockLabel(run.nextRunAt)}`,
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
  const longest = (...texts: string[]) => texts.reduce((a, b) => (b.length > a.length ? b : a), "");
  const charSpan = input.charSpan ?? 0;
  // Half of what a label is reckoned to take, never less than half a face.
  const halfOf = (text: string) => Math.max(input.faceSpan, text.length * charSpan) / 2;
  type Row = {
    face: Omit<RibbonFace, "x" | "label" | "align"> | null;
    item: (typeof runs)[number] | null;
    at: number;
    x: number;
    text: string;
    width: number;
  };
  // A face takes its name and a character of air, so two names never touch;
  // an item takes a face, and says what it is only where there is room.
  const faceRows: Row[] = wanted.map((face) => {
    const text = names.get(face.agentId) ?? "";
    const x = xAt(scale, face.at);
    return { face, item: null, at: face.at, x, text, width: 2 * halfOf(text) + charSpan };
  });
  const itemRows: Row[] = [...runs, ...occasions].map((item) => ({
    face: null,
    item,
    at: item.at,
    x: xAt(scale, item.at),
    text: longest(item.what, item.when),
    width: input.faceSpan,
  }));
  const byX = (a: Row, b: Row) => a.x - b.x || a.at - b.at;
  const total = (rows: Row[]) => rows.reduce((sum, one) => sum + one.width, 0);

  // No label is drawn under the now line. What happened is spread left of it
  // and what is live or still coming right of it, each side kept a label's
  // half and a character clear: Marlow working at 9:12 sits just after the
  // line rather than with his name cut through by it.
  const nowX = xAt(scale, now);
  const after = (one: Row) => one.at > now || one.face?.state === "working";
  const beforeHi = (rows: Row[]) => {
    const last = rows[rows.length - 1] as Row;
    return nowX - halfOf(last.text) - charSpan + last.width / 2;
  };
  const laterLo = (rows: Row[]) => {
    const first = rows[0] as Row;
    return nowX + halfOf(first.text) + charSpan - first.width / 2;
  };

  // When the faces left of now cannot stand side by side, the oldest go to
  // the pile, one at a time, until the rest can; nobody who needs you does.
  const before = faceRows.filter((one) => !after(one)).sort(byX);
  const piled = new Set<AgentId>();
  while (before.length > 0 && total(before) > beforeHi(before) + 1e-9) {
    const oldest = before
      .filter((one) => one.face?.state !== "needsYou")
      .reduce<Row | null>((a, b) => (a === null || b.at < a.at ? b : a), null);
    if (!oldest?.face) break;
    piled.add(oldest.face.agentId);
    before.splice(before.indexOf(oldest), 1);
  }
  const pile = input.agents
    .map((agent) => agent.id)
    .filter((id) => drowsy.includes(id) || piled.has(id));

  // Faces and what is scheduled share one row right of now, so they are
  // spread as one: spread apart, a run due at eight would sit on top of a
  // face working now. When that side cannot hold them all, the schedule gives
  // way: who is working comes first.
  const liveFaces = faceRows.filter(after);
  const withItems = [...liveFaces, ...itemRows].sort(byX);
  const later =
    withItems.length === 0 || total(withItems) <= 1 - laterLo(withItems) + 1e-9
      ? withItems
      : liveFaces.sort(byX);
  const ordered = [...before, ...later];
  const xs = [
    ...(before.length
      ? spread(
          before.map((one) => one.x),
          before.map((one) => one.width),
          0,
          beforeHi(before),
        )
      : []),
    ...(later.length
      ? spread(
          later.map((one) => one.x),
          later.map((one) => one.width),
          laterLo(later),
          1,
        )
      : []),
  ];

  const align = (x: number, text: string): Align =>
    x - halfOf(text) < 0 ? "start" : x + halfOf(text) > 1 ? "end" : "center";
  const faces: RibbonFace[] = [];
  const scheduled: RibbonScheduled[] = [];
  ordered.forEach((one, i) => {
    const x = xs[i] as number;
    if (one.face) {
      const label = faceLabel(names.get(one.face.agentId) ?? "", one.face.state, one.face.at, now);
      faces.push({ ...one.face, x, label, align: align(x, one.text) });
      return;
    }
    if (!one.item) return;
    // At an end of the strip the label hangs inward from its mark, so the
    // only room it is short of is toward its one neighbor.
    const left = i > 0 ? x - (xs[i - 1] as number) : 1;
    const right = i < xs.length - 1 ? (xs[i + 1] as number) - x : 1;
    const room = Math.min(left, right);
    const words = room >= WORDS_SPANS * input.faceSpan;
    const shown = words ? one.text : one.item.when;
    scheduled.push({ ...one.item, x, room, words, align: align(x, shown) });
  });

  // A request from an agent the strip does not know (deleted since it asked)
  // has nobody to name, and "It stops waiting" says nothing about whose.
  const lapses = input.approvals.flatMap((one) => {
    const name = names.get(one.agentId);
    return one.lapsesAt === null || name === undefined
      ? []
      : [
          {
            approvalId: one.id,
            agentId: one.agentId,
            at: one.lapsesAt,
            x: xAt(scale, one.lapsesAt),
            label: `${name} stops waiting`,
            when: stamp(one.lapsesAt, now),
          },
        ];
  });

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

  // Wide enough for every unseen face as drawn, not only as timed: a crowd
  // pushed left of its moment stays inside the band it is news in.
  const bandAt = band;
  const bandX = Math.max(
    0,
    Math.min(
      bandAt === null ? nowX : xAt(scale, bandAt),
      ...faces
        .filter((face) => face.state === "finished" || face.state === "failed")
        .map((face) => face.x - input.faceSpan / 2),
    ),
  );
  // Words that would run under the now pill are left off; the ground still says it.
  // Reckoned in characters where the width is known, since the pill and the
  // words keep their size as the window narrows and a share of it does not.
  const nowLabel = bareClock(now);
  const wordsSpan = charSpan
    ? (BAND_LABEL.length + BAND_LABEL_LEAD_CHARS) * charSpan
    : BAND_LABEL_SPAN;
  const pillHalf = ((nowLabel.length + PILL_PAD_CHARS) / 2) * charSpan;
  const bandLabel = nowX - pillHalf - bandX >= wordsSpan ? BAND_LABEL : null;

  return {
    now: { at: now, x: xAt(scale, now), label: nowLabel },
    band: bandAt === null ? null : { from: bandAt, x: bandX, label: bandLabel },
    pile,
    crowded: piled.size,
    faces,
    scheduled,
    lapses,
    ticks: [...hourTicks(scale, start, now, bandLabel ? bandX : null, wordsSpan), ...dayTicks],
    scale,
  };
}
