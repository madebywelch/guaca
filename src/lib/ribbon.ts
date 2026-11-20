import { numbered } from "./rail";
import { clockLabel, midnight, whenLabel } from "./time";
import type {
  Activity,
  AgentCard,
  AgentId,
  Approval,
  ApprovalId,
  GroupId,
  Occasion,
  Outcome,
  UpcomingRun,
} from "./types";

export interface RibbonInput {
  now: number;
  agents: AgentCard[];
  groups: GroupId[];
  activity: Record<AgentId, Activity | undefined>;
  outcomes: Outcome[];
  approvals: Approval[];
  upcoming: UpcomingRun[];
  occasions: Occasion[];
  faceSpan: number;
  charSpan?: number;
}

export type Align = "center" | "start" | "end";

export type FaceState = "needsYou" | "working" | "finished" | "failed" | "rest" | "idle";

export interface RibbonFace {
  agentId: AgentId;
  state: FaceState;
  at: number;
  x: number;
  label: string;
  align: Align;
  room: number | null;
  name: string;
}

export type CountKind = "more" | "needsYou";

export interface RibbonCount {
  key: string;
  kind: CountKind;
  agentIds: AgentId[];
  x: number;
  text: string;
  label: string;
  align: Align;
}

function countText(kind: CountKind, n: number): string {
  return kind === "more" ? `+${n} more` : `+${n} ${n === 1 ? "needs" : "need"} you`;
}

export interface RibbonScheduled {
  key: string;
  agentId: AgentId | null;
  at: number;
  x: number;
  what: string;
  when: string;
  label: string;
  room: number;
  words: boolean;
  align: Align;
}

export interface RibbonLapse {
  approvalId: ApprovalId;
  agentId: AgentId;
  at: number;
  x: number;
  label: string;
  when: string;
}

export interface RibbonTick {
  at: number;
  x: number;
  label: string;
  kind: "hour" | "day";
}

export interface Knot {
  at: number;
  x: number;
}

export interface Ribbon {
  axis: number;
  now: { at: number; x: number; label: string };
  band: { from: number; x: number; label: string | null } | null;
  counts: RibbonCount[];
  faces: RibbonFace[];
  scheduled: RibbonScheduled[];
  lapses: RibbonLapse[];
  ticks: RibbonTick[];
  scale: Knot[];
}

export const RIBBON_FACE_PX = 52;

export const RIBBON_CHAR_PX = 8;

const WORDS_SPANS = 2;

export const BAND_LABEL = "Since you last looked";

export const UNNAMED_ROUTINE = "Routine";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;

const TODAY_END = 0.58;
const SHARE = { before: 0.45, near: 0.45, after: 0.1 };
const NEAR_BEFORE = 75 * MINUTE;
const NEAR_AFTER = 105 * MINUTE;
const LEAD = 30 * MINUTE;
const DAY_FROM = 6 * HOUR;
const DAY_TO = 21 * HOUR;
const DAY_INSET = 0.14;
const MAX_DAYS = 4;
const TICK_GAP = 0.12;
const TICK_CLEAR_NOW = 0.05;
const BAND_LABEL_SPAN = 0.14;
const BAND_LABEL_LEAD_CHARS = 1;
const PILL_PAD_CHARS = 2;
const TRAY_GAP_CHARS = 2;
const SHORT_WORD_LEAST = 3;
const FUTURE_LEAST = 0.22;
const FOLD_ORDER: readonly FaceState[] = [
  "idle",
  "rest",
  "finished",
  "working",
  "failed",
  "needsYou",
];

function dayStart(at: number, days: number): number {
  const day = new Date(midnight(at));
  day.setDate(day.getDate() + days);
  return day.getTime();
}

function comingDays(now: number): number {
  const weekday = new Date(now).getDay();
  return weekday === 0 ? MAX_DAYS : Math.min(MAX_DAYS, 7 - weekday);
}

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

export function spread(xs: number[], widths: number | number[], lo = 0, hi = 1): number[] {
  const w = (i: number) => (typeof widths === "number" ? widths : (widths[i] as number));
  const tail = xs.length - 1;
  const total = xs.reduce((sum, _, i) => sum + w(i), 0);
  if (tail > 0 && total > hi - lo + 1e-9) {
    const from = lo + w(0) / 2;
    const to = Math.max(from, hi - w(tail) / 2);
    return xs.map((_, i) => from + ((to - from) * i) / tail);
  }
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

function stamp(at: number, now: number): string {
  return whenLabel(at, now)
    .replace(/^Today /, "")
    .replace(/^Yesterday/, "yesterday");
}

function bareClock(at: number): string {
  return clockLabel(at).replace(/\s\S+$/, "");
}

function hourLabel(at: number): string {
  const hour = new Date(at).getHours();
  if (hour === 12) return "Noon";
  if (hour === 0) return "Midnight";
  return new Date(at).toLocaleTimeString([], { hour: "numeric" });
}

function shortClock(at: number): string {
  return new Date(at).getMinutes()
    ? clockLabel(at)
    : new Date(at).toLocaleTimeString([], { hour: "numeric" });
}

function span(from: number, to: number): string {
  const start = shortClock(from);
  const end = shortClock(to);
  const half = (text: string) => text.split(/\s/).pop();
  return `${half(start) === half(end) ? start.replace(/\s\S+$/, "") : start} to ${end}`;
}

type Placed = Pick<RibbonFace, "agentId" | "state" | "at">;

function placeFace(agent: AgentCard, input: RibbonInput, today: number): Placed | null {
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

function faceLabel(
  name: string,
  state: FaceState,
  at: number,
  now: number,
  paused: boolean,
): string {
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
    case "idle":
      return `${name}, ${paused ? "paused" : "idle"}`;
  }
}

function bandFrom(outcomes: Outcome[]): number | null {
  const unseen = outcomes.filter((one) => one.seenAt === null);
  if (!unseen.length) return null;
  const seen = outcomes.flatMap((one) => (one.seenAt === null ? [] : [one.seenAt]));
  const oldest = Math.min(...unseen.map((one) => one.startedAt));
  return seen.length ? Math.min(Math.max(...seen), oldest) : oldest;
}

function hourTicks(
  scale: Knot[],
  start: number,
  now: number,
  bandX: number | null,
  wordsSpan: number,
  axis: number,
): RibbonTick[] {
  const first = new Date(start);
  first.setMinutes(0, 0, 0);
  const candidates: number[] = [];
  for (let at = first.getTime(); at < dayStart(now, 1); at += HOUR) {
    if (at >= start) candidates.push(at);
  }
  const rank = (at: number) => {
    const hour = new Date(at).getHours();
    return hour % 12 === 0 ? 0 : hour % 6 === 0 ? 1 : hour % 3 === 0 ? 2 : 3;
  };
  const nowX = xAt(scale, now);
  const tomorrowX = xAt(scale, dayStart(now, 1));
  const kept: RibbonTick[] = [];
  for (const at of candidates.sort((a, b) => rank(a) - rank(b) || a - b)) {
    const x = xAt(scale, at);
    if (x > tomorrowX - TICK_GAP / 2) continue;
    if (axis > 0 && x < axis + TICK_GAP / 2) continue;
    if (Math.abs(x - nowX) < TICK_CLEAR_NOW) continue;
    if (bandX !== null && x > bandX - TICK_CLEAR_NOW && x < bandX + wordsSpan) continue;
    if (kept.some((tick) => Math.abs(tick.x - x) < TICK_GAP)) continue;
    kept.push({ at, x, label: hourLabel(at), kind: "hour" });
  }
  return kept.sort((a, b) => a.at - b.at);
}

function shortNames(names: string[]): Map<string, string> {
  const words = names.map((name) => name.trim().split(/\s+/));
  return new Map(
    names.map((name, i) => {
      const own = words[i] as string[];
      const told = own.find(
        (word) =>
          word.length >= SHORT_WORD_LEAST &&
          words.every((other, j) => j === i || !other.includes(word)),
      );
      return [name, told ?? name];
    }),
  );
}

function pinned(base: Knot[], pins: Knot[]): Knot[] {
  const all = [
    ...base.map((knot) => ({ ...knot, pin: false })),
    ...pins.map((knot) => ({ ...knot, pin: true })),
  ].sort((a, b) => a.at - b.at || Number(b.pin) - Number(a.pin));
  const out: Knot[] = [];
  all.forEach((knot, i) => {
    const last = out[out.length - 1];
    if (last && (knot.at <= last.at || knot.x <= last.x)) return;
    const nextPin = all.slice(i + 1).find((one) => one.pin);
    if (!knot.pin && nextPin && knot.x >= nextPin.x) return;
    out.push({ at: knot.at, x: knot.x });
  });
  return out;
}

export function layoutRibbon(given: RibbonInput): Ribbon {
  const now = Math.max(
    given.now,
    ...given.outcomes.flatMap((one) => [one.finishedAt, one.seenAt ?? one.finishedAt]),
    ...given.approvals.map((one) => one.createdAt),
  );
  const input = { ...given, now };
  const today = midnight(now);
  const days = comingDays(now);
  const weekEnd = dayStart(now, days + 1);
  const names = new Map(input.agents.map((agent) => [agent.id, agent.name]));

  const placed = input.agents.map((agent) => ({
    agent,
    face: placeFace(agent, input, today),
  }));
  const idle = numbered(
    placed.flatMap(({ agent, face }) => (face ? [] : [agent])),
    input.groups.map((id) => ({ id })),
  );
  const wanted = placed.flatMap(({ face }) => (face ? [face] : []));

  const band = bandFrom(input.outcomes);
  const earliest = Math.min(now - NEAR_BEFORE, band ?? now, ...wanted.map((face) => face.at));
  const start = Math.max(now - 24 * HOUR, Math.floor((earliest - LEAD) / HOUR) * HOUR);
  const base = buildScale(start, now, days);
  const baseNow = xAt(base, now);
  const short = shortNames(input.agents.map((agent) => agent.name));

  const dayName = (at: number) =>
    at < dayStart(now, 1) ? "today" : new Date(at).toLocaleDateString([], { weekday: "short" });
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
  const halfOf = (text: string) => Math.max(input.faceSpan, text.length * charSpan) / 2;
  const takes = (text: string) => 2 * halfOf(text) + charSpan;
  const tightWidth = input.faceSpan + charSpan;
  type Slot = { face: Placed; text: string; tight: boolean };
  const slotOf = (face: Placed): Slot => ({
    face,
    text: names.get(face.agentId) ?? "",
    tight: false,
  });
  let tray = idle.map((agent) => slotOf({ agentId: agent.id, state: "idle", at: start }));
  let past = wanted.map(slotOf);
  let more: Slot[] = [];
  let asks: Slot[] = [];
  const widthOf = (slot: Slot) => (slot.tight ? tightWidth : takes(slot.text));
  const sum = (slots: Slot[]) => slots.reduce((total, slot) => total + widthOf(slot), 0);
  const chip = (kind: CountKind, slots: Slot[]) =>
    slots.length ? takes(countText(kind, slots.length)) : 0;
  const trayWidth = () => sum(tray) + chip("more", more);
  const pastWidth = () => sum(past) + chip("needsYou", asks);
  const gap = TRAY_GAP_CHARS * charSpan;
  const most = 1 - FUTURE_LEAST;
  const need = () => (trayWidth() > 0 ? trayWidth() + gap : 0) + pastWidth() + charSpan / 2;
  const fits = () => need() <= most + 1e-9;
  const rank = (slot: Slot) => FOLD_ORDER.indexOf(slot.face.state);

  for (;;) {
    if (fits()) break;
    const next = [...tray, ...past]
      .filter(
        (slot) => !slot.tight && (short.get(slot.text) ?? slot.text).length < slot.text.length,
      )
      .sort((a, b) => b.text.length - a.text.length || rank(a) - rank(b))[0];
    if (!next) break;
    next.text = short.get(next.text) ?? next.text;
  }
  for (;;) {
    if (fits()) break;
    const next = [...tray, ...past]
      .filter(
        (slot) =>
          !slot.tight && slot.face.state !== "needsYou" && widthOf(slot) > tightWidth + 1e-9,
      )
      .sort((a, b) => rank(a) - rank(b) || widthOf(b) - widthOf(a))[0];
    if (!next) break;
    next.tight = true;
  }
  const others = [...tray]
    .reverse()
    .concat(
      [...past]
        .filter((slot) => slot.face.state !== "needsYou")
        .sort((a, b) => rank(a) - rank(b) || a.face.at - b.face.at),
    );
  for (const slot of others) {
    if (fits()) break;
    more = [...more, slot];
    tray = tray.filter((one) => one !== slot);
    past = past.filter((one) => one !== slot);
  }
  const questions = past
    .filter((slot) => slot.face.state === "needsYou")
    .sort((a, b) => b.face.at - a.face.at);
  for (const slot of questions.slice(0, -1)) {
    if (fits()) break;
    asks = [...asks, slot];
    past = past.filter((one) => one !== slot);
  }

  const axis = trayWidth() > 0 ? trayWidth() + gap : 0;
  const nowX = Math.min(
    most,
    Math.max(axis + baseNow * (1 - axis), axis + pastWidth() + charSpan / 2),
  );
  const remap = (x: number) =>
    x <= baseNow
      ? axis + (baseNow > 0 ? (x / baseNow) * (nowX - axis) : 0)
      : nowX + ((x - baseNow) / (1 - baseNow)) * (1 - nowX);
  const bent = [...base, { at: now, x: baseNow }]
    .sort((a, b) => a.at - b.at)
    .filter((knot, i, all) => i === 0 || knot.at > (all[i - 1] as Knot).at)
    .map((knot) => ({ at: knot.at, x: remap(knot.x) }));

  type Row = {
    slot: Slot | null;
    count: { kind: CountKind; slots: Slot[] } | null;
    item: (typeof runs)[number] | null;
    at: number;
    x: number;
    text: string;
    half: number;
    width: number;
  };
  const slotRow = (slot: Slot, x: number): Row => ({
    slot,
    count: null,
    item: null,
    at: slot.face.at,
    x,
    text: slot.text,
    half: slot.tight ? input.faceSpan / 2 : halfOf(slot.text),
    width: widthOf(slot),
  });
  const countRow = (kind: CountKind, slots: Slot[], at: number, x: number): Row => {
    const text = countText(kind, slots.length);
    return {
      slot: null,
      count: { kind, slots },
      item: null,
      at,
      x,
      text,
      half: halfOf(text),
      width: takes(text),
    };
  };

  const trayRows: Row[] = [
    ...tray.map((slot) => slotRow(slot, 0)),
    ...(more.length ? [countRow("more", more, start, 0)] : []),
  ];
  let cursor = 0;
  const trayXs = trayRows.map((row) => {
    const x = cursor + row.width / 2;
    cursor += row.width;
    return x;
  });

  const lastAsk = asks[0];
  const pastRows: Row[] = [
    ...past.map((slot) => slotRow(slot, xAt(bent, slot.face.at))),
    ...(lastAsk ? [countRow("needsYou", asks, lastAsk.face.at, xAt(bent, lastAsk.face.at))] : []),
  ].sort((a, b) => a.x - b.x || a.at - b.at);
  const pastXs = pastRows.length
    ? spread(
        pastRows.map((row) => row.x),
        pastRows.map((row) => row.width),
        axis,
        nowX - charSpan / 2,
      )
    : [];

  const pins: Knot[] = [];
  pastRows.forEach((row, i) => {
    const prev = pins[pins.length - 1];
    pins.push({
      at: prev ? Math.max(row.at, prev.at + 1) : row.at,
      x: pastXs[i] as number,
    });
  });
  for (let i = pins.length - 1; i >= 0; i--) {
    const pin = pins[i] as Knot;
    pin.at = Math.min(pin.at, (pins[i + 1]?.at ?? now) - 1);
  }
  const scale = pinned(bent, pins);

  const itemRows: Row[] = [...runs, ...occasions].map((item) => {
    const text = longest(item.what, item.when);
    return {
      slot: null,
      count: null,
      item,
      at: item.at,
      x: xAt(scale, item.at),
      text,
      half: halfOf(text),
      width: input.faceSpan,
    };
  });
  const laterLo = (rows: Row[]) => {
    const first = rows[0] as Row;
    return nowX + first.half + charSpan - first.width / 2;
  };
  const total = (rows: Row[]) => rows.reduce((all, row) => all + row.width, 0);
  const sorted = itemRows.sort((a, b) => a.x - b.x || a.at - b.at);
  const later = sorted.length && total(sorted) <= 1 - laterLo(sorted) + 1e-9 ? sorted : [];
  const laterXs = later.length
    ? spread(
        later.map((row) => row.x),
        later.map((row) => row.width),
        laterLo(later),
        1,
      )
    : [];

  const align = (x: number, half: number): Align =>
    x - half < 0 ? "start" : x + half > 1 ? "end" : "center";
  const faces: RibbonFace[] = [];
  const scheduled: RibbonScheduled[] = [];
  const counts: RibbonCount[] = [];
  const emit = (rows: Row[], xs: number[]) =>
    rows.forEach((one, i) => {
      const x = xs[i] as number;
      const gapBeside = Math.min(
        i > 0 ? x - (xs[i - 1] as number) : 1,
        i < xs.length - 1 ? (xs[i + 1] as number) - x : 1,
      );
      if (one.count) {
        const agentIds = one.count.slots.map((slot) => slot.face.agentId);
        const who = agentIds.map((id) => names.get(id) ?? "").join(", ");
        counts.push({
          key: `count:${one.count.kind}`,
          kind: one.count.kind,
          agentIds,
          x,
          text: one.text,
          label: `${one.text}: ${who}`,
          align: align(x, one.half),
        });
        return;
      }
      if (one.slot) {
        const { face, tight, text } = one.slot;
        const paused = input.activity[face.agentId]?.state === "paused";
        const label = faceLabel(names.get(face.agentId) ?? "", face.state, face.at, now, paused);
        const room = tight ? Math.max(0, Math.min(2 * one.half, gapBeside - charSpan)) : null;
        faces.push({
          ...face,
          x,
          label,
          align: align(x, one.half),
          room,
          name: text,
        });
        return;
      }
      if (!one.item) return;
      const words = gapBeside >= WORDS_SPANS * input.faceSpan;
      const shown = words ? one.text : one.item.when;
      scheduled.push({
        ...one.item,
        x,
        room: gapBeside,
        words,
        align: align(x, halfOf(shown)),
      });
    });
  emit(trayRows, trayXs);
  emit(pastRows, pastXs);
  emit(later, laterXs);

  const lapses = input.approvals.flatMap((one) => {
    const name = names.get(one.agentId);
    return one.lapsesAt === null || one.lapsesAt <= now || name === undefined
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
      x: remap(TODAY_END + width * i),
      label: new Date(at).toLocaleDateString([], { weekday: "short" }),
      kind: "day",
    };
  });

  const bandAt = band;
  const bandX = Math.max(
    axis,
    Math.min(
      bandAt === null ? nowX : xAt(scale, bandAt),
      ...faces
        .filter((face) => face.state === "finished" || face.state === "failed")
        .map((face) => face.x - input.faceSpan / 2),
    ),
  );
  const nowLabel = bareClock(now);
  const wordsSpan = charSpan
    ? (BAND_LABEL.length + BAND_LABEL_LEAD_CHARS) * charSpan
    : BAND_LABEL_SPAN;
  const pillHalf = ((nowLabel.length + PILL_PAD_CHARS) / 2) * charSpan;
  const bandLabel = nowX - pillHalf - bandX >= wordsSpan ? BAND_LABEL : null;

  return {
    axis,
    now: { at: now, x: nowX, label: nowLabel },
    band: bandAt === null ? null : { from: bandAt, x: bandX, label: bandLabel },
    faces,
    counts,
    scheduled,
    lapses,
    ticks: [
      ...hourTicks(scale, start, now, bandLabel ? bandX : null, wordsSpan, axis),
      ...dayTicks,
    ],
    scale,
  };
}
