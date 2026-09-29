import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { AgentCard, Approval, Envelope, Participant, Reveal } from "../lib/types";
import { aDecision, aGroup, DEFAULT_GROUP } from "../test-fixtures";
import { MenubarPanel } from "./MenubarPanel";

const mocks = vi.hoisted(() => ({
  openWindow: vi.fn<(target: Reveal | null) => Promise<void>>(),
  closeMenubar: vi.fn<() => Promise<void>>(),
  fitMenubar: vi.fn<(height: number) => Promise<void>>(),
  sendMessage: vi.fn<(agent: string, text: string) => Promise<string>>(),
  stopEverything: vi.fn<() => Promise<number>>(),
  stopRun: vi.fn<(run: string) => Promise<boolean>>(),
  channelMessages: vi.fn<(agent: string) => Promise<Envelope[]>>(),
  decideApproval: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  api: {
    ...mocks,
    approvalStates: async () => ({}),
    pendingApprovals: async () => [],
    openEscalations: async () => [],
  },
  openExternal: async () => {},
}));

function agent(name: string, over: Partial<AgentCard> = {}): AgentCard {
  return {
    id: `id-${name}`,
    railOrder: 0,
    groupId: DEFAULT_GROUP,
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    browserConsent: "open",
    hasTerminal: false,
    harness: "pi",
    gate: "open",
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "m",
    subscriptionModel: "",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    version: 1,
    createdAt: 1,
    updatedAt: 1,
    discardedAt: null,
    ...over,
  };
}

const YOU: Participant = { kind: "human" };
const CHEF: Participant = { kind: "agent", id: "id-Chef" };

function said(from: Participant, to: Participant, text: string, at: number): Envelope {
  return {
    id: `m-${at}`,
    runId: "run-1",
    channelId: "id-Chef",
    from,
    to,
    parts: [{ type: "text", text }],
    trust: "operator",
    hop: 0,
    expectsReply: false,
    intent: "work",
    cause: null,
    createdAt: at,
  };
}

function waiting(over: Partial<Approval> = {}): Approval {
  return {
    id: "req-1",
    agentId: "id-Chef",
    groupId: DEFAULT_GROUP,
    runId: "run-1",
    request: { kind: "permission", action: "createAgent" },
    summary: "Chef wants to hire a scribe",
    detail: [],
    state: "pending",
    answer: null,
    createdAt: 0,
    decidedAt: null,
    ...over,
  };
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.openWindow.mockResolvedValue(undefined);
  mocks.closeMenubar.mockResolvedValue(undefined);
  mocks.fitMenubar.mockResolvedValue(undefined);
  mocks.sendMessage.mockResolvedValue("run-2");
  mocks.stopEverything.mockResolvedValue(1);
  mocks.channelMessages.mockResolvedValue([]);
  useStore.setState({
    agents: [agent("Chef"), agent("Scout")],
    groups: [aGroup()],
    activity: {},
    building: {},
    trail: {},
    lastActive: {},
    finishedAt: {},
    pending: [],
    stuck: [],
    decisions: [],
    activeRun: {},
    streams: {},
    messages: {},
    banner: null,
    sessionSpend: { prompt: 0, completion: 0, cost: null, calls: 0 },
  });
});

/** Opens Chef's conversation from the list. */
async function talkToChef() {
  render(<MenubarPanel ready />);
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: /Chef/ }));
  });
  return screen.getByRole<HTMLTextAreaElement>("textbox", { name: "Message Chef" });
}

describe("a line to an agent from the panel", () => {
  it("keeps the words when the message could not be sent, and says why", async () => {
    mocks.sendMessage.mockRejectedValue({ kind: "unreachable", message: "the host is down" });
    const field = await talkToChef();

    fireEvent.change(field, { target: { value: "Can you check the order?" } });
    await act(async () => {
      fireEvent.keyDown(field, { key: "Enter" });
    });

    expect(await screen.findByRole("alert")).toHaveProperty("textContent", "the host is down");
    expect(field.value).toBe("Can you check the order?");
  });

  it("sends on Enter and clears the field once the message is taken", async () => {
    const field = await talkToChef();

    fireEvent.change(field, { target: { value: "  Can you check the order?  " } });
    await act(async () => {
      fireEvent.keyDown(field, { key: "Enter" });
    });

    expect(mocks.sendMessage).toHaveBeenCalledWith("id-Chef", "Can you check the order?");
    await waitFor(() => expect(field.value).toBe(""));
  });

  it("takes Shift+Enter as a new line, not a send", async () => {
    const field = await talkToChef();

    fireEvent.change(field, { target: { value: "one" } });
    fireEvent.keyDown(field, { key: "Enter", shiftKey: true });

    expect(mocks.sendMessage).not.toHaveBeenCalled();
  });

  it("sends nothing that is only whitespace", async () => {
    const field = await talkToChef();

    fireEvent.change(field, { target: { value: "   " } });
    fireEvent.keyDown(field, { key: "Enter" });

    expect(mocks.sendMessage).not.toHaveBeenCalled();
  });

  it("opens on the last things the two of them said, read from the channel", async () => {
    mocks.channelMessages.mockResolvedValue([
      said(YOU, CHEF, "What is on the menu?", 1),
      said(CHEF, YOU, "Soup, then fish.", 2),
    ]);
    await talkToChef();

    expect(mocks.channelMessages).toHaveBeenCalledWith("id-Chef", 300, undefined);
    expect(await screen.findByText("Soup, then fish.")).toBeTruthy();
    expect(screen.getByText("What is on the menu?")).toBeTruthy();
  });

  it("shows a reply while it is being written", async () => {
    useStore.setState({
      streams: {
        "m-live": { channelId: "id-Chef", agentId: "id-Chef", text: "Checking the stock", to: YOU },
      },
    });
    await talkToChef();

    expect(screen.getByText("Checking the stock")).toBeTruthy();
  });

  it("goes back to everyone on Escape, and closes the panel on the next one", async () => {
    await talkToChef();

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("textbox", { name: "Message Chef" })).toBeNull();
    expect(mocks.closeMenubar).not.toHaveBeenCalled();

    fireEvent.keyDown(window, { key: "Escape" });
    expect(mocks.closeMenubar).toHaveBeenCalled();
  });

  it("keeps what was being typed when the operator steps back to the list", async () => {
    const field = await talkToChef();
    fireEvent.change(field, { target: { value: "Half a thought" } });

    fireEvent.keyDown(window, { key: "Escape" });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Chef/ }));
    });

    expect(screen.getByRole<HTMLTextAreaElement>("textbox", { name: "Message Chef" }).value).toBe(
      "Half a thought",
    );
  });

  it("falls back to the list when the agent it was talking to is deleted", async () => {
    await talkToChef();

    act(() => useStore.setState({ agents: [agent("Scout")] }));

    expect(screen.queryByRole("textbox", { name: "Message Chef" })).toBeNull();
    expect(screen.getByRole("button", { name: /Scout/ })).toBeTruthy();
  });

  it("opens the whole conversation in the window", async () => {
    await talkToChef();

    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(mocks.openWindow).toHaveBeenCalledWith({ kind: "agent", id: "id-Chef" });
  });
});

describe("what the panel says at a glance", () => {
  it("says it is still connecting rather than that there is nobody", () => {
    render(<MenubarPanel ready={false} />);
    expect(screen.getByRole("status").textContent).toContain("Connecting");
    expect(screen.queryByText(/No agents yet/)).toBeNull();
  });

  it("answers a request where it is seen, and sends Open channel to the window", () => {
    useStore.setState({ pending: [waiting()] });
    render(<MenubarPanel ready />);

    expect(screen.getByText("1 waiting on you")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Open channel" }));
    expect(mocks.openWindow).toHaveBeenCalledWith({ kind: "agent", id: "id-Chef" });
  });

  it("counts decisions for For you, and opens For you in the window for them", () => {
    useStore.setState({ decisions: [aDecision(), aDecision({ id: "decision-2" })] });
    render(<MenubarPanel ready />);

    expect(screen.getByText("2 waiting on you")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /2 decisions need you/ }));
    expect(mocks.openWindow).toHaveBeenCalledWith({ kind: "forYou" });
  });

  it("lists whoever is working first, saying what they are doing", () => {
    useStore.setState({
      activity: { "id-Scout": { state: "thinking" } },
      lastActive: { "id-Chef": Date.now() },
    });
    render(<MenubarPanel ready />);

    const rows = screen.getAllByRole("button").filter((b) => b.className === "agent-row");
    expect(rows.map((row) => row.querySelector(".agent-row__name")?.textContent)).toEqual([
      "Scout",
      "Chef",
    ]);
    expect(screen.getByText("typing")).toBeTruthy();
    expect(screen.getByText("1 working")).toBeTruthy();
  });

  it("names a crew only when there is another to tell it from", () => {
    const { unmount } = render(<MenubarPanel ready />);
    expect(screen.queryByText("Everyone")).toBeNull();
    unmount();

    useStore.setState({
      groups: [aGroup(), aGroup({ id: "g-ops", name: "Ops" })],
      agents: [agent("Chef"), agent("Scout", { groupId: "g-ops" })],
    });
    render(<MenubarPanel ready />);
    expect(screen.getByText("Everyone")).toBeTruthy();
    expect(screen.getByText("Ops")).toBeTruthy();
  });

  it("offers a stop only while something is running", () => {
    const { unmount } = render(<MenubarPanel ready />);
    expect(screen.queryByRole("button", { name: /^Stop/ })).toBeNull();
    unmount();

    useStore.setState({ activeRun: { "id-Chef": "run-1" } });
    render(<MenubarPanel ready />);
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(mocks.stopEverything).toHaveBeenCalled();
  });

  it("says what the session spent in the floor the meters use", () => {
    useStore.setState({ sessionSpend: { prompt: 900, completion: 100, cost: 0, calls: 3 } });
    render(<MenubarPanel ready />);
    // A free model prices at a real zero, which is not a price.
    expect(screen.getByText("1.0k tokens this session")).toBeTruthy();
  });

  it("opens the window from the footer", () => {
    render(<MenubarPanel ready />);
    fireEvent.click(screen.getByRole("button", { name: "Open Guaca" }));
    expect(mocks.openWindow).toHaveBeenCalledWith(null);
  });
});
