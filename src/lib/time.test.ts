import { describe, expect, it } from "vitest";

import { elapsed, relativeTime, whenLabel } from "./time";

const NOW = 1_700_000_000_000;
const ago = (ms: number) => relativeTime(NOW - ms, NOW);

describe("relativeTime", () => {
  it("reads as now for anything recent", () => {
    expect(ago(0)).toBe("now");
    expect(ago(30_000)).toBe("now");
  });

  it("steps up through minutes, hours, days, and weeks", () => {
    expect(ago(5 * 60_000)).toBe("5m");
    expect(ago(3 * 3_600_000)).toBe("3h");
    expect(ago(2 * 86_400_000)).toBe("2d");
    expect(ago(21 * 86_400_000)).toBe("3w");
  });

  it("never rounds down to zero", () => {
    // "0m" would read as broken. A minute is the smallest unit above "now".
    expect(ago(50_000)).toBe("1m");
  });

  it("stays short enough for a narrow sidebar column", () => {
    for (const ms of [0, 60_000, 3_600_000, 86_400_000, 30 * 86_400_000]) {
      expect(ago(ms).length).toBeLessThanOrEqual(4);
    }
  });

  it("does not go negative when a clock skews forward", () => {
    expect(relativeTime(NOW + 5_000, NOW)).toBe("now");
  });
});

describe("whenLabel", () => {
  /** 2pm on a Wednesday, local, so "yesterday" and "last week" are unambiguous. */
  const wednesday = new Date(2024, 4, 15, 14, 0).getTime();
  const day = 86_400_000;

  it("names today and yesterday rather than dating them", () => {
    expect(whenLabel(wednesday, wednesday)).toMatch(/^Today /);
    expect(whenLabel(wednesday - day, wednesday)).toMatch(/^Yesterday /);
  });

  it("names the weekday while the name still means one day", () => {
    expect(whenLabel(wednesday - 3 * day, wednesday)).toMatch(/^Sunday /);
  });

  it("dates anything a weekday name would be ambiguous about", () => {
    // "Tuesday" three weeks back is four different Tuesdays.
    expect(whenLabel(wednesday - 21 * day, wednesday)).not.toMatch(/day /);
    expect(whenLabel(wednesday - 21 * day, wednesday)).toMatch(/24/);
  });

  it("counts calendar days, not multiples of 24 hours", () => {
    // Eleven at night and one in the morning are two hours apart and two
    // different days, and an hour ago is never "yesterday".
    const lateWednesday = new Date(2024, 4, 15, 23, 0).getTime();
    const earlyThursday = new Date(2024, 4, 16, 1, 0).getTime();
    expect(whenLabel(lateWednesday, earlyThursday)).toMatch(/^Yesterday /);
    expect(whenLabel(new Date(2024, 4, 16, 0, 30).getTime(), earlyThursday)).toMatch(/^Today /);
  });

  it("carries the clock, because a day on its own does not place a message", () => {
    expect(whenLabel(wednesday, wednesday)).toMatch(/\d{1,2}[:.]\d\d/);
  });
});

describe("elapsed", () => {
  const waiting = (ms: number) => elapsed(NOW, NOW - ms);

  it("counts seconds, where relativeTime would say now", () => {
    // Read while waiting on a call that has not come back, where the whole
    // question is whether the number is still moving. "now" for the first
    // forty-four seconds of a wait answers it wrong.
    expect(waiting(4_000)).toBe("4s");
    expect(waiting(59_000)).toBe("59s");
  });

  it("counts from zero, and leaves what is worth reporting to the caller", () => {
    expect(waiting(0)).toBe("0s");
    expect(waiting(900)).toBe("0s");
  });

  it("keeps the seconds once there are minutes, padded so they do not jump", () => {
    expect(waiting(60_000)).toBe("1m 00s");
    expect(waiting(184_000)).toBe("3m 04s");
  });

  it("does not count backward from a clock that moved", () => {
    expect(elapsed(NOW, NOW + 5_000)).toBe("0s");
  });
});
