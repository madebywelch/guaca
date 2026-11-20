import { describe, expect, it } from "vitest";
import { anAgent } from "../test-fixtures";
import { formatCombo } from "./keybinds";
import { firstChoice, type MenuInput, type MenuRow, menuRows } from "./menu";
import { DEFAULT_PREFS } from "./prefs";

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

describe("the command menu's way back and on", () => {
  const agents = ["Dara", "Marlow", "Ines"].map((name) => anAgent(name.toLowerCase(), { name }));
  const input = (open: string | null): MenuInput => ({
    query: "",
    recipient: undefined,
    agents,
    activity: {},
    pending: [],
    stuck: [],
    open,
    prefs: DEFAULT_PREFS,
  });
  const actions = (open: string | null) =>
    menuRows(input(open)).filter((row) => row.section === "Actions");

  it("names the next and the previous agent by number order, with their keys", () => {
    const rows = actions("marlow");
    const next = rows.find((row) => row.title === "Next agent");
    const previous = rows.find((row) => row.title === "Previous agent");
    expect(next).toMatchObject({
      detail: "Ines",
      key: formatCombo({ key: "Tab", ctrl: true, shift: false }),
      action: { do: "open", agentId: "ines" },
    });
    expect(previous).toMatchObject({
      detail: "Dara",
      key: formatCombo({ key: "Tab", ctrl: true, shift: true }),
      action: { do: "open", agentId: "dara" },
    });
  });

  it("wraps past the last agent, and from Today starts at the first and the last", () => {
    expect(actions("ines").find((row) => row.title === "Next agent")?.detail).toBe("Dara");
    const today = actions(null);
    expect(today.find((row) => row.title === "Next agent")?.detail).toBe("Dara");
    expect(today.find((row) => row.title === "Previous agent")?.detail).toBe("Ines");
  });

  it("offers Back to Today with its key only while an agent is open", () => {
    expect(actions("dara").find((row) => row.title === "Back to Today")?.key).toBe(
      formatCombo({ key: "[", mod: true }),
    );
    expect(actions(null).some((row) => row.title === "Back to Today")).toBe(false);
  });

  it("offers no step to the agent already open when it is the only one", () => {
    const alone = menuRows({ ...input("dara"), agents: agents.slice(0, 1) });
    expect(alone.some((row) => row.id.startsWith("step:"))).toBe(false);
  });
});
