import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { useStore } from "../lib/store";
import { CodingPanel } from "./CodingPanel";

const AGENT = "a1";
const REPO = "r1";

function seed(over: Partial<ReturnType<typeof useStore.getState>> = {}) {
  useStore.setState({
    building: {},
    coding: {},
    repositories: [
      {
        id: REPO,
        groupId: "g1",
        name: "vision-ios",
        path: "/Users/you/dev/vision-ios",
        note: "",
        harness: "pi",
        createdAt: 0,
        updatedAt: 0,
      },
    ],
    ...over,
  });
}

describe("CodingPanel", () => {
  beforeEach(() => seed());

  it("draws nothing when no job is running", () => {
    // Furniture that is always there is furniture. This is on screen only
    // while something is happening that has not landed as a message yet.
    const { container } = render(<CodingPanel agent={AGENT} />);
    expect(container.firstChild).toBeNull();
  });

  it("names the repository being worked in", () => {
    seed({ building: { [REPO]: AGENT } });
    render(<CodingPanel agent={AGENT} />);
    expect(screen.getByText(/Writing code in vision-ios/)).toBeTruthy();
  });

  it("says it is starting before the first tool call arrives", () => {
    // Several seconds of model call sit between starting the harness and its
    // first tool. Silence there reads as a broken panel.
    seed({ building: { [REPO]: AGENT } });
    render(<CodingPanel agent={AGENT} />);
    expect(screen.getByText(/Starting the coding agent/)).toBeTruthy();
  });

  it("shows what the coding agent is running, with the argument worth reading", () => {
    seed({
      building: { [REPO]: AGENT },
      coding: {
        [AGENT]: [
          { tool: "bash", detail: "swift test" },
          { tool: "edit", detail: "Sources/Vision/Pause.swift" },
        ],
      },
    });
    render(<CodingPanel agent={AGENT} />);
    expect(screen.getByText("swift test")).toBeTruthy();
    expect(screen.getByText("Sources/Vision/Pause.swift")).toBeTruthy();
  });

  it("draws what it says differently from what it runs", () => {
    // Prose is the part worth reading and wraps; a command is a line that is
    // clipped. Telling them apart is the whole reason `tool` can be empty.
    seed({
      building: { [REPO]: AGENT },
      coding: { [AGENT]: [{ tool: "", detail: "The pause flow has no test coverage." }] },
    });
    const { container } = render(<CodingPanel agent={AGENT} />);
    expect(container.querySelector(".coding__said")).toBeTruthy();
    expect(container.querySelector(".coding__tool")).toBeNull();
  });

  it("keeps two identical lines, because two test runs are two test runs", () => {
    seed({
      building: { [REPO]: AGENT },
      coding: {
        [AGENT]: [
          { tool: "bash", detail: "swift test" },
          { tool: "bash", detail: "swift test" },
        ],
      },
    });
    render(<CodingPanel agent={AGENT} />);
    expect(screen.getAllByText("swift test")).toHaveLength(2);
  });

  it("belongs to the agent that started the job and nobody else", () => {
    // The panel hangs in one channel. Another agent's job is another agent's
    // business, and drawing it here would say this agent was working.
    seed({ building: { [REPO]: "someone-else" } });
    const { container } = render(<CodingPanel agent={AGENT} />);
    expect(container.firstChild).toBeNull();
  });
});
