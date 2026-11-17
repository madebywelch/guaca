import type { Routine, TriggerSpec } from "./types";

const HOUR_SECS = 3600;
const GAP_PREFIX = "every:";
const EVENT_PREFIX = "event:";
const MAX_EVENT_SERVICE = 48;
const MAX_EVENT_TOPIC = 120;
const MAX_GAP_SECS = 4_294_967_295;

export const COPIED_MS = 1800;

export const EVENT_CHOICE: TriggerSpec = `${EVENT_PREFIX}/`;

export function eventSpec(service: string, topic: string): TriggerSpec {
  return `${EVENT_PREFIX}${service.trim()}/${topic.trim()}`;
}

export function eventFields(spec: TriggerSpec): { service: string; topic: string } | null {
  const raw = spec.trim();
  if (!raw.startsWith(EVENT_PREFIX)) return null;
  const rest = raw.slice(EVENT_PREFIX.length);
  const cut = rest.indexOf("/");
  return cut < 0
    ? { service: rest, topic: "" }
    : { service: rest.slice(0, cut), topic: rest.slice(cut + 1) };
}

export function webhookLine(url: string, secret: string, service: string, topic: string): string {
  return [
    `curl -X POST ${url}/${service.trim()}/${topic.trim()} \\`,
    `  -H 'Authorization: Bearer ${secret}' \\`,
    `  -d '{"any":"body"}'`,
  ].join("\n");
}

export type CalendarRepeat = "daily" | "weekdays" | "weekly" | "monthly";

export type Trigger =
  | { kind: "gap"; secs: number }
  | { kind: "calendar"; repeat: CalendarRepeat }
  | { kind: "once" }
  | { kind: "event"; service: string; topic: string }
  | { kind: "unknown"; spec: string };

export function parseTrigger(spec: TriggerSpec): Trigger {
  const raw = spec.trim();
  if (raw === "once") return { kind: "once" };
  if (raw === "daily" || raw === "weekdays" || raw === "weekly" || raw === "monthly") {
    return { kind: "calendar", repeat: raw };
  }
  if (raw.startsWith(GAP_PREFIX)) {
    const digits = raw.slice(GAP_PREFIX.length);
    const secs = /^\+?\d+$/.test(digits) ? Number(digits) : Number.NaN;
    return secs <= MAX_GAP_SECS ? { kind: "gap", secs } : { kind: "unknown", spec: raw };
  }
  if (raw.startsWith(EVENT_PREFIX)) {
    const rest = raw.slice(EVENT_PREFIX.length).trim();
    const cut = rest.indexOf("/");
    const service = cut < 0 ? "" : rest.slice(0, cut).trim().toLowerCase();
    const topic = cut < 0 ? "" : rest.slice(cut + 1).trim();
    return sound(service, MAX_EVENT_SERVICE) && sound(topic, MAX_EVENT_TOPIC)
      ? { kind: "event", service, topic }
      : { kind: "unknown", spec: raw };
  }
  return { kind: "unknown", spec: raw };
}

function sound(part: string, max: number): boolean {
  return part.length > 0 && [...part].length <= max && !/\s/.test(part);
}

export type Anchor = "none" | "time" | "weekday" | "monthday" | "date";

export function anchorFor(spec: TriggerSpec): Anchor {
  const trigger = parseTrigger(spec);
  switch (trigger.kind) {
    case "once":
      return "date";
    case "calendar":
      if (trigger.repeat === "weekly") return "weekday";
      if (trigger.repeat === "monthly") return "monthday";
      return "time";
    default:
      return "none";
  }
}

export interface TriggerChoice {
  spec: TriggerSpec;
  label: string;
}

export const TRIGGER_CHOICES: TriggerChoice[] = [
  { spec: `every:${HOUR_SECS}`, label: "Every hour" },
  { spec: "daily", label: "Every day" },
  { spec: "weekdays", label: "Weekdays" },
  { spec: "weekly", label: "Every week" },
  { spec: "monthly", label: "Every month" },
  { spec: "once", label: "Once" },
  { spec: EVENT_CHOICE, label: "When an event is posted" },
];

export function choiceFor(spec: TriggerSpec): TriggerSpec {
  return eventFields(spec) ? EVENT_CHOICE : spec;
}

export function titled(word: string): string {
  const [first = "", ...rest] = word;
  return first.toUpperCase() + rest.join("");
}

export function repeatLabel(spec: TriggerSpec): string {
  const trigger = parseTrigger(spec);
  switch (trigger.kind) {
    case "event":
      return `When ${titled(trigger.service)} reports ${trigger.topic}`;
    case "gap":
      return `Every ${humanGap(trigger.secs)}`;
    case "unknown":
      return trigger.spec;
    default:
      return TRIGGER_CHOICES.find((choice) => choice.spec === spec)?.label ?? spec;
  }
}

export function repeats(spec: TriggerSpec): boolean {
  return parseTrigger(spec).kind !== "once";
}

export function humanGap(secs: number): string {
  const units: [number, string][] = [
    [86_400, "day"],
    [3600, "hour"],
    [60, "minute"],
  ];
  for (const [size, name] of units) {
    if (secs % size === 0 && secs >= size) {
      const n = secs / size;
      return n === 1 ? name : `${n} ${name}s`;
    }
  }
  return secs === 1 ? "second" : `${secs} seconds`;
}

const TITLE_MAX = 44;

export function routineTitle(routine: Pick<Routine, "name" | "what">): string {
  const named = routine.name.trim();
  if (named) return named;

  const instruction = routine.what.trim().replace(/\s+/g, " ");
  const stop = instruction.search(/[.!?](\s|$)/);
  const clause = stop > 0 && stop <= TITLE_MAX ? instruction.slice(0, stop) : instruction;
  if (clause.length <= TITLE_MAX) return clause || "Untitled routine";

  const cut = clause.slice(0, TITLE_MAX);
  const lastSpace = cut.lastIndexOf(" ");
  return `${(lastSpace > TITLE_MAX / 2 ? cut.slice(0, lastSpace) : cut).trimEnd()}…`;
}

export function clockTime(at: number): string {
  return new Date(at).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

export function describeTrigger(spec: TriggerSpec, nextRunAt: number | null): string {
  const trigger = parseTrigger(spec);
  if (nextRunAt === null) return repeatLabel(spec);

  switch (trigger.kind) {
    case "once":
      return `Once, on ${dayLabel(nextRunAt)} at ${clockTime(nextRunAt)}`;
    case "calendar": {
      if (trigger.repeat === "weekly") {
        const weekday = new Date(nextRunAt).toLocaleDateString(undefined, { weekday: "long" });
        return `Every ${weekday} at ${clockTime(nextRunAt)}`;
      }
      if (trigger.repeat === "monthly") {
        return `Monthly on the ${ordinal(new Date(nextRunAt).getDate())} at ${clockTime(nextRunAt)}`;
      }
      return `${repeatLabel(spec)} at ${clockTime(nextRunAt)}`;
    }
    default:
      return repeatLabel(spec);
  }
}

function dayLabel(at: number): string {
  return new Date(at).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export function ordinal(day: number): string {
  const teen = day % 100 >= 11 && day % 100 <= 13;
  const suffix = teen ? "th" : (["th", "st", "nd", "rd"][day % 10] ?? "th");
  return `${day}${suffix}`;
}

export function toTimeField(at: number): string {
  const when = new Date(at);
  return `${String(when.getHours()).padStart(2, "0")}:${String(when.getMinutes()).padStart(2, "0")}`;
}

export function toDateField(at: number): string {
  const when = new Date(at);
  const month = String(when.getMonth() + 1).padStart(2, "0");
  return `${when.getFullYear()}-${month}-${String(when.getDate()).padStart(2, "0")}`;
}

export const WEEKDAYS = [
  { day: 1, label: "Monday" },
  { day: 2, label: "Tuesday" },
  { day: 3, label: "Wednesday" },
  { day: 4, label: "Thursday" },
  { day: 5, label: "Friday" },
  { day: 6, label: "Saturday" },
  { day: 0, label: "Sunday" },
];

export interface Moment {
  time: string;
  weekday: number;
  monthday: number;
  date: string;
}

function readTime(time: string): { hours: number; minutes: number } | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(time.trim());
  if (!match) return null;
  const hours = Number(match[1]);
  const minutes = Number(match[2]);
  if (hours > 23 || minutes > 59) return null;
  return { hours, minutes };
}

export function secondsUntil(time: string, from: number = Date.now()): number | null {
  const clock = readTime(time);
  if (!clock) return null;

  const target = new Date(from);
  target.setHours(clock.hours, clock.minutes, 0, 0);
  if (target.getTime() <= from) target.setDate(target.getDate() + 1);

  return Math.round((target.getTime() - from) / 1000);
}

function daysInMonth(year: number, month: number): number {
  return new Date(year, month + 1, 0).getDate();
}

export function firstRunDelay(
  spec: TriggerSpec,
  moment: Moment,
  from: number = Date.now(),
): number | null {
  const anchor = anchorFor(spec);
  if (anchor === "none") return null;

  const clock = readTime(moment.time);
  if (!clock) return null;
  const { hours, minutes } = clock;

  const seconds = (target: Date) => {
    const ahead = Math.round((target.getTime() - from) / 1000);
    return ahead > 0 ? ahead : null;
  };

  if (anchor === "time") return secondsUntil(moment.time, from);

  if (anchor === "weekday") {
    const target = new Date(from);
    target.setHours(hours, minutes, 0, 0);
    const shift = (moment.weekday - target.getDay() + 7) % 7;
    target.setDate(target.getDate() + shift);
    if (target.getTime() <= from) target.setDate(target.getDate() + 7);
    return seconds(target);
  }

  if (anchor === "monthday") {
    const start = new Date(from);
    for (let step = 0; step <= 14; step += 1) {
      const month = new Date(start.getFullYear(), start.getMonth() + step, 1);
      if (moment.monthday > daysInMonth(month.getFullYear(), month.getMonth())) continue;
      const target = new Date(month.getFullYear(), month.getMonth(), moment.monthday);
      target.setHours(hours, minutes, 0, 0);
      if (target.getTime() > from) return seconds(target);
    }
    return null;
  }

  const parts = /^(\d{4})-(\d{2})-(\d{2})$/.exec(moment.date.trim());
  if (!parts) return null;
  const target = new Date(Number(parts[1]), Number(parts[2]) - 1, Number(parts[3]));
  target.setHours(hours, minutes, 0, 0);
  return seconds(target);
}

export function momentOf(at: number): Moment {
  const when = new Date(at);
  return {
    time: toTimeField(at),
    weekday: when.getDay(),
    monthday: when.getDate(),
    date: toDateField(at),
  };
}

export function nextRoundMoment(from: number = Date.now()): Moment {
  const when = new Date(from);
  when.setMinutes(Math.ceil(when.getMinutes() / 5) * 5, 0, 0);
  return momentOf(when.getTime());
}
