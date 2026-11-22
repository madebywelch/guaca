import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { rememberConversation } from "./lib/frame";
import { expectRestart, forgetRestart } from "./lib/link";
import { heldStart, holdStart, saveFirstTask } from "./lib/onboarding";
import type {
  AgentCard,
  Approval,
  Envelope,
  Routine,
  Running,
  Settings,
  TaskRecord,
} from "./lib/types";
import { aGroup, anApproval, anOutcome } from "./test-fixtures";

const pendingApprovals = vi.fn<() => Promise<Approval[]>>(async () => []);
const runningRuns = vi.fn<() => Promise<Running[]>>(async () => []);
const markOutcomesSeen = vi.fn(async (target: { runIds?: string[] }) => target.runIds ?? []);
const listGroups = vi.fn(async () => [aGroup({ agentCount: 3 })]);
const listAgents = vi.fn<() => Promise<AgentCard[]>>(async () => []);
const testConnection = vi.fn(async () => "Connected");
const agentLastActive = vi.fn<() => Promise<Record<string, number>>>(async () => ({}));
const agentActivity = vi.fn<() => Promise<Record<string, unknown>>>(async () => ({}));
const agentRoutines = vi.fn<() => Promise<Routine[]>>(async () => []);
const setAgentPinned = vi.fn<(id: string, pinned: boolean) => Promise<AgentCard>>();
const duplicateAgent = vi.fn<(id: string) => Promise<AgentCard>>();
const taskRecord = vi.fn<(channel: string, run: string) => Promise<TaskRecord>>(async () => ({
  messages: [],
  resultId: null,
}));
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
    testConnection: () => testConnection(),
    updateSettings: async () => ({ ...(await getSettings()), apiKeySet: true }),
    agentActivity: () => agentActivity(),
    usageSummary: async () => [],
    approvalStates: async () => ({}),
    approvalAnswers: async () => ({}),
    pendingApprovals: () => pendingApprovals(),
    runningRuns: () => runningRuns(),
    markOutcomesSeen: (target: { runIds?: string[] }) => markOutcomesSeen(target),
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
    subscriptionStatus: async () => ({
      signedIn: false,
      email: "",
      plan: "",
      includesCodex: false,
    }),
    claudeSigninStatus: async () => ({ signedIn: false }),
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
    taskRecord: (channel: string, run: string) => taskRecord(channel, run),
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
  localStorage.clear();
  pendingApprovals.mockResolvedValue([]);
  runningRuns.mockResolvedValue([]);
  listAgents.mockResolvedValue([]);
  listGroups.mockResolvedValue([aGroup({ agentCount: 3 })]);
  agentLastActive.mockResolvedValue({});
  agentActivity.mockResolvedValue({});
  useStore.setState({
    runningKnown: false,
    runningUnavailable: false,
    runningReadFailed: false,
    agents: [],
    activity: {},
    lastActive: {},
    settings: null,
    selected: null,
    railGroup: null,
    pending: [],
    stuck: [],
    decisions: [],
    outcomes: [],
    running: [],
    forYou: false,
    messages: {},
    streams: {},
    pulses: [],
    banner: null,
    artifactsOpen: null,
    settingsPage: null,
    settingsDraft: null,
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
  it("renders the roster and reaches every agent through the Groups sheet", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    expect(await screen.findByRole("navigation", { name: "Agent conversations" })).toBeTruthy();
    expect(within(groupsSheet()).getByText("guaca")).toBeTruthy();
  });

  it("numbers up to nine without hiding a waiting agent in another group", async () => {
    const second = aGroup({ id: "second", name: "Research" });
    listGroups.mockResolvedValue([aGroup(), second]);
    listAgents.mockResolvedValue([
      ...Array.from({ length: 9 }, (_, index) => agent(`Agent ${index + 1}`, index)),
      { ...agent("Market Researcher"), groupId: second.id },
    ]);
    render(<App />);
    await screen.findByRole("combobox", { name: "Write to Agent 1" });
    const roster = screen.getByRole("navigation", { name: "Agent conversations" });
    const first = within(roster).getByRole("button", { name: "Agent 1, Idle" });
    expect(first.getAttribute("aria-keyshortcuts")).toBe("Meta+1 Control+1");
    act(() => useStore.setState({ pending: [anApproval({ agentId: "id-Market Researcher" })] }));
    const asking = within(roster).getByRole("button", {
      name: "Market Researcher, Has a question",
    });
    expect(asking.hasAttribute("aria-keyshortcuts")).toBe(false);
    expect(first.getAttribute("aria-current")).toBe("page");
    fireEvent.click(within(roster).getByRole("button", { name: "1 question waiting" }));
    expect(await screen.findByRole("dialog", { name: "Questions" })).toBeTruthy();
    expect(document.activeElement).toBe(
      screen.getByRole("article", { name: /^Market Researcher asks/ }),
    );
    expect(useStore.getState().selected).toBe("id-Agent 1");
  });

  it("marks an unread result seen only when its conversation opens", async () => {
    listAgents.mockResolvedValue([agent("Manager", 0), agent("Writer", 1)]);
    render(<App />);
    await screen.findByRole("combobox", { name: "Write to Manager" });
    act(() =>
      useStore.setState({ outcomes: [anOutcome({ agentId: "id-Writer", runId: "written" })] }),
    );
    const unread = screen.getByRole("button", { name: "Writer, Has a result" });
    expect(markOutcomesSeen).not.toHaveBeenCalled();
    fireEvent.click(unread);
    await screen.findByRole("button", { name: "Writer, Idle" });
    expect(markOutcomesSeen).toHaveBeenCalledExactlyOnceWith({ runIds: ["written"] });
  });

  it("shows neither the roster, the places nor the window's buttons while the first run is on screen", async () => {
    render(<App />);
    await screen.findByRole("heading", { name: "What would you like help with?" });
    expect(screen.queryByRole("navigation", { name: "Agent conversations" })).toBeNull();
    expect(screen.queryByRole("navigation", { name: "Places" })).toBeNull();
    expect(screen.queryByRole("navigation", { name: "Window" })).toBeNull();
    expect(screen.queryByRole("button", { name: "App settings" })).toBeNull();
  });

  it("gives the window no row of its own beside the sidebar, and keeps the date at the sidebar's foot", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    const { container } = render(<App />);
    await screen.findByRole("combobox", { name: "Write to Manager" });
    const roster = screen.getByRole("navigation", { name: "Agent conversations" });
    const foot = container.querySelector(".roster__foot") as HTMLElement;
    const day = new Date().toLocaleDateString([], {
      weekday: "long",
      month: "long",
      day: "numeric",
    });
    expect(within(foot).getByText(day)).toBeTruthy();
    expect(within(foot).getByRole("navigation", { name: "Places" })).toBeTruthy();
    expect(screen.queryByText("A kept page can be pinned here, from Artifacts.")).toBeNull();
    expect(roster.compareDocumentPosition(foot) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    const bar = container.querySelector(".topbar") as HTMLElement;
    expect(bar.textContent).not.toContain(day);
    expect(
      within(bar)
        .getAllByRole("button")
        .map((one) => one.getAttribute("aria-label")),
    ).toEqual(["Search", "Agents", "Make something new", "App settings"]);
    const header = container.querySelector(".pane__header") as HTMLElement;
    expect(within(header).getByRole("heading", { name: "Manager" })).toBeTruthy();
    expect(within(header).getByRole("navigation", { name: "Manager's page" })).toBeTruthy();
    expect(header.hasAttribute("data-tauri-drag-region")).toBe(true);
  });

  it("names the top row's settings button App settings, apart from each group's own", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    const row = await screen.findByRole("navigation", { name: "Window" });
    fireEvent.click(within(row).getByRole("button", { name: "App settings" }));
    expect(await screen.findByRole("region", { name: "Settings" })).toBeTruthy();
  });

  it("shows the window's notices inside Settings, after its section list", async () => {
    getSettings.mockResolvedValue({
      ...(await getSettings()),
      provider: "compatible",
      apiKeySet: false,
    });
    listGroups.mockResolvedValue([aGroup({ apiKeySet: false })]);
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    await screen.findByText(/Add an API key/i);
    act(() =>
      useStore.setState({
        handoff: "https://example.test/sign-in",
        banner: { tone: "error", text: "The host stopped answering." },
      }),
    );
    const row = await screen.findByRole("navigation", { name: "Window" });
    fireEvent.click(within(row).getByRole("button", { name: "App settings" }));
    const page = await screen.findByRole("region", { name: "Settings" });
    const list = within(page).getByRole("tablist", { name: "Settings sections" });
    for (const text of [
      /Add an API key/i,
      /A sign-in is waiting/i,
      /The host stopped answering/i,
    ]) {
      const notice = screen.getByText(text);
      expect(page.contains(notice)).toBe(true);
      expect(list.compareDocumentPosition(notice) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    }
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

  it("leaves an open sheet alone on Command-comma", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /open the cafeteria/i }));
    const cafeteria = await screen.findByRole("dialog", { name: /cafeteria/i });
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    expect(screen.queryByRole("region", { name: "Settings" })).toBeNull();
    expect(useStore.getState().settingsPage).toBeNull();
    expect(cafeteria.isConnected).toBe(true);
  });

  it("opens Settings as a page in place of onboarding", async () => {
    render(<App />);
    const heading = await screen.findByRole("heading", { name: "What would you like help with?" });
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    const page = await screen.findByRole("region", { name: "Settings" });
    expect(page.closest("main")).toBeTruthy();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(heading.isConnected).toBe(true);
    expect(screen.queryByRole("heading", { name: "What would you like help with?" })).toBeNull();
    expect(screen.getByRole("button", { name: "Workspace" })).toBeTruthy();
  });

  it("goes back on Command-comma, Escape and the back control, restoring the focus", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    await screen.findByRole("log", { name: "Conversation with Manager" });
    const gear = within(screen.getByRole("navigation", { name: "Window" })).getByRole("button", {
      name: "App settings",
    });
    for (const leave of ["comma", "escape", "back"] as const) {
      gear.focus();
      fireEvent.click(gear);
      const page = await screen.findByRole("region", { name: "Settings" });
      await waitFor(() => expect(document.activeElement).toBe(page));
      if (leave === "comma") fireEvent.keyDown(window, { key: ",", ctrlKey: true });
      if (leave === "escape") fireEvent.keyDown(window, { key: "Escape" });
      if (leave === "back") fireEvent.click(screen.getByRole("button", { name: "Manager" }));
      expect(screen.queryByRole("region", { name: "Settings" })).toBeNull();
      expect(screen.getByRole("log", { name: "Conversation with Manager" })).toBeTruthy();
      await waitFor(() => expect(document.activeElement).toBe(gear));
    }
  });

  it("goes back to the agent page it was opened from, as that page was left", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    await waitFor(() => expect(useStore.getState().agents).toHaveLength(1));
    await act(async () => useStore.getState().select(useStore.getState().agents[0]!.id));
    const agentPage = await screen.findByRole("navigation", { name: "Manager's page" });
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    await screen.findByRole("region", { name: "Settings" });
    expect(screen.queryByRole("navigation", { name: "Manager's page" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Manager" }));
    expect(screen.queryByRole("region", { name: "Settings" })).toBeNull();
    expect(screen.getByRole("navigation", { name: "Manager's page" })).toBe(agentPage);
  });

  it("leaves the page for an agent opened from anywhere while it is shown", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    await waitFor(() => expect(useStore.getState().agents).toHaveLength(1));
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    await screen.findByRole("region", { name: "Settings" });
    await act(async () => useStore.getState().select(useStore.getState().agents[0]!.id));
    expect(screen.queryByRole("region", { name: "Settings" })).toBeNull();
    expect(await screen.findByRole("navigation", { name: "Manager's page" })).toBeTruthy();
  });

  it.each(["openMessage", "select"] as const)(
    "leaves the page on %s, keeping the draft and not pulling the focus back to the gear",
    async (how) => {
      listAgents.mockResolvedValue([agent("Manager")]);
      render(<App />);
      await waitFor(() => expect(useStore.getState().agents).toHaveLength(1));
      const id = useStore.getState().agents[0]!.id;
      const gear = within(screen.getByRole("navigation", { name: "Window" })).getByRole("button", {
        name: "App settings",
      });
      gear.focus();
      fireEvent.click(gear);
      await screen.findByRole("region", { name: "Settings" });
      fireEvent.change(screen.getByLabelText(/^Your name/), { target: { value: "Draft name" } });
      const elsewhere = within(screen.getByRole("navigation", { name: "Window" })).getByRole(
        "button",
        { name: "Search" },
      );
      elsewhere.focus();
      await act(async () => {
        if (how === "openMessage") await useStore.getState().openMessage(id, "m1");
        else await useStore.getState().select(id);
      });
      expect(screen.queryByRole("region", { name: "Settings" })).toBeNull();
      expect(screen.getByRole("navigation", { name: "Manager's page" })).toBeTruthy();
      await act(async () => {});
      expect(document.activeElement).not.toBe(gear);
      fireEvent.click(gear);
      await screen.findByRole("region", { name: "Settings" });
      expect((screen.getByLabelText(/^Your name/) as HTMLInputElement).value).toBe("Draft name");
    },
  );

  it("leaves the focus where the person put it when they open the agent the page was over", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    await waitFor(() => expect(useStore.getState().agents).toHaveLength(1));
    const id = useStore.getState().agents[0]!.id;
    await act(async () => useStore.getState().select(id));
    const bar = screen.getByRole("navigation", { name: "Window" });
    const gear = within(bar).getByRole("button", { name: "App settings" });
    gear.focus();
    fireEvent.click(gear);
    await screen.findByRole("region", { name: "Settings" });
    const elsewhere = within(bar).getByRole("button", { name: "Search" });
    elsewhere.focus();
    await act(async () => useStore.getState().select(id));
    expect(screen.queryByRole("region", { name: "Settings" })).toBeNull();
    await act(async () => {});
    expect(document.activeElement).toBe(elsewhere);
  });

  it("turns to the section the command menu names while the page is open", async () => {
    render(<App />);
    await screen.findByRole("heading", { name: "What would you like help with?" });
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    await screen.findByRole("region", { name: "Settings" });
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    const menu = await screen.findByRole("dialog", { name: "Command menu" });
    const input = within(menu).getByRole("textbox", { name: "Command menu" });
    fireEvent.change(input, { target: { value: "Settings: Appearance" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Appearance" }).getAttribute("aria-selected")).toBe(
        "true",
      ),
    );
    expect(screen.queryByRole("dialog", { name: "Command menu" })).toBeNull();
  });

  it("puts a sheet opened from the page above it, and Escape closes the sheet first", async () => {
    render(<App />);
    await screen.findByRole("heading", { name: "What would you like help with?" });
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    const page = await screen.findByRole("region", { name: "Settings" });
    const tab = screen.getByRole("tab", { name: "General" });
    tab.focus();
    fireEvent.keyDown(window, { key: "f", ctrlKey: true });
    const search = await screen.findByRole("dialog", { name: /search/i });
    fireEvent.keyDown(window, { key: "Escape" });
    expect(search.isConnected).toBe(false);
    expect(page.isConnected).toBe(true);
    expect(document.activeElement).toBe(tab);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(page.isConnected).toBe(false);
  });

  it.each(["Calendar", "Artifacts"])("keeps %s open under search", async (name) => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    await screen.findByRole("navigation", { name: "Places" });
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

  it("opens the command menu from the Groups sheet and returns to the sheet on Escape", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Agents" }));
    const groups = await screen.findByRole("dialog", { name: "Groups and agents" });
    const opener = within(groups).getByRole("button", { name: /command menu/i });
    fireEvent.click(opener);
    const input = await screen.findByRole("textbox", { name: "Command menu" });
    expect(document.activeElement).toBe(input);
    fireEvent.keyDown(input, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "Command menu" })).toBeNull();
    expect(document.activeElement).toBe(opener);
  });

  it.each([["Search", "f"]])(
    "keeps For you open under %s and restores focus",
    async (upperName, key) => {
      listAgents.mockResolvedValue([agent("Manager")]);
      render(<App />);
      await screen.findByRole("navigation", { name: "Places" });
      const opener = within(screen.getByRole("navigation", { name: "Places" })).getByRole(
        "button",
        {
          name: /^For you/,
        },
      );
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
    },
  );

  it.each([["Search", "f"]])(
    "keeps the calendar editor under %s and unwinds each layer",
    async (upperName, key) => {
      listAgents.mockResolvedValue([agent("Manager")]);
      render(<App />);
      await screen.findByRole("navigation", { name: "Places" });
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
    },
  );

  it.each([["Search", "f"]])(
    "keeps an agent submenu under %s and unwinds each layer",
    async (upperName, key) => {
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
    },
  );

  it.each(["New agent", "New group"])(
    "restores the menu opener after %s replaces the menu",
    async (name) => {
      listAgents.mockResolvedValue([agent("Manager")]);
      render(<App />);
      await screen.findByRole("log", { name: "Conversation with Manager" });
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
    ["New agent", "Search", "f"],
    ["New group", "Search", "f"],
  ])("keeps %s open under %s", async (name, upperName, key) => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    await screen.findByRole("log", { name: "Conversation with Manager" });
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

  it("opens the last conversation when there is no question or running work", async () => {
    listAgents.mockResolvedValue([agent("Manager"), agent("Chef")]);
    rememberConversation("id-Chef");
    render(<App />);
    const input = await screen.findByRole("combobox", { name: "Write to Chef" });
    await waitFor(() => expect(document.activeElement).toBe(input));
    expect(useStore.getState().selected).toBe("id-Chef");
  });

  it("chooses another conversation when the open agent is deleted", async () => {
    listAgents.mockResolvedValue([agent("Manager"), agent("Chef")]);
    rememberConversation("id-Manager");
    render(<App />);
    await screen.findByRole("navigation", { name: "Manager's page" });
    listAgents.mockResolvedValue([agent("Chef")]);
    await act(() => useStore.getState().refreshAgents());
    expect(screen.queryByRole("navigation", { name: "Manager's page" })).toBeNull();
    expect(await screen.findByRole("navigation", { name: "Chef's page" })).toBeTruthy();
  });

  it("opens the task owner and focuses a delegated question that is due first", async () => {
    listAgents.mockResolvedValue([agent("Manager", 0), agent("Chef", 1), agent("Writer", 2)]);
    rememberConversation("id-Writer");
    pendingApprovals.mockResolvedValue([
      anApproval({
        id: "later",
        agentId: "id-Writer",
        runId: "independent",
        lapsesAt: Date.now() + 60_000,
      }),
      anApproval({
        id: "first",
        agentId: "id-Chef",
        runId: "delegated",
        lapsesAt: Date.now() + 30_000,
      }),
    ]);
    runningRuns.mockResolvedValue([
      {
        runId: "delegated",
        agentId: "id-Manager",
        groupId: agent("Manager").groupId,
        startedAt: 1,
        note: null,
      },
    ]);
    render(<App />);
    const tray = await screen.findByRole("dialog", { name: "Questions" });
    expect(useStore.getState().selected).toBe("id-Manager");
    expect(document.activeElement).toBe(within(tray).getByRole("article", { name: /^Chef asks/ }));
    fireEvent.keyDown(document.activeElement as Element, { key: "Escape" });
    const input = await screen.findByRole("combobox", { name: "Write to Manager" });
    await waitFor(() => expect(document.activeElement).toBe(input));
  });

  const askOnTheMap = () => {
    listAgents.mockResolvedValue([agent("Manager", 0), agent("Chef", 1)]);
    pendingApprovals.mockResolvedValue([
      anApproval({
        id: "budget",
        agentId: "id-Chef",
        runId: "delegated",
        request: { kind: "question", options: ["Under 12,000", "Up to 15,000"] },
        summary: "Keep the venue under 12,000 euros?",
        lapsesAt: Date.now() + 30_000,
      }),
    ]);
    runningRuns.mockResolvedValue([
      {
        runId: "delegated",
        agentId: "id-Manager",
        groupId: agent("Manager").groupId,
        startedAt: 1,
        note: null,
      },
    ]);
    const envelope = (over: Partial<Envelope>): Envelope => ({
      id: "brief",
      runId: "delegated",
      channelId: "id-Manager",
      from: { kind: "human" },
      to: { kind: "agent", id: "id-Manager" },
      parts: [{ type: "text", text: "Plan the team offsite in Lisbon." }],
      trust: "operator",
      hop: 0,
      intent: "work",
      expectsReply: true,
      cause: null,
      createdAt: 1,
      ...over,
    });
    return {
      messages: [
        envelope({}),
        envelope({
          id: "asked",
          from: { kind: "agent", id: "id-Manager" },
          to: { kind: "agent", id: "id-Chef" },
          createdAt: 2,
        }),
      ],
      resultId: null,
    };
  };

  it("opens the task owner on a colleague's question drawn on its map, focuses it there and opens no tray", async () => {
    taskRecord.mockResolvedValue(askOnTheMap());
    const { container } = render(<App />);
    const asked = await waitFor(() => {
      const drawn = container.querySelector<HTMLElement>(
        '.task-map__question[data-question="budget"]',
      );
      expect(drawn).toBeTruthy();
      return drawn as HTMLElement;
    });
    await waitFor(() => expect(document.activeElement).toBe(asked));
    expect(useStore.getState().selected).toBe("id-Manager");
    expect(screen.queryByRole("dialog", { name: "Questions" })).toBeNull();
    const input = screen.getByRole("combobox", { name: "Write to Manager" });
    input.focus();
    fireEvent.keyDown(input, { key: "Escape" });
    expect(document.activeElement).toBe(asked);
    expect(screen.queryByRole("dialog", { name: "Questions" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "1 question waiting" }));
    expect(await screen.findByRole("dialog", { name: "Questions" })).toBeTruthy();
  });

  it("leaves focus where the person put it when the map draws the question after they acted", async () => {
    const record = askOnTheMap();
    let draw: (value: typeof record) => void = () => {};
    taskRecord.mockReturnValue(
      new Promise((resolve) => {
        draw = resolve;
      }),
    );
    const { container } = render(<App />);
    const input = await screen.findByRole("combobox", { name: "Write to Manager" });
    await waitFor(() => expect(document.activeElement).toBe(input));
    fireEvent.pointerDown(screen.getByRole("log", { name: "Conversation with Manager" }));
    input.blur();
    await act(async () => draw(record));
    await waitFor(() =>
      expect(container.querySelector('.task-map__question[data-question="budget"]')).toBeTruthy(),
    );
    expect(document.activeElement).toBe(document.body);
    expect(screen.queryByRole("dialog", { name: "Questions" })).toBeNull();
  });

  it.each(["automatic", "manual"])(
    "uses a successful %s running retry to choose the conversation",
    async (retry) => {
      listAgents.mockResolvedValue([agent("Manager"), agent("Chef")]);
      runningRuns.mockRejectedValue(new Error("temporarily unavailable"));
      rememberConversation("id-Manager");
      render(<App />);
      const retryButton = await screen.findByRole("button", {
        name: "Retry checking running work",
      });
      expect(screen.getByRole("navigation", { name: "Agent conversations" })).toBeTruthy();
      expect(useStore.getState().selected).toBeNull();
      runningRuns.mockResolvedValue([
        {
          runId: "working",
          agentId: "id-Chef",
          groupId: agent("Chef").groupId,
          startedAt: 1,
          note: null,
        },
      ]);
      if (retry === "manual") fireEvent.click(retryButton);
      expect(
        await screen.findByRole("log", { name: "Conversation with Chef" }, { timeout: 2_000 }),
      ).toBeTruthy();
    },
  );

  it("opens the remembered conversation on a host without a running snapshot command", async () => {
    listAgents.mockResolvedValue([agent("Manager"), agent("Chef")]);
    runningRuns.mockRejectedValue({ kind: "unknownCommand" });
    rememberConversation("id-Chef");
    render(<App />);
    expect(await screen.findByRole("log", { name: "Conversation with Chef" })).toBeTruthy();
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

    it("opens another agent selected after a reveal on its conversation too", async () => {
      const reveal = await workingWindow();

      await act(async () => {
        reveal({ kind: "agent", id: "id-Manager" });
        await useStore.getState().select("id-Chef");
      });

      expect(currentTab("Chef")).toBe("Conversation");
      fireEvent.click(railRow("Manager"));
      expect(currentTab("Manager")).toBe("Conversation");
    });

    it("keeps the conversation and tab open when a new question arrives elsewhere", async () => {
      await workingWindow();
      fireEvent.click(railRow("Chef"));
      const page = screen.getByRole("navigation", { name: "Chef's page" });
      fireEvent.click(within(page).getByRole("button", { name: "Memory" }));
      act(() => useStore.setState({ pending: [anApproval({ agentId: "id-Manager" })] }));
      expect(currentTab("Chef")).toBe("Memory");
      expect(screen.queryByRole("dialog", { name: "Questions" })).toBeNull();
      expect(screen.getByRole("button", { name: "1 question waiting" })).toBeTruthy();
    });

    it("keeps Now a tab the person chooses, and opens the conversation again after another agent", async () => {
      await workingWindow();
      fireEvent.click(railRow("Manager"));
      expect(currentTab("Manager")).toBe("Conversation");

      const page = screen.getByRole("navigation", { name: "Manager's page" });
      fireEvent.click(within(page).getByRole("button", { name: "Now" }));
      expect(currentTab("Manager")).toBe("Now");
      fireEvent.click(railRow("Chef"));
      expect(screen.queryByRole("navigation", { name: "Manager's page" })).toBeNull();

      fireEvent.click(railRow("Manager"));
      expect(currentTab("Manager")).toBe("Conversation");
    });
  });

  it("lists agents and opens a channel", async () => {
    listAgents.mockResolvedValue([agent("Manager"), agent("Chef")]);
    render(<App />);

    await waitFor(() => expect(screen.getByRole("button", { name: /^Manager, / })).toBeTruthy());
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

    await waitFor(() => expect(screen.getByRole("button", { name: /^Manager, / })).toBeTruthy());
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
    await waitFor(() => expect(screen.getByRole("button", { name: /^Manager, / })).toBeTruthy());
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

  it("keeps a task queued before the host was up on screen, even on a host that already has an agent", async () => {
    holdStart({
      task: "plan",
      brief: "Plan our launch",
      step: "start",
      provider: null,
      chosen: false,
      queued: true,
      focus: null,
    });
    listAgents.mockResolvedValue([agent("Product Manager")]);
    try {
      render(<App />);
      expect(await screen.findByRole("heading", { name: "Ready when you are." })).toBeTruthy();
    } finally {
      holdStart(null);
      saveFirstTask(null);
    }
  });

  it("drops a key typed while the host started when that host already has a workspace and nothing was queued", async () => {
    const said = [
      vi.spyOn(console, "log"),
      vi.spyOn(console, "info"),
      vi.spyOn(console, "warn"),
      vi.spyOn(console, "error"),
    ];
    const setItem = vi.spyOn(Storage.prototype, "setItem");
    holdStart({
      task: "plan",
      brief: "Plan our launch",
      step: "start",
      provider: {
        provider: "compatible",
        baseUrl: "https://openrouter.ai/api/v1",
        model: "anthropic/claude-sonnet-4.5",
        apiKey: "sk-typed-early",
        subscriptionModel: "",
        effort: "auto",
      },
      chosen: true,
      queued: false,
      focus: null,
    });
    listAgents.mockResolvedValue([agent("Product Manager")]);
    try {
      render(<App />);
      const notice = await screen.findByText(/the API key you typed while it started was not/);
      expect(notice.textContent).toContain("Guaca did not save it anywhere");
      expect(heldStart()).toBeNull();
      expect(screen.queryByRole("heading", { name: "Ready when you are." })).toBeNull();
      expect(testConnection).not.toHaveBeenCalled();
      for (const [, value] of setItem.mock.calls) expect(value).not.toContain("sk-typed-early");
      for (const spy of said)
        for (const call of spy.mock.calls) expect(JSON.stringify(call)).not.toContain("sk-typed");
      fireEvent.click(within(notice.closest(".banner")!).getByRole("button", { name: "Dismiss" }));
      expect(screen.queryByText(/the API key you typed while it started/)).toBeNull();
    } finally {
      for (const spy of said) spy.mockRestore();
      setItem.mockRestore();
      holdStart(null);
    }
  });

  it("forgets what was held without a word when no key was typed and the host already has a workspace", async () => {
    holdStart({
      task: "plan",
      brief: "Plan our launch",
      step: "task",
      provider: null,
      chosen: false,
      queued: false,
      focus: null,
    });
    listAgents.mockResolvedValue([agent("Product Manager")]);
    try {
      render(<App />);
      await waitFor(() => expect(heldStart()).toBeNull());
      expect(screen.queryByText(/the API key you typed/)).toBeNull();
    } finally {
      holdStart(null);
    }
  });

  it("still tests and saves a key typed early on a new host's first run", async () => {
    holdStart({
      task: "plan",
      brief: "Plan our launch",
      step: "start",
      provider: {
        provider: "compatible",
        baseUrl: "https://openrouter.ai/api/v1",
        model: "anthropic/claude-sonnet-4.5",
        apiKey: "sk-typed-early",
        subscriptionModel: "",
        effort: "auto",
      },
      chosen: true,
      queued: false,
      focus: null,
    });
    listGroups.mockResolvedValue([aGroup({ apiKeySet: false })]);
    try {
      render(<App />);
      await waitFor(() => expect(testConnection).toHaveBeenCalledOnce());
      expect(heldStart()).toBeNull();
      expect(screen.queryByText(/the API key you typed/)).toBeNull();
    } finally {
      holdStart(null);
      saveFirstTask(null);
    }
  });

  it.each([
    { paid: true, banner: false },
    { paid: false, banner: true },
  ])("asks for a key if the initial agent cannot reply: %j", async ({ paid, banner }) => {
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
    expect(useStore.getState().selected).toBe("id-Manager");
    expect(screen.queryByText(/Add an API key/i) !== null).toBe(banner);
  });

  it("names the Provider section on the no-key banner once Settings is open", async () => {
    getSettings.mockResolvedValue({
      ...(await getSettings()),
      provider: "compatible",
      apiKeySet: false,
    });
    const first = aGroup({ apiKeySet: false });
    listGroups.mockResolvedValue([first]);
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Open settings" }));
    expect(await screen.findByRole("button", { name: "Go to Provider" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open settings" })).toBeNull();
    act(() => useStore.getState().closeSettings());
  });

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
    await waitFor(() => expect(screen.getByRole("button", { name: /^Manager, / })).toBeTruthy());
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
    await waitFor(() => expect(screen.getByRole("button", { name: /^Manager, / })).toBeTruthy());
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

    await screen.findByRole("button", { name: "Chef, Working" });
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

  it("still offers workspace controls when startup fails", async () => {
    listAgents.mockRejectedValue({ kind: "storage", message: "disk on fire" });
    render(<App />);
    expect(
      within(screen.getByRole("navigation", { name: "Window" })).getByRole("button", {
        name: "Agents",
      }),
    ).toBeTruthy();
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
  listAgents.mockResolvedValue([agent("Manager")]);
  render(<App />);
  await screen.findByRole("navigation", { name: "Agent conversations" });
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
    await screen.findByRole("navigation", { name: "Agent conversations" });
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
    await screen.findByRole("navigation", { name: "Agent conversations" });
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
    await screen.findByRole("navigation", { name: "Agent conversations" });
    const { lost } = await connection();

    act(() => lost(gone));

    expect(screen.queryByText(/Lost the connection/)).toBeNull();
    expect(container.querySelector(".app")?.getAttribute("data-link")).toBeNull();
    expect(useStore.getState().link).toBe("up");
  });
});

describe("phone navigation", () => {
  it("opens the same channel again and preserves its draft across its tabs and another conversation", async () => {
    listAgents.mockResolvedValue([agent("Ada"), agent("Lin")]);
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
    fireEvent.click(railRow("Lin"));
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
    await screen.findByRole("combobox", { name: "Write to Ada" });
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

  it.each([
    ["the roster", () => fireEvent.click(screen.getByRole("button", { name: /^Manager, / }))],
    ["the Groups sheet", () => fireEvent.click(railRow("Manager"))],
    ["Command 1", () => fireEvent.keyDown(window, { key: "1", metaKey: true })],
    [
      "a select from the command menu, search or Start task",
      () => act(() => useStore.getState().select("id-Manager")),
    ],
  ])("opens a working agent on its conversation from %s", async (_route, open) => {
    listAgents.mockResolvedValue([agent("Manager")]);
    agentActivity.mockResolvedValue({ "id-Manager": { state: "thinking" } });
    render(<App />);
    await waitFor(() => expect(useStore.getState().activity["id-Manager"]?.state).toBe("thinking"));
    await waitFor(() => expect(screen.getByRole("button", { name: /^Manager, / })).toBeTruthy());
    useStore.setState({
      running: [
        {
          runId: "run-1",
          agentId: "id-Manager",
          groupId: "00000000-0000-4000-8000-000000000001",
          startedAt: Date.now(),
          note: "Reading the brief",
        },
      ],
      activeRun: { "id-Manager": "run-1" },
    });

    await open();

    const page = await screen.findByRole("navigation", { name: "Manager's page" });
    expect(within(page).getByRole("button", { current: "page" }).textContent).toBe("Conversation");
    expect(screen.getByRole("log", { name: "Conversation with Manager" })).toBeTruthy();
    const line = screen
      .getAllByRole("status")
      .find((one) => one.textContent?.includes("Reading the brief"));
    expect(line).toBeTruthy();
    expect(line?.closest("[hidden]")).toBeNull();
  });
});

describe("the command menu's shortcut", () => {
  it("only steps through agents when the command menu is the top layer", async () => {
    listAgents.mockResolvedValue([agent("Manager", 0), agent("Writer", 1)]);
    render(<App />);
    await screen.findByRole("combobox", { name: "Write to Manager" });
    fireEvent.keyDown(window, { key: "1", ctrlKey: true });
    await screen.findByRole("combobox", { name: "Write to Manager" });
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    await screen.findByRole("dialog", { name: "Command menu" });
    fireEvent.keyDown(window, { key: "f", ctrlKey: true });
    const search = await screen.findByRole("dialog", { name: "Search" });
    fireEvent.keyDown(search, { key: "Tab", ctrlKey: true });
    expect(useStore.getState().selected).toBe("id-Manager");
    expect(screen.getByRole("dialog", { name: "Command menu" })).toBeTruthy();
    fireEvent.keyDown(search, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Search" })).toBeNull());
    fireEvent.keyDown(window, { key: "Tab", ctrlKey: true });
    await screen.findByRole("combobox", { name: "Write to Writer" });
    expect(screen.queryByRole("dialog", { name: "Command menu" })).toBeNull();
  });

  it("lands in the composer when the window opens", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    const field = await screen.findByRole("combobox", { name: "Write to Manager" });
    await waitFor(() => expect(document.activeElement).toBe(field));
  });

  it("does not open the menu from under the Groups sheet", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    await screen.findByRole("combobox", { name: "Write to Manager" });
    groupsSheet();
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    await act(() => new Promise<void>((done) => requestAnimationFrame(() => done())));
    expect(screen.queryByRole("dialog", { name: "Command menu" })).toBeNull();
  });

  it("opens the menu with its input focused, and Escape gives the composer back", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    const field = await screen.findByRole("combobox", { name: "Write to Manager" });
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
    const field = await screen.findByRole("combobox", { name: "Write to Manager" });
    const send = vi.fn(async () => {});
    act(() => useHeld.getState().hold("req-1", "Allowed what Manager asked.", send));
    fireEvent.change(field, { target: { value: "half a sentence" } });
    fireEvent.keyDown(field, { key: "z", metaKey: true });
    expect(useHeld.getState().held?.id).toBe("req-1");
    fireEvent.change(field, { target: { value: "" } });
    fireEvent.click(await screen.findByRole("button", { name: /^Undo/ }));
    expect(useHeld.getState().held).toBeNull();
    act(() => useHeld.getState().hold("req-2", "Allowed what Manager asked.", send));
    fireEvent.keyDown(document.body, { key: "z", metaKey: true });
    expect(useHeld.getState().held).toBeNull();
    expect(screen.queryByRole("button", { name: /^Undo/ })).toBeNull();
    expect(send).not.toHaveBeenCalled();
  });

  it("treats a held answer as answered on every surface, and Undo brings the question back everywhere at once", async () => {
    listAgents.mockResolvedValue([agent("Manager")]);
    render(<App />);
    await screen.findByRole("combobox", { name: "Write to Manager" });
    const asking = anApproval({ agentId: "id-Manager", createdAt: Date.now() - 60_000 });
    act(() =>
      useStore.setState({
        pending: [asking],
        activity: { "id-Manager": { state: "awaitingApproval" } },
        running: [
          {
            runId: asking.runId,
            agentId: "id-Manager",
            groupId: asking.groupId,
            startedAt: Date.now() - 60_000,
            note: null,
          },
        ],
      }),
    );
    const places = screen.getByRole("navigation", { name: "Places" });
    const waiting = () => {
      expect(within(places).getByRole("button", { name: "For you, 1 question" })).toBeTruthy();
      expect(screen.getByRole("button", { name: "Manager, Has a question" })).toBeTruthy();
      expect(screen.getByRole("button", { name: "1 question waiting" })).toBeTruthy();
    };
    waiting();

    const send = vi.fn(async () => {});
    act(() => useHeld.getState().hold(asking.id, "Allowed what Manager asked.", send));
    expect(within(places).getByRole("button", { name: "For you" }).textContent).toBe("For you");
    expect(screen.queryByRole("button", { name: "Manager, Has a question" })).toBeNull();
    expect(screen.getByRole("button", { name: "Manager, Working" })).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "0 questions waiting" }).hasAttribute("disabled"),
    ).toBe(true);

    fireEvent.click(await screen.findByRole("button", { name: /^Undo/ }));
    waiting();
    expect(send).not.toHaveBeenCalled();
    act(() => useStore.setState({ pending: [], activity: {}, running: [] }));
  });

  it("opens the agent a number names, in the menu's order, and leaves the next number alone", async () => {
    listAgents.mockResolvedValue([agent("Manager", 1), agent("Writer", 0)]);
    render(<App />);
    await screen.findByRole("combobox", { name: "Write to Writer" });
    fireEvent.click(screen.getByRole("button", { name: /^Manager, / }));
    fireEvent.keyDown(window, { key: "3", metaKey: true });
    expect(useStore.getState().selected).toBe("id-Manager");
    fireEvent.keyDown(window, { key: "1", metaKey: true });
    await waitFor(() => expect(useStore.getState().selected).toBe("id-Writer"));
    await waitFor(() =>
      expect(document.activeElement).toBe(
        screen.getByRole("combobox", { name: "Write to Writer" }),
      ),
    );
  });
});
