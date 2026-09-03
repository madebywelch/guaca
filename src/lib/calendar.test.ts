import { describe, expect, it } from "vitest";

import {
  busyDays,
  crewsWith,
  dayLabel,
  dayOf,
  daysIn,
  isPast,
  monthOf,
  nextUp,
  openingWindow,
  timeLabel,
} from "./calendar";
import type { GroupId, Occasion } from "./types";

/** A local wall-clock moment, so every assertion here survives any timezone. */
function at(year: number, month: number, day: number, hour = 0, minute = 0): number {
  return new Date(year, month - 1, day, hour, minute, 0, 0).getTime();
}

let next = 0;
function occasion(over: Partial<Occasion> = {}): Occasion {
  next += 1;
  return {
    id: `occasion-${next}`,
    groupId: "crew-1",
    agentId: null,
    title: "Board call",
    detail: "",
    place: "",
    startsAt: at(2026, 9, 14, 15),
    minutes: 60,
    allDay: false,
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

describe("a month", () => {
  it("is the whole of the one a moment falls in", () => {
    const window = monthOf(at(2026, 9, 14, 15));
    expect(window.from).toBe(at(2026, 9, 1));
    expect(window.until).toBe(at(2026, 10, 1));
  });

  it("moves a month at a time without walking off the end of a short one", () => {
    // The 31st of a 31-day month stepped forward into a 30-day one lands on the
    // 1st of the month after that, which is how a "next month" button skips
    // one. The day is set before the month for exactly this.
    const window = monthOf(at(2026, 1, 31), 1);
    expect(window.from).toBe(at(2026, 2, 1));
    expect(window.until).toBe(at(2026, 3, 1));
  });

  it("crosses a year boundary in both directions", () => {
    expect(monthOf(at(2026, 12, 15), 1).from).toBe(at(2027, 1, 1));
    expect(monthOf(at(2026, 1, 15), -1).from).toBe(at(2025, 12, 1));
  });

  it("opens on this month and the rest of the next", () => {
    // A calendar opened on the 29th and showing only the calendar month is a
    // calendar showing two days, and "what is coming" at the end of a month is
    // mostly next month.
    const window = openingWindow(at(2026, 9, 29, 10));
    expect(window.from).toBe(at(2026, 9, 1));
    expect(window.until).toBe(at(2026, 11, 1));
  });
});

describe("grouping into days", () => {
  it("gives every day in the window a place, including the empty ones", () => {
    // An empty Thursday is information. A list of only the busy days cannot say
    // that nothing is happening on one.
    const days = daysIn([occasion({ startsAt: at(2026, 9, 14, 15) })], monthOf(at(2026, 9, 1)));
    expect(days).toHaveLength(30);
    expect(days[0]!.at).toBe(at(2026, 9, 1));
    expect(days[0]!.occasions).toEqual([]);
    expect(days[13]!.occasions).toHaveLength(1);
  });

  it("drops what is outside the window rather than clamping it into view", () => {
    // A view showing September must not draw an August meeting on the 1st.
    const days = daysIn(
      [
        occasion({ startsAt: at(2026, 8, 31, 15) }),
        occasion({ startsAt: at(2026, 10, 1, 15) }),
        occasion({ startsAt: at(2026, 9, 14, 15) }),
      ],
      monthOf(at(2026, 9, 1)),
    );
    expect(busyDays(days)).toHaveLength(1);
    expect(busyDays(days)[0]!.at).toBe(at(2026, 9, 14));
  });

  it("puts the day's frame before its appointments", () => {
    // An all-day occasion is what the day is, not the first thing in it.
    // Sorted against timed ones by `startsAt`, a deadline at local midnight
    // reads as the first appointment of the morning.
    const day = daysIn(
      [
        occasion({ title: "Nine o'clock", startsAt: at(2026, 9, 14, 9) }),
        occasion({ title: "Filing due", startsAt: at(2026, 9, 14), allDay: true, minutes: null }),
        occasion({ title: "Three o'clock", startsAt: at(2026, 9, 14, 15) }),
      ],
      monthOf(at(2026, 9, 1)),
    )[13]!;

    expect(day.occasions.map((one) => one.title)).toEqual([
      "Filing due",
      "Nine o'clock",
      "Three o'clock",
    ]);
  });

  it("orders two things at one time by name, so the list does not shuffle", () => {
    const day = daysIn(
      [
        occasion({ title: "Zebra", startsAt: at(2026, 9, 14, 9) }),
        occasion({ title: "Alpha", startsAt: at(2026, 9, 14, 9) }),
      ],
      monthOf(at(2026, 9, 1)),
    )[13]!;
    expect(day.occasions.map((one) => one.title)).toEqual(["Alpha", "Zebra"]);
  });

  it("counts the days of a month a clock change runs through", () => {
    // March has 31 days whether or not one of them is 23 hours long. Stepped by
    // 86,400,000 the short day is skipped and the month comes out at 30.
    const days = daysIn([], monthOf(at(2026, 3, 1)));
    expect(days).toHaveLength(31);
    expect(days.every((day) => day.at === dayOf(day.at))).toBe(true);
  });
});

describe("what a day is called", () => {
  const now = at(2026, 9, 14, 10);

  it("names the two days a name is unambiguous for", () => {
    expect(dayLabel(at(2026, 9, 14, 23), now)).toBe("Today");
    expect(dayLabel(at(2026, 9, 15, 1), now)).toBe("Tomorrow");
    expect(dayLabel(at(2026, 9, 13, 1), now)).toBe("Yesterday");
  });

  it("dates every other one, because a weekday three weeks out means nothing", () => {
    const said = dayLabel(at(2026, 10, 8), now);
    expect(said).toContain("Thursday");
    expect(said).toContain("8");
    expect(said).toContain("October");
  });

  it("is a day rather than a moment, so late tonight is still today", () => {
    expect(dayLabel(at(2026, 9, 14, 23, 59), at(2026, 9, 14, 0, 1))).toBe("Today");
  });
});

describe("when an occasion happens", () => {
  it("draws a range when there is one and a moment when there is not", () => {
    expect(timeLabel(occasion({ startsAt: at(2026, 9, 14, 15), minutes: 60 }))).toBe(
      "3:00 – 4:00 PM",
    );
    // Most of what lands here is a deadline. A range invented for one would say
    // a filing takes half an hour.
    expect(timeLabel(occasion({ startsAt: at(2026, 9, 14, 15), minutes: null }))).toBe("3:00 PM");
  });

  it("keeps both meridiems when the pair straddles noon", () => {
    expect(timeLabel(occasion({ startsAt: at(2026, 9, 14, 11, 30), minutes: 60 }))).toBe(
      "11:30 AM – 12:30 PM",
    );
  });

  it("says a whole day is one rather than drawing midnight", () => {
    // Midnight is a time somebody chose, and a filing shown as 12:00 AM is a
    // filing nobody reads as a deadline.
    expect(timeLabel(occasion({ allDay: true, minutes: null }))).toBe("All day");
  });
});

describe("what has already happened", () => {
  it("counts a meeting as over only once it has ended", () => {
    const one = occasion({ startsAt: at(2026, 9, 14, 15), minutes: 60 });
    expect(isPast(one, at(2026, 9, 14, 15, 30))).toBe(false);
    expect(isPast(one, at(2026, 9, 14, 16, 1))).toBe(true);
  });

  it("keeps a whole day current until the day is over", () => {
    const filing = occasion({ startsAt: at(2026, 9, 14), allDay: true, minutes: null });
    expect(isPast(filing, at(2026, 9, 14, 23, 59))).toBe(false);
    expect(isPast(filing, at(2026, 9, 15, 0, 1))).toBe(true);
  });

  it("treats a moment with no length as over the moment it passes", () => {
    const lapse = occasion({ startsAt: at(2026, 9, 14, 15), minutes: null });
    expect(isPast(lapse, at(2026, 9, 14, 14, 59))).toBe(false);
    expect(isPast(lapse, at(2026, 9, 14, 15, 1))).toBe(true);
  });
});

describe("the next thing coming", () => {
  const now = at(2026, 9, 14, 12);

  it("is the soonest that has not happened", () => {
    const soon = occasion({ title: "Three o'clock", startsAt: at(2026, 9, 14, 15) });
    const later = occasion({ title: "Tomorrow", startsAt: at(2026, 9, 15, 9) });
    const gone = occasion({ title: "This morning", startsAt: at(2026, 9, 14, 9) });
    expect(nextUp([later, gone, soon], now)?.title).toBe("Three o'clock");
  });

  it("is nothing when a whole calendar is behind us", () => {
    // Which is not the same as an empty calendar, and a badge counting down to
    // last month's meeting is worse than no badge.
    expect(nextUp([occasion({ startsAt: at(2026, 8, 1, 9) })], now)).toBeNull();
    expect(nextUp([], now)).toBeNull();
  });
});

describe("the crews you can filter to", () => {
  it("offers every crew, including the ones with an empty calendar", () => {
    // A chip that appeared only once somebody wrote a date is a filter you
    // cannot use until you no longer need it.
    const groups = [
      { id: "crew-1" as GroupId, name: "Ops" },
      { id: "crew-2" as GroupId, name: "Legal" },
    ];
    const crews = crewsWith(groups, [occasion({ groupId: "crew-1" })]);
    expect(crews.map((crew) => [crew.name, crew.count])).toEqual([
      ["Ops", 1],
      ["Legal", 0],
    ]);
  });

  it("keeps the order the rail draws them in", () => {
    const groups = [
      { id: "crew-2" as GroupId, name: "Legal" },
      { id: "crew-1" as GroupId, name: "Ops" },
    ];
    expect(crewsWith(groups, []).map((crew) => crew.name)).toEqual(["Legal", "Ops"]);
  });
});
