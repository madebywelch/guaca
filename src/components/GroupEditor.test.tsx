import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { clockTime } from "../lib/routine";
import type {
  Envelope,
  Group,
  GroupDraft,
  GroupReset,
  Settings,
  SubscriptionStatus,
} from "../lib/types";
import { aGroup } from "../test-fixtures";

const KITCHEN = "00000000-0000-4000-8000-000000000001";

function settings(over: Partial<Settings> = {}): Settings {
  return {
    operatorName: "Robert",
    e2bKeySet: false,
    e2bKeyHint: "",
    computerIdleMinutes: 15,
    kernelKeySet: false,
    kernelKeyHint: "",
    browserIdleMinutes: 60,
    browserStealth: false,
    searchKeySet: false,
    searchKeyHint: "",
    searchesWith: "",
    baseUrl: "https://openrouter.ai/api/v1",
    defaultModel: "anthropic/claude-sonnet-4.5",
    provider: "compatible",
    subscriptionModel: "gpt-5.6-luna",
    reasoningEffort: "auto",
    subscriptionModels: ["gpt-5.6-luna", "gpt-5.4-mini"],
    apiKeySet: true,
    apiKeyHint: "…9f2c",
    requestTimeoutSecs: 120,
    limits: {
      maxHops: 8,
      maxStepsPerRun: 60,
      maxFanoutPerCall: 8,
      maxSendsPerPair: 6,
      maxToolRounds: 24,
    },
    ...over,
  };
}

function signedIn(): SubscriptionStatus {
  return { signedIn: true, email: "robert@example.com", plan: "pro", includesCodex: true };
}

const updateGroup = vi.fn<(id: string, draft: GroupDraft) => Promise<Group>>(async () => aGroup());
const createGroup = vi.fn<(draft: GroupDraft) => Promise<Group>>(async () => aGroup());
const deleteGroup = vi.fn<(id: string) => Promise<void>>(async () => {});
const disbandGroup = vi.fn<(id: string) => Promise<void>>(async () => {});
const clearGroup = vi.fn<(id: string) => Promise<GroupReset>>(async () => ({
  messages: 0,
  routines: 0,
  notes: 0,
  workingNotes: 0,
  calls: 0,
}));
const testGroupConnection = vi.fn<(id: string | null, draft: GroupDraft) => Promise<string>>(
  async () => "Reached it.",
);
const subscriptionStatus = vi.fn<() => Promise<SubscriptionStatus>>(async () => ({
  signedIn: false,
  email: "",
  plan: "",
  includesCodex: false,
}));
const groupConnectors = vi.fn(async () => []);
const groupPlugins = vi.fn(async () => []);
const pluginCatalog = vi.fn(async () => []);
const codingHarnesses = vi.fn(async () => []);
const conversationFlow = vi.fn<(group: string, limit?: number) => Promise<Envelope[]>>(
  async () => [],
);
const usageForRuns = vi.fn(async () => []);

vi.mock("../lib/ipc", () => ({
  api: {
    updateGroup: (id: string, draft: GroupDraft) => updateGroup(id, draft),
    createGroup: (draft: GroupDraft) => createGroup(draft),
    deleteGroup: (id: string) => deleteGroup(id),
    disbandGroup: (id: string) => disbandGroup(id),
    clearGroup: (id: string) => clearGroup(id),
    testGroupConnection: (id: string | null, draft: GroupDraft) => testGroupConnection(id, draft),
    subscriptionStatus: () => subscriptionStatus(),
    subscriptionModels: async () =>
      ["gpt-5.6-luna", "gpt-5.4-mini"].map((slug) => ({
        slug,
        defaultReasoningEffort: "medium",
        reasoningEfforts: [
          { effort: "low", description: "Faster" },
          { effort: "high", description: "More thorough" },
        ],
      })),
    groupConnectors: () => groupConnectors(),
    groupPlugins: () => groupPlugins(),
    pluginCatalog: () => pluginCatalog(),
    codingHarnesses: () => codingHarnesses(),
    conversationFlow: (group: string, limit?: number) => conversationFlow(group, limit),
    usageForRuns: () => usageForRuns(),
    builtinTools: async () => [],
    groupReconnect: async () => [],
  },
}));

const { GroupEditor } = await import("./GroupEditor");
const { useStore } = await import("../lib/store");

const onClose = vi.fn();

function open(group: Group | null = aGroup({ id: KITCHEN, name: "Kitchen" }), over = {}) {
  useStore.setState({ settings: settings(over), refreshAgents: async () => {} });
  return render(<GroupEditor group={group ?? undefined} onClose={onClose} />);
}

function pane(label: string): void {
  fireEvent.click(screen.getByRole("tab", { name: label }));
}

function field(label: RegExp): HTMLInputElement {
  return screen.getByLabelText(label) as HTMLInputElement;
}

function type(label: RegExp, value: string): void {
  fireEvent.change(field(label), { target: { value } });
}

async function save(): Promise<GroupDraft> {
  const before = updateGroup.mock.calls.length;
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(updateGroup.mock.calls.length).toBe(before + 1));
  return updateGroup.mock.calls.at(-1)![1];
}

function button(name: string | RegExp): HTMLButtonElement {
  return screen.getByRole("button", { name }) as HTMLButtonElement;
}

beforeEach(() => {
  vi.clearAllMocks();
  useStore.setState({ settings: settings() });
});

describe("what a group sends", () => {
  it("says inherit as null rather than as an empty string", async () => {
    open();
    type(/Name/, "Kitchenette");
    const draft = await save();

    expect(draft.inference).toEqual({
      provider: null,
      baseUrl: null,
      defaultModel: null,
      subscriptionModel: null,
      reasoningEffort: null,
      requestTimeoutSecs: null,
    });
    expect(draft.limits).toEqual({
      maxHops: null,
      maxStepsPerRun: null,
      maxFanoutPerCall: null,
      maxSendsPerPair: null,
      maxToolRounds: null,
    });
  });

  it("leaves a stored key alone until somebody types one", async () => {
    open(aGroup({ id: KITCHEN, name: "Kitchen", apiKeySet: true, apiKeyHint: "...9999" }));
    type(/Name/, "Kitchenette");
    expect("apiKey" in (await save())).toBe(false);
  });

  it("sends a key that was typed", async () => {
    open(aGroup({ id: KITCHEN, name: "Kitchen", apiKeySet: true, apiKeyHint: "...9999" }));
    pane("Provider");
    type(/API key/, "sk-typed");
    expect((await save()).apiKey).toBe("sk-typed");
  });

  it("clears a key that was deliberately emptied", async () => {
    open(aGroup({ id: KITCHEN, name: "Kitchen", apiKeySet: true, apiKeyHint: "...9999" }));
    pane("Provider");
    type(/API key/, "sk-typed");
    type(/API key/, "");
    expect((await save()).apiKey).toBe("");
  });

  it("sends a limit as a number and the rest as inherit", async () => {
    open();
    pane("Limits");
    type(/Model calls per conversation/, "12");

    const draft = await save();
    expect(draft.limits?.maxStepsPerRun).toBe(12);
    expect(draft.limits?.maxHops).toBeNull();
  });

  it("keeps what was typed in one section while another is open", async () => {
    open();
    pane("Provider");
    type(/Inference endpoint/, "http://localhost:1234/v1");
    pane("Limits");
    pane("Provider");
    expect(field(/Inference endpoint/).value).toBe("http://localhost:1234/v1");

    expect((await save()).inference?.baseUrl).toBe("http://localhost:1234/v1");
  });
});

describe("who pays for a group's turns", () => {
  it("follows the app until the operator says otherwise", async () => {
    open(null);
    pane("Provider");
    expect(button(/Follow the app settings/).getAttribute("aria-current")).toBe("true");
  });

  it("names the endpoint chosen from a preset, and chooses to pay with a key", async () => {
    open(null, { provider: "chatgpt" });
    type(/Name/, "Groqers");
    pane("Provider");
    fireEvent.click(button(/Groq/));

    fireEvent.click(button("Create"));
    await waitFor(() => expect(createGroup).toHaveBeenCalled());
    const draft = createGroup.mock.calls.at(-1)![0];
    expect(draft.inference?.provider).toBe("compatible");
    expect(draft.inference?.baseUrl).toBe("https://api.groq.com/openai/v1");
  });

  it("withholds the Claude plan and the machine-local presets on a server", () => {
    useStore.setState({
      capabilities: {
        loopbackEndpoints: false,
        borrowedSignins: false,
        claudeProvider: false,
        localFiles: false,
      },
    });
    try {
      open();
      pane("Provider");
      expect(screen.getByText("Not on a server")).toBeTruthy();
      expect(screen.queryByRole("button", { name: "Use the Claude subscription" })).toBeNull();
      expect(button(/LM Studio/).disabled).toBe(true);
      expect(button(/LM Studio/).textContent).toContain("Not from a server");
      expect(button(/Groq/).disabled).toBe(false);
    } finally {
      useStore.setState({
        capabilities: {
          loopbackEndpoints: true,
          borrowedSignins: true,
          claudeProvider: true,
          localFiles: true,
        },
      });
    }
  });

  it("can spend the subscription while the app pays with a key", async () => {
    subscriptionStatus.mockResolvedValueOnce(signedIn());
    open();
    pane("Provider");
    await screen.findByText(/robert@example.com/);

    fireEvent.click(button("Use the ChatGPT subscription"));
    expect((await save()).inference?.provider).toBe("chatgpt");
  });

  it("says where the sign-in is managed whether or not there is one", async () => {
    subscriptionStatus.mockResolvedValueOnce(signedIn());
    open();
    pane("Provider");
    await screen.findByText(/robert@example.com/);
    expect(screen.getByText(/Settings . Provider/)).toBeTruthy();
  });

  it("offers the subscription's own models once it is the one paying", async () => {
    subscriptionStatus.mockResolvedValueOnce(signedIn());
    open(
      aGroup({
        id: KITCHEN,
        name: "Kitchen",
        inference: { ...aGroup().inference, provider: "chatgpt" },
      }),
    );
    pane("Provider");
    await screen.findByText(/robert@example.com/);

    fireEvent.change(screen.getByLabelText(/^Model/), { target: { value: "gpt-5.4-mini" } });
    const draft = await save();
    expect(draft.inference?.subscriptionModel).toBe("gpt-5.4-mini");
    expect(draft.inference?.defaultModel).toBeNull();
  });

  it("sends what is on screen to the test, not what is stored", async () => {
    open();
    pane("Provider");
    type(/Inference endpoint/, "http://localhost:1234/v1");
    fireEvent.click(button("Test connection"));

    await waitFor(() => expect(testGroupConnection).toHaveBeenCalled());
    const [id, draft] = testGroupConnection.mock.calls.at(-1)!;
    expect(id).toBe(KITCHEN);
    expect(draft.inference?.baseUrl).toBe("http://localhost:1234/v1");
    await screen.findByText("Reached it.");
  });
});

describe("what the operator is shown", () => {
  it("shows what would be inherited rather than an empty box", async () => {
    open();
    pane("Provider");
    expect(field(/Inference endpoint/).placeholder).toBe("https://openrouter.ai/api/v1");
    expect(field(/Default model/).placeholder).toBe("anthropic/claude-sonnet-4.5");
    pane("Limits");
    expect(field(/Model calls per conversation/).placeholder).toBe("60");
  });

  it("does not offer connectors or secrets for a group that does not exist yet", () => {
    open(null);
    expect((screen.getByRole("tab", { name: "Connectors" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    expect((screen.getByRole("tab", { name: "Secrets" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("has no repositories pane, because a repository is something an agent clones", () => {
    open();
    expect(screen.queryByRole("tab", { name: "Repositories" })).toBeNull();
  });

  it("refuses to save a group with no name", () => {
    open(null);
    expect(button("Create").disabled).toBe(true);
  });
});

describe("the section list", () => {
  const tabs = () => screen.getAllByRole("tab");
  const stops = () => tabs().filter((tab) => tab.tabIndex === 0);
  const tab = (name: string) => screen.getByRole("tab", { name });

  it("is one Tab stop, on the open section", () => {
    open();
    expect(stops()).toEqual([tab("General")]);
    pane("Limits");
    expect(stops()).toEqual([tab("Limits")]);
  });

  it("moves to the next and previous section with the arrow keys, and opens it", () => {
    open();
    fireEvent.keyDown(tab("General"), { key: "ArrowDown" });
    expect(document.activeElement).toBe(tab("Provider"));
    expect(tab("Provider").getAttribute("aria-selected")).toBe("true");
    expect(stops()).toEqual([tab("Provider")]);
    fireEvent.keyDown(tab("Provider"), { key: "ArrowRight" });
    expect(document.activeElement).toBe(tab("Limits"));
    fireEvent.keyDown(tab("Limits"), { key: "ArrowUp" });
    expect(document.activeElement).toBe(tab("Provider"));
    fireEvent.keyDown(tab("Provider"), { key: "ArrowLeft" });
    expect(document.activeElement).toBe(tab("General"));
    expect(screen.getByRole("tabpanel", { name: "General" })).toBeTruthy();
  });

  it("goes to either end with Home and End, and wraps past them", () => {
    open();
    fireEvent.keyDown(tab("General"), { key: "End" });
    const last = tabs().at(-1) as HTMLElement;
    expect(document.activeElement).toBe(last);
    expect(last.getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(last, { key: "ArrowDown" });
    expect(document.activeElement).toBe(tab("General"));
    fireEvent.keyDown(tab("General"), { key: "ArrowUp" });
    expect(document.activeElement).toBe(last);
    fireEvent.keyDown(last, { key: "Home" });
    expect(document.activeElement).toBe(tab("General"));
  });

  it("names the open panel after its section", () => {
    open();
    pane("Limits");
    const panel = screen.getByRole("tabpanel", { name: "Limits" });
    expect(tab("Limits").getAttribute("aria-controls")).toBe(panel.id);
  });

  it("steps over a section a new group cannot open yet", () => {
    open(null);
    pane("Limits");
    fireEvent.keyDown(tab("Limits"), { key: "ArrowDown" });
    expect(document.activeElement).toBe(tab("Tools"));
    expect(tab("Tools").getAttribute("aria-selected")).toBe("true");
  });
});

describe("the Save in the foot", () => {
  const saveButton = () => screen.queryByRole("button", { name: "Save" });

  it("is not there until something is waiting for it", () => {
    open();
    expect(saveButton()).toBeNull();
    type(/Name/, "Kitchenette");
    expect(saveButton()).toBeTruthy();
  });

  it("says Close rather than Cancel while there is nothing to cancel", () => {
    open();
    expect(button("Close")).toBeTruthy();
    type(/Name/, "Kitchenette");
    expect(button("Cancel")).toBeTruthy();
  });

  it("offers nothing on the panes that save as the operator goes", async () => {
    open();
    pane("Secrets");
    expect(await screen.findByText("Add a secret")).toBeTruthy();
    expect(saveButton()).toBeNull();
  });

  it("is still reachable from a pane that stages nothing", async () => {
    open();
    type(/Name/, "Kitchenette");
    pane("Secrets");
    expect(await screen.findByText("Add a secret")).toBeTruthy();
    expect(saveButton()).toBeTruthy();
  });

  it("is always there for a group that does not exist yet", () => {
    open(null);
    expect(button("Create")).toBeTruthy();
  });
});

describe("deleting a group", () => {
  it("restores the pending reset confirmation after canceling delete", () => {
    open();
    fireEvent.click(button("Start fresh"));
    fireEvent.click(button("Delete"));
    expect(screen.queryByRole("button", { name: "Reset every agent" })).toBeNull();
    fireEvent.click(button("Keep"));
    expect(button("Reset every agent")).toBeTruthy();
    fireEvent.click(button("Keep them"));
    expect(button("Start fresh")).toBeTruthy();
    expect(clearGroup).not.toHaveBeenCalled();
    expect(deleteGroup).not.toHaveBeenCalled();
  });

  it("keeps the reset pending and disables its action until the response arrives", async () => {
    let finish!: (result: GroupReset) => void;
    clearGroup.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    open();
    fireEvent.click(button("Start fresh"));
    fireEvent.click(button("Reset every agent"));
    expect(button("Reset every agent").disabled).toBe(true);
    finish({ messages: 1, routines: 0, notes: 0, workingNotes: 0, calls: 0 });
    expect(await screen.findByText(/Reset: 1 message/)).toBeTruthy();
  });

  it("takes two clicks to delete anything", () => {
    open();
    fireEvent.click(button("Delete"));
    expect(deleteGroup).not.toHaveBeenCalled();
    expect(disbandGroup).not.toHaveBeenCalled();
  });

  it("deletes an empty group without disbanding anybody", async () => {
    open(aGroup({ id: KITCHEN, name: "Research" }));
    fireEvent.click(button("Delete"));
    fireEvent.click(button("Delete Research"));

    await waitFor(() => expect(deleteGroup).toHaveBeenCalledWith(KITCHEN));
    expect(disbandGroup).not.toHaveBeenCalled();
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("says how many agents go, and what goes with them", () => {
    open(aGroup({ id: KITCHEN, name: "Research", agentCount: 4 }));
    fireEvent.click(button("Delete"));

    expect(button("Delete Research and 4 agents")).toBeTruthy();
    expect(screen.getByText(/computers, browsers/)).toBeTruthy();
  });

  it("counts one agent as one agent", () => {
    open(aGroup({ id: KITCHEN, name: "Research", agentCount: 1 }));
    fireEvent.click(button("Delete"));
    expect(button("Delete Research and 1 agent")).toBeTruthy();
  });

  it("disbands a group that still holds a crew", async () => {
    open(aGroup({ id: KITCHEN, name: "Research", agentCount: 4 }));
    fireEvent.click(button("Delete"));
    fireEvent.click(button("Delete Research and 4 agents"));

    await waitFor(() => expect(disbandGroup).toHaveBeenCalledWith(KITCHEN));
    expect(deleteGroup).not.toHaveBeenCalled();
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("keeps the crew when the second click is Keep", () => {
    open(aGroup({ id: KITCHEN, name: "Research", agentCount: 4 }));
    fireEvent.click(button("Delete"));
    fireEvent.click(button("Keep"));

    expect(disbandGroup).not.toHaveBeenCalled();
    expect(screen.queryByText(/computers, browsers/)).toBeNull();
    expect(button("Delete")).toBeTruthy();
  });

  it("leaves the dialog open on a refusal, with the reason from the runtime", async () => {
    disbandGroup.mockRejectedValueOnce({
      kind: "groupNotEmpty",
      message: "at least one group must remain, so the last group cannot be deleted",
    });
    open(aGroup({ id: KITCHEN, name: "Research", agentCount: 2 }));
    fireEvent.click(button("Delete"));
    fireEvent.click(button("Delete Research and 2 agents"));

    expect(await screen.findByText(/cannot be deleted/)).toBeTruthy();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("says what the reset took, both stores included", async () => {
    clearGroup.mockResolvedValueOnce({
      messages: 12,
      routines: 2,
      notes: 3,
      workingNotes: 7,
      calls: 41,
    });
    open();
    fireEvent.click(button("Start fresh"));
    fireEvent.click(button("Reset every agent"));

    expect(await screen.findByText(/7 working notes/)).toBeTruthy();
  });

  it("does not offer to reset a group it is already deleting", () => {
    open();
    expect(button("Start fresh")).toBeTruthy();
    fireEvent.click(button("Delete"));
    expect(screen.queryByText("Start fresh")).toBeNull();
  });

  it("offers nothing to delete on a group that does not exist yet", () => {
    open(null);
    expect(screen.queryByText("Delete")).toBeNull();
    expect(screen.queryByText("Start fresh")).toBeNull();
  });
});

describe("a crew's activity", () => {
  function said(over: Partial<Envelope> = {}): Envelope {
    return {
      id: "m1",
      runId: "r1",
      channelId: "chef",
      from: { kind: "human" },
      to: { kind: "agent", id: "chef" },
      parts: [{ type: "text", text: "how is the prep going" }],
      trust: "operator",
      hop: 0,
      expectsReply: true,
      intent: "work",
      cause: null,
      createdAt: 1_700_000_000_000,
      ...over,
    };
  }

  it("reads this crew's conversation and nobody else's", async () => {
    conversationFlow.mockResolvedValueOnce([said()]);
    open(aGroup({ id: KITCHEN, name: "Kitchen" }));
    pane("Activity");

    await waitFor(() => expect(conversationFlow).toHaveBeenCalled());
    expect(conversationFlow.mock.calls[0]![0]).toBe(KITCHEN);
    expect(await screen.findAllByText(/how is the prep going/)).toHaveLength(2);
  });

  it("says nothing has happened rather than drawing an empty board", async () => {
    open();
    pane("Activity");

    expect(await screen.findByText(/Nothing has happened yet/)).toBeTruthy();
  });

  it("is not offered on a group that does not exist yet", () => {
    open(null);
    expect(screen.getByRole("tab", { name: "Activity" }).hasAttribute("disabled")).toBe(true);
  });
});

it("sets effort while inheriting the app's ChatGPT provider and model", async () => {
  open(aGroup(), { provider: "chatgpt", reasoningEffort: "low" });
  pane("Provider");
  const effort = await screen.findByRole("combobox", { name: /^Reasoning effort/ });
  await screen.findByRole("option", { name: "high" });
  fireEvent.change(effort, { target: { value: "high" } });
  const draft = await save();
  expect(draft.inference).toMatchObject({
    provider: null,
    subscriptionModel: null,
    reasoningEffort: "high",
  });
});

it("clears a group's effort override and can explicitly choose the model default", async () => {
  const group = aGroup({
    inference: { ...aGroup().inference, provider: "chatgpt", reasoningEffort: "high" },
  });
  const view = open(group);
  pane("Provider");
  fireEvent.change(await screen.findByRole("combobox", { name: /^Reasoning effort/ }), {
    target: { value: "" },
  });
  expect((await save()).inference?.reasoningEffort).toBeNull();
  view.unmount();
  open(group);
  pane("Provider");
  fireEvent.change(await screen.findByRole("combobox", { name: /^Reasoning effort/ }), {
    target: { value: "auto" },
  });
  expect((await save()).inference?.reasoningEffort).toBe("auto");
});

describe("the overnight tidy", () => {
  it("opens off at the hour it was left at, and sends the switch and the hour", async () => {
    open();
    const switched = screen.getByRole("checkbox", { name: "Tidy memory overnight" });
    const hour = screen.getByRole("combobox", { name: "Hour" }) as HTMLSelectElement;
    expect((switched as HTMLInputElement).checked).toBe(false);
    expect(hour.value).toBe("3");

    fireEvent.click(switched);
    fireEvent.change(hour, { target: { value: "4" } });
    expect((await save()).tidy).toEqual({ on: true, hour: 4 });
  });

  it("keeps a group's hour while the switch is off", async () => {
    open(aGroup({ id: KITCHEN, name: "Kitchen", tidy: { on: true, hour: 22 } }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Tidy memory overnight" }));
    expect((await save()).tidy).toEqual({ on: false, hour: 22 });
  });

  it("draws each hour as a routine's time is drawn", () => {
    open();
    const hour = screen.getByRole("combobox", { name: "Hour" }) as HTMLSelectElement;
    expect(hour.options).toHaveLength(24);
    expect(hour.options[3]!.textContent).toBe(clockTime(new Date(2000, 0, 1, 3).getTime()));
  });
});
