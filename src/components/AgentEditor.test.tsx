import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "../lib/store";
import type { AgentCard, AgentDraft, Group, Settings } from "../lib/types";
import { aGroup } from "../test-fixtures";
import { AgentEditor } from "./AgentEditor";

const api = vi.hoisted(() => ({
  subscriptionModels: vi.fn(async () =>
    ["chat-default", "chat-specialist", "crew-default"].map((slug) => ({
      slug,
      defaultReasoningEffort: "medium",
      reasoningEfforts: [
        { effort: "low", description: "Faster" },
        { effort: "high", description: "More thorough" },
      ],
    })),
  ),
  updateAgent: vi.fn(async () => {}),
  setAgentErrands: vi.fn(async () => {}),
  setAgentWeb: vi.fn(async () => {}),
  createAgent: vi.fn(async () => ({ id: "new-agent" })),
  rankedModels: vi.fn(async () => []),
}));
vi.mock("../lib/ipc", () => ({ api }));
vi.mock("./GrantList", () => ({ GrantList: () => null }));
vi.mock("./SigninList", () => ({ SigninList: () => null }));

const settings: Settings = {
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
  defaultModel: "anthropic/endpoint-model",
  provider: "compatible",
  subscriptionModel: "chat-default",
  reasoningEffort: "auto",
  subscriptionModels: ["chat-default"],
  apiKeySet: true,
  apiKeyHint: "",
  requestTimeoutSecs: 120,
  limits: {
    maxHops: 8,
    maxStepsPerRun: 60,
    maxFanoutPerCall: 8,
    maxSendsPerPair: 6,
    maxToolRounds: 24,
  },
};
const crew = aGroup({
  name: "Research",
  inference: { ...aGroup().inference, provider: "chatgpt", subscriptionModel: "crew-default" },
});
function card(model = "", subscriptionModel = ""): AgentCard {
  return {
    id: "agent-1",
    groupId: crew.id,
    name: "Counsel",
    avatar: "avocado",
    color: "#c7d96b",
    model,
    subscriptionModel,
    systemPrompt: "",
    skills: [],
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    readsWeb: true,
    browserConsent: "open",
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    discardedAt: null,
  };
}
function open(agent?: AgentCard, groups: Group[] = [crew], app = settings) {
  useStore.setState({
    settings: app,
    groups,
    agents: agent ? [agent] : [],
    refreshAgents: async () => {},
    select: async () => {},
  });
  return render(<AgentEditor agent={agent} onClose={vi.fn()} />);
}
function modelSelect() {
  return screen.getByRole("combobox", { name: /^Model/ }) as HTMLSelectElement;
}
async function save() {
  fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() => expect(api.updateAgent).toHaveBeenCalled());
  return api.updateAgent.mock.calls.at(-1);
}

beforeEach(() => {
  vi.clearAllMocks();
  api.subscriptionModels.mockResolvedValue(
    ["chat-default", "chat-specialist", "crew-default"].map((slug) => ({
      slug,
      defaultReasoningEffort: "medium",
      reasoningEfforts: [
        { effort: "low", description: "Faster" },
        { effort: "high", description: "Thorough" },
      ],
    })),
  );
});

it("keeps an endpoint's model out of the ChatGPT list, and keeps it for the endpoint", async () => {
  open(card("xiaomi/mimo-v2.6-pro"));
  await screen.findByRole("option", { name: "chat-specialist" });
  expect(modelSelect().value).toBe("");
  expect(screen.queryByRole("option", { name: "xiaomi/mimo-v2.6-pro" })).toBeNull();
  await save();
  expect(api.updateAgent).toHaveBeenCalledWith(
    "agent-1",
    expect.objectContaining({ model: "xiaomi/mimo-v2.6-pro", subscriptionModel: "" }),
  );
});

it("keeps a retired ChatGPT choice visible and explains it after the account list loads", async () => {
  open(card("", "retired-model"));
  expect(modelSelect().value).toBe("retired-model");
  expect((await screen.findByRole("status")).textContent).toContain(
    "not in your account's current ChatGPT model list",
  );
  fireEvent.change(modelSelect(), { target: { value: "" } });
  await save();
  expect(api.updateAgent).toHaveBeenCalledWith(
    "agent-1",
    expect.objectContaining({ subscriptionModel: "" }),
  );
});

it("keeps inheritance and the saved override usable when discovery fails", async () => {
  api.subscriptionModels.mockRejectedValue(new Error("offline"));
  open(card("", "saved-model"));
  expect((await screen.findByRole("status")).textContent).toContain(
    "Could not load current ChatGPT models",
  );
  expect(modelSelect().value).toBe("saved-model");
  expect(screen.getByRole("option", { name: "Use group default · crew-default" })).toBeTruthy();
  await save();
  expect(api.updateAgent).toHaveBeenCalledWith(
    "agent-1",
    expect.objectContaining({ subscriptionModel: "saved-model" }),
  );
});

it("uses the group's ChatGPT provider and saves an account model for just this agent", async () => {
  open(card());
  expect(screen.getByText("Provider · ChatGPT subscription")).toBeTruthy();
  expect(screen.getByText(/Set in Research.s settings/)).toBeTruthy();
  expect(modelSelect().value).toBe("");
  expect(screen.getByRole("option", { name: "Use group default · crew-default" })).toBeTruthy();
  await screen.findByRole("option", { name: "chat-specialist" });
  fireEvent.change(modelSelect(), { target: { value: "chat-specialist" } });
  await save();
  expect(api.updateAgent).toHaveBeenCalledWith(
    "agent-1",
    expect.objectContaining({ model: "", subscriptionModel: "chat-specialist" }),
  );
  expect(api.rankedModels).not.toHaveBeenCalled();
});

it("creates agents inheriting the group model instead of pinning the app's endpoint model", async () => {
  open();
  await screen.findByRole("option", { name: "chat-specialist" });
  fireEvent.change(screen.getByRole("textbox", { name: /^Name/ }), {
    target: { value: "Researcher" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Create agent" }));
  await waitFor(() =>
    expect(api.createAgent).toHaveBeenCalledWith(
      expect.objectContaining({ model: "", subscriptionModel: "" }),
    ),
  );
});

it("follows the app's ChatGPT model when the group inherits both settings", async () => {
  open(card(), [aGroup()], { ...settings, provider: "chatgpt" });
  expect(screen.getByRole("option", { name: "Use group default · chat-default" })).toBeTruthy();
  await screen.findByRole("option", { name: "chat-specialist" });
});

it("uses the endpoint and model of a group that overrides an app on ChatGPT", async () => {
  open(
    card("local/specialist"),
    [
      aGroup({
        inference: {
          ...aGroup().inference,
          provider: "compatible",
          baseUrl: "http://localhost:1234/v1",
          defaultModel: "local/default",
        },
      }),
    ],
    { ...settings, provider: "chatgpt" },
  );
  expect(screen.getByText("Provider · LM Studio")).toBeTruthy();
  const field = screen.getByRole("textbox", { name: /^Model/ }) as HTMLInputElement;
  expect(field.placeholder).toBe("Use group default · local/default");
  expect(field.value).toBe("local/specialist");
  fireEvent.click(screen.getByRole("button", { name: "Use group default" }));
  expect(field.value).toBe("");
  await save();
  expect(api.updateAgent).toHaveBeenCalledWith("agent-1", expect.objectContaining({ model: "" }));
  expect(api.subscriptionModels).not.toHaveBeenCalled();
  expect(api.rankedModels).not.toHaveBeenCalled();
});

it("keeps each provider's override when moving groups, and shows only the one that runs", async () => {
  open(card("anthropic/specialist", "chat-specialist"), [
    crew,
    aGroup({ id: "endpoint", name: "Endpoint" }),
  ]);
  await screen.findByRole("option", { name: "chat-specialist" });
  expect(modelSelect().value).toBe("chat-specialist");
  fireEvent.change(screen.getByRole("combobox", { name: /^Group/ }), {
    target: { value: "endpoint" },
  });
  expect(screen.getByText("Provider · OpenRouter")).toBeTruthy();
  expect((screen.getByRole("textbox", { name: /^Model/ }) as HTMLInputElement).value).toBe(
    "anthropic/specialist",
  );
  fireEvent.change(screen.getByRole("combobox", { name: /^Group/ }), {
    target: { value: crew.id },
  });
  expect(modelSelect().value).toBe("chat-specialist");
  await save();
  expect(api.updateAgent).toHaveBeenCalledWith(
    "agent-1",
    expect.objectContaining({
      model: "anthropic/specialist",
      subscriptionModel: "chat-specialist",
    }),
  );
});

it("explains Claude's model ownership and preserves the inactive override when saving", async () => {
  open(card("saved-model"), [aGroup({ inference: { ...aGroup().inference, provider: "claude" } })]);
  expect(screen.getByText("Provider · Claude subscription")).toBeTruthy();
  expect(screen.getByText(/Claude controls which model runs/)).toBeTruthy();
  expect(screen.queryByRole("textbox", { name: /^Model/ })).toBeNull();
  expect(screen.queryByRole("combobox", { name: /^Model/ })).toBeNull();
  await save();
  expect(api.updateAgent).toHaveBeenCalledWith(
    "agent-1",
    expect.objectContaining({ model: "saved-model" }),
  );
  expect(api.subscriptionModels).not.toHaveBeenCalled();
  expect(api.rankedModels).not.toHaveBeenCalled();
});

it("overrides effort while inheriting the model and restores group inheritance", async () => {
  const view = open({ ...card(), reasoningEffort: "high" });
  const effort = await screen.findByRole("combobox", { name: /^Reasoning effort/ });
  await screen.findByRole("option", { name: "low" });
  fireEvent.change(effort, { target: { value: "low" } });
  await save();
  expect(api.updateAgent).toHaveBeenLastCalledWith(
    "agent-1",
    expect.objectContaining({ model: "", reasoningEffort: "low" }),
  );
  view.unmount();
  open({ ...card(), reasoningEffort: "low" });
  fireEvent.change(screen.getByRole("combobox", { name: /^Reasoning effort/ }), {
    target: { value: "" },
  });
  await save();
  expect(api.updateAgent).toHaveBeenLastCalledWith(
    "agent-1",
    expect.objectContaining({ model: "", reasoningEffort: null }),
  );
});

it("updates inherited effort when the agent changes groups", () => {
  const second = aGroup({
    id: "second",
    name: "Second",
    inference: { ...crew.inference, reasoningEffort: "high" },
  });
  open(card(), [crew, second]);
  fireEvent.change(screen.getByRole("combobox", { name: /^Group/ }), {
    target: { value: "second" },
  });
  expect(screen.getByRole("option", { name: "Use group default · high" })).toBeTruthy();
});

it("stays off when switching errands on is refused, and says why", async () => {
  api.setAgentErrands.mockRejectedValueOnce(new Error("The agent is gone."));
  open(card());
  const toggle = screen.getByRole("checkbox", { name: /^Errands/ }) as HTMLInputElement;
  expect(toggle.checked, "off until the operator says otherwise").toBe(false);

  fireEvent.click(toggle);
  expect(await screen.findByText("The agent is gone.")).toBeTruthy();
  expect(toggle.checked).toBe(false);
  expect(api.updateAgent, "a grant is not an edit to the card").not.toHaveBeenCalled();
});

it("switches errands on at once, without waiting for Save", async () => {
  open(card());
  const toggle = screen.getByRole("checkbox", { name: /^Errands/ }) as HTMLInputElement;
  fireEvent.click(toggle);
  await waitFor(() => expect(toggle.checked).toBe(true));
  expect(api.setAgentErrands).toHaveBeenCalledWith("agent-1", true);
});

it("shows web reading on for an agent and switches it off at once", async () => {
  open(card());
  const toggle = screen.getByRole("checkbox", { name: /^Web reading/ }) as HTMLInputElement;
  expect(toggle.checked, "on unless the operator turned it off").toBe(true);
  fireEvent.click(toggle);
  await waitFor(() => expect(toggle.checked).toBe(false));
  expect(api.setAgentWeb).toHaveBeenCalledWith("agent-1", false);
  expect(api.updateAgent, "a grant is not an edit to the card").not.toHaveBeenCalled();
});

it("keeps web reading on when switching it off is refused, and says why", async () => {
  api.setAgentWeb.mockRejectedValueOnce(new Error("The agent is gone."));
  open(card());
  const toggle = screen.getByRole("checkbox", { name: /^Web reading/ }) as HTMLInputElement;
  fireEvent.click(toggle);
  expect(await screen.findByText("The agent is gone.")).toBeTruthy();
  expect(toggle.checked).toBe(true);
});

it("offers no errands switch while the agent is still being created", async () => {
  open();
  await waitFor(() => expect(api.subscriptionModels).toHaveBeenCalled());
  await act(async () => {});
  expect(screen.queryByRole("checkbox", { name: /^Errands/ })).toBeNull();
});

function Sheet() {
  const [shown, setShown] = useState(true);
  return shown ? <AgentEditor onClose={() => setShown(false)} /> : null;
}

describe("crossing New agent from the keyboard", () => {
  const name = () => screen.getByRole("textbox", { name: /^Name/ });
  const field = () => {
    const box = document.createElement("textarea");
    box.dataset.field = "";
    document.body.append(box);
    return box;
  };

  afterEach(() => {
    for (const box of document.querySelectorAll("[data-field]")) box.remove();
  });

  it("creates nothing on Return while the name is blank", () => {
    open();
    fireEvent.keyDown(name(), { key: "Enter" });
    expect(api.createAgent).not.toHaveBeenCalled();
  });

  it("lets Return write a new line in Instructions, and creates nothing", () => {
    open();
    fireEvent.change(name(), { target: { value: "Travel Planner" } });
    const instructions = screen.getByRole("textbox", { name: /^Instructions/ });
    expect(fireEvent.keyDown(instructions, { key: "Enter" })).toBe(true);
    expect(api.createAgent).not.toHaveBeenCalled();
  });

  it("creates once when Return is pressed again while the first create is in flight", async () => {
    let finish!: (card: { id: string }) => void;
    api.createAgent.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    open();
    fireEvent.change(name(), { target: { value: "Travel Planner" } });
    fireEvent.keyDown(name(), { key: "Enter" });
    fireEvent.keyDown(name(), { key: "Enter" });
    await act(async () => finish({ id: "new-agent" }));
    expect(api.createAgent).toHaveBeenCalledOnce();
  });

  it("keeps the dialog and the focus in Name when the create fails, and moves it once one succeeds", async () => {
    api.createAgent.mockRejectedValueOnce(new Error("The host is restarting."));
    const box = field();
    open();
    cleanup();
    render(<Sheet />);
    fireEvent.change(name(), { target: { value: "Travel Planner" } });
    name().focus();
    fireEvent.keyDown(name(), { key: "Enter" });
    expect(await screen.findByText("The host is restarting.")).toBeTruthy();
    await new Promise((settle) => requestAnimationFrame(settle));
    expect(screen.getByRole("dialog", { name: "New agent" })).toBeTruthy();
    expect(document.activeElement).toBe(name());
    fireEvent.keyDown(name(), { key: "Enter" });
    await waitFor(() => expect(document.activeElement).toBe(box));
    expect(api.createAgent).toHaveBeenCalledTimes(2);
  });

  it("creates nothing on the Return that confirms an input method's conversion", () => {
    open();
    fireEvent.change(name(), { target: { value: "旅行" } });
    fireEvent.keyDown(name(), { key: "Enter", isComposing: true });
    fireEvent.keyDown(name(), { key: "Enter", keyCode: 229 });
    expect(api.createAgent).not.toHaveBeenCalled();
  });

  it("leaves Command Return on a button to that button, so it never saves instead of deleting", () => {
    open(card());
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    fireEvent.keyDown(screen.getByRole("button", { name: /^Delete Counsel/ }), {
      key: "Enter",
      metaKey: true,
    });
    expect(api.updateAgent).not.toHaveBeenCalled();
  });

  it("saves an edited agent on Return in Name", async () => {
    open(card());
    fireEvent.change(name(), { target: { value: "Counsel Two" } });
    fireEvent.keyDown(name(), { key: "Enter" });
    await waitFor(() =>
      expect(api.updateAgent).toHaveBeenCalledWith(
        "agent-1",
        expect.objectContaining({ name: "Counsel Two" }),
      ),
    );
    expect(api.createAgent).not.toHaveBeenCalled();
  });

  it("creates on Return in Name and puts the focus in the new agent's composer", async () => {
    const box = field();
    open();
    cleanup();
    render(<Sheet />);
    fireEvent.change(name(), { target: { value: "Travel Planner" } });
    fireEvent.keyDown(name(), { key: "Enter" });
    await waitFor(() =>
      expect(api.createAgent).toHaveBeenCalledWith(
        expect.objectContaining({ name: "Travel Planner" }),
      ),
    );
    await waitFor(() => expect(document.activeElement).toBe(box));
  });

  it("creates on Command Return or Control Return from any field", async () => {
    open();
    fireEvent.change(name(), { target: { value: "Travel Planner" } });
    const skills = screen.getByRole("textbox", { name: /^Skills/ });
    fireEvent.keyDown(skills, { key: "Enter", metaKey: true });
    await waitFor(() => expect(api.createAgent).toHaveBeenCalledOnce());
    vi.clearAllMocks();
    open();
    fireEvent.change(screen.getAllByRole("textbox", { name: /^Name/ }).at(-1)!, {
      target: { value: "Chef" },
    });
    fireEvent.keyDown(screen.getAllByRole("textbox", { name: /^Instructions/ }).at(-1)!, {
      key: "Enter",
      ctrlKey: true,
    });
    await waitFor(() =>
      expect(api.createAgent).toHaveBeenCalledWith(expect.objectContaining({ name: "Chef" })),
    );
  });

  it("keeps the optional parts folded for a new agent and open for one being edited", () => {
    const view = open();
    const more = () =>
      screen.getByText("Instructions, skills and model").closest("details") as HTMLDetailsElement;
    expect(more().open).toBe(false);
    expect(more().contains(screen.getByRole("textbox", { name: /^Instructions/ }))).toBe(true);
    expect(more().contains(name())).toBe(false);
    view.unmount();
    open(card());
    expect(more().open).toBe(true);
  });

  it("offers Color and Character as one choice each, saved with the agent", async () => {
    open();
    const colors = screen.getAllByRole("radio").filter((radio) => radio.closest(".swatches"));
    const characters = screen.getAllByRole("radio").filter((radio) => radio.closest(".picker"));
    expect(new Set(colors.map((radio) => (radio as HTMLInputElement).name)).size).toBe(1);
    expect(new Set(characters.map((radio) => (radio as HTMLInputElement).name)).size).toBe(1);
    expect(colors[0]!.getAttribute("name")).not.toBe(characters[0]!.getAttribute("name"));
    expect(colors.filter((radio) => (radio as HTMLInputElement).checked)).toHaveLength(1);
    expect(characters.filter((radio) => (radio as HTMLInputElement).checked)).toHaveLength(1);
    fireEvent.click(colors.at(-1)!);
    fireEvent.click(characters.at(-1)!);
    fireEvent.change(name(), { target: { value: "Travel Planner" } });
    fireEvent.click(screen.getByRole("button", { name: "Create agent" }));
    await waitFor(() => expect(api.createAgent).toHaveBeenCalledOnce());
    const sent = (api.createAgent.mock.calls[0] as unknown[])[0] as AgentDraft;
    expect(sent.color).toBe(
      (colors.at(-1)!.closest(".swatch") as HTMLElement).style.getPropertyValue("--swatch"),
    );
    expect(sent.avatar).not.toBe("");
    expect((characters.at(-1) as HTMLInputElement).checked).toBe(true);
  });
});
