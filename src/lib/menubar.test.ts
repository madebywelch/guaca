import { describe, expect, it } from "vitest";
import { aDecision, aGroup, anApproval } from "../test-fixtures";
import {
  exchange,
  headline,
  liveReply,
  panelOrder,
  presenceOf,
  presenceReporter,
  said,
  samePresence,
  workingCount,
} from "./menubar";
import { type StreamBuffer, useStore } from "./store";
import type { AgentCard, Envelope, Outcome, Part, Participant } from "./types";

describe("presence reports", () => {
  it("retries a failed report and sends the latest value after a delayed report", async () => {
    const sent: number[] = [];
    let finish!: () => void;
    const pending = new Promise<void>((resolve) => {
      finish = resolve;
    });
    let calls = 0;
    const report = presenceReporter(async (value: number) => {
      sent.push(value);
      if (++calls === 1) throw new Error("offline");
      if (calls === 2) await pending;
    });
    await report(1);
    const retry = report(1);
    const next = report(2);
    finish();
    await Promise.all([retry, next]);
    await report(2);
    expect(sent).toEqual([1, 1, 2]);
  });
});

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

  it("hands the icon a held answer as answered, so its amber clears until Undo", () => {
    const asking = anApproval({ agentId: "id-Chef" });
    const state = {
      ...useStore.getState(),
      agents: [agent("Chef")],
      pending: [asking],
      activity: { "id-Chef": { state: "awaitingApproval" as const } },
    };
    const held = presenceOf(state, [asking.id]);
    expect(held.waiting).toEqual([]);
    expect(held.activity["id-Chef"]).toEqual({ state: "thinking" });
    const undone = presenceOf(state);
    expect(undone.waiting).toEqual([asking]);
    expect(undone.activity["id-Chef"]).toEqual({ state: "awaitingApproval" });
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

  it("counts finished work the operator has not seen, and only work that is news", () => {
    const finished = (runId: string, over: Partial<Outcome>): Outcome => ({
      runId,
      agentId: "id-Chef",
      groupId: "00000000-0000-4000-8000-000000000001",
      trigger: "routine",
      triggerRef: null,
      status: "done",
      headline: "Done.",
      noteId: null,
      colleagues: [],
      needs: [],
      steps: 1,
      startedAt: 0,
      finishedAt: 1,
      seenAt: null,
      ...over,
    });
    const presence = presenceOf({
      ...useStore.getState(),
      outcomes: [
        finished("done", {}),
        finished("failed", { status: "failed" }),
        finished("seen", { seenAt: 2 }),
        finished("quiet", { status: "nothingNew" }),
      ],
    });
    expect(presence.finished).toBe(2);
  });

  it("knows when nothing the icon draws has moved", () => {
    const a = presenceOf(useStore.getState());
    const b = presenceOf({ ...useStore.getState(), messages: { x: [] } });
    expect(samePresence(a, b)).toBe(true);
    const c = presenceOf({ ...useStore.getState(), agents: [agent("New")] });
    expect(samePresence(a, c)).toBe(false);
  });
});

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
  const quiet = { activity: {}, lastActive: {} };

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

  it("keeps a turn parked on the operator with the working, and out of the count", () => {
    const parked = { "id-Bea": { state: "awaitingApproval" as const } };
    const order = panelOrder([agent("Ada"), agent("Bea")], {
      ...quiet,
      activity: parked,
      lastActive: { "id-Ada": 9 },
    });
    expect(order.map((a) => a.name)).toEqual(["Bea", "Ada"]);
    expect(workingCount([agent("Ada"), agent("Bea")], parked)).toBe(0);
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
    expect(headline(2, 5)).toBe("2 questions waiting on you");
    expect(headline(0, 3)).toBe("3 working");
    expect(headline(0, 0)).toBe("Nothing running");
  });
});

describe("a conversation in the panel", () => {
  it("is only the operator and that agent, only in words, and only the last few", () => {
    const messages = [
      message(YOU, CHEF, "first", 1),
      message(CHEF, YOU, "second", 2),
      message(SCOUT, CHEF, "between us", 3),
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
    expect(liveReply({ a: stream({ channelId: "id-Scout" }) }, "id-Chef")).toBeNull();
  });
});
