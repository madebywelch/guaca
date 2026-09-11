import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { SubscriptionModel as Model } from "../lib/types";
import { SubscriptionModel } from "./ProviderFields";

const discover = vi.hoisted(() => vi.fn<() => Promise<Model[]>>());
vi.mock("../lib/ipc", () => ({ api: { subscriptionModels: discover } }));

beforeEach(() => {
  discover.mockReset();
});

function catalog(...slugs: string[]): Model[] {
  return slugs.map((slug) => ({
    slug,
    defaultReasoningEffort: "medium",
    reasoningEfforts: [
      { effort: "low", description: "Faster responses" },
      { effort: "high", description: "Deeper reasoning" },
    ],
  }));
}

function draw(value = "saved-model", inherit?: string) {
  const onChange = vi.fn();
  const view = render(
    <SubscriptionModel
      effort="auto"
      onEffortChange={vi.fn()}
      value={value}
      models={["fallback-model"]}
      onChange={onChange}
      inherit={inherit}
      hint="Choose a model."
    />,
  );
  return { ...view, onChange };
}

const options = () =>
  [...(screen.getByRole("combobox", { name: /^Model/ }) as HTMLSelectElement).options].map(
    (option) => option.value,
  );

it("keeps the saved selection and explains discovery failure", async () => {
  discover.mockRejectedValue(new Error("offline"));
  const { onChange } = draw();
  expect((await screen.findByRole("status")).textContent).toContain(
    "Could not load current ChatGPT models",
  );
  expect(options()).toEqual(["fallback-model", "saved-model"]);
  expect(onChange).not.toHaveBeenCalled();
});

it("replaces fallback choices with the live catalog without changing the saved model", async () => {
  discover.mockResolvedValue(catalog("new-model", "another-model"));
  const { onChange } = draw();
  await screen.findByRole("option", { name: "new-model" });
  expect(options()).toEqual(["new-model", "another-model", "saved-model"]);
  expect((screen.getByRole("combobox", { name: /^Model/ }) as HTMLSelectElement).value).toBe(
    "saved-model",
  );
  expect(onChange).not.toHaveBeenCalled();
  expect(screen.getByRole("status").textContent).toContain(
    "saved-model is not in your account's current ChatGPT model list",
  );
  fireEvent.change(screen.getByRole("combobox", { name: /^Model/ }), {
    target: { value: "new-model" },
  });
  expect(onChange).toHaveBeenCalledWith("new-model");
});

it("keeps group inheritance available after discovery", async () => {
  discover.mockResolvedValue(catalog("new-model"));
  draw("", "Inherit app model");
  await waitFor(() => expect(options()).toEqual(["", "new-model"]));
  expect((screen.getByRole("combobox", { name: /^Model/ }) as HTMLSelectElement).value).toBe("");
});

it("discovers again on reopening and ignores an abandoned request", async () => {
  let finish!: (models: Model[]) => void;
  discover.mockReturnValueOnce(
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  draw().unmount();
  discover.mockResolvedValueOnce(catalog("current-account-model"));
  draw();
  await screen.findByRole("option", { name: "current-account-model" });
  await act(async () => finish(catalog("old-account-model")));
  expect(options()).toEqual(["current-account-model", "saved-model"]);
  expect(discover).toHaveBeenCalledTimes(2);
});

it("uses the effective model's effort list and preserves an unsupported saved effort", async () => {
  const models = catalog("one", "two");
  models[1]!.reasoningEfforts = [{ effort: "ultra", description: "Most thorough" }];
  discover.mockResolvedValue(models);
  const change = vi.fn();
  const props = {
    models: [],
    onChange: vi.fn(),
    hint: "Choose a model",
    effort: "high" as const,
    onEffortChange: change,
  };
  const view = render(
    <SubscriptionModel
      {...props}
      value=""
      inheritedModel="one"
      inheritedEffort="low"
      effortInherit="Inherit"
    />,
  );
  await screen.findByRole("option", { name: "low" });
  view.rerender(<SubscriptionModel {...props} value="two" />);
  expect(screen.queryByRole("option", { name: "low" })).toBeNull();
  expect(screen.getByRole("option", { name: "ultra" })).toBeTruthy();
  const effort = screen.getByRole("combobox", { name: /^Reasoning effort/ }) as HTMLSelectElement;
  expect(effort.value).toBe("high");
  expect(screen.getByRole("status").textContent).toContain("high is not offered for two");
  expect(change).not.toHaveBeenCalled();
  fireEvent.change(effort, { target: { value: "ultra" } });
  expect(change).toHaveBeenCalledWith("ultra");
});

it("preserves saved effort when catalog discovery fails", async () => {
  discover.mockRejectedValue(new Error("offline"));
  render(
    <SubscriptionModel
      value="one"
      models={["one"]}
      onChange={vi.fn()}
      hint="Model"
      effort="max"
      onEffortChange={vi.fn()}
    />,
  );
  await screen.findByRole("status");
  expect(
    (screen.getByRole("combobox", { name: /^Reasoning effort/ }) as HTMLSelectElement).value,
  ).toBe("max");
});
