import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { PULSE_WINDOW_MS, useStore } from "../lib/store";
import type { Tokens } from "../lib/types";
import { bars, compact, money, priced, TokenMeter } from "./TokenMeter";

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

describe("priced", () => {
  it("says no to the two ways a provider reports no charge", () => {
    // A local server prices nothing and reports null. A free model prices every
    // call at a real zero. Neither has anything to say in a narrow rail.
    expect(priced(null)).toBe(false);
    expect(priced(undefined)).toBe(false);
    expect(priced(0)).toBe(false);
  });

  it("says no to a price that would draw as zeros anyway", () => {
    // money() pads to four places, so anything under a ten-thousandth of a
    // dollar is $0.0000: the same nothing, at more precision.
    expect(priced(0.000_02)).toBe(false);
    expect(money(0.000_02)).toBe("$0.0000");
  });

  it("says yes as soon as there is a digit to draw", () => {
    expect(priced(0.0001)).toBe(true);
    expect(priced(4.2)).toBe(true);
  });
});

describe("TokenMeter", () => {
  const GROUP = "00000000-0000-4000-8000-000000000001";

  function draw(total: Tokens | undefined, points: { at: number; tokens: number }[] = []) {
    useStore.setState({
      usage: total ? { [GROUP]: total } : {},
      pulse: { [GROUP]: points },
    });
    return render(<TokenMeter groupId={GROUP} />);
  }

  function spent(over: Partial<Tokens> = {}): Tokens {
    return { prompt: 1200, completion: 300, cost: null, calls: 4, ...over };
  }

  // The count is the invariant: it is the one figure every provider produces
  // and the one that climbs while a crew works.
  it.each<[string, number | null]>([
    ["a paid model", 0.0234],
    ["a free model", 0],
    ["a local server", null],
  ])("draws the count under %s", (_what, cost) => {
    const { container } = draw(spent({ cost }));
    expect(container.querySelector(".meter__count")?.textContent).toBe("1.5k");
  });

  it("drops the price a free model reports, and keeps the count", () => {
    // The whole complaint: free inference charged a real zero, the price won
    // the one slot, and the rail spent seven characters on $0.0000 while the
    // only figure going anywhere was not drawn at all.
    const { container } = draw(spent({ cost: 0 }));
    expect(container.textContent).toContain("1.5k");
    expect(container.textContent).not.toContain("$");
    expect(container.querySelector(".meter")?.getAttribute("title")).not.toContain("$");
  });

  it("draws the price beside the count once there is one", () => {
    const { container } = draw(spent({ cost: 0.0234 }));
    expect(container.textContent).toContain("$0.023");
    expect(container.querySelector(".meter")?.getAttribute("title")).toContain("$0.023");
  });

  it("draws nothing at all for a group that has never spent anything", () => {
    // An idle rail is not a column of zeros.
    const { container } = draw(undefined);
    expect(container.textContent).toBe("");
  });
});
