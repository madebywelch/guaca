import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { platform, mode, docker, update, reload } = vi.hoisted(() => ({
  platform: { desktop: true },
  mode: vi.fn(),
  docker: vi.fn(),
  update: vi.fn(),
  reload: vi.fn(),
}));
vi.mock("../lib/host", () => ({ hostMode: mode, localHost: { status: docker, update } }));
vi.mock("../lib/transport", () => ({
  get desktop() {
    return platform.desktop;
  },
  workspaceOrigin: () => "https://host.example",
  token: () => "secret",
  RECONNECTED: "guaca:reconnected",
  UNAUTHORIZED_EVENT: "guaca:unauthorized",
  restart: reload,
}));
vi.mock("../lib/build", () => ({ COMMIT: "aaaaaaa", VERSION: "0.1.0" }));
vi.mock("./HostSetup", () => ({ HostChoice: () => <div>Host choice</div> }));

import { HostMonitor, HostUpdateNotice, HostUpdatePanel } from "./HostUpdates";

const original = {
  service: "guacad",
  build: "aaaaaaa",
  version: "0.1.0",
  release: true,
  apiGeneration: 1,
};
const published = {
  automatic: true,
  checkedAt: "2026-09-10T00:00:00Z",
  error: null,
  latest: {
    schema: 1,
    version: "0.2.0",
    commit: "b".repeat(40),
    image: `ghcr.io/madebywelch/guaca/guacad@sha256:${"c".repeat(64)}`,
    apiGeneration: 1,
    clientMinimum: 1,
    clientMaximum: 1,
    notes: "https://github.com/madebywelch/guaca/releases/tag/v0.2.0",
  },
};
let health: Record<string, unknown>;
let release: unknown;
const fetched = vi.fn();
beforeEach(() => {
  vi.clearAllMocks();
  platform.desktop = true;
  mode.mockReturnValue("remote");
  health = { ...original };
  release = structuredClone(published);
  docker.mockResolvedValue({
    state: "running",
    message: "Ready",
    updateAvailable: true,
    origin: "https://host.example",
    targetImage: "guacad:new",
    targetVersion: "0.1.0",
  });
  fetched.mockImplementation((url: string) =>
    Promise.resolve(new Response(JSON.stringify(url.endsWith("/health") ? health : release))),
  );
  vi.stubGlobal("fetch", fetched);
  sessionStorage.clear();
});
afterEach(() => vi.unstubAllGlobals());
function mount() {
  return render(
    <HostMonitor>
      <div>Workspace mounted</div>
      <HostUpdateNotice onReview={() => {}} />
      <HostUpdatePanel />
    </HostMonitor>,
  );
}
describe("host updates in either client", () => {
  it("checks before mounting and detects an equally old browser and backend", async () => {
    platform.desktop = false;
    mount();
    expect(screen.queryByText("Workspace mounted")).toBeNull();
    await screen.findByText("Host update available.");
    expect(screen.getByText("Workspace mounted")).toBeTruthy();
    expect(docker).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "Back up and update host" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "View update instructions" }));
    expect(screen.getByText(/This connection does not manage/)).toBeTruthy();
  });
  it("blocks an incompatible API while leaving host instructions available", async () => {
    health.apiGeneration = 2;
    mount();
    await screen.findByText("This Guaca needs an update");
    expect(screen.queryByText("Workspace mounted")).toBeNull();
    expect(screen.getByRole("button", { name: "View update instructions" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Continue with unverified host" })).toBeNull();
  });
  it("labels legacy hosts unverified and lets the operator use their existing API", async () => {
    delete health.apiGeneration;
    delete health.release;
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Continue with unverified host" }));
    await screen.findByText("Workspace mounted");
    expect(screen.queryByText("Host update available.")).toBeNull();
  });
  it("does not block a compatible workspace on release-service failure", async () => {
    release = { ...published, error: "Release service is offline." };
    mount();
    await screen.findByText("Workspace mounted");
    await screen.findByText("Release service is offline.");
    expect(screen.getByText(/latest check failed/)).toBeTruthy();
  });
  it("only updates the selected container on an explicit click", async () => {
    mode.mockReturnValue("local");
    update.mockResolvedValue({ origin: "https://host.example", token: "secret" });
    mount();
    const button = await screen.findByRole("button", { name: "Back up and update host" });
    expect(update).not.toHaveBeenCalled();
    fireEvent.click(button);
    await screen.findByText(/Host updated. Review/);
    expect(update).toHaveBeenCalledWith("https://host.example");
    expect(reload).not.toHaveBeenCalled();
  });
  it("does not offer to replace a different local container", async () => {
    mode.mockReturnValue("local");
    docker.mockResolvedValue({
      state: "running",
      updateAvailable: true,
      origin: "http://127.0.0.1:5000",
    });
    mount();
    await screen.findByText("A newer host release is available.");
    expect(screen.queryByRole("button", { name: "Back up and update host" })).toBeNull();
  });
  it("keeps the mounted draft when compatibility changes after reconnect", async () => {
    render(
      <HostMonitor>
        <input aria-label="Draft" defaultValue="Unsent words" />
      </HostMonitor>,
    );
    const input = await screen.findByLabelText("Draft");
    health.apiGeneration = 2;
    await act(async () => window.dispatchEvent(new Event("guaca:reconnected")));
    await screen.findByText("This Guaca needs an update");
    expect((input as HTMLInputElement).value).toBe("Unsent words");
    expect(input.closest("[hidden]")).toBeTruthy();
  });
  it("preserves failures and dismisses only the current release notice", async () => {
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Later" }));
    expect(screen.queryByText("Host update available.")).toBeNull();
    release = {
      ...published,
      latest: {
        ...published.latest,
        version: "0.3.0",
        notes: "https://github.com/madebywelch/guaca/releases/tag/v0.3.0",
      },
    };
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Check for updates" }).hasAttribute("disabled"),
      ).toBe(false),
    );
    fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));
    await screen.findByText("Host update available.");
  });
});
