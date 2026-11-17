import { expect, it, vi } from "vitest";

import { anApproval } from "../test-fixtures";
import { api, onFileDrop } from "./ipc";
import { invoke, invokeLocal } from "./transport";

const tauriEvents = vi.hoisted(() => ({
  handlers: new Map<string, (message: { payload: unknown }) => void>(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  TauriEvent: { DRAG_ENTER: "enter", DRAG_LEAVE: "leave", DRAG_DROP: "drop" },
  listen: async (name: string, handler: (message: { payload: unknown }) => void) => {
    tauriEvents.handlers.set(name, handler);
    return () => tauriEvents.handlers.delete(name);
  },
}));

vi.mock("./transport", async (original) => ({
  ...(await original<typeof import("./transport")>()),
  invoke: vi.fn(),
  invokeLocal: vi.fn(),
  workspaceOrigin: () => "http://127.0.0.1:8787",
  token: () => "workspace-token",
}));

it("reads a pending request from a host that never says when it lapses as one with no known lapse", async () => {
  const { lapsesAt: _, ...older } = anApproval({ agentId: "dara", createdAt: 1_000 });
  vi.mocked(invoke).mockResolvedValue([older]);
  const [request] = await api.pendingApprovals();
  expect(invoke).toHaveBeenCalledWith("pending_approvals");
  expect(request).toEqual({ ...older, lapsesAt: null });
});

it("keeps the lapse a host sends with a pending request", async () => {
  vi.mocked(invoke).mockResolvedValue([anApproval({ agentId: "dara", lapsesAt: 5_000 })]);
  const [request] = await api.pendingApprovals();
  expect(request?.lapsesAt).toBe(5_000);
});

it("downloads through the native client using the connected backend", async () => {
  vi.mocked(invokeLocal).mockResolvedValue("/Downloads/brief.md");
  await expect(api.saveFile("a".repeat(64), "brief.md")).resolves.toBe("/Downloads/brief.md");
  expect(invokeLocal).toHaveBeenCalledWith("download_file", {
    origin: "http://127.0.0.1:8787",
    token: "workspace-token",
    digest: "a".repeat(64),
    name: "brief.md",
  });
  expect(invoke).not.toHaveBeenCalled();
});

it("forwards a file dropped on a desktop window to the host it shows", async () => {
  vi.mocked(invokeLocal).mockResolvedValue({ attached: [], refused: [] });
  const dropped = vi.fn();
  const stop = await onFileDrop({ dropped, over: () => {} });
  tauriEvents.handlers.get("drop")?.({ payload: { paths: ["/Users/me/brief.pdf"] } });
  expect(dropped).toHaveBeenCalledTimes(1);
  expect(invokeLocal).toHaveBeenCalledWith("forward_files", {
    origin: "http://127.0.0.1:8787",
    token: "workspace-token",
    paths: ["/Users/me/brief.pdf"],
  });
  expect(invoke).not.toHaveBeenCalled();
  stop();
});
