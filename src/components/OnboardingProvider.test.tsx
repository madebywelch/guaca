import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { useStore } from "../lib/store";
import type { DeviceCode, Settings, SettingsPatch, SubscriptionStatus } from "../lib/types";
import { OnboardingProvider } from "./OnboardingProvider";

const mocks = vi.hoisted(() => ({
  claudeProgram: vi.fn(),
  subscriptionStatus: vi.fn(),
  beginSubscriptionSignin: vi.fn(),
  completeSubscriptionSignin: vi.fn(),
  subscriptionModels: vi.fn(),
  testConnection: vi.fn(),
  updateSettings: vi.fn(),
  openExternal: vi.fn(),
}));
vi.mock("../lib/ipc", () => ({ api: mocks, openExternal: mocks.openExternal }));

function stored(over: Partial<Settings> = {}): Settings {
  return {
    operatorName: "Robin",
    provider: "compatible",
    baseUrl: "https://openrouter.ai/api/v1",
    defaultModel: "anthropic/claude-sonnet-4.5",
    subscriptionModel: "saved-chatgpt-model",
    reasoningEffort: "high",
    subscriptionModels: ["saved-chatgpt-model"],
    apiKeySet: false,
    apiKeyHint: "",
    e2bKeySet: false,
    e2bKeyHint: "",
    computerIdleMinutes: 15,
    kernelKeySet: false,
    kernelKeyHint: "",
    browserIdleMinutes: 60,
    browserStealth: false,
    requestTimeoutSecs: 120,
    limits: {
      maxHops: 8,
      maxStepsPerRun: 60,
      maxFanoutPerCall: 8,
      maxSendsPerPair: 6,
      maxToolRounds: 24,
    },
    ...over,
  };
}

const account: SubscriptionStatus = {
  signedIn: true,
  email: "robin@example.com",
  plan: "pro",
  includesCodex: true,
};
const code: DeviceCode = {
  userCode: "ABCD-EFGH",
  verificationUrl: "https://auth.openai.com/codex/device",
  deviceAuthId: "device-1",
  intervalSecs: 2,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  vi.restoreAllMocks();
  vi.resetAllMocks();
  useStore.setState({
    settings: stored(),
    capabilities: { claudeProvider: true, loopbackEndpoints: true, localFiles: true },
  });
  mocks.claudeProgram.mockResolvedValue({
    installed: true,
    version: "2.1.247 (Claude Code)",
    install: "npm install -g @anthropic-ai/claude-code",
  });
  mocks.subscriptionStatus.mockResolvedValue({
    signedIn: false,
    email: "",
    plan: "",
    includesCodex: false,
  });
  mocks.beginSubscriptionSignin.mockResolvedValue(code);
  mocks.completeSubscriptionSignin.mockResolvedValue(account);
  mocks.subscriptionModels.mockResolvedValue([]);
  mocks.testConnection.mockResolvedValue("Connected");
  mocks.updateSettings.mockImplementation(async (patch: SettingsPatch) => {
    const { apiKey, ...rest } = patch;
    return {
      ...useStore.getState().settings,
      ...rest,
      apiKeySet: apiKey ? true : useStore.getState().settings?.apiKeySet,
    };
  });
  mocks.openExternal.mockResolvedValue(undefined);
});

async function draw() {
  const onConnected = vi.fn();
  const onBack = vi.fn();
  const view = render(<OnboardingProvider onConnected={onConnected} onBack={onBack} />);
  await act(async () => {});
  return { ...view, onConnected, onBack };
}

function key(value = "test-provider-key") {
  fireEvent.change(screen.getByLabelText(/^API key/), { target: { value } });
}

function submit() {
  fireEvent.click(screen.getByRole("button", { name: "Test and continue" }));
}

function chatgpt() {
  fireEvent.click(screen.getByRole("button", { name: /^ChatGPT subscription/ }));
}

it("does not save or advance when verification fails, and can retry", async () => {
  mocks.testConnection.mockRejectedValueOnce(new Error("Invalid key"));
  const { onConnected } = await draw();
  key();
  submit();
  expect((await screen.findByRole("alert")).textContent).toContain("Invalid key");
  expect(mocks.updateSettings).not.toHaveBeenCalled();
  expect(onConnected).not.toHaveBeenCalled();
  key("replacement-key");
  submit();
  await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
  expect(mocks.testConnection).toHaveBeenLastCalledWith(
    expect.objectContaining({ apiKey: "replacement-key" }),
  );
});

it("tests before saving, clears the key, preserves the other provider, and stores no browser secret", async () => {
  const tested = deferred<string>();
  mocks.testConnection.mockReturnValueOnce(tested.promise);
  const persist = vi.spyOn(Storage.prototype, "setItem");
  const { onConnected } = await draw();
  key("  private-test-key  ");
  submit();
  expect(mocks.updateSettings).not.toHaveBeenCalled();
  expect(onConnected).not.toHaveBeenCalled();
  await act(async () => tested.resolve("OK"));
  await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
  const patch = {
    provider: "compatible",
    baseUrl: "https://openrouter.ai/api/v1",
    defaultModel: "anthropic/claude-sonnet-4.5",
    apiKey: "private-test-key",
  };
  expect(mocks.testConnection).toHaveBeenCalledWith(patch);
  expect(mocks.updateSettings).toHaveBeenCalledWith(patch);
  expect(useStore.getState().settings?.subscriptionModel).toBe("saved-chatgpt-model");
  expect((screen.getByLabelText(/^API key/) as HTMLInputElement).value).toBe("");
  expect(persist).not.toHaveBeenCalled();
  persist.mockRestore();
});

it("keeps the key input for a retry when saving fails", async () => {
  mocks.updateSettings.mockRejectedValueOnce(new Error("Could not save"));
  const { onConnected } = await draw();
  key();
  submit();
  expect((await screen.findByRole("alert")).textContent).toContain("Could not save");
  expect(onConnected).not.toHaveBeenCalled();
  expect((screen.getByLabelText(/^API key/) as HTMLInputElement).value).toBe("test-provider-key");
  submit();
  await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
  expect(mocks.testConnection).toHaveBeenCalledTimes(2);
});

it("reuses a saved key only for its own endpoint", async () => {
  useStore.setState({ settings: stored({ apiKeySet: true }) });
  const { onConnected } = await draw();
  submit();
  await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
  expect(mocks.testConnection.mock.calls[0]![0]).not.toHaveProperty("apiKey");
  fireEvent.change(screen.getByLabelText("API provider"), { target: { value: "openai" } });
  expect(
    (screen.getByRole("button", { name: "Test and continue" }) as HTMLButtonElement).disabled,
  ).toBe(true);
  key("openai-test-key");
  submit();
  await waitFor(() => expect(onConnected).toHaveBeenCalledTimes(2));
  expect(mocks.testConnection).toHaveBeenLastCalledWith({
    provider: "compatible",
    baseUrl: "https://api.openai.com/v1",
    defaultModel: "gpt-4.1",
    apiKey: "openai-test-key",
  });
});

it("clears typed keys on endpoint changes and excludes stored keys from custom probes", async () => {
  useStore.setState({ settings: stored({ apiKeySet: true }) });
  await draw();
  key();
  fireEvent.change(screen.getByLabelText("API provider"), { target: { value: "custom" } });
  expect((screen.getByLabelText(/^API key/) as HTMLInputElement).value).toBe("");
  fireEvent.change(screen.getByLabelText("Inference endpoint"), {
    target: { value: "http://localhost:1234/v1" },
  });
  fireEvent.change(screen.getByLabelText("Default model"), { target: { value: "local-model" } });
  submit();
  await waitFor(() => expect(mocks.updateSettings).toHaveBeenCalledOnce());
  expect(mocks.testConnection).toHaveBeenCalledWith({
    provider: "compatible",
    baseUrl: "http://localhost:1234/v1",
    defaultModel: "local-model",
    apiKey: "",
  });
});

it("leaves all endpoint fields untouched when signing in and verifying ChatGPT", async () => {
  const completion = deferred<SubscriptionStatus>();
  mocks.completeSubscriptionSignin.mockReturnValueOnce(completion.promise);
  const { onConnected } = await draw();
  chatgpt();
  fireEvent.click(screen.getByRole("button", { name: "Sign in with ChatGPT" }));
  await screen.findByText("ABCD-EFGH");
  expect(mocks.openExternal).toHaveBeenCalledWith(code.verificationUrl);
  expect(mocks.completeSubscriptionSignin).toHaveBeenCalledWith(code);
  expect(mocks.updateSettings).not.toHaveBeenCalled();
  await act(async () => completion.resolve(account));
  await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
  expect(mocks.testConnection).toHaveBeenCalledWith({
    provider: "chatgpt",
    subscriptionModel: "saved-chatgpt-model",
    reasoningEffort: "high",
  });
  expect(mocks.updateSettings).toHaveBeenCalledWith(mocks.testConnection.mock.calls[0]![0]);
  expect(useStore.getState().settings?.defaultModel).toBe(stored().defaultModel);
  expect(useStore.getState().settings?.baseUrl).toBe(stored().baseUrl);
});

it("gates ineligible plans without testing or saving and allows another sign-in", async () => {
  mocks.completeSubscriptionSignin.mockResolvedValueOnce({
    ...account,
    plan: "free",
    includesCodex: false,
  });
  const { onConnected } = await draw();
  chatgpt();
  fireEvent.click(screen.getByRole("button", { name: "Sign in with ChatGPT" }));
  await screen.findByText(/This plan does not include Codex/);
  expect(mocks.testConnection).not.toHaveBeenCalled();
  expect(mocks.updateSettings).not.toHaveBeenCalled();
  expect(onConnected).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Sign in with ChatGPT" }));
  await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
});

it("tests an existing eligible ChatGPT sign-in without starting a new one", async () => {
  mocks.subscriptionStatus.mockResolvedValue(account);
  const { onConnected } = await draw();
  chatgpt();
  submit();
  await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
  expect(mocks.beginSubscriptionSignin).not.toHaveBeenCalled();
  expect(mocks.testConnection).toHaveBeenCalledOnce();
  expect(mocks.subscriptionModels).not.toHaveBeenCalled();
});

it("loads model choices only in advanced options and tests the chosen subscription model", async () => {
  mocks.subscriptionStatus.mockResolvedValue(account);
  mocks.subscriptionModels.mockResolvedValue([
    {
      slug: "new-model",
      defaultReasoningEffort: "low",
      reasoningEfforts: [{ effort: "low", description: "Faster" }],
    },
  ]);
  const { onConnected } = await draw();
  chatgpt();
  expect(mocks.subscriptionModels).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText("Advanced options"));
  await screen.findByRole("option", { name: "new-model" });
  fireEvent.change(screen.getByRole("combobox", { name: /^Model/ }), {
    target: { value: "new-model" },
  });
  fireEvent.change(screen.getByRole("combobox", { name: /^Reasoning effort/ }), {
    target: { value: "low" },
  });
  submit();
  await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
  expect(mocks.testConnection).toHaveBeenCalledWith({
    provider: "chatgpt",
    subscriptionModel: "new-model",
    reasoningEffort: "low",
  });
});

it("allows retry after the device-code sign-in expires", async () => {
  mocks.completeSubscriptionSignin.mockRejectedValueOnce(new Error("Code expired"));
  const { onConnected } = await draw();
  chatgpt();
  fireEvent.click(screen.getByRole("button", { name: "Sign in with ChatGPT" }));
  expect((await screen.findByRole("alert")).textContent).toContain("Code expired");
  expect(screen.queryByText("ABCD-EFGH")).toBeNull();
  expect(mocks.testConnection).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Sign in with ChatGPT" }));
  await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
});

it("can retry a failed sign-in and retains a manual browser fallback", async () => {
  mocks.beginSubscriptionSignin.mockRejectedValueOnce(new Error("Sign-in unavailable"));
  mocks.openExternal.mockRejectedValue(new Error("Browser unavailable"));
  const completion = deferred<SubscriptionStatus>();
  mocks.completeSubscriptionSignin.mockReturnValueOnce(completion.promise);
  const { onConnected } = await draw();
  chatgpt();
  fireEvent.click(screen.getByRole("button", { name: "Sign in with ChatGPT" }));
  expect((await screen.findByRole("alert")).textContent).toContain("Sign-in unavailable");
  fireEvent.click(screen.getByRole("button", { name: "Sign in with ChatGPT" }));
  await screen.findByText(code.verificationUrl);
  fireEvent.click(screen.getByRole("button", { name: "Open sign-in page" }));
  expect((await screen.findByRole("alert")).textContent).toContain("Copy the address above");
  await act(async () => completion.resolve(account));
  await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
});

it("ignores a canceled sign-in when a later attempt is underway", async () => {
  const old = deferred<SubscriptionStatus>();
  const current = deferred<SubscriptionStatus>();
  mocks.completeSubscriptionSignin
    .mockReturnValueOnce(old.promise)
    .mockReturnValueOnce(current.promise);
  const { onConnected } = await draw();
  chatgpt();
  fireEvent.click(screen.getByRole("button", { name: "Sign in with ChatGPT" }));
  await screen.findByText("ABCD-EFGH");
  fireEvent.click(screen.getByRole("button", { name: "Cancel sign-in" }));
  fireEvent.click(screen.getByRole("button", { name: "Sign in with ChatGPT" }));
  await screen.findByText("ABCD-EFGH");
  await act(async () => old.resolve(account));
  expect(mocks.testConnection).not.toHaveBeenCalled();
  expect(mocks.updateSettings).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "Cancel sign-in" })).toBeTruthy();
  await act(async () => current.resolve(account));
  await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
});

it("does not open a browser or finish a sign-in abandoned before the code arrives", async () => {
  const pending = deferred<DeviceCode>();
  mocks.beginSubscriptionSignin.mockReturnValueOnce(pending.promise);
  const { onBack } = await draw();
  chatgpt();
  fireEvent.click(screen.getByRole("button", { name: "Sign in with ChatGPT" }));
  fireEvent.click(screen.getByRole("button", { name: "Back" }));
  expect(onBack).toHaveBeenCalledOnce();
  await act(async () => pending.resolve(code));
  expect(mocks.openExternal).not.toHaveBeenCalled();
  expect(mocks.completeSubscriptionSignin).not.toHaveBeenCalled();
});

it("does not test or save after unmounting during sign-in", async () => {
  const pending = deferred<SubscriptionStatus>();
  mocks.completeSubscriptionSignin.mockReturnValueOnce(pending.promise);
  const { unmount, onConnected } = await draw();
  chatgpt();
  fireEvent.click(screen.getByRole("button", { name: "Sign in with ChatGPT" }));
  await screen.findByText("ABCD-EFGH");
  unmount();
  await act(async () => pending.resolve(account));
  expect(mocks.testConnection).not.toHaveBeenCalled();
  expect(mocks.updateSettings).not.toHaveBeenCalled();
  expect(onConnected).not.toHaveBeenCalled();
});

it("ignores a browser-open failure after canceling sign-in", async () => {
  const browser = deferred<void>();
  mocks.completeSubscriptionSignin.mockReturnValueOnce(deferred<SubscriptionStatus>().promise);
  mocks.openExternal.mockResolvedValueOnce(undefined).mockReturnValueOnce(browser.promise);
  await draw();
  chatgpt();
  fireEvent.click(screen.getByRole("button", { name: "Sign in with ChatGPT" }));
  await screen.findByText("ABCD-EFGH");
  fireEvent.click(screen.getByRole("button", { name: "Open sign-in page" }));
  fireEvent.click(screen.getByRole("button", { name: "Cancel sign-in" }));
  await act(async () => browser.reject(new Error("Browser unavailable")));
  expect(screen.queryByRole("alert")).toBeNull();
});

it("does not advance or replace store state after unmounting during a save", async () => {
  const saving = deferred<Settings>();
  mocks.updateSettings.mockReturnValueOnce(saving.promise);
  const { unmount, onConnected } = await draw();
  key();
  submit();
  await screen.findByText("Saving your provider…");
  expect((screen.getByRole("button", { name: "Back" }) as HTMLButtonElement).disabled).toBe(true);
  unmount();
  await act(async () => saving.resolve(stored({ apiKeySet: true })));
  expect(onConnected).not.toHaveBeenCalled();
  expect(useStore.getState().settings?.apiKeySet).toBe(false);
});

it("does not save after abandoning a pending connection test", async () => {
  const pending = deferred<string>();
  mocks.testConnection.mockReturnValueOnce(pending.promise);
  const { onBack, onConnected } = await draw();
  key();
  submit();
  fireEvent.click(screen.getByRole("button", { name: "Back" }));
  await act(async () => pending.resolve("Connected"));
  expect(onBack).toHaveBeenCalledOnce();
  expect(mocks.updateSettings).not.toHaveBeenCalled();
  expect(onConnected).not.toHaveBeenCalled();
});

it("says why the primary button cannot be pressed, and stops saying it once it can", async () => {
  await draw();
  const button = screen.getByRole("button", { name: "Test and continue" }) as HTMLButtonElement;
  expect(button.disabled).toBe(true);
  const described = button.getAttribute("aria-describedby");
  expect(described).toBeTruthy();
  expect(document.getElementById(described!)?.textContent).toBe(
    "Paste a key for OpenRouter, then test it.",
  );

  key();
  const ready = screen.getByRole("button", { name: "Test and continue" }) as HTMLButtonElement;
  expect(ready.disabled).toBe(false);
  expect(document.body.textContent).not.toContain("then test it.");
});

it("verifies Claude through the existing program without changing either model", async () => {
  const { onConnected } = await draw();
  // Offered beside the other two rather than behind a disclosure: the plan the
  // operator already pays for is not an advanced case.
  fireEvent.click(screen.getByRole("button", { name: /^Claude subscription/ }));
  expect(screen.getByText(/Claude's own account and model settings apply/)).toBeTruthy();
  submit();
  await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
  expect(mocks.testConnection).toHaveBeenCalledWith({ provider: "claude" });
  expect(mocks.updateSettings).toHaveBeenCalledWith({ provider: "claude" });
});

it("says the host has no claude program instead of hiding the option", async () => {
  mocks.claudeProgram.mockResolvedValue({
    installed: false,
    version: "",
    install: "npm install -g @anthropic-ai/claude-code",
  });
  await draw();
  const card = screen.getByRole("button", { name: /^Claude subscription/ }) as HTMLButtonElement;
  expect(card.disabled).toBe(true);
  expect(card.textContent).toContain("which this host does not have");
  expect(card.textContent).toContain("npm install -g @anthropic-ai/claude-code");
  expect(mocks.testConnection).not.toHaveBeenCalled();
});

it("refuses Claude when the host lacks the capability", async () => {
  useStore.setState({
    capabilities: { claudeProvider: false, loopbackEndpoints: false, localFiles: false },
  });
  await draw();
  const card = screen.getByRole("button", { name: /^Claude subscription/ }) as HTMLButtonElement;
  expect(card.disabled).toBe(true);
  expect(card.textContent).toContain("does not offer the claude program");
  expect(mocks.testConnection).not.toHaveBeenCalled();
});
