import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const {
  status,
  start,
  update,
  existing,
  connect,
  probe,
  activate,
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
  setRemote: persist,
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
    expect(activate).toHaveBeenCalled();
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
    expect(savedMode).toHaveBeenCalledWith("existing");
    expect(probe).toHaveBeenCalledWith(connection);
    expect(persist).toHaveBeenCalledWith(connection);
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
    expect(persist).toHaveBeenCalledWith({ origin: "https://vps.example", token: "private" });
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
    expect(persist).toHaveBeenCalledWith({ origin: "https://other.example", token: "other" });
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
  it("makes an update explicit and reports that jobs will be interrupted", async () => {
    status.mockResolvedValue({ state: "running", message: "Ready", updateAvailable: true });
    update.mockResolvedValue({ origin: "http://127.0.0.1:54321", token: "private" });
    render(<HostChoice />);
    const button = await screen.findByRole("button", { name: "Back up and update host" });
    expect(screen.getByText(/Updating stops work in progress/)).toBeTruthy();
    fireEvent.click(button);
    await waitFor(() => expect(restart).toHaveBeenCalled());
    expect(update).toHaveBeenCalledOnce();
    expect(start).not.toHaveBeenCalled();
  });
});
