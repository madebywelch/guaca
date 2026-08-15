import { describe, expect, it } from "vitest";

import { PULSE_WINDOW_MS } from "../lib/store";
import { bars, compact, money } from "./TokenMeter";

describe("compact", () => {
  it("is exact while the numbers are small", () => {
    // The first calls of a session are the ones an operator is watching most
    // closely, and "0.1k" says less than "94".
    expect(compact(0)).toBe("0");
    expect(compact(94)).toBe("94");
    expect(compact(999)).toBe("999");
  });

  it("shortens once exactness stops helping", () => {
    expect(compact(1000)).toBe("1.0k");
    expect(compact(12_400)).toBe("12k");
    expect(compact(1_240_000)).toBe("1.2M");
  });
});

describe("money", () => {
  it("keeps enough places to show a crew that has only just started", () => {
    // Calls cost fractions of a cent. Two decimal places would report an hour
    // of work as $0.00.
    expect(money(0.001121)).toBe("$0.0011");
    expect(money(0.0234)).toBe("$0.023");
  });

  it("drops the places once they stop meaning anything", () => {
    expect(money(4.2)).toBe("$4.20");
    expect(money(128.4)).toBe("$128");
  });
});

describe("bars", () => {
  const now = 1_000_000;

  it("puts the newest activity on the right", () => {
    const drawn = bars([{ at: now, tokens: 40 }], now);
    expect(drawn[drawn.length - 1]).toBe(40);
    expect(drawn.slice(0, -1).every((v) => v === 0)).toBe(true);
  });

  it("sums calls that land in the same slice", () => {
    const drawn = bars(
      [
        { at: now, tokens: 40 },
        { at: now - 10, tokens: 2 },
      ],
      now,
    );
    expect(drawn[drawn.length - 1]).toBe(42);
  });

  it("drops what has scrolled out of the window", () => {
    // Otherwise a burst an hour ago would sit in the meter forever, which is
    // the opposite of showing whether something is happening now.
    const drawn = bars([{ at: now - PULSE_WINDOW_MS - 1, tokens: 999 }], now);
    expect(drawn.every((v) => v === 0)).toBe(true);
  });

  it("keeps a fixed width whatever it is given", () => {
    expect(bars([], now)).toHaveLength(bars([{ at: now, tokens: 1 }], now).length);
  });
});
