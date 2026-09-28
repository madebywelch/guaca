import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { platform, mode, docker, update, reload } = vi.hoisted(() => ({
  platform: { desktop: true },
  mode: vi.fn(),
  docker: vi.fn(),
  update: vi.fn(),
  reload: vi.fn(),
}));
vi.mock("../lib/host", async (actual) => ({
  ...(await actual<typeof import("../lib/host")>()),
  hostMode: mode,
  localHost: { status: docker, update },
}));
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
let box: Record<string, unknown>;
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
  box = { managed: false };
  fetched.mockImplementation((url: string) =>
    Promise.resolve(
      new Response(
        JSON.stringify(url.endsWith("/health") ? health : url.endsWith("/v1/host") ? box : release),
        { headers: { "content-type": "application/json" } },
      ),
    ),
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
    await screen.findByText("Host update available: Guaca 0.2.0.");
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
  it("reads a page served in place of the release route as an older host", async () => {
    fetched.mockImplementation((url: string) =>
      Promise.resolve(
        url.endsWith("/health")
          ? new Response(JSON.stringify(health), {
              headers: { "content-type": "application/json" },
            })
          : new Response("<!doctype html><title>Guaca</title>", {
              headers: { "content-type": "text/html" },
            }),
      ),
    );
    mount();
    await screen.findByText("Workspace mounted");
    await screen.findByText(/does not support release checks yet/);
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
    expect(screen.queryByText(/is available for this app and its host/)).toBeNull();
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
    await screen.findByText("Guaca 0.3.0 is available for this app and its host.");
  });
  it("tells a desktop that its host has moved ahead of it, and where to get the app", async () => {
    health.version = "0.2.0";
    mount();
    await screen.findByText(
      "This host runs Guaca 0.2.0 and this app is 0.1.0. Update this app to match.",
    );
    expect(screen.getByText("This app runs an older Guaca than its host.")).toBeTruthy();
    expect(
      screen.getByRole("link", { name: "Download the latest Guaca desktop app" }),
    ).toBeTruthy();
  });
  it("tells a desktop that its host is behind it", async () => {
    health.version = "0.0.9";
    release = { ...published, latest: null };
    mount();
    await screen.findByText(
      "This host runs Guaca 0.0.9 and this app is 0.1.0. Update the host to match.",
    );
    expect(
      screen.queryByRole("link", { name: "Download the latest Guaca desktop app" }),
    ).toBeNull();
  });
});

describe("a box that updates itself", () => {
  const idle = {
    managed: true,
    updating: false,
    running: { image: "ghcr.io/madebywelch/guaca/guacad@sha256:old", version: "0.1.0" },
    operation: null,
    error: null,
  };
  const json = (value: unknown, status = 200) =>
    Promise.resolve(
      new Response(JSON.stringify(value), {
        status,
        headers: { "content-type": "application/json" },
      }),
    );
  /** Serves the box, and answers an update request with `answer`. */
  function serve(answer: () => Promise<Response>) {
    fetched.mockImplementation((url: string, init?: RequestInit) => {
      if (init?.method === "POST") return answer();
      if (url.endsWith("/health"))
        return health.down ? Promise.reject(new TypeError("offline")) : json(health);
      return json(url.endsWith("/v1/host") ? box : release);
    });
  }
  const poke = () => act(async () => window.dispatchEvent(new Event("guaca:reconnected")));

  beforeEach(() => {
    box = { ...idle };
  });

  it("updates on one click, to the release that was reviewed, and says when it is done", async () => {
    serve(() => json({ ...idle, updating: true }));
    mount();
    const button = await screen.findByRole("button", { name: "Back up and update host" });
    expect(screen.getByText("Update this host to Guaca 0.2.0.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "View update instructions" })).toBeNull();
    fireEvent.click(button);
    await waitFor(() =>
      expect(fetched).toHaveBeenCalledWith(
        "https://host.example/v1/host/update",
        expect.objectContaining({ method: "POST", body: JSON.stringify({ version: "0.2.0" }) }),
      ),
    );
    // The host stops for the swap. That is the update working, not a failure.
    health.down = true;
    box = { ...idle, updating: true };
    await poke();
    await screen.findByText("The host is restarting on its new version.");
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByText(/closing Guaca does not stop it/)).toBeTruthy();
    delete health.down;
    health.version = "0.2.0";
    box = {
      ...idle,
      operation: {
        stage: "Host updated",
        backup: "guacad-backup-1",
        previousImage: "old",
        targetImage: "new",
        targetVersion: "0.2.0",
        error: null,
      },
    };
    await poke();
    await screen.findByText(
      "Host updated to Guaca 0.2.0. Review any interrupted work before trying it again.",
    );
  });

  it("says the previous version was put back when the update did not finish", async () => {
    serve(() => json({ ...idle, updating: true }));
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Back up and update host" }));
    await waitFor(() =>
      expect(fetched.mock.calls.some(([, init]) => init?.method === "POST")).toBe(true),
    );
    box = {
      ...idle,
      operation: {
        stage: "Previous version restored",
        backup: "guacad-backup-1",
        previousImage: "old",
        targetImage: "new",
        targetVersion: "0.2.0",
        preserved: "guacad-failed-1",
        error: "The updated host reported 0.1.0 (API 1), not Guaca 0.2.0.",
      },
    };
    await poke();
    const said = await screen.findByRole("alert");
    expect(said.textContent).toContain("not Guaca 0.2.0.");
    expect(said.textContent).toContain("Guaca put the previous version back");
    fireEvent.click(screen.getByText("Last host update"));
    expect(screen.getByText("guacad-failed-1")).toBeTruthy();
  });

  it("never reports the last update as this one when this one never began", async () => {
    box = {
      ...idle,
      operation: {
        stage: "Host updated",
        backup: "guacad-backup-0",
        previousImage: "older",
        targetImage: "old",
        targetVersion: "0.1.0",
        error: null,
      },
    };
    serve(() => json({ ...box, updating: true }));
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Back up and update host" }));
    await waitFor(() =>
      expect(fetched.mock.calls.some(([, init]) => init?.method === "POST")).toBe(true),
    );
    box = { ...box, updating: false, error: "Another Guaca process is managing this host." };
    await poke();
    const said = await screen.findByRole("alert");
    expect(said.textContent).toContain("Another Guaca process");
    expect(screen.queryByText(/Host updated to/)).toBeNull();
  });

  it("shows the updater's refusal and changes nothing", async () => {
    serve(() =>
      json(
        {
          err: {
            kind: "updater",
            message:
              "The latest release is Guaca 0.3.0, not 0.2.0. Check for updates and review it again.",
          },
        },
        409,
      ),
    );
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Back up and update host" }));
    const said = await screen.findByRole("alert");
    expect(said.textContent).toContain("review it again");
    expect(
      screen.getByRole("button", { name: "Back up and update host" }).hasAttribute("disabled"),
    ).toBe(false);
  });

  it("does not offer a desktop a release it could not talk to afterward", async () => {
    const latest = (release as { latest: Record<string, unknown> }).latest;
    Object.assign(latest, { apiGeneration: 2, clientMaximum: 2 });
    serve(() => json({ ...idle, updating: true }));
    mount();
    await screen.findByText(/needs a newer desktop app/);
    expect(screen.queryByRole("button", { name: "Back up and update host" })).toBeNull();
  });

  it("offers a browser the same release, since the box serves the page that goes with it", async () => {
    platform.desktop = false;
    const latest = (release as { latest: Record<string, unknown> }).latest;
    Object.assign(latest, { apiGeneration: 2, clientMaximum: 2 });
    serve(() => json({ ...idle, updating: true }));
    mount();
    expect(await screen.findByRole("button", { name: "Back up and update host" })).toBeTruthy();
  });

  it("offers a box on a source build the signed release, and never an older one", async () => {
    health.release = false;
    health.version = "0.2.0";
    serve(() => json({ ...idle, updating: true }));
    mount();
    expect(await screen.findByRole("button", { name: "Back up and update host" })).toBeTruthy();
    cleanup();
    health.version = "0.3.0";
    mount();
    await screen.findByText("This app runs an older Guaca than its host.");
    expect(screen.queryByRole("button", { name: "Back up and update host" })).toBeNull();
  });

  it("offers nothing on a host without an updater, and keeps the instructions", async () => {
    box = { managed: false };
    serve(() => json({}));
    mount();
    await screen.findByText("A newer host release is available.");
    expect(screen.queryByRole("button", { name: "Back up and update host" })).toBeNull();
    expect(screen.getByRole("button", { name: "View update instructions" })).toBeTruthy();
  });
});
