import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { expectRestart, forgetRestart } from "./lib/link";
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
    // The shared fixture rather than a hand-built object: this one had drifted
    // into a mixture of a group and the app's settings, with a group's own
    // `inference` block missing entirely, and nothing typechecks a mock.
    listGroups: () => listGroups(),
    agentActivity: () => agentActivity(),
    usageSummary: async () => [],
    approvalStates: async () => ({}),
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
    // The window reports what it is showing on a timer. Without this the timer
    // threw into an unhandled error that vitest reports after whichever test
    // happened to be running, which is a failure nobody can act on.
    reportView: async () => {},
    capabilities: async () => ({
      loopbackEndpoints: true,
      claudeProvider: true,
      localFiles: true,
    }),
    claudeProgram: async () => ({
      installed: true,
      version: "2.1.247 (Claude Code)",
      install: "npm install -g @anthropic-ai/claude-code",
    }),
    channelMessages: async () => [],
    activityFeed: async () => [],
    agentRoutines: () => agentRoutines(),
    agentComputer: async () => null,
    agentMemory: async () => "",
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
    // The banner used to say the truth and hand over a blindfold: the one
    // control removed the diagnosis and nothing else. These are the two things
    // that help, and `Check the host` is where a person changes hosts.
    useStore.setState({ banner: down });
    render(<App />);
    expect(screen.getByText(/could not reach this workspace/)).toBeTruthy();
    // What to go and look at, in the words of the host this window shows: this
    // suite is the desktop showing its own workspace.
    expect(screen.getByText(/Check that Docker is running/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Check the host" }));
    // The pane that holds On this Mac and Remote host, open rather than the
    // dialog's first section.
    const pane = await screen.findByRole("tab", { name: "Workspace" });
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
    // The real path rather than a banner set by hand: this is also the one
    // place the store's classification of an unreachable refusal is drawn.
    listAgents.mockRejectedValue({ kind: "unreachable", message: down.text });
    render(<App />);
    await screen.findByRole("button", { name: "Retry" });
    const before = listAgents.mock.calls.length;

    // The retry asks again and gets the same nothing, which is not news: the
    // banner is already saying it, and a second sentence saying it again is
    // noise on the one line that matters.
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
  it("renders the rail", async () => {
    render(<App />);
    // The rail is always present, whether or not there are agents. If this is
    // missing, the window is blank.
    expect(await screen.findByRole("navigation", { name: /agents/i })).toBeTruthy();
    expect(screen.getByText("guaca")).toBeTruthy();
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
    // The banner that says a key is missing is suppressed while the first-run
    // page is up, because that page is the thing asking for one: step one is a
    // task and step two is the provider. A test that expected the banner was
    // asking for two prompts for the same key on one screen.
    expect(await screen.findByText(/What would you like help with/i)).toBeTruthy();
    expect(screen.queryByText(/Add an API key/i)).toBeNull();
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
    await waitFor(() => expect(screen.getByText("Manager")).toBeTruthy());
    fireEvent.click(railRow("Manager"));
    await waitFor(() => expect(screen.queryByText(/Add an API key/i)).toBeNull());
    fireEvent.click(railRow("Other agent"));
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
    const crew = railRow("Chef").closest(".rail__group");
    expect(crew?.textContent).toContain("Everyone");
    // Both rows under the one heading, with the pinned one at the head of them.
    // The crew's size used to be read off a digit in that heading; the heading
    // gave the digit's width back to the crew's name, so what says a pinned
    // agent is still in the crew is that it is drawn inside it.
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
    expect(screen.getByRole("complementary", { name: /Manager: details/i })).toBeTruthy();
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

// A component that throws on its first render. At module level because a
// component made inside the test is a new type on every render, which is the
// one thing React cannot keep state across.
const Boom = (): never => {
  throw new Error("render exploded");
};

describe("failure surfacing", () => {
  it("shows the error instead of a blank window when a child throws", async () => {
    const { ErrorBoundary } = await import("./components/ErrorBoundary");
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
  // What the socket hands up, in the words `transport.ts` writes it in: the
  // address, and the one fact the close adds — that the page is not live.
  const gone = {
    kind: "unreachable",
    message:
      "lost the connection to this host at http://127.0.0.1:8787. It may be restarting, or the connection to it is down, and nothing on this page is live until it answers again",
  };

  /** The subscription this page made, as `useRuntime` handed it over. */
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

    // The banner a failed call raises, without a call: the diagnosis, and the
    // controls that do something about it.
    expect(screen.getByText(/lost the connection to this host/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
    // The page stops claiming what it draws is current, and the one row that
    // names the host says since when.
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

    await waitFor(() => expect(screen.queryByText(/lost the connection/)).toBeNull());
    expect(container.querySelector(".app")?.getAttribute("data-link")).toBeNull();
    expect(screen.queryByText(/last heard/)).toBeNull();
    expect(useStore.getState().link).toBe("up");
  });

  it("says nothing while an update is restarting the host on purpose", async () => {
    // The panel that started the update is already saying the host is
    // restarting, and a page that also raised a lost host would argue with it.
    expectRestart();
    listAgents.mockResolvedValue([agent("Ada")]);
    const { container } = render(<App />);
    await screen.findByRole("navigation", { name: /agents/i });
    const { lost } = await connection();

    act(() => lost(gone));

    expect(screen.queryByText(/lost the connection/)).toBeNull();
    expect(container.querySelector(".app")?.getAttribute("data-link")).toBeNull();
    expect(useStore.getState().link).toBe("up");
  });
});

describe("phone navigation", () => {
  it("opens the same channel again and preserves its draft across panes", async () => {
    listAgents.mockResolvedValue([agent("Ada")]);
    localStorage.setItem("guac.inspector", "closed");
    const { container } = render(<App />);
    await screen.findByText("Ada");
    fireEvent.click(railRow("Ada"));
    await screen.findByRole("heading", { name: "Ada" });
    const pane = () => container.querySelector(".app")?.getAttribute("data-mobile-pane");
    const input = container.querySelector(".composer__input") as HTMLTextAreaElement;
    fireEvent.change(input, { target: { value: "Keep this draft" } });
    fireEvent.click(screen.getByRole("button", { name: "Details" }));
    expect(pane()).toBe("details");
    expect(container.querySelector(".inspector--closed")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Back to conversation" }));
    expect(pane()).toBe("conversation");
    fireEvent.click(screen.getByRole("button", { name: "Chats" }));
    expect(pane()).toBe("agents");
    fireEvent.click(railRow("Ada"));
    expect(pane()).toBe("conversation");
    expect((container.querySelector(".composer__input") as HTMLTextAreaElement).value).toBe(
      "Keep this draft",
    );
  });

  it("lets a phone choose a crew by name without hovering", async () => {
    const other = aGroup({ id: "crew-other", name: "Research" });
    listGroups.mockResolvedValue([aGroup(), other]);
    listAgents.mockResolvedValue([agent("Ada"), { ...agent("Lin"), groupId: other.id }]);
    const { container } = render(<App />);
    await screen.findByText("Ada");
    fireEvent.change(screen.getByRole("combobox", { name: "Group" }), {
      target: { value: other.id },
    });
    await waitFor(() => expect(useStore.getState().railGroup).toBe(other.id));
    expect(railNames()).toEqual(["Lin"]);
    expect(container.querySelector(".app")?.getAttribute("data-mobile-pane")).toBe("agents");
  });
});
