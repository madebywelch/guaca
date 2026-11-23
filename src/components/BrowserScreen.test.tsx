import { act, render as baseRender, fireEvent, screen, waitFor } from "@testing-library/react";
import { useEffect, useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { bindingFor } from "../lib/keybinds";
import { useStore } from "../lib/store";
import type { AgentCard, Browser, BrowserConsent } from "../lib/types";
import { BrowserScreen, browserReadReducer } from "./BrowserScreen";
import { Search } from "./Search";

const agentBrowser = vi.fn<(id: string) => Promise<Browser | null>>();
const stopAgentBrowser = vi.fn<(id: string) => Promise<void>>();
const giveAgentBrowser = vi.fn<(id: string) => Promise<unknown>>();
const takeAgentBrowser = vi.fn<(id: string) => Promise<unknown>>();
const setAgentBrowserConsent = vi.fn<(id: string, consent: string) => Promise<unknown>>();
const localBrowser = vi.fn<() => Promise<string>>();
const browserDownloadFailed = vi.fn<() => Promise<boolean>>();

vi.mock("../lib/host", async (original) => ({
  ...(await original<typeof import("../lib/host")>()),
  localHost: { browserDownloadFailed: () => browserDownloadFailed() },
}));

vi.mock("../lib/ipc", async (original) => ({
  ...(await original<typeof import("../lib/ipc")>()),
  api: {
    agentBrowser: (id: string) => agentBrowser(id),
    localBrowser: () => localBrowser(),
    giveAgentBrowser: (id: string) => giveAgentBrowser(id),
    takeAgentBrowser: (id: string) => takeAgentBrowser(id),
    setAgentBrowserConsent: (id: string, consent: string) => setAgentBrowserConsent(id, consent),
    startAgentBrowser: vi.fn(),
    stopAgentBrowser: (id: string) => stopAgentBrowser(id),
    search: async () => ({ messages: [], files: [], links: [], routines: [], artifacts: [] }),
    claudeProgram: async () => ({ installed: false, version: null, install: "" }),
  },
}));

function ScreenWorkspace() {
  const [upper, setUpper] = useState<"search" | null>(null);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const binding = bindingFor(event);
      if (binding?.id === "search") {
        event.preventDefault();
        setUpper(binding.id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const close = () => setUpper(null);
  return (
    <>
      <BrowserScreen agent={card("a", "Cook")} />
      {upper === "search" && <Search onClose={close} />}
    </>
  );
}

function card(id: string, name: string, given = true, consent: BrowserConsent = "open"): AgentCard {
  return {
    id,
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: given,
    runsErrands: false,
    browserConsent: consent,
    name,
    avatar: "plain",
    color: "#c7d96b",
    model: "m",
    subscriptionModel: "",
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

const HAS_ONE: Browser = {
  sessionId: "kb-live",
  state: "running",
  liveViewUrl: "https://prod-jfk-1.kernel.sh:8443/browser/live/tok",
  unwatchable: null,
};

describe("browser read transitions", () => {
  it("loads, accepts a result, preserves it on failure, and resets", () => {
    const loading = { status: "loading" } as const;
    const empty = { status: "ready", browser: null } as const;
    const live = { status: "ready", browser: HAS_ONE } as const;
    expect(browserReadReducer(loading, { type: "found", browser: null })).toEqual(empty);
    expect(browserReadReducer(loading, { type: "found", browser: HAS_ONE })).toEqual(live);
    expect(browserReadReducer(loading, { type: "failed" })).toEqual(empty);
    expect(browserReadReducer(empty, { type: "failed" })).toEqual(empty);
    expect(browserReadReducer(live, { type: "failed" })).toEqual(live);
    expect(browserReadReducer(empty, { type: "found", browser: HAS_ONE })).toEqual(live);
    expect(browserReadReducer(live, { type: "found", browser: null })).toEqual(empty);
    expect(browserReadReducer(live, { type: "reset" })).toEqual(loading);
    expect(browserReadReducer(empty, { type: "reset" })).toEqual(loading);
    expect(browserReadReducer(loading, { type: "reset" })).toEqual(loading);
  });
});

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
      provider: "compatible",
      subscriptionModel: "gpt-5.6-luna",
      reasoningEffort: "auto",
      subscriptionModels: ["gpt-5.6-luna", "gpt-5.4-mini"],
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

function render(ui: React.ReactElement) {
  const result = baseRender(ui);
  const disclosure = screen.queryByRole("button", { name: "Browser" });
  if (disclosure?.getAttribute("aria-expanded") === "false") fireEvent.click(disclosure);
  return result;
}

describe("BrowserScreen", () => {
  beforeEach(() => {
    agentBrowser.mockReset();
    stopAgentBrowser.mockReset();
    giveAgentBrowser.mockReset();
    giveAgentBrowser.mockResolvedValue(undefined);
    takeAgentBrowser.mockReset();
    takeAgentBrowser.mockResolvedValue(undefined);
    setAgentBrowserConsent.mockReset();
    setAgentBrowserConsent.mockResolvedValue(undefined);
    configure(true);
  });

  it("keeps an ungiven place to one expandable header", () => {
    baseRender(<BrowserScreen agent={card("a", "Cook", false)} />);
    const disclosure = screen.getByRole("button", { name: "Browser" });
    expect(disclosure.getAttribute("aria-expanded")).toBe("false");
    expect(screen.getByRole("switch", { name: "Browser" })).toBeTruthy();
    fireEvent.click(disclosure);
    expect(disclosure.getAttribute("aria-expanded")).toBe("true");
  });

  it("shows a live picture while its controls are collapsed", async () => {
    agentBrowser.mockResolvedValue(HAS_ONE);
    baseRender(<BrowserScreen agent={card("a", "Cook")} />);
    await waitFor(() => expect(screen.getByTitle("Cook's browser")).toBeTruthy());
    expect(screen.getByRole("button", { name: "Browser" }).getAttribute("aria-expanded")).toBe(
      "false",
    );
    expect(screen.queryByLabelText(/Ask me before it acts in my name/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Browser" }));
    expect(screen.getByLabelText(/Ask me before it acts in my name/)).toBeTruthy();
  });

  it("says nothing at all when no browser provider is configured", async () => {
    configure(false);
    const { container } = render(<BrowserScreen agent={card("a", "Cook")} />);
    await waitFor(() => expect(container.firstChild).toBeNull());
    expect(agentBrowser).not.toHaveBeenCalled();
  });

  it("never shows one agent's browser in another agent's panel", async () => {
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

    expect(screen.getByRole("button", { name: "Browser" }).getAttribute("aria-expanded")).toBe(
      "false",
    );
    fireEvent.click(screen.getByRole("button", { name: "Browser" }));
    await waitFor(() => expect(screen.getByText(/Opens when it needs the web/)).toBeTruthy());
    expect(screen.getByRole("region", { name: "Scribe's browser" })).toBeTruthy();
    expect(screen.queryByTitle("Cook's browser")).toBeNull();
  });

  it("does not restore a browser from a poll started before close", async () => {
    let settle: (value: Browser | null) => void = () => {};
    agentBrowser.mockResolvedValueOnce(HAS_ONE).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          settle = resolve;
        }),
    );
    stopAgentBrowser.mockResolvedValue(undefined);
    vi.useFakeTimers();
    try {
      render(<BrowserScreen agent={card("a", "Cook")} />);
      await act(async () => {});
      act(() => vi.advanceTimersByTime(20000));
      fireEvent.click(screen.getByRole("button", { name: /take over/i }));
      fireEvent.click(screen.getByRole("button", { name: "Close" }));
      fireEvent.click(screen.getByRole("button", { name: /Close it and save/ }));
      await act(async () => {});
      expect(screen.queryByTitle("Cook's browser")).toBeNull();
      await act(async () => settle(HAS_ONE));
      expect(screen.queryByTitle("Cook's browser")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("discards a lookup from the first visit after A to B to A", async () => {
    let settle: (value: Browser | null) => void = () => {};
    agentBrowser
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            settle = resolve;
          }),
      )
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce(null);
    const view = render(<BrowserScreen agent={card("a", "Cook")} />);
    view.rerender(<BrowserScreen agent={card("b", "Scribe")} />);
    view.rerender(<BrowserScreen agent={card("a", "Cook")} />);
    await act(async () => {});
    await act(async () => settle(HAS_ONE));
    expect(screen.queryByTitle("Cook's browser")).toBeNull();
  });

  it("polls every 20 seconds and stops on unmount", async () => {
    agentBrowser.mockResolvedValue(null);
    vi.useFakeTimers();
    try {
      const view = render(<BrowserScreen agent={card("a", "Cook")} />);
      await act(async () => {});
      expect(agentBrowser).toHaveBeenCalledTimes(1);
      await act(async () => vi.advanceTimersByTime(19999));
      expect(agentBrowser).toHaveBeenCalledTimes(1);
      await act(async () => vi.advanceTimersByTime(1));
      expect(agentBrowser).toHaveBeenCalledTimes(2);
      view.unmount();
      await act(async () => vi.advanceTimersByTime(40000));
      expect(agentBrowser).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps the newest poll when an older poll finishes later", async () => {
    let settle: (value: Browser | null) => void = () => {};
    agentBrowser
      .mockResolvedValueOnce(null)
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            settle = resolve;
          }),
      )
      .mockResolvedValueOnce(null);
    vi.useFakeTimers();
    try {
      render(<BrowserScreen agent={card("a", "Cook")} />);
      await act(async () => {});
      await act(async () => vi.advanceTimersByTime(40000));
      await act(async () => settle(HAS_ONE));
      expect(screen.queryByTitle("Cook's browser")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("invalidates a poll started while close is pending", async () => {
    let finishClose: () => void = () => {};
    let finishPoll: (value: Browser | null) => void = () => {};
    agentBrowser.mockResolvedValueOnce(HAS_ONE).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finishPoll = resolve;
        }),
    );
    stopAgentBrowser.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finishClose = resolve;
        }),
    );
    vi.useFakeTimers();
    try {
      render(<BrowserScreen agent={card("a", "Cook")} />);
      await act(async () => {});
      fireEvent.click(screen.getByRole("button", { name: /take over/i }));
      fireEvent.click(screen.getByRole("button", { name: "Close" }));
      fireEvent.click(screen.getByRole("button", { name: /Close it and save/ }));
      act(() => vi.advanceTimersByTime(20000));
      await act(async () => finishClose());
      await act(async () => finishPoll(HAS_ONE));
      expect(screen.queryByTitle("Cook's browser")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("preserves the live browser when a poll fails and recovers on the next poll", async () => {
    agentBrowser
      .mockResolvedValueOnce(HAS_ONE)
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce(null);
    vi.useFakeTimers();
    try {
      render(<BrowserScreen agent={card("a", "Cook")} />);
      await act(async () => {});
      await act(async () => vi.advanceTimersByTime(20000));
      expect(screen.getByTitle("Cook's browser")).toBeTruthy();
      await act(async () => vi.advanceTimersByTime(20000));
      expect(screen.queryByTitle("Cook's browser")).toBeNull();
      expect(screen.queryByText("offline")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows an initial lookup error rather than staying in loading", async () => {
    agentBrowser.mockRejectedValue(new Error("offline"));
    render(<BrowserScreen agent={card("a", "Cook")} />);
    expect(await screen.findByText("offline")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open" })).toBeTruthy();
  });

  it("ignores an earlier visit's consent failure while a new decision is pending", async () => {
    let rejectOld: (error: Error) => void = () => {};
    let finishNew: () => void = () => {};
    agentBrowser.mockResolvedValue(null);
    setAgentBrowserConsent
      .mockImplementationOnce(
        () =>
          new Promise((_, reject) => {
            rejectOld = reject;
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finishNew = () => resolve(undefined);
          }),
      );
    const view = render(<BrowserScreen agent={card("a", "Cook")} />);
    await act(async () => {});
    fireEvent.click(screen.getByLabelText(/Ask me before it acts in my name/));
    view.rerender(<BrowserScreen agent={card("b", "Scribe")} />);
    view.rerender(<BrowserScreen agent={card("a", "Cook")} />);
    await act(async () => {});
    fireEvent.click(screen.getByRole("button", { name: "Browser" }));
    const consent = screen.getByLabelText(/Ask me before it acts in my name/) as HTMLInputElement;
    fireEvent.click(consent);
    expect(consent.disabled).toBe(true);
    await act(async () => rejectOld(new Error("old decision failed")));
    expect(consent.disabled).toBe(true);
    expect(screen.queryByText("old decision failed")).toBeNull();
    await act(async () => finishNew());
    expect(consent.disabled).toBe(false);
  });

  it("explains that signing the agent in is the operator's job", async () => {
    agentBrowser.mockResolvedValue(null);
    render(<BrowserScreen agent={card("a", "Cook")} />);

    expect(await screen.findByText(/sign this agent in/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open" })).toBeTruthy();
  });

  it("offers a browser to an agent that has not been given one, and asks nobody", async () => {
    render(<BrowserScreen agent={card("a", "Cook", false)} />);

    const grant = await screen.findByRole("switch", { name: "Browser" });
    expect(grant.getAttribute("aria-checked")).toBe("false");
    expect(agentBrowser).not.toHaveBeenCalled();

    fireEvent.click(grant);
    await waitFor(() => expect(giveAgentBrowser).toHaveBeenCalledWith("a"));
  });

  it("names an address it cannot frame, rather than drawing a black rectangle", async () => {
    agentBrowser.mockResolvedValue({
      sessionId: "kb-live",
      state: "running",
      liveViewUrl: null,
      unwatchable: "https://prod-jfk-1.example.com:8443",
    });
    render(<BrowserScreen agent={card("a", "Cook")} />);

    expect(await screen.findByText(/prod-jfk-1\.example\.com:8443/)).toBeTruthy();
    expect(screen.getByText(/still use the web/)).toBeTruthy();
    expect(screen.queryByTitle("Cook's browser")).toBeNull();
  });

  it("takes it back from the switch beside its name, and from nowhere else", async () => {
    agentBrowser.mockResolvedValue(HAS_ONE);
    render(<BrowserScreen agent={card("a", "Cook")} />);

    fireEvent.click(await screen.findByRole("button", { name: /take over/i }));
    expect(screen.queryByRole("button", { name: /Take it back/ })).toBeNull();

    fireEvent.click(screen.getByRole("switch", { name: "Browser" }));
    await waitFor(() => expect(takeAgentBrowser).toHaveBeenCalledWith("a"));
  });

  it("keeps the live view off the panel of an agent whose browser was taken back", async () => {
    agentBrowser.mockResolvedValue(HAS_ONE);
    const view = render(<BrowserScreen agent={card("a", "Cook")} />);
    await waitFor(() => expect(screen.getByTitle("Cook's browser")).toBeTruthy());

    view.rerender(<BrowserScreen agent={card("a", "Cook", false)} />);
    await waitFor(() => expect(screen.queryByTitle("Cook's browser")).toBeNull());
    expect(screen.getByRole("switch", { name: "Browser" }).getAttribute("aria-checked")).toBe(
      "false",
    );
  });

  it("keeps a click out of the browser until the operator asks for it", async () => {
    agentBrowser.mockResolvedValue(HAS_ONE);
    render(<BrowserScreen agent={card("a", "Cook")} />);

    const veil = await screen.findByRole("button", { name: /take over/i });
    fireEvent.click(veil);

    expect(screen.queryByRole("button", { name: /take over/i })).toBeNull();
    expect(screen.getByRole("dialog", { name: "Cook's browser" })).toBeTruthy();
  });

  it.each(["Escape", "button"])("returns focus to the browser control after %s", async (how) => {
    agentBrowser.mockResolvedValue(HAS_ONE);
    render(<BrowserScreen agent={card("a", "Cook")} />);
    const veil = await screen.findByRole("button", { name: /take over/i });
    veil.focus();
    fireEvent.click(veil);
    if (how === "Escape") fireEvent.keyDown(window, { key: "Escape" });
    else fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(document.activeElement).toBe(screen.getByRole("button", { name: /take over/i }));
  });

  it.each([["Search", "f"]])(
    "keeps the expanded browser under %s and restores focus",
    async (name, key) => {
      agentBrowser.mockResolvedValue(HAS_ONE);
      render(<ScreenWorkspace />);
      const veil = await screen.findByRole("button", { name: /take over/i });
      veil.focus();
      fireEvent.click(veil);
      const browser = screen.getByRole("dialog", { name: "Cook's browser" });
      const done = screen.getByRole("button", { name: "Done" });
      done.focus();
      fireEvent.keyDown(window, { key, ctrlKey: true });
      const upper = await screen.findByRole("dialog", { name });
      await waitFor(() => expect(browser.contains(document.activeElement)).toBe(false));
      fireEvent.keyDown(window, { key: "Escape" });
      expect(upper.isConnected).toBe(false);
      expect(screen.getByRole("dialog", { name: "Cook's browser" })).toBe(browser);
      expect(document.activeElement).toBe(done);
      fireEvent.keyDown(window, { key: "Escape" });
      expect(screen.queryByRole("dialog", { name: "Cook's browser" })).toBeNull();
      expect(document.activeElement).toBe(screen.getByRole("button", { name: /take over/i }));
    },
  );

  it("keeps the same connection open across the change of size", async () => {
    agentBrowser.mockResolvedValue(HAS_ONE);
    render(<BrowserScreen agent={card("a", "Cook")} />);

    const before = await screen.findByTitle("Cook's browser");
    fireEvent.click(screen.getByRole("button", { name: /take over/i }));
    expect(screen.getByTitle("Cook's browser")).toBe(before);
  });

  it("hands the keyboard to the live view when the operator takes over", async () => {
    agentBrowser.mockResolvedValue(HAS_ONE);
    render(<BrowserScreen agent={card("a", "Cook")} />);

    const frame = await screen.findByTitle("Cook's browser");
    const focused = vi.spyOn(frame as HTMLIFrameElement, "focus");

    fireEvent.click(screen.getByRole("button", { name: /take over/i }));
    expect(focused).toHaveBeenCalled();
  });

  it("lets a paste reach the page, because signing in means pasting a password", async () => {
    agentBrowser.mockResolvedValue(HAS_ONE);
    render(<BrowserScreen agent={card("a", "Cook")} />);

    const frame = await screen.findByTitle("Cook's browser");
    expect(frame.getAttribute("allow")).toContain("clipboard-write");
  });

  it("offers the consent switch while the browser is working, not only when it is closed", async () => {
    agentBrowser.mockResolvedValue(HAS_ONE);
    render(<BrowserScreen agent={card("a", "Cook")} />);

    const box = await screen.findByLabelText(/Ask me before it acts in my name/);
    expect((box as HTMLInputElement).checked).toBe(false);

    fireEvent.click(box);
    await waitFor(() =>
      expect(setAgentBrowserConsent).toHaveBeenCalledWith("a", "askBeforeActing"),
    );
  });

  it("switches an agent that is being asked about back to acting on its own", async () => {
    agentBrowser.mockResolvedValue(null);
    render(<BrowserScreen agent={card("a", "Cook", true, "askBeforeActing")} />);

    const box = await screen.findByLabelText(/Ask me before it acts in my name/);
    expect((box as HTMLInputElement).checked).toBe(true);

    fireEvent.click(box);
    await waitFor(() => expect(setAgentBrowserConsent).toHaveBeenCalledWith("a", "open"));
  });

  it("says nothing about consent for an agent that has no browser", async () => {
    agentBrowser.mockResolvedValue(null);
    render(<BrowserScreen agent={card("a", "Cook", false)} />);

    await screen.findByRole("switch", { name: "Browser" });
    expect(screen.queryByLabelText(/Ask me before it acts in my name/)).toBeNull();
  });

  it("does not close a browser on one click", async () => {
    agentBrowser.mockResolvedValue(HAS_ONE);
    render(<BrowserScreen agent={card("a", "Cook")} />);

    fireEvent.click(await screen.findByRole("button", { name: /take over/i }));
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(stopAgentBrowser).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: /Close it and save/ }));
    await waitFor(() => expect(stopAgentBrowser).toHaveBeenCalledWith("a"));
  });
});

describe("BrowserScreen with a browser on this Mac and no Kernel key", () => {
  const besideTheHost = (localBrowser: boolean) =>
    useStore.setState((s) => ({ capabilities: { ...s.capabilities, localBrowser } }));

  beforeEach(() => {
    agentBrowser.mockReset();
    localBrowser.mockReset();
    browserDownloadFailed.mockReset();
    browserDownloadFailed.mockResolvedValue(false);
    configure(false);
    besideTheHost(true);
  });

  afterEach(() => besideTheHost(false));

  it("says the browser is being set up and asks for no browser until it is ready", async () => {
    localBrowser.mockResolvedValue("settingUp");
    render(<BrowserScreen agent={card("a", "Scout")} />);
    expect(await screen.findByText(/Setting up the browser on this Mac/)).toBeTruthy();
    expect(screen.getByText(/Scout is told to try again shortly/)).toBeTruthy();
    expect(screen.getByText("setting up")).toBeTruthy();
    expect(agentBrowser).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "Open" })).toBeNull();
  });

  it("says a failed download failed, that it is being tried again, and keeps asking", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      localBrowser.mockResolvedValue("settingUp");
      browserDownloadFailed.mockResolvedValue(true);
      render(<BrowserScreen agent={card("a", "Scout")} />);
      expect(await screen.findByText(/could not be downloaded to this Mac/)).toBeTruthy();
      expect(screen.getByText(/tries again on its own every minute/)).toBeTruthy();
      expect(screen.getByText("retrying")).toBeTruthy();
      expect(screen.queryByText(/Setting up the browser/)).toBeNull();

      browserDownloadFailed.mockResolvedValue(false);
      localBrowser.mockResolvedValue("ready");
      agentBrowser.mockResolvedValue({
        sessionId: "page-0",
        state: "running",
        liveViewUrl: null,
        unwatchable: null,
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(5000);
      });
      expect(await screen.findByText(/Public web only, and no live view/)).toBeTruthy();
      expect(screen.queryByText(/could not be downloaded/)).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("says what the browser on this Mac cannot do once it is ready", async () => {
    localBrowser.mockResolvedValue("ready");
    agentBrowser.mockResolvedValue({
      sessionId: "page-0",
      state: "running",
      liveViewUrl: null,
      unwatchable: null,
    });
    render(<BrowserScreen agent={card("a", "Scout")} />);
    expect(await screen.findByText(/Public web only, and no live view/)).toBeTruthy();
    expect(await screen.findByText("running")).toBeTruthy();
    expect(screen.queryByTitle(/Open Scout's browser/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Open" })).toBeNull();
  });

  it("offers it to an agent that has none, as a browser on this Mac", async () => {
    render(<BrowserScreen agent={card("a", "Scout", false)} />);
    expect(screen.getByRole("switch", { name: "Browser" }).getAttribute("title")).toContain(
      "browser on this Mac",
    );
    expect(localBrowser).not.toHaveBeenCalled();
  });

  it("leaves a workspace with a Kernel key exactly as it was", async () => {
    configure(true);
    agentBrowser.mockResolvedValue(HAS_ONE);
    render(<BrowserScreen agent={card("a", "Scout")} />);
    await waitFor(() => expect(agentBrowser).toHaveBeenCalled());
    expect(localBrowser).not.toHaveBeenCalled();
    expect(screen.queryByText(/Public web only/)).toBeNull();
  });

  it("says nothing on a host with neither a key nor a browser of its own", () => {
    besideTheHost(false);
    const { container } = render(<BrowserScreen agent={card("a", "Scout")} />);
    expect(container.innerHTML).toBe("");
  });
});
