/**
 * The composer under Now: what an agent is doing is what a person most often
 * wants to say something about, so the box to say it in stays on that tab.
 */
import { render, screen, within } from "@testing-library/react";
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
  it("keeps only the step count and Stop on the line above the composer", async () => {
    // Now already says who is working, on what, and each step. The face and
    // the tail of the model's working belong to the conversation's line.
    const agent = anAgent("manager", { name: "Manager" });
    const args = { action: "open", url: "https://plumbers.example" };
    useStore.setState({
      prefs: DEFAULT_PREFS,
      agents: [agent],
      messages: { [agent.id]: [] },
      streams: {},
      reasoning: { [agent.id]: "Reading the listings one by one, and the last one is newer." },
      trail: {
        [agent.id]: [
          {
            callId: "c1",
            name: "browse",
            arguments: args,
            done: {
              type: "toolCall",
              name: "browse",
              arguments: args,
              outcome: { status: "ok", summary: "opened" },
            },
            startedAt: 0,
          },
        ],
      },
      activity: { [agent.id]: { state: "thinking" } },
      activeRun: { [agent.id]: "r1" },
      lastActive: {},
      banner: null,
    });
    const { container } = render(
      <ChannelView channel={agent.id} onOpenMenu={() => {}} view="now" onView={() => {}} />,
    );
    await screen.findByRole("heading", { name: "Working" });
    const line = container.querySelector(".working") as HTMLElement;
    expect(within(line).getByRole("button", { name: /^.?1 step$/ })).toBeTruthy();
    expect(within(line).getByRole("button", { name: "Stop" })).toBeTruthy();
    expect(line.querySelector("[title='Manager is working']")).toBeNull();
    expect(line.textContent).not.toContain("newer");
    expect(line.textContent).not.toContain("Manager is working");
  });
});
