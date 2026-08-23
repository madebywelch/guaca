import { describe, expect, it } from "vitest";

import { type Envelope, errorMessage, isCommandError, isInterAgent, plainText } from "./types";

function envelope(overrides: Partial<Envelope> = {}): Envelope {
  return {
    id: "m1",
    runId: "r1",
    channelId: "a2",
    from: { kind: "human" },
    to: { kind: "agent", id: "a2" },
    parts: [{ type: "text", text: "hello" }],
    trust: "operator",
    hop: 0,
    expectsReply: true,
    intent: "courtesy",
    cause: null,
    createdAt: 1,
    ...overrides,
  };
}

describe("plainText", () => {
  it("joins text parts and ignores the rest", () => {
    const message = envelope({
      parts: [
        { type: "text", text: "one" },
        { type: "notice", kind: "guardStop", text: "hidden" },
        {
          type: "toolCall",
          name: "directory",
          arguments: {},
          outcome: { status: "ok", summary: "x" },
        },
        { type: "text", text: "two" },
      ],
    });
    expect(plainText(message)).toBe("one\ntwo");
  });

  it("returns an empty string for a message with no text", () => {
    const message = envelope({ parts: [{ type: "notice", kind: "lifecycle", text: "x" }] });
    expect(plainText(message)).toBe("");
  });

  it("counts a fired routine's instruction, which is what the model was sent", () => {
    // The transcript draws a firing as one line instead of a bubble, and does
    // it by choosing a row for the part. If that were done by hiding the words
    // here, the activity board could not say what opened the run.
    const message = envelope({
      from: { kind: "system" },
      parts: [{ type: "routine", routineId: "rt1", name: "Sweep", what: "check the listings" }],
    });
    expect(plainText(message)).toBe("check the listings");
  });
});

describe("isInterAgent", () => {
  it("is true only when both ends are agents", () => {
    expect(isInterAgent(envelope())).toBe(false);
    expect(
      isInterAgent(envelope({ from: { kind: "agent", id: "a1" }, to: { kind: "human" } })),
    ).toBe(false);
    expect(
      isInterAgent(
        envelope({ from: { kind: "agent", id: "a1" }, to: { kind: "agent", id: "a2" } }),
      ),
    ).toBe(true);
  });

  it("excludes system messages", () => {
    expect(
      isInterAgent(envelope({ from: { kind: "system" }, to: { kind: "agent", id: "a2" } })),
    ).toBe(false);
  });
});

describe("error handling", () => {
  it("recognizes a structured command error", () => {
    expect(isCommandError({ kind: "validation", message: "bad" })).toBe(true);
    expect(isCommandError(new Error("boom"))).toBe(false);
    expect(isCommandError(null)).toBe(false);
  });

  it("produces a message for anything that can be thrown", () => {
    expect(errorMessage({ kind: "duplicateName", message: "taken" })).toBe("taken");
    expect(errorMessage(new Error("boom"))).toBe("boom");
    expect(errorMessage("plain string")).toBe("plain string");
    expect(errorMessage(undefined)).toBe("Something went wrong.");
  });
});
