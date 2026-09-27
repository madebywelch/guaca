import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { AgentCard, QuickAction, QuickDoes, Settings } from "../lib/types";

const addQuickAction = vi.fn<(label: string, does: QuickDoes) => Promise<Settings>>();
const removeQuickAction = vi.fn<(id: string) => Promise<Settings>>();
vi.mock("../lib/ipc", () => ({
  api: {
    addQuickAction: (label: string, does: QuickDoes) => addQuickAction(label, does),
    removeQuickAction: (id: string) => removeQuickAction(id),
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

function seed(quickActions: QuickAction[]) {
  const settings = { quickActions } as unknown as Settings;
  useStore.setState({ settings, agents: [card("a1", "Scout"), card("a2", "Pip")] });
  return settings;
}

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

  it("flags a host on the same version and another build", () => {
    seed([]);
    host.build = "bbbbbbb";
    render(<StatusBar onOpen={onOpen} onMessage={onMessage} />);
    const other = screen.getByRole("button", { name: "Host 0.1.0" });
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
