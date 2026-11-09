import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { isFault, readChart } from "../lib/chart";
import { Chart } from "./Chart";

describe("chart interaction", () => {
  it("keeps the highlighted category while hiding and showing a series", () => {
    const result = readChart({
      type: "bar",
      labels: ["Q1", "Q2"],
      series: [
        { name: "North", data: [10, 20] },
        { name: "South", data: [30, 40] },
      ],
    });
    if (isFault(result)) throw new Error(result.why);
    const { container } = render(<Chart chart={result.chart} />);
    const bands = container.querySelectorAll(".chart__band");
    fireEvent.pointerEnter(bands[0]!);
    expect(container.querySelector(".chart__readout")?.textContent).toContain("North");
    fireEvent.click(screen.getByRole("button", { name: "North" }));
    expect(container.querySelector(".chart__readout")?.textContent).not.toContain("North");
    expect(container.querySelector(".chart__readout")?.textContent).toContain("South");
    fireEvent.click(screen.getByRole("button", { name: "South" }));
    expect(screen.getByRole("button", { name: "South" }).getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: "North" }));
    expect(container.querySelector(".chart__readout")?.textContent).toContain("North");
    fireEvent.pointerLeave(screen.getByRole("img"));
    expect(container.querySelector(".chart__readout")).toBeNull();
  });
});
