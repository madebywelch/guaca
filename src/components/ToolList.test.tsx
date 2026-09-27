import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { ToolSummary } from "../lib/types";

const builtinTools = vi.fn<() => Promise<ToolSummary[]>>();
vi.mock("../lib/ipc", () => ({ api: { builtinTools: () => builtinTools() } }));

import { ToolList } from "./ToolList";

describe("ToolList", () => {
  it("names each tool, says what the agent is told, and what it needs first", async () => {
    builtinTools.mockResolvedValue([
      { name: "skill", summary: "Skills are documents of instructions.", needs: null },
      { name: "browse", summary: "Use your browser.", needs: "a browser" },
    ]);
    render(<ToolList />);
    expect(await screen.findByText("skill")).toBeTruthy();
    expect(screen.getByText("Use your browser.")).toBeTruthy();
    expect(screen.getByText("needs a browser")).toBeTruthy();
  });

  it("says why when the host cannot list them", async () => {
    builtinTools.mockRejectedValue({ kind: "unknownCommand", message: "update the host" });
    render(<ToolList />);
    expect((await screen.findByRole("alert")).textContent).toBe("update the host");
  });
});
