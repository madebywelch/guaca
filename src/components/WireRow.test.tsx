import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useEffect, useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { bindingFor } from "../lib/keybinds";
import { useStore } from "../lib/store";
import type { PeerSummary, WirePeer } from "../lib/transcript";
import type { RunId } from "../lib/types";
import { Search } from "./Search";
import { HandoffRow, NoticeRow, RefusedRow, WritingRow, why } from "./WireRow";

vi.mock("../lib/ipc", async (original) => {
  const actual = await original<typeof import("../lib/ipc")>();
  return {
    ...actual,
    api: {
      ...actual.api,
      search: async () => ({ messages: [], files: [], links: [], routines: [], artifacts: [] }),
    },
  };
});

const GOV: WirePeer = {
  id: "gov",
  name: "Government Procurement",
  color: "#c7d96b",
  avatar: "plain",
};
const REV: WirePeer = { id: "rev", name: "Revenue Operations", color: "#8bbf6a", avatar: "plain" };

describe("why", () => {
  it("keeps what happened and drops the instructions to the model", () => {
    expect(
      why(
        "Refused: you already sent Chef this exact message in this run. Repeating it will not produce a different reply. Move on.",
      ),
    ).toBe("you already sent Chef this exact message in this run");
  });

  it("survives a reason that is one sentence, or none of the expected shape", () => {
    expect(why("Refused: no agent named Ghost exists")).toBe("no agent named Ghost exists");
    expect(why("something went wrong")).toBe("something went wrong");
  });
});

describe("a system notice", () => {
  it("keeps its paragraphs apart rather than running the second onto the first", () => {
    const text =
      "Analyst could not reply: upstream exploded\n\nIt had already made 2 calls in this turn, listed above.";
    const { container } = render(<NoticeRow kind="upstreamError" text={text} />);
    const words = [...container.querySelectorAll<HTMLElement>(".chip span")].find((span) =>
      span.textContent?.startsWith("Analyst"),
    );
    expect(words?.textContent).toBe(text);
    expect(words?.style.whiteSpace).toBe("pre-line");
  });
});

describe("a peer message arriving", () => {
  it("is announced by name while it is written", () => {
    const { container } = render(<WritingRow peer={GOV} />);
    expect(container.querySelector(".footnote__send")?.textContent).toBe(
      "Message from Government Procurement",
    );
    expect(container.querySelector(".footnote__dots")).not.toBeNull();
  });
});

function Toggled({ peers, onOpen }: { peers: PeerSummary[]; onOpen: (peer: string) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <HandoffRow
      peers={peers}
      runId={"run-1" as RunId}
      channel="lead"
      rowKey="peers:1"
      open={open}
      onToggle={() => setOpen((held) => !held)}
      onOpen={onOpen}
    />
  );
}

describe("a hand-off", () => {
  afterEach(() => {
    vi.useRealTimers();
  });
  const RUN = "run-1" as RunId;
  const NOW = 100_000;
  const asked = (peer: WirePeer, over: Partial<PeerSummary> = {}): PeerSummary => ({
    peer,
    agentId: peer.id,
    sent: 1,
    received: 0,
    gist: null,
    asked: `What does ${peer.name} see this quarter?`,
    askedAt: NOW - 15_000,
    answer: null,
    answeredAt: null,
    ...over,
  });
  const answered = asked(GOV, {
    received: 1,
    answer: "Two tenders closed early.",
    answeredAt: NOW - 6_000,
  });

  function draw(peers: PeerSummary[], live: boolean, onOpen = vi.fn()) {
    vi.useFakeTimers({ now: NOW, toFake: ["Date"] });
    act(() => {
      useStore.setState({
        running: live
          ? [{ runId: RUN, agentId: "lead", groupId: "g", startedAt: NOW - 20_000, note: null }]
          : [],
        activity: { rev: { state: "thinking" } },
        activeRun: { rev: RUN },
        streams: {},
      });
    });
    return render(<Toggled peers={peers} onOpen={onOpen} />);
  }

  it("draws a colleague who wrote unasked as a line that opens the pair's thread", () => {
    const onOpen = vi.fn();
    draw(
      [asked(GOV, { sent: 0, received: 1, asked: null, askedAt: null, gist: "Two tenders" })],
      true,
      onOpen,
    );
    expect(screen.queryByRole("region", { name: /Asked/ })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Message from Government Procurement" }));
    expect(onOpen).toHaveBeenCalledWith("gov");
  });

  it("shows each colleague's face, the question, what they are doing and for how long, and an answer at reading size", () => {
    const { container } = draw([answered, asked(REV)], true);
    const card = screen.getByRole("region", { name: "Asked 2 colleagues" });
    expect(
      within(card).getByText("What does Government Procurement see this quarter?"),
    ).toBeTruthy();
    expect(within(card).getByText("answered in 9s")).toBeTruthy();
    expect(container.querySelector(".handoff__answer .md")?.textContent).toContain(
      "Two tenders closed early.",
    );
    expect(within(card).getByText("working, 15s")).toBeTruthy();
    expect(card.querySelectorAll(".avatar")).toHaveLength(2);
  });

  it("folds to one line once every colleague has answered, and opens again on a press", () => {
    draw([answered, asked(REV, { received: 1, answer: "Flat.", answeredAt: NOW - 1_000 })], true);
    expect(screen.queryByRole("region", { name: /Asked/ })).toBeNull();
    const fold = screen.getByRole("button", {
      name: /Asked Government Procurement and Revenue Operations, both answered/,
    });
    expect(fold.textContent).toContain("14s");
    fireEvent.click(fold);
    expect(fold.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("Flat.")).toBeTruthy();
  });

  it("folds when the conversation ended without an answer, and says who answered", () => {
    draw([answered, asked(REV)], false);
    expect(
      screen.getByRole("button", {
        name: /Asked Government Procurement and Revenue Operations, Government Procurement answered/,
      }),
    ).toBeTruthy();
  });

  it("opens the pair's own thread from a colleague's name", () => {
    const onOpen = vi.fn();
    draw([answered, asked(REV)], true, onOpen);
    fireEvent.click(screen.getByRole("button", { name: "Revenue Operations" }));
    expect(onOpen).toHaveBeenCalledWith("rev");
  });
});

function Workspace() {
  const [upper, setUpper] = useState<"search" | null>(null);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const binding = bindingFor(event);
      if (binding?.id === "search") {
        event.preventDefault();
        setUpper(binding.id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const close = () => setUpper(null);
  return (
    <>
      <RefusedRow peer={GOV} at={0} body="The message" reason="Refused: no" />
      {upper === "search" && (
        <Search
          onClose={close}
          onOpenChannel={vi.fn()}
          onEditAgent={vi.fn()}
          onEditGroup={vi.fn()}
          onNewAgent={vi.fn()}
          onNewGroup={vi.fn()}
          onOpenCafeteria={vi.fn()}
          onOpenSettings={vi.fn()}
        />
      )}
    </>
  );
}

describe("a wire detail under a workspace dialog", () => {
  it.each([["Search", "f"]])(
    "dismisses %s before the wire detail and restores focus",
    async (name, key) => {
      render(<Workspace />);
      const opener = screen.getByRole("button", {
        name: /Not delivered to Government Procurement/,
      });
      opener.focus();
      fireEvent.click(opener);
      const detail = screen.getByRole("dialog", {
        name: "Not delivered to Government Procurement",
      });
      const close = within(detail).getByRole("button", { name: "Close" });
      close.focus();
      fireEvent.keyDown(window, { key, ctrlKey: true });
      const upper = await screen.findByRole("dialog", { name });
      await waitFor(() => expect(detail.contains(document.activeElement)).toBe(false));
      fireEvent.keyDown(window, { key: "Escape" });
      expect(upper.isConnected).toBe(false);
      expect(detail.isConnected).toBe(true);
      expect(document.activeElement).toBe(close);
      fireEvent.keyDown(window, { key: "Escape" });
      expect(detail.isConnected).toBe(false);
      expect(document.activeElement).toBe(opener);
    },
  );
});
