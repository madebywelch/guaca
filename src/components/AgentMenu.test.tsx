import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AgentCard, Group } from "../lib/types";
import { aGroup } from "../test-fixtures";
import { AgentMenu } from "./AgentMenu";

function card(over: Partial<AgentCard> = {}): AgentCard {
  return {
    id: "a1",
    groupId: "g1",
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    name: "Manager",
    avatar: "avocado",
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
    ...over,
  };
}

function group(id: string, name: string): Group {
  return aGroup({ id, name, agentCount: 1 });
}

function open(agent: AgentCard, at = { x: 40, y: 40 }, groups: Group[] = []) {
  const handlers = {
    onClose: vi.fn(),
    onEditProfile: vi.fn(),
    onTogglePin: vi.fn(),
    onTogglePause: vi.fn(),
    onDuplicate: vi.fn(),
    onClearHistory: vi.fn(),
    onNudge: vi.fn(),
    onMoveToGroup: vi.fn(),
  };
  render(<AgentMenu target={{ agent, ...at }} groups={groups} {...handlers} />);
  return handlers;
}

describe("AgentMenu", () => {
  it("offers everything you do to an agent, from either place it opens", () => {
    open(card());
    for (const name of [
      "Pause",
      "Edit profile",
      "Pin to top",
      "Move up",
      "Move down",
      "Duplicate",
      "Clear history…",
    ]) {
      expect(screen.getByRole("button", { name })).toBeTruthy();
    }
  });

  it("says resume on an agent that is already paused", () => {
    open(card({ lifecycle: "paused" }));
    expect(screen.getByRole("button", { name: "Resume" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
  });

  it("names the model, which is what you come here to check", () => {
    // It used to sit under the agent's name over every message it ever wrote.
    open(card({ model: "openai/gpt-5.6-terra" }));
    expect(screen.getByText("openai/gpt-5.6-terra")).toBeTruthy();
  });

  it("says unpin on an agent that is already pinned", () => {
    // One item, not two: a menu that offers both is asking a question the
    // agent's own state has already answered.
    open(card({ pinned: true }));
    expect(screen.getByRole("button", { name: "Unpin" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Pin to top" })).toBeNull();
  });

  it("arranges the rail without a mouse", () => {
    // A rail that can only be arranged by dragging cannot be arranged from a
    // keyboard at all, and this menu is already where everything else lives.
    const handlers = open(card());
    fireEvent.click(screen.getByRole("button", { name: "Move up" }));
    expect(handlers.onNudge).toHaveBeenCalledWith(expect.objectContaining({ id: "a1" }), -1);
  });

  it("offers the crews this agent is not in, and never the one it is", () => {
    const handlers = open(card(), { x: 40, y: 40 }, [
      group("g1", "everyone"),
      group("g2", "research"),
    ]);

    expect(screen.queryByRole("button", { name: "Move to everyone" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Move to research" }));
    expect(handlers.onMoveToGroup).toHaveBeenCalledWith(
      expect.objectContaining({ id: "a1" }),
      expect.objectContaining({ id: "g2" }),
    );
  });

  it("says nothing about groups while there is only one", () => {
    open(card(), { x: 40, y: 40 }, [group("g1", "everyone")]);
    expect(screen.queryByText(/^Move to /)).toBeNull();
  });

  it("closes as it acts, so the menu is never left over a stale row", () => {
    // The rail reorders itself as agents talk. A menu still open after a click
    // is pointing at whichever row has since moved under it.
    const handlers = open(card());
    fireEvent.click(screen.getByRole("button", { name: "Duplicate" }));
    expect(handlers.onDuplicate).toHaveBeenCalledTimes(1);
    expect(handlers.onClose).toHaveBeenCalledTimes(1);
  });

  it("dismisses on escape and on a click away", () => {
    const handlers = open(card());
    fireEvent.keyDown(window, { key: "Escape" });
    expect(handlers.onClose).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "Close menu" }));
    expect(handlers.onClose).toHaveBeenCalledTimes(2);
  });

  it("asks twice before deleting a history, without closing in between", () => {
    // The first click is the operator finding the item, not deciding anything.
    // Closing on it would mean the decision is taken in a menu they have to
    // open again, having already seen the word "clear" act like a button.
    const handlers = open(card());
    fireEvent.click(screen.getByRole("button", { name: "Clear history…" }));
    expect(handlers.onClearHistory).not.toHaveBeenCalled();
    expect(handlers.onClose).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Delete this history" }));
    expect(handlers.onClearHistory).toHaveBeenCalledTimes(1);
    expect(handlers.onClose).toHaveBeenCalledTimes(1);
  });

  it("stays inside the window when opened near an edge", () => {
    // A right-click can land anywhere, including three pixels from the bottom.
    // A menu hanging off it has items nothing can reach.
    open(card(), { x: 10_000, y: 10_000 });
    const menu = screen.getByRole("menu");
    expect(Number.parseFloat(menu.style.left)).toBeLessThan(window.innerWidth);
    expect(Number.parseFloat(menu.style.top)).toBeLessThan(window.innerHeight);
  });
});
