import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { PermissionRule } from "../lib/types";
import { Permissions } from "./Permissions";

const { list, save, remove, plugins } = vi.hoisted(() => ({
  list: vi.fn(),
  save: vi.fn(),
  remove: vi.fn(),
  plugins: vi.fn(),
}));
vi.mock("../lib/ipc", () => ({
  api: {
    listPermissionRules: list,
    savePermissionRule: save,
    deletePermissionRule: remove,
    groupPlugins: plugins,
  },
}));
vi.mock("../lib/store", () => ({
  useStore: (select: (state: { agents: unknown[]; groups: unknown[] }) => unknown) =>
    select({ agents: [], groups: [] }),
}));

const global: PermissionRule = {
  id: "global",
  version: 2,
  groupId: null,
  effect: "block",
  class: "actOnSite",
  tool: null,
  agents: [],
  conditions: [],
  wrote: "Never press a site",
  createdAt: 1767225600000,
};
const local: PermissionRule = {
  ...global,
  id: "local",
  groupId: "group",
  wrote: "Never change settings",
  class: "changeSettings",
};

beforeEach(() => {
  vi.clearAllMocks();
  list.mockResolvedValue([global, local]);
  save.mockResolvedValue(local);
  remove.mockResolvedValue(undefined);
  plugins.mockResolvedValue([{ kind: "gmail", name: "Gmail", tools: [{ name: "send_message" }] }]);
});

it("shows inherited rules read-only and sends versioned edits and deletes", async () => {
  render(<Permissions groupId="group" />);
  const inherited = (await screen.findByText("Never press a site")).closest(
    ".permissions__row",
  ) as HTMLElement;
  expect(within(inherited).queryByRole("button")).toBeNull();
  const own = screen.getByText("Never change settings").closest(".permissions__row") as HTMLElement;
  fireEvent.click(within(own).getByRole("button", { name: "Edit" }));
  fireEvent.click(screen.getByRole("button", { name: "Save rule" }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({ groupId: "group", class: "changeSettings" }),
      "local",
      2,
    ),
  );
  vi.spyOn(window, "confirm").mockReturnValue(true);
  fireEvent.click(await screen.findByRole("button", { name: "Remove" }));
  await waitFor(() => expect(remove).toHaveBeenCalledWith("local", "group", 2));
});

it("offers outside-domain and greater-than conditions with typed values", async () => {
  list.mockResolvedValue([]);
  render(<Permissions groupId={null} />);
  const buttons = await screen.findAllByRole("button", { name: "Add rule" });
  fireEvent.click(buttons[1]!);
  fireEvent.click(screen.getByRole("button", { name: "Add condition" }));
  fireEvent.change(screen.getByLabelText("Comparison"), { target: { value: "outsideDomain" } });
  fireEvent.change(screen.getByLabelText("Value"), { target: { value: "acme.com" } });
  fireEvent.click(screen.getByRole("button", { name: "Save rule" }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({
        conditions: [
          { field: "recipients", predicate: { op: "outsideDomain", value: "acme.com" } },
        ],
      }),
      null,
      null,
    ),
  );
  fireEvent.click(screen.getAllByRole("button", { name: "Add rule" })[1]!);
  fireEvent.click(screen.getByRole("button", { name: "Add condition" }));
  fireEvent.change(screen.getByLabelText("Fact"), { target: { value: "amount" } });
  fireEvent.change(screen.getByLabelText("Comparison"), { target: { value: "greaterThan" } });
  fireEvent.change(screen.getByLabelText("Value"), { target: { value: "50" } });
  fireEvent.change(screen.getByLabelText("Currency"), { target: { value: "USD" } });
  fireEvent.click(screen.getByRole("button", { name: "Save rule" }));
  await waitFor(() =>
    expect(save).toHaveBeenLastCalledWith(
      expect.objectContaining({
        conditions: [
          { field: "amount", predicate: { op: "greaterThan", value: "50", currency: "USD" } },
        ],
      }),
      null,
      null,
    ),
  );
});

it("uses the connector-prefixed tool identifier", async () => {
  list.mockResolvedValue([]);
  render(<Permissions groupId="group" />);
  fireEvent.click((await screen.findAllByRole("button", { name: "Add rule" }))[0]!);
  const choice = await screen.findByRole("option", {
    name: "Gmail · send_message (gmail__send_message)",
  });
  expect(choice).toBeTruthy();
  fireEvent.change(screen.getByLabelText("Connector tool (optional)"), {
    target: { value: "gmail__send_message" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save rule" }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({ tool: "gmail__send_message" }),
      null,
      null,
    ),
  );
});
