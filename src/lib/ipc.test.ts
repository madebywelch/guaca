import { expect, it, vi } from "vitest";

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
  // Every window is hosted now. Choosing the branch by `hosted` sent the
  // desktop to the browser's DOM listeners, where Tauri's drop never arrives.
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
