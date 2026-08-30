import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard } from "../lib/types";
import { aGroup, DEFAULT_GROUP } from "../test-fixtures";

/**
 * The compost, over a mocked store.
 *
 * What is worth checking here is what a click actually sends, because the two
 * buttons on a row do opposite things and one of them destroys a memory. The
 * arithmetic behind the clock is `lib/compost`'s, and is tested there.
 */

const restoreAgent = vi.fn<(id: string) => Promise<AgentCard>>();
const purgeAgent = vi.fn<(id: string) => Promise<void>>(async () => {});
const select = vi.fn(async () => {});

vi.mock("../lib/ipc", () => ({
  api: {
    restoreAgent: (id: string) => restoreAgent(id),
    purgeAgent: (id: string) => purgeAgent(id),
    listAgents: async () => [],
    listGroups: async () => [],
    listRepositories: async () => [],
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
    browserConsent: "open",
    repositoryId: null,
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "",
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

const onClose = vi.fn();

function open(agents: AgentCard[]) {
  useStore.setState({
    agents,
    groups: [aGroup({ name: "Kitchen" })],
    activity: {},
    lastActive: {},
    selected: null,
    select,
  });
  return render(<Compost onClose={onClose} />);
}

/** Deleted this many days ago. */
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
    // The one thing that is invisible everywhere else, on the one screen where
    // somebody is deciding whether they meant it. Once rather than per row:
    // the same sentence three times is wallpaper, and it would be read as
    // three rows of gray beside the one number that differs.
    const { container } = open([thrownOut("Scribe", 2), thrownOut("Paralegal", 4)]);

    expect(
      [...container.querySelectorAll(".dialog__lede")].filter((node) =>
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
    // Opened rather than only restored: the agent comes back paused and may
    // come back renamed, so nothing about it would otherwise draw attention.
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

  it("closes itself once there is nobody left in it", () => {
    // Emptying it is the one reason to be here, and finishing is the answer.
    // A panel that stays open drawing nothing is one the operator has to
    // dismiss for no reason.
    open([]);
    expect(onClose).toHaveBeenCalled();
  });

  it("draws the wait the runtime actually enforces", () => {
    // The one number in this file that is somebody else's. Read out of the
    // Rust rather than trusted: this panel tells the operator when their
    // agent's memory will be deleted, and nothing else in the build compares
    // the two. Both sides compile, both sides pass, and the only symptom is a
    // countdown that is not the one being counted down.
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

    // Newest first, so the one with weeks to go is drawn above the one that is
    // about to be swept. The mark is what says which is which.
    const marked = [...container.querySelectorAll(".compost__clock")].map((node) =>
      node.getAttribute("data-soon"),
    );
    expect(marked).toEqual([null, "true"]);
  });
});
