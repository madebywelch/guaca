import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { HOLD_MS, useHeld } from "../lib/held";
import { api, openExternal } from "../lib/ipc";
import { DEFAULT_PREFS } from "../lib/prefs";
import { useStore } from "../lib/store";
import { aGroup, anAgent, anApproval } from "../test-fixtures";
import { CommandMenu } from "./CommandMenu";

vi.mock("../lib/ipc", () => ({
  api: {
    sendMessage: vi.fn(async () => "run"),
    search: vi.fn(async () => ({
      messages: [
        {
          id: "message-1",
          channelId: "agent-1",
          from: { kind: "human" },
          to: { kind: "agent", id: "agent-1" },
          excerpt: "the quarterly ledger",
          createdAt: 1,
        },
      ],
      files: [],
      links: [
        {
          url: "https://example.com/ledger",
          messageId: "message-1",
          channelId: "agent-1",
          createdAt: 1,
        },
      ],
      routines: [],
      artifacts: [],
    })),
  },
  openExternal: vi.fn(async () => {}),
}));

const group = aGroup();
const dara = anAgent("agent-1", {
  name: "Dara",
  groupId: group.id,
  railOrder: 0,
  systemPrompt: "You are a helpful assistant who answers questions about this kitchen.",
});
const noor = anAgent("agent-2", { name: "Noor", groupId: group.id, railOrder: 1 });
const request = anApproval({ agentId: "agent-1", summary: "Change your name to Robin?" });
const decide = vi.fn(async () => {});
const select = vi.fn(async () => {});

function draw(pending = [request], agents = [dara, noor]) {
  const onClose = vi.fn();
  const onSection = vi.fn();
  const onPlace = vi.fn();
  const onEditAgent = vi.fn();
  act(() => {
    useHeld.setState({ held: null, notice: null });
    useStore.setState({
      agents,
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
  render(
    <CommandMenu
      onClose={onClose}
      onSection={onSection}
      onPlace={onPlace}
      onEditAgent={onEditAgent}
    />,
  );
  const input = screen.getByRole("textbox", { name: "Command menu" });
  const type = (text: string) => fireEvent.change(input, { target: { value: text } });
  const chosen = () =>
    document.querySelector<HTMLElement>("[data-selected='true'] .palette__title")?.textContent;
  return { input, type, chosen, onClose, onSection, onPlace, onEditAgent };
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  vi.mocked(api.sendMessage).mockClear();
  decide.mockClear();
  select.mockClear();
});

describe("the command menu", () => {
  it("opens a message result directly without handing the query to another palette", async () => {
    const openMessage = vi.fn(async () => {});
    act(() => useStore.setState({ openMessage }));
    const { input, type, onPlace } = draw([]);
    type("ledger");
    await act(async () => vi.advanceTimersByTime(120));
    const hit = screen.getByText("the quarterly ledger");
    fireEvent.mouseMove(hit);
    fireEvent.keyDown(input, { key: "Enter" });
    expect(openMessage).toHaveBeenCalledExactlyOnceWith("agent-1", "message-1");
    expect(onPlace).not.toHaveBeenCalled();
    expect(api.sendMessage).not.toHaveBeenCalled();
  });

  it("keeps a draft selected when results arrive and opens a selected link externally", async () => {
    const { type, chosen } = draw([]);
    type("the quarterly ledger");
    expect(chosen()).toBe("Write to Noor");
    await act(async () => vi.advanceTimersByTime(120));
    expect(chosen()).toBe("Write to Noor");
    fireEvent.click(screen.getByText("example.com/ledger"));
    expect(openExternal).toHaveBeenCalledExactlyOnceWith("https://example.com/ledger");
    expect(api.sendMessage).not.toHaveBeenCalled();
  });

  it("opens an agent's editor without opening the conversation", () => {
    const { input, type, chosen, onEditAgent } = draw([]);
    type("Dara settings");
    expect(chosen()).toBe("Dara settings");
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onEditAgent).toHaveBeenCalledExactlyOnceWith(dara);
    expect(select).not.toHaveBeenCalled();
  });

  it("keeps a short draft selected over incidental matches in an agent prompt", () => {
    const { input, type, chosen } = draw([], [dara]);
    type("on it");
    expect(chosen()).toBe("Write to Dara");
    fireEvent.keyDown(input, { key: "Enter" });
    expect(api.sendMessage).toHaveBeenCalledExactlyOnceWith("agent-1", "on it");
    expect(select).not.toHaveBeenCalled();
  });

  it("can find an agent by its prompt without choosing it in place of a draft", () => {
    const { type, chosen } = draw([]);
    type("this kitchen");
    expect(chosen()).toBe("Write to Noor");
    fireEvent.click(screen.getByRole("button", { name: /Dara.*this kitchen/ }));
    expect(select).toHaveBeenCalledExactlyOnceWith("agent-1");
    expect(api.sendMessage).not.toHaveBeenCalled();
  });

  it.each([
    ["calendar", "Calendar"],
    ["cafeteria", "Open the cafeteria"],
    ["app settings", "App settings"],
  ])("offers only one named action for %s", (query, label) => {
    const { type } = draw([]);
    type(query);
    expect(screen.getAllByText(label)).toHaveLength(1);
    expect(screen.queryByText(/^Search for/)).toBeNull();
  });

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
    expect(useHeld.getState().notice?.text).toBe("Allowed what Dara asked.");
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
