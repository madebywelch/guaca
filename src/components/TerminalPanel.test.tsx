import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard, Gate, Harness, HarnessOnMachine } from "../lib/types";
import { TerminalPanel } from "./TerminalPanel";

const agentTerminal = vi.fn<(id: string) => Promise<{ path: string }>>();
const codingHarnesses = vi.fn<() => Promise<HarnessOnMachine[]>>();
const giveAgentTerminal = vi.fn<(id: string) => Promise<void>>();
const takeAgentTerminal = vi.fn<(id: string) => Promise<void>>();
const setAgentCoding = vi.fn<(id: string, harness: Harness, gate: Gate) => Promise<void>>();

vi.mock("../lib/ipc", () => ({
  api: {
    agentTerminal: (id: string) => agentTerminal(id),
    codingHarnesses: () => codingHarnesses(),
    giveAgentTerminal: (id: string) => giveAgentTerminal(id),
    takeAgentTerminal: (id: string) => takeAgentTerminal(id),
    setAgentCoding: (id: string, harness: Harness, gate: Gate) => setAgentCoding(id, harness, gate),
  },
}));

function card(given: boolean, harness: Harness = "claude", gate: Gate = "open"): AgentCard {
  return {
    id: "a1",
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    browserConsent: "open",
    hasTerminal: given,
    harness,
    gate,
    name: "Engineer",
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

function row(harness: Harness, over: Partial<HarnessOnMachine> = {}): HarnessOnMachine {
  return {
    harness,
    installed: true,
    version: "1.0.0",
    bridged: true,
    install: `npm install -g ${harness}`,
    withheld: null,
    signedIn: true,
    signIn: `${harness} login`,
    ...over,
  };
}

describe("TerminalPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    agentTerminal.mockResolvedValue({ path: "/var/lib/guaca/data/terminals/a1" });
    codingHarnesses.mockResolvedValue([row("codex"), row("claude"), row("pi")]);
    giveAgentTerminal.mockResolvedValue(undefined);
    takeAgentTerminal.mockResolvedValue(undefined);
    setAgentCoding.mockResolvedValue(undefined);
  });

  it("offers a terminal to an agent that has none, and asks the backend nothing", async () => {
    // Nothing about a directory that does not exist yet could change what is
    // drawn, and asking would make one.
    render(<TerminalPanel agent={card(false)} />);
    expect(screen.getByText(/Engineer has no terminal/)).toBeTruthy();
    expect(agentTerminal).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Give one" }));
    await waitFor(() => expect(giveAgentTerminal).toHaveBeenCalledWith("a1"));
  });

  it("says where the directory is, and that it is not a sandbox", async () => {
    render(<TerminalPanel agent={card(true)} />);
    expect(await screen.findByText("/var/lib/guaca/data/terminals/a1")).toBeTruthy();
    expect(screen.getByText(/This is not a sandbox/)).toBeTruthy();
  });

  it("switches the harness on the click, keeping the gate it had", async () => {
    // A switch made because a plan just ran out is the change most likely to
    // be lost under a Save button, so the click is the change.
    render(<TerminalPanel agent={card(true, "claude", "askBeforePushing")} />);
    await screen.findByText("/var/lib/guaca/data/terminals/a1");

    fireEvent.click(screen.getByRole("button", { name: "Coding harness: Codex" }));
    await waitFor(() =>
      expect(setAgentCoding).toHaveBeenCalledWith("a1", "codex", "askBeforePushing"),
    );
  });

  it("turns the gate on without touching the harness", async () => {
    render(<TerminalPanel agent={card(true, "pi", "open")} />);
    await screen.findByText("/var/lib/guaca/data/terminals/a1");

    fireEvent.click(screen.getByRole("checkbox", { name: /Ask me before pushing/ }));
    await waitFor(() =>
      expect(setAgentCoding).toHaveBeenCalledWith("a1", "pi", "askBeforePushing"),
    );
  });

  it("draws a harness that is not installed, disabled, with the command that installs it", async () => {
    codingHarnesses.mockResolvedValue([
      row("codex", { installed: false }),
      row("claude"),
      row("pi"),
    ]);
    render(<TerminalPanel agent={card(true, "codex")} />);

    await waitFor(() =>
      expect(
        (screen.getByRole("button", { name: "Coding harness: Codex" }) as HTMLButtonElement)
          .disabled,
      ).toBe(true),
    );
    expect(screen.getByText("npm install -g codex")).toBeTruthy();
  });

  it("names the sign-in a signed-out harness needs, and that Guaca's own does not count", async () => {
    codingHarnesses.mockResolvedValue([
      row("codex"),
      row("claude", { signedIn: false }),
      row("pi"),
    ]);
    render(<TerminalPanel agent={card(true, "claude")} />);

    expect(await screen.findByText("claude login")).toBeTruthy();
    expect(screen.getByText(/does not sign in the coding tool/)).toBeTruthy();
  });

  it("takes it back, and says the directory is kept", async () => {
    render(<TerminalPanel agent={card(true)} />);
    const take = await screen.findByRole("button", { name: "Take it back" });
    expect(take.getAttribute("title")).toMatch(/directory is kept/);

    fireEvent.click(take);
    await waitFor(() => expect(takeAgentTerminal).toHaveBeenCalledWith("a1"));
  });

  it("shows why a change was refused rather than looking like it landed", async () => {
    setAgentCoding.mockRejectedValue({ kind: "notFound", message: "no agent with id a1" });
    render(<TerminalPanel agent={card(true)} />);
    await screen.findByText("/var/lib/guaca/data/terminals/a1");

    fireEvent.click(screen.getByRole("button", { name: "Coding harness: pi" }));
    expect((await screen.findByRole("alert")).textContent).toContain("no agent with id a1");
  });
});
