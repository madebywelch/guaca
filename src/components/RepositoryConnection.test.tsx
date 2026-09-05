import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { RepositoryConnection } from "./RepositoryConnection";

const connection = {
  remote: "https://github.com/team/code.git",
  pushRemote: "https://github.com/team/code.git",
  acceptsToken: true,
  managedCredential: false,
};
const { read, save, remove, check } = vi.hoisted(() => ({
  read: vi.fn(),
  save: vi.fn(),
  remove: vi.fn(),
  check: vi.fn(),
}));
vi.mock("../lib/ipc", () => ({
  api: {
    repositoryConnection: read,
    setRepositoryCredential: save,
    clearRepositoryCredential: remove,
    checkRepositoryConnection: check,
  },
  openExternal: vi.fn(),
}));

beforeEach(() => {
  vi.resetAllMocks();
  read.mockResolvedValue(connection);
  save.mockResolvedValue({ ...connection, managedCredential: true });
  remove.mockResolvedValue(connection);
  check.mockResolvedValue("Read access and push dry run succeeded. No remote refs changed.");
});

it("replaces, checks and removes access without reading the token back", async () => {
  render(<RepositoryConnection id="repo-1" />);
  fireEvent.click(screen.getByText("Git access"));
  fireEvent.change(await screen.findByLabelText("Git username"), { target: { value: "engineer" } });
  fireEvent.change(screen.getByLabelText("Repository access token"), {
    target: { value: "test-token" },
  });
  fireEvent.click(screen.getByText("Save token"));
  await waitFor(() => expect(save).toHaveBeenCalledWith("repo-1", "engineer", "test-token"));
  expect((screen.getByLabelText("Repository access token") as HTMLInputElement).value).toBe("");
  fireEvent.click(await screen.findByText("Check read and push access"));
  expect(await screen.findByText(/No remote refs changed/)).not.toBeNull();
  fireEvent.click(screen.getByText("Remove saved token"));
  await waitFor(() => expect(remove).toHaveBeenCalledWith("repo-1"));
  await waitFor(() => expect(screen.queryByText("Remove saved token")).toBeNull());
});

it("clears a failed token submission and shows the actionable error", async () => {
  save.mockRejectedValue(new Error("Could not save the repository credential"));
  render(<RepositoryConnection id="repo-1" />);
  fireEvent.click(screen.getByText("Git access"));
  fireEvent.change(await screen.findByLabelText("Repository access token"), {
    target: { value: "test-token" },
  });
  fireEvent.click(screen.getByText("Save token"));
  expect((await screen.findByRole("alert")).textContent).toContain("Could not save");
  expect((screen.getByLabelText("Repository access token") as HTMLInputElement).value).toBe("");
});

it("shows a separate push destination without claiming the origin token covers it", async () => {
  read.mockResolvedValue({ ...connection, pushRemote: "ssh://git@forge.example/team/code.git" });
  render(<RepositoryConnection id="repo-1" />);
  fireEvent.click(screen.getByText("Git access"));
  expect(await screen.findByText(/token saved here applies to origin only/)).not.toBeNull();
});
