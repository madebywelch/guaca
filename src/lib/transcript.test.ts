import { describe, expect, it } from "vitest";

import { type Lookups, type Row, rowStandsFor, summaryLabel, transcriptRows } from "./transcript";
import type { AgentCard, Envelope, Part } from "./types";

function card(id: string, name: string): AgentCard {
  return {
    id,
    groupId: "g1",
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    browserConsent: "open",
    name,
    avatar: "plain",
    color: "#c7d96b",
    model: "m",
    subscriptionModel: "",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    discardedAt: null,
  };
}

const AGENTS = [card("manager", "Manager"), card("chef", "Chef"), card("scribe", "Scribe")];

const lookups: Lookups = {
  byId: (id) => AGENTS.find((a) => a.id === id),
  byName: (name) => AGENTS.find((a) => a.name.toLowerCase() === name.trim().toLowerCase()),
};

let clock = 1_700_000_000_000;

function envelope(overrides: Partial<Envelope>): Envelope {
  clock += 1_000;
  return {
    id: `m${clock}`,
    runId: "r1",
    channelId: "manager",
    from: { kind: "human" },
    to: { kind: "agent", id: "manager" },
    parts: [{ type: "text", text: "hello" }],
    trust: "operator",
    hop: 0,
    expectsReply: true,
    intent: "work",
    cause: null,
    createdAt: clock,
    ...overrides,
  };
}

function inbound(from: string, text = "peer says something"): Envelope {
  return envelope({
    from: { kind: "agent", id: from },
    to: { kind: "agent", id: "manager" },
    parts: [{ type: "text", text }],
  });
}

function record(...parts: Part[]): Envelope {
  return envelope({
    from: { kind: "agent", id: "manager" },
    to: { kind: "system" },
    trust: "system",
    parts,
  });
}

function send(to: string[], outcome: Extract<Part, { type: "toolCall" }>["outcome"]): Part {
  return {
    type: "toolCall",
    name: "send_message",
    arguments: { to, text: "the briefing" },
    outcome,
  };
}

const ok = { status: "ok", summary: "queued" } as const;

function bursts(rows: Row[]) {
  return rows.filter((row): row is Extract<Row, { kind: "peers" }> => row.kind === "peers");
}

describe("peer traffic in a channel", () => {
  it("collapses a burst into one row per peer, counting both directions", () => {
    const rows = transcriptRows(
      [record(send(["Chef", "Scribe"], ok)), inbound("chef"), inbound("scribe"), inbound("chef")],
      lookups,
    );

    expect(rows).toHaveLength(1);
    const [burst] = bursts(rows);
    expect(burst?.peers.map((p) => [p.peer.name, p.sent, p.received])).toEqual([
      ["Chef", 1, 2],
      ["Scribe", 1, 1],
    ]);
    expect(burst?.peers.map(summaryLabel)).toEqual([
      "3 messages with Chef",
      "2 messages with Scribe",
    ]);
  });

  it("says which way a single message went", () => {
    const sent = bursts(transcriptRows([record(send(["Chef"], ok))], lookups))[0];
    expect(sent?.peers.map(summaryLabel)).toEqual(["Messaged Chef"]);

    const got = bursts(transcriptRows([inbound("chef")], lookups))[0];
    expect(got?.peers.map(summaryLabel)).toEqual(["Message from Chef"]);
  });

  it("carries the peer's id so the row has a thread to open", () => {
    const burst = bursts(transcriptRows([inbound("chef"), record(send(["Chef"], ok))], lookups))[0];
    expect(burst?.peers).toHaveLength(1);
    expect(burst?.peers[0]?.agentId).toBe("chef");
  });

  it("offers no thread for a name that resolves to nobody", () => {
    const burst = bursts(transcriptRows([record(send(["Ghost"], ok))], lookups))[0];
    expect(burst?.peers[0]?.peer.name).toBe("Ghost");
    expect(burst?.peers[0]?.agentId).toBeNull();
  });

  it("ends a burst at anything that is not peer traffic", () => {
    const rows = transcriptRows(
      [
        inbound("chef"),
        record({
          type: "toolCall",
          name: "update_memory",
          arguments: { content: "x" },
          outcome: { status: "ok", summary: "saved" },
        }),
        inbound("chef"),
      ],
      lookups,
    );

    expect(rows.map((row) => row.kind)).toEqual(["peers", "message", "peers"]);
  });

  it("files an answer under its question when the asker spoke in between", () => {
    const rows = transcriptRows(
      [
        record(send(["Chef", "Scribe"], ok)),
        envelope({ from: { kind: "agent", id: "manager" }, to: { kind: "human" } }),
        inbound("chef", "Dinner is at eight."),
      ],
      lookups,
    );

    expect(rows.map((row) => row.kind)).toEqual(["peers", "message"]);
    expect(bursts(rows)[0]?.peers.map((p) => [p.peer.name, p.answer])).toEqual([
      ["Chef", "Dinner is at eight."],
      ["Scribe", null],
    ]);
  });

  it("does not swallow the operator's own conversation", () => {
    const rows = transcriptRows(
      [
        envelope({}),
        inbound("chef"),
        envelope({ from: { kind: "agent", id: "manager" }, to: { kind: "human" } }),
      ],
      lookups,
    );
    expect(rows.map((row) => row.kind)).toEqual(["message", "peers", "message"]);
  });
});

describe("a send the runtime stopped", () => {
  it("stays its own row, because it is not part of the conversation", () => {
    const rows = transcriptRows(
      [record(send(["Chef"], { status: "refused", reason: "Refused: hop limit reached." }))],
      lookups,
    );

    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      kind: "refused",
      reason: "Refused: hop limit reached.",
      body: "the briefing",
    });
  });

  it("counts only the recipients a half-delivered fan-out actually reached", () => {
    const rows = transcriptRows(
      [
        record(
          send(["Chef", "Ghost", "Scribe"], {
            status: "partial",
            summary: "queued for 2 of 3 agent(s)",
            refused: [{ to: "Ghost", reason: "Refused: Ghost has been deleted." }],
          }),
        ),
      ],
      lookups,
    );

    expect(bursts(rows)[0]?.peers.map((p) => p.peer.name)).toEqual(["Chef", "Scribe"]);
    expect(rows.filter((row) => row.kind === "refused")).toHaveLength(1);
  });

  it("interrupts the burst it lands in rather than being absorbed by it", () => {
    const rows = transcriptRows(
      [
        inbound("chef"),
        record(send(["Chef"], { status: "refused", reason: "Refused: said that already." })),
        inbound("chef"),
      ],
      lookups,
    );
    expect(rows.map((row) => row.kind)).toEqual(["peers", "refused", "peers"]);
  });
});

describe("an agent's own trail", () => {
  it("keeps a send that named nobody, where the reason it failed is legible", () => {
    const rows = transcriptRows(
      [
        record({
          type: "toolCall",
          name: "send_message",
          arguments: { text: "hello?" },
          outcome: { status: "refused", reason: "Refused: name a recipient." },
        }),
      ],
      lookups,
    );

    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({ kind: "message" });
  });

  it("keeps a guard stop that shared a record with a send", () => {
    const rows = transcriptRows(
      [
        record(send(["Chef"], ok), {
          type: "notice",
          kind: "guardStop",
          text: "Reply to Scribe was not delivered",
        }),
      ],
      lookups,
    );

    expect(rows.map((row) => row.kind)).toEqual(["peers", "message"]);
    const trail = rows.find((row) => row.kind === "message");
    expect(trail?.kind === "message" && trail.message.parts).toEqual([
      { type: "notice", kind: "guardStop", text: "Reply to Scribe was not delivered" },
    ]);
  });

  it("keeps the text a turn trailed after answering through the tool", () => {
    const rows = transcriptRows(
      [record(send(["Chef"], ok), { type: "text", text: "Replied to Chef." })],
      lookups,
    );

    expect(rows.map((row) => row.kind)).toEqual(["peers", "message"]);
    const trail = rows.find((row) => row.kind === "message");
    expect(trail?.kind === "message" && trail.message.parts).toEqual([
      { type: "text", text: "Replied to Chef." },
    ]);
  });
});

describe("merging consecutive messages under one header", () => {
  it("merges a follow-up from the same author", () => {
    const first = envelope({});
    const second = envelope({ createdAt: first.createdAt + 1_000 });
    const rows = transcriptRows([first, second], lookups);
    expect(rows[1]).toMatchObject({ continued: true });
  });

  it("does not merge across a burst of peer traffic", () => {
    const first = envelope({});
    const second = envelope({ createdAt: first.createdAt + 1_000 });
    const rows = transcriptRows([first, inbound("chef"), second], lookups);
    expect(rows[2]).toMatchObject({ continued: false });
  });

  it("does not merge a message four minutes later", () => {
    const first = envelope({});
    const second = envelope({ createdAt: first.createdAt + 5 * 60 * 1000 });
    expect(transcriptRows([first, second], lookups)[1]).toMatchObject({ continued: false });
  });
});

describe("saying when a conversation picked up again", () => {
  const hour = 60 * 60 * 1000;

  it("draws nothing between messages of the same sitting", () => {
    const first = envelope({});
    const rows = transcriptRows(
      [first, envelope({ createdAt: first.createdAt + 5 * 60 * 1000 })],
      lookups,
    );
    expect(rows.map((row) => row.kind)).toEqual(["message", "message"]);
  });

  it("draws a line where the silence was long enough to notice", () => {
    const first = envelope({});
    const later = envelope({ createdAt: first.createdAt + 3 * hour });
    const rows = transcriptRows([first, later], lookups);

    expect(rows.map((row) => row.kind)).toEqual(["message", "when", "message"]);
    expect(rows[1]).toMatchObject({ kind: "when", at: later.createdAt });
  });

  it("ends the burst it interrupts", () => {
    const first = inbound("chef");
    const later = inbound("chef");
    later.createdAt = first.createdAt + 3 * hour;

    const rows = transcriptRows([first, later], lookups);
    expect(rows.map((row) => row.kind)).toEqual(["peers", "when", "peers"]);
  });

  it("never trails a transcript with a line nothing follows", () => {
    const first = envelope({});
    const quiet = record(send(["Chef"], { status: "ok", summary: "sent" }));
    quiet.createdAt = first.createdAt + 3 * hour;
    const rows = transcriptRows([first, quiet], lookups);

    expect(rows[rows.length - 1]?.kind).not.toBe("when");
    expect(rows.map((row) => row.kind)).toEqual(["message", "when", "peers"]);
  });

  it("stands for no message, so search never lands on one", () => {
    const first = envelope({});
    const rows = transcriptRows(
      [first, envelope({ createdAt: first.createdAt + 3 * hour })],
      lookups,
    );
    expect(rowStandsFor(rows[1]!)).toEqual([]);
  });
});

describe("a decision the operator answered", () => {
  it("is never continued by what Guaca says after it", () => {
    const answered = envelope({
      from: { kind: "system" },
      trust: "system",
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
          answer: "11",
          resumed: false,
        },
      ],
    });
    const after = envelope({ from: { kind: "system" }, trust: "system" });

    const rows = transcriptRows([answered, after], lookups);
    expect(rows.map((row) => row.kind)).toEqual(["message", "message"]);
    expect(rows[1]).toMatchObject({ continued: false });
  });
});

describe("a request for permission", () => {
  it("is never folded into a burst, whoever the envelope says it is from", () => {
    const asking = envelope({
      from: { kind: "agent", id: "chef" },
      to: { kind: "agent", id: "manager" },
      parts: [
        {
          type: "approval",
          id: "a1",
          action: "createAgent",
          summary: "Add an agent called Sous",
          detail: [],
        },
      ],
    });

    const rows = transcriptRows([inbound("chef"), asking, inbound("chef")], lookups);
    expect(rows.map((row) => row.kind)).toEqual(["peers", "message", "peers"]);
  });
});

describe("an overnight tidy", () => {
  it("is a row of its own, with the rewrite it made as the trail under it", () => {
    const tidy = envelope({
      from: { kind: "system" },
      to: { kind: "agent", id: "manager" },
      trust: "system",
      expectsReply: false,
      parts: [{ type: "tidy", reported: ["- 2h ago: Booked the venue."], notes: [] }],
    });
    const rewrite = envelope({
      from: { kind: "agent", id: "manager" },
      to: { kind: "system" },
      parts: [
        {
          type: "toolCall",
          name: "update_memory",
          arguments: { content: "The venue is the Hall." },
          outcome: { status: "ok", summary: "Saved." },
        } as Part,
      ],
    });

    const rows = transcriptRows([tidy, rewrite], lookups);
    expect(rows.map((row) => row.kind)).toEqual(["message", "message"]);
    const [line, trail] = rows as Extract<Row, { kind: "message" }>[];
    expect(line!.message.id).toBe(tidy.id);
    expect(line!.continued).toBe(false);
    expect(trail!.message.parts.map((part) => part.type)).toEqual(["toolCall"]);
  });
});
