import { describe, expect, it } from "vitest";

import { COMPOST_DAYS, composted, goingSoon, timeLeft } from "./compost";
import type { AgentCard } from "./types";

const DAY = 24 * 60 * 60 * 1000;
const NOW = 1_700_000_000_000;

function agent(name: string, discardedAt: number | null): AgentCard {
  return {
    id: `id-${name}`,
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    repositoryId: null,
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "",
    systemPrompt: "",
    skills: [],
    lifecycle: discardedAt === null ? "active" : "terminated",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    discardedAt,
  };
}

/** Deleted this many days ago. */
const thrownOut = (name: string, daysAgo: number) => agent(name, NOW - daysAgo * DAY);

describe("who is in the compost", () => {
  it("is whoever carries a stamp, and nobody else", () => {
    const rows = composted([
      agent("Manager", null),
      thrownOut("Scribe", 1),
      // Terminated with no stamp: deleted before the compost existed, or
      // swept out of it. Its transcript still reads and its name is still
      // drawn there, but there is nothing left to put back.
      agent("Ghost", null),
    ]);

    expect(rows.map((row) => row.name)).toEqual(["Scribe"]);
  });

  it("is newest first, because that is what somebody came here for", () => {
    const rows = composted([thrownOut("Old", 20), thrownOut("New", 1), thrownOut("Middle", 9)]);

    expect(rows.map((row) => row.name)).toEqual(["New", "Middle", "Old"]);
  });
});

describe("how long is left", () => {
  it("counts down in days for most of the wait", () => {
    expect(timeLeft(thrownOut("a", 0), NOW)).toBe(`${COMPOST_DAYS} days left`);
    expect(timeLeft(thrownOut("a", 29), NOW)).toBe("1 day left");
  });

  it("drops to hours on the last day, which is when the unit starts to matter", () => {
    // Rounded down on purpose. An agent with three hours left is not one that
    // has a day, and telling somebody it does is the error that costs them the
    // agent.
    const threeHoursLeft = agent("a", NOW - COMPOST_DAYS * DAY + 3 * 60 * 60 * 1000);
    expect(timeLeft(threeHoursLeft, NOW)).toBe("3 hours left");

    const minutes = agent("a", NOW - COMPOST_DAYS * DAY + 90_000);
    expect(timeLeft(minutes, NOW)).toBe("less than an hour left");
  });

  it("says the wait is over rather than counting backwards", () => {
    // The sweep runs hourly, so a row can outlive its deadline by up to an
    // hour. "-0 days left" for that hour reads as something broken.
    expect(timeLeft(thrownOut("a", COMPOST_DAYS + 1), NOW)).toBe("going now");
  });

  it("says nothing at all about an agent that is not in there", () => {
    expect(timeLeft(agent("Manager", null), NOW)).toBe("");
  });
});

describe("what is about to go", () => {
  it("is marked inside the last week and not before it", () => {
    expect(goingSoon(thrownOut("a", COMPOST_DAYS - 6), NOW)).toBe(true);
    expect(goingSoon(thrownOut("a", COMPOST_DAYS - 8), NOW)).toBe(false);
  });

  it("covers an agent already past its deadline", () => {
    expect(goingSoon(thrownOut("a", COMPOST_DAYS + 1), NOW)).toBe(true);
  });

  it("is never true of an agent nobody deleted", () => {
    expect(goingSoon(agent("Manager", null), NOW)).toBe(false);
  });
});
