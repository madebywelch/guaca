import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const {
  status,
  start,
  update,
  existing,
  connect,
  probe,
  activate,
  adopt,
  preflight,
  persist,
  restart,
  savedMode,
  mode,
  current,
} = vi.hoisted(() => ({
  mode: vi.fn(),
  current: vi.fn(),
  savedMode: vi.fn(),
  existing: vi.fn(),
  connect: vi.fn(),
  status: vi.fn(),
  start: vi.fn(),
  update: vi.fn(),
  probe: vi.fn(),
  activate: vi.fn(),
  adopt: vi.fn(),
  preflight: vi.fn(),
  persist: vi.fn(),
  restart: vi.fn(),
}));
vi.mock("../lib/host", () => ({
  localHost: { status, start, update, existing, connect, openDocker: vi.fn() },
  hostMode: mode,
  rememberMode: savedMode,
}));
vi.mock("../lib/transport", () => ({
  desktop: true,
  attached: current,
  activateRemote: activate,
  adoptRemote: adopt,
  preflightReload: preflight,
  saveConnection: persist,
  restart,
  probe,
  openExternal: vi.fn(),
}));
vi.mock("./GroupTransfer", () => ({ LegacyGroups: () => null }));

import { HostChoice, HostSetup } from "./HostSetup";

beforeEach(() => {
  vi.clearAllMocks();
  mode.mockReturnValue("remote");
  current.mockReturnValue(null);
  existing.mockResolvedValue([]);
  status.mockResolvedValue({ state: "ready", message: "Docker is ready.", updateAvailable: false });
  probe.mockResolvedValue({});
});
describe("desktop host setup", () => {
  it("lets the operator choose another host while a saved host is connecting", async () => {
    const connection = { origin: "https://k1-guaca.tailnet.test", token: "private" };
    current.mockReturnValue(connection);
    let finish!: () => void;
    probe.mockReturnValue(
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
    );
    render(
      <HostSetup>
        <div>Workspace mounted</div>
      </HostSetup>,
    );
    expect(screen.getByText("Connecting to k1-guaca…")).toBeTruthy();
    expect(screen.getByText(connection.origin)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Choose another host" }));
    expect(screen.getByRole("button", { name: "Remote host" })).toBeTruthy();
    expect(screen.getByText(/Each group stays on the host it was made on/)).toBeTruthy();
    finish();
    await waitFor(() => expect(screen.queryByText("Connecting to k1-guaca…")).toBeNull());
    expect(activate).not.toHaveBeenCalled();
    expect(screen.queryByText("Workspace mounted")).toBeNull();
  });
  it("explains a failed saved host probe and retries using its saved credentials", async () => {
    const connection = { origin: "https://k1-guaca.tailnet.test", token: "private" };
    current.mockReturnValue(connection);
    probe.mockRejectedValueOnce({
      kind: "unreachable",
      message: `nothing answered at ${connection.origin} (timeout)`,
    });
    render(
      <HostSetup>
        <div>Workspace mounted</div>
      </HostSetup>,
    );
    expect((await screen.findByRole("alert")).textContent).toBe(
      "k1-guaca isn't answering at https://k1-guaca.tailnet.test. It may be off or offline.",
    );
    expect(screen.getByRole("button", { name: "Remote host" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await screen.findByText("Workspace mounted");
    expect(probe).toHaveBeenLastCalledWith(connection);
  });
  it("opens the chooser within 15 seconds even if the probe never settles", async () => {
    vi.useFakeTimers();
    try {
      current.mockReturnValue({ origin: "https://k1-guaca.tailnet.test", token: "private" });
      probe.mockReturnValue(new Promise(() => {}));
      render(
        <HostSetup>
          <div>Workspace mounted</div>
        </HostSetup>,
      );
      await act(async () => {
        await vi.advanceTimersByTimeAsync(15_000);
      });
      expect(screen.getByRole("alert").textContent).toContain("Couldn't connect to k1-guaca");
      expect(screen.getByRole("button", { name: "Remote host" })).toBeTruthy();
      expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
  });
  it("does not mount the workspace until a host is ready and authenticated", async () => {
    start.mockResolvedValue({ origin: "http://127.0.0.1:54321", token: "private" });
    render(
      <HostSetup>
        <div>Workspace mounted</div>
      </HostSetup>,
    );
    expect(screen.queryByText("Workspace mounted")).toBeNull();
    await screen.findByText("Docker is ready.");
    fireEvent.click(screen.getByRole("button", { name: "Use this Mac" }));
    await screen.findByText("Workspace mounted");
    expect(probe).toHaveBeenCalledWith({ origin: "http://127.0.0.1:54321", token: "private" });
    expect(adopt).toHaveBeenCalled();
  });
  it("connects an existing local workspace without creating another host", async () => {
    existing.mockResolvedValue([
      { name: "existing-guacad-1", label: "My workspace", origin: "http://127.0.0.1:8788" },
    ]);
    const connection = { origin: "http://127.0.0.1:8788", token: "private" };
    connect.mockResolvedValue(connection);
    render(<HostChoice />);
    fireEvent.click(await screen.findByRole("button", { name: /Use My workspace/ }));
    await waitFor(() => expect(restart).toHaveBeenCalled());
    expect(connect).toHaveBeenCalledWith("existing-guacad-1");
    expect(probe).toHaveBeenCalledWith(connection);
    expect(persist).toHaveBeenCalledWith(connection, "existing");
    expect(start).not.toHaveBeenCalled();
  });
  it("shows an existing local host as local without starting a managed container", async () => {
    mode.mockReturnValue("existing");
    current.mockReturnValue({ origin: "http://127.0.0.1:8788", token: "private" });
    const setup = render(
      <HostSetup>
        <div>Workspace mounted</div>
      </HostSetup>,
    );
    await screen.findByText("Workspace mounted");
    expect(start).not.toHaveBeenCalled();
    setup.unmount();
    render(<HostChoice />);
    await screen.findByText("Docker is ready.");
    expect(screen.getByRole("button", { name: "On this Mac" }).getAttribute("aria-pressed")).toBe(
      "true",
    );
  });
  it("keeps setup open when the host fails", async () => {
    start.mockRejectedValue("The host could not be downloaded.");
    render(
      <HostSetup>
        <div>Workspace mounted</div>
      </HostSetup>,
    );
    await screen.findByText("Docker is ready.");
    fireEvent.click(screen.getByRole("button", { name: "Use this Mac" }));
    await screen.findByRole("alert");
    expect(screen.queryByText("Workspace mounted")).toBeNull();
    expect(activate).not.toHaveBeenCalled();
  });
  it("reads Docker again after a failed start instead of showing what it said before", async () => {
    status.mockResolvedValueOnce({
      state: "stopped",
      message: "Docker is ready. Your local host is stopped.",
      updateAvailable: false,
    });
    status.mockResolvedValueOnce({
      state: "ready",
      message: "Docker is ready. Guaca can set up your local host.",
      updateAvailable: false,
    });
    start.mockRejectedValue("The registry refused Guaca's host image.");
    render(<HostChoice />);
    await screen.findByText("Docker is ready. Your local host is stopped.");
    fireEvent.click(screen.getByRole("button", { name: "Use this Mac" }));
    await screen.findByRole("alert");
    await screen.findByText("Docker is ready. Guaca can set up your local host.");
    expect(status).toHaveBeenCalledTimes(2);
  });
  it("shows an update that stopped part way, and not one that finished", async () => {
    const operation = {
      backup: "guac-host-backup-1",
      previousImage: "guacad:old",
      targetImage: "guacad:new",
    };
    status.mockResolvedValueOnce({
      state: "running",
      message: "Docker is running. Your local host is available.",
      updateAvailable: false,
      operation: { ...operation, stage: "Host updated", error: null },
    });
    const first = render(<HostChoice />);
    await screen.findByText("Docker is running. Your local host is available.");
    expect(screen.queryByText(/Host updated/)).toBeNull();
    expect(screen.queryByText(/guac-host-backup-1/)).toBeNull();
    expect(screen.queryByRole("link", { name: /recovery instructions/ })).toBeNull();
    first.unmount();

    status.mockResolvedValueOnce({
      state: "running",
      message: "Docker is running. Your local host is available.",
      updateAvailable: true,
      operation: { ...operation, stage: "Update canceled", error: "The registry refused it." },
    });
    render(<HostChoice />);
    await screen.findByText("Update canceled: The registry refused it.");
    expect(screen.getByRole("link", { name: /recovery instructions/ })).toBeTruthy();
  });
  it("offers Docker's own actions only while Docker cannot be used", async () => {
    const ready = render(<HostChoice />);
    await screen.findByText("Docker is ready.");
    expect(screen.queryByRole("button", { name: "Open Docker" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Check again" })).toBeNull();
    ready.unmount();

    status.mockResolvedValueOnce({
      state: "unavailable",
      message: "Docker is installed but is not ready.",
      updateAvailable: false,
    });
    render(<HostChoice />);
    await screen.findByText("Docker is installed but is not ready.");
    expect(screen.getByRole("button", { name: "Open Docker" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Check again" })).toBeTruthy();
  });
  it("gives missing Docker an installation action and a retry", async () => {
    status.mockResolvedValueOnce({
      state: "missing",
      message: "Install Docker Desktop.",
      updateAvailable: false,
    });
    render(<HostChoice />);
    await screen.findByRole("button", { name: "Get Docker Desktop" });
    expect(
      (screen.getByRole("button", { name: "Use this Mac" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    await screen.findByText("Docker is ready.");
    expect(
      (screen.getByRole("button", { name: "Use this Mac" }) as HTMLButtonElement).disabled,
    ).toBe(false);
  });
  it("refuses insecure remote addresses before sending an access key", async () => {
    render(<HostChoice />);
    fireEvent.click(screen.getByRole("button", { name: "Remote host" }));
    fireEvent.change(screen.getByLabelText("Host address"), {
      target: { value: "http://vps.example" },
    });
    fireEvent.change(screen.getByLabelText("Access key"), { target: { value: "private" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect to host" }));
    await screen.findByText(/secure https/);
    expect(probe).not.toHaveBeenCalled();
    expect(persist).not.toHaveBeenCalled();
  });
  it("switches only after the remote host accepts the key", async () => {
    render(<HostChoice />);
    fireEvent.click(screen.getByRole("button", { name: "Remote host" }));
    fireEvent.change(screen.getByLabelText("Host address"), {
      target: { value: "https://vps.example" },
    });
    fireEvent.change(screen.getByLabelText("Access key"), { target: { value: "private" } });
    probe.mockRejectedValueOnce("Access key was not accepted.");
    fireEvent.click(screen.getByRole("button", { name: "Connect to host" }));
    await screen.findByRole("alert");
    expect(persist).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Connect to host" }));
    await waitFor(() => expect(restart).toHaveBeenCalled());
    expect(persist).toHaveBeenCalledWith(
      { origin: "https://vps.example", token: "private" },
      "remote",
    );
  });
  it("does not commit a remote host when reload preflight is canceled", async () => {
    preflight.mockImplementationOnce(() => {
      throw new Error("Wait for the current work.");
    });
    render(<HostChoice />);
    fireEvent.click(screen.getByRole("button", { name: "Remote host" }));
    fireEvent.change(screen.getByLabelText("Host address"), {
      target: { value: "https://other.example" },
    });
    fireEvent.change(screen.getByLabelText("Access key"), { target: { value: "private" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect to host" }));
    await screen.findByText("Wait for the current work.");
    expect(persist).not.toHaveBeenCalled();
    expect(restart).not.toHaveBeenCalled();
  });
  it("does not reload when saving the selected host fails", async () => {
    persist.mockImplementationOnce(() => {
      throw new Error("Storage is unavailable.");
    });
    render(<HostChoice />);
    fireEvent.click(screen.getByRole("button", { name: "Remote host" }));
    fireEvent.change(screen.getByLabelText("Host address"), {
      target: { value: "https://other.example" },
    });
    fireEvent.change(screen.getByLabelText("Access key"), { target: { value: "private" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect to host" }));
    await screen.findByText("Storage is unavailable.");
    expect(restart).not.toHaveBeenCalled();
  });
  it("keeps the form for a saved remote host the workspace is not open on", async () => {
    // Onboarding and a host turned away both reach here with a remote attached,
    // and the operator needs the fields to correct it, not a claim it works.
    current.mockReturnValue({ origin: "https://vps.example", token: "private" });
    render(<HostChoice />);
    expect(screen.queryByRole("button", { name: "Change host" })).toBeNull();
    expect((screen.getByLabelText("Host address") as HTMLInputElement).value).toBe(
      "https://vps.example",
    );
    expect(screen.getByLabelText("Access key")).toBeTruthy();
  });
  it("states the remote host in use instead of drawing an empty access key", () => {
    current.mockReturnValue({ origin: "https://vps.example", token: "private" });
    const { container } = render(<HostChoice inUse />);
    expect(screen.getByRole("status").textContent).toBe(
      "Connected to https://vps.example. Your access key is saved on this Mac.",
    );
    expect(screen.queryByLabelText("Access key")).toBeNull();
    expect(screen.queryByLabelText("Host address")).toBeNull();
    expect(screen.queryByRole("button", { name: "Connect to host" })).toBeNull();
    expect(container.innerHTML).not.toContain("private");
  });
  it("changes the host in use only once the new one accepts its key, and can go back", async () => {
    current.mockReturnValue({ origin: "https://vps.example", token: "private" });
    render(<HostChoice inUse />);
    fireEvent.click(screen.getByRole("button", { name: "Change host" }));
    const address = screen.getByLabelText("Host address") as HTMLInputElement;
    expect(address.value).toBe("https://vps.example");
    expect(document.activeElement).toBe(address);
    expect((screen.getByLabelText("Access key") as HTMLInputElement).value).toBe("");
    const connectButton = screen.getByRole("button", { name: "Connect to host" });
    expect((connectButton as HTMLButtonElement).disabled).toBe(true);

    fireEvent.change(address, { target: { value: "https://other.example" } });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await screen.findByText(/Connected to https:\/\/vps\.example\./);
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "Change host" })),
    );

    fireEvent.click(screen.getByRole("button", { name: "Change host" }));
    expect((screen.getByLabelText("Host address") as HTMLInputElement).value).toBe(
      "https://vps.example",
    );
    fireEvent.change(screen.getByLabelText("Host address"), {
      target: { value: "https://other.example" },
    });
    fireEvent.change(screen.getByLabelText("Access key"), { target: { value: "other" } });
    probe.mockRejectedValueOnce("Access key was not accepted.");
    fireEvent.click(screen.getByRole("button", { name: "Connect to host" }));
    await screen.findByRole("alert");
    expect(persist).not.toHaveBeenCalled();
    expect(screen.queryByText(/Connected to/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Connect to host" }));
    await waitFor(() => expect(restart).toHaveBeenCalled());
    expect(persist).toHaveBeenCalledWith(
      { origin: "https://other.example", token: "other" },
      "remote",
    );
  });
  it("does not offer a local host's loopback address as a remote host", async () => {
    mode.mockReturnValue("existing");
    current.mockReturnValue({ origin: "http://127.0.0.1:8788", token: "private" });
    render(<HostChoice inUse />);
    await screen.findByText("Docker is ready.");
    fireEvent.click(screen.getByRole("button", { name: "Remote host" }));
    expect(screen.queryByRole("button", { name: "Change host" })).toBeNull();
    expect((screen.getByLabelText("Host address") as HTMLInputElement).value).toBe("");
  });
  it("makes an update explicit and says that work pauses for it", async () => {
    status.mockResolvedValue({ state: "running", message: "Ready", updateAvailable: true });
    update.mockResolvedValue({ origin: "http://127.0.0.1:54321", token: "private" });
    render(<HostChoice />);
    const button = await screen.findByRole("button", { name: "Back up and update host" });
    expect(screen.getByText(/Work in progress pauses/)).toBeTruthy();
    fireEvent.click(button);
    await waitFor(() => expect(restart).toHaveBeenCalled());
    expect(update).toHaveBeenCalledOnce();
    expect(start).not.toHaveBeenCalled();
  });
});
