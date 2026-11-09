import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { GroupArchive } from "../lib/transfer";
import { aGroup } from "../test-fixtures";

const mocks = vi.hoisted(() => ({
  reconnect: vi.fn(),
  exportGroup: vi.fn(),
  importGroup: vi.fn(),
  save: vi.fn(),
  refresh: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  api: {
    groupReconnect: mocks.reconnect,
    exportGroup: mocks.exportGroup,
    importGroup: mocks.importGroup,
  },
}));
vi.mock("../lib/store", () => ({
  useStore: { getState: () => ({ refreshAgents: mocks.refresh }) },
}));
vi.mock("../lib/transport", () => ({ desktop: false }));
vi.mock("../lib/transfer", async (original) => ({
  ...(await original<typeof import("../lib/transfer")>()),
  saveGroup: mocks.save,
}));

const { GroupTransfer } = await import("./GroupTransfer");
const group = aGroup({ name: "Kitchen" });
const archive: GroupArchive = {
  format: "guaca-group",
  version: 1,
  tables: { groups: [{ name: "Kitchen" }], agents: [], messages: [], routines: [] },
  memories: {},
  files: {},
  reconnect: [],
};

function choose(text: string) {
  const file = new File([text], "group.json", { type: "application/json" });
  Object.defineProperty(file, "text", { value: async () => text });
  fireEvent.change(screen.getByLabelText("Import a group file"), { target: { files: [file] } });
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.reconnect.mockResolvedValue([]);
  mocks.refresh.mockResolvedValue(undefined);
});

it("rejects an invalid file and clears the previous preview", async () => {
  render(<GroupTransfer group={group} />);
  choose(JSON.stringify(archive));
  await screen.findByRole("button", { name: "Import as a new group" });
  choose("{}");
  expect((await screen.findByRole("alert")).textContent).toContain(
    "This is not a supported Guaca group file.",
  );
  expect(screen.queryByRole("button", { name: "Import as a new group" })).toBeNull();
});

it("retains the preview after a failed import", async () => {
  mocks.importGroup.mockRejectedValue(new Error("Import failed"));
  render(<GroupTransfer group={group} />);
  choose(JSON.stringify(archive));
  fireEvent.click(await screen.findByRole("button", { name: "Import as a new group" }));
  expect((await screen.findByRole("alert")).textContent).toContain("Import failed");
  expect(screen.getByRole("button", { name: "Import as a new group" })).toBeTruthy();
});

it("ignores a reconnect lookup failure and leaves import available", async () => {
  mocks.reconnect.mockRejectedValue(new Error("Reconnect unavailable"));
  render(<GroupTransfer group={group} />);
  choose(JSON.stringify(archive));
  expect(await screen.findByRole("button", { name: "Import as a new group" })).toBeTruthy();
  expect(screen.queryByRole("alert")).toBeNull();
});

it("trims the imported name and refreshes the agent list", async () => {
  mocks.importGroup.mockResolvedValue({ name: "Copy" });
  render(<GroupTransfer group={group} />);
  choose(JSON.stringify(archive));
  fireEvent.change(await screen.findByLabelText("Imported group name"), {
    target: { value: "  Copy  " },
  });
  fireEvent.click(screen.getByRole("button", { name: "Import as a new group" }));
  expect(await screen.findByRole("status")).toBeTruthy();
  expect(mocks.importGroup).toHaveBeenCalledWith(archive, "Copy");
  expect(mocks.refresh).toHaveBeenCalledOnce();
  expect(screen.queryByLabelText("Imported group name")).toBeNull();
});

it("passes the exported archive to the saver and disables transfer while waiting", async () => {
  let finish!: (value: GroupArchive) => void;
  mocks.exportGroup.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  mocks.save.mockResolvedValue("Group export downloaded.");
  render(<GroupTransfer group={group} />);
  const button = screen.getByRole("button", { name: "Export Kitchen" }) as HTMLButtonElement;
  fireEvent.click(button);
  expect(button.disabled).toBe(true);
  finish(archive);
  await waitFor(() => expect(mocks.save).toHaveBeenCalledWith(archive));
  expect((await screen.findByRole("status")).textContent).toContain("Group export downloaded.");
});

it("does not add the group credential hint to the export or logs", async () => {
  const log = vi.spyOn(console, "log");
  const error = vi.spyOn(console, "error");
  mocks.exportGroup.mockResolvedValue(archive);
  mocks.save.mockResolvedValue("Saved");
  render(
    <GroupTransfer
      group={aGroup({ name: "Kitchen", apiKeySet: true, apiKeyHint: "credential-hint" })}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Export Kitchen" }));
  await screen.findByText("Saved");
  expect(mocks.exportGroup.mock.calls[0]).toHaveLength(1);
  expect(mocks.save).toHaveBeenCalledWith(archive);
  expect(JSON.stringify(mocks.save.mock.calls)).not.toContain("credential-hint");
  expect(log).not.toHaveBeenCalled();
  expect(error).not.toHaveBeenCalled();
  log.mockRestore();
  error.mockRestore();
});
