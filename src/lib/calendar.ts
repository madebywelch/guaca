/**
 * What a calendar shows, and in what order.
 *
 * The whole of the view's reasoning, with no DOM in it, for the reason
 * `lib/transcript.ts` and `lib/trail.ts` are the same shape: a panel that
 * decides what to draw while it draws it is a panel nothing can assert about,
 * and every rule here is one somebody has to be able to disagree with in a
 * test.
 *
 * Two decisions live here and nowhere else.
 *
 * **A calendar is grouped by day, not listed by time.** A flat list sorted by
 * `startsAt` is what the store hands back and reads as a feed: nothing in it
 * says where Tuesday ends. Days are the unit a person plans in, so days are the
 * unit this returns, including the ones with nothing on them inside a window
 * the operator asked for — an empty Thursday is information.
 *
 * **A window is a month of days, moved a month at a time.** Not "the next
 * thirty days", which slides under the operator and never lines up with what
 * anybody calls September; and not a rolling infinite scroll, which has no
 * answer to "show me October".
 */

import type { GroupId, Occasion } from "./types";

/** A day, and everything happening on it. Empty days are days too. */
export interface Day {
  /** Local midnight. The key, and what everything else is derived from. */
  at: number;
  /** Soonest first, all-day occasions before timed ones. */
  occasions: Occasion[];
}

/** The window a view is showing, as two instants and the month it names. */
export interface Window {
  /** Local midnight of the first day, inclusive. */
  from: number;
  /** Local midnight of the day after the last, exclusive. */
  until: number;
}

/** Local midnight of the day a moment falls on. */
export function dayOf(at: number): number {
  const day = new Date(at);
  day.setHours(0, 0, 0, 0);
  return day.getTime();
}

/**
 * The whole of the month `at` falls in, as a window.
 *
 * Built by walking the `Date` rather than by adding milliseconds, because a
 * month is four different lengths and two days a year are 23 or 25 hours long.
 * `offset` moves whole months: `-1` is the month before, `+1` the month after.
 */
export function monthOf(at: number, offset = 0): Window {
  const start = new Date(at);
  start.setHours(0, 0, 0, 0);
  start.setDate(1);
  // Set before the month, or the 31st of a 31-day month walked forward into a
  // 30-day one lands on the 1st of the month after that.
  start.setMonth(start.getMonth() + offset);

  const end = new Date(start);
  end.setMonth(end.getMonth() + 1);

  return { from: start.getTime(), until: end.getTime() };
}

/**
 * The window an operator opens on: this month, and the rest of the next one.
 *
 * Not the calendar month alone, which on the 29th is a calendar showing two
 * days. What the question "what is coming" means at the end of a month is
 * mostly next month, so the default window runs to the end of it and the view
 * still names the month it started in.
 */
export function openingWindow(now: number): Window {
  return { from: monthOf(now).from, until: monthOf(now, 1).until };
}

/**
 * Every day in a window, with what falls on each.
 *
 * Occasions outside the window are dropped rather than clamped into the nearest
 * day: a view showing September must not draw an August meeting on the 1st.
 *
 * Within a day, all-day occasions come first. They are the day's frame — a
 * filing deadline, a holiday, a launch — and a timed one sorted against them by
 * `startsAt` would put every deadline before the 9am and read as the first
 * appointment of the morning.
 */
export function daysIn(occasions: Occasion[], window: Window): Day[] {
  const byDay = new Map<number, Occasion[]>();
  for (const one of occasions) {
    if (one.startsAt < window.from || one.startsAt >= window.until) continue;
    const key = dayOf(one.startsAt);
    const held = byDay.get(key);
    if (held) held.push(one);
    else byDay.set(key, [one]);
  }

  const days: Day[] = [];
  // Walked as dates rather than stepped by 86,400,000, so the two days a year
  // that are not 24 hours long neither double nor vanish.
  const walk = new Date(window.from);
  while (walk.getTime() < window.until) {
    const at = walk.getTime();
    days.push({ at, occasions: (byDay.get(at) ?? []).sort(compare) });
    walk.setDate(walk.getDate() + 1);
    // A day that begins in a DST gap lands an hour in; a day that ends in one
    // lands an hour short. Both are the same local date, and midnight is what
    // every key here is.
    walk.setHours(0, 0, 0, 0);
  }
  return days;
}

/** Days with something on them. What an agenda draws. */
export function busyDays(days: Day[]): Day[] {
  return days.filter((day) => day.occasions.length > 0);
}

/** All-day first, then by time, then by title so a tie is stable. */
function compare(one: Occasion, other: Occasion): number {
  if (one.allDay !== other.allDay) return one.allDay ? -1 : 1;
  if (one.startsAt !== other.startsAt) return one.startsAt - other.startsAt;
  return one.title.localeCompare(other.title);
}

/**
 * What a day is called: `Today`, `Tomorrow`, or `Thursday 14 September`.
 *
 * Named for the two days a name is unambiguous for and dated for every other,
 * which is the rule `whenLabel` follows in `lib/time.ts`. "Thursday" three
 * weeks out means nothing, and a calendar is mostly weeks out.
 */
export function dayLabel(at: number, now: number): string {
  const days = Math.round((dayOf(at) - dayOf(now)) / 86_400_000);
  if (days === 0) return "Today";
  if (days === 1) return "Tomorrow";
  if (days === -1) return "Yesterday";
  return new Date(at).toLocaleDateString([], {
    weekday: "long",
    day: "numeric",
    month: "long",
  });
}

/**
 * When an occasion happens, as one line: `3:00 PM`, `3:00 – 4:00 PM`, `All day`.
 *
 * The end is only drawn when there is one. Most of what lands on this calendar
 * is a deadline or a reminder with no length, and a range invented for it would
 * say a filing takes half an hour.
 */
export function timeLabel(occasion: Occasion): string {
  if (occasion.allDay) return "All day";

  const clock = (at: number) =>
    new Date(at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const start = clock(occasion.startsAt);
  if (occasion.minutes === null) return start;

  const end = clock(occasion.startsAt + occasion.minutes * 60_000);
  // The meridiem on the start is dropped when both halves share one, because
  // `3:00 PM – 4:00 PM` is the same word twice in a column six characters wide.
  const [, startMeridiem] = start.split(" ");
  const [, endMeridiem] = end.split(" ");
  const head = startMeridiem && startMeridiem === endMeridiem ? start.split(" ")[0] : start;
  return `${head} – ${end}`;
}

/** Whether an occasion has already finished. Drawn back, never hidden. */
export function isPast(occasion: Occasion, now: number): boolean {
  if (occasion.allDay) return dayOf(occasion.startsAt) < dayOf(now);
  const ends = occasion.startsAt + (occasion.minutes ?? 0) * 60_000;
  return ends < now;
}

/**
 * The next thing coming, across everything handed in.
 *
 * What the rail's badge counts down to. `null` when a calendar holds nothing
 * ahead, which is not the same as holding nothing: a workspace whose whole
 * calendar is last month has nothing to put on a badge.
 */
export function nextUp(occasions: Occasion[], now: number): Occasion | null {
  let soonest: Occasion | null = null;
  for (const one of occasions) {
    if (isPast(one, now)) continue;
    if (soonest === null || one.startsAt < soonest.startsAt) soonest = one;
  }
  return soonest;
}

/**
 * The crews an operator can filter down to, in the order the rail draws them.
 *
 * Derived from the groups rather than from the occasions, so a crew with an
 * empty calendar is still a filter: picking it and finding nothing is an
 * answer, and a chip that appeared only once somebody wrote a date would be a
 * filter you cannot use until you no longer need it.
 */
export function crewsWith(
  groups: { id: GroupId; name: string }[],
  occasions: Occasion[],
): { id: GroupId; name: string; count: number }[] {
  const counts = new Map<GroupId, number>();
  for (const one of occasions) counts.set(one.groupId, (counts.get(one.groupId) ?? 0) + 1);
  return groups.map((group) => ({ ...group, count: counts.get(group.id) ?? 0 }));
}
