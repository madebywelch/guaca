import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { HIREABLE, toDraft } from "../lib/cafeteria";
import { readFirstTask, saveFirstTask } from "../lib/onboarding";
import { useStore } from "../lib/store";
import type { AgentCard } from "../lib/types";
import { aGroup } from "../test-fixtures";
import { Onboarding } from "./Onboarding";

const { hire, send } = vi.hoisted(() => ({ hire: vi.fn(), send: vi.fn() }));
vi.mock("../lib/ipc", () => ({ api: { hireAgents: hire, sendMessage: send } }));
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

const preset = HIREABLE.find((agent) => agent.id === "product-manager")!;
const agent: AgentCard = {
  ...toDraft(preset),
  id: "first-agent",
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
};
const refresh = vi.fn(async () => {});
const select = vi.fn(async () => {});
const done = vi.fn();
const starting = vi.fn();

function mount() {
  return render(
    <Onboarding
      group={aGroup()}
      onStarting={starting}
      onDone={done}
      onBrowse={() => {}}
      onCreate={() => {}}
    />,
  );
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
  localStorage.clear();
  hire.mockResolvedValue([agent]);
  send.mockResolvedValue("run-1");
  refresh.mockResolvedValue();
  select.mockResolvedValue();
  useStore.setState({ agents: [], refreshAgents: refresh, select });
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
    ).toContain("first draft");
  });

  it("creates one recommended agent and sends only after the explicit Start action", async () => {
    mount();
    toReview();
    expect(hire).not.toHaveBeenCalled();
    expect(send).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    expect(hire).toHaveBeenCalledWith(aGroup().id, [toDraft(preset)]);
    expect(send).toHaveBeenCalledExactlyOnceWith(agent.id, "Plan our launch next month.");
    expect(select).toHaveBeenCalledWith(agent.id);
    expect(readFirstTask().brief).toBe("");
  });

  it("keeps the created agent and brief when sending fails, without hiring twice", async () => {
    send.mockRejectedValueOnce(new Error("Connection lost"));
    mount();
    toReview();
    fireEvent.click(screen.getByRole("button", { name: "Start task" }));
    expect(await screen.findByRole("alert")).toHaveProperty("textContent", "Connection lost");
    expect(readFirstTask().agentId).toBe(agent.id);
    expect(done).not.toHaveBeenCalled();
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
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    expect(readFirstTask().brief).toBe("Saved brief");
  });
});
