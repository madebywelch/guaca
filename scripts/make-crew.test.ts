import { describe, expect, it, vi } from "vitest";

import { moodFor } from "../src/avatars/moods";

async function strip(): Promise<string> {
  const written: string[] = [];
  const write = vi.spyOn(process.stdout, "write").mockImplementation((chunk) => {
    written.push(String(chunk));
    return true;
  });
  try {
    await import("./make-crew");
  } finally {
    write.mockRestore();
  }
  return written.join("");
}

describe("the README's crew strip", () => {
  it("paints amber only on the agent waiting on a person", async () => {
    const svg = await strip();
    const { CREW } = await import("./make-crew");
    const amber = svg.match(/--attention-fill:(#[0-9a-f]{6})/i)?.[1];
    expect(amber).toBeDefined();

    const cells = svg.split("\n").filter((line) => line.startsWith("  <g transform="));
    expect(cells).toHaveLength(CREW.length);

    const asking = moodFor({ activity: { state: "awaitingApproval" } }, 0);
    const painted = cells.flatMap((cell, at) => {
      const drawn = cell.replace(/--(on-)?attention-fill:[^;"]+;?/g, "").toLowerCase();
      return drawn.includes("attention-fill") || drawn.includes(String(amber).toLowerCase())
        ? [CREW[at]?.mood]
        : [];
    });
    expect(painted).toEqual(CREW.filter((member) => member.mood === asking).map((m) => m.mood));
    expect(painted).not.toEqual([]);
  });
});
