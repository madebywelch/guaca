import { beforeEach, describe, expect, it, vi } from "vitest";

const { adopt, invoke, render, host } = vi.hoisted(() => ({
  adopt: vi.fn(),
  invoke: vi.fn(),
  render: vi.fn(),
  host: vi.fn(),
}));

vi.mock("./page", () => ({}));
vi.mock("react-dom/client", () => ({ default: { createRoot: () => ({ render }) } }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("./lib/ipc", () => ({ api: { menubarHost: host } }));
vi.mock("./lib/transport", () => ({ adoptRemote: adopt, UNAUTHORIZED_EVENT: "unauthorized" }));
vi.mock("./components/ErrorBoundary", () => ({ ErrorBoundary: () => null }));
vi.mock("./components/Markdown", () => ({ Roster: { Provider: () => null } }));
vi.mock("./components/MenubarPanel", () => ({ MenubarPanel: () => null }));
vi.mock("./avatars/cast", () => ({ CastContext: { Provider: () => null } }));
vi.mock("./lib/runtime", () => ({ useRuntime: () => false }));
vi.mock("./lib/store", () => ({ useStore: () => [] }));

describe("panel boot", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    document.body.innerHTML = '<div id="root"></div>';
  });

  it("adopts before acknowledging and does not render a stale generation", async () => {
    const assignment = { origin: "http://localhost", token: "token", generation: 3 };
    host.mockResolvedValue(assignment);
    invoke.mockResolvedValue(false);
    await import("./menubar");
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(adopt).toHaveBeenCalledWith(assignment);
    expect(adopt.mock.invocationCallOrder[0]).toBeLessThan(invoke.mock.invocationCallOrder[0]!);
    expect(invoke).toHaveBeenCalledWith("report_host", { host: assignment, generation: 3 });
    expect(render).not.toHaveBeenCalled();
  });
});
