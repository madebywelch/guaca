import { describe, expect, it } from "vitest";
import { aDecision, aGroup } from "../test-fixtures";
import {
  exchange,
  headline,
  liveReply,
  panelOrder,
  presenceOf,
  said,
  samePresence,
  workingCount,
} from "./menubar";
import { type StreamBuffer, useStore } from "./store";
import type { AgentCard, Envelope, Part, Participant } from "./types";

/**
 * The icon's view of the store, and what the panel under it decides about its
 * own. The first has to match what Rust reads, and `ipc.contract.test.ts`
 * holds the fields equal; this holds the values.
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
    runsErrands: false,
    browserConsent: "open",
    hasTerminal: false,
    harness: "pi",
    gate: "open",
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "m",
    subscriptionModel: "",
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
  it("hands the icon the live roster, the crews, and what is moving", () => {
    const state = {
      ...useStore.getState(),
      agents: [agent("Chef"), agent("Gone", { lifecycle: "terminated" })],
      groups: [aGroup({ name: "Kitchen" })],
      activity: { "id-Chef": { state: "thinking" as const } },
      activeRun: { "id-Chef": "run-1" as never, "id-Idle": undefined },
      sessionSpend: { prompt: 3, completion: 2, cost: null, calls: 1 },
    };
    const presence = presenceOf(state);

    // A terminated agent is out of the icon's count, as it is out of the rail.
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
  });

  it("counts unanswered and interrupted decisions, including snoozed ones", () => {
    const presence = presenceOf({
      ...useStore.getState(),
      decisions: [
        aDecision({ id: "pending", snoozedUntil: Date.now() + 3600000 }),
        aDecision({ id: "interrupted", status: "answered", interrupted: true }),
        aDecision({ id: "following", status: "answered" }),
        aDecision({ id: "done", status: "completed" }),
      ],
    });
    expect(presence.decisions.map((item) => item.id)).toEqual(["pending", "interrupted"]);
  });

  it("knows when nothing the icon draws has moved", () => {
    const a = presenceOf(useStore.getState());
    const b = presenceOf({ ...useStore.getState(), messages: { x: [] } });
    expect(samePresence(a, b)).toBe(true);
    const c = presenceOf({ ...useStore.getState(), agents: [agent("New")] });
    expect(samePresence(a, c)).toBe(false);
  });
});

/** A message between two ends of a channel, `at` milliseconds in. */
function message(
  from: Participant,
  to: Participant,
  text: string,
  at: number,
  parts: Part[] = [{ type: "text", text }],
): Envelope {
  return {
    id: `m-${at}`,
    runId: "run-1",
    channelId: "id-Chef",
    from,
    to,
    parts,
    trust: "operator",
    hop: 0,
    expectsReply: false,
    intent: "work",
    cause: null,
    createdAt: at,
  };
}

const YOU: Participant = { kind: "human" };
const CHEF: Participant = { kind: "agent", id: "id-Chef" };
const SCOUT: Participant = { kind: "agent", id: "id-Scout" };

describe("the panel's list", () => {
  const quiet = { activity: {}, building: {}, lastActive: {} };

  it("never lists an agent that has been let go", () => {
    const order = panelOrder([agent("Chef"), agent("Gone", { lifecycle: "terminated" })], quiet);
    expect(order.map((a) => a.name)).toEqual(["Chef"]);
  });

  it("puts whoever is working first, whatever the rail's order", () => {
    const order = panelOrder([agent("Ada"), agent("Bea"), agent("Cy")], {
      ...quiet,
      activity: { "id-Cy": { state: "thinking" } },
      lastActive: { "id-Ada": 1, "id-Bea": 2 },
    });
    expect(order.map((a) => a.name)).toEqual(["Cy", "Bea", "Ada"]);
  });

  // A coding job outlives the turn that started it, so the agent is idle to
  // the activity map for the twenty minutes it is writing code.
  it("counts an agent whose coding job is running as working", () => {
    const order = panelOrder([agent("Ada"), agent("Bea")], {
      ...quiet,
      building: { "id-Bea": "repo-1" },
    });
    expect(order.map((a) => a.name)).toEqual(["Bea", "Ada"]);
    expect(workingCount([agent("Ada"), agent("Bea")], {}, { "id-Bea": "repo-1" })).toBe(1);
  });

  it("keeps a turn parked on the operator with the working, and out of the count", () => {
    const parked = { "id-Bea": { state: "awaitingApproval" as const } };
    const order = panelOrder([agent("Ada"), agent("Bea")], {
      ...quiet,
      activity: parked,
      lastActive: { "id-Ada": 9 },
    });
    expect(order.map((a) => a.name)).toEqual(["Bea", "Ada"]);
    // It is waiting on a person, which the headline counts separately.
    expect(workingCount([agent("Ada"), agent("Bea")], parked, {})).toBe(0);
  });

  it("puts the paused last, and holds two that never spoke still", () => {
    const order = panelOrder([agent("Zed", { lifecycle: "paused" }), agent("Bo"), agent("Al")], {
      ...quiet,
      lastActive: { "id-Zed": 100 },
    });
    expect(order.map((a) => a.name)).toEqual(["Al", "Bo", "Zed"]);
  });
});

describe("the panel's headline", () => {
  it("says what is waiting before it says what is working", () => {
    expect(headline(2, 5)).toBe("2 waiting on you");
    expect(headline(0, 3)).toBe("3 working");
    expect(headline(0, 0)).toBe("Nothing running");
  });
});

describe("a conversation in the panel", () => {
  it("is only the operator and that agent, only in words, and only the last few", () => {
    const messages = [
      message(YOU, CHEF, "first", 1),
      message(CHEF, YOU, "second", 2),
      // A peer's message filed in the channel is the crew's traffic.
      message(SCOUT, CHEF, "between us", 3),
      // A reply with nothing to read in it.
      message(CHEF, YOU, "", 4, [{ type: "notice", kind: "upstreamError", text: "timed out" }]),
      message(YOU, CHEF, "third", 5),
      message(CHEF, YOU, "fourth", 6),
    ];
    expect(exchange(messages, "id-Chef", 3).map(said)).toEqual(["second", "third", "fourth"]);
  });

  it("is empty for a channel that has not been read", () => {
    expect(exchange(undefined, "id-Chef", 3)).toEqual([]);
  });

  it("reads every paragraph of a reply, and nothing that was not words", () => {
    const reply = message(CHEF, YOU, "", 1, [
      { type: "text", text: "  First.  " },
      { type: "json", name: "plan", value: {} },
      { type: "text", text: "Second." },
    ]);
    expect(said(reply)).toBe("First.\n\nSecond.");
  });

  it("shows a reply being written to the operator, and not one being written to a peer", () => {
    const stream = (over: Partial<StreamBuffer>): StreamBuffer => ({
      channelId: "id-Chef",
      agentId: "id-Chef",
      text: "Working on it",
      to: YOU,
      ...over,
    });
    expect(liveReply({ a: stream({}) }, "id-Chef")).toBe("Working on it");
    expect(liveReply({ a: stream({ to: SCOUT }) }, "id-Chef")).toBeNull();
    expect(liveReply({ a: stream({ text: "   " }) }, "id-Chef")).toBeNull();
    // Chef writing into Scout's channel is Chef's, and it is not this conversation.
    expect(liveReply({ a: stream({ channelId: "id-Scout" }) }, "id-Chef")).toBeNull();
  });
});
