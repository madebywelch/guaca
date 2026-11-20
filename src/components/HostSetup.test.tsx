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
  resume,
  choose,
  elsewhere,
  startElsewhere,
  openDocker,
  source,
  rebuild,
} = vi.hoisted(() => ({
  resume: vi.fn(),
  choose: vi.fn(),
  elsewhere: vi.fn(),
  startElsewhere: vi.fn(),
  openDocker: vi.fn(),
  source: vi.fn(),
  rebuild: vi.fn(),
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
vi.mock("../lib/host", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/host")>()),
  localHost: {
    status,
    start,
    update,
    existing,
    connect,
    openDocker,
    resume,
    choose,
    elsewhere,
    startElsewhere,
  },
  thisApp: { source, rebuild },
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
const { sent } = vi.hoisted(() => ({ sent: [] as string[] }));
vi.mock("../lib/ipc", () => ({
  api: new Proxy(
    {},
    {
      get: (_, command) => async () => {
        sent.push(String(command));
        return null;
      },
    },
  ),
}));

import { HostChoice, HostSetup } from "./HostSetup";

it("does not send a stale panel command after another window commits a host", async () => {
  const transport = await vi.importActual<typeof import("../lib/transport")>("../lib/transport");
  const a = { origin: "https://host-a.example", token: "a" };
  const b = { origin: "https://host-b.example", token: "b" };
  const fetch = vi
    .spyOn(globalThis, "fetch")
    .mockResolvedValue(new Response(JSON.stringify({ ok: null }), { status: 200 }));
  const preserve = (event: Event) => event.preventDefault();
  window.addEventListener(transport.BEFORE_RELOAD, preserve);
  try {
    transport.saveConnection(a, "remote");
    transport.adoptRemote(a);
    transport.guardPanelHost(a, async () => true);
    transport.saveConnection(b, "remote");
    await transport.invoke("list_agents").catch(() => {});
    expect(fetch).not.toHaveBeenCalledWith(`${a.origin}/v1/call`, expect.anything());
  } finally {
    fetch.mockRestore();
    window.removeEventListener(transport.BEFORE_RELOAD, preserve);
    transport.adoptRemote({ origin: "", token: "" });
    localStorage.clear();
  }
});

it("refuses a command if B commits while the panel generation check is pending", async () => {
  const transport = await vi.importActual<typeof import("../lib/transport")>("../lib/transport");
  const a = { origin: "https://host-a.example", token: "a" };
  const b = { origin: "https://host-b.example", token: "b" };
  let acknowledge!: (valid: boolean) => void;
  const fetch = vi.spyOn(globalThis, "fetch");
  const preserve = (event: Event) => event.preventDefault();
  window.addEventListener(transport.BEFORE_RELOAD, preserve);
  try {
    transport.saveConnection(a, "remote");
    transport.adoptRemote(a);
    transport.guardPanelHost(
      a,
      () =>
        new Promise((resolve) => {
          acknowledge = resolve;
        }),
    );
    const command = transport.invoke("list_agents");
    transport.saveConnection(b, "remote");
    acknowledge(true);
    await expect(command).rejects.toMatchObject({ kind: "config" });
    expect(fetch).not.toHaveBeenCalled();
  } finally {
    fetch.mockRestore();
    window.removeEventListener(transport.BEFORE_RELOAD, preserve);
    localStorage.clear();
  }
});

it("allows a same-host panel retry after a transient command failure", async () => {
  const transport = await vi.importActual<typeof import("../lib/transport")>("../lib/transport");
  const a = { origin: "https://host-a.example", token: "a" };
  const fetch = vi
    .spyOn(globalThis, "fetch")
    .mockRejectedValueOnce(new Error("offline"))
    .mockResolvedValueOnce(new Response(JSON.stringify({ ok: "ready" }), { status: 200 }));
  try {
    transport.saveConnection(a, "remote");
    transport.adoptRemote(a);
    transport.guardPanelHost(a, async () => true);
    await expect(transport.invoke("list_agents")).rejects.toMatchObject({ kind: "unreachable" });
    await expect(transport.invoke("list_agents")).resolves.toBe("ready");
    expect(fetch).toHaveBeenCalledTimes(2);
  } finally {
    fetch.mockRestore();
    localStorage.clear();
  }
});

it("preserves a draft and refuses a panel with a stale generation", async () => {
  const transport = await vi.importActual<typeof import("../lib/transport")>("../lib/transport");
  const a = { origin: "https://host-a.example", token: "a" };
  const fetch = vi.spyOn(globalThis, "fetch");
  const preserve = (event: Event) => {
    sessionStorage.setItem("panel-draft", "unsent");
    event.preventDefault();
  };
  window.addEventListener(transport.BEFORE_RELOAD, preserve);
  try {
    transport.saveConnection(a, "remote");
    transport.adoptRemote(a);
    transport.guardPanelHost(a, async () => false);
    await expect(transport.invoke("send_message")).rejects.toMatchObject({ kind: "config" });
    expect(fetch).not.toHaveBeenCalled();
    expect(sessionStorage.getItem("panel-draft")).toBe("unsent");
  } finally {
    fetch.mockRestore();
    window.removeEventListener(transport.BEFORE_RELOAD, preserve);
    localStorage.clear();
    sessionStorage.removeItem("panel-draft");
  }
});

beforeEach(() => {
  vi.clearAllMocks();
  mode.mockReturnValue("remote");
  current.mockReturnValue(null);
  existing.mockResolvedValue([]);
  status.mockResolvedValue({ state: "ready", message: "Docker is ready.", updateAvailable: false });
  probe.mockResolvedValue({});
  openDocker.mockResolvedValue(undefined);
  elsewhere.mockResolvedValue({ from: null, to: null, data: false });
  source.mockResolvedValue({ checkout: null, unavailable: "release", running: false });
});

const LOCAL = { origin: "http://127.0.0.1:54321", token: "private" };
const COLIMA = { context: "colima", label: "Colima", action: "Start Colima" };
const colimaDown = {
  kind: "engine",
  message: "Your host runs in Colima, which is not running.",
  engines: [COLIMA],
};
function savedLocalHost() {
  mode.mockReturnValue("local");
  current.mockReturnValue(LOCAL);
  return render(
    <HostSetup>
      <div>Workspace mounted</div>
    </HostSetup>,
  );
}

describe("a Mac whose host is already set up", () => {
  it("waits for the host's own engine instead of offering setup, and reconnects when it starts", async () => {
    vi.useFakeTimers();
    try {
      resume
        .mockRejectedValueOnce(colimaDown)
        .mockRejectedValueOnce(colimaDown)
        .mockResolvedValue(LOCAL);
      savedLocalHost();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(screen.getByRole("heading").textContent).toBe("Waiting for your host");
      expect(screen.getByRole("status").textContent).toBe(colimaDown.message);
      expect(screen.getByText("Guaca reconnects by itself as soon as it starts.")).toBeTruthy();
      expect(screen.queryByText("Where should your agents work?")).toBeNull();
      expect(screen.queryByRole("button", { name: "Use this Mac" })).toBeNull();
      expect(start).not.toHaveBeenCalled();
      fireEvent.click(screen.getByRole("button", { name: "Start Colima" }));
      expect(openDocker).toHaveBeenCalledWith("colima");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(3_000);
      });
      expect(screen.queryByText("Workspace mounted")).toBeNull();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(3_000);
      });
      expect(screen.getByText("Workspace mounted")).toBeTruthy();
      expect(resume).toHaveBeenCalledTimes(3);
      expect(probe).toHaveBeenCalledWith(LOCAL);
      expect(activate).toHaveBeenCalledWith(LOCAL);
      expect(start).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });
  it("stops waiting after a bounded number of checks and offers to try again", async () => {
    vi.useFakeTimers();
    try {
      resume.mockRejectedValue(colimaDown);
      savedLocalHost();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(100 * 3_000);
      });
      expect(
        screen.getByText("Guaca stopped waiting after five minutes. Start it, then try again."),
      ).toBeTruthy();
      expect(resume).toHaveBeenCalledTimes(101);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(60_000);
      });
      expect(resume).toHaveBeenCalledTimes(101);
      resume.mockResolvedValue(LOCAL);
      fireEvent.click(screen.getByRole("button", { name: "Try again" }));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(screen.getByText("Workspace mounted")).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
  });
  it("asks which host is the person's when one is found in two engines", async () => {
    resume.mockRejectedValue({
      kind: "choose",
      message: "Guaca found a host in more than one Docker engine on this Mac.",
      engines: [
        COLIMA,
        { context: "desktop-linux", label: "Docker Desktop", action: "Open Docker Desktop" },
      ],
    });
    choose.mockResolvedValue(LOCAL);
    savedLocalHost();
    fireEvent.click(await screen.findByRole("button", { name: "Use the host in Colima" }));
    await screen.findByText("Workspace mounted");
    expect(choose).toHaveBeenCalledWith("colima");
    expect(start).not.toHaveBeenCalled();
  });
  it("says a host it cannot find is missing and keeps a new one behind an explicit choice", async () => {
    resume.mockRejectedValue({
      kind: "missing",
      message: "Guaca looked for your host in Colima and Docker Desktop and did not find it.",
    });
    savedLocalHost();
    expect((await screen.findByRole("alert")).textContent).toContain("did not find it");
    expect(screen.getByRole("heading").textContent).toBe("Guaca can't find your host");
    expect(screen.getByText(`Guaca last reached it at ${LOCAL.origin}.`)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Use this Mac" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Set up a new host" }));
    expect(
      screen.getByText("A new host starts without the agents of the host Guaca can't find."),
    ).toBeTruthy();
    expect(await screen.findByRole("button", { name: "Use this Mac" })).toBeTruthy();
    expect(start).not.toHaveBeenCalled();
  });
  it("offers a rebuild, not a script, when a source build's image cannot be had", async () => {
    start.mockRejectedValue({ kind: "image", message: "The registry refused Guaca's host image." });
    source.mockResolvedValue({ checkout: "/Users/me/guaca", unavailable: null, running: false });
    rebuild.mockResolvedValue(undefined);
    render(<HostChoice />);
    fireEvent.click(await screen.findByRole("button", { name: "Use this Mac" }));
    fireEvent.click(await screen.findByRole("button", { name: "Rebuild Guaca" }));
    await waitFor(() => expect(rebuild).toHaveBeenCalled());
    expect(document.body.textContent).not.toContain("scripts/");
  });
  it("offers the download, not a script, when a release's image is refused at setup", async () => {
    start.mockRejectedValue({ kind: "image", message: "The registry refused Guaca's host image." });
    render(<HostChoice />);
    fireEvent.click(await screen.findByRole("button", { name: "Use this Mac" }));
    const download = await screen.findByRole("link", { name: "Download Guaca" });
    expect(download.getAttribute("href")).toContain("/releases");
    expect(screen.queryByRole("button", { name: "Rebuild Guaca" })).toBeNull();
    expect(document.body.textContent).not.toContain("scripts/");
  });
  it("opens the engine the host is in, not Docker Desktop", async () => {
    status.mockResolvedValue({
      state: "unavailable",
      message: colimaDown.message,
      updateAvailable: false,
      engine: COLIMA,
    });
    render(<HostChoice />);
    fireEvent.click(await screen.findByRole("button", { name: "Start Colima" }));
    expect(openDocker).toHaveBeenCalledWith("colima");
    expect(screen.queryByRole("button", { name: "Open Docker" })).toBeNull();
  });
});

describe("a host whose engine is gone for good", () => {
  const DESKTOP = {
    context: "desktop-linux",
    label: "Docker Desktop",
    action: "Open Docker Desktop",
  };
  const plan = { from: COLIMA, to: DESKTOP, toEngine: "desktop-engine", data: false };
  const way = { name: "Set up a new host in Docker Desktop" };
  async function giveUp() {
    resume.mockRejectedValue(colimaDown);
    savedLocalHost();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(100 * 3_000);
    });
  }
  it("is not offered a new host while its engine may still be starting", async () => {
    vi.useFakeTimers();
    try {
      elsewhere.mockResolvedValue(plan);
      resume.mockRejectedValue(colimaDown);
      savedLocalHost();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(30_000);
      });
      expect(screen.getByRole("heading").textContent).toBe("Waiting for your host");
      expect(screen.queryByRole("button", way)).toBeNull();
      expect(elsewhere).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });
  it("cancelling the new host changes nothing", async () => {
    vi.useFakeTimers();
    try {
      elsewhere.mockResolvedValue(plan);
      await giveUp();
      fireEvent.click(screen.getByRole("button", way));
      fireEvent.click(screen.getByRole("checkbox"));
      fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
      expect(screen.getByRole("heading").textContent).toBe("Your host is not running");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(screen.getByRole("button", way)).toBeTruthy();
      expect(startElsewhere).not.toHaveBeenCalled();
      expect(persist).not.toHaveBeenCalled();
      expect(start).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });
  it("sets up a new host elsewhere only after a deliberate confirmation that names what stays behind", async () => {
    vi.useFakeTimers();
    try {
      elsewhere.mockResolvedValue({ ...plan, data: true });
      startElsewhere.mockResolvedValue(LOCAL);
      await giveUp();
      fireEvent.click(screen.getByRole("button", way));
      const confirm = screen.getByRole("button", { name: "Set up the new host" });
      expect(confirm.hasAttribute("disabled")).toBe(true);
      expect(screen.getAllByRole("listitem").map((item) => item.textContent)).toEqual([
        "Your agents and conversations stay with the host in Colima. They are not moved, and they will not appear in the new host.",
        "The host in Colima and its data are left untouched. Nothing is deleted.",
        "Once Colima runs again, Host settings offer to go back to it.",
      ]);
      expect(screen.getByRole("note").textContent).toBe(
        "Docker Desktop already holds Guaca data, probably from an earlier host on this Mac. The new host starts with that data instead of starting empty.",
      );
      fireEvent.click(confirm);
      expect(startElsewhere).not.toHaveBeenCalled();
      fireEvent.click(
        screen.getByRole("checkbox", { name: /my agents stay with the host in Colima/ }),
      );
      fireEvent.click(screen.getByRole("button", { name: "Set up the new host" }));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(startElsewhere).toHaveBeenCalledWith("desktop-linux", "desktop-engine");
      expect(persist).toHaveBeenCalledWith(LOCAL, "local");
      expect(screen.getByText("Workspace mounted")).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
  });
  it("shows the confirmation alone, with nothing from the screen behind it", async () => {
    vi.useFakeTimers();
    try {
      elsewhere.mockResolvedValue(plan);
      await giveUp();
      fireEvent.click(screen.getByRole("button", way));
      expect(screen.getByRole("heading").textContent).toBe("Set up a new host in Docker Desktop");
      expect(screen.getAllByRole("button").map((button) => button.textContent)).toEqual([
        "Set up the new host",
        "Cancel",
      ]);
      expect(screen.queryByText(/Colima, which is not running/)).toBeNull();
      expect(screen.queryByText(/stopped waiting/)).toBeNull();
      expect(screen.queryByRole("note")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });
  it("leaves the chooser out of the confirmation and returns to it on cancel", async () => {
    elsewhere.mockResolvedValue(plan);
    resume.mockRejectedValue(colimaDown);
    savedLocalHost();
    fireEvent.click(await screen.findByRole("button", { name: "Choose another host" }));
    fireEvent.click(await screen.findByRole("button", way));
    expect(screen.queryByText("Where should your agents work?")).toBeNull();
    expect(screen.queryByRole("button", { name: "Remote host" })).toBeNull();
    expect(screen.queryByRole("button", { name: /Use this Mac/ })).toBeNull();
    expect(screen.queryByRole("button", { name: "Back to your host" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByRole("heading").textContent).toBe("Where should your agents work?");
    expect(await screen.findByRole("button", way)).toBeTruthy();
    expect(startElsewhere).not.toHaveBeenCalled();
  });
  it("stops waiting once the person chooses another host, so the old host cannot be attached behind the choice", async () => {
    vi.useFakeTimers();
    try {
      elsewhere.mockResolvedValue(plan);
      resume.mockRejectedValue(colimaDown);
      savedLocalHost();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      fireEvent.click(screen.getByRole("button", { name: "Choose another host" }));
      const asked = resume.mock.calls.length;
      resume.mockResolvedValue(LOCAL);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(30_000);
      });
      expect(resume.mock.calls.length).toBe(asked);
      expect(activate).not.toHaveBeenCalled();
      expect(screen.queryByText("Workspace mounted")).toBeNull();
      expect(screen.getByRole("button", way)).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
  });
  it("says the new host was set up when it does not answer, and offers it no second time", async () => {
    vi.useFakeTimers();
    try {
      elsewhere.mockResolvedValue(plan);
      startElsewhere.mockResolvedValue(LOCAL);
      probe.mockRejectedValue(new Error("nothing answered."));
      await giveUp();
      fireEvent.click(screen.getByRole("button", way));
      fireEvent.click(screen.getByRole("checkbox"));
      fireEvent.click(screen.getByRole("button", { name: "Set up the new host" }));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(
        screen.getByText(/The new host in Docker Desktop was set up, but it did not answer/),
      ).toBeTruthy();
      expect(screen.queryByRole("button", way)).toBeNull();
      expect(persist).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });
  it("says when it could not check the other engines", async () => {
    elsewhere.mockRejectedValue(new Error("Docker could not finish"));
    resume.mockRejectedValue(colimaDown);
    savedLocalHost();
    fireEvent.click(await screen.findByRole("button", { name: "Choose another host" }));
    expect(
      await screen.findByText("Guaca could not check the other Docker engines on this Mac."),
    ).toBeTruthy();
  });
  it("says so when no other engine is running", async () => {
    elsewhere.mockResolvedValue({ from: COLIMA, to: null, toEngine: null, data: false });
    resume.mockRejectedValue(colimaDown);
    savedLocalHost();
    fireEvent.click(await screen.findByRole("button", { name: "Choose another host" }));
    expect(await screen.findByText(/No other Docker engine is running on this Mac/)).toBeTruthy();
    expect(screen.queryByRole("button", way)).toBeNull();
  });
  it("offers the way back to the earlier host once its engine runs again", async () => {
    status.mockResolvedValue({
      state: "running",
      message: "Docker is running. Your local host is available.",
      updateAvailable: false,
      engine: DESKTOP,
      previous: COLIMA,
    });
    choose.mockResolvedValue(LOCAL);
    render(<HostChoice />);
    fireEvent.click(await screen.findByRole("button", { name: "Go back to the host in Colima" }));
    await waitFor(() => expect(restart).toHaveBeenCalled());
    expect(choose).toHaveBeenCalledWith("colima");
    expect(persist).toHaveBeenCalledWith(LOCAL, "local");
  });
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
    await screen.findByText("Guaca could not save this choice. Nothing changed.");
    expect(screen.queryByText("Storage is unavailable.")).toBeNull();
    expect(restart).not.toHaveBeenCalled();
  });
  it("keeps the form for a saved remote host the workspace is not open on", async () => {
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
  it("switches host without asking either host to stop or move a group", async () => {
    sent.length = 0;
    start.mockResolvedValue({ origin: "http://127.0.0.1:54321", token: "private" });
    const local = render(<HostChoice />);
    fireEvent.click(await screen.findByRole("button", { name: "Use this Mac" }));
    await waitFor(() => expect(restart).toHaveBeenCalledTimes(1));
    local.unmount();

    existing.mockResolvedValue([
      { name: "existing-guacad-1", label: "My workspace", origin: "http://127.0.0.1:8788" },
    ]);
    connect.mockResolvedValue({ origin: "http://127.0.0.1:8788", token: "private" });
    const found = render(<HostChoice />);
    fireEvent.click(await screen.findByRole("button", { name: /Use My workspace/ }));
    await waitFor(() => expect(restart).toHaveBeenCalledTimes(2));
    found.unmount();

    render(<HostChoice />);
    fireEvent.click(screen.getByRole("button", { name: "Remote host" }));
    fireEvent.change(screen.getByLabelText("Host address"), {
      target: { value: "https://vps.example" },
    });
    fireEvent.change(screen.getByLabelText("Access key"), { target: { value: "private" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect to host" }));
    await waitFor(() => expect(restart).toHaveBeenCalledTimes(3));

    expect(sent).toEqual([]);
  });
});
