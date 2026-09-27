import { describe, expect, it } from "vitest";

import { describeQuick, quickTitle } from "./quick";
import type { QuickAction } from "./types";

const names = (id: string) => (id === "a1" ? "Scout" : "a deleted agent");

describe("a quick action, in words", () => {
  it("quotes the message whole, as the approval card did", () => {
    const action: QuickAction = {
      id: "q",
      label: "Brief",
      does: { kind: "message", agentId: "a1", text: "Brief me." },
      addedBy: "Pip",
    };
    expect(describeQuick(action, names)).toBe("Sends Scout: “Brief me.”");
    expect(quickTitle(action, names)).toContain("Added by Pip, with your approval.");
  });

  it("says where a place is, and nothing about who added the operator's own", () => {
    const open = (place: QuickAction["does"]): QuickAction => ({
      id: "q",
      label: "Go",
      does: place,
      addedBy: "the operator",
    });
    expect(describeQuick(open({ kind: "open", place: { kind: "forYou" } }), names)).toBe(
      "Opens For You",
    );
    const limits = open({ kind: "open", place: { kind: "settings", section: "limits" } });
    expect(quickTitle(limits, names)).toBe("Opens Settings on limits");
    const gone = open({ kind: "open", place: { kind: "channel", agentId: "zz" } });
    expect(describeQuick(gone, names)).toBe("Opens the channel with a deleted agent");
  });
});
