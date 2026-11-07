import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { trailStep } from "../lib/trail";
import { TrailFootnote, TrailRow } from "./Trail";

describe("a turn's trail", () => {
  it("does not leave separators after a wrapped footnote item", () => {
    const steps = [
      trailStep(
        {
          type: "toolCall",
          name: "directory",
          arguments: {},
          outcome: { status: "ok", summary: "Found agents" },
        },
        "first",
      ),
      trailStep(
        {
          type: "toolCall",
          name: "request_permission",
          arguments: {},
          outcome: { status: "failed", error: "Nothing happened." },
        },
        "second",
      ),
    ];
    const { container } = render(<TrailFootnote steps={steps} number={1} />);
    expect(container.querySelectorAll(".footnote__item")).toHaveLength(2);
    expect(container.querySelector(".footnote__line")?.textContent).not.toContain("·");
  });

  it("separates a failed call from its reason in the footnote and accessible name", () => {
    const step = trailStep(
      {
        type: "toolCall",
        name: "request_permission",
        arguments: {},
        outcome: { status: "failed", error: "Nothing happened." },
      },
      "call-1",
    );
    const { container } = render(<TrailFootnote steps={[step]} number={1} />);
    const button = screen.getByRole("button", {
      name: "Tried to ask for permission — Nothing happened",
    });
    expect(container.querySelector(".footnote__call")?.textContent).toBe(
      "Tried to ask for permission — Nothing happened",
    );
    fireEvent.click(button);
    expect(button.getAttribute("aria-expanded")).toBe("true");
    expect(container.querySelector(".footnote__steps")?.textContent).toContain("Nothing happened.");
  });

  it("shows one allowance for one ruled call in either presentation", () => {
    const step = trailStep(
      {
        type: "toolCall",
        name: "request_permission",
        arguments: {},
        outcome: { status: "ok", summary: "allowed by a rule" },
        ruled: "Allow reports",
      },
      "call-1",
    );
    const footnote = render(<TrailFootnote steps={[step]} number={1} />);
    expect(footnote.container.textContent?.match(/allowed by a rule/gi)).toHaveLength(1);
    footnote.unmount();
    const chip = render(<TrailRow steps={[step]} />);
    expect(chip.container.textContent?.match(/allowed by a rule/gi)).toHaveLength(1);
  });

  it("separates the failed chip's label and preview for sight and speech", () => {
    const step = trailStep(
      {
        type: "toolCall",
        name: "request_permission",
        arguments: {},
        outcome: { status: "failed", error: "Nothing happened." },
      },
      "call-1",
    );
    render(<TrailRow steps={[step]} />);
    const button = screen.getByRole("button", {
      name: "Tried to ask for permission — Nothing happened.",
    });
    expect(button.textContent).toContain("Tried to ask for permission — Nothing happened.");
    fireEvent.click(button);
    expect(button.getAttribute("aria-expanded")).toBe("true");
    expect(button.textContent).not.toContain(" — ");
  });
});
