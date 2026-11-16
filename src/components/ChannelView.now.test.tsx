/**
 * The composer under Now: what an agent is doing is what a person most often
 * wants to say something about, so the box to say it in stays on that tab.
 */
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { DEFAULT_PREFS } from "../lib/prefs";
import { useStore } from "../lib/store";
import { anAgent } from "../test-fixtures";
import { ChannelView } from "./ChannelView";

vi.mock("../lib/ipc", () => ({
  api: {
    channelMessages: async () => [],
    conversationFlow: async () => [],
    agentComputer: async () => null,
    agentBrowser: async () => null,
    agentRoutines: async () => [],
    agentNotebook: async () => [],
    claudeProgram: async () => ({ installed: false }),
  },
  onFileDrop: async () => () => {},
  openExternal: async () => {},
}));

describe("an agent's page on Now", () => {
  it("keeps the composer under what the agent is doing", async () => {
    const agent = anAgent("manager", { name: "Manager" });
    useStore.setState({
      prefs: DEFAULT_PREFS,
      agents: [agent],
      messages: { [agent.id]: [] },
      streams: {},
      reasoning: {},
      trail: {},
      activity: { [agent.id]: { state: "thinking" } },
      activeRun: {},
      lastActive: {},
      banner: null,
    });
    render(<ChannelView channel={agent.id} onOpenMenu={() => {}} view="now" onView={() => {}} />);
    expect(await screen.findByRole("heading", { name: "Working" })).toBeTruthy();
    const box = screen.getByPlaceholderText("Write to Manager…");
    expect(box.closest("[hidden]")).toBeNull();
  });
});
