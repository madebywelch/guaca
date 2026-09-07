import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { SubscriptionModel } from "./ProviderFields";

const discover = vi.hoisted(() => vi.fn<() => Promise<string[]>>());
vi.mock("../lib/ipc", () => ({ api: { subscriptionModels: discover } }));

beforeEach(() => {
  discover.mockReset();
});

function draw(value = "saved-model", inherit?: string) {
  const onChange = vi.fn();
  const view = render(
    <SubscriptionModel
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
  [...(screen.getByRole("combobox") as HTMLSelectElement).options].map((option) => option.value);

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
  discover.mockResolvedValue(["new-model", "another-model"]);
  const { onChange } = draw();
  await screen.findByRole("option", { name: "new-model" });
  expect(options()).toEqual(["new-model", "another-model", "saved-model"]);
  expect((screen.getByRole("combobox") as HTMLSelectElement).value).toBe("saved-model");
  expect(onChange).not.toHaveBeenCalled();
  fireEvent.change(screen.getByRole("combobox"), { target: { value: "new-model" } });
  expect(onChange).toHaveBeenCalledWith("new-model");
});

it("keeps group inheritance available after discovery", async () => {
  discover.mockResolvedValue(["new-model"]);
  draw("", "Inherit app model");
  await waitFor(() => expect(options()).toEqual(["", "new-model"]));
  expect((screen.getByRole("combobox") as HTMLSelectElement).value).toBe("");
});

it("discovers again on reopening and ignores an abandoned request", async () => {
  let finish!: (models: string[]) => void;
  discover.mockReturnValueOnce(
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  draw().unmount();
  discover.mockResolvedValueOnce(["current-account-model"]);
  draw();
  await screen.findByRole("option", { name: "current-account-model" });
  await act(async () => finish(["old-account-model"]));
  expect(options()).toEqual(["current-account-model", "saved-model"]);
  expect(discover).toHaveBeenCalledTimes(2);
});
