import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { expectRestart, forgetRestart, restarting } from "./link";

/**
 * A host an update restarts goes away on purpose, and the window must not call
 * that a host that left. It is a length of time rather than a flag because the
 * read that would clear a flag is the one thing a host that is not answering
 * cannot provide.
 */
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
    // Long enough for a container to stop, copy its volume and pass a health
    // check, and no longer: a host still gone after this is not the update
    // working, and saying so is the point of the mark.
    vi.advanceTimersByTime(120_001);
    expect(restarting()).toBe(false);
  });

  it("moves with a restart that runs longer, as long as a read still says one is underway", () => {
    expectRestart();
    vi.advanceTimersByTime(150_000);
    expect(restarting()).toBe(true);
    // The panel read again, and the updater still says the update is running.
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
