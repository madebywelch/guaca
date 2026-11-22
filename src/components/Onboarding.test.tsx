import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { HIREABLE, toDraft } from "../lib/cafeteria";
import { readCrewOffer, readFirstTask, saveFirstTask } from "../lib/onboarding";
import { useStore } from "../lib/store";
import { BEFORE_RELOAD } from "../lib/transport";
import type { AgentCard, AgentDraft } from "../lib/types";
import { aGroup } from "../test-fixtures";
import { Onboarding } from "./Onboarding";

const { hire, send, rename } = vi.hoisted(() => ({
  hire: vi.fn(),
  send: vi.fn(),
  rename: vi.fn(),
}));
vi.mock("../lib/ipc", () => ({
  api: { hireAgents: hire, sendMessage: send, updateGroup: rename },
}));
const platform = vi.hoisted(() => ({ desktop: true }));
vi.mock("../lib/transport", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/transport")>()),
  get desktop() {
    return platform.desktop;
  },
}));
vi.mock("./OnboardingProvider", () => ({
  OnboardingProvider: ({
    onConnected,
    onBack,
  }: {
    onConnected: () => void;
    onBack: () => void;
  }) => (
    <>
      <button type="button" onClick={onConnected}>
        Connect test provider
      </button>
      <button type="button" onClick={onBack}>
        Back to task
      </button>
    </>
  ),
}));

const PLAN_AGENT = HIREABLE.find((agent) => agent.id === "product-manager")!;
const card = (draft: AgentDraft, index: number): AgentCard => ({
  ...draft,
  id: index === 0 ? "first-agent" : "colleague-" + index,
  groupId: aGroup().id,
  model: "",
  subscriptionModel: "",
  sandboxId: null,
  browserId: null,
  hasComputer: false,
  hasBrowser: false,
  runsErrands: false,
  browserConsent: "open",
  lifecycle: "active",
  pinned: false,
  railOrder: 0,
  version: 1,
  createdAt: 1,
  updatedAt: 1,
  discardedAt: null,
});
const cards = (drafts: AgentDraft[]) => drafts.map(card);
const agent = card(toDraft(PLAN_AGENT), 0);
const refresh = vi.fn(async () => {});
const select = vi.fn(async () => {});
const done = vi.fn();
const starting = vi.fn();
const inspecting = vi.fn();

function mount(starter = aGroup()) {
  return render(
    <Onboarding
      group={starter}
      onStarting={starting}
      onDone={done}
      onInspect={inspecting}
      onBrowse={() => {}}
      onCreate={() => {}}
    />,
  );
}

function storage() {
  const value = {
    getItem: localStorage.getItem.bind(localStorage),
    setItem: vi.fn(localStorage.setItem.bind(localStorage)),
    removeItem: vi.fn(localStorage.removeItem.bind(localStorage)),
  };
  vi.stubGlobal("localStorage", value);
  return value;
}

function toReview() {
  fireEvent.click(screen.getByRole("button", { name: /Plan a project/ }));
  fireEvent.change(screen.getByRole("textbox", { name: "Your first brief" }), {
    target: { value: "Plan our launch next month." },
  });
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Connect test provider" }));
}

beforeEach(() => {
  vi.resetAllMocks();
  platform.desktop = true;
  localStorage.clear();
  hire.mockImplementation(async (_group, drafts) => cards(drafts));
  send.mockResolvedValue("run-1");
  rename.mockImplementation(async (_id, draft) => aGroup({ name: draft.name }));
  refresh.mockResolvedValue();
  select.mockResolvedValue();
  useStore.setState({ agents: [], refreshAgents: refresh, select });
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("where Guaca goes when the window closes", () => {
  it("says the menu bar panel exists, on a desktop", () => {
    mount();
    toReview();
    expect(screen.getByText(/Guaca stays in the menu bar/)).toBeTruthy();
  });

  it("says nothing about a menu bar in a browser, which has none", () => {
    platform.desktop = false;
    mount();
    toReview();
    expect(screen.queryByText(/menu bar/)).toBeNull();
  });
});

describe("guided first task", () => {
  it("keeps a written brief when choosing another task, going back, or reloading", () => {
    const view = mount();
    fireEvent.change(screen.getByRole("textbox", { name: "Your first brief" }), {
      target: { value: "My actual task" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Draft content/ }));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    fireEvent.click(screen.getByRole("button", { name: "Back to task" }));
    expect(
      (screen.getByRole("textbox", { name: "Your first brief" }) as HTMLTextAreaElement).value,
    ).toBe("My actual task");
    view.unmount();
    mount();
    expect(
      (screen.getByRole("textbox", { name: "Your first brief" }) as HTMLTextAreaElement).value,
    ).toBe("My actual task");
    expect(hire).not.toHaveBeenCalled();
    expect(send).not.toHaveBeenCalled();
  });

  it("replaces an untouched suggestion, but requires a nonempty brief", () => {
    mount();
    expect((screen.getByRole("button", { name: "Continue" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    fireEvent.click(screen.getByRole("button", { name: /Plan a project/ }));
    fireEvent.click(screen.getByRole("button", { name: /Draft content/ }));
    expect(
      (screen.getByRole("textbox", { name: "Your first brief" }) as HTMLTextAreaElement).value,
    ).toContain("one draft I can send");
  });

  it("says on the first step that one agent answers first and colleagues can follow, and keeps the way to set up alone", () => {
    mount();
    expect(screen.getByText(/Guaca hires the agent suited to it/).textContent).toContain(
      "bring in two colleagues for a deeper pass",
    );
    expect(screen.getByRole("button", { name: "Create one agent" })).toBeTruthy();
  });

  it("names the one agent it will hire and who can join later, before anything is created", () => {
    mount();
    toReview();
    const listed = screen.getByRole("region", { name: "Your agent for this task" });
    expect(
      Array.from(listed.querySelectorAll("li")).map(
        (row) => row.querySelector("strong")!.textContent,
      ),
    ).toEqual(["Product Manager"]);
    expect(listed.textContent).toContain("Finds the smallest plan that would work");
    expect(listed.textContent).toContain(
      "Once its result is in, you can bring in Chief of Staff and QA Tester for a deeper pass.",
    );
    expect(screen.getByText(/Product Manager takes it, works on it alone/).textContent).toContain(
      "one question if it cannot go on without you",
    );
    expect(screen.getByText(/waits on nobody else/).textContent).toContain(
      "Start task hires Product Manager",
    );
    expect(screen.queryByText(/at least five instead of one/)).toBeNull();
    expect(hire).not.toHaveBeenCalled();
  });

  it("still names a group of three for a task in the person's own words", () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: /Something else/ }));
    fireEvent.change(screen.getByRole("textbox", { name: "Your first brief" }), {
      target: { value: "Plan a small team offsite." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    fireEvent.click(screen.getByRole("button", { name: "Connect test provider" }));
    const listed = screen.getByRole("region", { name: "Your group for this task" });
    expect(listed.querySelectorAll("li")).toHaveLength(3);
    expect(screen.queryByText(/deeper pass/)).toBeNull();
  });

  it("does not promise a split it cannot see in a brief the person wrote", () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: /Something else/ }));
    fireEvent.change(screen.getByRole("textbox", { name: "Your first brief" }), {
      target: { value: "Plan a small team offsite." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    fireEvent.click(screen.getByRole("button", { name: "Connect test provider" }));
    expect(
      screen.getByText(/decides what to hand Market Researcher and Product Manager/),
    ).toBeTruthy();
    expect(screen.queryByText(/at the same time/)).toBeNull();
  });

  it("hires only the task's one agent in one keyed request and sends only after the explicit Start action", async () => {
    mount();
    toReview();
    expect(hire).not.toHaveBeenCalled();
    expect(send).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    const key = hire.mock.calls[0]![2];
    expect(key).toMatch(/^[0-9a-f-]{36}$/);
    expect(hire).toHaveBeenCalledExactlyOnceWith(aGroup().id, [toDraft(PLAN_AGENT)], key);
    expect(send).toHaveBeenCalledExactlyOnceWith(agent.id, "Plan our launch next month.", [], key);
    expect(select).toHaveBeenCalledWith(agent.id);
    expect(readFirstTask().brief).toBe("");
    expect(useStore.getState().agents.map((hired) => hired.id)).toEqual([agent.id]);
  });

  it("leaves the crew offer for after the first result, with the brief that was sent", async () => {
    mount();
    toReview();
    expect(readCrewOffer()).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    expect(readCrewOffer()).toEqual({
      task: "plan",
      agentId: agent.id,
      brief: "Plan our launch next month.",
    });
  });

  it("leaves no crew offer when Start fails before the brief is sent", async () => {
    send.mockRejectedValueOnce(new Error("Connection lost"));
    mount();
    toReview();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await screen.findByRole("alert");
    expect(readCrewOffer()).toBeNull();
  });

  it("leaves no crew offer for a task in the person's own words, whose group is already there", async () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: /Something else/ }));
    fireEvent.change(screen.getByRole("textbox", { name: "Your first brief" }), {
      target: { value: "Sort out my week." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    fireEvent.click(screen.getByRole("button", { name: "Connect test provider" }));
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    expect(hire.mock.calls[0]![1]).toHaveLength(3);
    expect(readCrewOffer()).toBeNull();
  });

  it("sends nothing and keeps no agent when the host answers with less than the whole group", async () => {
    hire.mockResolvedValueOnce([]);
    mount();
    toReview();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    expect((await screen.findByRole("alert")).textContent).toBe(
      "The host did not return the whole group. Try again.",
    );
    expect(send).not.toHaveBeenCalled();
    expect(readFirstTask().agentId).toBeNull();
    expect(done).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    expect(hire.mock.calls[1]![2]).toBe(hire.mock.calls[0]![2]);
    expect(send).toHaveBeenCalledExactlyOnceWith(
      agent.id,
      "Plan our launch next month.",
      [],
      hire.mock.calls[0]![2],
    );
  });

  it("hires nobody and sends nothing when the starter group cannot be named, and keeps the brief", async () => {
    rename.mockRejectedValueOnce(new Error("The host is restarting."));
    mount();
    toReview();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    expect((await screen.findByRole("alert")).textContent).toBe("The host is restarting.");
    expect(hire).not.toHaveBeenCalled();
    expect(send).not.toHaveBeenCalled();
    expect(readFirstTask().brief).toBe("Plan our launch next month.");
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    expect(rename).toHaveBeenLastCalledWith(aGroup().id, { name: "Planning" });
    expect(hire).toHaveBeenCalledOnce();
  });

  it("names the empty starter group for the task before hiring into it", async () => {
    mount();
    toReview();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    expect(rename).toHaveBeenCalledExactlyOnceWith(aGroup().id, { name: "Planning" });
    expect(rename.mock.invocationCallOrder[0]!).toBeLessThan(hire.mock.invocationCallOrder[0]!);
  });

  it("gives a task in the person's own words a plain group name", async () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: /Something else/ }));
    fireEvent.change(screen.getByRole("textbox", { name: "Your first brief" }), {
      target: { value: "Sort out my week." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    fireEvent.click(screen.getByRole("button", { name: "Connect test provider" }));
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    expect(rename).toHaveBeenCalledExactlyOnceWith(aGroup().id, { name: "My group" });
  });

  it("leaves the name of a group that already has agents", async () => {
    mount(aGroup({ name: "Kitchen", agentCount: 2 }));
    toReview();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    expect(rename).not.toHaveBeenCalled();
  });

  it("sends nothing when the hire fails, and retries the same group under the same key", async () => {
    hire.mockRejectedValueOnce(new Error("The host is restarting."));
    mount();
    toReview();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe("The host is restarting.");
    expect(alert.closest(".onboarding-actions")).toBe(
      screen.getByRole("button", { name: "Start task" }).closest(".onboarding-actions"),
    );
    expect(send).not.toHaveBeenCalled();
    expect(readFirstTask().agentId).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    expect(hire.mock.calls[1]).toEqual(hire.mock.calls[0]);
  });

  it("persists the key before a lost hire response and reuses it after reload", async () => {
    hire.mockImplementationOnce(async (_group, _drafts, key) => {
      expect(readFirstTask().key).toBe(key);
      throw new Error("Hire response lost");
    });
    const view = mount();
    toReview();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await screen.findByRole("alert");
    const key = readFirstTask().key;
    expect(key).toBeTruthy();
    view.unmount();
    mount();
    expect(
      (screen.getByRole("textbox", { name: "Your first brief" }) as HTMLTextAreaElement).readOnly,
    ).toBe(true);
    expect(screen.getByText(/previous start may have hired Product Manager/i)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    expect(hire.mock.calls[1]![2]).toBe(key);
    expect(send).toHaveBeenCalledWith(agent.id, "Plan our launch next month.", [], key);
  });

  it("reuses the key after a lost send response and reload", async () => {
    send.mockRejectedValueOnce(new Error("Send response lost"));
    const view = mount();
    toReview();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await screen.findByRole("alert");
    const key = readFirstTask().key;
    expect(key).toBeTruthy();
    view.unmount();
    mount();
    fireEvent.click(screen.getByRole("button", { name: "Send brief again" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    expect(hire).toHaveBeenCalledOnce();
    expect(send).toHaveBeenCalledTimes(2);
    expect(send.mock.calls[0]![3]).toBe(key);
    expect(send.mock.calls[1]![3]).toBe(key);
  });

  it("keeps the created agent and brief when sending fails, without hiring twice", async () => {
    send.mockRejectedValueOnce(new Error("Connection lost"));
    mount();
    toReview();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    expect(await screen.findByRole("alert")).toHaveProperty("textContent", "Connection lost.");
    expect(readFirstTask().agentId).toBe(agent.id);
    expect(done).not.toHaveBeenCalled();
    const brief = screen.getByRole("textbox", { name: "Your first brief" }) as HTMLTextAreaElement;
    expect(brief.readOnly).toBe(true);
    expect(brief.disabled).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "Send brief again" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    expect(hire).toHaveBeenCalledOnce();
  });

  it("does not resend an accepted task when opening the channel fails", async () => {
    select.mockRejectedValueOnce(new Error("Could not load channel"));
    mount();
    toReview();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await screen.findByRole("alert");
    expect(readFirstTask().sent).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Open conversation" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    expect(send).toHaveBeenCalledOnce();
    expect(hire).toHaveBeenCalledOnce();
  });

  it("resumes an interrupted first task without automatically sending it", async () => {
    saveFirstTask({ task: "plan", brief: "Saved brief", agentId: agent.id, sent: false });
    useStore.setState({ agents: [agent] });
    mount();
    expect(screen.getByRole("button", { name: "Check conversation" })).toBeTruthy();
    expect(send).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Check conversation" }));
    await screen.findByRole("button", { name: "Return to first task" });
    expect(inspecting).toHaveBeenLastCalledWith(true);
    expect(done).not.toHaveBeenCalled();
    expect(readFirstTask().brief).toBe("Saved brief");
    fireEvent.click(screen.getByRole("button", { name: "Return to first task" }));
    expect(inspecting).toHaveBeenLastCalledWith(false);
    expect(screen.getByRole("button", { name: "Send brief again" })).toBeTruthy();
    expect(
      (screen.getByRole("textbox", { name: "Your first brief" }) as HTMLTextAreaElement).value,
    ).toBe("Saved brief");
    fireEvent.click(screen.getByRole("button", { name: "Check conversation" }));
    fireEvent.click(await screen.findByRole("button", { name: "Task was delivered" }));
    expect(done).toHaveBeenCalledOnce();
    expect(readFirstTask().agentId).toBeNull();
    expect(readCrewOffer()).toEqual({ task: "plan", agentId: agent.id, brief: "Saved brief" });
    expect(send).not.toHaveBeenCalled();
  });

  it("keeps an unsaved brief in memory and blocks reload until persistence recovers", () => {
    storage().setItem.mockImplementation(() => {
      throw new Error("Quota exceeded");
    });
    mount();
    fireEvent.change(screen.getByRole("textbox", { name: "Your first brief" }), {
      target: { value: "Do not lose this brief" },
    });
    expect(screen.getByRole("alert").textContent).toContain("Keep this window open");
    const reload = new Event(BEFORE_RELOAD, { cancelable: true });
    window.dispatchEvent(reload);
    expect(reload.defaultPrevented).toBe(true);
    const unload = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(unload);
    expect(unload.defaultPrevented).toBe(true);
    vi.unstubAllGlobals();
    fireEvent.click(screen.getByRole("button", { name: "Retry saving" }));
    expect(readFirstTask().brief).toBe("Do not lose this brief");
    expect(screen.queryByRole("alert")).toBeNull();
    expect(window.dispatchEvent(new Event(BEFORE_RELOAD, { cancelable: true }))).toBe(true);
  });

  it("does not create an agent when the initial recovery checkpoint cannot be saved", () => {
    mount();
    toReview();
    storage().setItem.mockImplementation(() => {
      throw new Error("Storage denied");
    });
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    expect(screen.getByRole("alert")).toBeTruthy();
    expect(hire).not.toHaveBeenCalled();
    expect(send).not.toHaveBeenCalled();
  });

  it("says beside Start that a save failed, so a Start stopped by storage is never silent", () => {
    mount();
    toReview();
    storage().setItem.mockImplementation(() => {
      throw new Error("Storage denied");
    });
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    const said = screen.getByRole("status");
    expect(said.textContent).toContain("could not save your progress");
    expect(said.closest(".onboarding-actions")).toBe(
      screen.getByRole("button", { name: "Start task" }).closest(".onboarding-actions"),
    );
  });

  it("says in plain words that the host could not be reached, and gives Start task the focus back", async () => {
    hire.mockRejectedValueOnce({
      kind: "unreachable",
      message:
        "could not reach this host at http://127.0.0.1:8787. It may be starting up, or the connection to it is down. Its agents keep working either way",
    });
    mount();
    toReview();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe(
      "Guaca could not reach its host. Your brief is kept here. Check that the host is running, then try again.",
    );
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "Start task" })),
    );
  });

  it("puts focus on the first task, and on Start task at the last step", () => {
    mount();
    expect(document.activeElement).toBe(
      screen.getByRole("button", { name: /^Research a decision/ }),
    );
    toReview();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Start task" }));
  });

  it("says nothing beside Start about saving when no Start was stopped by it", () => {
    mount();
    toReview();
    storage().setItem.mockImplementation(() => {
      throw new Error("Storage denied");
    });
    fireEvent.change(screen.getByRole("textbox", { name: "Your first brief" }), {
      target: { value: "A brief typed on the Start step" },
    });
    expect(screen.getByRole("alert").textContent).toContain("Keep this window open");
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("keeps a confirmed send in memory if saving its marker fails", async () => {
    mount();
    toReview();
    const save = storage().setItem;
    send.mockImplementationOnce(async () => {
      save.mockImplementation(() => {
        throw new Error("Storage denied");
      });
      return "run-1";
    });
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await screen.findByRole("alert");
    expect(done).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Open conversation" })).toBeTruthy();
    vi.unstubAllGlobals();
    fireEvent.click(screen.getByRole("button", { name: "Retry saving" }));
    fireEvent.click(screen.getByRole("button", { name: "Open conversation" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    expect(send).toHaveBeenCalledOnce();
  });

  it("does not dismiss recovery when the stale draft cannot be cleared", async () => {
    saveFirstTask({ task: "plan", brief: "Saved brief", agentId: agent.id, sent: false });
    useStore.setState({ agents: [agent] });
    mount();
    fireEvent.click(screen.getByRole("button", { name: "Check conversation" }));
    await screen.findByRole("button", { name: "Task was delivered" });
    storage().removeItem.mockImplementation(() => {
      throw new Error("Storage denied");
    });
    fireEvent.click(screen.getByRole("button", { name: "Task was delivered" }));
    expect(screen.getByRole("alert")).toBeTruthy();
    expect(done).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
    fireEvent.click(screen.getByRole("button", { name: "Task was delivered" }));
    expect(done).toHaveBeenCalledOnce();
    expect(readFirstTask().agentId).toBeNull();
  });
});
