import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentCard, Routine, Settings } from "./lib/types";
import { aGroup } from "./test-fixtures";

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
const agentActivity = vi.fn<() => Promise<Record<string, unknown>>>(async () => ({}));
const agentRoutines = vi.fn<() => Promise<Routine[]>>(async () => []);
const setAgentPinned = vi.fn<(id: string, pinned: boolean) => Promise<AgentCard>>();
const duplicateAgent = vi.fn<(id: string) => Promise<AgentCard>>();
const getSettings = vi.fn<() => Promise<Settings>>(async () => ({
  baseUrl: "https://openrouter.ai/api/v1",
  defaultModel: "test/model",
  operatorName: "",
  provider: "compatible",
  subscriptionModel: "gpt-5.6-luna",
  subscriptionModels: ["gpt-5.6-luna", "gpt-5.4-mini"],
  apiKeySet: true,
  e2bKeySet: false,
  e2bKeyHint: "",
  computerIdleMinutes: 15,
  kernelKeySet: false,
  kernelKeyHint: "",
  browserIdleMinutes: 60,
  browserStealth: false,
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
    // The shared fixture rather than a hand-built object: this one had drifted
    // into a mixture of a group and the app's settings, with a group's own
    // `inference` block missing entirely, and nothing typechecks a mock.
    listGroups: async () => [aGroup({ agentCount: 3 })],
    listRepositories: async () => [],
    agentActivity: () => agentActivity(),
    usageSummary: async () => [],
    approvalStates: async () => ({}),
    pendingApprovals: async () => [],
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
  onRevealRequest: async () => () => {},
  onFileDrop: async () => () => {},
}));

const { default: App } = await import("./App");
const { useStore } = await import("./lib/store");

function agent(name: string, railOrder = 0): AgentCard {
  return {
    id: `id-${name}`,
    railOrder,
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    repositoryId: null,
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

/** Every row in the rail, top to bottom. */
function railNames(): (string | null)[] {
  const rail = screen.getByRole("navigation", { name: /agents/i });
  return [...rail.querySelectorAll(".agent-row__name")].map((n) => n.textContent);
}

beforeEach(() => {
  vi.clearAllMocks();
  listAgents.mockResolvedValue([]);
  agentLastActive.mockResolvedValue({});
  agentActivity.mockResolvedValue({});
  useStore.setState({
    agents: [],
    activity: {},
    lastActive: {},
    settings: null,
    selected: null,
    railGroup: null,
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
    expect(await screen.findByRole("button", { name: /open the cafeteria/i })).toBeTruthy();
    expect(screen.getByRole("button", { name: /create one agent/i })).toBeTruthy();
  });

  it("opens the cafeteria from the empty state", async () => {
    // The one path a workspace with nothing in it is expected to take, so it
    // has to reach somewhere that can actually fill it.
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /open the cafeteria/i }));
    expect(await screen.findByRole("dialog", { name: /cafeteria/i })).toBeTruthy();
  });

  it("lists agents and opens a channel", async () => {
    listAgents.mockResolvedValue([agent("Manager"), agent("Chef")]);
    render(<App />);

    await waitFor(() => expect(screen.getByText("Manager")).toBeTruthy());
    expect(screen.getByText("Chef")).toBeTruthy();
    // A channel header, meaning the pane rendered rather than staying empty.
    await waitFor(() => expect(screen.getByRole("heading", { name: "Manager" })).toBeTruthy());
  });

  it("draws the rail in the order the operator arranged it", async () => {
    // Having spoken recently is not a reason to move. The rail used to be
    // ordered by nothing else, so a conversation rewrote the arrangement and
    // the row you reached for was the row that had just left.
    listAgents.mockResolvedValue([agent("Manager", 0), agent("Chef", 1), agent("Host", 2)]);
    agentLastActive.mockResolvedValue({ "id-Host": 900, "id-Chef": 500 });
    render(<App />);

    await waitFor(() => expect(screen.getByText("Manager")).toBeTruthy());
    expect(railNames()).toEqual(["Manager", "Chef", "Host"]);
  });

  it("lifts whoever is working to the top, and gives the place back", async () => {
    listAgents.mockResolvedValue([agent("Manager", 0), agent("Chef", 1), agent("Host", 2)]);
    agentActivity.mockResolvedValue({ "id-Host": { state: "thinking" } });
    render(<App />);

    await waitFor(() => expect(screen.getByText("Manager")).toBeTruthy());
    expect(railNames()).toEqual(["Host", "Manager", "Chef"]);

    // The turn ends, and the row goes back where it was put.
    useStore
      .getState()
      .applyEvent({ type: "activityChanged", agentId: "id-Host", activity: { state: "idle" } });
    await waitFor(() => expect(railNames()).toEqual(["Manager", "Chef", "Host"]));
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
      provider: "compatible",
      subscriptionModel: "gpt-5.6-luna",
      subscriptionModels: ["gpt-5.6-luna", "gpt-5.4-mini"],
      apiKeySet: false,
      e2bKeySet: false,
      e2bKeyHint: "",
      computerIdleMinutes: 15,
      kernelKeySet: false,
      kernelKeyHint: "",
      browserIdleMinutes: 60,
      browserStealth: false,
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

  it("does not ask for a key when a subscription is what pays", async () => {
    // There is nothing to paste on a subscription, so this banner asked forever
    // for something the Provider pane does not offer. The one place the app says
    // "you are not set up yet" has to keep meaning that.
    getSettings.mockResolvedValue({
      operatorName: "",
      baseUrl: "https://openrouter.ai/api/v1",
      defaultModel: "test/model",
      provider: "chatgpt",
      subscriptionModel: "gpt-5.6-luna",
      subscriptionModels: ["gpt-5.6-luna", "gpt-5.4-mini"],
      apiKeySet: false,
      e2bKeySet: false,
      e2bKeyHint: "",
      computerIdleMinutes: 15,
      kernelKeySet: false,
      kernelKeyHint: "",
      browserIdleMinutes: 60,
      browserStealth: false,
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
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    // Wait for the load that would have produced the banner before asserting it
    // is absent, or this passes on a render that had no settings yet.
    await waitFor(() => expect(screen.getByText("Manager")).toBeTruthy());
    expect(screen.queryByText(/Add an API key/i)).toBeNull();
  });

  it("puts a pinned agent above the rest and leaves it there", async () => {
    // The rail lifts whoever is working to the top of its section. A row pinned
    // so it could be found in one glance must not join in, wherever it sits in
    // the arrangement.
    listAgents.mockResolvedValue([
      agent("Manager", 0),
      { ...agent("Chef", 1), pinned: true },
      agent("Host", 2),
    ]);
    agentActivity.mockResolvedValue({ "id-Chef": { state: "thinking" } });
    render(<App />);

    await waitFor(() => expect(screen.getByText("Chef")).toBeTruthy());
    expect(railNames()).toEqual(["Chef", "Manager", "Host"]);
  });

  it("draws a pinned agent at the head of its crew rather than above the crews", async () => {
    // The pins had a section of their own above the groups, which made a pin
    // the one arrangement that came undone on the way into the crew it was
    // arranging. They are the head of the crew now, and counted in it: a pinned
    // agent is still in the group, still costs it money, and is still someone
    // its peers can message.
    listAgents.mockResolvedValue([agent("Manager"), { ...agent("Chef"), pinned: true }]);
    render(<App />);

    await waitFor(() => expect(screen.getByText("Chef")).toBeTruthy());
    expect(screen.queryByText("Pinned")).toBeNull();
    expect(railRow("Chef").closest(".rail__group")?.textContent).toContain("Everyone");
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
        skipIfWorking: false,
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
