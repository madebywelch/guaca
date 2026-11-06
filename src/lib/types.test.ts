import { describe, expect, it } from "vitest";

import {
  asSentence,
  type Envelope,
  errorMessage,
  isCommandError,
  isInterAgent,
  plainText,
} from "./types";

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
    // here, the flow board could not say what opened the run.
    const message = envelope({
      from: { kind: "system" },
      parts: [
        {
          type: "routine",
          routineId: "rt1",
          name: "Sweep",
          what: "check the listings",
          payload: null,
        },
      ],
    });
    expect(plainText(message)).toBe("check the listings");
  });

  it("counts a decision's answer, not the brief the model was sent around it", () => {
    // The flow board names what opened a run with the first 140 characters.
    // Of the brief, that is Guaca introducing the answer and nothing of it.
    const message = envelope({
      from: { kind: "system" },
      parts: [
        {
          type: "decision",
          id: "d1",
          request: {
            question: "10 or 11?",
            context: "",
            recommendation: "",
            options: [],
            source: "",
          },
          answer: "11, and move the call",
          resumed: false,
        },
      ],
    });
    expect(plainText(message)).toBe("11, and move the call");
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

describe("asSentence", () => {
  it("finishes a line that was written for a log rather than for a person", () => {
    // The transport's own words, and the shape the polish audit found twice: an
    // opening that is not a capital and an ending that is not a full stop.
    expect(
      asSentence(
        "lost the connection to this host at http://127.0.0.1:8787. It may be restarting, and nothing on this page is live until it answers again",
      ),
    ).toBe(
      "Lost the connection to this host at http://127.0.0.1:8787. It may be restarting, and nothing on this page is live until it answers again.",
    );
    expect(asSentence("the disk is full")).toBe("The disk is full.");
  });

  it("leaves a sentence somebody already finished exactly as it is", () => {
    for (const said of [
      "Could not authenticate with http://box:8787. Try again.",
      "Nothing was sent: it addressed itself.",
      "Is the box up?",
      "No agents yet!",
      "The host did not answer…",
    ]) {
      expect(asSentence(said)).toBe(said);
    }
  });

  it("does not invent a capital on something that is not prose, or words on nothing", () => {
    expect(asSentence("`name` is a desktop command, and this page is not the desktop app")).toBe(
      "`name` is a desktop command, and this page is not the desktop app.",
    );
    expect(asSentence("  two spaces  ")).toBe("Two spaces.");
    expect(asSentence("")).toBe("");
    expect(asSentence("   ")).toBe("");
  });
});
