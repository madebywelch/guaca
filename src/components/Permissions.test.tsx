import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { PermissionRule } from "../lib/types";
import { Permissions } from "./Permissions";

const { list, save, remove, plugins, state } = vi.hoisted(() => ({
  list: vi.fn(),
  save: vi.fn(),
  remove: vi.fn(),
  plugins: vi.fn(),
  state: {
    agents: [] as { id: string; name: string; groupId: string }[],
    groups: [] as { id: string; name: string }[],
  },
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
  useStore: (select: (state: { agents: unknown[]; groups: unknown[] }) => unknown) => select(state),
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
  state.agents = [];
  state.groups = [];
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
    name: "Gmail › Send message",
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

it("makes the verdict, scope, and natural preview explicit", async () => {
  list.mockResolvedValue([]);
  state.groups = [{ id: "group", name: "Sales" }];
  state.agents = [{ id: "agent", name: "Ava", groupId: "group" }];
  render(<Permissions groupId={null} />);
  fireEvent.click((await screen.findAllByRole("button", { name: "Add rule" }))[0]!);
  expect(screen.getByLabelText("Verdict")).toHaveProperty("value", "allow");
  fireEvent.click(screen.getByLabelText("Ava · Sales"));
  await screen.findByRole("option", { name: "Gmail › Send message" });
  fireEvent.change(screen.getByLabelText("Connector tool (optional)"), {
    target: { value: "gmail__send_message" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Add condition" }));
  fireEvent.change(screen.getByLabelText("Comparison"), { target: { value: "under" } });
  fireEvent.change(screen.getByLabelText("Value"), { target: { value: "acme.com" } });
  expect(
    screen.getByText(
      /Allow acting in your name with Gmail › Send message for selected agents when every recipient is at acme.com/,
    ),
  ).toBeTruthy();
  fireEvent.change(screen.getByLabelText("Verdict"), { target: { value: "block" } });
  fireEvent.change(screen.getByLabelText("Fact"), { target: { value: "amount" } });
  fireEvent.change(screen.getByLabelText("Comparison"), { target: { value: "greaterThan" } });
  fireEvent.change(screen.getByLabelText("Value"), { target: { value: "50" } });
  fireEvent.change(screen.getByLabelText("Currency"), { target: { value: "USD" } });
  expect(
    screen.getByText(
      /Never act in your name with Gmail › Send message for selected agents when the amount is over 50 USD/,
    ),
  ).toBeTruthy();
  expect(screen.getByText(/Applies to Ava\./)).toBeTruthy();
});

it("shows an empty agent picker and starts Never from the block list", async () => {
  list.mockResolvedValue([]);
  render(<Permissions groupId="group" />);
  fireEvent.click((await screen.findAllByRole("button", { name: "Add rule" }))[1]!);
  expect(screen.getByLabelText("Verdict")).toHaveProperty("value", "block");
  expect(screen.queryByText("Who")).toBeNull();
  expect(screen.getByText(/Applies to everyone in this group/)).toBeTruthy();
  expect(screen.getByText("How rules match")).toBeTruthy();
});

it("humanizes a disconnected connector in the selector and preview", async () => {
  list.mockResolvedValue([{ ...local, class: "actOnBehalf", tool: "stripe__stripe_refund" }]);
  plugins.mockResolvedValue([]);
  render(<Permissions groupId="group" />);
  fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
  expect(
    screen.getByRole("option", { name: "Stripe › Refund (not currently connected)" }),
  ).toBeTruthy();
  expect(screen.getByText(/Never act in your name with Stripe › Refund/)).toBeTruthy();
});
