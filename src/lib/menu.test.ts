import { describe, expect, it } from "vitest";
import { firstChoice, type MenuRow } from "./menu";

const agent = (id: string, title: string): MenuRow => ({
  id,
  section: "Agents",
  title,
  words: "",
  action: { do: "open", agentId: id },
});

describe("the command menu's first choice", () => {
  it("picks the agent whose whole name was typed over one whose name only contains it", () => {
    const rows = [agent("market", "Market Researcher"), agent("plain", "Researcher")];
    expect(firstChoice(rows, "Researcher")).toBe("plain");
    expect(firstChoice(rows, "researcher ")).toBe("plain");
    expect(firstChoice(rows, "Market")).toBe("market");
    expect(firstChoice(rows, "search")).toBe("market");
  });

  it("chooses a saved hit before the Write row, and never the Write row", () => {
    const write: MenuRow = {
      id: "write",
      section: null,
      title: "Write to Noor",
      words: "",
      action: { do: "write", agentId: "noor", text: "lisbon" },
    };
    const hit: MenuRow = {
      id: "message-1",
      section: "Search results",
      title: "lisbon trip notes",
      words: "",
      action: { do: "result", action: { do: "openMessage", channelId: "noor", messageId: "m1" } },
    };
    expect(firstChoice([hit, write], "lisbon")).toBe("message-1");
    expect(firstChoice([write, hit], "lisbon")).toBe("message-1");
    expect(firstChoice([write], "lisbon")).toBeNull();
    expect(firstChoice([write], "draft the renewal note for friday")).toBeNull();
  });
});
