import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { seatOf } from "../avatars/Accent";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { CHARACTERS } from "../avatars/catalog";
import { FORM } from "../avatars/form";
import { DEFAULT_PREFS } from "../lib/prefs";
import { type Season, SeasonContext } from "../lib/season";
import { useStore } from "../lib/store";
import { SeasonalLooks, SeasonGarland, SeasonGreeting, SeasonScene } from "./Seasonal";

vi.mock("../avatars/clock", async (original) => ({
  ...(await original<typeof import("../avatars/clock")>()),
  join: () => () => {},
}));
vi.mock("../lib/motion", () => ({ prefersReducedMotion: () => true }));

function Window() {
  return (
    <>
      <AgentAvatar avatar="orb" color="#5a7d99" seed="a" />
      <AgentAvatar avatar="bean" color="#4f5f96" seed="b" />
      <SeasonGreeting />
      <SeasonScene />
      <SeasonGarland />
    </>
  );
}

function shown(container: HTMLElement) {
  return {
    accents: container.querySelectorAll(".avatar__accent").length,
    greeting: screen.queryByText(/Happy Thanksgiving/)?.textContent ?? null,
    scene: container.querySelector(".roster__scene") !== null,
    garland: container.querySelector(".pane__garland") !== null,
  };
}

beforeEach(() => {
  useStore.setState({ prefs: DEFAULT_PREFS });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("on an ordinary day", () => {
  it("draws nothing seasonal at all", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 10, 22, 12));
    const { container } = render(
      <SeasonalLooks>
        <Window />
      </SeasonalLooks>,
    );
    expect(shown(container)).toEqual({ accents: 0, greeting: null, scene: false, garland: false });
  });
});

describe("on the day", () => {
  it.each([["thanksgiving", new Date(2026, 10, 26, 9), "Happy Thanksgiving"]] as const)(
    "draws %s's greeting, accents and decorations",
    (season, at, greeting) => {
      vi.useFakeTimers();
      vi.setSystemTime(at);
      const { container } = render(
        <SeasonalLooks>
          <Window />
        </SeasonalLooks>,
      );
      expect(shown(container)).toEqual({ accents: 2, greeting, scene: true, garland: true });
      for (const accent of container.querySelectorAll(".avatar__accent")) {
        expect(accent.getAttribute("data-accent")).toBe(season);
      }
      for (const art of container.querySelectorAll("svg.decor")) {
        expect(art.getAttribute("aria-hidden")).toBe("true");
      }
    },
  );

  it("turns everything off when seasonal looks are switched off, and back on with the switch", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 10, 26, 9));
    const { container } = render(
      <SeasonalLooks>
        <Window />
      </SeasonalLooks>,
    );
    expect(shown(container).accents).toBe(2);

    act(() => useStore.getState().setPrefs({ seasonal: false }));
    expect(shown(container)).toEqual({ accents: 0, greeting: null, scene: false, garland: false });

    act(() => useStore.getState().setPrefs({ seasonal: true }));
    expect(shown(container).greeting).toBe("Happy Thanksgiving");
  });

  it("puts the decorations away at midnight in a window left open", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 10, 26, 23, 59));
    const { container } = render(
      <SeasonalLooks>
        <Window />
      </SeasonalLooks>,
    );
    expect(shown(container).greeting).toBe("Happy Thanksgiving");
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    expect(shown(container)).toEqual({ accents: 0, greeting: null, scene: false, garland: false });
  });
});

describe("an avatar's accent", () => {
  it("can be left off where a face stands for a choice rather than an agent", () => {
    const { container } = render(
      <SeasonContext.Provider value="thanksgiving">
        <AgentAvatar avatar="orb" color="#5a7d99" plain />
      </SeasonContext.Provider>,
    );
    expect(container.querySelector(".avatar__accent")).toBeNull();
  });

  it("sits on top of every character, anchored to its own outline rather than its box", () => {
    for (const season of ["thanksgiving"] as Season[]) {
      const seats = new Set<string>();
      for (const lump of CHARACTERS) {
        const { x, y } = seatOf(lump, season);
        expect(x, lump.key).toBeGreaterThan(FORM.center);
        expect(y, lump.key).toBeLessThan(FORM.center - FORM.radius * 0.3);
        expect(Math.hypot(x - FORM.center, y - FORM.center), lump.key).toBeLessThan(FORM.reach);
        seats.add(`${x.toFixed(1)},${y.toFixed(1)}`);
      }
      expect(seats.size).toBeGreaterThan(CHARACTERS.length / 2);
    }
  });
});
