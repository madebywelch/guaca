import { describe, expect, it } from "vitest";
import type { LiveCall } from "../lib/trail";
import { MOODS, type Mood, moodFor, PLEASED_MS, type Signals, STRUCK_MS } from "./moods";

const NOW = 1_700_000_000_000;

function call(status?: "ok" | "refused" | "failed"): LiveCall {
  const done =
    status === undefined
      ? null
      : ({
          type: "toolCall",
          name: "send",
          arguments: {},
          outcome:
            status === "ok"
              ? { status: "ok", summary: "" }
              : status === "refused"
                ? { status: "refused", reason: "" }
                : { status: "failed", error: "" },
        } as LiveCall["done"]);
  return { callId: "c", name: "send", arguments: {}, done, startedAt: NOW };
}

const mood = (signals: Signals) => moodFor(signals, NOW);

describe("what a signal makes of a face", () => {
  it("says nothing at all when there is nothing to say", () => {
    expect(mood({})).toBe("idle");
    expect(mood({ activity: { state: "idle" }, lifecycle: "active" })).toBe("idle");
  });

  it("reads the activity the runtime publishes", () => {
    expect(mood({ activity: { state: "thinking" } })).toBe("thinking");
    expect(mood({ activity: { state: "queued", depth: 2 } })).toBe("listening");
    expect(mood({ activity: { state: "awaitingApproval" } })).toBe("blocked");
    expect(mood({ activity: { state: "paused" } })).toBe("paused");
  });

  it("tells a turn that is running from one that is working", () => {
    expect(mood({ activity: { state: "thinking" }, work: [] })).toBe("thinking");
    expect(mood({ activity: { state: "thinking" }, work: [call()] })).toBe("working");
    expect(mood({ activity: { state: "thinking" }, work: [call("ok")] })).toBe("thinking");
  });

  // The runtime does not publish its retries, so a refusal or a failure coming
  // back is the only honest "this is not going well" the app has.
  it("minds a call that came back refused or failed", () => {
    expect(mood({ activity: { state: "thinking" }, work: [call("refused")] })).toBe("frustrated");
    expect(mood({ activity: { state: "thinking" }, work: [call("failed")] })).toBe("frustrated");
    expect(mood({ activity: { state: "thinking" }, work: [call("failed"), call("ok")] })).toBe(
      "thinking",
    );
  });

  it("lets being switched off outrank everything", () => {
    for (const lifecycle of ["paused", "terminated"] as const) {
      expect(mood({ lifecycle, activity: { state: "awaitingApproval" } })).toBe("paused");
      expect(mood({ lifecycle, escalated: true })).toBe("paused");
    }
  });

  it("keeps waiting on a person above reacting to a peer", () => {
    expect(mood({ activity: { state: "awaitingApproval" }, struckAt: NOW })).toBe("blocked");
  });

  it("lets the two transients expire on their own", () => {
    expect(mood({ struckAt: NOW - 10 })).toBe("surprised");
    expect(mood({ struckAt: NOW - STRUCK_MS - 1 })).toBe("idle");
    expect(mood({ finishedAt: NOW - 10 })).toBe("pleased");
    expect(mood({ finishedAt: NOW - PLEASED_MS - 1 })).toBe("idle");
  });

  it("goes back to being stuck once it has stopped being pleased", () => {
    expect(mood({ escalated: true, finishedAt: NOW - 10 })).toBe("pleased");
    expect(mood({ escalated: true })).toBe("stuck");
    // and an escalation says nothing about a turn that is actually running
    expect(mood({ escalated: true, activity: { state: "thinking" } })).toBe("thinking");
  });

  // Ten expressions is ten drawings to keep working. One that no signal can
  // reach is one nobody would notice going wrong.
  it("can reach every mood in the table", () => {
    const reachable = new Set<Mood>([
      mood({}),
      mood({ activity: { state: "queued", depth: 1 } }),
      mood({ activity: { state: "thinking" } }),
      mood({ activity: { state: "thinking" }, work: [call()] }),
      mood({ activity: { state: "thinking" }, work: [call("failed")] }),
      mood({ activity: { state: "awaitingApproval" } }),
      mood({ finishedAt: NOW }),
      mood({ lifecycle: "paused" }),
      mood({ escalated: true }),
      mood({ struckAt: NOW }),
    ]);
    expect([...reachable].sort()).toEqual((Object.keys(MOODS) as Mood[]).sort());
  });
});

describe("the table", () => {
  it("keeps every body still enough to read a face on", () => {
    for (const [key, expression] of Object.entries(MOODS)) {
      const [ax, ay] = expression.shape.aspect ?? [1, 1];
      expect(Math.abs(ax - 1), key).toBeLessThanOrEqual(0.1);
      expect(Math.abs(ay - 1), key).toBeLessThanOrEqual(0.1);
      expect(expression.shape.knead?.amp ?? 0, key).toBeLessThanOrEqual(0.09);
    }
  });

  // Amber is the app's one signal that a person is being waited on. A second
  // mood wearing it is a rail where the color stops meaning anything.
  it("spends the amber on exactly one mood", () => {
    const amber = Object.entries(MOODS).filter(([, e]) => e.mark === "bang");
    expect(amber.map(([key]) => key)).toEqual(["blocked"]);
  });
});
