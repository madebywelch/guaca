import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type {
  AgentCard,
  CodingSession,
  Continued,
  Gate,
  GuacaKey,
  Harness,
  HarnessOnMachine,
  ModelOffer,
  Payer,
  TerminalView,
  Tuning,
} from "../lib/types";
import { TerminalPanel } from "./TerminalPanel";

const agentTerminal = vi.fn<(id: string) => Promise<TerminalView>>();
const messageCodingJob = vi.fn<(id: string, message: string) => Promise<Continued>>();
const codingHarnesses = vi.fn<() => Promise<HarnessOnMachine[]>>();
const giveAgentTerminal = vi.fn<(id: string) => Promise<void>>();
const takeAgentTerminal = vi.fn<(id: string) => Promise<void>>();
const setAgentCoding = vi.fn<(id: string, harness: Harness, gate: Gate) => Promise<void>>();
const setCodingTuning = vi.fn<(id: string, harness: Harness, tuning: Tuning) => Promise<void>>();
const codingModels = vi.fn<(harness: Harness, pays: Payer) => Promise<ModelOffer[]>>();

vi.mock("../lib/ipc", () => ({
  api: {
    agentTerminal: (id: string) => agentTerminal(id),
    codingHarnesses: () => codingHarnesses(),
    giveAgentTerminal: (id: string) => giveAgentTerminal(id),
    takeAgentTerminal: (id: string) => takeAgentTerminal(id),
    setAgentCoding: (id: string, harness: Harness, gate: Gate) => setAgentCoding(id, harness, gate),
    messageCodingJob: (id: string, message: string) => messageCodingJob(id, message),
    setCodingTuning: (id: string, harness: Harness, tuning: Tuning) =>
      setCodingTuning(id, harness, tuning),
    codingModels: (harness: Harness, pays: Payer) => codingModels(harness, pays),
  },
}));

// The shell draws on a canvas jsdom does not have, and `Console.test.tsx` is
// where its wiring is checked. Here it is what the panel opens and closes.
vi.mock("./Console", async () => {
  const { createElement } = await import("react");
  return {
    Console: ({ agent, path, onClose }: { agent: AgentCard; path: string; onClose: () => void }) =>
      createElement(
        "div",
        { role: "dialog", "aria-label": `${agent.name}'s terminal`, "data-path": path },
        createElement("button", { type: "button", onClick: onClose }, "Done"),
      ),
  };
});

const PATH = "/var/lib/guaca/data/terminals/a1";

const NO_KEY: GuacaKey = {
  set: false,
  endpoint: "https://openrouter.ai/api/v1",
  openrouter: true,
  defaultModel: "qwen/qwen3-coder",
};

/** A terminal view with nothing chosen in any harness. */
function view(over: Partial<TerminalView> = {}): TerminalView {
  return {
    path: PATH,
    session: null,
    resume: null,
    tunings: (["codex", "claude", "pi"] as const).map((harness) => ({
      harness,
      model: null,
      effort: null,
      pays: "own",
    })),
    guacaKey: NO_KEY,
    ...over,
  };
}

function offer(id: string, over: Partial<ModelOffer> = {}): ModelOffer {
  return { id, label: id, detail: "", default: false, efforts: [], ...over };
}

function session(harness: Harness = "claude"): CodingSession {
  return { harness, id: "s1", directory: "site", updatedAt: Date.now() };
}

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
    efforts: ["low", "medium", "high"],
    ...over,
  };
}

describe("TerminalPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useStore.setState({ building: {} });
    agentTerminal.mockResolvedValue(view());
    messageCodingJob.mockResolvedValue({ kind: "resumed", directory: "site" });
    setCodingTuning.mockResolvedValue(undefined);
    codingModels.mockResolvedValue([]);
    codingHarnesses.mockResolvedValue([row("codex"), row("claude"), row("pi")]);
    giveAgentTerminal.mockResolvedValue(undefined);
    takeAgentTerminal.mockResolvedValue(undefined);
    setAgentCoding.mockResolvedValue(undefined);
  });

  it("offers a terminal to an agent that has none, and asks the backend nothing", async () => {
    // Nothing about a directory that does not exist yet could change what is
    // drawn, and asking would make one.
    render(<TerminalPanel agent={card(false)} />);
    const grant = screen.getByRole("switch", { name: "Terminal" });
    expect(grant.getAttribute("aria-checked")).toBe("false");
    expect(agentTerminal).not.toHaveBeenCalled();

    fireEvent.click(grant);
    await waitFor(() => expect(giveAgentTerminal).toHaveBeenCalledWith("a1"));
  });

  it("says it is not a sandbox where the decision to give one is made", () => {
    render(<TerminalPanel agent={card(false)} />);
    expect(screen.getByRole("switch", { name: "Terminal" }).getAttribute("title")).toMatch(
      /not a sandbox/,
    );
  });

  it("offers no shell to an agent with no terminal", () => {
    render(<TerminalPanel agent={card(false)} />);
    expect(screen.queryByRole("button", { name: "Open terminal" })).toBeNull();
  });

  it("offers no shell before it knows where the directory is", () => {
    agentTerminal.mockReturnValue(new Promise(() => {}));
    render(<TerminalPanel agent={card(true)} />);
    const open = screen.getByRole("button", { name: "Open terminal" }) as HTMLButtonElement;
    expect(open.disabled).toBe(true);
  });

  it("opens a shell in the agent's directory, and hands the keyboard back when it closes", async () => {
    render(<TerminalPanel agent={card(true)} />);
    await screen.findByText(PATH);

    const open = screen.getByRole("button", { name: "Open terminal" });
    expect(open.getAttribute("title")).toMatch(/gh auth login, is every agent's/);
    fireEvent.click(open);
    const shell = screen.getByRole("dialog", { name: "Engineer's terminal" });
    expect(shell.getAttribute("data-path")).toBe(PATH);

    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(open);
  });

  it("closes the shell when the terminal is taken back", async () => {
    const { rerender } = render(<TerminalPanel agent={card(true)} />);
    await screen.findByText(PATH);
    fireEvent.click(screen.getByRole("button", { name: "Open terminal" }));
    expect(screen.getByRole("dialog")).toBeTruthy();

    rerender(<TerminalPanel agent={card(false)} />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("says where the directory is, whole, however narrow the panel", async () => {
    // Cut at the column's edge on screen, so the whole of it has to be one
    // hover away.
    render(<TerminalPanel agent={card(true)} />);
    expect((await screen.findByText(PATH)).getAttribute("title")).toBe(PATH);
  });

  it("explains nothing while every part of it works", async () => {
    // The section used to be a paragraph per control, and read as a manual
    // with the controls somewhere inside it. A hint is for something wrong,
    // and the gate's one line is the only other sentence it keeps.
    codingModels.mockResolvedValue([offer("gpt-6-astra", { default: true })]);
    const { container } = render(<TerminalPanel agent={card(true, "codex")} />);
    await screen.findByText(PATH);
    await waitFor(() => expect(codingModels).toHaveBeenCalled());
    await waitFor(() =>
      expect((screen.getByLabelText("Codex model") as HTMLInputElement).placeholder).toBe(
        "gpt-6-astra",
      ),
    );

    const hints = Array.from(container.querySelectorAll(".field__hint")).map(
      (hint) => hint.textContent,
    );
    expect(hints).toEqual(["Pushes, pull requests, merges and releases wait for your approval."]);
  });

  it("switches the harness on the click, keeping the gate it had", async () => {
    // A switch made because a plan just ran out is the change most likely to
    // be lost under a Save button, so the click is the change.
    render(<TerminalPanel agent={card(true, "claude", "askBeforePushing")} />);
    await screen.findByText(PATH);

    fireEvent.click(screen.getByRole("button", { name: "Coding harness: Codex" }));
    await waitFor(() =>
      expect(setAgentCoding).toHaveBeenCalledWith("a1", "codex", "askBeforePushing"),
    );
  });

  it("turns the gate on without touching the harness", async () => {
    render(<TerminalPanel agent={card(true, "pi", "open")} />);
    await screen.findByText(PATH);

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
    expect(screen.getByText(/Open the terminal and run/)).toBeTruthy();
    expect(screen.getByText(/does not sign in the coding tool/)).toBeTruthy();
  });

  it("takes it back from the switch beside its name, and says the directory is kept", async () => {
    // The button this replaced said "Take it back" without saying what, and
    // sat at the foot of the section above the next heading, where it read as
    // belonging to the schedule.
    render(<TerminalPanel agent={card(true)} />);
    await screen.findByText(PATH);
    expect(screen.queryByRole("button", { name: /Take it back/ })).toBeNull();

    const grant = screen.getByRole("switch", { name: "Terminal" });
    expect(grant.getAttribute("aria-checked")).toBe("true");
    expect(grant.getAttribute("title")).toMatch(/directory is kept/);

    fireEvent.click(grant);
    await waitFor(() => expect(takeAgentTerminal).toHaveBeenCalledWith("a1"));
  });

  it("offers the last session's resume line and carries it on from a follow-up", async () => {
    agentTerminal.mockResolvedValue(
      view({ session: session("claude"), resume: `cd '${PATH}/site' && claude --resume s1` }),
    );
    render(<TerminalPanel agent={card(true, "claude")} />);

    expect(await screen.findByText(`cd '${PATH}/site' && claude --resume s1`)).toBeTruthy();
    expect(screen.getByText(/Claude Code in site, just now/)).toBeTruthy();
    const box = screen.getByLabelText("Send a follow-up to the last coding session");
    fireEvent.change(box, { target: { value: "  now add a test for it  " } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    await waitFor(() =>
      expect(messageCodingJob).toHaveBeenCalledWith("a1", "now add a test for it"),
    );
    expect(await screen.findByText(/Engineer is told what comes of it/)).toBeTruthy();
    expect((box as HTMLInputElement).value).toBe("");
  });

  it("keeps a refused follow-up in the box and says why", async () => {
    agentTerminal.mockResolvedValue(view({ session: session("pi") }));
    messageCodingJob.mockRejectedValue({
      kind: "invalid",
      message: "the last coding session worked in `site`, which is not in the terminal any more",
    });
    render(<TerminalPanel agent={card(true, "pi")} />);

    const box = await screen.findByLabelText("Send a follow-up to the last coding session");
    fireEvent.change(box, { target: { value: "carry on" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    expect(await screen.findByText(/not in the terminal any more/)).toBeTruthy();
    expect((box as HTMLInputElement).value).toBe("carry on");
  });

  it("does not offer to carry on a session the chosen harness cannot read", async () => {
    // A switch made because a plan ran out is the ordinary case, and the
    // other program has never heard of the session id.
    agentTerminal.mockResolvedValue(view({ session: session("claude") }));
    render(<TerminalPanel agent={card(true, "codex")} />);

    expect(await screen.findByText(/Codex cannot carry on a Claude Code session/)).toBeTruthy();
    expect(screen.queryByLabelText("Send a follow-up to the last coding session")).toBeNull();
  });

  it("leaves a running job to its channel, and reads the session again when it ends", async () => {
    agentTerminal.mockResolvedValue(view({ session: session("claude") }));
    useStore.setState({ building: { a1: "site" } });
    render(<TerminalPanel agent={card(true, "claude")} />);

    expect(await screen.findByText(/Steer it from Engineer's channel/)).toBeTruthy();
    expect(screen.queryByLabelText("Send a follow-up to the last coding session")).toBeNull();
    const before = agentTerminal.mock.calls.length;

    act(() => useStore.setState({ building: {} }));
    await waitFor(() => expect(agentTerminal.mock.calls.length).toBe(before + 1));
    expect(
      await screen.findByLabelText("Send a follow-up to the last coding session"),
    ).toBeTruthy();
  });

  it("suggests the program's own models and saves the one typed on Enter", async () => {
    codingModels.mockResolvedValue([
      offer("claude-fable-5-1", { label: "Fable 5.1", default: true, efforts: ["low", "max"] }),
      offer("haiku", { label: "Haiku 4.5" }),
    ]);
    render(<TerminalPanel agent={card(true, "claude")} />);

    const field = await screen.findByLabelText("Claude Code model");
    await waitFor(() => expect(codingModels).toHaveBeenCalledWith("claude", "own"));
    // What runs when nothing is chosen, in the program's own name for it.
    await waitFor(() => expect((field as HTMLInputElement).placeholder).toBe("Fable 5.1"));
    const suggested = document.getElementById(field.getAttribute("list") ?? "");
    expect(Array.from(suggested?.querySelectorAll("option") ?? []).map((o) => o.value)).toEqual([
      "claude-fable-5-1",
      "haiku",
    ]);

    fireEvent.change(field, { target: { value: " haiku " } });
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() =>
      expect(setCodingTuning).toHaveBeenCalledWith("a1", "claude", {
        model: "haiku",
        effort: null,
        pays: "own",
      }),
    );
    expect(await screen.findByText(/The next job runs with it/)).toBeTruthy();
  });

  it("offers the chosen model's own efforts rather than every word the program knows", async () => {
    agentTerminal.mockResolvedValue(
      view({
        tunings: [{ harness: "codex", model: "gpt-6-astra", effort: null, pays: "own" }],
      }),
    );
    codingModels.mockResolvedValue([
      offer("gpt-6-astra", { default: true, efforts: ["low", "ultra"] }),
    ]);
    render(<TerminalPanel agent={card(true, "codex")} />);

    const effort = (await screen.findByLabelText("Codex effort")) as HTMLSelectElement;
    await waitFor(() =>
      expect(Array.from(effort.options).map((option) => option.value)).toEqual([
        "",
        "low",
        "ultra",
      ]),
    );
    fireEvent.change(effort, { target: { value: "ultra" } });
    await waitFor(() =>
      expect(setCodingTuning).toHaveBeenCalledWith("a1", "codex", {
        model: "gpt-6-astra",
        effort: "ultra",
        pays: "own",
      }),
    );
  });

  it("lends Guaca's key to pi only when there is one, and says pi never holds it", async () => {
    render(<TerminalPanel agent={card(true, "pi")} />);
    const lend = await screen.findByRole("button", { name: "Paid for by: Guaca's API key" });
    expect((lend as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/Guaca has no API key in Settings > Provider/)).toBeTruthy();
  });

  it("switches pi to Guaca's key without carrying over a model named for its own sign-in", async () => {
    agentTerminal.mockResolvedValue(
      view({
        guacaKey: { ...NO_KEY, set: true },
        tunings: [{ harness: "pi", model: "anthropic/claude-opus-4-7", effort: null, pays: "own" }],
      }),
    );
    render(<TerminalPanel agent={card(true, "pi")} />);

    fireEvent.click(await screen.findByRole("button", { name: "Paid for by: Guaca's API key" }));
    // A model named for pi's own sign-in is not one the key can be assumed to
    // reach, so it goes back to the key's own.
    await waitFor(() =>
      expect(setCodingTuning).toHaveBeenCalledWith("a1", "pi", {
        model: null,
        effort: null,
        pays: "guacaKey",
      }),
    );
    // And the view is read again, so what is drawn is what was stored.
    await waitFor(() => expect(agentTerminal.mock.calls.length).toBeGreaterThan(1));
  });

  it("draws a pi on Guaca's key with the endpoint and the key's own model", async () => {
    agentTerminal.mockResolvedValue(
      view({
        guacaKey: { ...NO_KEY, set: true },
        tunings: [{ harness: "pi", model: null, effort: null, pays: "guacaKey" }],
      }),
    );
    render(<TerminalPanel agent={card(true, "pi")} />);
    expect(await screen.findByText(/never the key/)).toBeTruthy();
    expect(screen.getByText("https://openrouter.ai/api/v1")).toBeTruthy();
    const field = screen.getByLabelText("pi model") as HTMLInputElement;
    expect(field.placeholder).toContain("qwen/qwen3-coder");
    await waitFor(() => expect(codingModels).toHaveBeenCalledWith("pi", "guacaKey"));
  });

  it("keeps the model field usable when the program could not list its models", async () => {
    codingModels.mockRejectedValue({
      kind: "unavailable",
      message:
        "pi did not list its models within 20 seconds. The model field still takes a name typed by hand",
    });
    render(<TerminalPanel agent={card(true, "pi")} />);
    expect(await screen.findByText(/typed by hand/)).toBeTruthy();

    const field = screen.getByLabelText("pi model");
    fireEvent.change(field, { target: { value: "openai/gpt-5.5" } });
    fireEvent.blur(field);
    await waitFor(() =>
      expect(setCodingTuning).toHaveBeenCalledWith("a1", "pi", {
        model: "openai/gpt-5.5",
        effort: null,
        pays: "own",
      }),
    );
  });

  it("says why a model was refused", async () => {
    setCodingTuning.mockRejectedValue({
      kind: "badRequest",
      message: "`-x` is not a model name Claude Code can be given",
    });
    render(<TerminalPanel agent={card(true, "claude")} />);
    const field = await screen.findByLabelText("Claude Code model");
    fireEvent.change(field, { target: { value: "-x" } });
    fireEvent.keyDown(field, { key: "Enter" });
    expect(await screen.findByText(/is not a model name Claude Code can be given/)).toBeTruthy();
  });

  it("shows why a change was refused rather than looking like it landed", async () => {
    setAgentCoding.mockRejectedValue({ kind: "notFound", message: "no agent with id a1" });
    render(<TerminalPanel agent={card(true)} />);
    await screen.findByText(PATH);

    fireEvent.click(screen.getByRole("button", { name: "Coding harness: pi" }));
    expect((await screen.findByRole("alert")).textContent).toContain("no agent with id a1");
  });
});
