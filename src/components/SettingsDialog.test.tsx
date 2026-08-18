import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { ComputerProviderStatus, Settings, SettingsPatch } from "../lib/types";
import { SettingsDialog } from "./SettingsDialog";

const updateSettings = vi.fn<(patch: SettingsPatch) => Promise<Settings>>();
const computerProviderStatuses = vi.fn<() => Promise<ComputerProviderStatus[]>>();

vi.mock("../lib/ipc", () => ({
  api: {
    updateSettings: (patch: SettingsPatch) => updateSettings(patch),
    testConnection: vi.fn(),
    computerProviderStatuses: () => computerProviderStatuses(),
  },
}));

const SAVED: Settings = {
  operatorName: "",
  e2bKeySet: false,
  e2bKeyHint: "",
  computerProvider: "automatic",
  computerIdleMinutes: 15,
  baseUrl: "https://openrouter.ai/api/v1",
  defaultModel: "test/model",
  apiKeySet: true,
  apiKeyHint: "...9999",
  requestTimeoutSecs: 120,
  limits: {
    maxHops: 8,
    maxStepsPerRun: 60,
    maxFanoutPerCall: 8,
    maxSendsPerPair: 6,
    maxToolRounds: 24,
  },
};

const NOTHING_READY: ComputerProviderStatus[] = [
  {
    provider: "appleContainer",
    state: "notInstalled",
    canStart: false,
    detail: "Apple Container is not installed. Get it from github.com/apple/container/releases.",
  },
  { provider: "e2b", state: "notInstalled", canStart: false, detail: "E2B needs an API key." },
];

function open(settings: Settings = SAVED) {
  useStore.setState({ settings, computerStatuses: [] });
  return render(<SettingsDialog onClose={() => {}} />);
}

/** The one dropdown in the dialog: which provider runs new computers. */
function selector(): HTMLSelectElement {
  return screen.getByRole("combobox") as HTMLSelectElement;
}

describe("SettingsDialog", () => {
  beforeEach(() => {
    updateSettings.mockReset();
    updateSettings.mockResolvedValue(SAVED);
    computerProviderStatuses.mockReset();
    computerProviderStatuses.mockResolvedValue(NOTHING_READY);
  });

  it("offers every provider this build can drive, and saves the one chosen", async () => {
    open();
    expect(screen.getByText("Computer provider")).toBeTruthy();
    const options = [...selector().options].map((option) => option.value);
    expect(options).toEqual(["automatic", "appleContainer", "e2b"]);
    expect(selector().value).toBe("automatic");

    fireEvent.change(selector(), { target: { value: "appleContainer" } });
    fireEvent.click(screen.getByText("Save"));

    await waitFor(() => expect(updateSettings).toHaveBeenCalled());
    expect(updateSettings.mock.calls[0]![0]!.computerProvider).toBe("appleContainer");
  });

  it("says what each provider would do if asked for a machine now", async () => {
    // The status is the whole point of the section: "no computer" with no
    // reason leaves the operator with nothing to act on.
    open();
    await waitFor(() => expect(computerProviderStatuses).toHaveBeenCalled());
    await screen.findByText(/Apple Container is not installed/);
    expect(screen.getByText("E2B needs an API key.")).toBeTruthy();
    // The state as a word, so a line that has not been read still reads.
    expect(screen.getAllByText("not installed").length).toBe(2);
  });

  it("discloses what a local computer can reach, and only for a local one", async () => {
    // Local mode is not a claim that agent commands are harmless: the guest
    // cannot see host files, but it is on this Mac's network.
    open();
    expect(screen.queryByText(/may reach services exposed by this Mac/)).toBeNull();

    fireEvent.change(selector(), { target: { value: "appleContainer" } });
    expect(
      screen.getByText(/Local computers run untrusted agent commands on this Mac/),
    ).toBeTruthy();
    expect(screen.getByText(/Use E2B when you need an off-device network boundary/)).toBeTruthy();

    fireEvent.change(selector(), { target: { value: "e2b" } });
    expect(screen.queryByText(/may reach services exposed by this Mac/)).toBeNull();
  });

  it("discloses it for automatic too, once a local provider is what automatic would pick", async () => {
    // `automatic` is not a promise of the hosted provider. On a Mac where the
    // local one is ready, leaving the setting alone is choosing it, and the
    // operator should read the same warning they would have read by naming it.
    computerProviderStatuses.mockResolvedValue([
      {
        provider: "appleContainer",
        state: "ready",
        canStart: false,
        detail: "Apple Container 1.2.2 is running.",
      },
      { provider: "e2b", state: "notInstalled", canStart: false, detail: "E2B needs an API key." },
    ]);
    open();

    await screen.findByText(/Local computers run untrusted agent commands on this Mac/);
    expect(selector().value).toBe("automatic");
  });

  it("promises nothing about the computers that already exist", async () => {
    // Changing the setting must never read as a migration: a machine keeps the
    // provider that made it, and its disk, until the operator destroys it.
    open();
    expect(screen.queryByText(/Existing computers keep their current provider/)).toBeNull();

    fireEvent.change(selector(), { target: { value: "e2b" } });
    expect(screen.getByText(/Existing computers keep their current provider/)).toBeTruthy();

    // Back to what is saved: nothing is changing, so there is nothing to say.
    fireEvent.change(selector(), { target: { value: "automatic" } });
    expect(screen.queryByText(/Existing computers keep their current provider/)).toBeNull();
  });

  it("asks again after a save, because a key just added is a provider that works", async () => {
    open();
    await waitFor(() => expect(computerProviderStatuses).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByText("Save"));
    await waitFor(() => expect(computerProviderStatuses).toHaveBeenCalledTimes(2));
  });
});
