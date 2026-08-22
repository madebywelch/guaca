import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Group } from "../lib/types";

/**
 * The group dialog, over a mocked runtime.
 *
 * The risk on this surface is the delete button, which is two different calls
 * behind one word. An empty group loses a row in a table; a group with a crew
 * in it loses four agents, their computers and their browsers, and none of that
 * comes back. Which call a click makes, and what the operator is told before
 * they make it, is what is asserted here.
 */

const deleteGroup = vi.fn<(id: string) => Promise<void>>();
const disbandGroup = vi.fn<(id: string) => Promise<void>>();
const clearGroup = vi.fn();
const updateGroup = vi.fn();
const createGroup = vi.fn();

vi.mock("../lib/ipc", () => ({
  api: {
    deleteGroup: (id: string) => deleteGroup(id),
    disbandGroup: (id: string) => disbandGroup(id),
    clearGroup: (id: string) => clearGroup(id),
    updateGroup: (id: string, draft: unknown) => updateGroup(id, draft),
    createGroup: (draft: unknown) => createGroup(draft),
    // The roster refresh the dialog fires after any of the above, and the one
    // call CredentialList makes on mount.
    listAgents: () => Promise.resolve([]),
    listGroups: () => Promise.resolve([]),
    groupConnectors: () => Promise.resolve([]),
  },
}));

const { GroupEditor } = await import("./GroupEditor");

const ID = "00000000-0000-4000-8000-000000000001";

function group(over: Partial<Group> = {}): Group {
  return {
    id: ID,
    name: "Research",
    agentCount: 0,
    createdAt: 0,
    baseUrl: null,
    defaultModel: null,
    apiKeySet: false,
    apiKeyHint: "",
    ...over,
  };
}

const onClose = vi.fn();

/**
 * The dialog, mounted and settled.
 *
 * `CredentialList` reads the group's accounts on mount, so letting that land
 * before the first click keeps every assertion on a tree that has stopped
 * moving.
 */
async function open(on?: Group) {
  const view = render(<GroupEditor group={on} onClose={onClose} />);
  await act(async () => {});
  return view;
}

describe("GroupEditor", () => {
  beforeEach(() => {
    deleteGroup.mockReset();
    disbandGroup.mockReset();
    clearGroup.mockReset();
    onClose.mockReset();
    deleteGroup.mockResolvedValue(undefined);
    disbandGroup.mockResolvedValue(undefined);
  });

  it("takes two clicks to delete anything", async () => {
    await open(group());
    fireEvent.click(screen.getByText("Delete"));
    expect(deleteGroup).not.toHaveBeenCalled();
    expect(disbandGroup).not.toHaveBeenCalled();
  });

  it("deletes an empty group without disbanding anybody", async () => {
    await open(group());
    fireEvent.click(screen.getByText("Delete"));
    fireEvent.click(screen.getByText("Delete Research"));

    await waitFor(() => expect(deleteGroup).toHaveBeenCalledWith(ID));
    expect(disbandGroup).not.toHaveBeenCalled();
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("says how many agents go, and what goes with them", async () => {
    // The count is on the button because that is what the operator is about to
    // press. The machines are in the banner because a count does not say that
    // anything was rented, and destroying a computer is the half of this that
    // cannot be undone.
    await open(group({ agentCount: 4 }));
    fireEvent.click(screen.getByText("Delete"));

    expect(screen.getByText("Delete Research and 4 agents")).toBeTruthy();
    expect(screen.getByText(/computers, browsers/)).toBeTruthy();
  });

  it("counts one agent as one agent", async () => {
    await open(group({ agentCount: 1 }));
    fireEvent.click(screen.getByText("Delete"));
    expect(screen.getByText("Delete Research and 1 agent")).toBeTruthy();
  });

  it("disbands a group that still holds a crew", async () => {
    await open(group({ agentCount: 4 }));
    fireEvent.click(screen.getByText("Delete"));
    fireEvent.click(screen.getByText("Delete Research and 4 agents"));

    await waitFor(() => expect(disbandGroup).toHaveBeenCalledWith(ID));
    // The plain delete would be refused for a group with agents in it, and
    // refusing is all it could do: nothing about that error is what was asked
    // for here.
    expect(deleteGroup).not.toHaveBeenCalled();
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("keeps the crew when the second click is Keep", async () => {
    await open(group({ agentCount: 4 }));
    fireEvent.click(screen.getByText("Delete"));
    fireEvent.click(screen.getByText("Keep"));

    expect(disbandGroup).not.toHaveBeenCalled();
    expect(screen.queryByText(/computers, browsers/)).toBeNull();
    expect(screen.getByText("Delete")).toBeTruthy();
  });

  it("leaves the dialog open on a refusal, with the reason from the runtime", async () => {
    // The first group cannot be deleted, because every agent has to be in one.
    // A dialog that closed on that would look like it had worked.
    disbandGroup.mockRejectedValue({
      kind: "groupNotEmpty",
      message: "every agent has to be in a group, so the first one cannot be deleted",
    });
    await open(group({ agentCount: 2 }));
    fireEvent.click(screen.getByText("Delete"));
    fireEvent.click(screen.getByText("Delete Research and 2 agents"));

    expect(await screen.findByText(/cannot be deleted/)).toBeTruthy();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("does not offer to reset a group it is already deleting", async () => {
    // Two destructive confirmations open at once is a click on the wrong one.
    await open(group({ agentCount: 2 }));
    expect(screen.getByText("Start fresh")).toBeTruthy();
    fireEvent.click(screen.getByText("Delete"));
    expect(screen.queryByText("Start fresh")).toBeNull();
  });

  it("offers nothing to delete on a group that does not exist yet", async () => {
    await open();
    expect(screen.queryByText("Delete")).toBeNull();
    expect(screen.queryByText("Start fresh")).toBeNull();
  });
});
