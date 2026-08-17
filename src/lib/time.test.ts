import { describe, expect, it } from "vitest";

import { relativeTime } from "./time";

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
