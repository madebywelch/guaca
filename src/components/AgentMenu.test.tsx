import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AgentCard } from "../lib/types";
import { AgentMenu } from "./AgentMenu";

function card(over: Partial<AgentCard> = {}): AgentCard {
  return {
    id: "a1",
    groupId: "g1",
    computerId: null,
    name: "Manager",
    avatar: "avocado",
    color: "#c7d96b",
    model: "m",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

function open(agent: AgentCard, at = { x: 40, y: 40 }) {
  const handlers = {
    onClose: vi.fn(),
    onEditProfile: vi.fn(),
    onTogglePin: vi.fn(),
    onDuplicate: vi.fn(),
  };
  render(<AgentMenu target={{ agent, ...at }} {...handlers} />);
  return handlers;
}

describe("AgentMenu", () => {
  it("offers the three things you do to an agent without opening it", () => {
    open(card());
    expect(screen.getByRole("button", { name: "Edit profile" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Pin to top" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Duplicate" })).toBeTruthy();
  });

  it("says unpin on an agent that is already pinned", () => {
    // One item, not two: a menu that offers both is asking a question the
    // agent's own state has already answered.
    open(card({ pinned: true }));
    expect(screen.getByRole("button", { name: "Unpin" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Pin to top" })).toBeNull();
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

  it("stays inside the window when opened near an edge", () => {
    // A right-click can land anywhere, including three pixels from the bottom.
    // A menu hanging off it has items nothing can reach.
    open(card(), { x: 10_000, y: 10_000 });
    const menu = screen.getByRole("menu");
    expect(Number.parseFloat(menu.style.left)).toBeLessThan(window.innerWidth);
    expect(Number.parseFloat(menu.style.top)).toBeLessThan(window.innerHeight);
  });
});
