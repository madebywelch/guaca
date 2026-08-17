/**
 * How a routine reads to the operator: what to call it, and when it fires.
 *
 * A trigger's stored form is one string, mirrored from
 * `domain::routine::Trigger` in Rust: `once`, `daily`, `weekdays`, `weekly`,
 * `monthly`, or `every:<seconds>`. Nothing outside this file reads that text.
 *
 * A trigger says the shape of the repeat and not the hour it happens at. The
 * hour lives in `nextRunAt`, which is what lets one weekday routine be nine in
 * the morning and another be five in the afternoon without either of them
 * being a different kind of thing.
 */

import type { Routine, TriggerSpec } from "./types";

/** The gaps a person picks by name. Anything else arrives as `every:N`. */
const HOUR_SECS = 3600;

export interface TriggerChoice {
  spec: TriggerSpec;
  label: string;
  /** Whether this one happens at a particular time of day. */
  timed: boolean;
}

/**
 * What the picker offers, in the order a person thinks of them.
 *
 * "Every hour" is a gap because an hourly job has no hour to hold on to; the
 * rest are calendar repeats, which keep their time of day across a clock
 * change and, for weekdays, genuinely skip the weekend.
 */
export const TRIGGER_CHOICES: TriggerChoice[] = [
  { spec: `every:${HOUR_SECS}`, label: "Every hour", timed: false },
  { spec: "daily", label: "Every day", timed: true },
  { spec: "weekdays", label: "Weekdays", timed: true },
  { spec: "weekly", label: "Every week", timed: true },
  { spec: "monthly", label: "Every month", timed: true },
  { spec: "once", label: "Once", timed: true },
];

/** Whether a trigger fires at a time of day worth showing and setting. */
export function isTimed(spec: TriggerSpec): boolean {
  return !spec.startsWith("every:");
}

/** The seconds in `every:N`, or null for anything else. */
export function gapSeconds(spec: TriggerSpec): number | null {
  if (!spec.startsWith("every:")) return null;
  const secs = Number(spec.slice("every:".length));
  return Number.isFinite(secs) && secs > 0 ? secs : null;
}

/**
 * The repeat, in words.
 *
 * Handles gaps no choice offers, because an agent setting its own schedule
 * works in seconds and picks whatever it likes: `every:18000` has to read as
 * "Every 5 hours" rather than as the raw row.
 */
export function repeatLabel(spec: TriggerSpec): string {
  const known = TRIGGER_CHOICES.find((choice) => choice.spec === spec);
  if (known) return known.label;

  const gap = gapSeconds(spec);
  if (gap !== null) return `Every ${humanGap(gap)}`;
  // A trigger written by a newer build. Saying so beats drawing nothing.
  return spec;
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
 * answer to when.
 */
export function describeTrigger(spec: TriggerSpec, nextRunAt: number): string {
  if (spec === "once") {
    const when = new Date(nextRunAt).toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
    });
    return `Once, on ${when} at ${clockTime(nextRunAt)}`;
  }
  if (!isTimed(spec)) return repeatLabel(spec);
  return `${repeatLabel(spec)} at ${clockTime(nextRunAt)}`;
}

/** `09:28`, the form an `<input type="time">` reads and writes. */
export function toTimeField(at: number): string {
  const when = new Date(at);
  return `${String(when.getHours()).padStart(2, "0")}:${String(when.getMinutes()).padStart(2, "0")}`;
}

/**
 * How long until the next local `HH:MM`, in seconds.
 *
 * The one number the backend needs: it anchors the first firing, and every
 * repeat after it keeps that time of day. Today's slot is used while it is
 * still ahead, tomorrow's once it has gone by, and the day the trigger
 * actually accepts is the backend's decision rather than this one's: only it
 * knows that a routine set on a Friday evening for the weekday morning means
 * Monday.
 */
export function secondsUntil(time: string, from: number = Date.now()): number | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(time.trim());
  if (!match) return null;
  const hours = Number(match[1]);
  const minutes = Number(match[2]);
  if (hours > 23 || minutes > 59) return null;

  const target = new Date(from);
  target.setHours(hours, minutes, 0, 0);
  // Built by mutating a local date rather than by arithmetic on milliseconds,
  // so the day a clock change shortens or lengthens still lands on the hour
  // that was asked for.
  if (target.getTime() <= from) target.setDate(target.getDate() + 1);

  return Math.round((target.getTime() - from) / 1000);
}
