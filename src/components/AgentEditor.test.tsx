import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { useStore } from "../lib/store";
import type { AgentCard, Group, Settings } from "../lib/types";
import { aGroup } from "../test-fixtures";
import { AgentEditor } from "./AgentEditor";

const api = vi.hoisted(() => ({
  subscriptionModels: vi.fn(async () => ["chat-default", "chat-specialist"]),
  updateAgent: vi.fn(async () => {}),
  createAgent: vi.fn(async () => ({ id: "new-agent" })),
  rankedModels: vi.fn(async () => []),
}));
vi.mock("../lib/ipc", () => ({ api }));
vi.mock("./AgentRepositories", () => ({ AgentRepositories: () => null }));
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
  baseUrl: "https://openrouter.ai/api/v1",
  defaultModel: "anthropic/endpoint-model",
  provider: "compatible",
  subscriptionModel: "chat-default",
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
function card(model = ""): AgentCard {
  return {
    id: "agent-1",
    groupId: crew.id,
    name: "Counsel",
    avatar: "avocado",
    color: "#c7d96b",
    model,
    systemPrompt: "",
    skills: [],
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    browserConsent: "open",
    repositoryId: null,
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
  api.subscriptionModels.mockResolvedValue(["chat-default", "chat-specialist"]);
});

it("keeps a legacy endpoint override visible and explains it after the account list loads", async () => {
  open(card("anthropic/endpoint-model"));
  expect(modelSelect().value).toBe("anthropic/endpoint-model");
  expect((await screen.findByRole("status")).textContent).toContain(
    "not in your account's current ChatGPT model list",
  );
  fireEvent.change(modelSelect(), { target: { value: "" } });
  await save();
  expect(api.updateAgent).toHaveBeenCalledWith("agent-1", expect.objectContaining({ model: "" }));
});

it("keeps inheritance and the saved override usable when discovery fails", async () => {
  api.subscriptionModels.mockRejectedValue(new Error("offline"));
  open(card("saved-model"));
  expect((await screen.findByRole("status")).textContent).toContain(
    "Could not load current ChatGPT models",
  );
  expect(modelSelect().value).toBe("saved-model");
  expect(screen.getByRole("option", { name: "Use group default · crew-default" })).toBeTruthy();
  await save();
  expect(api.updateAgent).toHaveBeenCalledWith(
    "agent-1",
    expect.objectContaining({ model: "saved-model" }),
  );
});

it("uses the group's ChatGPT provider and saves an account model for just this agent", async () => {
  open(card());
  expect(screen.getByText("Provider · ChatGPT subscription")).toBeTruthy();
  expect(screen.getByText(/Inherited from Research/)).toBeTruthy();
  expect(modelSelect().value).toBe("");
  expect(screen.getByRole("option", { name: "Use group default · crew-default" })).toBeTruthy();
  await screen.findByRole("option", { name: "chat-specialist" });
  fireEvent.change(modelSelect(), { target: { value: "chat-specialist" } });
  await save();
  expect(api.updateAgent).toHaveBeenCalledWith(
    "agent-1",
    expect.objectContaining({ model: "chat-specialist" }),
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
    expect(api.createAgent).toHaveBeenCalledWith(expect.objectContaining({ model: "" })),
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

it("updates the model control when moving groups and preserves an explicit override", async () => {
  open(card("chat-specialist"), [crew, aGroup({ id: "endpoint", name: "Endpoint" })]);
  await screen.findByRole("option", { name: "chat-specialist" });
  fireEvent.change(screen.getByRole("combobox", { name: /^Group/ }), {
    target: { value: "endpoint" },
  });
  expect(screen.getByText("Provider · OpenRouter")).toBeTruthy();
  expect((screen.getByRole("textbox", { name: /^Model/ }) as HTMLInputElement).value).toBe(
    "chat-specialist",
  );
  fireEvent.change(screen.getByRole("combobox", { name: /^Group/ }), {
    target: { value: crew.id },
  });
  expect(modelSelect().value).toBe("chat-specialist");
  await screen.findByRole("option", { name: "chat-default" });
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
