import { describe, expect, it } from "vitest";

import { cadence, relativeTime, splitGap, toSeconds } from "./time";

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

describe("splitGap", () => {
  it("says a gap in the largest whole unit that divides it", () => {
    // 7200 seconds is "2 hours". Saying it in seconds is the same fact told
    // badly, and it is what the operator typed in the first place.
    expect(splitGap(7200)).toEqual({ value: 2, unit: "hours" });
    expect(splitGap(86_400)).toEqual({ value: 1, unit: "days" });
    expect(splitGap(600)).toEqual({ value: 10, unit: "minutes" });
  });

  it("still draws a gap nothing in the editor could have produced", () => {
    // The editor cannot set this and the backend refuses it, but a row written
    // by an agent's own tool still has to render.
    expect(splitGap(30)).toEqual({ value: 1, unit: "minutes" });
    expect(splitGap(5400)).toEqual({ value: 90, unit: "minutes" });
  });

  it("round trips through seconds", () => {
    for (const secs of [60, 600, 3600, 7200, 86_400, 172_800]) {
      const { value, unit } = splitGap(secs);
      expect(toSeconds(value, unit)).toBe(secs);
    }
  });
});

describe("cadence", () => {
  it("reads as a person would say it", () => {
    expect(cadence(3600)).toBe("every hour");
    expect(cadence(7200)).toBe("every 2 hours");
    expect(cadence(86_400)).toBe("every day");
  });

  it("says once for a routine that does not repeat", () => {
    expect(cadence(null)).toBe("once");
  });
});
