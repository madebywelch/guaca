import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { Group } from "../lib/types";
import { Sidebar } from "./Sidebar";

vi.mock("../lib/ipc", () => ({
  api: {
    listAgents: async () => [],
    listGroups: async () => [],
  },
}));

const MODEL = "anthropic/claude-opus-4-1-20250805";

function group(name: string, defaultModel: string | null): Group {
  return {
    id: "00000000-0000-4000-8000-000000000001",
    name,
    agentCount: 0,
    createdAt: 0,
    baseUrl: null,
    defaultModel,
    apiKeySet: false,
    apiKeyHint: "",
  };
}

function draw(g: Group) {
  useStore.setState({
    agents: [],
    groups: [g],
    activity: {},
    lastActive: {},
    usage: { [g.id]: { prompt: 900_000, completion: 900_000, calls: 400, cost: 123.45 } },
    pulse: {},
    pulses: [],
    selected: null,
  });
  return render(
    <Sidebar
      onNewAgent={vi.fn()}
      onEditAgent={vi.fn()}
      onNewGroup={vi.fn()}
      onEditGroup={vi.fn()}
      onOpenCafeteria={vi.fn()}
      onOpenSettings={vi.fn()}
      onOpenSearch={vi.fn()}
      onOpenMenu={vi.fn()}
    />,
  );
}

describe("group header", () => {
  beforeEach(() => {
    useStore.setState({ groups: [], agents: [] });
  });

  it("does not draw the pinned model in the rail at all", () => {
    // The rail is 15.5rem. A model id beside the name left one letter of
    // "everyone" and an ellipsis; on a line of its own it cost a whole row per
    // group to say something the gear already shows.
    const { container } = draw(group("everyone", MODEL));

    expect(container.querySelector(".rail__group-head")?.textContent).toContain("everyone");
    expect(screen.queryByText(MODEL)).toBeNull();
  });
});
