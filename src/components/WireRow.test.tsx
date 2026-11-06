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
  /** What a note says, ignoring the live-only dots. */
  const said = (root: HTMLElement) =>
    [...root.querySelectorAll(".footnote__send")].map((one) => one.textContent);

  it("is announced as the line it settles into", () => {
    // One message landing must not read as two events. The live row was a
    // sentence with an arrow and no face, and the settled row a chip with a
    // face and different words, so a message arriving moved, renamed itself and
    // grew an avatar in one frame. Both are the same line of the same page now,
    // and the only thing that changes is the dots giving way to the gist.
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
    // The row is the page's apparatus: one line under the turn, however many
    // peers were involved. A line apiece is the burial the burst was collapsed
    // to fix in the first place.
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
    // And it is numbered, because a note is a position in a page.
    expect(container.querySelector(".footnote__number")?.textContent).toBe("2");
  });
});
