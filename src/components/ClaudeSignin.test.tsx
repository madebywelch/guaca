import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { ClaudeSignin, signinRequest } from "./ClaudeSignin";

const api = vi.hoisted(() => ({
  claudeSigninStatus: vi.fn(),
  beginClaudeSignin: vi.fn(),
  completeClaudeSignin: vi.fn(),
  cancelClaudeSignin: vi.fn(),
}));
const openExternal = vi.hoisted(() => vi.fn());
vi.mock("../lib/ipc", () => ({ api, openExternal }));

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

it("transitions request ownership only along valid edges", () => {
  const states: Parameters<typeof signinRequest>[0][] = [
    { kind: "idle" },
    { kind: "starting" },
    { kind: "session", id: "one" },
    { kind: "submitting", id: "one" },
  ];
  const events: Parameters<typeof signinRequest>[1][] = [
    { type: "start" },
    { type: "opened", id: "one" },
    { type: "submit" },
    { type: "reset" },
  ];
  const expected = [
    [states[1], states[0], states[0], states[0]],
    [states[1], states[2], states[1], states[0]],
    [states[2], states[2], states[3], states[0]],
    [states[3], states[3], states[3], states[0]],
  ];
  for (const [i, state] of states.entries()) {
    for (const [j, event] of events.entries()) {
      expect(signinRequest(state, event)).toEqual(expected[i]?.[j]);
      if (expected[i]?.[j] === state) expect(signinRequest(state, event)).toBe(state);
    }
  }
});

it("opens a browser link and submits the returned code", async () => {
  api.completeClaudeSignin.mockResolvedValue({ signedIn: true });
  render(<ClaudeSignin />);
  fireEvent.click(await screen.findByRole("button", { name: "Sign in with Claude" }));
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
  fireEvent.click(await screen.findByRole("button", { name: "Sign in with Claude" }));
  fireEvent.change(await screen.findByLabelText("Code from browser"), {
    target: { value: "secret-code" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Submit code" }));
  expect((await screen.findByRole("alert")).textContent).not.toContain("secret-code");
  expect(screen.queryByLabelText("Code from browser")).toBeNull();
  expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  view.unmount();
  expect(api.cancelClaudeSignin).toHaveBeenCalledWith("session");
});

it("cancels a session returned after cancellation", async () => {
  const pending = deferred<{ id: string; url: string }>();
  api.beginClaudeSignin.mockReturnValue(pending.promise);
  render(<ClaudeSignin />);
  fireEvent.click(await screen.findByRole("button", { name: "Sign in with Claude" }));
  fireEvent.click(screen.getByRole("button", { name: "Cancel sign-in" }));
  await act(async () => pending.resolve({ id: "late", url: "https://example.com" }));
  await waitFor(() => expect(api.cancelClaudeSignin).toHaveBeenCalledWith("late"));
  expect(screen.queryByRole("link")).toBeNull();
});

it("ignores a completion returned after cancellation", async () => {
  const pending = deferred<{ signedIn: boolean }>();
  api.completeClaudeSignin.mockReturnValue(pending.promise);
  render(<ClaudeSignin />);
  fireEvent.click(await screen.findByRole("button", { name: "Sign in with Claude" }));
  fireEvent.change(await screen.findByLabelText("Code from browser"), {
    target: { value: "code" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Submit code" }));
  fireEvent.click(screen.getByRole("button", { name: "Cancel sign-in" }));
  await act(async () => pending.resolve({ signedIn: true }));
  expect(api.cancelClaudeSignin).toHaveBeenCalledWith("session");
  expect(screen.queryByText("Signed in to Claude on this host.")).toBeNull();
});

it("cancels a session returned after unmount", async () => {
  const pending = deferred<{ id: string; url: string }>();
  api.beginClaudeSignin.mockReturnValue(pending.promise);
  const view = render(<ClaudeSignin />);
  fireEvent.click(await screen.findByRole("button", { name: "Sign in with Claude" }));
  view.unmount();
  await act(async () => pending.resolve({ id: "late", url: "https://example.com" }));
  expect(api.cancelClaudeSignin).toHaveBeenCalledWith("late");
});

it("submits a code only once before a render", async () => {
  api.completeClaudeSignin.mockReturnValue(deferred<{ signedIn: boolean }>().promise);
  render(<ClaudeSignin />);
  fireEvent.click(await screen.findByRole("button", { name: "Sign in with Claude" }));
  fireEvent.change(await screen.findByLabelText("Code from browser"), {
    target: { value: "code" },
  });
  const submit = screen.getByRole("button", { name: "Submit code" });
  act(() => {
    fireEvent.click(submit);
    fireEvent.click(submit);
  });
  expect(api.completeClaudeSignin).toHaveBeenCalledTimes(1);
});

it("starts sign-in only once before a render", async () => {
  api.beginClaudeSignin.mockReturnValue(deferred<{ id: string; url: string }>().promise);
  render(<ClaudeSignin />);
  const start = await screen.findByRole("button", { name: "Sign in with Claude" });
  await waitFor(() => expect(start.hasAttribute("disabled")).toBe(false));
  act(() => {
    fireEvent.click(start);
    fireEvent.click(start);
  });
  expect(api.beginClaudeSignin).toHaveBeenCalledTimes(1);
});

it("allows a fresh start after cancelling a pending start", async () => {
  const pending = deferred<{ id: string; url: string }>();
  api.beginClaudeSignin.mockReturnValueOnce(pending.promise);
  render(<ClaudeSignin />);
  fireEvent.click(await screen.findByRole("button", { name: "Sign in with Claude" }));
  fireEvent.click(screen.getByRole("button", { name: "Cancel sign-in" }));
  fireEvent.click(screen.getByRole("button", { name: "Sign in with Claude" }));
  await screen.findByRole("link", { name: "https://example.com/login" });
  await act(async () => pending.resolve({ id: "late", url: "https://example.com/late" }));
  expect(api.cancelClaudeSignin).toHaveBeenCalledWith("late");
  expect(screen.getByRole("link", { name: "https://example.com/login" })).toBeTruthy();
});

it("keeps the address and code input when the browser fails to open", async () => {
  openExternal.mockRejectedValue(new Error("private error"));
  render(<ClaudeSignin />);
  fireEvent.click(await screen.findByRole("button", { name: "Sign in with Claude" }));
  fireEvent.click(await screen.findByRole("link", { name: "https://example.com/login" }));
  expect((await screen.findByRole("alert")).textContent).toBe(
    "The browser didn't open. Copy the address above to continue.",
  );
  expect(screen.getByLabelText("Code from browser")).toBeTruthy();
  expect(api.cancelClaudeSignin).not.toHaveBeenCalled();
});

it.each([
  [{ kind: "claudeSignin", message: "Claude sign-in timed out." }, "Claude sign-in timed out."],
  [{ kind: "claudeSignin", message: "secret-code" }, "Claude sign-in didn't finish."],
  [{ kind: "other", message: "Claude sign-in timed out." }, "Claude sign-in didn't finish."],
  [new Error("secret-code"), "Claude sign-in didn't finish."],
])("only displays allowlisted backend errors", async (failure, expected) => {
  api.beginClaudeSignin.mockRejectedValue(failure);
  render(<ClaudeSignin />);
  fireEvent.click(await screen.findByRole("button", { name: "Sign in with Claude" }));
  expect((await screen.findByRole("alert")).textContent).toBe(expected);
  expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
});
