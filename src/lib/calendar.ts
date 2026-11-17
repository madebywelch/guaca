import type { GroupId, Occasion } from "./types";

export interface Day {
  at: number;
  occasions: Occasion[];
}

export interface Window {
  from: number;
  until: number;
}

export function dayOf(at: number): number {
  const day = new Date(at);
  day.setHours(0, 0, 0, 0);
  return day.getTime();
}

export function monthOf(at: number, offset = 0): Window {
  const start = new Date(at);
  start.setHours(0, 0, 0, 0);
  start.setDate(1);
  start.setMonth(start.getMonth() + offset);

  const end = new Date(start);
  end.setMonth(end.getMonth() + 1);

  return { from: start.getTime(), until: end.getTime() };
}

export function openingWindow(now: number): Window {
  return { from: monthOf(now).from, until: monthOf(now, 1).until };
}

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
  const walk = new Date(window.from);
  while (walk.getTime() < window.until) {
    const at = walk.getTime();
    days.push({ at, occasions: (byDay.get(at) ?? []).sort(compare) });
    walk.setDate(walk.getDate() + 1);
    walk.setHours(0, 0, 0, 0);
  }
  return days;
}

export function busyDays(days: Day[]): Day[] {
  return days.filter((day) => day.occasions.length > 0);
}

function compare(one: Occasion, other: Occasion): number {
  if (one.allDay !== other.allDay) return one.allDay ? -1 : 1;
  if (one.startsAt !== other.startsAt) return one.startsAt - other.startsAt;
  return one.title.localeCompare(other.title);
}

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

export function timeLabel(occasion: Occasion): string {
  if (occasion.allDay) return "All day";

  const clock = (at: number) =>
    new Date(at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const start = clock(occasion.startsAt);
  if (occasion.minutes === null) return start;

  const end = clock(occasion.startsAt + occasion.minutes * 60_000);
  const [, startMeridiem] = start.split(" ");
  const [, endMeridiem] = end.split(" ");
  const head = startMeridiem && startMeridiem === endMeridiem ? start.split(" ")[0] : start;
  return `${head} – ${end}`;
}

export function isPast(occasion: Occasion, now: number): boolean {
  if (occasion.allDay) return dayOf(occasion.startsAt) < dayOf(now);
  const ends = occasion.startsAt + (occasion.minutes ?? 0) * 60_000;
  return ends < now;
}

export function nextUp(occasions: Occasion[], now: number): Occasion | null {
  let soonest: Occasion | null = null;
  for (const one of occasions) {
    if (isPast(one, now)) continue;
    if (soonest === null || one.startsAt < soonest.startsAt) soonest = one;
  }
  return soonest;
}

export function crewsWith(
  groups: { id: GroupId; name: string }[],
  occasions: Occasion[],
): { id: GroupId; name: string; count: number }[] {
  const counts = new Map<GroupId, number>();
  for (const one of occasions) counts.set(one.groupId, (counts.get(one.groupId) ?? 0) + 1);
  return groups.map((group) => ({ ...group, count: counts.get(group.id) ?? 0 }));
}
