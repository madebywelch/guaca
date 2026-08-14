import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  AIM_MS,
  CATCH_MS,
  FLIGHT_MS,
  roleOf,
  STAGGER_MS,
  type StagedPulse,
  usePulseChoreography,
} from "./choreography";
import type { Pulse } from "./store";

function pulse(id: number, from: string, to: string): Pulse {
  return { id, from, to, color: "#c7d96b" };
}

describe("roleOf", () => {
  const staged = (phase: StagedPulse["phase"]): StagedPulse[] => [
    { id: 1, from: "a", to: "b", color: "#000", phase },
  ];

  it("has the sender wind up and shout while aiming", () => {
    const sender = roleOf(staged("aim"), "a");
    expect(sender.gesture).toBe("send");
    expect(sender.says).toBe("!");
    expect(sender.facing).toBe("b");
  });

  it("keeps both eggs facing each other through the flight", () => {
    // This is what makes it read as an exchange rather than two unrelated
    // animations that happen to overlap.
    for (const phase of ["aim", "flight"] as const) {
      expect(roleOf(staged(phase), "a").facing).toBe("b");
      expect(roleOf(staged(phase), "b").facing).toBe("a");
    }
  });

  it("only knocks the recipient back once the parcel lands", () => {
    expect(roleOf(staged("flight"), "b").gesture).toBeNull();
    expect(roleOf(staged("catch"), "b").gesture).toBe("receive");
  });

  it("releases the look once the exchange is over", () => {
    expect(roleOf(staged("catch"), "a").facing).toBeNull();
    expect(roleOf(staged("catch"), "b").facing).toBeNull();
  });

  it("leaves uninvolved agents alone", () => {
    expect(roleOf(staged("flight"), "someone-else")).toEqual({
      gesture: null,
      facing: null,
      says: null,
    });
  });
});

describe("pacing", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("gives each message a visible beat instead of firing them at once", () => {
    // The runtime delivers a fan-out within milliseconds. Animating that
    // literally is over before the eye can follow it.
    const done = vi.fn();
    const burst = [pulse(1, "m", "a"), pulse(2, "m", "b"), pulse(3, "m", "c")];
    const { result, rerender } = renderHook(({ pulses }) => usePulseChoreography(pulses, done), {
      initialProps: { pulses: burst },
    });
    rerender({ pulses: burst });

    act(() => {
      vi.advanceTimersByTime(100);
    });
    expect(result.current.staged).toHaveLength(1);

    act(() => {
      vi.advanceTimersByTime(STAGGER_MS);
    });
    expect(result.current.staged.length).toBeLessThanOrEqual(2);
    expect(result.current.staged.length).toBeGreaterThanOrEqual(1);
  });

  it("moves one message through aim, flight, and catch", () => {
    const done = vi.fn();
    const only = [pulse(1, "a", "b")];
    const { result, rerender } = renderHook(({ pulses }) => usePulseChoreography(pulses, done), {
      initialProps: { pulses: only },
    });
    rerender({ pulses: only });

    act(() => {
      vi.advanceTimersByTime(100);
    });
    expect(result.current.staged[0]?.phase).toBe("aim");
    expect(result.current.inFlight).toHaveLength(0);

    act(() => {
      vi.advanceTimersByTime(AIM_MS);
    });
    expect(result.current.staged[0]?.phase).toBe("flight");
    expect(result.current.inFlight).toHaveLength(1);

    act(() => {
      vi.advanceTimersByTime(FLIGHT_MS);
    });
    expect(result.current.staged[0]?.phase).toBe("catch");

    act(() => {
      vi.advanceTimersByTime(CATCH_MS);
    });
    expect(result.current.staged).toHaveLength(0);
    expect(done).toHaveBeenCalledWith(1);
  });

  it("is slow enough to watch", () => {
    // The whole point of this module. If these drop back to a few hundred
    // milliseconds, the exchange becomes a flicker again.
    expect(AIM_MS + FLIGHT_MS + CATCH_MS).toBeGreaterThanOrEqual(2000);
    expect(STAGGER_MS).toBeGreaterThanOrEqual(600);
  });

  it("never animates the same message twice", () => {
    const done = vi.fn();
    const only = [pulse(1, "a", "b")];
    const { rerender } = renderHook(({ pulses }) => usePulseChoreography(pulses, done), {
      initialProps: { pulses: only },
    });
    // Re-rendering with the same pulse must not queue it again.
    rerender({ pulses: only });
    rerender({ pulses: only });

    act(() => {
      vi.advanceTimersByTime(AIM_MS + FLIGHT_MS + CATCH_MS + 200);
    });
    expect(done).toHaveBeenCalledTimes(1);
  });

  it("drops the animation, never the message, when a backlog builds", () => {
    // Delivery already happened. A queue longer than anyone will watch just
    // means some throws go undrawn.
    const done = vi.fn();
    const flood = Array.from({ length: 40 }, (_, i) => pulse(i, "m", `a${i}`));
    const { rerender } = renderHook(({ pulses }) => usePulseChoreography(pulses, done), {
      initialProps: { pulses: flood },
    });
    rerender({ pulses: flood });

    expect(done.mock.calls.length).toBeGreaterThan(20);
  });
});
