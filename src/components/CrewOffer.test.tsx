import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { HIREABLE, toDraft } from "../lib/cafeteria";
import { readCrewOffer, saveCrewOffer } from "../lib/onboarding";
import { useStore } from "../lib/store";
import type { AgentCard, AgentDraft } from "../lib/types";
import { CrewOffer } from "./CrewOffer";

const { hire, send } = vi.hoisted(() => ({ hire: vi.fn(), send: vi.fn() }));
vi.mock("../lib/ipc", () => ({ api: { hireAgents: hire, sendMessage: send } }));

const GROUP = "00000000-0000-4000-8000-0000000000b1";
const preset = (id: string) => HIREABLE.find((agent) => agent.id === id)!;
const card = (draft: AgentDraft, id: string): AgentCard => ({
  ...draft,
  id,
  groupId: GROUP,
  subscriptionModel: "",
  sandboxId: null,
  browserId: null,
  hasComputer: false,
  hasBrowser: false,
  runsErrands: false,
  readsWeb: true,
  browserConsent: "open",
  lifecycle: "active",
  pinned: false,
  railOrder: 0,
  version: 1,
  createdAt: 1,
  updatedAt: 1,
  discardedAt: null,
});
const analyst = card(toDraft(preset("financial-analyst")), "analyst");
const BRIEF = "Help me decide whether to buy or lease my next car.";
const refresh = vi.fn(async () => {});
const select = vi.fn(async () => {});

function offer(overrides = {}) {
  saveCrewOffer({ task: "research", agentId: analyst.id, brief: BRIEF, ...overrides });
}

beforeEach(() => {
  vi.resetAllMocks();
  localStorage.clear();
  hire.mockImplementation(async (_group, drafts: AgentDraft[]) =>
    drafts.map((draft, index) => card(draft, `crew-${index}`)),
  );
  send.mockResolvedValue("run-2");
  refresh.mockResolvedValue();
  select.mockResolvedValue();
  useStore.setState({ agents: [analyst], refreshAgents: refresh, select });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the crew offer after the first result", () => {
  it("offers nothing when no first task left an offer, or left it for another agent", () => {
    const view = render(<CrewOffer agent={analyst} />);
    expect(screen.queryByRole("region", { name: "A deeper pass" })).toBeNull();
    view.unmount();
    offer({ agentId: "someone-else" });
    render(<CrewOffer agent={analyst} />);
    expect(screen.queryByRole("region", { name: "A deeper pass" })).toBeNull();
  });

  it("names who would join, under the name the first agent has now, and hires nobody yet", () => {
    offer();
    render(<CrewOffer agent={{ ...analyst, name: "Money" }} />);
    const region = screen.getByRole("region", { name: "A deeper pass" });
    expect(region.textContent).toContain("Chief of Staff can ask Market Researcher");
    expect(region.textContent).toContain("check it with Money");
    expect(screen.getByRole("button", { name: "Bring them in" })).toBeTruthy();
    expect(hire).not.toHaveBeenCalled();
    expect(send).not.toHaveBeenCalled();
  });

  it("hires the coordinator and the colleague into the first agent's group, and sends the deeper pass to the coordinator", async () => {
    offer();
    render(<CrewOffer agent={analyst} />);
    fireEvent.click(screen.getByRole("button", { name: "Bring them in" }));
    await waitFor(() => expect(select).toHaveBeenCalledWith("crew-0"));
    const key = hire.mock.calls[0]![2];
    expect(key).toMatch(/^[0-9a-f-]{36}$/);
    expect(hire).toHaveBeenCalledExactlyOnceWith(
      GROUP,
      [toDraft(preset("chief-of-staff")), toDraft(preset("market-researcher"))],
      key,
    );
    const [to, text, files, sentKey] = send.mock.calls[0]!;
    expect([to, files, sentKey]).toEqual(["crew-0", [], key]);
    expect(text).toContain(`what I asked Financial Analyst, which has answered once: "${BRIEF}"`);
    expect(text).toContain("Ask Financial Analyst and Market Researcher at the same time");
    expect(text).not.toContain("{brief}");
    expect(readCrewOffer()).toBeNull();
    expect(useStore.getState().agents.map((agent) => agent.id)).toEqual([
      "analyst",
      "crew-0",
      "crew-1",
    ]);
    expect(screen.queryByRole("region", { name: "A deeper pass" })).toBeNull();
  });

  it("writes the names the host gave the colleagues into the brief it sends", async () => {
    hire.mockImplementationOnce(async (_group, drafts: AgentDraft[]) => [
      card(drafts[0]!, "crew-0"),
      card({ ...drafts[1]!, name: "Market Researcher 2" }, "crew-1"),
    ]);
    offer();
    render(<CrewOffer agent={{ ...analyst, name: "Money" }} />);
    fireEvent.click(screen.getByRole("button", { name: "Bring them in" }));
    await waitFor(() => expect(send).toHaveBeenCalledOnce());
    const text = send.mock.calls[0]![1] as string;
    expect(text).toContain("Ask Money and Market Researcher 2 at the same time");
    expect(text).not.toMatch(/Market Researcher(?! 2)|Financial Analyst/);
  });

  it("puts the person's brief in as written, even where it reads like a replacement pattern", async () => {
    offer({ brief: "Compare $& and $1 plans." });
    render(<CrewOffer agent={analyst} />);
    fireEvent.click(screen.getByRole("button", { name: "Bring them in" }));
    await waitFor(() => expect(send).toHaveBeenCalledOnce());
    expect(send.mock.calls[0]![1]).toContain('"Compare $& and $1 plans."');
  });

  it("sends nothing when the hire fails, says why, and retries the same hire under the same key", async () => {
    hire.mockRejectedValueOnce(new Error("The host is restarting."));
    offer();
    render(<CrewOffer agent={analyst} />);
    fireEvent.click(screen.getByRole("button", { name: "Bring them in" }));
    expect((await screen.findByRole("alert")).textContent).toBe("The host is restarting.");
    expect(send).not.toHaveBeenCalled();
    expect(readCrewOffer()?.key).toBe(hire.mock.calls[0]![2]);
    fireEvent.click(screen.getByRole("button", { name: "Bring them in" }));
    await waitFor(() => expect(send).toHaveBeenCalledOnce());
    expect(hire.mock.calls[1]).toEqual(hire.mock.calls[0]);
  });

  it("keeps the key after a lost send so a retry, even after reload, cannot hire or send twice", async () => {
    send.mockRejectedValueOnce(new Error("Send response lost"));
    offer();
    const view = render(<CrewOffer agent={analyst} />);
    fireEvent.click(screen.getByRole("button", { name: "Bring them in" }));
    await screen.findByRole("alert");
    const key = readCrewOffer()?.key;
    view.unmount();
    render(<CrewOffer agent={analyst} />);
    fireEvent.click(screen.getByRole("button", { name: "Bring them in" }));
    await waitFor(() => expect(select).toHaveBeenCalledWith("crew-0"));
    expect(hire.mock.calls.map((call) => call[2])).toEqual([key, key]);
    expect(send.mock.calls.map((call) => call[3])).toEqual([key, key]);
    expect(send.mock.calls[1]![1]).toBe(send.mock.calls[0]![1]);
  });

  it("hires nobody when the host returns less than both colleagues", async () => {
    hire.mockResolvedValueOnce([card(toDraft(preset("chief-of-staff")), "crew-0")]);
    offer();
    render(<CrewOffer agent={analyst} />);
    fireEvent.click(screen.getByRole("button", { name: "Bring them in" }));
    expect((await screen.findByRole("alert")).textContent).toBe(
      "The host did not return both colleagues. Try again.",
    );
    expect(send).not.toHaveBeenCalled();
  });

  it("starts nothing when the key cannot be saved first, and says so", () => {
    offer();
    render(<CrewOffer agent={analyst} />);
    vi.stubGlobal("localStorage", {
      getItem: () => null,
      setItem: () => {
        throw new Error("Storage denied");
      },
      removeItem: () => {},
    });
    fireEvent.click(screen.getByRole("button", { name: "Bring them in" }));
    expect(screen.getByRole("alert").textContent).toContain("nobody was hired");
    expect(hire).not.toHaveBeenCalled();
  });

  it("goes away on Not now, for good, without hiring or sending", () => {
    offer();
    const view = render(<CrewOffer agent={analyst} />);
    fireEvent.click(screen.getByRole("button", { name: "Not now" }));
    expect(screen.queryByRole("region", { name: "A deeper pass" })).toBeNull();
    expect(readCrewOffer()).toBeNull();
    view.unmount();
    render(<CrewOffer agent={analyst} />);
    expect(screen.queryByRole("region", { name: "A deeper pass" })).toBeNull();
    expect(hire).not.toHaveBeenCalled();
    expect(send).not.toHaveBeenCalled();
  });
});
