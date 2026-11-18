import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { expectRestart, forgetRestart } from "./lib/link";
import { saveFirstTask } from "./lib/onboarding";
import type { AgentCard, Routine, Settings } from "./lib/types";
import { aGroup } from "./test-fixtures";

const listGroups = vi.fn(async () => [aGroup({ agentCount: 3 })]);
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
  reasoningEffort: "auto",
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
    listGroups: () => listGroups(),
    agentActivity: () => agentActivity(),
    usageSummary: async () => [],
    approvalStates: async () => ({}),
    approvalAnswers: async () => ({}),
    pendingApprovals: async () => [],
    listDecisions: async () => [],
    openEscalations: async () => [],
    agentLastActive: () => agentLastActive(),
    getSettings: () => getSettings(),
    subscriptionModels: async () =>
      ["gpt-5.6-luna"].map((slug) => ({
        slug,
        defaultReasoningEffort: "medium",
        reasoningEfforts: [
          { effort: "low", description: "Faster" },
          { effort: "high", description: "More thorough" },
        ],
      })),
    reportPresence: async () => {},
    reportHost: async () => {},
    reportView: async () => {},
    capabilities: async () => ({
      loopbackEndpoints: true,
      claudeProvider: true,
      localFiles: true,
      borrowedSignins: true,
    }),
    claudeProgram: async () => ({
      installed: true,
      version: "2.1.247 (Claude Code)",
      install: "npm install -g @anthropic-ai/claude-code",
    }),
    channelMessages: async () => [],
    activityFeed: async () => [],
    agentRoutines: () => agentRoutines(),
    calendar: async () => [],
    upcomingRuns: async () => [],
    listOutcomes: async () => [],
    artifacts: async () => [],
    agentComputer: async () => null,
    agentMemory: async () => "",
    agentMemoryVersioned: async () => ({ content: "", version: "version" }),
    agentSignins: async () => [],
    agentGrants: async () => [],
    setAgentPinned: (id: string, pinned: boolean) => setAgentPinned(id, pinned),
    duplicateAgent: (id: string) => duplicateAgent(id),
    createAgent: async () => {
      throw new Error("not used");
    },
  },
  onRuntimeEvent: vi.fn(async (callback) => {
    callback({ type: "liveSnapshot", activity: await agentActivity(), streams: {}, building: {} });
    return () => {};
  }),
  onRevealRequest: vi.fn(async () => () => {}),
  onFileDrop: async () => () => {},
}));

const { default: App } = await import("./App");
const { useStore } = await import("./lib/store");
const { useHeld } = await import("./lib/held");

function agent(name: string, railOrder = 0): AgentCard {
  return {
    id: `id-${name}`,
    railOrder,
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    browserConsent: "open",
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "test/model",
    subscriptionModel: "",
    systemPrompt: "",
    skills: ["testing"],
    lifecycle: "active",
    pinned: false,
    version: 1,
    createdAt: 1,
    updatedAt: 1,
    discardedAt: null,
  };
}

function groupsSheet(): HTMLElement {
  const open = screen.queryByRole("dialog", { name: "Groups and agents" });
  if (open) return open;
  fireEvent.click(screen.getByRole("button", { name: /^(Groups and agents|Agents)$/ }));
  return screen.getByRole("dialog", { name: "Groups and agents" });
}

function railRow(name: string): HTMLElement {
  const rail = within(groupsSheet()).getByRole("navigation", { name: /agents/i });
  const row = [...rail.querySelectorAll(".agent-row__name")].find((n) => n.textContent === name);
  if (!row) throw new Error(`no row for ${name} in the rail`);
  return row as HTMLElement;
}

function railNames(): (string | null)[] {
  const rail = within(groupsSheet()).getByRole("navigation", { name: /agents/i });
  return [...rail.querySelectorAll(".agent-row__name")].map((n) => n.textContent);
}

beforeEach(() => {
  vi.clearAllMocks();
  listAgents.mockResolvedValue([]);
  listGroups.mockResolvedValue([aGroup({ agentCount: 3 })]);
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
    artifactsOpen: null,
    link: "up",
    lastHeard: null,
  });
  forgetRestart();
});

describe("when the host goes away", () => {
  const down = {
    tone: "error" as const,
    text: "could not reach this workspace at http://127.0.0.1:8787. It may be starting up, or the connection to it is down. Its agents keep working either way",
    fix: "host" as const,
  };

  it("offers a retry and the place hosts are changed, not only a dismissal", async () => {
    useStore.setState({ banner: down });
    render(<App />);
    expect(screen.getByText(/could not reach this workspace/)).toBeTruthy();
    expect(screen.getByText(/Check that Docker is running/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Check the host" }));
    const pane = await screen.findByRole("tab", { name: "Host" });
    expect(pane.getAttribute("aria-selected")).toBe("true");
  });

  it("takes the banner away once a retry gets an answer", async () => {
    useStore.setState({ banner: down });
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(useStore.getState().banner).toBeNull());
    expect(screen.queryByText(/could not reach this workspace/)).toBeNull();
  });

  it("leaves it up when the host still is not there", async () => {
    listAgents.mockRejectedValue({ kind: "unreachable", message: down.text });
    render(<App />);
    await screen.findByRole("button", { name: "Retry" });
    const before = listAgents.mock.calls.length;

    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(listAgents.mock.calls.length).toBeGreaterThan(before));
    expect(useStore.getState().banner?.fix).toBe("host");
    expect(screen.getByText(/Check that Docker is running/)).toBeTruthy();
  });

  it("says nothing about Docker to a failure that is not the host", async () => {
    useStore.setState({ banner: { tone: "error", text: "the disk is full" } });
    render(<App />);
    expect(screen.getByText("the disk is full")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
    expect(screen.queryByText(/Check that Docker is running/)).toBeNull();
  });
});

describe("App", () => {
  it("renders the ribbon and reaches every agent through the Groups sheet", async () => {
    render(<App />);
    expect(await screen.findByRole("navigation", { name: /agents/i })).toBeTruthy();
    expect(within(groupsSheet()).getByText("guaca")).toBeTruthy();
  });

  it("names the top row's settings button App settings, apart from each group's own", async () => {
    render(<App />);
    const row = await screen.findByRole("navigation", { name: "Window" });
    fireEvent.click(within(row).getByRole("button", { name: "App settings" }));
    expect(await screen.findByRole("dialog", { name: "Settings" })).toBeTruthy();
  });

  it("offers a way in when there are no agents", async () => {
    render(<App />);
    expect(await screen.findByRole("button", { name: /open the cafeteria/i })).toBeTruthy();
    expect(screen.getByRole("button", { name: /create one agent/i })).toBeTruthy();
  });

  it("opens the cafeteria from the empty state", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /open the cafeteria/i }));
    expect(await screen.findByRole("dialog", { name: /cafeteria/i })).toBeTruthy();
  });

  it("keeps the cafeteria open when settings is opened and dismissed above it", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /open the cafeteria/i }));
    const cafeteria = await screen.findByRole("dialog", { name: /cafeteria/i });
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    const settings = await screen.findByRole("dialog", { name: /settings/i });
    expect(cafeteria.isConnected).toBe(true);
    expect(document.activeElement).toBe(settings);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: /settings/i })).toBeNull();
    expect(cafeteria.isConnected).toBe(true);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(cafeteria.isConnected).toBe(false);
  });

  it("leaves settings open when search above it is dismissed", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /open the cafeteria/i }));
    const cafeteria = await screen.findByRole("dialog", { name: /cafeteria/i });
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    const settings = await screen.findByRole("dialog", { name: /settings/i });
    fireEvent.keyDown(window, { key: "f", ctrlKey: true });
    const search = await screen.findByRole("dialog", { name: /search/i });
    expect(settings.closest(".scrim")?.nextElementSibling).toBe(search.closest(".scrim"));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: /search/i })).toBeNull();
    expect(settings.isConnected).toBe(true);
    expect(cafeteria.isConnected).toBe(true);
  });

  it("opens settings above search and dismisses it first", async () => {
    render(<App />);
    await screen.findByRole("heading", { name: "What would you like help with?" });
    fireEvent.keyDown(window, { key: "f", ctrlKey: true });
    const search = await screen.findByRole("dialog", { name: "Search" });
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    const settings = await screen.findByRole("dialog", { name: "Settings" });
    expect(search.isConnected).toBe(true);
    expect(search.closest(".scrim")?.nextElementSibling).toBe(settings.closest(".scrim"));
    fireEvent.keyDown(settings, { key: "Enter" });
    expect(search.isConnected).toBe(true);
    expect(settings.isConnected).toBe(true);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(settings.isConnected).toBe(false);
    expect(search.isConnected).toBe(true);
    expect(screen.getByRole("dialog", { name: "Search" })).toBe(search);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(search.isConnected).toBe(false);
  });

  it.each(["Calendar", "Artifacts"])(
    "keeps %s open under settings and restores focus in order",
    async (name) => {
      render(<App />);
      const heading = await screen.findByRole("heading", {
        name: "What would you like help with?",
      });
      await waitFor(() => expect(document.activeElement).toBe(heading));
      const opener = screen.getByRole("button", { name: new RegExp(`^${name}`) });
      opener.focus();
      fireEvent.click(opener);
      const lower = await screen.findByRole("dialog", { name });
      await waitFor(() => expect(document.activeElement).toBe(lower));
      fireEvent.keyDown(window, { key: ",", ctrlKey: true });
      const settings = await screen.findByRole("dialog", { name: "Settings" });
      await waitFor(() => expect(document.activeElement).toBe(settings));
      fireEvent.keyDown(window, { key: "Escape" });
      expect(settings.isConnected).toBe(false);
      expect(lower.isConnected).toBe(true);
      expect(document.activeElement).toBe(lower);
      fireEvent.keyDown(window, { key: "Escape" });
      expect(lower.isConnected).toBe(false);
      expect(document.activeElement).toBe(opener);
    },
  );

  it.each(["Calendar", "Artifacts"])("keeps %s open under search", async (name) => {
    render(<App />);
    await screen.findByRole("heading", { name: "What would you like help with?" });
    fireEvent.click(screen.getByRole("button", { name: new RegExp(`^${name}`) }));
    const lower = await screen.findByRole("dialog", { name });
    await waitFor(() => expect(document.activeElement).toBe(lower));
    fireEvent.keyDown(window, { key: "f", ctrlKey: true });
    const search = await screen.findByRole("dialog", { name: /search/i });
    fireEvent.keyDown(window, { key: "Escape" });
    expect(search.isConnected).toBe(false);
    expect(lower.isConnected).toBe(true);
    expect(document.activeElement).toBe(lower);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(lower.isConnected).toBe(false);
  });

  it.each([
    ["Settings", ","],
    ["Search", "f"],
  ])("keeps For you open under %s and restores focus", async (upperName, key) => {
    render(<App />);
    await screen.findByRole("heading", { name: "What would you like help with?" });
    const opener = within(screen.getByRole("navigation", { name: "Places" })).getByRole("button", {
      name: /^For you/,
    });
    opener.focus();
    fireEvent.click(opener);
    const lower = await screen.findByRole("dialog", { name: "For you" });
    await waitFor(() => expect(document.activeElement).toBe(lower));
    fireEvent.keyDown(window, { key, ctrlKey: true });
    const upper = await screen.findByRole("dialog", { name: upperName });
    fireEvent.keyDown(window, { key: "Escape" });
    expect(upper.isConnected).toBe(false);
    expect(lower.isConnected).toBe(true);
    expect(document.activeElement).toBe(lower);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(lower.isConnected).toBe(false);
    expect(document.activeElement).toBe(opener);
  });

  it.each([
    ["Settings", ","],
    ["Search", "f"],
  ])("keeps the calendar editor under %s and unwinds each layer", async (upperName, key) => {
    render(<App />);
    await screen.findByRole("heading", { name: "What would you like help with?" });
    const opener = screen.getByRole("button", { name: /^Calendar/ });
    opener.focus();
    fireEvent.click(opener);
    const calendar = await screen.findByRole("dialog", { name: "Calendar" });
    const add = screen.getByRole("button", { name: "Add" });
    add.focus();
    fireEvent.click(add);
    const title = await screen.findByRole("textbox", { name: "What is happening" });
    await waitFor(() => expect(document.activeElement).toBe(title));
    fireEvent.keyDown(window, { key, ctrlKey: true });
    const upper = await screen.findByRole("dialog", { name: upperName });
    fireEvent.keyDown(window, { key: "Escape" });
    expect(upper.isConnected).toBe(false);
    expect(title.isConnected).toBe(true);
    expect(document.activeElement).toBe(title);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(title.isConnected).toBe(false);
    expect(calendar.isConnected).toBe(true);
    expect(document.activeElement).toBe(add);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(calendar.isConnected).toBe(false);
    expect(document.activeElement).toBe(opener);
  });

  it.each([
    ["Settings", ","],
    ["Search", "f"],
  ])("keeps an agent submenu under %s and unwinds each layer", async (upperName, key) => {
    listAgents.mockResolvedValue([agent("Manager")]);
    listGroups.mockResolvedValue([
      aGroup({ agentCount: 1 }),
      aGroup({ id: "00000000-0000-4000-8000-000000000002", name: "Other" }),
    ]);
    render(<App />);
    await waitFor(() => railRow("Manager"));
    const row = railRow("Manager").closest("button") as HTMLButtonElement;
    row.focus();
    fireEvent.contextMenu(row);
    const menu = screen.getByRole("menu", { name: "Manager" });
    const move = screen.getByRole("button", { name: "Move to another group" });
    fireEvent.pointerEnter(move.parentElement as HTMLElement);
    const submenu = screen.getByRole("menu", { name: "Move to another group" });
    fireEvent.keyDown(window, { key, ctrlKey: true });
    const upper = await screen.findByRole("dialog", { name: upperName });
    fireEvent.keyDown(window, { key: "Escape" });
    expect(upper.isConnected).toBe(false);
    expect(submenu.isConnected).toBe(true);
    expect(document.activeElement).toBe(row);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(submenu.isConnected).toBe(false);
    expect(menu.isConnected).toBe(true);
    expect(document.activeElement).toBe(row);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(menu.isConnected).toBe(false);
    expect(document.activeElement).toBe(row);
  });

  it.each(["New agent", "New group"])(
    "restores the menu opener after %s replaces the menu",
    async (name) => {
      render(<App />);
      await screen.findByRole("heading", { name: "What would you like help with?" });
      const opener = await screen.findByRole("button", { name: "Make something new" });
      opener.focus();
      fireEvent.click(opener);
      const item = screen.getByRole("menuitem", { name: new RegExp(`^${name}`) });
      item.focus();
      fireEvent.click(item);
      const dialog = await screen.findByRole("dialog", { name });
      fireEvent.keyDown(window, { key: "Escape" });
      expect(dialog.isConnected).toBe(false);
      expect(document.activeElement).toBe(opener);
    },
  );

  it.each([
    ["New agent", "Settings", ","],
    ["New agent", "Search", "f"],
    ["New group", "Settings", ","],
    ["New group", "Search", "f"],
  ])("keeps %s open under %s", async (name, upperName, key) => {
    render(<App />);
    await screen.findByRole("heading", { name: "What would you like help with?" });
    fireEvent.click(screen.getByRole("button", { name: "Make something new" }));
    fireEvent.click(screen.getByRole("menuitem", { name: new RegExp(`^${name}`) }));
    const lower = await screen.findByRole("dialog", { name });
    const focused = document.activeElement;
    fireEvent.keyDown(window, { key, ctrlKey: true });
    const upper = await screen.findByRole("dialog", { name: upperName });
    fireEvent.keyDown(window, { key: "Escape" });
    expect(upper.isConnected).toBe(false);
    expect(lower.isConnected).toBe(true);
    expect(document.activeElement).toBe(focused);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(lower.isConnected).toBe(false);
  });

  it("opens on today rather than on the first agent's channel", async () => {
    listAgents.mockResolvedValue([agent("Manager"), agent("Chef")]);
    render(<App />);
    await waitFor(() => expect(useStore.getState().agents).toHaveLength(2));
    await act(async () => {});
    expect(useStore.getState().selected).toBeNull();
    expect(screen.queryByRole("navigation", { name: "Manager's page" })).toBeNull();
  });

  it("goes back to today when the open agent is deleted", async () => {
    listAgents.mockResolvedValue([agent("Manager"), agent("Chef")]);
    render(<App />);
    await waitFor(() => expect(railRow("Manager")).toBeTruthy());
    fireEvent.click(railRow("Manager"));
    await screen.findByRole("navigation", { name: "Manager's page" });
    listAgents.mockResolvedValue([agent("Chef")]);
    await act(() => useStore.getState().refreshAgents());
    expect(useStore.getState().selected).toBeNull();
    expect(screen.queryByRole("navigation", { name: "Manager's page" })).toBeNull();
    expect(screen.queryByRole("navigation", { name: "Chef's page" })).toBeNull();
  });

  it("lands a notification for an agent on that agent's conversation", async () => {
    const { onRevealRequest } = await import("./lib/ipc");
    listAgents.mockResolvedValue([agent("Manager"), agent("Chef")]);
    render(<App />);
    await waitFor(() => expect(vi.mocked(onRevealRequest)).toHaveBeenCalled());
    const reveal = vi.mocked(onRevealRequest).mock.calls.at(-1)![0];
    await waitFor(() => expect(railRow("Chef")).toBeTruthy());
    fireEvent.click(railRow("Chef"));
    const chef = await screen.findByRole("navigation", { name: "Chef's page" });
    fireEvent.click(within(chef).getByRole("button", { name: "Memory" }));

    await act(async () => reveal({ kind: "agent", id: "id-Manager" }));

    const page = await screen.findByRole("navigation", { name: "Manager's page" });
    expect(within(page).getByRole("button", { current: "page" }).textContent).toBe("Conversation");
    expect(screen.getByRole("log", { name: "Conversation with Manager" })).toBeTruthy();
  });

  it("lands a notification on the conversation even while that agent is working", async () => {
    const { onRevealRequest } = await import("./lib/ipc");
    listAgents.mockResolvedValue([agent("Manager"), agent("Chef")]);
    agentActivity.mockResolvedValue({ "id-Manager": { state: "thinking" } });
    render(<App />);
    await waitFor(() => expect(vi.mocked(onRevealRequest)).toHaveBeenCalled());
    const reveal = vi.mocked(onRevealRequest).mock.calls.at(-1)![0];
    await waitFor(() => expect(useStore.getState().activity["id-Manager"]?.state).toBe("thinking"));

    await act(async () => reveal({ kind: "agent", id: "id-Manager" }));

    const page = await screen.findByRole("navigation", { name: "Manager's page" });
    expect(within(page).getByRole("button", { current: "page" }).textContent).toBe("Conversation");
  });

  describe("reveal ordering", () => {
    async function workingWindow() {
      const { onRevealRequest } = await import("./lib/ipc");
      listAgents.mockResolvedValue([agent("Manager"), agent("Chef")]);
      agentActivity.mockResolvedValue({
        "id-Manager": { state: "thinking" },
        "id-Chef": { state: "thinking" },
      });
      render(<App />);
      await waitFor(() => expect(vi.mocked(onRevealRequest)).toHaveBeenCalled());
      await waitFor(() => expect(useStore.getState().activity["id-Chef"]?.state).toBe("thinking"));
      return vi.mocked(onRevealRequest).mock.calls.at(-1)![0];
    }

    function currentTab(name: string) {
      const page = screen.getByRole("navigation", { name: `${name}'s page` });
      return within(page).getByRole("button", { current: "page" }).textContent;
    }

    it("keeps duplicate same-target reveals in one task on Conversation", async () => {
      const reveal = await workingWindow();

      act(() => {
        reveal({ kind: "agent", id: "id-Manager" });
        reveal({ kind: "agent", id: "id-Manager" });
      });

      expect(currentTab("Manager")).toBe("Conversation");
    });

    it.each([
      ["Manager", "Chef"],
      ["Chef", "Manager"],
    ])("lands on the last revealed target from %s to %s", async (first, last) => {
      const reveal = await workingWindow();

      act(() => {
        reveal({ kind: "agent", id: `id-${first}` });
        reveal({ kind: "agent", id: `id-${last}` });
        reveal({ kind: "agent", id: `id-${last}` });
      });

      expect(currentTab(last)).toBe("Conversation");
      expect(screen.queryByRole("navigation", { name: `${first}'s page` })).toBeNull();
    });

    it("does not lend a reveal's Conversation intent to another selected agent", async () => {
      const reveal = await workingWindow();

      await act(async () => {
        reveal({ kind: "agent", id: "id-Manager" });
        await useStore.getState().select("id-Chef");
      });

      expect(currentTab("Chef")).toBe("Now");
      fireEvent.click(railRow("Manager"));
      expect(currentTab("Manager")).toBe("Now");
    });

    it("drops a reveal that Today cancels before it renders, so a later click opens Now", async () => {
      const reveal = await workingWindow();

      act(() => {
        reveal({ kind: "agent", id: "id-Manager" });
        useStore.getState().leaveChannel();
      });
      expect(useStore.getState().selected).toBeNull();
      expect(screen.queryByRole("navigation", { name: "Manager's page" })).toBeNull();

      fireEvent.click(railRow("Manager"));
      expect(currentTab("Manager")).toBe("Now");
    });

    it("opens a working agent on Now after leaving a revealed conversation for Today", async () => {
      const reveal = await workingWindow();
      fireEvent.click(railRow("Manager"));
      expect(currentTab("Manager")).toBe("Now");

      act(() => reveal({ kind: "agent", id: "id-Manager" }));
      expect(currentTab("Manager")).toBe("Conversation");
      fireEvent.click(screen.getByRole("button", { name: "Today" }));
      expect(screen.queryByRole("navigation", { name: "Manager's page" })).toBeNull();

      fireEvent.click(railRow("Manager"));
      expect(currentTab("Manager")).toBe("Now");
    });
  });

  it("lists agents and opens a channel", async () => {
    listAgents.mockResolvedValue([agent("Manager"), agent("Chef")]);
    render(<App />);

    await waitFor(() => expect(screen.getByText("Manager")).toBeTruthy());
    expect(railRow("Chef")).toBeTruthy();
    fireEvent.click(railRow("Manager"));
    await waitFor(() => expect(screen.getByRole("heading", { name: "Manager" })).toBeTruthy());
  });

  it("draws the rail in the order the operator arranged it", async () => {
    listAgents.mockResolvedValue([agent("Manager", 0), agent("Chef", 1), agent("Host", 2)]);
    agentLastActive.mockResolvedValue({ "id-Host": 900, "id-Chef": 500 });
    render(<App />);

    await waitFor(() => expect(railRow("Manager")).toBeTruthy());
    expect(railNames()).toEqual(["Manager", "Chef", "Host"]);
  });

  it("lifts whoever is working to the top, and gives the place back", async () => {
    listAgents.mockResolvedValue([agent("Manager", 0), agent("Chef", 1), agent("Host", 2)]);
    agentActivity.mockResolvedValue({ "id-Host": { state: "thinking" } });
    render(<App />);

    await waitFor(() => expect(screen.getByText("Manager")).toBeTruthy());
    expect(railNames()).toEqual(["Host", "Manager", "Chef"]);

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

  it("starts a workspace with no provider at the first-run page", async () => {
    getSettings.mockResolvedValue({
      operatorName: "",
      baseUrl: "https://openrouter.ai/api/v1",
      defaultModel: "test/model",
      provider: "compatible",
      subscriptionModel: "gpt-5.6-luna",
      reasoningEffort: "auto",
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
    expect(await screen.findByText(/What would you like help with/i)).toBeTruthy();
    expect(screen.queryByText(/Add an API key/i)).toBeNull();
  });

  it("resumes a keyed first task when a lost hire response left an agent in the workspace", async () => {
    saveFirstTask({
      task: "plan",
      brief: "Plan our launch",
      agentId: null,
      sent: false,
      key: "request-key",
    });
    listAgents.mockResolvedValue([agent("Product Manager")]);
    try {
      render(<App />);
      expect(await screen.findByRole("heading", { name: "Ready when you are." })).toBeTruthy();
      expect(screen.getByRole("button", { name: "Start task" })).toBeTruthy();
    } finally {
      saveFirstTask(null);
    }
  });

  it.each([
    { paid: true, banner: false },
    { paid: false, banner: true },
  ])(
    "on Today, asks for a key only when no group with agents can reply: %j",
    async ({ paid, banner }) => {
      getSettings.mockResolvedValue({
        ...(await getSettings()),
        provider: "compatible",
        apiKeySet: false,
      });
      const first = aGroup({ apiKeySet: paid });
      const second = aGroup({ id: "other-group", name: "Other", apiKeySet: false });
      listGroups.mockResolvedValue([first, second]);
      listAgents.mockResolvedValue([
        agent("Manager"),
        { ...agent("Other agent"), groupId: second.id },
      ]);
      render(<App />);
      await waitFor(() => expect(useStore.getState().agents).toHaveLength(2));
      await waitFor(() => expect(useStore.getState().settings).not.toBeNull());
      await act(async () => {});
      expect(useStore.getState().selected).toBeNull();
      expect(screen.queryByText(/Add an API key/i) !== null).toBe(banner);
    },
  );

  it.each([
    { provider: "compatible" as const, apiKeySet: true },
    { provider: "chatgpt" as const, apiKeySet: false },
  ])("uses the selected group's provider and credentials: %j", async ({ provider, apiKeySet }) => {
    getSettings.mockResolvedValue({
      ...(await getSettings()),
      provider: "compatible",
      apiKeySet: false,
    });
    const paid = aGroup({ apiKeySet });
    paid.inference.provider = provider;
    const unpaid = aGroup({ id: "other-group", name: "Other", apiKeySet: false });
    listGroups.mockResolvedValue([paid, unpaid]);
    listAgents.mockResolvedValue([
      agent("Manager"),
      { ...agent("Other agent"), groupId: unpaid.id },
    ]);
    render(<App />);
    await waitFor(() => expect(screen.getByText("Manager")).toBeTruthy());
    fireEvent.click(railRow("Manager"));
    await waitFor(() => expect(screen.queryByText(/Add an API key/i)).toBeNull());
    fireEvent.click(railRow("Other agent"));
    expect(await screen.findByText(/Add an API key/i)).toBeTruthy();
  });

  it("does not ask for a key for a custom endpoint, which may not need one", async () => {
    getSettings.mockResolvedValue({
      ...(await getSettings()),
      provider: "compatible",
      baseUrl: "http://127.0.0.1:9/v1",
      apiKeySet: false,
    });
    listGroups.mockResolvedValue([aGroup({ apiKeySet: false })]);
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    await waitFor(() =>
      expect(useStore.getState().settings?.baseUrl).toBe("http://127.0.0.1:9/v1"),
    );
    await waitFor(() => expect(useStore.getState().agents).toHaveLength(1));
    await act(async () => {});
    expect(screen.queryByText(/Add an API key/i)).toBeNull();
  });

  it("does not ask for a key when a subscription is what pays", async () => {
    getSettings.mockResolvedValue({
      operatorName: "",
      baseUrl: "https://openrouter.ai/api/v1",
      defaultModel: "test/model",
      provider: "chatgpt",
      subscriptionModel: "gpt-5.6-luna",
      reasoningEffort: "auto",
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
    await waitFor(() => expect(screen.getByText("Manager")).toBeTruthy());
    expect(screen.queryByText(/Add an API key/i)).toBeNull();
  });

  it("puts a pinned agent above the rest and leaves it there", async () => {
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
    listAgents.mockResolvedValue([agent("Manager"), { ...agent("Chef"), pinned: true }]);
    render(<App />);

    await waitFor(() => expect(railRow("Chef")).toBeTruthy());
    expect(screen.queryByText("Pinned")).toBeNull();
    const crew = railRow("Chef").closest(".rail__group");
    expect(crew?.textContent).toContain("Everyone");
    expect(
      [...(crew?.querySelectorAll(".agent-row__name") ?? [])].map((n) => n.textContent),
    ).toEqual(["Chef", "Manager"]);
  });

  it("pins and duplicates from the menu that right-clicking opens", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    setAgentPinned.mockResolvedValue({ ...agent("Manager"), pinned: true });
    duplicateAgent.mockResolvedValue(agent("Manager copy"));
    render(<App />);

    await waitFor(() => railRow("Manager"));
    fireEvent.contextMenu(railRow("Manager"));

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

  it("shows the open agent's screen and routines on the agent's page", async () => {
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
    await waitFor(() => expect(railRow("Manager")).toBeTruthy());
    fireEvent.click(railRow("Manager"));
    fireEvent.click(
      within(await screen.findByRole("navigation", { name: "Manager's page" })).getByRole(
        "button",
        {
          name: "Routines",
        },
      ),
    );

    expect(await screen.findByText("Boss commitment nudge")).toBeTruthy();
    expect(screen.getByText(/Weekdays at 9:28/)).toBeTruthy();
    expect(screen.getByRole("complementary", { name: /Manager: details/i })).toBeTruthy();
  });

  it("still renders the rail when startup fails", async () => {
    listAgents.mockRejectedValue({ kind: "storage", message: "disk on fire" });
    render(<App />);
    expect(await screen.findByRole("navigation", { name: /agents/i })).toBeTruthy();
    expect(await screen.findByText(/disk on fire/i)).toBeTruthy();
  });
});

const Boom = (): never => {
  throw new Error("render exploded");
};

describe("failure surfacing", () => {
  it("shows the error instead of a blank window when a child throws", async () => {
    const { ErrorBoundary } = await import("./components/ErrorBoundary");
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

it("refreshes the roster when the event connection returns", async () => {
  const { onRuntimeEvent } = await import("./lib/ipc");
  render(<App />);
  await screen.findByRole("navigation", { name: /agents/i });
  const before = listAgents.mock.calls.length;
  const calls = vi.mocked(onRuntimeEvent).mock.calls;
  const reconnect = calls[calls.length - 1]?.[1];
  expect(reconnect).toBeTypeOf("function");
  reconnect?.();
  await waitFor(() => expect(listAgents.mock.calls.length).toBeGreaterThan(before));
});

describe("when the connection to the host is lost", () => {
  const gone = {
    kind: "unreachable",
    message:
      "lost the connection to this host at http://127.0.0.1:8787. It may be restarting, or the connection to it is down, and nothing on this page is live until it answers again",
  };

  async function connection() {
    const { onRuntimeEvent } = await import("./lib/ipc");
    const calls = vi.mocked(onRuntimeEvent).mock.calls;
    const [, reconnect, lost] = calls[calls.length - 1]!;
    expect(lost).toBeTypeOf("function");
    return { reconnect: reconnect!, lost: lost! };
  }

  it("says so before anything has been asked for, and marks the page as out of date", async () => {
    listAgents.mockResolvedValue([agent("Ada")]);
    const { container } = render(<App />);
    await screen.findByRole("navigation", { name: /agents/i });
    const { lost } = await connection();

    act(() => lost(gone));

    expect(screen.getByText(/Lost the connection to this host/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
    expect(container.querySelector(".app")?.getAttribute("data-link")).toBe("down");
    expect(screen.getByText(/last heard/)).toBeTruthy();
  });

  it("clears itself when the connection comes back", async () => {
    listAgents.mockResolvedValue([agent("Ada")]);
    const { container } = render(<App />);
    await screen.findByRole("navigation", { name: /agents/i });
    const { reconnect, lost } = await connection();

    act(() => lost(gone));
    expect(container.querySelector(".app")?.getAttribute("data-link")).toBe("down");

    await act(async () => reconnect());

    await waitFor(() => expect(screen.queryByText(/Lost the connection/)).toBeNull());
    expect(container.querySelector(".app")?.getAttribute("data-link")).toBeNull();
    expect(screen.queryByText(/last heard/)).toBeNull();
    expect(useStore.getState().link).toBe("up");
  });

  it("says nothing while an update is restarting the host on purpose", async () => {
    expectRestart();
    listAgents.mockResolvedValue([agent("Ada")]);
    const { container } = render(<App />);
    await screen.findByRole("navigation", { name: /agents/i });
    const { lost } = await connection();

    act(() => lost(gone));

    expect(screen.queryByText(/Lost the connection/)).toBeNull();
    expect(container.querySelector(".app")?.getAttribute("data-link")).toBeNull();
    expect(useStore.getState().link).toBe("up");
  });
});

describe("phone navigation", () => {
  it("opens the same channel again and preserves its draft across its tabs and Today", async () => {
    listAgents.mockResolvedValue([agent("Ada")]);
    const { container } = render(<App />);
    await waitFor(() => expect(railRow("Ada")).toBeTruthy());
    fireEvent.click(railRow("Ada"));
    await screen.findByRole("heading", { name: "Ada" });
    const page = () => screen.getByRole("navigation", { name: "Ada's page" });
    const pane = () => within(page()).queryByRole("button", { current: "page" })?.textContent;
    const input = container.querySelector(".composer__input") as HTMLTextAreaElement;
    fireEvent.change(input, { target: { value: "Keep this draft" } });
    fireEvent.click(within(page()).getByRole("button", { name: "Now" }));
    expect(pane()).toBe("Now");
    expect(screen.getByRole("complementary", { name: "Ada: details" })).toBeTruthy();
    fireEvent.click(within(page()).getByRole("button", { name: "Conversation" }));
    expect(pane()).toBe("Conversation");
    fireEvent.click(screen.getByRole("button", { name: "Today" }));
    expect(screen.queryByRole("navigation", { name: "Ada's page" })).toBeNull();
    fireEvent.click(railRow("Ada"));
    await screen.findByRole("heading", { name: "Ada" });
    expect(pane()).toBe("Conversation");
    expect((container.querySelector(".composer__input") as HTMLTextAreaElement).value).toBe(
      "Keep this draft",
    );
  });

  it("lets a phone choose a crew by name without hovering", async () => {
    const other = aGroup({ id: "crew-other", name: "Research" });
    listGroups.mockResolvedValue([aGroup(), other]);
    listAgents.mockResolvedValue([agent("Ada"), { ...agent("Lin"), groupId: other.id }]);
    render(<App />);
    await waitFor(() => expect(railRow("Ada")).toBeTruthy());
    fireEvent.change(within(groupsSheet()).getByRole("combobox", { name: "Group" }), {
      target: { value: other.id },
    });
    await waitFor(() => expect(useStore.getState().railGroup).toBe(other.id));
    expect(railNames()).toEqual(["Lin"]);
    expect(screen.getByRole("dialog", { name: "Groups and agents" }).isConnected).toBe(true);
  });
});

describe("where an agent's page lands", () => {
  const nudge: Routine = {
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
  };

  it("opens a fired routine from the conversation on the Routines tab", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    agentRoutines.mockResolvedValue([nudge]);
    render(<App />);
    await waitFor(() => expect(railRow("Manager")).toBeTruthy());
    fireEvent.click(railRow("Manager"));
    const page = await screen.findByRole("navigation", { name: "Manager's page" });
    expect(within(page).getByRole("button", { current: "page" }).textContent).toBe("Conversation");

    act(() => useStore.getState().openRoutine("r1"));

    expect(within(page).getByRole("button", { current: "page", name: "Routines" })).toBeTruthy();
    expect(await screen.findByRole("switch")).toBeTruthy();
    expect(useStore.getState().openingRoutine).toBeNull();
    fireEvent.click(within(page).getByRole("button", { name: "Memory" }));
    fireEvent.click(within(page).getByRole("button", { name: "Routines" }));
    expect(useStore.getState().openingRoutine).toBeNull();
  });

  it("stays on a found message after its mark clears, while the agent works", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    agentActivity.mockResolvedValue({ "id-Manager": { state: "thinking" } });
    render(<App />);
    await waitFor(() => expect(useStore.getState().activity["id-Manager"]?.state).toBe("thinking"));

    await act(() => useStore.getState().openMessage("id-Manager", "m1"));
    const page = await screen.findByRole("navigation", { name: "Manager's page" });
    expect(within(page).getByRole("button", { current: "page" }).textContent).toBe("Conversation");

    act(() => useStore.getState().clearFocus());

    expect(within(page).getByRole("button", { current: "page" }).textContent).toBe("Conversation");
  });

  it("still opens a working agent on Now", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    agentActivity.mockResolvedValue({ "id-Manager": { state: "thinking" } });
    render(<App />);
    await waitFor(() => expect(useStore.getState().activity["id-Manager"]?.state).toBe("thinking"));
    fireEvent.click(railRow("Manager"));
    const page = await screen.findByRole("navigation", { name: "Manager's page" });
    expect(within(page).getByRole("button", { current: "page" }).textContent).toBe("Now");
  });
});

describe("the command menu's shortcut", () => {
  it("lands in the field when the window opens on Today", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    const field = await screen.findByRole("textbox", { name: "What do you need?" });
    await waitFor(() => expect(document.activeElement).toBe(field));
  });

  it("does not open the menu from under the Groups sheet", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    await screen.findByRole("textbox", { name: "What do you need?" });
    groupsSheet();
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    await act(() => new Promise<void>((done) => requestAnimationFrame(() => done())));
    expect(screen.queryByRole("dialog", { name: "Command menu" })).toBeNull();
  });

  it("opens the menu with its input focused, and Escape gives the field back", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    const field = await screen.findByRole("textbox", { name: "What do you need?" });
    await waitFor(() => expect(document.activeElement).toBe(field));
    fireEvent.keyDown(window, { key: "k", metaKey: true });
    const menu = await screen.findByRole("dialog", { name: "Command menu" });
    expect(document.activeElement).toBe(
      within(menu).getByRole("textbox", { name: "Command menu" }),
    );
    fireEvent.keyDown(document.activeElement as Element, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Command menu" })).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(field));
  });

  it("takes a held answer back on Command Z, and leaves Command Z to a box with words in it", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    const field = await screen.findByRole("textbox", { name: "What do you need?" });
    const send = vi.fn(async () => {});
    act(() => useHeld.getState().hold("req-1", "Allowed Manager's request.", send));
    fireEvent.change(field, { target: { value: "half a sentence" } });
    fireEvent.keyDown(field, { key: "z", metaKey: true });
    expect(useHeld.getState().held?.id).toBe("req-1");
    fireEvent.change(field, { target: { value: "" } });
    fireEvent.click(await screen.findByRole("button", { name: /^Undo/ }));
    expect(useHeld.getState().held).toBeNull();
    act(() => useHeld.getState().hold("req-2", "Allowed Manager's request.", send));
    fireEvent.keyDown(document.body, { key: "z", metaKey: true });
    expect(useHeld.getState().held).toBeNull();
    expect(screen.queryByRole("status", { name: "" })).toBeNull();
    expect(send).not.toHaveBeenCalled();
  });

  it("opens the agent a number names, in the menu's order, and leaves the next number alone", async () => {
    listAgents.mockResolvedValue([agent("Manager", 1), agent("Writer", 0)]);
    render(<App />);
    await screen.findByRole("textbox", { name: "What do you need?" });
    fireEvent.keyDown(window, { key: "3", metaKey: true });
    expect(useStore.getState().selected).toBeNull();
    fireEvent.keyDown(window, { key: "1", metaKey: true });
    await waitFor(() => expect(useStore.getState().selected).toBe("id-Writer"));
    await waitFor(() =>
      expect(document.activeElement).toBe(
        screen.getByRole("combobox", { name: "Write to Writer" }),
      ),
    );
  });
});
