import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { focusWriting } from "../lib/focus";
import { HOLD_MS, useHeld } from "../lib/held";
import { useStore } from "../lib/store";
import type { Escalation } from "../lib/types";
import { anAgent, anApproval } from "../test-fixtures";
import { QuestionTray } from "./QuestionTray";

const dara = anAgent("agent-1", { name: "Dara" });
const NOW = new Date(2026, 10, 16, 9, 12).getTime();

function TrayHarness({ first }: { first: string }) {
  const [selected, setSelected] = useState<string | null>(first);
  return (
    <>
      <textarea data-field aria-label="Write to Dara" />
      {selected && (
        <QuestionTray
          selected={selected}
          onSelect={setSelected}
          onOpen={() => {}}
          onClose={() => {
            setSelected(null);
            focusWriting();
          }}
        />
      )}
    </>
  );
}

afterEach(() => vi.useRealTimers());

describe("Needs you as a keyboard queue", () => {
  const noor = anAgent("agent-2", { name: "Noor" });
  const permission = anApproval({ agentId: "agent-1", summary: "Change your name to Robin?" });
  const question = anApproval({
    id: "req-noor",
    agentId: "agent-2",
    summary: "Which story leads Friday?",
    request: { kind: "question", options: ["Notebooks", "The renewal"] },
  });
  const decide = vi.fn(async () => {});
  const answer = vi.fn(async () => {});

  function drawQueue(pending = [permission, question], stuck: Escalation[] = []) {
    act(() => {
      useHeld.setState({ held: null, notice: null });
      useStore.setState({
        agents: [dara, noor],
        groups: [],
        pending,
        stuck,
        decisions: [],
        outcomes: [],
        activity: {},
        lastActive: {},
        railGroup: null,
        refreshOutcomes: async () => {},
        decideApproval: decide,
        answerQuestion: answer,
      });
    });
    render(<TrayHarness first={pending[0]?.id ?? stuck[0]?.id ?? ""} />);
  }

  const card = (name: RegExp) => screen.getByRole("article", { name });

  beforeEach(() => {
    vi.useFakeTimers({
      now: NOW,
      toFake: ["Date", "setTimeout", "clearTimeout", "requestAnimationFrame"],
    });
    decide.mockClear();
    answer.mockClear();
  });

  it("focuses the first card and J and K move the selection", () => {
    drawQueue();
    expect(document.activeElement).toBe(card(/^Dara asks/));
    expect(card(/^Dara asks/).hasAttribute("data-selected")).toBe(true);
    fireEvent.keyDown(document.activeElement as Element, { key: "j" });
    expect(document.activeElement).toBe(card(/^Noor asks/));
    expect(card(/^Dara asks/).hasAttribute("data-selected")).toBe(false);
    fireEvent.keyDown(document.activeElement as Element, { key: "k" });
    expect(document.activeElement).toBe(card(/^Dara asks/));
  });

  it("draws the keys on the selected card's buttons and on no other card", () => {
    drawQueue();
    const selected = card(/^Dara asks/);
    expect(within(selected).getByRole("button", { name: "Allow" }).textContent).toMatch(/↩|Enter/);
    expect(within(card(/^Noor asks/)).getByRole("button", { name: "Notebooks" }).textContent).toBe(
      "Notebooks",
    );
    fireEvent.keyDown(selected, { key: "j" });
    expect(within(card(/^Noor asks/)).getByRole("button", { name: "Notebooks" }).textContent).toBe(
      "Notebooks1",
    );
  });

  it("holds an Allow for the Undo's few seconds, moves on, and only then sends it", () => {
    drawQueue();
    fireEvent.keyDown(document.activeElement as Element, { key: "Enter", metaKey: true });
    expect(decide).not.toHaveBeenCalled();
    expect(screen.queryByRole("article", { name: /^Dara asks/ })).toBeNull();
    expect(document.activeElement).toBe(card(/^Noor asks/));
    expect(useHeld.getState().notice?.text).toBe("Allowed what Dara asked.");
    act(() => vi.advanceTimersByTime(HOLD_MS));
    expect(decide).toHaveBeenCalledExactlyOnceWith(permission.id, "allow");
  });

  it("holds a clear and names whose question it cleared", () => {
    const clear = vi.fn(async () => {});
    const stuck = {
      id: "esc-noor",
      agentId: "agent-2",
      groupId: noor.groupId,
      runId: "run-1",
      summary: "The deploy needs a key only you have.",
      raisedAt: NOW,
      saidAt: NOW,
      times: 1,
      clearedAt: null,
    } as unknown as Escalation;
    drawQueue([], [stuck]);
    act(() => useStore.setState({ clearEscalation: clear }));
    fireEvent.click(within(card(/Noor/)).getByRole("button", { name: "Clear" }));
    expect(useHeld.getState().notice?.text).toBe("Cleared what Noor asked.");
    expect(clear).not.toHaveBeenCalled();
    act(() => vi.advanceTimersByTime(HOLD_MS));
    expect(clear).toHaveBeenCalledExactlyOnceWith("esc-noor");
  });

  it("sends nothing at all when the answer is undone in time, and puts the card back", () => {
    drawQueue();
    fireEvent.keyDown(document.activeElement as Element, { key: "Backspace", ctrlKey: true });
    act(() => {
      useHeld.getState().undo();
    });
    act(() => vi.advanceTimersByTime(HOLD_MS * 2));
    expect(decide).not.toHaveBeenCalled();
    expect(card(/^Dara asks/)).toBeTruthy();
  });

  it("answers a question with its number, and gives the composer back after the last card", () => {
    drawQueue([question]);
    fireEvent.keyDown(document.activeElement as Element, { key: "2" });
    act(() => vi.advanceTimersByTime(HOLD_MS));
    expect(answer).toHaveBeenCalledExactlyOnceWith("req-noor", "The renewal");
    act(() => vi.advanceTimersByTime(16));
    expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "Write to Dara" }));
  });

  it("sends a held answer at once when the next one is given, so two answers never wait on one Undo", () => {
    drawQueue();
    fireEvent.keyDown(document.activeElement as Element, { key: "Enter", metaKey: true });
    fireEvent.keyDown(document.activeElement as Element, { key: "1" });
    expect(decide).toHaveBeenCalledExactlyOnceWith(permission.id, "allow");
    expect(answer).not.toHaveBeenCalled();
  });
});
