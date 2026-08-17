import { act, render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { AgentCard, Envelope, MessageId } from "../lib/types";
import { ChannelView } from "./ChannelView";

/**
 * Arriving at a message from search.
 *
 * Finding a message and being put in the right channel next to it are two
 * different things, and only the second one is any use. What is checked here is
 * that the row is brought into view and marked, and that the mark does not
 * outlive the visit.
 */

vi.mock("../lib/ipc", () => ({
  api: {
    channelMessages: async () => [],
    conversationFlow: async () => [],
    agentComputer: async () => null,
    clearChannel: async () => 0,
    sendMessage: async () => "run",
  },
  onFileDrop: async () => () => {},
  openExternal: async () => {},
}));

const AGENT = "00000000-0000-4000-8000-0000000000a1";

function agent(): AgentCard {
  return {
    id: AGENT,
    groupId: "00000000-0000-4000-8000-0000000000b1",
    name: "Manager",
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
    version: 1,
  };
}

function message(index: number): Envelope {
  return {
    id: `00000000-0000-4000-8000-${String(index).padStart(12, "0")}` as MessageId,
    runId: "00000000-0000-4000-8000-0000000000c1",
    channelId: AGENT,
    from: { kind: "human" },
    to: { kind: "agent", id: AGENT },
    parts: [{ type: "text", text: `message ${index}` }],
    trust: "operator",
    hop: 0,
    expectsReply: true,
    intent: "work",
    cause: null,
    createdAt: index,
  };
}

const OLDEST = message(0).id;

beforeEach(() => {
  useStore.setState({
    agents: [agent()],
    messages: { [AGENT]: Array.from({ length: 12 }, (_, i) => message(i)) },
    streams: {},
    activity: {},
    focused: null,
  });
});

describe("a message arrived at from search", () => {
  it("is marked and brought into view", async () => {
    const scrolled = vi.spyOn(Element.prototype, "scrollIntoView");
    useStore.setState({ focused: OLDEST });

    const { container } = render(<ChannelView channel={AGENT} onEditAgent={() => {}} />);

    await waitFor(() => {
      expect(container.querySelector(`[data-message="${OLDEST}"][data-found="true"]`)).toBeTruthy();
    });
    expect(scrolled).toHaveBeenCalled();
    // One row, not the whole transcript lit up.
    expect(container.querySelectorAll("[data-found='true']")).toHaveLength(1);
    scrolled.mockRestore();
  });

  it("stops being marked once it has been shown", async () => {
    // A mark that outlives the visit is still there next time the channel is
    // opened, saying nothing about anything.
    vi.useFakeTimers();
    try {
      useStore.setState({ focused: OLDEST });
      const { container } = render(<ChannelView channel={AGENT} onEditAgent={() => {}} />);

      await act(async () => {
        await vi.advanceTimersByTimeAsync(5_000);
      });
      expect(useStore.getState().focused).toBeNull();
      expect(container.querySelector("[data-found='true']")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("leaves the transcript alone when the message is not in the window", async () => {
    // The window is bounded, so a hit older than it opens the channel at the
    // newest end. That is a partial jump, not a broken transcript.
    useStore.setState({ focused: "00000000-0000-4000-8000-999999999999" as MessageId });
    const { container } = render(<ChannelView channel={AGENT} onEditAgent={() => {}} />);

    await waitFor(() => expect(container.querySelectorAll("[data-message]")).toHaveLength(12));
    expect(container.querySelector("[data-found='true']")).toBeNull();
    // Still held, so nothing has quietly decided the jump succeeded.
    expect(useStore.getState().focused).toBe("00000000-0000-4000-8000-999999999999");
  });
});
