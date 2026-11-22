import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  forcedSeason,
  seasonOn,
  thanksgiving,
  untilMidnight,
  useSeason,
  watchSeason,
} from "./season";

const day = (year: number, month: number, date: number, hour = 12) =>
  new Date(year, month - 1, date, hour);

afterEach(() => {
  vi.useRealTimers();
});

describe("the day a season is on", () => {
  it.each([
    [2026, 0, 26],
    [2027, 1, 25],
    [2022, 2, 24],
    [2023, 3, 23],
    [2029, 4, 22],
    [2024, 5, 28],
    [2025, 6, 27],
  ])(
    "finds Thanksgiving in %i, whose November starts on weekday %i, on the %ith",
    (year, weekday, date) => {
      expect(new Date(year, 10, 1).getDay()).toBe(weekday);
      expect(thanksgiving(year)).toBe(date);
      expect(new Date(year, 10, date).getDay()).toBe(4);
      expect(seasonOn(day(year, 11, date))).toBe("thanksgiving");
      expect(seasonOn(day(year, 11, date - 7))).toBeNull();
      expect(seasonOn(day(year, 11, date + 7))).toBeNull();
    },
  );

  it("takes the first Thursday as the first week when November starts on one", () => {
    expect(new Date(2029, 10, 1).getDay()).toBe(4);
    expect(seasonOn(day(2029, 11, 22))).toBe("thanksgiving");
    expect(seasonOn(day(2029, 11, 29))).toBeNull();
  });

  it("is Thanksgiving on November 26, 2026, and not the day either side", () => {
    expect(seasonOn(day(2026, 11, 25, 23))).toBeNull();
    expect(seasonOn(day(2026, 11, 26, 0))).toBe("thanksgiving");
    expect(seasonOn(day(2026, 11, 26, 23))).toBe("thanksgiving");
    expect(seasonOn(day(2026, 11, 27, 0))).toBeNull();
  });

  it("is off across the year boundary and on an ordinary day", () => {
    expect(seasonOn(day(2026, 12, 31, 23))).toBeNull();
    expect(seasonOn(day(2027, 1, 1, 0))).toBeNull();
    expect(seasonOn(day(2026, 11, 22))).toBeNull();
    expect(seasonOn(day(2026, 7, 4))).toBeNull();
  });
});

describe("the next local midnight", () => {
  it("is the time left in today, never zero", () => {
    expect(untilMidnight(day(2026, 11, 25, 23))).toBe(60 * 60 * 1000);
    expect(untilMidnight(new Date(2026, 11, 31, 23, 59, 59, 999))).toBe(1);
    expect(untilMidnight(day(2026, 11, 26, 0))).toBe(24 * 60 * 60 * 1000);
  });
});

describe("a window left open across midnight", () => {
  it("turns the season on at midnight and off again at the next one", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 10, 25, 23, 59, 30));
    const seen: (string | null)[] = [];
    const stop = watchSeason((season) => seen.push(season));
    expect(seen).toEqual([null]);

    vi.advanceTimersByTime(29_000);
    expect(seen.at(-1)).toBeNull();
    vi.advanceTimersByTime(1_000);
    expect(seen.at(-1)).toBe("thanksgiving");

    vi.advanceTimersByTime(24 * 60 * 60 * 1000 - 1);
    expect(seen.at(-1)).toBe("thanksgiving");
    vi.advanceTimersByTime(1);
    expect(seen.at(-1)).toBeNull();
    stop();
  });

  it("checks the date again when the window is focused or shown, since a sleeping laptop skips timers", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 10, 25, 22, 0));
    let shown: string | null = "unset";
    const stop = watchSeason((season) => {
      shown = season;
    });
    expect(shown).toBeNull();

    vi.setSystemTime(new Date(2026, 10, 26, 9, 0));
    expect(shown).toBeNull();
    window.dispatchEvent(new Event("focus"));
    expect(shown).toBe("thanksgiving");

    vi.setSystemTime(new Date(2026, 10, 27, 9, 0));
    document.dispatchEvent(new Event("visibilitychange"));
    expect(shown).toBeNull();
    stop();
  });

  it("goes back when the clock is set back past midnight, because it recomputes rather than counting down", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 10, 26, 0, 5));
    let shown: string | null = "unset";
    const stop = watchSeason((season) => {
      shown = season;
    });
    expect(shown).toBe("thanksgiving");
    vi.setSystemTime(new Date(2026, 10, 25, 23, 50));
    window.dispatchEvent(new Event("focus"));
    expect(shown).toBeNull();
    stop();
  });

  it("stops listening once it is let go", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 10, 25, 23, 0));
    const seen: (string | null)[] = [];
    const stop = watchSeason((season) => seen.push(season));
    stop();
    vi.setSystemTime(new Date(2026, 10, 26, 9, 0));
    window.dispatchEvent(new Event("focus"));
    vi.advanceTimersByTime(2 * 60 * 60 * 1000);
    expect(seen).toEqual([null]);
  });
});

describe("the hook a window reads", () => {
  it("follows the date across midnight", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 10, 25, 23, 59));
    const { result } = renderHook(() => useSeason(true));
    expect(result.current).toBeNull();
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    expect(result.current).toBe("thanksgiving");
  });

  it("is off whatever the date when seasonal looks are switched off", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 10, 26, 12));
    const { result, rerender } = renderHook(({ on }) => useSeason(on), {
      initialProps: { on: false },
    });
    expect(result.current).toBeNull();
    rerender({ on: true });
    expect(result.current).toBe("thanksgiving");
    rerender({ on: false });
    expect(result.current).toBeNull();
  });
});

describe("forcing a season to preview it", () => {
  it("is refused outside a development build, whatever is asked", () => {
    for (const asked of ["thanksgiving", "none"]) {
      expect(forcedSeason(asked, false)).toBeUndefined();
    }
  });

  it("names a season, none, or nothing it recognizes in a development build", () => {
    expect(forcedSeason("thanksgiving", true)).toBe("thanksgiving");
    expect(forcedSeason("none", true)).toBeNull();
    expect(forcedSeason("christmas", true)).toBeUndefined();
    expect(forcedSeason("constructor", true)).toBeUndefined();
    expect(forcedSeason(null, true)).toBeUndefined();
  });
});
