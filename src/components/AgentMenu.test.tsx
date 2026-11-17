import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AgentCard, Group, Provider } from "../lib/types";
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
    runsErrands: false,
    browserConsent: "open",
    name: "Manager",
    avatar: "avocado",
    color: "#c7d96b",
    model: "m",
    subscriptionModel: "",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    discardedAt: null,
    ...over,
  };
}

function group(id: string, name: string): Group {
  return aGroup({ id, name, agentCount: 1 });
}

function open(
  agent: AgentCard,
  at = { x: 40, y: 40 },
  groups: Group[] = [],
  provider: Provider = "compatible",
) {
  const handlers = {
    onClose: vi.fn(),
    onEditProfile: vi.fn(),
    onTogglePin: vi.fn(),
    onTogglePause: vi.fn(),
    onDuplicate: vi.fn(),
    onClearHistory: vi.fn(),
    onDelete: vi.fn(),
    onMoveToGroup: vi.fn(),
  };
  render(<AgentMenu target={{ agent, ...at }} groups={groups} provider={provider} {...handlers} />);
  return handlers;
}

describe("AgentMenu", () => {
  it("offers everything you do to an agent, from either place it opens", () => {
    open(card());
    for (const name of [
      "Pause",
      "Edit profile",
      "Pin to top",
      "Duplicate",
      "Clear chat history…",
      "Delete…",
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
    open(card({ model: "openai/gpt-5.6-terra" }));
    expect(screen.getByText("openai/gpt-5.6-terra")).toBeTruthy();
  });

  it("names the model its provider runs, never the one kept for the other", () => {
    const engineer = card({ model: "xiaomi/mimo-v2.6-pro", subscriptionModel: "" });
    open(engineer, undefined, [], "chatgpt");
    expect(screen.queryByText("xiaomi/mimo-v2.6-pro")).toBeNull();
    cleanup();
    open({ ...engineer, subscriptionModel: "gpt-6-astra" }, undefined, [], "chatgpt");
    expect(screen.getByText("gpt-6-astra")).toBeTruthy();
    cleanup();
    open({ ...engineer, subscriptionModel: "gpt-6-astra" }, undefined, [], "claude");
    expect(screen.queryByText(/gpt-6-astra|xiaomi/)).toBeNull();
  });

  it("says unpin on an agent that is already pinned", () => {
    open(card({ pinned: true }));
    expect(screen.getByRole("button", { name: "Unpin" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Pin to top" })).toBeNull();
  });

  it("keeps the crews behind one row, and opens them beside it", () => {
    const handlers = open(card(), { x: 40, y: 40 }, [
      group("g1", "everyone"),
      group("g2", "research"),
      group("g3", "support"),
    ]);

    const opener = screen.getByRole("button", { name: "Move to another group" });
    expect(screen.queryByRole("button", { name: "research" })).toBeNull();
    expect(opener.getAttribute("aria-expanded")).toBe("false");

    fireEvent.pointerEnter(opener);
    expect(opener.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByRole("menu", { name: "Move to another group" })).toBeTruthy();

    expect(screen.queryByRole("button", { name: "everyone" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "research" }));
    expect(handlers.onMoveToGroup).toHaveBeenCalledWith(
      expect.objectContaining({ id: "a1" }),
      expect.objectContaining({ id: "g2" }),
    );
    expect(handlers.onClose).toHaveBeenCalledTimes(1);
  });

  it("opens the crews for a keyboard too, and closes when focus leaves them", () => {
    open(card(), { x: 40, y: 40 }, [group("g1", "everyone"), group("g2", "research")]);

    const opener = screen.getByRole("button", { name: "Move to another group" });
    fireEvent.focus(opener);
    const inside = screen.getByRole("button", { name: "research" });
    expect(inside).toBeTruthy();

    fireEvent.blur(opener, { relatedTarget: inside });
    expect(screen.getByRole("button", { name: "research" })).toBeTruthy();

    fireEvent.blur(inside, { relatedTarget: document.body });
    expect(screen.queryByRole("button", { name: "research" })).toBeNull();
  });

  it("dismisses the group submenu before its parent menu", () => {
    const handlers = open(card(), { x: 40, y: 40 }, [
      group("g1", "everyone"),
      group("g2", "research"),
    ]);
    const opener = screen.getByRole("button", { name: "Move to another group" });
    act(() => opener.focus());
    expect(screen.getByRole("button", { name: "research" })).toBeTruthy();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("button", { name: "research" })).toBeNull();
    expect(handlers.onClose).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(opener);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(handlers.onClose).toHaveBeenCalledTimes(1);
  });

  it("says nothing about groups while there is only one", () => {
    open(card(), { x: 40, y: 40 }, [group("g1", "everyone")]);
    expect(screen.queryByText(/^Move to /)).toBeNull();
  });

  it("closes as it acts, so the menu is never left over a stale row", () => {
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
    const handlers = open(card());
    fireEvent.click(screen.getByRole("button", { name: "Clear chat history…" }));
    expect(handlers.onClearHistory).not.toHaveBeenCalled();
    expect(handlers.onClose).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Delete history, keep memory" }));
    expect(handlers.onClearHistory).toHaveBeenCalledTimes(1);
    expect(handlers.onClose).toHaveBeenCalledTimes(1);
  });

  it("asks twice before deleting an agent, and names it on the way", () => {
    const handlers = open(card({ name: "Scribe" }));
    fireEvent.click(screen.getByRole("button", { name: "Delete…" }));
    expect(handlers.onDelete).not.toHaveBeenCalled();
    expect(handlers.onClose).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Delete Scribe, into the compost" }));
    expect(handlers.onDelete).toHaveBeenCalledTimes(1);
    expect(handlers.onClose).toHaveBeenCalledTimes(1);
  });

  it("holds one confirmation at a time", () => {
    const handlers = open(card());
    fireEvent.click(screen.getByRole("button", { name: "Clear chat history…" }));

    expect(screen.getByRole("button", { name: "Delete…" })).toBeTruthy();
    expect(handlers.onDelete).not.toHaveBeenCalled();
  });

  it("stays inside the window when opened near an edge", () => {
    open(card(), { x: 10_000, y: 10_000 });
    const menu = screen.getByRole("menu");
    expect(Number.parseFloat(menu.style.left)).toBeLessThan(window.innerWidth);
    expect(Number.parseFloat(menu.style.top)).toBeLessThan(window.innerHeight);
  });
});
