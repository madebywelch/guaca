import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "../lib/store";
import type { AgentCard, AgentDraft, Group, Settings } from "../lib/types";
import { AgentEditor } from "./AgentEditor";

interface FetchModelsOptions {
  groupId?: string;
}

const fetchModels = vi.fn<(options?: FetchModelsOptions) => Promise<string[]>>();
const agentNotes = vi.fn<(id: string) => Promise<string>>();
const updateAgent = vi.fn<(id: string, draft: AgentDraft) => Promise<AgentCard>>();
const onClose = vi.fn();

vi.mock("../lib/ipc", () => ({
  api: {
    agentNotes: (id: string) => agentNotes(id),
    fetchModels: (options?: FetchModelsOptions) => fetchModels(options),
    updateAgent: (id: string, draft: AgentDraft) => updateAgent(id, draft),
  },
}));

vi.mock("./GrantList", () => ({ GrantList: () => null }));
vi.mock("./RoutineList", () => ({ RoutineList: () => null }));
vi.mock("./SigninList", () => ({ SigninList: () => null }));

const GROUP_ID = "00000000-0000-4000-8000-000000000001";

function group(): Group {
  return {
    id: GROUP_ID,
    name: "Everyone",
    agentCount: 1,
    createdAt: 1,
    baseUrl: null,
    defaultModel: null,
    apiKeySet: false,
    apiKeyHint: "",
  };
}

function agent(): AgentCard {
  return {
    id: "00000000-0000-4000-8000-000000000002",
    groupId: GROUP_ID,
    name: "Researcher",
    avatar: "avocado",
    color: "#4e6b16",
    model: "deepseek-v4-flash-0731",
    systemPrompt: "Research things.",
    skills: ["research"],
    sandboxId: null,
    lifecycle: "active",
    version: 1,
    createdAt: 1,
    updatedAt: 1,
  };
}

function settings(): Settings {
  return {
    operatorName: "",
    baseUrl: "https://api.superagency.club/v1",
    defaultModel: "deepseek-v4-flash-0731",
    apiKeySet: true,
    apiKeyHint: "...9999",
    e2bKeySet: false,
    e2bKeyHint: "",
    computerIdleMinutes: 15,
    requestTimeoutSecs: 120,
    limits: {
      maxHops: 8,
      maxStepsPerRun: 60,
      maxFanoutPerCall: 8,
      maxSendsPerPair: 6,
      maxToolRounds: 24,
    },
  };
}

describe("AgentEditor model catalogue", () => {
  beforeEach(() => {
    fetchModels.mockReset();
    agentNotes.mockReset();
    updateAgent.mockReset();
    onClose.mockReset();
    agentNotes.mockResolvedValue("");
    updateAgent.mockResolvedValue(agent());
    useStore.setState({ agents: [agent()], groups: [group()], settings: settings() });
  });

  it("keeps the editable model field and explains a fetch failure", async () => {
    fetchModels.mockRejectedValue({ kind: "inference", message: "API key rejected" });
    render(<AgentEditor agent={agent()} onClose={onClose} />);

    fireEvent.click(screen.getByRole("button", { name: "Fetch models" }));

    expect((await screen.findByRole("alert")).textContent).toContain("API key rejected");
    expect(screen.getByRole("textbox", { name: "Model" })).toBeTruthy();
    expect(fetchModels).toHaveBeenCalledWith({ groupId: GROUP_ID });
  });

  it("fetches for the worker's group and saves a switched model", async () => {
    fetchModels.mockResolvedValue(["deepseek-v4-flash-0731", "qwen3.8-27b"]);
    render(<AgentEditor agent={agent()} onClose={onClose} />);

    fireEvent.click(screen.getByRole("button", { name: "Fetch models" }));

    const dropdown = await screen.findByRole<HTMLSelectElement>("combobox", { name: "Model" });
    expect(dropdown.value).toBe("deepseek-v4-flash-0731");
    expect([...dropdown.options].map((option) => option.value)).toEqual([
      "deepseek-v4-flash-0731",
      "qwen3.8-27b",
    ]);

    fireEvent.change(dropdown, { target: { value: "qwen3.8-27b" } });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateAgent).toHaveBeenCalled());
    expect(updateAgent.mock.calls[0]?.[1]).toMatchObject({ model: "qwen3.8-27b" });
    expect(onClose).toHaveBeenCalled();
  });
});
