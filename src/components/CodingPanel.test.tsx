import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { CodingPanel } from "./CodingPanel";

vi.mock("../lib/ipc", () => ({
  api: { messageCodingJob: vi.fn(), stopCodingJob: vi.fn() },
}));

const messageCodingJob = vi.mocked(api.messageCodingJob);
const stopCodingJob = vi.mocked(api.stopCodingJob);

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
        gate: "open",
        createdAt: 0,
        updatedAt: 0,
      },
    ],
    ...over,
  });
}

describe("CodingPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    messageCodingJob.mockResolvedValue(undefined);
    stopCodingJob.mockResolvedValue(undefined);
    seed();
  });

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

  it("sends a correction into the job that is running", async () => {
    // The gap this closes: a job runs for up to forty-five minutes and used to
    // be write-only. An operator watching one go wrong at minute three could
    // only wait for it to finish.
    seed({ building: { [REPO]: AGENT } });
    render(<CodingPanel agent={AGENT} />);

    const box = screen.getByLabelText("Send a correction to the running coding job");
    fireEvent.change(box, { target: { value: "  use the staging bucket  " } });
    fireEvent.click(screen.getByText("Send"));

    await waitFor(() =>
      expect(messageCodingJob).toHaveBeenCalledWith(REPO, "use the staging bucket"),
    );
    // Said, because what was typed never becomes a message anywhere: without a
    // word here the only evidence it arrived is the job changing course later.
    expect(await screen.findByText(/Sent\./)).toBeTruthy();
    expect((box as HTMLInputElement).value).toBe("");
  });

  it("says why a correction was refused rather than looking like it landed", async () => {
    seed({ building: { [REPO]: AGENT } });
    messageCodingJob.mockRejectedValue(new Error("pi has no way to be reached"));
    render(<CodingPanel agent={AGENT} />);

    fireEvent.change(screen.getByLabelText("Send a correction to the running coding job"), {
      target: { value: "stop after the tests" },
    });
    fireEvent.click(screen.getByText("Send"));

    expect(await screen.findByText(/no way to be reached/)).toBeTruthy();
    // The text stays in the box: it was not delivered, so it is still the
    // operator's to send somewhere.
    expect(
      (screen.getByLabelText("Send a correction to the running coding job") as HTMLInputElement)
        .value,
    ).toBe("stop after the tests");
  });

  it("arms the stop before it fires it, and says what survives", async () => {
    // This ends work that cannot be resumed. The confirmation is drawn where
    // the click happened rather than somewhere the operator has to go and find.
    seed({ building: { [REPO]: AGENT } });
    render(<CodingPanel agent={AGENT} />);

    fireEvent.click(screen.getByText("Stop"));
    expect(stopCodingJob).not.toHaveBeenCalled();
    expect(screen.getByText(/Whatever it has committed stays/)).toBeTruthy();

    fireEvent.click(screen.getByText("Stop it"));
    await waitFor(() => expect(stopCodingJob).toHaveBeenCalledWith(REPO));
  });

  it("lets an armed stop be called off", async () => {
    seed({ building: { [REPO]: AGENT } });
    render(<CodingPanel agent={AGENT} />);

    fireEvent.click(screen.getByText("Stop"));
    fireEvent.click(screen.getByText("Keep going"));

    expect(screen.queryByText("Stop it")).toBeNull();
    expect(stopCodingJob).not.toHaveBeenCalled();
  });

  it("belongs to the agent that started the job and nobody else", () => {
    // The panel hangs in one channel. Another agent's job is another agent's
    // business, and drawing it here would say this agent was working.
    seed({ building: { [REPO]: "someone-else" } });
    const { container } = render(<CodingPanel agent={AGENT} />);
    expect(container.firstChild).toBeNull();
  });
});
