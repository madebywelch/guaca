import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { HOLD_MS, useHeld } from "../lib/held";
import { api } from "../lib/ipc";
import { DEFAULT_PREFS } from "../lib/prefs";
import { useStore } from "../lib/store";
import { aGroup, anAgent, anApproval } from "../test-fixtures";
import { CommandMenu } from "./CommandMenu";

vi.mock("../lib/ipc", () => ({
  api: { sendMessage: vi.fn(async () => "run") },
}));

const group = aGroup();
const dara = anAgent("agent-1", { name: "Dara", groupId: group.id, railOrder: 0 });
const noor = anAgent("agent-2", { name: "Noor", groupId: group.id, railOrder: 1 });
const request = anApproval({ agentId: "agent-1", summary: "Change your name to Robin?" });
const decide = vi.fn(async () => {});
const select = vi.fn(async () => {});

function draw(pending = [request]) {
  const onClose = vi.fn();
  const onSection = vi.fn();
  const onPlace = vi.fn();
  act(() => {
    useHeld.setState({ held: null, notice: null });
    useStore.setState({
      agents: [dara, noor],
      groups: [group],
      pending,
      stuck: [],
      activity: {},
      lastActive: { "agent-2": 5 },
      railGroup: null,
      selected: null,
      prefs: { ...DEFAULT_PREFS, surface: "light" },
      decideApproval: decide,
      select,
    });
  });
  render(<CommandMenu onClose={onClose} onSection={onSection} onPlace={onPlace} />);
  const input = screen.getByRole("textbox", { name: "Command menu" });
  const type = (text: string) => fireEvent.change(input, { target: { value: text } });
  const chosen = () =>
    document.querySelector<HTMLElement>("[data-selected='true'] .palette__title")?.textContent;
  return { input, type, chosen, onClose, onSection, onPlace };
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  vi.mocked(api.sendMessage).mockClear();
  decide.mockClear();
  select.mockClear();
});

describe("the command menu", () => {
  it("lays out what waits, the agents, the actions and the settings, in that order", () => {
    draw();
    const headings = [...document.querySelectorAll(".palette__section")].map((h) => h.textContent);
    expect(headings).toEqual(["Waiting on you", "Agents", "Actions", "Settings"]);
  });

  it("sends three words or more to the recipient on Return, which is whoever was written to last", async () => {
    const { input, type, chosen, onClose } = draw();
    type("draft the renewal note");
    expect(chosen()).toBe("Write to Noor");
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onClose).toHaveBeenCalled();
    expect(api.sendMessage).toHaveBeenCalledExactlyOnceWith("agent-2", "draft the renewal note");
  });

  it("changes the recipient on Tab, and Shift Tab goes back", () => {
    const { input, type } = draw();
    expect(screen.getByText("To Noor")).toBeTruthy();
    fireEvent.keyDown(input, { key: "Tab" });
    expect(screen.getByText("To Dara")).toBeTruthy();
    fireEvent.keyDown(input, { key: "Tab", shiftKey: true });
    expect(screen.getByText("To Noor")).toBeTruthy();
    fireEvent.keyDown(input, { key: "Tab" });
    type("please book the room");
    fireEvent.keyDown(input, { key: "Enter" });
    expect(api.sendMessage).toHaveBeenCalledExactlyOnceWith("agent-1", "please book the room");
  });

  it("switches the reading surface on one word, dark, and Return", () => {
    const { input, type, chosen } = draw();
    type("dark");
    expect(chosen()).toBe("Reading surface: Dark");
    fireEvent.keyDown(input, { key: "Enter" });
    expect(useStore.getState().prefs.surface).toBe("dark");
    expect(api.sendMessage).not.toHaveBeenCalled();
  });

  it("finds the appearance settings on theme", () => {
    const { type } = draw();
    type("theme");
    const titles = [...document.querySelectorAll(".palette__title")].map((t) => t.textContent);
    expect(titles).toContain("Settings: Appearance");
    expect(titles).toContain("Reading surface: Dark");
  });

  it("selects an agent by name and opens it", () => {
    const { input, type, chosen } = draw();
    type("dara");
    expect(chosen()).toBe("Dara");
    fireEvent.keyDown(input, { key: "Enter" });
    expect(select).toHaveBeenCalledWith("agent-1");
  });

  it("finds the Allow on approve, and holds it for Undo before it is sent", () => {
    const { input, type, chosen } = draw();
    type("approve");
    expect(chosen()).toBe("Allow");
    fireEvent.keyDown(input, { key: "Enter" });
    expect(decide).not.toHaveBeenCalled();
    expect(useHeld.getState().notice?.text).toBe("Allowed Dara's request.");
    act(() => vi.advanceTimersByTime(HOLD_MS));
    expect(decide).toHaveBeenCalledExactlyOnceWith(request.id, "allow");
  });

  it("denies the request on Command Backspace, and shows that key on its Deny row only", () => {
    const { input } = draw();
    const deny = screen.getByRole("button", { name: /^Deny/ });
    expect(within(deny).getByText(/⌫|Backspace/)).toBeTruthy();
    fireEvent.keyDown(input, { key: "Backspace", metaKey: true });
    act(() => vi.advanceTimersByTime(HOLD_MS));
    expect(decide).toHaveBeenCalledExactlyOnceWith(request.id, "deny");
  });

  it("heads each request once with its asker, and lists its answers under it without repeating it", () => {
    draw([
      request,
      anApproval({
        id: "req-2",
        agentId: "agent-2",
        summary: "Which story leads Friday?",
        request: { kind: "question", options: ["Notebooks", "The renewal"] },
      }),
    ]);
    const headings = [...document.querySelectorAll(".palette__asking")].map((h) => h.textContent);
    expect(headings).toEqual([
      "Dara: Change your name to Robin?",
      "Noor: Which story leads Friday?",
    ]);
    expect(document.querySelector(".palette__asking .palette__who")?.textContent).toBe("Dara:");
    expect(document.body.textContent?.split("Change your name to Robin?").length).toBe(2);
    for (const name of ["Allow", "Deny", "Notebooks", "The renewal"]) {
      const row = screen.getByRole("button", { name: new RegExp(`^${name}`) });
      expect(row.querySelector(".palette__detail")).toBeNull();
      expect(row.querySelector(".palette__face")?.childElementCount).toBe(0);
    }
  });

  it("names the asker once when the request already starts with the name", () => {
    draw([
      anApproval({ agentId: "agent-1", summary: "Dara wants to change the workspace's settings" }),
    ]);
    const heading = document.querySelector(".palette__asking");
    expect(heading?.textContent).toBe("Dara wants to change the workspace's settings");
    expect(heading?.querySelector(".palette__who")?.textContent).toBe("Dara");
  });

  it("draws the Return key on the row Return acts on, and on no other", () => {
    const { type, chosen } = draw();
    const returns = () =>
      [...document.querySelectorAll(".palette__key")]
        .filter((key) => key.textContent === "↩")
        .map((key) => key.closest(".palette__row")?.querySelector(".palette__title")?.textContent);
    expect(returns()).toEqual([chosen()]);
    type("dark");
    expect(chosen()).toBe("Reading surface: Dark");
    expect(returns()).toEqual(["Reading surface: Dark"]);
    type("draft the renewal note");
    expect(returns()).toEqual(["Write to Noor"]);
  });

  it("numbers the agents with the keys that open them", () => {
    draw();
    const row = screen.getByRole("button", { name: /^Dara/ });
    expect(row.textContent).toMatch(/1$/);
  });
});
