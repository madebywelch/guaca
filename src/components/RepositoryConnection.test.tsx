import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { api } from "../lib/ipc";
import { RepositoryConnection } from "./RepositoryConnection";

const connection = {
  remote: "https://github.com/team/code.git",
  pushRemote: "https://github.com/team/code.git",
  acceptsToken: true,
  managedCredential: false,
};
const { read, save, remove, check, app, author } = vi.hoisted(() => ({
  read: vi.fn(),
  save: vi.fn(),
  remove: vi.fn(),
  check: vi.fn(),
  app: vi.fn(),
  author: vi.fn(),
}));
vi.mock("./RepositoryGithubUser", () => ({ RepositoryGithubUser: () => null }));
vi.mock("../lib/ipc", () => ({
  api: {
    savedRepositoryCredentials: vi.fn().mockResolvedValue([]),
    reuseRepositoryCredential: vi.fn(),
    repositoryConnection: read,
    setRepositoryAuthor: author,
    setRepositoryGithub: app,
    setRepositoryCredential: save,
    clearRepositoryCredential: remove,
    checkRepositoryConnection: check,
  },
  openExternal: vi.fn(),
}));

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(api.savedRepositoryCredentials).mockResolvedValue([]);
  read.mockResolvedValue(connection);
  save.mockResolvedValue({ ...connection, managedCredential: true });
  remove.mockResolvedValue(connection);
  check.mockResolvedValue("Read access and push dry run succeeded. No remote refs changed.");
});

it("replaces, checks and removes access without reading the token back", async () => {
  render(<RepositoryConnection id="repo-1" editing />);
  fireEvent.change(await screen.findByLabelText("Git username"), { target: { value: "engineer" } });
  fireEvent.change(screen.getByLabelText("Repository access token"), {
    target: { value: "test-token" },
  });
  fireEvent.click(screen.getByText("Save token"));
  await waitFor(() => expect(save).toHaveBeenCalledWith("repo-1", "engineer", "test-token"));
  await waitFor(() => expect(screen.queryByLabelText("Repository access token")).toBeNull());
  fireEvent.click(await screen.findByText("Check read and push access"));
  expect(await screen.findByText(/No remote refs changed/)).not.toBeNull();
  fireEvent.click(screen.getByText("Remove saved token"));
  await waitFor(() => expect(remove).toHaveBeenCalledWith("repo-1"));
  await waitFor(() => expect(screen.queryByText("Remove saved token")).toBeNull());
});

it("clears a failed token submission and shows the actionable error", async () => {
  save.mockRejectedValue(new Error("Could not save the repository credential"));
  render(<RepositoryConnection id="repo-1" editing />);
  fireEvent.change(await screen.findByLabelText("Repository access token"), {
    target: { value: "test-token" },
  });
  fireEvent.click(screen.getByText("Save token"));
  expect((await screen.findByRole("alert")).textContent).toContain("Could not save");
  expect((screen.getByLabelText("Repository access token") as HTMLInputElement).value).toBe("");
});

it("shows a separate push destination without claiming the origin token covers it", async () => {
  read.mockResolvedValue({ ...connection, pushRemote: "ssh://git@forge.example/team/code.git" });
  render(<RepositoryConnection id="repo-1" editing />);
  expect(await screen.findByText(/token saved here applies to origin only/)).not.toBeNull();
});

it("connects and disconnects App access without asking for or displaying a key", async () => {
  read.mockResolvedValue({ ...connection, githubAvailable: true });
  app.mockResolvedValue({ ...connection, githubAvailable: true, githubApp: true });
  render(<RepositoryConnection id="repo-1" editing />);
  fireEvent.click(await screen.findByText("Connect GitHub App"));
  await waitFor(() => expect(app).toHaveBeenCalledWith("repo-1"));
  expect(await screen.findByText(/short-lived tokens automatically/)).toBeTruthy();
  expect(screen.queryByLabelText("Repository access token")).toBeNull();
  fireEvent.click(screen.getByText("Disconnect GitHub App"));
  await waitFor(() => expect(remove).toHaveBeenCalledWith("repo-1"));
});

it("updates a legacy commit author independently of App access", async () => {
  read.mockResolvedValue({
    ...connection,
    githubApp: true,
    author: { name: "guaca", email: "guaca@localhost" },
  });
  const identity = { name: "Engineer", email: "123+engineer@users.noreply.github.com" };
  author.mockResolvedValue({ ...connection, githubApp: true, author: identity });
  render(<RepositoryConnection id="repo-1" editing />);
  expect(await screen.findByText(/Set your identity before/)).toBeTruthy();
  fireEvent.change(screen.getByLabelText("Commit author name"), {
    target: { value: identity.name },
  });
  fireEvent.change(screen.getByLabelText("Commit author email"), {
    target: { value: identity.email },
  });
  fireEvent.click(screen.getByText("Save commit author"));
  expect(await screen.findByText(/Commit author saved/)).toBeTruthy();
  expect(author).toHaveBeenCalledWith("repo-1", identity);
  expect(screen.getByText("Disconnect GitHub App")).toBeTruthy();
  expect(app).not.toHaveBeenCalled();
  expect(remove).not.toHaveBeenCalled();
});

it("shows a failed author save without reporting success", async () => {
  author.mockRejectedValue(new Error("Could not save this repository's commit identity"));
  render(<RepositoryConnection id="repo-1" editing />);
  fireEvent.change(await screen.findByLabelText("Commit author name"), {
    target: { value: "Engineer" },
  });
  fireEvent.change(screen.getByLabelText("Commit author email"), {
    target: { value: "engineer@example.com" },
  });
  fireEvent.click(screen.getByText("Save commit author"));
  expect((await screen.findByRole("alert")).textContent).toContain("Could not save");
  expect(screen.queryByText(/Commit author saved/)).toBeNull();
});

it("reuses a saved credential by ID without asking for its token", async () => {
  vi.mocked(api.savedRepositoryCredentials).mockResolvedValue([
    { id: "saved-id", remote: connection.remote, username: "engineer" },
  ]);
  vi.mocked(api.reuseRepositoryCredential).mockResolvedValue({
    ...connection,
    managedCredential: true,
  } as Awaited<ReturnType<typeof api.reuseRepositoryCredential>>);
  render(<RepositoryConnection id="repo-2" editing />);
  fireEvent.click(await screen.findByText("Use saved token"));
  await waitFor(() =>
    expect(api.reuseRepositoryCredential).toHaveBeenCalledWith("repo-2", "saved-id"),
  );
  expect(save).not.toHaveBeenCalled();
  expect(screen.queryByLabelText("Repository access token")).toBeNull();
});

it("shows saved identity and access without asking for the token again", async () => {
  read.mockResolvedValue({
    ...connection,
    managedCredential: true,
    author: { name: "Robert", email: "robert@example.com" },
  });
  const { rerender } = render(<RepositoryConnection id="repo-1" />);
  expect(await screen.findByText(/Repository token saved/)).toBeTruthy();
  expect(screen.getByText("Commit author: Robert <robert@example.com>")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Git access" })).toBeNull();
  expect(screen.queryByLabelText("Commit author name")).toBeNull();
  expect(screen.queryByLabelText("Repository access token")).toBeNull();
  expect(api.savedRepositoryCredentials).not.toHaveBeenCalled();
  rerender(<RepositoryConnection id="repo-1" editing />);
  expect((screen.getByLabelText("Commit author name") as HTMLInputElement).value).toBe("Robert");
  expect((screen.getByLabelText("Commit author email") as HTMLInputElement).value).toBe(
    "robert@example.com",
  );
  expect(screen.queryByLabelText("Repository access token")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Change saved token" }));
  fireEvent.change(await screen.findByLabelText("Repository access token"), {
    target: { value: "discard-me" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Cancel token change" }));
  expect(screen.queryByLabelText("Repository access token")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Change saved token" }));
  expect(
    ((await screen.findByLabelText("Repository access token")) as HTMLInputElement).value,
  ).toBe("");
  expect(save).not.toHaveBeenCalled();
  expect(remove).not.toHaveBeenCalled();
});

it("closing Edit discards unsaved identity and token changes", async () => {
  read.mockResolvedValue({
    ...connection,
    managedCredential: true,
    author: { name: "Robert", email: "robert@example.com" },
  });
  const { rerender } = render(<RepositoryConnection id="repo-1" editing />);
  fireEvent.change(await screen.findByLabelText("Commit author name"), {
    target: { value: "unsaved" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Change saved token" }));
  fireEvent.change(await screen.findByLabelText("Repository access token"), {
    target: { value: "discard-me" },
  });
  rerender(<RepositoryConnection id="repo-1" />);
  rerender(<RepositoryConnection id="repo-1" editing />);
  expect((screen.getByLabelText("Commit author name") as HTMLInputElement).value).toBe("Robert");
  expect(screen.queryByLabelText("Repository access token")).toBeNull();
});

it("does not mistake a failed settings read for missing credentials and can retry", async () => {
  read.mockRejectedValueOnce(new Error("Backend unavailable"));
  const { rerender } = render(<RepositoryConnection id="repo-1" editing />);
  expect((await screen.findByRole("alert")).textContent).toContain("Backend unavailable");
  expect(screen.queryByText(/No token saved/)).toBeNull();
  expect(screen.queryByLabelText("Repository access token")).toBeNull();
  read.mockResolvedValue({ ...connection, managedCredential: true });
  fireEvent.click(screen.getByRole("button", { name: "Retry Git settings" }));
  expect(await screen.findByText(/Repository token saved/)).toBeTruthy();
  rerender(<RepositoryConnection id="repo-1" />);
  expect(screen.queryByRole("alert")).toBeNull();
});
