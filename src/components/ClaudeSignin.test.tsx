import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { ClaudeSignin } from "./ClaudeSignin";

const api = vi.hoisted(() => ({
  claudeSigninStatus: vi.fn(),
  beginClaudeSignin: vi.fn(),
  completeClaudeSignin: vi.fn(),
  cancelClaudeSignin: vi.fn(),
}));
vi.mock("../lib/ipc", () => ({ api, openExternal: vi.fn() }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

beforeEach(() => {
  vi.resetAllMocks();
  api.claudeSigninStatus.mockResolvedValue({ signedIn: false });
  api.beginClaudeSignin.mockResolvedValue({ id: "session", url: "https://example.com/login" });
  api.cancelClaudeSignin.mockResolvedValue(undefined);
});

it("opens a browser link and submits the returned code", async () => {
  api.completeClaudeSignin.mockResolvedValue({ signedIn: true });
  render(<ClaudeSignin />);
  fireEvent.click(screen.getByRole("button", { name: "Sign in with Claude" }));
  expect(
    (await screen.findByRole("link", { name: "https://example.com/login" })).getAttribute("href"),
  ).toBe("https://example.com/login");
  fireEvent.change(screen.getByLabelText("Code from browser"), {
    target: { value: "secret-code" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Submit code" }));
  await screen.findByText("Signed in to Claude on this host.");
  expect(api.completeClaudeSignin).toHaveBeenCalledWith("session", "secret-code");
});

it("does not expose errors and allows a fresh attempt after failure", async () => {
  api.completeClaudeSignin.mockRejectedValue(new Error("secret-code"));
  const view = render(<ClaudeSignin />);
  fireEvent.click(screen.getByRole("button", { name: "Sign in with Claude" }));
  fireEvent.change(await screen.findByLabelText("Code from browser"), {
    target: { value: "secret-code" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Submit code" }));
  expect((await screen.findByRole("alert")).textContent).not.toContain("secret-code");
  expect(screen.queryByLabelText("Code from browser")).toBeNull();
  expect(screen.getByRole("button", { name: "Sign in with Claude" })).toBeTruthy();
  view.unmount();
  expect(api.cancelClaudeSignin).toHaveBeenCalledWith("session");
});

it("cancels a session returned after cancellation", async () => {
  const pending = deferred<{ id: string; url: string }>();
  api.beginClaudeSignin.mockReturnValue(pending.promise);
  render(<ClaudeSignin />);
  fireEvent.click(screen.getByRole("button", { name: "Sign in with Claude" }));
  fireEvent.click(screen.getByRole("button", { name: "Cancel sign-in" }));
  await act(async () => pending.resolve({ id: "late", url: "https://example.com" }));
  await waitFor(() => expect(api.cancelClaudeSignin).toHaveBeenCalledWith("late"));
  expect(screen.queryByRole("link")).toBeNull();
});

it("ignores a completion returned after cancellation", async () => {
  const pending = deferred<{ signedIn: boolean }>();
  api.completeClaudeSignin.mockReturnValue(pending.promise);
  render(<ClaudeSignin />);
  fireEvent.click(screen.getByRole("button", { name: "Sign in with Claude" }));
  fireEvent.change(await screen.findByLabelText("Code from browser"), {
    target: { value: "code" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Submit code" }));
  fireEvent.click(screen.getByRole("button", { name: "Cancel sign-in" }));
  await act(async () => pending.resolve({ signedIn: true }));
  expect(api.cancelClaudeSignin).toHaveBeenCalledWith("session");
  expect(screen.queryByText("Signed in to Claude on this host.")).toBeNull();
});
