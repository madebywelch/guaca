import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { formatCombo } from "../lib/keybinds";
import { useStore } from "../lib/store";
import { anAgent } from "../test-fixtures";
import { ConversationRoster } from "./ConversationRoster";

const TOUCH = "(hover: none) and (pointer: coarse)";

function pointer(coarse: boolean) {
  vi.stubGlobal("matchMedia", (media: string) => ({
    matches: coarse && media === TOUCH,
    media,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
}

function picked(): string[] {
  const picker = screen.getByRole("combobox", { name: "Switch agent" }) as HTMLSelectElement;
  return Array.from(picker.options).map((option) => option.textContent ?? "");
}

beforeEach(() => {
  useStore.setState({
    agents: [anAgent("dara", { name: "Dara" }), anAgent("pip", { name: "Pip" })],
    activity: {},
    outcomes: [],
    pending: [],
    stuck: [],
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

it("draws no key hint on a touch screen, in a row or in the picker, and keeps the accessible shortcut", () => {
  pointer(true);
  const { container } = render(
    <ConversationRoster onOpen={vi.fn()} onQuestions={vi.fn()} foot={null} />,
  );
  expect(container.querySelectorAll("kbd")).toHaveLength(0);
  expect(picked()).toEqual(["Dara, Idle", "Pip, Idle"]);
  expect(screen.getByRole("button", { name: "Dara, Idle" }).getAttribute("aria-keyshortcuts")).toBe(
    "Meta+1 Control+1",
  );
});

it("draws each of the first nine agents' key where there is a keyboard", () => {
  pointer(false);
  const { container } = render(
    <ConversationRoster onOpen={vi.fn()} onQuestions={vi.fn()} foot={null} />,
  );
  const first = formatCombo({ key: "1", mod: true });
  const second = formatCombo({ key: "2", mod: true });
  expect(Array.from(container.querySelectorAll("kbd")).map((one) => one.textContent)).toEqual([
    first,
    second,
  ]);
  expect(picked()).toEqual([`Dara, Idle (${first})`, `Pip, Idle (${second})`]);
});
