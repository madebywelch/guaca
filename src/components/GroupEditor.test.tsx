import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Group, GroupDraft, GroupReset, Settings, SubscriptionStatus } from "../lib/types";
import { aGroup } from "../test-fixtures";

/**
 * A group's settings, over a mocked runtime.
 *
 * Everything on this surface is an override, and the whole risk is in the draft
 * it sends rather than in the markup. A blank box means "inherit" and has to
 * arrive as null; a key nobody retyped has to be absent, because an empty one
 * clears the stored key; and a group following the app must not send a provider
 * at all. None of that is visible on screen, which is exactly why it is here.
 *
 * The other risk is the delete button, which is two different calls behind one
 * word. An empty group loses a row in a table; a group with a crew in it loses
 * four agents, their computers and their browsers, and none of that comes back.
 */

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
    baseUrl: "https://openrouter.ai/api/v1",
    defaultModel: "anthropic/claude-sonnet-4.5",
    provider: "compatible",
    subscriptionModel: "gpt-5.6-luna",
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
const groupRepositories = vi.fn(async () => []);
const codingHarnesses = vi.fn(async () => []);

vi.mock("../lib/ipc", () => ({
  api: {
    updateGroup: (id: string, draft: GroupDraft) => updateGroup(id, draft),
    createGroup: (draft: GroupDraft) => createGroup(draft),
    deleteGroup: (id: string) => deleteGroup(id),
    disbandGroup: (id: string) => disbandGroup(id),
    clearGroup: (id: string) => clearGroup(id),
    testGroupConnection: (id: string | null, draft: GroupDraft) => testGroupConnection(id, draft),
    subscriptionStatus: () => subscriptionStatus(),
    groupConnectors: () => groupConnectors(),
    groupPlugins: () => groupPlugins(),
    pluginCatalog: () => pluginCatalog(),
    groupRepositories: () => groupRepositories(),
    codingHarnesses: () => codingHarnesses(),
  },
}));

const { GroupEditor } = await import("./GroupEditor");
const { useStore } = await import("../lib/store");

const onClose = vi.fn();

/** `null` opens the editor on a group that does not exist yet. */
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
  // Counted rather than asserted as called at all, so a second save in one test
  // reads the draft it just sent rather than the first one.
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
    const draft = await save();

    expect(draft.inference).toEqual({
      provider: null,
      baseUrl: null,
      defaultModel: null,
      subscriptionModel: null,
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
    // A key sent as "" clears the stored one, so saving a group whose key was
    // never touched must not mention it at all: the field shows a hint, and a
    // hint written back is not a key.
    open(aGroup({ id: KITCHEN, name: "Kitchen", apiKeySet: true, apiKeyHint: "...9999" }));
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
    // Typed and then emptied is an instruction, and the only one that can
    // put a group back on the app's key.
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
    // The panes are unmounted on a section change, so anything a pane held
    // would be discarded by a glance at Limits and back.
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
    // Choosing an endpoint while following an app that is on a subscription
    // would otherwise fill in a URL that nothing read.
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

  it("can spend the subscription while the app pays with a key", async () => {
    subscriptionStatus.mockResolvedValueOnce(signedIn());
    open();
    pane("Provider");
    await screen.findByText(/robert@example.com/);

    fireEvent.click(button("Use it"));
    expect((await save()).inference?.provider).toBe("chatgpt");
  });

  it("says where the sign-in is managed whether or not there is one", async () => {
    // Signed in is the state that matters. A group editor is where an operator
    // whose turns are being refused looks first, and this row deliberately has
    // no sign-out button of its own: the credential belongs to the app. Saying
    // only "robert@example.com · Pro" leaves them looking at a healthy sign-in
    // with nothing to press and nowhere named to go.
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

    // A model is picked from what the plan runs rather than typed, and the
    // first row is the app's, because leaving it alone is a real answer.
    fireEvent.change(screen.getByLabelText(/^Model/), { target: { value: "gpt-5.4-mini" } });
    const draft = await save();
    expect(draft.inference?.subscriptionModel).toBe("gpt-5.4-mini");
    // The endpoint model is a different field and is not touched by this.
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

  it("does not offer plugins or repositories for a group that does not exist yet", () => {
    // A sign-in, a credential and a linked directory all have to belong to
    // something, and there is no row to hang any of them on yet.
    open(null);
    expect((screen.getByRole("tab", { name: "Plugins" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("tab", { name: "Repositories" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
  });

  it("puts repositories in a section of their own rather than under plugins", async () => {
    // A plugin is a server this crew signs in to and a repository is a
    // directory on this machine it writes in. They share a shape and nothing
    // else, and stacked in one pane the operator scrolled past two sign-in
    // panels to reach the one about their own source.
    open();
    pane("Repositories");
    expect(await screen.findByText("Link a repository")).toBeTruthy();

    pane("Plugins");
    await waitFor(() => expect(groupPlugins).toHaveBeenCalled());
    expect(screen.queryByText("Link a repository")).toBeNull();
  });

  it("refuses to save a group with no name", () => {
    open(null);
    expect(button("Create").disabled).toBe(true);
  });
});

describe("deleting a group", () => {
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
    // The count is on the button because that is what the operator is about to
    // press. The machines are in the banner because a count does not say that
    // anything was rented, and destroying a computer is the half of this that
    // cannot be undone.
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
    // The plain delete would be refused for a group with agents in it, and
    // refusing is all it could do: nothing about that error is what was asked
    // for here.
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
    // The first group cannot be deleted, because every agent has to be in one.
    // A dialog that closed on that would look like it had worked.
    disbandGroup.mockRejectedValueOnce({
      kind: "groupNotEmpty",
      message: "every agent has to be in a group, so the first one cannot be deleted",
    });
    open(aGroup({ id: KITCHEN, name: "Research", agentCount: 2 }));
    fireEvent.click(button("Delete"));
    fireEvent.click(button("Delete Research and 2 agents"));

    expect(await screen.findByText(/cannot be deleted/)).toBeTruthy();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("does not offer to reset a group it is already deleting", () => {
    // Two destructive confirmations open at once is a click on the wrong one.
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
