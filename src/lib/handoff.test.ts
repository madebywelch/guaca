import { describe, expect, it } from "vitest";

import {
  foldLine,
  peerState,
  type Seen,
  settled,
  spanOf,
  statusWord,
  waitingLine,
} from "./handoff";
import type { PeerSummary } from "./transcript";
import type { AgentId, RunId } from "./types";

const RUN = "run" as RunId;

function peer(name: string, over: Partial<PeerSummary> = {}): PeerSummary {
  return {
    peer: { id: name, name, color: "#000", avatar: "blank" },
    agentId: name as AgentId,
    sent: 1,
    received: 0,
    gist: null,
    asked: "What changed?",
    askedAt: 1_000,
    answer: null,
    answeredAt: null,
    ...over,
  };
}

const nobody: Seen = { activity: {}, activeRun: {}, writing: new Set() };

describe("peerState", () => {
  it("is answered, with how long the answer took, once a reply lands", () => {
    const state = peerState(peer("Analyst", { received: 1, answeredAt: 10_000 }), RUN, nobody);
    expect(state).toEqual({ kind: "answered", took: 9_000 });
    expect(statusWord(state, 50_000)).toBe("answered in 9s");
  });

  it("says what the colleague is doing in this conversation, and for how long", () => {
    const seen: Seen = {
      activity: { Analyst: { state: "thinking" } },
      activeRun: { Analyst: RUN },
      writing: new Set(),
    } as unknown as Seen;
    expect(statusWord(peerState(peer("Analyst"), RUN, seen), 15_000)).toBe("working, 14s");
    const writing = { ...seen, writing: new Set(["Analyst"]) };
    expect(statusWord(peerState(peer("Analyst"), RUN, writing), 15_000)).toBe("writing, 14s");
  });

  it("does not call a colleague busy with another conversation working on this one", () => {
    const seen = {
      activity: { Analyst: { state: "thinking" } },
      activeRun: { Analyst: "other" },
      writing: new Set(),
    } as unknown as Seen;
    expect(statusWord(peerState(peer("Analyst"), RUN, seen), 15_000)).toBe("asked, 14s");
  });

  it("says a colleague waits on the person when its turn is parked on a request", () => {
    const seen = {
      activity: { Analyst: { state: "awaitingApproval" } },
      activeRun: { Analyst: RUN },
      writing: new Set(),
    } as unknown as Seen;
    expect(statusWord(peerState(peer("Analyst"), RUN, seen), 15_000)).toBe("waiting for you, 14s");
  });
});

describe("the folded line", () => {
  it("names who was asked and that every one answered", () => {
    expect(foldLine([peer("Researcher", { received: 1 })])).toBe("Asked Researcher, answered");
    expect(foldLine([peer("Researcher", { received: 1 }), peer("Analyst", { received: 1 })])).toBe(
      "Asked Researcher and Analyst, both answered",
    );
    expect(
      foldLine([
        peer("Researcher", { received: 1 }),
        peer("Analyst", { received: 1 }),
        peer("Chef", { received: 2 }),
      ]),
    ).toBe("Asked Researcher, Analyst and Chef, all answered");
  });

  it("says who did not answer when a conversation ended without them", () => {
    expect(foldLine([peer("Researcher", { received: 1 }), peer("Analyst")])).toBe(
      "Asked Researcher and Analyst, Researcher answered",
    );
    expect(foldLine([peer("Analyst")])).toBe("Asked Analyst, no answer");
    expect(foldLine([peer("Analyst"), peer("Chef")])).toBe("Asked Analyst and Chef, no answers");
  });

  it("says heard from when a colleague wrote without being asked", () => {
    expect(foldLine([peer("Analyst", { sent: 0, received: 1 })])).toBe("Heard from Analyst");
  });

  it("is settled only once every colleague asked has answered", () => {
    expect(settled([peer("Researcher", { received: 1 }), peer("Analyst")])).toBe(false);
    expect(settled([peer("Researcher", { received: 1 }), peer("Analyst", { received: 1 })])).toBe(
      true,
    );
  });

  it("measures from the first question to the last answer", () => {
    expect(
      spanOf([
        peer("Researcher", { askedAt: 1_000, received: 1, answeredAt: 10_000 }),
        peer("Analyst", { askedAt: 2_000, received: 1, answeredAt: 25_000 }),
      ]),
    ).toBe(24_000);
    expect(spanOf([peer("Analyst")])).toBeNull();
  });
});

describe("waitingLine", () => {
  it("says who was messaged and who is still owed an answer", () => {
    expect(waitingLine(["Researcher", "Analyst"], ["Analyst"])).toBe(
      "Messaged Researcher and Analyst, waiting for Analyst",
    );
    expect(waitingLine([], ["Analyst"])).toBe("Waiting for Analyst");
    expect(waitingLine(["Analyst"], [])).toBe("Messaged Analyst");
    expect(waitingLine([], [])).toBeNull();
  });
});
