import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { PULSE_WINDOW_MS, useStore } from "../lib/store";
import type { Tokens } from "../lib/types";
import { bars, compact, money, priced, SpendTag } from "./Spend";

describe("compact", () => {
  it("is exact while the numbers are small", () => {
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
    const drawn = bars([{ at: now - PULSE_WINDOW_MS - 1, tokens: 999 }], now);
    expect(drawn.every((v) => v === 0)).toBe(true);
  });

  it("keeps a fixed width whatever it is given", () => {
    expect(bars([], now)).toHaveLength(bars([{ at: now, tokens: 1 }], now).length);
  });
});

describe("priced", () => {
  it("says no to the two ways a provider reports no charge", () => {
    expect(priced(null)).toBe(false);
    expect(priced(undefined)).toBe(false);
    expect(priced(0)).toBe(false);
  });

  it("says no to a price that would draw as zeros anyway", () => {
    expect(priced(0.000_02)).toBe(false);
    expect(money(0.000_02)).toBe("$0.0000");
  });

  it("says yes as soon as there is a digit to draw", () => {
    expect(priced(0.0001)).toBe(true);
    expect(priced(4.2)).toBe(true);
  });
});

describe("SpendTag", () => {
  const GROUP = "00000000-0000-4000-8000-000000000001";

  function draw(total: Tokens | undefined, points: { at: number; tokens: number }[] = []) {
    useStore.setState({
      usage: total ? { [GROUP]: total } : {},
      pulse: { [GROUP]: points },
    });
    return render(<SpendTag groupId={GROUP} at={{ x: 0, y: 0 }} />);
  }

  function spent(over: Partial<Tokens> = {}): Tokens {
    return { prompt: 1200, completion: 300, cost: null, calls: 4, ...over };
  }

  it.each<[string, number | null]>([
    ["a paid model", 0.0234],
    ["a free model", 0],
    ["a local server", null],
  ])("leads with the count under %s", (_what, cost) => {
    const { container } = draw(spent({ cost }));
    expect(container.querySelector(".spend__total")?.textContent).toBe("1.5k");
  });

  it("drops the price a free model reports, and keeps the count", () => {
    const { container } = draw(spent({ cost: 0 }));
    expect(container.textContent).toContain("1.5k");
    expect(container.textContent).not.toContain("$");
  });

  it("draws the price beside the count once there is one", () => {
    const { container } = draw(spent({ cost: 0.0234 }));
    expect(container.querySelector(".spend__cost")?.textContent).toBe("$0.023");
  });

  it("splits the total, in full, rather than rounding it twice", () => {
    const { container } = draw(spent({ prompt: 1_200_000, completion: 345_000 }));
    const figures = [...container.querySelectorAll(".spend__figure")].map((n) => n.textContent);

    expect(container.querySelector(".spend__total")?.textContent).toBe("1.5M");
    expect(figures).toEqual(["1,200,000", "345,000", "4"]);
  });

  it("answers a crew that has never spent anything, rather than opening empty", () => {
    const { container } = draw(undefined);
    expect(container.querySelector(".spend__quiet")?.textContent).toBe("Nothing spent yet.");
  });

  it("says nothing to a screen reader, because the numbers have a home", () => {
    const { container } = draw(spent());
    expect(container.querySelector(".spend")?.getAttribute("aria-hidden")).toBe("true");
  });
});
