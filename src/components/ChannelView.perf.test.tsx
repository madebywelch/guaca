import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { DEFAULT_PREFS } from "../lib/prefs";
import { useStore } from "../lib/store";
import type { AgentCard, Envelope, MessageId, UiEvent } from "../lib/types";
import { ChannelView } from "./ChannelView";

let rendersOfMessages = 0;
let rendersOfBubbles = 0;
let rendersOfTrail = 0;
let latestBubble = "";

vi.mock("./MessageItem", () => ({
  MessageItem: () => {
    rendersOfMessages += 1;
    return <div />;
  },
  StreamingMessage: ({ text }: { text: string }) => {
    rendersOfBubbles += 1;
    latestBubble = text;
    return <div>{text}</div>;
  },
}));

vi.mock("./Trail", () => ({
  DiffBlock: () => {
    rendersOfTrail += 1;
    return <div />;
  },
  TrailRow: () => {
    rendersOfTrail += 1;
    return <div />;
  },
}));

vi.mock("../lib/ipc", () => ({
  api: {
    channel: async () => [],
    clearChannel: async () => {},
    sendMessage: async () => "run",
  },
  onFileDrop: async () => () => {},
}));

const AGENT = "00000000-0000-4000-8000-0000000000a1";
const OTHER = "00000000-0000-4000-8000-0000000000a2";

function agent(id: string, name: string): AgentCard {
  return {
    id,
    groupId: "00000000-0000-4000-8000-0000000000b1",
    name,
    avatar: "plain",
    color: "#c7d96b",
    model: "",
    subscriptionModel: "",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    createdAt: 0,
    updatedAt: 0,
    discardedAt: null,
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    browserConsent: "open",
    version: 1,
  };
}

function message(index: number): Envelope {
  return {
    id: `00000000-0000-4000-8000-${String(index).padStart(12, "0")}` as MessageId,
    runId: "00000000-0000-4000-8000-0000000000c1",
    channelId: AGENT,
    from: { kind: "agent", id: AGENT },
    to: { kind: "human" },
    parts: [{ type: "text", text: `message ${index}` }],
    trust: "peer",
    hop: 0,
    expectsReply: false,
    intent: "courtesy",
    cause: null,
    createdAt: index,
  };
}

function stream(messageId: string, channelId: string, agentId: string, tokens: number): UiEvent[] {
  const events: UiEvent[] = [
    {
      type: "streamStarted",
      messageId: messageId as MessageId,
      channelId,
      agentId,
      runId: "00000000-0000-4000-8000-0000000000c1",
      to: { kind: "human" },
    },
  ];
  for (let i = 0; i < tokens; i += 1) {
    events.push({
      type: "streamDelta",
      messageId: messageId as MessageId,
      channelId,
      text: "tok ",
    });
  }
  return events;
}

describe("ChannelView under streaming load", () => {
  beforeEach(() => {
    rendersOfMessages = 0;
    rendersOfBubbles = 0;
    rendersOfTrail = 0;
    latestBubble = "";
    useStore.setState({
      prefs: DEFAULT_PREFS,
      agents: [agent(AGENT, "Manager"), agent(OTHER, "Chef")],
      messages: { [AGENT]: Array.from({ length: 30 }, (_, i) => message(i)) },
      streams: {},
      reasoning: {},
      trail: {},
      activity: { [AGENT]: { state: "thinking" } },
    });
  });

  function draw() {
    render(<ChannelView channel={AGENT} onOpenMenu={() => {}} />);
    rendersOfMessages = 0;
  }

  async function feed(events: UiEvent[]) {
    const apply = useStore.getState().applyEvent;
    for (const event of events) {
      await act(async () => {
        apply(event);
      });
    }
  }

  it("does not re-render the transcript for every token", async () => {
    draw();
    await feed(stream("00000000-0000-4000-8000-0000000000d1", AGENT, AGENT, 200));

    expect(rendersOfMessages).toBe(0);

    expect(latestBubble).toBe("tok ".repeat(200));
  });

  it("does not re-render this channel for another channel's tokens", async () => {
    draw();
    await feed(stream("00000000-0000-4000-8000-0000000000d2", OTHER, OTHER, 200));

    expect(rendersOfMessages).toBe(0);
  });

  it("does not re-render the transcript or a bubble for a thought", async () => {
    const id = "00000000-0000-4000-8000-0000000000d3";
    draw();
    await feed(stream(id, AGENT, AGENT, 1));
    const bubbles = rendersOfBubbles;

    await feed(
      Array.from({ length: 200 }, () => ({
        type: "reasoningDelta" as const,
        messageId: id as MessageId,
        text: "thinking ",
      })),
    );

    expect(rendersOfMessages).toBe(0);
    expect(rendersOfBubbles).toBe(bubbles);

    expect(useStore.getState().reasoning[AGENT]?.endsWith("thinking ")).toBe(true);
  });

  it("does not re-render the turn's chips for a thought", async () => {
    const id = "00000000-0000-4000-8000-0000000000d4";
    draw();
    await feed([
      ...stream(id, AGENT, AGENT, 1),
      {
        type: "toolStarted",
        messageId: id as MessageId,
        callId: "call_1",
        name: "run_command",
        arguments: { command: "ls" },
      },
      {
        type: "toolFinished",
        messageId: id as MessageId,
        callId: "call_1",
        part: {
          type: "toolCall",
          name: "run_command",
          arguments: { command: "ls" },
          outcome: { status: "ok", summary: "exit 0" },
        },
      },
    ]);
    expect(rendersOfTrail).toBe(0);
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: /1 step/ }));
    });
    const chips = rendersOfTrail;
    expect(chips).toBeGreaterThan(0);

    await feed(
      Array.from({ length: 200 }, () => ({
        type: "reasoningDelta" as const,
        messageId: id as MessageId,
        text: "thinking ",
      })),
    );

    expect(rendersOfTrail).toBe(chips);
  });

  it("keeps inline reasoning updates out of settled messages, bubbles and tool cards", async () => {
    useStore.setState({ prefs: { ...DEFAULT_PREFS, showReasoning: true } });
    draw();
    const id = "inline-load";
    await feed([
      ...stream(id, AGENT, AGENT, 1),
      {
        type: "toolStarted",
        messageId: id,
        callId: "call",
        name: "update_memory",
        arguments: { content: "New memory" },
      },
      {
        type: "toolFinished",
        messageId: id,
        callId: "call",
        part: {
          type: "toolCall",
          name: "update_memory",
          arguments: { content: "New memory" },
          outcome: { status: "ok", summary: "Memory updated" },
          replaced: "Old memory",
        },
      },
    ]);
    const chips = rendersOfTrail;
    const bubbles = rendersOfBubbles;
    expect(chips).toBeGreaterThan(0);
    rendersOfMessages = 0;
    await feed(
      Array.from({ length: 200 }, () => ({
        type: "reasoningDelta",
        messageId: id,
        text: "thinking ",
      })),
    );
    expect(rendersOfMessages).toBe(0);
    expect(rendersOfBubbles).toBe(bubbles);
    expect(rendersOfTrail).toBe(chips);
  });

  it("does not re-render the transcript for a tool call", async () => {
    const id = "00000000-0000-4000-8000-0000000000d5";
    draw();
    await feed(stream(id, AGENT, AGENT, 1));
    rendersOfMessages = 0;

    await feed([
      {
        type: "toolStarted",
        messageId: id as MessageId,
        callId: "call_1",
        name: "browse",
        arguments: { action: "open", url: "https://cnn.com" },
      },
      {
        type: "toolFinished",
        messageId: id as MessageId,
        callId: "call_1",
        part: {
          type: "toolCall",
          name: "browse",
          arguments: { action: "open", url: "https://cnn.com" },
          outcome: { status: "ok", summary: "read cnn.com" },
        },
      },
    ]);

    expect(rendersOfMessages).toBe(0);
  });
});
