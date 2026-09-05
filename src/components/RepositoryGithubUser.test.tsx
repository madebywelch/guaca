import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { RepositoryGithubUser } from "./RepositoryGithubUser";

const { status, begin, poll, signOut, open } = vi.hoisted(() => ({
  status: vi.fn(),
  begin: vi.fn(),
  poll: vi.fn(),
  signOut: vi.fn(),
  open: vi.fn(),
}));
vi.mock("../lib/ipc", () => ({
  api: {
    repositoryGithubUser: status,
    beginRepositoryGithubSignin: begin,
    pollRepositoryGithubSignin: poll,
    signOutRepositoryGithubUser: signOut,
  },
  openExternal: open,
}));
const flow = {
  flowId: "opaque",
  userCode: "USER-CODE",
  verificationUri: "https://github.com/login/device",
  interval: 5,
  expiresIn: 900,
};
const author = { name: "human", email: "7+human@users.noreply.github.com" };
beforeEach(() => {
  vi.resetAllMocks();
  vi.useFakeTimers();
  status.mockResolvedValue({ status: "signedOut" });
  begin.mockResolvedValue(flow);
  signOut.mockResolvedValue({ status: "signedOut" });
});
afterEach(() => vi.useRealTimers());

it("waits for GitHub's poll interval then applies the authorized human identity", async () => {
  const updated = vi.fn();
  await act(async () => {
    render(<RepositoryGithubUser id="repo" onAuthorized={updated} />);
  });
  await act(async () => {
    fireEvent.click(screen.getByText("Sign in to GitHub"));
  });
  expect(screen.getByText("USER-CODE")).toBeTruthy();
  expect(poll).not.toHaveBeenCalled();
  await act(async () => {
    fireEvent.click(screen.getByText("Open GitHub"));
  });
  expect(open).toHaveBeenCalledWith("https://github.com/login/device");
  poll.mockResolvedValueOnce({ status: "pending", interval: 10 });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(5000);
  });
  expect(poll).toHaveBeenCalledTimes(1);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(5000);
  });
  expect(poll).toHaveBeenCalledTimes(1);
  poll.mockResolvedValueOnce({ status: "authorized", login: "human", author });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(5000);
  });
  expect(updated).toHaveBeenCalledWith(author);
  expect(screen.getByText(/Pull requests are opened as human/)).toBeTruthy();
  await act(async () => {
    fireEvent.click(screen.getByText("Sign out of GitHub"));
  });
  expect(signOut).toHaveBeenCalledWith("repo");
  expect(screen.getByText("Sign in to GitHub")).toBeTruthy();
});

it("shows authorization failures without changing commit identity", async () => {
  const updated = vi.fn();
  poll.mockRejectedValue(new Error("GitHub authorization expired"));
  await act(async () => {
    render(<RepositoryGithubUser id="repo" onAuthorized={updated} />);
  });
  await act(async () => {
    fireEvent.click(screen.getByText("Sign in to GitHub"));
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(5000);
  });
  expect(screen.getByRole("alert").textContent).toContain("expired");
  expect(updated).not.toHaveBeenCalled();
  expect(screen.getByText("Sign in to GitHub")).toBeTruthy();
});

it("stops polling when the operator cancels", async () => {
  await act(async () => {
    render(<RepositoryGithubUser id="repo" onAuthorized={vi.fn()} />);
  });
  await act(async () => {
    fireEvent.click(screen.getByText("Sign in to GitHub"));
  });
  fireEvent.click(screen.getByText("Cancel sign-in"));
  await act(async () => {
    await vi.advanceTimersByTimeAsync(30000);
  });
  expect(poll).not.toHaveBeenCalled();
});
