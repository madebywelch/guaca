import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { WirePeer } from "../lib/transcript";
import { PeerBurstRow, WritingRow, why } from "./WireRow";

const GOV: WirePeer = {
  id: "gov",
  name: "Government Procurement",
  color: "#c7d96b",
  avatar: "plain",
};
const REV: WirePeer = { id: "rev", name: "Revenue Operations", color: "#8bbf6a", avatar: "plain" };

describe("why", () => {
  it("keeps what happened and drops the instructions to the model", () => {
    // Guard refusals are written for a model reading them mid-turn: a label, a
    // reason, and what to do instead. A chip wants the middle one.
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

describe("a peer message arriving", () => {
  /** What a chip says and what it is wearing, ignoring the live-only dots. */
  const chips = (root: HTMLElement) =>
    [...root.querySelectorAll(".wire__chip")].map((chip) => ({
      label: chip.querySelector(".wire__label")?.textContent,
      // The size class, so a chip that grew or lost its face fails here.
      avatar: chip.querySelector(".avatar")?.className,
    }));

  it("is announced as the chip it settles into", () => {
    // One message landing must not read as two events. The live row was a
    // sentence with an arrow and no avatar, and the settled row a chip with an
    // avatar and different words, so a message arriving moved, renamed itself
    // and grew a face in one frame.
    const live = render(<WritingRow peer={GOV} />);
    const settled = render(
      <PeerBurstRow
        peers={[{ peer: GOV, agentId: "gov", sent: 0, received: 1 }]}
        onOpen={vi.fn()}
      />,
    );

    expect(chips(live.container)).toEqual(chips(settled.container));
    expect(chips(live.container)).toEqual([
      { label: "Message from Government Procurement", avatar: "avatar avatar--xs" },
    ]);
    expect(live.container.querySelector(".wire__dots")).not.toBeNull();
    expect(settled.container.querySelector(".wire__dots")).toBeNull();
  });

  it("keeps a burst in one group, so the rules bracket it rather than wrap into it", () => {
    // The rules either side of a wire row are `.wire` pseudo-elements. As items
    // of a wrapping flex row they were laid out with the chips: the left rule
    // squeezed onto the first line, the right one stranded beside whichever
    // chip wrapped last. Every chip goes in one child, and `.wire` never wraps.
    const { container } = render(
      <PeerBurstRow
        peers={[
          { peer: GOV, agentId: "gov", sent: 1, received: 1 },
          { peer: REV, agentId: "rev", sent: 0, received: 1 },
        ]}
        onOpen={vi.fn()}
      />,
    );

    const wire = container.querySelector(".wire");
    expect(wire?.children).toHaveLength(1);
    expect(wire?.firstElementChild?.className).toBe("wire__chips");
    expect(chips(container).map((c) => c.label)).toEqual([
      "2 messages with Government Procurement",
      "Message from Revenue Operations",
    ]);
  });
});
