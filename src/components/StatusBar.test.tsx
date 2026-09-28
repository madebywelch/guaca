import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type {
  AgentCard,
  Artifact,
  ArtifactRead,
  CondensedView,
  QuickAction,
  QuickDoes,
  Settings,
  Widget,
} from "../lib/types";

const addQuickAction = vi.fn<(label: string, does: QuickDoes) => Promise<Settings>>();
const removeQuickAction = vi.fn<(id: string) => Promise<Settings>>();
const artifactCondensed = vi.fn<(id: string) => Promise<CondensedView>>();
const artifactData = vi.fn<(id: string) => Promise<ArtifactRead[]>>(async () => []);
const frameArtifact = vi.fn<(html: string) => Promise<{ port: number; id: string }>>(async () => ({
  port: 1,
  id: "digest",
}));
const unpinArtifact = vi.fn<(id: string) => Promise<Settings>>();
vi.mock("../lib/ipc", () => ({
  api: {
    addQuickAction: (label: string, does: QuickDoes) => addQuickAction(label, does),
    removeQuickAction: (id: string) => removeQuickAction(id),
    artifactCondensed: (id: string) => artifactCondensed(id),
    artifactData: (id: string) => artifactData(id),
    frameArtifact: (html: string) => frameArtifact(html),
    unpinArtifact: (id: string) => unpinArtifact(id),
  },
}));
const host = vi.hoisted(() => ({ version: "0.1.0" as string | undefined, build: "aaaaaaa" }));
vi.mock("./HostUpdates", () => ({
  useHostState: () => ({ health: { service: "guacad", build: host.build, version: host.version } }),
}));
vi.mock("../lib/build", () => ({ VERSION: "0.1.0", COMMIT: "aaaaaaa" }));

import { StatusBar } from "./StatusBar";

function card(id: string, name: string): AgentCard {
  return {
    id,
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    browserConsent: "open",
    hasTerminal: false,
    harness: "pi",
    gate: "open",
    name,
    avatar: "plain",
    color: "#c7d96b",
    model: "m",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    discardedAt: null,
  };
}

const brief: QuickAction = {
  id: "q1",
  label: "Morning brief",
  does: { kind: "message", agentId: "a1", text: "Give me the morning brief." },
  addedBy: "Scout",
};
const calendar: QuickAction = {
  id: "q2",
  label: "Calendar",
  does: { kind: "open", place: { kind: "calendar" } },
  addedBy: "the operator",
};

function seed(quickActions: QuickAction[], widgets: Widget[] = []) {
  const settings = { quickActions, widgets } as unknown as Settings;
  useStore.setState({ settings, agents: [card("a1", "Scout"), card("a2", "Pip")] });
  return settings;
}

function page(over: Partial<Artifact> = {}): Artifact {
  return {
    id: "art-1",
    groupId: "00000000-0000-4000-8000-000000000001",
    owner: { id: "a1", name: "Scout", gone: false },
    title: "Open PRs",
    version: 3,
    editedBy: { kind: "agent", id: "a1", name: "Scout" },
    createdAt: 0,
    updatedAt: 0,
    sources: [],
    sourcesAllowed: false,
    condensed: true,
    ...over,
  };
}

const pin = (over: Partial<Widget> = {}): Widget => ({
  artifactId: "art-1",
  width: "narrow",
  everyMinutes: 5,
  addedBy: "Scout",
  ...over,
});

const reads = [{ name: "prs", data: [1, 2, 3], text: "[1,2,3]", error: null }];

describe("StatusBar", () => {
  const onOpen = vi.fn();
  const onMessage = vi.fn<(agentId: string, text: string) => Promise<void>>();
  beforeEach(() => {
    vi.clearAllMocks();
    host.version = "0.1.0";
    host.build = "aaaaaaa";
    onMessage.mockResolvedValue();
  });

  it("runs each kind of button, and says who asked for an agent's", async () => {
    seed([brief, calendar]);
    render(<StatusBar onOpen={onOpen} onMessage={onMessage} />);
    const button = screen.getByRole("button", { name: "Morning brief" });
    expect(button.title).toBe(
      "Sends Scout: “Give me the morning brief.”. Added by Scout, with your approval.",
    );
    fireEvent.click(button);
    await waitFor(() => expect(onMessage).toHaveBeenCalledWith("a1", "Give me the morning brief."));
    fireEvent.click(screen.getByRole("button", { name: "Calendar" }));
    expect(onOpen).toHaveBeenCalledWith({ kind: "calendar" });
    expect(screen.getByRole("button", { name: "Calendar" }).title).toBe("Opens the calendar");
  });

  it("names the host and flags a host on another release", () => {
    seed([]);
    const { rerender } = render(<StatusBar onOpen={onOpen} onMessage={onMessage} />);
    const label = screen.getByRole("button", { name: "Host 0.1.0" });
    expect(label.hasAttribute("data-drift")).toBe(false);
    fireEvent.click(label);
    expect(onOpen).toHaveBeenCalledWith({ kind: "settings", section: "workspace" });
    host.version = "0.2.0";
    rerender(<StatusBar onOpen={onOpen} onMessage={onMessage} />);
    const behind = screen.getByRole("button", { name: "Host 0.2.0" });
    expect(behind.hasAttribute("data-drift")).toBe(true);
    expect(behind.title).toContain("Update this app");
  });

  it("says which build a host on the same version is, without a hover", () => {
    // The label read "Host 0.1.0" for either build, and the only difference
    // was a color and a title nobody hovers for.
    seed([]);
    host.build = "a".repeat(40);
    const { rerender } = render(<StatusBar onOpen={onOpen} onMessage={onMessage} />);
    const same = screen.getByRole("button", { name: /^Host/ });
    expect(same.textContent).toBe("Host 0.1.0");
    expect(same.hasAttribute("data-drift")).toBe(false);
    host.build = "c15bd9a".padEnd(40, "0");
    rerender(<StatusBar onOpen={onOpen} onMessage={onMessage} />);
    const other = screen.getByRole("button", { name: /^Host/ });
    expect(other.textContent).toBe("Host 0.1.0 · c15bd9a");
    expect(other.hasAttribute("data-drift")).toBe(true);
    expect(other.title).toBe(
      "This host and this app are different builds of Guaca 0.1.0. Open Workspace settings to see which.",
    );
  });

  it("adds the operator's own button, and takes one off in edit mode", async () => {
    const before = seed([calendar]);
    addQuickAction.mockResolvedValue({ ...before, quickActions: [calendar, brief] });
    removeQuickAction.mockResolvedValue({ ...before, quickActions: [] });
    render(<StatusBar onOpen={onOpen} onMessage={onMessage} />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByLabelText("Label"), { target: { value: "Brief" } });
    fireEvent.change(screen.getByLabelText("To"), { target: { value: "a2" } });
    fireEvent.change(screen.getByLabelText("Message"), { target: { value: "Brief me." } });
    fireEvent.click(screen.getByRole("button", { name: "Add to the status bar" }));
    await waitFor(() =>
      expect(addQuickAction).toHaveBeenCalledWith("Brief", {
        kind: "message",
        agentId: "a2",
        text: "Brief me.",
      }),
    );
    await screen.findByRole("button", { name: "Morning brief" });
    fireEvent.click(screen.getByRole("button", { name: "Remove Calendar" }));
    await waitFor(() => expect(removeQuickAction).toHaveBeenCalledWith("q2"));
  });

  it("offers to add one when there are none, and says why a send failed", async () => {
    seed([brief]);
    onMessage.mockRejectedValue({ kind: "unreachable", message: "the host is down" });
    render(<StatusBar onOpen={onOpen} onMessage={onMessage} />);
    fireEvent.click(screen.getByRole("button", { name: "Morning brief" }));
    expect((await screen.findByRole("alert")).textContent).toBe("the host is down");
    useStore.setState({ settings: { quickActions: [] } as unknown as Settings });
    expect(await screen.findByRole("button", { name: "Add a quick action" })).toBeTruthy();
  });
});

describe("a pinned page", () => {
  const onOpen = vi.fn();
  const onMessage = vi.fn<(agentId: string, text: string) => Promise<void>>();
  beforeEach(() => {
    vi.clearAllMocks();
    useStore.setState({ artifactsOpen: null });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("is drawn from its condensed view, in the bar's look, and a click opens the page", async () => {
    seed([], [pin({ width: "wide" })]);
    artifactCondensed.mockResolvedValue({ artifact: page(), page: "<b>3 open</b>" });
    const { container } = render(<StatusBar onOpen={onOpen} onMessage={onMessage} />);

    const frame = await waitFor(() => {
      const found = container.querySelector("iframe.widget__frame");
      if (!found) throw new Error("no frame yet");
      return found;
    });
    // Scripts and nothing else, and never a target for the pointer.
    expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
    expect(container.querySelector(".widget")?.getAttribute("data-width")).toBe("wide");
    const framed = frameArtifact.mock.calls[0]?.[0] ?? "";
    expect(framed.endsWith("<b>3 open</b>")).toBe(true);
    expect(framed).toContain("--guaca-text:");

    fireEvent.click(screen.getByRole("button", { name: "Open Open PRs" }));
    expect(useStore.getState().artifactsOpen).toEqual({ id: "art-1" });
    expect(artifactData).not.toHaveBeenCalled();
  });

  it("reads on the pin's clock once its reads are allowed, and only while the window shows", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
    seed([], [pin({ everyMinutes: 5 })]);
    artifactCondensed.mockResolvedValue({
      artifact: page({
        sources: [{ name: "prs", tool: "github__list_pull_requests", arguments: {} }],
        sourcesAllowed: true,
      }),
      page: "<b id=n></b>",
    });
    artifactData.mockResolvedValue(reads);
    render(<StatusBar onOpen={onOpen} onMessage={onMessage} />);
    await waitFor(() => expect(artifactData).toHaveBeenCalledTimes(1));

    await act(async () => vi.advanceTimersByTime(5 * 60_000));
    expect(artifactData).toHaveBeenCalledTimes(2);

    const hidden = vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    await act(async () => vi.advanceTimersByTime(5 * 60_000));
    expect(artifactData).toHaveBeenCalledTimes(2);
    hidden.mockReturnValue("visible");
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    expect(artifactData).toHaveBeenCalledTimes(3);
    hidden.mockRestore();
  });

  it("asks once for reads nobody allowed, so the page can say so, and not again", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
    seed([], [pin()]);
    artifactCondensed.mockResolvedValue({
      artifact: page({
        sources: [{ name: "prs", tool: "github__list_pull_requests", arguments: {} }],
      }),
      page: "<b></b>",
    });
    render(<StatusBar onOpen={onOpen} onMessage={onMessage} />);
    await waitFor(() => expect(artifactData).toHaveBeenCalledTimes(1));
    await act(async () => vi.advanceTimersByTime(60 * 60_000));
    expect(artifactData).toHaveBeenCalledTimes(1);
  });

  it("holds its place with its title when this version has no condensed view", async () => {
    seed([], [pin()]);
    artifactCondensed.mockResolvedValue({ artifact: page({ condensed: false }), page: null });
    const { container } = render(<StatusBar onOpen={onOpen} onMessage={onMessage} />);
    expect(await screen.findByText("Open PRs")).toBeTruthy();
    expect(container.querySelector("iframe")).toBeNull();
    expect(frameArtifact).not.toHaveBeenCalled();
  });

  it("comes off the bar in edit mode", async () => {
    const before = seed([], [pin()]);
    artifactCondensed.mockResolvedValue({ artifact: page(), page: "<b>3</b>" });
    unpinArtifact.mockResolvedValue({ ...before, widgets: [] });
    render(<StatusBar onOpen={onOpen} onMessage={onMessage} />);
    expect(screen.getByRole("button", { name: "Edit" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.click(await screen.findByRole("button", { name: "Unpin Open PRs" }));
    await waitFor(() => expect(unpinArtifact).toHaveBeenCalledWith("art-1"));
    await waitFor(() => expect(screen.queryByRole("toolbar", { name: "Pinned pages" })).toBeNull());
  });
});
