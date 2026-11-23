import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard, AgentDraft, Group } from "../lib/types";
import { aGroup } from "../test-fixtures";

const KITCHEN = "00000000-0000-4000-8000-000000000001";
const GARDEN = "00000000-0000-4000-8000-000000000002";

const hireAgents = vi.fn<(groupId: string, drafts: AgentDraft[]) => Promise<AgentCard[]>>(
  async () => [],
);

vi.mock("../lib/ipc", () => ({
  api: {
    hireAgents: (groupId: string, drafts: AgentDraft[]) => hireAgents(groupId, drafts),
    listAgents: async () => [],
    listGroups: async () => [],
    channelMessages: async () => [],
    conversationFlow: async () => [],
  },
}));

const { Cafeteria } = await import("./Cafeteria");
const { useStore } = await import("../lib/store");
const { HIREABLE, STARTER_CREW } = await import("../lib/cafeteria");

function group(id: string, name: string): Group {
  return aGroup({ id, name, agentCount: 0, createdAt: 1 });
}

function agent(name: string, groupId = KITCHEN): AgentCard {
  return {
    id: `id-${name}`,
    groupId,
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    readsWeb: true,
    browserConsent: "open",
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "",
    subscriptionModel: "",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 1,
    updatedAt: 1,
    discardedAt: null,
  };
}

const onClose = vi.fn();

interface Standing {
  railGroup?: string | null;
  selected?: string | null;
}

function open(
  groups: Group[] = [group(KITCHEN, "Kitchen")],
  agents: AgentCard[] = [],
  standing: Standing = {},
) {
  useStore.setState({
    agents,
    groups,
    activity: {},
    lastActive: {},
    selected: standing.selected ?? null,
    railGroup: standing.railGroup ?? null,
  });
  return render(<Cafeteria onClose={onClose} />);
}

function hireButton(): HTMLButtonElement {
  return screen.getByRole("button", { name: /^(hire \d+ into|nobody picked|hiring)/i });
}

function tile(name: string): HTMLElement {
  const heading = screen.getByText(name);
  const button = heading.closest("button");
  if (!button) throw new Error(`no hireable tile for ${name}`);
  return button;
}

beforeEach(() => {
  vi.clearAllMocks();
  hireAgents.mockResolvedValue([]);
});

describe("picking", () => {
  it("offers every preset in the catalog", () => {
    open();
    for (const preset of HIREABLE) {
      expect(tile(preset.name), preset.id).toBeTruthy();
    }
  });

  it("hires nobody until somebody is picked", () => {
    open();
    const button = screen.getByRole("button", { name: /nobody picked/i });
    expect((button as HTMLButtonElement).disabled).toBe(true);
  });

  it("toggles a tile on and back off", () => {
    open();
    const manager = tile("Chief of Staff");
    expect(manager.getAttribute("aria-pressed")).toBe("false");

    fireEvent.click(manager);
    expect(manager.getAttribute("aria-pressed")).toBe("true");
    expect(hireButton().disabled).toBe(false);

    fireEvent.click(manager);
    expect(manager.getAttribute("aria-pressed")).toBe("false");
  });

  it("names the group on the button, so a hire says where it is going", () => {
    open([group(KITCHEN, "Kitchen"), group(GARDEN, "Garden")]);
    fireEvent.click(tile("Chief of Staff"));
    expect(screen.getByRole("button", { name: /hire 1 into kitchen/i })).toBeTruthy();
  });

  it("sends only the presets that were picked", async () => {
    open();
    fireEvent.click(tile("Chief of Staff"));
    fireEvent.click(tile("Market Researcher"));
    fireEvent.click(screen.getByRole("button", { name: /hire 2 into kitchen/i }));

    await waitFor(() => expect(hireAgents).toHaveBeenCalledTimes(1));
    const [groupId, drafts] = hireAgents.mock.calls[0]!;
    expect(groupId).toBe(KITCHEN);
    expect(drafts.map((draft) => draft.name)).toEqual(["Chief of Staff", "Market Researcher"]);
    expect(drafts.every((draft) => draft.model === "")).toBe(true);
  });

  it("hires into the group the operator chose, not the first one", async () => {
    open([group(KITCHEN, "Kitchen"), group(GARDEN, "Garden")]);
    fireEvent.change(screen.getByLabelText(/hire into/i), { target: { value: GARDEN } });
    fireEvent.click(tile("Chief of Staff"));
    fireEvent.click(screen.getByRole("button", { name: /hire 1 into garden/i }));

    await waitFor(() => expect(hireAgents).toHaveBeenCalledTimes(1));
    expect(hireAgents.mock.calls[0]![0]).toBe(GARDEN);
  });

  it("does not ask which group when there is only one", () => {
    open();
    expect(screen.queryByLabelText(/hire into/i)).toBeNull();
  });

  it("clears a selection without hiring anything", () => {
    open();
    fireEvent.click(tile("Chief of Staff"));
    fireEvent.click(screen.getByRole("button", { name: /^clear$/i }));
    expect(tile("Chief of Staff").getAttribute("aria-pressed")).toBe("false");
    expect(hireAgents).not.toHaveBeenCalled();
  });
});

describe("the starter crew", () => {
  it("picks a crew for an empty group", () => {
    open();
    fireEvent.click(screen.getByRole("button", { name: /starter group/i }));
    expect(
      screen.getByRole("button", { name: new RegExp(`hire ${STARTER_CREW.length} into`, "i") }),
    ).toBeTruthy();
  });

  it("is not offered to a group that already has agents in it", () => {
    open([group(KITCHEN, "Kitchen")], [agent("Chief of Staff")]);
    expect(screen.queryByRole("button", { name: /starter group/i })).toBeNull();
  });
});

describe("who is already here", () => {
  it("marks a preset whose name the group already holds", () => {
    open([group(KITCHEN, "Kitchen")], [agent("Chief of Staff")]);
    expect(tile("Chief of Staff").textContent).toContain("on staff");
    expect(tile("Market Researcher").textContent).not.toContain("on staff");
  });

  it("marks nobody from another group", () => {
    open([group(KITCHEN, "Kitchen"), group(GARDEN, "Garden")], [agent("Chief of Staff", GARDEN)]);
    expect(tile("Chief of Staff").textContent).not.toContain("on staff");
  });

  it("does not count somebody in the compost", () => {
    open(
      [group(KITCHEN, "Kitchen")],
      [{ ...agent("Executive Assistant"), lifecycle: "terminated", discardedAt: 1_000 }],
    );
    expect(tile("Executive Assistant").textContent).not.toContain("on staff");
    expect(screen.getByRole("button", { name: /starter group/i })).toBeTruthy();
  });

  it("still lets a second one be hired", async () => {
    open([group(KITCHEN, "Kitchen")], [agent("Chief of Staff")]);
    fireEvent.click(tile("Chief of Staff"));
    fireEvent.click(screen.getByRole("button", { name: /hire 1 into kitchen/i }));
    await waitFor(() => expect(hireAgents).toHaveBeenCalledTimes(1));
  });
});

describe("when a hire is refused", () => {
  it("says why and leaves the selection alone", async () => {
    hireAgents.mockRejectedValue({ kind: "storage", message: "the database is locked" });
    open();
    fireEvent.click(tile("Chief of Staff"));
    fireEvent.click(screen.getByRole("button", { name: /hire 1 into kitchen/i }));

    await waitFor(() => expect(screen.getByText(/the database is locked/i)).toBeTruthy());
    expect(onClose).not.toHaveBeenCalled();
    expect(tile("Chief of Staff").getAttribute("aria-pressed")).toBe("true");
    expect(hireButton().disabled).toBe(false);
  });
});

describe("where a hire lands", () => {
  it("hires into the crew the rail is inside", async () => {
    open(
      [group(KITCHEN, "Kitchen"), group(GARDEN, "Garden")],
      [agent("Executive Assistant", KITCHEN)],
      { railGroup: GARDEN },
    );

    expect(tile("Executive Assistant").textContent).not.toContain("on staff");
    expect(screen.getByRole("button", { name: /starter group/i })).toBeTruthy();

    fireEvent.click(tile("Executive Assistant"));
    fireEvent.click(screen.getByRole("button", { name: /hire 1 into garden/i }));
    await waitFor(() => expect(hireAgents).toHaveBeenCalledTimes(1));
    expect(hireAgents.mock.calls[0]![0]).toBe(GARDEN);
  });

  it("follows the open channel when the rail is in the overview", () => {
    open(
      [group(KITCHEN, "Kitchen"), group(GARDEN, "Garden")],
      [agent("Executive Assistant", KITCHEN), agent("Paralegal", GARDEN)],
      { selected: "id-Paralegal" },
    );
    expect(tile("Executive Assistant").textContent).not.toContain("on staff");
    expect(screen.getByRole("button", { name: /hire 0 into garden|nobody picked/i })).toBeTruthy();
    fireEvent.click(tile("Executive Assistant"));
    expect(screen.getByRole("button", { name: /hire 1 into garden/i })).toBeTruthy();
  });

  it("falls back to the first group with nothing open", () => {
    open([group(KITCHEN, "Kitchen"), group(GARDEN, "Garden")], [], {});
    fireEvent.click(tile("Executive Assistant"));
    expect(screen.getByRole("button", { name: /hire 1 into kitchen/i })).toBeTruthy();
  });

  it("keeps the crew it opened on with no channel open at all", () => {
    open([group(KITCHEN, "Kitchen"), group(GARDEN, "Garden")], [], {
      railGroup: GARDEN,
      selected: null,
    });
    fireEvent.click(tile("Executive Assistant"));
    expect(screen.getByRole("button", { name: /hire 1 into garden/i })).toBeTruthy();
  });
});
