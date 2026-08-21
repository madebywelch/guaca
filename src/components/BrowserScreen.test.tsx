import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { AgentCard, Browser } from "../lib/types";
import { BrowserScreen } from "./BrowserScreen";

const agentBrowser = vi.fn<(id: string) => Promise<Browser | null>>();
const stopAgentBrowser = vi.fn<(id: string) => Promise<void>>();

vi.mock("../lib/ipc", () => ({
  api: {
    agentBrowser: (id: string) => agentBrowser(id),
    startAgentBrowser: vi.fn(),
    stopAgentBrowser: (id: string) => stopAgentBrowser(id),
  },
}));

function card(id: string, name: string): AgentCard {
  return {
    id,
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    browserId: null,
    name,
    avatar: "plain",
    color: "#c7d96b",
    model: "m",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
  };
}

const HAS_ONE: Browser = {
  sessionId: "kb-live",
  state: "running",
  liveViewUrl: "https://x.onkernel.com:8443/browser/live/tok",
};

function configure(kernelKeySet: boolean) {
  useStore.setState({
    settings: {
      operatorName: "",
      e2bKeySet: false,
      e2bKeyHint: "",
      computerIdleMinutes: 15,
      kernelKeySet,
      kernelKeyHint: "",
      browserIdleMinutes: 60,
      browserStealth: false,
      baseUrl: "",
      defaultModel: "",
      apiKeySet: true,
      apiKeyHint: "",
      requestTimeoutSecs: 120,
      limits: {
        maxHops: 8,
        maxStepsPerRun: 60,
        maxFanoutPerCall: 8,
        maxSendsPerPair: 6,
        maxToolRounds: 24,
      },
    },
  });
}

describe("BrowserScreen", () => {
  beforeEach(() => {
    agentBrowser.mockReset();
    stopAgentBrowser.mockReset();
    configure(true);
  });

  it("says nothing at all when no browser provider is configured", async () => {
    // Offering to give an agent a browser that cannot be made is worse than not
    // mentioning browsers. It also must not ask, because the answer costs a
    // round trip to a provider with no key.
    configure(false);
    const { container } = render(<BrowserScreen agent={card("a", "Cook")} />);
    await waitFor(() => expect(container.firstChild).toBeNull());
    expect(agentBrowser).not.toHaveBeenCalled();
  });

  it("never shows one agent's browser in another agent's panel", async () => {
    // The lookup for the agent being left behind is the slower of the two, so
    // it lands after the operator has already switched. That is the whole bug.
    let settle: (value: Browser | null) => void = () => {};
    agentBrowser.mockImplementation(
      (id) =>
        new Promise((resolve) => {
          if (id === "has-one") settle = resolve;
          else resolve(null);
        }),
    );

    const view = render(<BrowserScreen agent={card("has-one", "Cook")} />);
    view.rerender(<BrowserScreen agent={card("has-none", "Scribe")} />);
    settle(HAS_ONE);

    await waitFor(() => expect(screen.getByText("Scribe's browser")).toBeTruthy());
    expect(screen.queryByTitle("Cook's browser")).toBeNull();
  });

  it("explains that signing the agent in is the operator's job", async () => {
    // The one thing an agent cannot do for itself, and the pane is where it
    // happens. An empty state that only said "no browser yet" left the operator
    // with no idea that taking over was the point.
    agentBrowser.mockResolvedValue(null);
    render(<BrowserScreen agent={card("a", "Cook")} />);

    expect(await screen.findByText(/sign this agent in/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open one" })).toBeTruthy();
  });

  it("keeps a click out of the browser until the operator asks for it", async () => {
    agentBrowser.mockResolvedValue(HAS_ONE);
    render(<BrowserScreen agent={card("a", "Cook")} />);

    const veil = await screen.findByRole("button", { name: /take over/i });
    fireEvent.click(veil);

    expect(screen.queryByRole("button", { name: /take over/i })).toBeNull();
    expect(screen.getByRole("dialog", { name: "Cook's browser" })).toBeTruthy();
  });

  it("keeps the same connection open across the change of size", async () => {
    // Remounting the frame would drop the live view and reconnect, which loses
    // whatever half-typed sign-in was on the screen.
    agentBrowser.mockResolvedValue(HAS_ONE);
    render(<BrowserScreen agent={card("a", "Cook")} />);

    const before = await screen.findByTitle("Cook's browser");
    fireEvent.click(screen.getByRole("button", { name: /take over/i }));
    expect(screen.getByTitle("Cook's browser")).toBe(before);
  });

  it("lets a paste reach the page, because signing in means pasting a password", async () => {
    // Without the clipboard permission on the frame the paste silently does
    // nothing, which reads as a password manager that will not fill.
    agentBrowser.mockResolvedValue(HAS_ONE);
    render(<BrowserScreen agent={card("a", "Cook")} />);

    const frame = await screen.findByTitle("Cook's browser");
    expect(frame.getAttribute("allow")).toContain("clipboard-write");
  });

  it("does not close a browser on one click", async () => {
    // Closing is how a sign-in is saved, but it is also the end of whatever the
    // agent had open, so it asks first.
    agentBrowser.mockResolvedValue(HAS_ONE);
    render(<BrowserScreen agent={card("a", "Cook")} />);

    fireEvent.click(await screen.findByRole("button", { name: /take over/i }));
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(stopAgentBrowser).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: /Close it and save/ }));
    await waitFor(() => expect(stopAgentBrowser).toHaveBeenCalledWith("a"));
  });
});
