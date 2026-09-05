import { describe, expect, it } from "vitest";
import { aGroup } from "../test-fixtures";
import { presenceOf, samePresence } from "./menubar";
import { useStore } from "./store";
import type { AgentCard } from "./types";

/**
 * The strip's view of the store, which has to match what Rust reads locally
 * or the corner of the screen tells two different stories about one crew.
 */

function agent(name: string, over: Partial<AgentCard> = {}): AgentCard {
  return {
    id: `id-${name}`,
    railOrder: 0,
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    browserConsent: "open",
    repositoryId: null,
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "m",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    version: 1,
    createdAt: 1,
    updatedAt: 1,
    discardedAt: null,
    ...over,
  };
}

describe("presenceOf", () => {
  it("hands the strip the live roster, the crews, and what is moving", () => {
    const state = {
      ...useStore.getState(),
      agents: [agent("Chef"), agent("Gone", { lifecycle: "terminated" })],
      groups: [aGroup({ name: "Kitchen" })],
      activity: { "id-Chef": { state: "thinking" as const } },
      activeRun: { "id-Chef": "run-1" as never, "id-Idle": undefined },
      usage: {
        "00000000-0000-4000-8000-000000000001": { prompt: 10, completion: 5, cost: 0.5, calls: 2 },
        elsewhere: { prompt: 1, completion: 1, cost: null, calls: 1 },
      },
      sessionSpend: { prompt: 3, completion: 2, cost: null, calls: 1 },
    };
    const presence = presenceOf(state);

    // A terminated agent is out of the strip, as it is out of the rail.
    expect(Object.keys(presence.roster)).toEqual(["id-Chef"]);
    expect(presence.roster["id-Chef"]).toEqual({
      name: "Chef",
      crew: "00000000-0000-4000-8000-000000000001",
    });
    expect(presence.crews).toEqual([
      { id: "00000000-0000-4000-8000-000000000001", name: "Kitchen" },
    ]);
    expect(presence.running).toBe(1);
    expect(presence.session).toEqual({ prompt: 3, completion: 2, cost: null, calls: 1 });
    // Priced calls sum; an unpriced one does not turn the total into zero.
    expect(presence.allTime).toEqual({ prompt: 11, completion: 6, cost: 0.5, calls: 3 });
  });

  it("knows when nothing the strip draws has moved", () => {
    const a = presenceOf(useStore.getState());
    const b = presenceOf({ ...useStore.getState(), messages: { x: [] } });
    expect(samePresence(a, b)).toBe(true);
    const c = presenceOf({ ...useStore.getState(), agents: [agent("New")] });
    expect(samePresence(a, c)).toBe(false);
  });
});
