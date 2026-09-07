import { expect, it, vi } from "vitest";

import { api } from "./ipc";
import { invoke, invokeLocal } from "./transport";

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
