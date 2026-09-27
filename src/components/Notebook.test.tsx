import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { NotebookEntry } from "../lib/types";

const agentNotebook = vi.fn<(id: string) => Promise<NotebookEntry[]>>();
const readNotebook = vi.fn<(id: string, path: string) => Promise<string>>();
const deleteNotebookFile = vi.fn<(id: string, path: string) => Promise<boolean>>();
vi.mock("../lib/ipc", () => ({
  api: {
    agentNotebook: (id: string) => agentNotebook(id),
    readNotebook: (id: string, path: string) => readNotebook(id, path),
    deleteNotebookFile: (id: string, path: string) => deleteNotebookFile(id, path),
  },
}));

import { Notebook } from "./Notebook";

const log: NotebookEntry = { path: "log.md", chars: 20, updatedAt: Date.now() };

describe("Notebook", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useStore.setState({ notebookVersion: {} });
    agentNotebook.mockResolvedValue([log]);
  });

  it("lists what the agent kept, opens a file, and takes one away", async () => {
    readNotebook.mockResolvedValue("- tried the API\n");
    deleteNotebookFile.mockResolvedValue(true);
    render(<Notebook agentId="a1" />);
    fireEvent.click(await screen.findByRole("button", { name: "log.md" }));
    expect(await screen.findByText(/tried the API/)).toBeTruthy();
    expect(readNotebook).toHaveBeenCalledWith("a1", "log.md");

    agentNotebook.mockResolvedValue([]);
    fireEvent.click(screen.getByRole("button", { name: "Delete log.md" }));
    await waitFor(() => expect(deleteNotebookFile).toHaveBeenCalledWith("a1", "log.md"));
    expect(await screen.findByText(/Empty\./)).toBeTruthy();
  });

  it("redraws when the agent writes mid-turn", async () => {
    agentNotebook.mockResolvedValue([]);
    render(<Notebook agentId="a1" />);
    await screen.findByText(/Empty\./);
    agentNotebook.mockResolvedValue([log]);
    act(() => useStore.getState().applyEvent({ type: "notebookChanged", agentId: "a1" }));
    expect(await screen.findByRole("button", { name: "log.md" })).toBeTruthy();
  });
});
