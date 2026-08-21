import { act, render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { AgentCard, Envelope, MessageId, UiEvent } from "../lib/types";
import { ChannelView } from "./ChannelView";

/**
 * What streaming costs the operator's window.
 *
 * The report this exists for: five agents working at once and the app stops
 * responding, with their answers landing in one lump at the end instead of
 * arriving as they are written. Both are the same fault. The main thread never
 * gets far enough ahead to paint, so the text is there and nobody can see it.
 *
 * Counted rather than timed. A wall clock in a test on this machine says
 * nothing about that machine, but "one token re-rendered ninety messages" is
 * the same number everywhere.
 */

let rendersOfMessages = 0;
let rendersOfBubbles = 0;
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
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    createdAt: 0,
    updatedAt: 0,
    sandboxId: null,
    browserId: null,
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

/** One agent's stream, opened and then fed `tokens` deltas. */
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
    latestBubble = "";
    useStore.setState({
      agents: [agent(AGENT, "Manager"), agent(OTHER, "Chef")],
      messages: { [AGENT]: Array.from({ length: 30 }, (_, i) => message(i)) },
      streams: {},
      reasoning: {},
      activity: { [AGENT]: { state: "thinking" } },
    });
  });

  function draw() {
    render(<ChannelView channel={AGENT} onOpenMenu={() => {}} />);
    rendersOfMessages = 0;
  }

  /**
   * One event per turn of the event loop, which is how they actually arrive:
   * each is its own IPC callback from the runtime. Firing them in one block
   * lets React batch the lot into a single render and measures nothing.
   */
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

    // The transcript above a streaming bubble does not change while text
    // arrives. Before this it re-rendered every message on every token: six
    // thousand renders for one reply, and that is before a second agent starts.
    expect(rendersOfMessages).toBe(0);

    // And the bubble itself still filled in, which is the point of the whole
    // arrangement: cheap is no good if the operator stops seeing the text.
    expect(latestBubble).toBe("tok ".repeat(200));
  });

  it("does not re-render this channel for another channel's tokens", async () => {
    // The report was about several agents at once. A token written to Chef's
    // channel has nothing to do with the window showing Manager.
    draw();
    await feed(stream("00000000-0000-4000-8000-0000000000d2", OTHER, OTHER, 200));

    expect(rendersOfMessages).toBe(0);
  });

  it("does not re-render the transcript or a bubble for a thought", async () => {
    // Reasoning arrives as fast as text and is drawn in one line above the
    // composer. Held in the stream buffer it would re-render, and re-parse the
    // markdown of, every bubble on screen for text that is in none of them.
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

    // And the line itself kept up, which is the point of drawing it at all.
    expect(useStore.getState().reasoning[AGENT]?.endsWith("thinking ")).toBe(true);
  });
});
