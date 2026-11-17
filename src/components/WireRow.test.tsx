import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useEffect, useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { bindingFor } from "../lib/keybinds";
import type { WirePeer } from "../lib/transcript";
import { Search } from "./Search";
import { SettingsDialog } from "./SettingsDialog";
import { NoticeRow, PeerBurstRow, RefusedRow, WritingRow, why } from "./WireRow";

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
  const said = (root: HTMLElement) =>
    [...root.querySelectorAll(".footnote__send")].map((one) => one.textContent);

  it("is announced as the line it settles into", () => {
    const live = render(<WritingRow peer={GOV} />);
    const settled = render(
      <PeerBurstRow
        peers={[{ peer: GOV, agentId: "gov", sent: 0, received: 1, gist: null }]}
        number={1}
        at={0}
        onOpen={vi.fn()}
      />,
    );

    expect(said(live.container)).toEqual(["Message from Government Procurement"]);
    expect(said(settled.container)).toEqual(said(live.container));
    expect(live.container.querySelector(".footnote__dots")).not.toBeNull();
    expect(settled.container.querySelector(".footnote__dots")).toBeNull();
  });

  it("keeps a burst on one line, with the count it stands for", () => {
    const { container } = render(
      <PeerBurstRow
        peers={[
          { peer: GOV, agentId: "gov", sent: 1, received: 1, gist: null },
          { peer: REV, agentId: "rev", sent: 0, received: 1, gist: null },
        ]}
        number={2}
        at={0}
        onOpen={vi.fn()}
      />,
    );

    const line = container.querySelector(".footnote__line");
    expect(line?.children).toHaveLength(2);
    expect(said(container)).toEqual([
      "2 messages with Government Procurement",
      "Message from Revenue Operations",
    ]);
    expect(container.querySelector(".footnote__number")?.textContent).toBe("2");
  });
});

function Workspace() {
  const [upper, setUpper] = useState<"settings" | "search" | null>(null);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const binding = bindingFor(event);
      if (binding?.id === "settings" || binding?.id === "search") {
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
      <RefusedRow peer={GOV} at={0} number={1} body="The message" reason="Refused: no" />
      {upper === "settings" && <SettingsDialog onClose={close} />}
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
  it.each([
    ["Settings", ","],
    ["Search", "f"],
  ])("dismisses %s before the wire detail and restores focus", async (name, key) => {
    render(<Workspace />);
    const opener = screen.getByRole("button", { name: /Not delivered to Government Procurement/ });
    opener.focus();
    fireEvent.click(opener);
    const detail = screen.getByRole("dialog", { name: "Not delivered to Government Procurement" });
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
  });
});
