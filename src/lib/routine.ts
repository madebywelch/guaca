/**
 * How a routine reads to the operator, and what they have to be able to say.
 *
 * A trigger's stored form is one string, mirrored from
 * `domain::routine::Trigger` in Rust: `once`, `daily`, `weekdays`, `weekly`,
 * `monthly`, `every:<seconds>`, or `event:<service>/<topic>`. {@link
 * parseTrigger} is the only thing that reads that text; everything else in the
 * app branches on what it returns.
 *
 * A trigger says the shape of the repeat and not the moment it happens at. The
 * moment lives in `nextRunAt`, which is what lets one weekday routine be nine
 * in the morning and another be five in the afternoon without either of them
 * being a different kind of thing. It also means the moment is the only record
 * of which weekday a weekly routine keeps, or which day of the month a monthly
 * one does: {@link firstRunDelay} is how the operator states it, and every
 * repeat after the first inherits it.
 */

import type { Routine, TriggerSpec } from "./types";

const HOUR_SECS = 3600;
const GAP_PREFIX = "every:";
const EVENT_PREFIX = "event:";

/** The calendar repeats, which are the ones that keep a time of day. */
export type CalendarRepeat = "daily" | "weekdays" | "weekly" | "monthly";

/**
 * A trigger, read.
 *
 * `unknown` is a forward-only migration arriving: a newer build can write a
 * value this one has never heard of, and drawing the raw text beats drawing
 * nothing and beats guessing.
 */
export type Trigger =
  | { kind: "gap"; secs: number }
  | { kind: "calendar"; repeat: CalendarRepeat }
  | { kind: "once" }
  | { kind: "event"; service: string; topic: string }
  | { kind: "unknown"; spec: string };

/** Reads the stored form. Mirrors `Trigger::parse` in Rust. */
export function parseTrigger(spec: TriggerSpec): Trigger {
  const raw = spec.trim();
  if (raw === "once") return { kind: "once" };
  if (raw === "daily" || raw === "weekdays" || raw === "weekly" || raw === "monthly") {
    return { kind: "calendar", repeat: raw };
  }
  if (raw.startsWith(GAP_PREFIX)) {
    const secs = Number(raw.slice(GAP_PREFIX.length));
    return Number.isFinite(secs) && secs > 0
      ? { kind: "gap", secs }
      : { kind: "unknown", spec: raw };
  }
  if (raw.startsWith(EVENT_PREFIX)) {
    const rest = raw.slice(EVENT_PREFIX.length);
    const cut = rest.indexOf("/");
    const service = cut > 0 ? rest.slice(0, cut) : "";
    const topic = cut > 0 ? rest.slice(cut + 1) : "";
    // Both halves or neither: a service with no topic names nothing, and the
    // Rust parser refuses it, so this must not draw it as though it worked.
    return service && topic ? { kind: "event", service, topic } : { kind: "unknown", spec: raw };
  }
  return { kind: "unknown", spec: raw };
}

/**
 * What the operator has to state for a trigger to mean what they meant.
 *
 * `weekday` and `monthday` are the reason this exists. A weekly routine keeps
 * the weekday of its first firing and a monthly one keeps the day of the
 * month, and neither was askable: "every week at 09:00" landed on whichever
 * day the operator happened to be setting it up on, and there was nothing on
 * screen that said so.
 */
export type Anchor = "none" | "time" | "weekday" | "monthday" | "date";

/** Which of those a trigger needs. */
export function anchorFor(spec: TriggerSpec): Anchor {
  const trigger = parseTrigger(spec);
  switch (trigger.kind) {
    case "once":
      // A time alone can only mean the next 24 hours, which is not what a
      // one-off is for: "remind me on the 3rd" had no way to be said.
      return "date";
    case "calendar":
      if (trigger.repeat === "weekly") return "weekday";
      if (trigger.repeat === "monthly") return "monthday";
      return "time";
    // A gap has no hour to hold on to, and an event happens when it happens.
    default:
      return "none";
  }
}

export interface TriggerChoice {
  spec: TriggerSpec;
  label: string;
}

/**
 * What the picker offers, in the order a person thinks of them.
 *
 * "Every hour" is a gap because an hourly job has no hour to hold on to; the
 * rest are calendar repeats, which keep their time of day across a clock
 * change and, for weekdays, genuinely skip the weekend.
 *
 * No event trigger is offered. The storage, the scheduler, the panel and this
 * file all handle one, but nothing delivers an event yet, so offering it would
 * be a routine the operator could set and watch never fire.
 */
export const TRIGGER_CHOICES: TriggerChoice[] = [
  { spec: `every:${HOUR_SECS}`, label: "Every hour" },
  { spec: "daily", label: "Every day" },
  { spec: "weekdays", label: "Weekdays" },
  { spec: "weekly", label: "Every week" },
  { spec: "monthly", label: "Every month" },
  { spec: "once", label: "Once" },
];

/** `stripe` as `Stripe`. Mirrors `titled` in Rust. */
function titled(word: string): string {
  return word ? word[0]!.toUpperCase() + word.slice(1) : word;
}

/**
 * The repeat, in words.
 *
 * Handles shapes no choice offers, because an agent setting its own schedule
 * works in seconds and picks whatever it likes: `every:18000` has to read as
 * "Every 5 hours" rather than as the raw row.
 */
export function repeatLabel(spec: TriggerSpec): string {
  const trigger = parseTrigger(spec);
  switch (trigger.kind) {
    case "event":
      return `When ${titled(trigger.service)} reports ${trigger.topic}`;
    case "gap":
      return `Every ${humanGap(trigger.secs)}`;
    case "unknown":
      // A trigger from a build that knows more than this one. Saying it beats
      // drawing nothing.
      return trigger.spec;
    default:
      return TRIGGER_CHOICES.find((choice) => choice.spec === spec)?.label ?? spec;
  }
}

/** Whole units where they divide evenly. Mirrors `human_gap` in Rust. */
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

/** As long as a title can be before it stops being one. */
const TITLE_MAX = 44;

/**
 * What to call a routine in a list.
 *
 * Its name, when it has one. Agents set routines for themselves and need not
 * name them, and the instruction is not a substitute: it is written to be
 * acted on with no other context, so it runs to several sentences and filled
 * the panel with one row. What is left of it after the first clause is a
 * usable label, and the operator can replace it by typing a real name.
 */
export function routineTitle(routine: Pick<Routine, "name" | "what">): string {
  const named = routine.name.trim();
  if (named) return named;

  const instruction = routine.what.trim().replace(/\s+/g, " ");
  // The first sentence, when there is one short enough to be a title.
  const stop = instruction.search(/[.!?](\s|$)/);
  const clause = stop > 0 && stop <= TITLE_MAX ? instruction.slice(0, stop) : instruction;
  if (clause.length <= TITLE_MAX) return clause || "Untitled routine";

  // Cut on a word boundary rather than mid-word, which reads as corruption.
  const cut = clause.slice(0, TITLE_MAX);
  const lastSpace = cut.lastIndexOf(" ");
  return `${(lastSpace > TITLE_MAX / 2 ? cut.slice(0, lastSpace) : cut).trimEnd()}…`;
}

/** `9:28 AM`, in the operator's own locale and clock. */
export function clockTime(at: number): string {
  return new Date(at).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

/**
 * The whole line under a routine's name: `Weekdays at 9:28 AM`.
 *
 * A one-off carries its date as well, because "Once at 9:00 AM" is not an
 * answer to when. A weekly or monthly one carries the day it keeps, for the
 * same reason: "Every week at 9:00 AM" leaves out the only part of the answer
 * the operator cannot work out for themselves.
 *
 * `nextRunAt` is null for a trigger that holds no moment, and then there is
 * nothing to add: what it says is already the whole of when.
 */
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

/** `Sep 25`. */
function dayLabel(at: number): string {
  return new Date(at).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** `1st`, `2nd`, `23rd`. */
export function ordinal(day: number): string {
  const teen = day % 100 >= 11 && day % 100 <= 13;
  const suffix = teen ? "th" : (["th", "st", "nd", "rd"][day % 10] ?? "th");
  return `${day}${suffix}`;
}

/** `09:28`, the form an `<input type="time">` reads and writes. */
export function toTimeField(at: number): string {
  const when = new Date(at);
  return `${String(when.getHours()).padStart(2, "0")}:${String(when.getMinutes()).padStart(2, "0")}`;
}

/** `2025-06-10`, the form an `<input type="date">` reads and writes. */
export function toDateField(at: number): string {
  const when = new Date(at);
  const month = String(when.getMonth() + 1).padStart(2, "0");
  return `${when.getFullYear()}-${month}-${String(when.getDate()).padStart(2, "0")}`;
}

/** Monday first, because a week of work starts there. `0` is Sunday in JS. */
export const WEEKDAYS = [
  { day: 1, label: "Monday" },
  { day: 2, label: "Tuesday" },
  { day: 3, label: "Wednesday" },
  { day: 4, label: "Thursday" },
  { day: 5, label: "Friday" },
  { day: 6, label: "Saturday" },
  { day: 0, label: "Sunday" },
];

/** What the operator picked, as far as the trigger needs it. */
export interface Moment {
  /** Local `HH:MM`. */
  time: string;
  /** `0`–`6`, Sunday first, as `Date.getDay` counts. For a weekly repeat. */
  weekday: number;
  /** `1`–`31`. For a monthly repeat. */
  monthday: number;
  /** Local `YYYY-MM-DD`. For a one-off. */
  date: string;
}

/** Reads `HH:MM`, or null if it is not one. */
function readTime(time: string): { hours: number; minutes: number } | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(time.trim());
  if (!match) return null;
  const hours = Number(match[1]);
  const minutes = Number(match[2]);
  if (hours > 23 || minutes > 59) return null;
  return { hours, minutes };
}

/**
 * How long until the next local `HH:MM`, in seconds.
 *
 * Today's slot is used while it is still ahead, tomorrow's once it has gone by,
 * and the day the trigger actually accepts is the backend's decision rather
 * than this one's: only it knows that a routine set on a Friday evening for the
 * weekday morning means Monday.
 */
export function secondsUntil(time: string, from: number = Date.now()): number | null {
  const clock = readTime(time);
  if (!clock) return null;

  const target = new Date(from);
  target.setHours(clock.hours, clock.minutes, 0, 0);
  // Built by mutating a local date rather than by arithmetic on milliseconds,
  // so the day a clock change shortens or lengthens still lands on the hour
  // that was asked for.
  if (target.getTime() <= from) target.setDate(target.getDate() + 1);

  return Math.round((target.getTime() - from) / 1000);
}

/** How many days a local month has. */
function daysInMonth(year: number, month: number): number {
  return new Date(year, month + 1, 0).getDate();
}

/**
 * How long until the first firing the operator asked for, in seconds.
 *
 * The one number the backend needs. It anchors the first firing and every
 * repeat after it inherits everything the moment carried: the hour, and for a
 * weekly or monthly repeat the day as well.
 *
 * Null means "nothing to state", for a trigger with no moment, or "that is not
 * a moment", for a field the operator has not finished filling in or a date
 * that has already gone. The two are told apart by {@link anchorFor}, and the
 * caller has to: null reaches the backend as "no delay", which is a deliberate
 * and quite different instruction.
 */
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
    // Days rather than milliseconds, so a week containing a clock change is
    // still seven days and still lands on the hour that was asked for.
    const shift = (moment.weekday - target.getDay() + 7) % 7;
    target.setDate(target.getDate() + shift);
    if (target.getTime() <= from) target.setDate(target.getDate() + 7);
    return seconds(target);
  }

  if (anchor === "monthday") {
    // The next month that actually has that day, rather than the last day of
    // this one. Clamping here would anchor a routine set for the 31st on the
    // 28th of February and keep it there for the rest of the year, which is
    // the walk backward down the calendar the Rust side is careful to avoid.
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

/** The moment a routine is currently set for, as the fields that state it. */
export function momentOf(at: number): Moment {
  const when = new Date(at);
  return {
    time: toTimeField(at),
    weekday: when.getDay(),
    monthday: when.getDate(),
    date: toDateField(at),
  };
}

/** Now, rounded up to the next five minutes: an easier row to recognize later. */
export function nextRoundMoment(from: number = Date.now()): Moment {
  const when = new Date(from);
  when.setMinutes(Math.ceil(when.getMinutes() / 5) * 5, 0, 0);
  return momentOf(when.getTime());
}
