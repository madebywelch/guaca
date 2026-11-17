import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard } from "../lib/types";
import { aGroup, DEFAULT_GROUP } from "../test-fixtures";

const restoreAgent = vi.fn<(id: string) => Promise<AgentCard>>();
const purgeAgent = vi.fn<(id: string) => Promise<void>>(async () => {});
const select = vi.fn(async () => {});

vi.mock("../lib/ipc", () => ({
  api: {
    restoreAgent: (id: string) => restoreAgent(id),
    purgeAgent: (id: string) => purgeAgent(id),
    listAgents: async () => [],
    listGroups: async () => [],
    channelMessages: async () => [],
    conversationFlow: async () => [],
  },
}));

const { Compost } = await import("./Compost");
const { useStore } = await import("../lib/store");
const { COMPOST_DAYS } = await import("../lib/compost");

const DAY = 24 * 60 * 60 * 1000;

function agent(name: string, discardedAt: number | null): AgentCard {
  return {
    id: `id-${name}`,
    groupId: DEFAULT_GROUP,
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    browserConsent: "open",
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "",
    subscriptionModel: "",
    systemPrompt: "",
    skills: [],
    lifecycle: discardedAt === null ? "active" : "terminated",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    discardedAt,
  };
}

function open(agents: AgentCard[]) {
  useStore.setState({
    agents,
    groups: [aGroup({ name: "Kitchen" })],
    activity: {},
    lastActive: {},
    selected: null,
    select,
  });
  return render(<Compost />);
}

const thrownOut = (name: string, daysAgo: number) => agent(name, Date.now() - daysAgo * DAY);

beforeEach(() => {
  vi.clearAllMocks();
  restoreAgent.mockResolvedValue(agent("Scribe", null));
});

describe("the compost", () => {
  it("lists whoever is in it, and nobody who is not", () => {
    open([thrownOut("Scribe", 2), agent("Manager", null)]);

    expect(screen.getByText("Scribe")).toBeTruthy();
    expect(screen.queryByText("Manager")).toBeNull();
  });

  it("says what a restore would get back, once, at the top", () => {
    const { container } = open([thrownOut("Scribe", 2), thrownOut("Paralegal", 4)]);

    expect(
      [...container.querySelectorAll(".settings__lede")].filter((node) =>
        /memory, their working notes, their schedule and their sign-ins/i.test(
          node.textContent ?? "",
        ),
      ),
    ).toHaveLength(1);
  });

  it("puts one back on a single click, and opens it", async () => {
    restoreAgent.mockResolvedValue(agent("Scribe copy", null));
    open([thrownOut("Scribe", 2)]);

    fireEvent.click(screen.getByRole("button", { name: /put back/i }));

    await waitFor(() => expect(restoreAgent).toHaveBeenCalledWith("id-Scribe"));
    await waitFor(() => expect(select).toHaveBeenCalledWith("id-Scribe copy"));
  });

  it("asks twice before it deletes a memory, and the second wording says so", async () => {
    open([thrownOut("Scribe", 2)]);

    fireEvent.click(screen.getByRole("button", { name: /delete now/i }));
    expect(purgeAgent).not.toHaveBeenCalled();

    const confirm = screen.getByRole("button", { name: /delete the memory too/i });
    fireEvent.click(confirm);
    await waitFor(() => expect(purgeAgent).toHaveBeenCalledWith("id-Scribe"));
  });

  it("lets somebody out of the confirmation without deleting anything", () => {
    open([thrownOut("Scribe", 2)]);

    fireEvent.click(screen.getByRole("button", { name: /delete now/i }));
    fireEvent.click(screen.getByRole("button", { name: /keep it/i }));

    expect(purgeAgent).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: /put back/i })).toBeTruthy();
  });

  it("shows an empty state without closing settings", () => {
    open([]);
    expect(screen.getByText("No deleted agents.")).toBeTruthy();
  });

  it("draws the wait the runtime actually enforces", () => {
    const rust = readFileSync(resolve(__dirname, "../../src-tauri/src/domain/agent.rs"), "utf8");
    const declared = rust.match(/pub const COMPOST_DAYS: i64 = ([\d_]+)/);
    expect(
      declared,
      "COMPOST_DAYS has been renamed or moved out of domain/agent.rs",
    ).not.toBeNull();
    expect(Number(declared![1]!.replaceAll("_", ""))).toBe(COMPOST_DAYS);
  });

  it("marks the last week and leaves the rest of the wait unmarked", () => {
    const { container } = open([thrownOut("Going", COMPOST_DAYS - 2), thrownOut("Waiting", 1)]);

    const marked = [...container.querySelectorAll(".compost__clock")].map((node) =>
      node.getAttribute("data-soon"),
    );
    expect(marked).toEqual([null, "true"]);
  });
});
