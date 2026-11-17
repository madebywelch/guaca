import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { expectRestart, forgetRestart, restarting } from "./link";

describe("a host an update is restarting", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    forgetRestart();
  });

  afterEach(() => vi.useRealTimers());

  it("is not a host that left, for as long as a restart takes", () => {
    expect(restarting()).toBe(false);
    expectRestart();
    expect(restarting()).toBe(true);
    vi.advanceTimersByTime(60_000);
    expect(restarting()).toBe(true);
    vi.advanceTimersByTime(120_001);
    expect(restarting()).toBe(false);
  });

  it("moves with a restart that runs longer, as long as a read still says one is underway", () => {
    expectRestart();
    vi.advanceTimersByTime(150_000);
    expect(restarting()).toBe(true);
    expectRestart();
    vi.advanceTimersByTime(150_000);
    expect(restarting()).toBe(true);
    vi.advanceTimersByTime(31_000);
    expect(restarting()).toBe(false);
  });

  it("is over the moment the host answers", () => {
    expectRestart();
    forgetRestart();
    expect(restarting()).toBe(false);
  });
});
