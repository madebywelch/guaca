import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "../lib/store";
import type { Settings, SettingsPatch } from "../lib/types";
import { SettingsDialog } from "./SettingsDialog";

interface FetchModelsOptions {
  patch?: SettingsPatch;
  groupId?: string;
}

const fetchModels = vi.fn<(options?: FetchModelsOptions) => Promise<string[]>>();
const onClose = vi.fn();

vi.mock("../lib/ipc", () => ({
  api: {
    fetchModels: (options?: FetchModelsOptions) => fetchModels(options),
  },
}));

function settings(): Settings {
  return {
    operatorName: "",
    baseUrl: "https://openrouter.ai/api/v1",
    defaultModel: "current/model",
    apiKeySet: true,
    apiKeyHint: "...9999",
    e2bKeySet: false,
    e2bKeyHint: "",
    computerIdleMinutes: 15,
    requestTimeoutSecs: 120,
    limits: {
      maxHops: 8,
      maxStepsPerRun: 60,
      maxFanoutPerCall: 8,
      maxSendsPerPair: 6,
      maxToolRounds: 24,
    },
  };
}

describe("SettingsDialog model catalogue", () => {
  beforeEach(() => {
    fetchModels.mockReset();
    onClose.mockReset();
    useStore.setState({ settings: settings() });
  });

  it("keeps the editable field and explains a fetch failure", async () => {
    fetchModels.mockRejectedValue({ kind: "inference", message: "API key rejected" });
    render(<SettingsDialog onClose={onClose} />);

    fireEvent.click(screen.getByRole("button", { name: "Fetch models" }));

    expect((await screen.findByRole("alert")).textContent).toContain("API key rejected");
    expect(screen.getByRole("textbox", { name: "Default model" })).toBeTruthy();
    // An empty password field means the stored key, so it must remain omitted
    // rather than replacing that key with an empty string.
    expect(fetchModels).toHaveBeenCalledWith({
      patch: { baseUrl: "https://openrouter.ai/api/v1" },
    });
  });

  it("uses an unsaved key and turns the model field into a dropdown", async () => {
    fetchModels.mockResolvedValue(["alpha/model", "zeta/model"]);
    render(<SettingsDialog onClose={onClose} />);
    fireEvent.change(screen.getByLabelText(/^API key/), {
      target: { value: "  sk-unsaved  " },
    });

    fireEvent.click(screen.getByRole("button", { name: "Fetch models" }));

    await waitFor(() =>
      expect(fetchModels).toHaveBeenCalledWith({
        patch: {
          baseUrl: "https://openrouter.ai/api/v1",
          apiKey: "sk-unsaved",
        },
      }),
    );
    const dropdown = await screen.findByRole<HTMLSelectElement>("combobox", {
      name: "Default model",
    });
    expect(dropdown.value).toBe("alpha/model");
    expect([...dropdown.options].map((option) => option.value)).toEqual([
      "alpha/model",
      "zeta/model",
    ]);

    fireEvent.change(dropdown, { target: { value: "zeta/model" } });
    expect(dropdown.value).toBe("zeta/model");
  });
});
