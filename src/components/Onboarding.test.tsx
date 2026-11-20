import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { HIREABLE, toDraft } from "../lib/cafeteria";
import { readFirstTask, saveFirstTask } from "../lib/onboarding";
import { useStore } from "../lib/store";
import { BEFORE_RELOAD } from "../lib/transport";
import type { AgentCard } from "../lib/types";
import { aGroup } from "../test-fixtures";
import { Onboarding } from "./Onboarding";

const { hire, send } = vi.hoisted(() => ({ hire: vi.fn(), send: vi.fn() }));
vi.mock("../lib/ipc", () => ({ api: { hireAgents: hire, sendMessage: send } }));
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

const PLAN_GROUP = ["chief-of-staff", "product-manager", "qa-tester"];
const presets = PLAN_GROUP.map((id) => HIREABLE.find((agent) => agent.id === id)!);
const card = (index: number): AgentCard => ({
  ...toDraft(presets[index]!),
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
const group = presets.map((_, index) => card(index));
const agent = group[0]!;
const refresh = vi.fn(async () => {});
const select = vi.fn(async () => {});
const done = vi.fn();
const starting = vi.fn();
const inspecting = vi.fn();

function mount() {
  return render(
    <Onboarding
      group={aGroup()}
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
  hire.mockResolvedValue(group);
  send.mockResolvedValue("run-1");
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

  it("says on the first step that a task goes to a group of three, and keeps the way to set up alone", () => {
    mount();
    expect(screen.getByText(/Guaca hires a group of three for it/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Create one agent" })).toBeTruthy();
  });

  it("names the three it will hire and what each is for before anything is created", () => {
    mount();
    toReview();
    const listed = screen.getByRole("region", { name: "Your group for this task" });
    expect(
      Array.from(listed.querySelectorAll("li")).map(
        (row) => row.querySelector("strong")!.textContent,
      ),
    ).toEqual(["Chief of Staff", "Product Manager", "QA Tester"]);
    expect(listed.textContent).toContain("Looks for the ways that plan could go wrong.");
    expect(
      screen.getByText(/asks Product Manager and QA Tester at the same time/).textContent,
    ).toContain("one question if it cannot go on without you");
    expect(screen.getByText(/at least five instead of one/).textContent).toContain(
      "stays inside your group's limits",
    );
    expect(hire).not.toHaveBeenCalled();
  });

  it("writes the names the host gave the colleagues into the brief it sends", async () => {
    hire.mockResolvedValueOnce([group[0], group[1], { ...group[2]!, name: "QA Tester 2" }]);
    mount();
    fireEvent.click(screen.getByRole("button", { name: /Plan a project/ }));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    fireEvent.click(screen.getByRole("button", { name: "Connect test provider" }));
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    const sent = send.mock.calls[0]![1] as string;
    expect(sent).toContain("Ask Product Manager and QA Tester 2 at the same time");
    expect(sent).not.toMatch(/QA Tester(?! 2)/);
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

  it("hires the task's group of three in one keyed request and sends only after the explicit Start action", async () => {
    mount();
    toReview();
    expect(hire).not.toHaveBeenCalled();
    expect(send).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    const key = hire.mock.calls[0]![2];
    expect(key).toMatch(/^[0-9a-f-]{36}$/);
    expect(hire).toHaveBeenCalledExactlyOnceWith(aGroup().id, presets.map(toDraft), key);
    expect(send).toHaveBeenCalledExactlyOnceWith(agent.id, "Plan our launch next month.", [], key);
    expect(select).toHaveBeenCalledWith(agent.id);
    expect(readFirstTask().brief).toBe("");
    expect(useStore.getState().agents.map((hired) => hired.id)).toEqual(
      group.map((hired) => hired.id),
    );
  });

  it("sends nothing and keeps no agent when the host answers with less than the whole group", async () => {
    hire.mockResolvedValueOnce([agent]);
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
    expect(screen.getByText(/previous start may have hired this group/i)).toBeTruthy();
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
    expect(await screen.findByRole("alert")).toHaveProperty("textContent", "Connection lost");
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
