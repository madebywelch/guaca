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
});
