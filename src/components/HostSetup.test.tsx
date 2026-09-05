import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { status, start, update, probe, activate, persist, restart } = vi.hoisted(() => ({
  status: vi.fn(),
  start: vi.fn(),
  update: vi.fn(),
  probe: vi.fn(),
  activate: vi.fn(),
  persist: vi.fn(),
  restart: vi.fn(),
}));
vi.mock("../lib/host", () => ({
  localHost: { status, start, update, openDocker: vi.fn() },
  hostMode: () => "remote",
  rememberMode: vi.fn(),
}));
vi.mock("../lib/transport", () => ({
  desktop: true,
  attached: () => null,
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
  it("makes an update explicit and reports that jobs will be interrupted", async () => {
    status.mockResolvedValue({ state: "running", message: "Ready", updateAvailable: true });
    update.mockResolvedValue({ origin: "http://127.0.0.1:54321", token: "private" });
    render(<HostChoice />);
    const button = await screen.findByRole("button", { name: "Back up and update host" });
    expect(screen.getByText(/Updating interrupts current jobs/)).toBeTruthy();
    fireEvent.click(button);
    await waitFor(() => expect(restart).toHaveBeenCalled());
    expect(update).toHaveBeenCalledOnce();
    expect(start).not.toHaveBeenCalled();
  });
});
