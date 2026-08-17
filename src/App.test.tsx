import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard, Routine, Settings } from "./lib/types";

/**
 * Smoke test for the shell.
 *
 * A blank window is the worst failure this app can have: nothing throws, the
 * background paints, and there is no clue what went wrong. This mounts the real
 * component tree over a mocked IPC layer, so anything that empties the window
 * fails here instead of in a release bundle.
 */

const listAgents = vi.fn<() => Promise<AgentCard[]>>(async () => []);
const agentLastActive = vi.fn<() => Promise<Record<string, number>>>(async () => ({}));
const agentRoutines = vi.fn<() => Promise<Routine[]>>(async () => []);
const setAgentPinned = vi.fn<(id: string, pinned: boolean) => Promise<AgentCard>>();
const duplicateAgent = vi.fn<(id: string) => Promise<AgentCard>>();
const getSettings = vi.fn<() => Promise<Settings>>(async () => ({
  baseUrl: "https://openrouter.ai/api/v1",
  defaultModel: "test/model",
  operatorName: "",
  apiKeySet: true,
  e2bKeySet: false,
  e2bKeyHint: "",
  computerIdleMinutes: 15,
  apiKeyHint: "...9999",
  requestTimeoutSecs: 120,
  limits: {
    maxHops: 4,
    maxStepsPerRun: 40,
    maxFanoutPerCall: 8,
    maxSendsPerPair: 3,
    maxToolRounds: 24,
  },
}));

vi.mock("./lib/ipc", () => ({
  api: {
    listAgents: () => listAgents(),
    listGroups: async () => [
      {
        id: "00000000-0000-4000-8000-000000000001",
        name: "Everyone",
        agentCount: 3,
        createdAt: 0,
        baseUrl: null,
        defaultModel: null,
        apiKeySet: false,
        e2bKeySet: false,
        e2bKeyHint: "",
        computerIdleMinutes: 15,
        apiKeyHint: "",
      },
    ],
    agentActivity: async () => ({}),
    usageSummary: async () => [],
    approvalStates: async () => ({}),
    agentLastActive: () => agentLastActive(),
    getSettings: () => getSettings(),
    channelMessages: async () => [],
    activityFeed: async () => [],
    agentRoutines: () => agentRoutines(),
    agentComputer: async () => null,
    agentNotes: async () => "",
    agentSignins: async () => [],
    agentGrants: async () => [],
    setAgentPinned: (id: string, pinned: boolean) => setAgentPinned(id, pinned),
    duplicateAgent: (id: string) => duplicateAgent(id),
    createAgent: async () => {
      throw new Error("not used");
    },
  },
  onRuntimeEvent: async () => () => {},
  onFileDrop: async () => () => {},
}));

const { default: App } = await import("./App");
const { useStore } = await import("./lib/store");

function agent(name: string): AgentCard {
  return {
    id: `id-${name}`,
    groupId: "00000000-0000-4000-8000-000000000001",
    computerId: null,
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "test/model",
    systemPrompt: "",
    skills: ["testing"],
    lifecycle: "active",
    pinned: false,
    version: 1,
    createdAt: 1,
    updatedAt: 1,
  };
}

/**
 * The row in the rail, specifically.
 *
 * An agent's name is on its row, on its channel header and in any menu open
 * over it, so a bare text query is ambiguous the moment more than one of those
 * is on screen.
 */
function railRow(name: string): HTMLElement {
  const rail = screen.getByRole("navigation", { name: /agents/i });
  const row = [...rail.querySelectorAll(".agent-row__name")].find((n) => n.textContent === name);
  if (!row) throw new Error(`no row for ${name} in the rail`);
  return row as HTMLElement;
}

beforeEach(() => {
  vi.clearAllMocks();
  listAgents.mockResolvedValue([]);
  agentLastActive.mockResolvedValue({});
  useStore.setState({
    agents: [],
    activity: {},
    lastActive: {},
    settings: null,
    selected: null,
    messages: {},
    streams: {},
    pulses: [],
    banner: null,
  });
});

describe("App", () => {
  it("renders the rail", async () => {
    render(<App />);
    // The rail is always present, whether or not there are agents. If this is
    // missing, the window is blank.
    expect(await screen.findByRole("navigation", { name: /agents/i })).toBeTruthy();
    expect(screen.getByText("Guaca")).toBeTruthy();
  });

  it("offers a way in when there are no agents", async () => {
    render(<App />);
    expect(await screen.findByRole("button", { name: /starter crew/i })).toBeTruthy();
    expect(screen.getByRole("button", { name: /create one agent/i })).toBeTruthy();
  });

  it("lists agents and opens a channel", async () => {
    listAgents.mockResolvedValue([agent("Manager"), agent("Chef")]);
    render(<App />);

    await waitFor(() => expect(screen.getByText("Manager")).toBeTruthy());
    expect(screen.getByText("Chef")).toBeTruthy();
    // A channel header, meaning the pane rendered rather than staying empty.
    await waitFor(() => expect(screen.getByRole("heading", { name: "Manager" })).toBeTruthy());
  });

  it("puts the most recently active agent at the top of the rail", async () => {
    listAgents.mockResolvedValue([agent("Manager"), agent("Chef"), agent("Host")]);
    agentLastActive.mockResolvedValue({ "id-Host": 900, "id-Chef": 500 });
    render(<App />);

    await waitFor(() => expect(screen.getByText("Manager")).toBeTruthy());
    const rail = screen.getByRole("navigation", { name: /agents/i });
    const names = [...rail.querySelectorAll(".agent-row__name")].map((n) => n.textContent);
    // Host spoke last, then Chef. Manager has never spoken, so it keeps its
    // creation position at the bottom.
    expect(names).toEqual(["Host", "Chef", "Manager"]);
  });

  it("keeps deleted agents out of the rail", async () => {
    listAgents.mockResolvedValue([
      agent("Manager"),
      { ...agent("Ghost"), lifecycle: "terminated" },
    ]);
    render(<App />);
    await waitFor(() => expect(screen.getByText("Manager")).toBeTruthy());
    expect(screen.queryByText("Ghost")).toBeNull();
  });

  it("prompts for a key when none is configured", async () => {
    getSettings.mockResolvedValue({
      operatorName: "",
      baseUrl: "https://openrouter.ai/api/v1",
      defaultModel: "test/model",
      apiKeySet: false,
      e2bKeySet: false,
      e2bKeyHint: "",
      computerIdleMinutes: 15,
      apiKeyHint: "",
      requestTimeoutSecs: 120,
      limits: {
        maxHops: 4,
        maxStepsPerRun: 40,
        maxFanoutPerCall: 8,
        maxSendsPerPair: 3,
        maxToolRounds: 24,
      },
    });
    render(<App />);
    expect(await screen.findByText(/Add an API key/i)).toBeTruthy();
  });

  it("puts a pinned agent above the rest and leaves it there", async () => {
    // The rail floats whoever just spoke to the top. A row pinned so it could
    // be found in one glance must not join in.
    listAgents.mockResolvedValue([
      agent("Manager"),
      { ...agent("Chef"), pinned: true },
      agent("Host"),
    ]);
    agentLastActive.mockResolvedValue({ "id-Host": 900, "id-Manager": 500 });
    render(<App />);

    await waitFor(() => expect(screen.getByText("Chef")).toBeTruthy());
    const rail = screen.getByRole("navigation", { name: /agents/i });
    const names = [...rail.querySelectorAll(".agent-row__name")].map((n) => n.textContent);
    expect(names).toEqual(["Chef", "Host", "Manager"]);
    expect(screen.getByText("Pinned")).toBeTruthy();
  });

  it("counts a pinned agent in its group even though the row is drawn above", async () => {
    // It is still in the group, still costs it money, and its peers can still
    // message it. Only where the row is drawn has changed.
    listAgents.mockResolvedValue([agent("Manager"), { ...agent("Chef"), pinned: true }]);
    render(<App />);

    await waitFor(() => expect(screen.getByText("Chef")).toBeTruthy());
    expect(screen.getByText("Everyone").closest(".rail__group-head")?.textContent).toContain("2");
  });

  it("pins and duplicates from the menu that right-clicking opens", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    setAgentPinned.mockResolvedValue({ ...agent("Manager"), pinned: true });
    duplicateAgent.mockResolvedValue(agent("Manager copy"));
    render(<App />);

    await waitFor(() => railRow("Manager"));
    fireEvent.contextMenu(railRow("Manager"));

    // Right-clicking used to open the whole profile dialog. It opens a menu
    // now, and the dialog is one deliberate click further away.
    expect(screen.queryByRole("dialog", { name: /edit agent/i })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Pin to top" }));
    await waitFor(() => expect(setAgentPinned).toHaveBeenCalledWith("id-Manager", true));

    fireEvent.contextMenu(railRow("Manager"));
    fireEvent.click(screen.getByRole("button", { name: "Duplicate" }));
    await waitFor(() => expect(duplicateAgent).toHaveBeenCalledWith("id-Manager"));
  });

  it("reaches the profile dialog in two clicks, not one", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);

    await waitFor(() => railRow("Manager"));
    fireEvent.contextMenu(railRow("Manager"));
    fireEvent.click(screen.getByRole("button", { name: "Edit profile" }));
    expect(await screen.findByRole("dialog", { name: /edit agent/i })).toBeTruthy();
  });

  it("shows the open agent's screen and routines beside the transcript", async () => {
    // Both used to be behind the dialog, which meant a standing commitment and
    // a live desktop were things you had to open a modal to find.
    listAgents.mockResolvedValue([agent("Manager")]);
    agentRoutines.mockResolvedValue([
      {
        id: "r1",
        agentId: "id-Manager",
        name: "Boss commitment nudge",
        what: "check what I promised",
        trigger: "weekdays",
        active: true,
        nextRunAt: new Date(2025, 5, 10, 9, 28).getTime(),
        lastRunAt: null,
        createdAt: 0,
      },
    ]);
    render(<App />);

    expect(await screen.findByText("Boss commitment nudge")).toBeTruthy();
    expect(screen.getByText(/Weekdays at 9:28/)).toBeTruthy();
    expect(screen.getByRole("complementary", { name: /screen and routines/i })).toBeTruthy();
  });

  it("still renders the rail when startup fails", async () => {
    // A failed bootstrap must degrade to a usable window with an error, not to
    // a blank one.
    listAgents.mockRejectedValue({ kind: "storage", message: "disk on fire" });
    render(<App />);
    expect(await screen.findByRole("navigation", { name: /agents/i })).toBeTruthy();
    expect(await screen.findByText(/disk on fire/i)).toBeTruthy();
  });
});

describe("failure surfacing", () => {
  it("shows the error instead of a blank window when a child throws", async () => {
    const { ErrorBoundary } = await import("./components/ErrorBoundary");
    const Boom = (): never => {
      throw new Error("render exploded");
    };
    // React logs the caught error; silence it so the run stays readable.
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});
    render(
      <ErrorBoundary>
        <Boom />
      </ErrorBoundary>,
    );
    expect(screen.getByText(/could not draw this window/i)).toBeTruthy();
    expect(screen.getByText(/render exploded/)).toBeTruthy();
    spy.mockRestore();
  });
});
