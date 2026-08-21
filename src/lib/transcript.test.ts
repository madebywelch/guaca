import { describe, expect, it } from "vitest";

import { type Lookups, type Row, rowStandsFor, summaryLabel, transcriptRows } from "./transcript";
import type { AgentCard, Envelope, Part } from "./types";

function card(id: string, name: string): AgentCard {
  return {
    id,
    groupId: "g1",
    sandboxId: null,
    browserId: null,
    name,
    avatar: "plain",
    color: "#c7d96b",
    model: "m",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
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

/** Manager's channel: a peer wrote to it. */
function inbound(from: string, text = "peer says something"): Envelope {
  return envelope({
    from: { kind: "agent", id: from },
    to: { kind: "agent", id: "manager" },
    parts: [{ type: "text", text }],
  });
}

/** Manager's channel: its own record of a turn. */
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
    // The whole point. A fan-out and its answers is a dozen lines of machine
    // chatter through the middle of the operator's own conversation.
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
    // "1 message with Chef" is a worse sentence than either of these, and it
    // loses the one thing a lone message has to say.
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
    // An agent that wrote to a name nobody can find is worth seeing, and there
    // is no conversation behind it to open.
    const burst = bursts(transcriptRows([record(send(["Ghost"], ok))], lookups))[0];
    expect(burst?.peers[0]?.peer.name).toBe("Ghost");
    expect(burst?.peers[0]?.agentId).toBeNull();
  });

  it("ends a burst at anything that is not peer traffic", () => {
    // Two exchanges either side of a memory update are two things that
    // happened, and one row saying "4 messages" would say otherwise.
    const rows = transcriptRows(
      [
        inbound("chef"),
        record({
          type: "toolCall",
          name: "update_notes",
          arguments: { content: "x" },
          outcome: { status: "ok", summary: "saved" },
        }),
        inbound("chef"),
      ],
      lookups,
    );

    expect(rows.map((row) => row.kind)).toEqual(["peers", "message", "peers"]);
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
    // The bug this inherits a test from: one verdict for the whole call, so a
    // send that reached two of three drew all three as delivered.
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
    // There is no peer to file it under, so extracting it would delete it.
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
    // The stop, and only the stop: the send beside it is in the burst.
    expect(trail?.kind === "message" && trail.message.parts).toEqual([
      { type: "notice", kind: "guardStop", text: "Reply to Scribe was not delivered" },
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
    // Something happened in between. Drawing the second message as a
    // continuation of the first says nothing did.
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
    // The clock a transcript used to print over every message. Four replies in
    // the same minute is four headers carrying one fact between them.
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
    // The time it started again, not the time it stopped: the line belongs to
    // what is under it.
    expect(rows[1]).toMatchObject({ kind: "when", at: later.createdAt });
  });

  it("ends the burst it interrupts", () => {
    // Two exchanges three hours apart are two things that happened. Counted as
    // one burst they read as a single conversation the operator missed.
    const first = inbound("chef");
    const later = inbound("chef");
    later.createdAt = first.createdAt + 3 * hour;

    const rows = transcriptRows([first, later], lookups);
    expect(rows.map((row) => row.kind)).toEqual(["peers", "when", "peers"]);
  });

  it("never trails a transcript with a line nothing follows", () => {
    // A turn whose every part folded into a burst leaves no row of its own, so
    // a line pushed the moment the gap is spotted can end up hanging off the
    // bottom pointing at nothing.
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

describe("a request for permission", () => {
  it("is never folded into a burst, whoever the envelope says it is from", () => {
    // The one thing in a transcript the operator is expected to act on.
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
