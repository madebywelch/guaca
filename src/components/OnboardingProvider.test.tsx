import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { HeldProvider } from "../lib/onboarding";
import { useStore } from "../lib/store";
import type { DeviceCode, Pickup, Settings, SettingsPatch, SubscriptionStatus } from "../lib/types";
import { OnboardingProvider, transitionRequest } from "./OnboardingProvider";

const place = vi.hoisted(() => ({ hostPlace: vi.fn(() => "thisMac") }));
vi.mock("../lib/host", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/host")>()),
  hostPlace: place.hostPlace,
}));

const mocks = vi.hoisted(() => ({
  claudeProgram: vi.fn(),
  claudeSigninStatus: vi.fn(),
  beginClaudeSignin: vi.fn(),
  completeClaudeSignin: vi.fn(),
  cancelClaudeSignin: vi.fn(),
  existingSignins: vi.fn(),
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
    searchKeySet: false,
    searchKeyHint: "",
    searchesWith: "",
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
    capabilities: {
      claudeProvider: true,
      loopbackEndpoints: true,
      localFiles: true,
      borrowedSignins: true,
    },
  });
  place.hostPlace.mockReturnValue("thisMac");
  mocks.existingSignins.mockResolvedValue([]);
  mocks.claudeSigninStatus.mockResolvedValue({ signedIn: false });
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

it.each([
  ["idle", "start", "running"],
  ["running", "start", "running"],
  ["running", "finish", "idle"],
  ["idle", "finish", "idle"],
] as const)("transitions request %s on %s to %s", (kind, event, expected) => {
  expect(transitionRequest({ kind }, event)).toEqual({ kind: expected });
});

it("starts only one connection test for duplicate submissions in the same batch", async () => {
  const pending = deferred<string>();
  mocks.testConnection.mockReturnValue(pending.promise);
  await draw();
  key();
  const button = screen.getByRole("button", { name: "Test and continue" });
  act(() => {
    fireEvent.click(button);
    fireEvent.click(button);
  });
  expect(mocks.testConnection).toHaveBeenCalledOnce();
});

it("does not replace an in-flight connection test with a late pickup offer", async () => {
  const pickups = deferred<Pickup[]>();
  const pending = deferred<string>();
  mocks.existingSignins.mockReturnValue(pickups.promise);
  mocks.testConnection.mockReturnValue(pending.promise);
  useStore.setState({ settings: stored({ apiKeySet: true }) });
  await draw();
  submit();
  await act(async () => pickups.resolve([{ program: "claudeCode", account: "", plan: "" }]));
  expect(screen.queryByRole("button", { name: "Use Claude Code" })).toBeNull();
  expect(screen.getByText("Testing the connection…")).toBeTruthy();
});

it("starts only one test for duplicate pickup submissions", async () => {
  const pending = deferred<string>();
  mocks.existingSignins.mockResolvedValue([{ program: "claudeCode", account: "", plan: "" }]);
  mocks.testConnection.mockReturnValue(pending.promise);
  await draw();
  const button = await screen.findByRole("button", { name: "Use Claude Code" });
  act(() => {
    fireEvent.click(button);
    fireEvent.click(button);
  });
  expect(mocks.testConnection).toHaveBeenCalledOnce();
});

it("does not save a pickup after leaving while its test is pending", async () => {
  const pending = deferred<string>();
  mocks.existingSignins.mockResolvedValue([{ program: "claudeCode", account: "", plan: "" }]);
  mocks.testConnection.mockReturnValue(pending.promise);
  const { onConnected, onBack } = await draw();
  fireEvent.click(await screen.findByRole("button", { name: "Use Claude Code" }));
  fireEvent.click(screen.getByRole("button", { name: "Back" }));
  await act(async () => pending.resolve("Connected"));
  expect(onBack).toHaveBeenCalledOnce();
  expect(mocks.updateSettings).not.toHaveBeenCalled();
  expect(onConnected).not.toHaveBeenCalled();
});

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

it("shows the endpoint and the model beside the key, with nothing to open first", async () => {
  await draw();
  expect((screen.getByLabelText("Inference endpoint") as HTMLInputElement).value).toBe(
    "https://openrouter.ai/api/v1",
  );
  expect((screen.getByLabelText("Default model") as HTMLInputElement).value).toBe(
    "anthropic/claude-sonnet-4.5",
  );
  expect(screen.queryByText("Advanced options")).toBeNull();
});

it("says in plain words that the host could not be reached during a test", async () => {
  mocks.testConnection.mockRejectedValue({
    kind: "unreachable",
    message:
      "could not reach this host at http://127.0.0.1:8787. Its agents keep working either way",
  });
  await draw();
  key();
  submit();
  expect((await screen.findByRole("alert")).textContent).toBe(
    "Guaca could not reach its host, so nothing was tested. Check that the host is running, then try again.",
  );
});

it("says in plain words that the endpoint could not be reached, and gives the test button focus back", async () => {
  mocks.testConnection.mockRejectedValue({
    kind: "endpointUnreachable",
    message:
      "could not reach the inference endpoint at http://localhost:11434/v1/chat/completions: error sending request",
  });
  await draw();
  fireEvent.change(screen.getByRole("combobox", { name: "API provider" }), {
    target: { value: "custom" },
  });
  fireEvent.change(screen.getByRole("textbox", { name: "Inference endpoint" }), {
    target: { value: "http://localhost:11434/v1" },
  });
  fireEvent.change(screen.getByRole("textbox", { name: "Default model" }), {
    target: { value: "llama3" },
  });
  submit();
  expect((await screen.findByRole("alert")).textContent).toBe(
    "Guaca could not reach http://localhost:11434/v1. Check that the server there is running and the address is right, then test again.",
  );
  await waitFor(() =>
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Test and continue" })),
  );
});

it("says a saved key was refused, and forgets it once another way to pay is chosen", async () => {
  useStore.setState({ settings: stored({ apiKeySet: true }) });
  mocks.testConnection.mockRejectedValue({ kind: "keyRejected", message: "rejected" });
  await draw();
  submit();
  expect((await screen.findByRole("alert")).textContent).toBe(
    "OpenRouter did not accept the saved key. Paste a new key, then test again.",
  );
  chatgpt();
  fireEvent.click(screen.getByRole("button", { name: /^API key/ }));
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.getByLabelText(/^API key/).getAttribute("aria-invalid")).toBeNull();
});

it("puts focus on the first sign-in this Mac already has when it is offered", async () => {
  mocks.existingSignins.mockResolvedValue([claudePickup, codexPickup]);
  await draw();
  const first = await screen.findByRole("button", { name: "Use Claude Code" });
  await waitFor(() => expect(document.activeElement).toBe(first));
});

it("puts focus on the chosen way to pay first", async () => {
  await draw();
  expect(document.activeElement).toBe(screen.getByRole("button", { name: /^API key/ }));
});

it("says a rejected key at the key field, in plain words, and clears it as the key is retyped", async () => {
  mocks.testConnection.mockRejectedValue({
    kind: "keyRejected",
    message: "the inference endpoint rejected the API key (HTTP 401): Incorrect API key provided.",
  });
  await draw();
  key("sk-wrong");
  submit();
  const alert = await screen.findByRole("alert");
  const field = screen.getByLabelText(/^API key/);
  expect(alert.textContent).toBe(
    "OpenRouter did not accept that key. Check that you pasted all of it, or create a new key there, then test again.",
  );
  expect(alert.textContent).not.toContain("HTTP");
  expect(field.getAttribute("aria-invalid")).toBe("true");
  expect(field.getAttribute("aria-describedby")).toBe(alert.id);
  expect(alert.closest(".onboarding-actions")).toBeNull();
  expect(mocks.updateSettings).not.toHaveBeenCalled();
  await waitFor(() => expect(document.activeElement).toBe(field));
  key("sk-right");
  expect(screen.queryByRole("alert")).toBeNull();
  expect(field.getAttribute("aria-invalid")).toBeNull();
});

it("says why a test failed in the bar with the buttons, so it is never scrolled under them", async () => {
  mocks.testConnection.mockRejectedValue(new Error("the endpoint said no"));
  await draw();
  key();
  submit();
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toContain("the endpoint said no");
  expect(alert.closest(".onboarding-actions")).toBe(
    screen.getByRole("button", { name: "Test and continue" }).closest(".onboarding-actions"),
  );
});

it("suggests no other provider's model for a custom endpoint, and tests one with no key", async () => {
  await draw();
  fireEvent.change(screen.getByLabelText("API provider"), { target: { value: "custom" } });
  const model = screen.getByLabelText("Default model") as HTMLInputElement;
  expect(model.value).toBe("");
  expect(model.placeholder).not.toContain("anthropic/");
  expect(screen.getByText(/may not need a key/)).toBeTruthy();
  fireEvent.change(screen.getByLabelText("Inference endpoint"), {
    target: { value: "http://127.0.0.1:8080/v1" },
  });
  fireEvent.change(model, { target: { value: "local-model" } });
  mocks.testConnection.mockRejectedValueOnce(
    new Error("http://127.0.0.1:8080/v1 asked for an API key, and none is set for it: Missing."),
  );
  submit();
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toContain("http://127.0.0.1:8080/v1");
  expect(alert.textContent).not.toMatch(/OpenRouter|Settings/);
  expect(mocks.testConnection).toHaveBeenCalledWith({
    provider: "compatible",
    baseUrl: "http://127.0.0.1:8080/v1",
    defaultModel: "local-model",
    apiKey: "",
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
  mocks.claudeSigninStatus.mockResolvedValue({ signedIn: true });
  const { onConnected } = await draw();
  fireEvent.click(screen.getByRole("button", { name: /^Claude subscription/ }));
  expect(
    screen.getByText(
      "Claude uses your plan's own model settings. Testing sends one small request.",
    ),
  ).toBeTruthy();
  await waitFor(() =>
    expect(
      (screen.getByRole("button", { name: "Test and continue" }) as HTMLButtonElement).disabled,
    ).toBe(false),
  );
  submit();
  await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
  expect(mocks.testConnection).toHaveBeenCalledWith({ provider: "claude" });
  expect(mocks.updateSettings).toHaveBeenCalledWith({ provider: "claude" });
});

it("offers host Claude sign-in even when borrowed sign-ins are unavailable", async () => {
  useStore.setState({
    capabilities: { ...useStore.getState().capabilities, borrowedSignins: false },
  });
  mocks.beginClaudeSignin.mockResolvedValue({
    id: "host-session",
    url: "https://example.com/claude",
  });
  mocks.cancelClaudeSignin.mockResolvedValue(undefined);
  await draw();
  fireEvent.click(screen.getByRole("button", { name: /^Claude subscription/ }));
  fireEvent.click(await screen.findByRole("button", { name: "Sign in with Claude" }));
  await screen.findByRole("link", { name: "https://example.com/claude" });
  fireEvent.click(screen.getByRole("button", { name: /^ChatGPT subscription/ }));
  expect(mocks.cancelClaudeSignin).toHaveBeenCalledWith("host-session");
});

it("keeps testing disabled until Claude sign-in succeeds, including failures", async () => {
  const pending = deferred<{ signedIn: boolean }>();
  const signingIn = deferred<{ id: string; url: string }>();
  mocks.claudeSigninStatus.mockReturnValue(pending.promise);
  mocks.beginClaudeSignin.mockReturnValueOnce(signingIn.promise);
  await draw();
  fireEvent.click(screen.getByRole("button", { name: /^Claude subscription/ }));
  const test = screen.getByRole("button", { name: "Test and continue" }) as HTMLButtonElement;
  expect(test.disabled).toBe(true);
  await act(async () => pending.resolve({ signedIn: false }));
  fireEvent.click(await screen.findByRole("button", { name: "Sign in with Claude" }));
  expect(test.disabled).toBe(true);
  await act(async () =>
    signingIn.reject({ kind: "claudeSignin", message: "Claude sign-in timed out." }),
  );
  await screen.findByText("Claude sign-in timed out.");
  expect(test.disabled).toBe(true);
  expect(mocks.testConnection).not.toHaveBeenCalled();
  mocks.beginClaudeSignin.mockResolvedValue({ id: "retry", url: "https://example.com/claude" });
  mocks.completeClaudeSignin.mockResolvedValue({ signedIn: true });
  fireEvent.click(screen.getByRole("button", { name: "Try again" }));
  fireEvent.change(await screen.findByLabelText("Code from browser"), {
    target: { value: "code" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Submit code" }));
  await waitFor(() => expect(test.disabled).toBe(false));
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
    capabilities: {
      claudeProvider: false,
      loopbackEndpoints: false,
      localFiles: false,
      borrowedSignins: false,
    },
  });
  await draw();
  const card = screen.getByRole("button", { name: /^Claude subscription/ }) as HTMLButtonElement;
  expect(card.disabled).toBe(true);
  expect(card.textContent).toContain("does not offer the claude program");
  expect(mocks.testConnection).not.toHaveBeenCalled();
});
const claudePickup: Pickup = {
  program: "claudeCode",
  account: "robin@example.com",
  plan: "max",
};
const codexPickup: Pickup = { program: "codex", account: "", plan: "" };

it("offers the Claude Code sign-in this Mac already has, and uses it in one step", async () => {
  mocks.existingSignins.mockResolvedValue([claudePickup]);
  const { onConnected } = await draw();
  expect(await screen.findByText(/robin@example.com/)).toBeTruthy();
  expect(screen.getByText(/Nothing changes until you choose/)).toBeTruthy();

  fireEvent.click(screen.getByRole("button", { name: "Use Claude Code" }));
  await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
  expect(mocks.testConnection).toHaveBeenCalledWith({ provider: "claude" });
  expect(mocks.updateSettings).toHaveBeenCalledWith({ provider: "claude" });
  expect(mocks.beginSubscriptionSignin).not.toHaveBeenCalled();
});

it("reuses the Codex account through Guaca's own sign-in rather than its token", async () => {
  mocks.existingSignins.mockResolvedValue([codexPickup]);
  const { onConnected } = await draw();
  expect(await screen.findByText(/both programs refresh that token/)).toBeTruthy();
  expect(screen.getByText(/signed in to ChatGPT/)).toBeTruthy();

  fireEvent.click(screen.getByRole("button", { name: "Sign in with the same account" }));
  await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
  expect(mocks.beginSubscriptionSignin).toHaveBeenCalledOnce();
  expect(mocks.completeSubscriptionSignin).toHaveBeenCalledWith(code);
  expect(mocks.updateSettings).toHaveBeenCalledWith({ provider: "chatgpt" });
});

it("says why a sign-in offer failed in the bar with the buttons, so it is never scrolled under them", async () => {
  mocks.existingSignins.mockResolvedValue([claudePickup]);
  mocks.testConnection.mockRejectedValue(new Error("claude is not signed in"));
  await draw();
  fireEvent.click(await screen.findByRole("button", { name: "Use Claude Code" }));
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toContain("claude is not signed in");
  expect(alert.closest(".onboarding-actions")).toBe(
    screen.getByRole("button", { name: "Back" }).closest(".onboarding-actions"),
  );
});

it("skips the offer without writing anything and shows the provider choices", async () => {
  mocks.existingSignins.mockResolvedValue([claudePickup, codexPickup]);
  await draw();
  fireEvent.click(await screen.findByRole("button", { name: "Skip and choose a provider" }));
  expect(screen.getByText("Choose how your agents are paid for")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Use Claude Code" })).toBeNull();
  expect(mocks.testConnection).not.toHaveBeenCalled();
  expect(mocks.updateSettings).not.toHaveBeenCalled();
  expect(mocks.beginSubscriptionSignin).not.toHaveBeenCalled();
});

it("does not offer a sign-in in a browser, which cannot see the host's programs", async () => {
  mocks.existingSignins.mockResolvedValue([claudePickup]);
  place.hostPlace.mockReturnValue("browser");
  await draw();
  expect(screen.getByText("Choose how your agents are paid for")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Use Claude Code" })).toBeNull();
  expect(mocks.existingSignins).not.toHaveBeenCalled();
});

it("does not offer a sign-in on a remote host", async () => {
  mocks.existingSignins.mockResolvedValue([claudePickup]);
  place.hostPlace.mockReturnValue("remote");
  await draw();
  expect(screen.queryByRole("button", { name: "Use Claude Code" })).toBeNull();
  expect(mocks.existingSignins).not.toHaveBeenCalled();
});

it("does not offer a sign-in on a box, even one that can run the claude program", async () => {
  useStore.setState({
    capabilities: {
      claudeProvider: true,
      loopbackEndpoints: true,
      localFiles: false,
      borrowedSignins: false,
    },
  });
  mocks.existingSignins.mockResolvedValue([claudePickup]);
  await draw();
  expect(screen.queryByRole("button", { name: "Use Claude Code" })).toBeNull();
  expect(mocks.existingSignins).not.toHaveBeenCalled();
});

it("offers nothing when a program is installed but logged out", async () => {
  mocks.existingSignins.mockResolvedValue([]);
  await draw();
  expect(screen.getByText("Choose how your agents are paid for")).toBeTruthy();
  expect(screen.queryByRole("button", { name: /^Use Claude Code$/ })).toBeNull();
});

it("does not offer anything to an operator who already configured a provider", async () => {
  useStore.setState({ settings: stored({ provider: "chatgpt" }) });
  mocks.existingSignins.mockResolvedValue([claudePickup]);
  await draw();
  expect(screen.queryByRole("button", { name: "Use Claude Code" })).toBeNull();
  expect(mocks.existingSignins).not.toHaveBeenCalled();
});

describe("before the host exists", () => {
  async function early(initial: HeldProvider | null = null, failure?: unknown) {
    const onConnected = vi.fn();
    const onDraft = vi.fn();
    const view = render(
      <OnboardingProvider
        waiting
        initial={initial}
        failure={failure}
        onConnected={onConnected}
        onBack={vi.fn()}
        onDraft={onDraft}
      />,
    );
    await act(async () => {});
    return { ...view, onConnected, onDraft };
  }

  beforeEach(() => {
    useStore.setState({ settings: null });
  });

  it("asks the host nothing, and holds a pasted key in memory until it can be tested there", async () => {
    const setItem = vi.spyOn(Storage.prototype, "setItem");
    const { onConnected, onDraft } = await early();
    for (const call of [
      mocks.claudeProgram,
      mocks.subscriptionStatus,
      mocks.existingSignins,
      mocks.testConnection,
      mocks.updateSettings,
    ])
      expect(call).not.toHaveBeenCalled();
    const next = screen.getByRole("button", { name: "Continue" }) as HTMLButtonElement;
    expect(next.disabled).toBe(true);
    key("sk-early-key");
    expect(screen.getByText(/keeps the key in this window's memory only/)).toBeTruthy();
    fireEvent.click(next);
    expect(onConnected).toHaveBeenCalledOnce();
    expect(onDraft).toHaveBeenLastCalledWith(expect.objectContaining({ apiKey: "sk-early-key" }));
    expect(mocks.testConnection).not.toHaveBeenCalled();
    expect(mocks.updateSettings).not.toHaveBeenCalled();
    for (const [, value] of setItem.mock.calls) expect(value).not.toContain("sk-early-key");
  });

  it("lets a ChatGPT or Claude plan be chosen, and says its sign-in waits for the host", async () => {
    const { onConnected } = await early();
    chatgpt();
    expect(screen.getByText(/ChatGPT sign-in runs on your Guaca host, which is still starting/));
    expect(screen.getByText("Sign-in waits for your host. Everything else can go ahead."));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    fireEvent.click(screen.getByRole("button", { name: /^Claude subscription/ }));
    expect(screen.getByText(/Claude sign-in runs on your Guaca host, which is still starting/));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(onConnected).toHaveBeenCalledTimes(2);
    expect(mocks.beginSubscriptionSignin).not.toHaveBeenCalled();
    expect(mocks.beginClaudeSignin).not.toHaveBeenCalled();
  });
});

it("opens on what was entered before the host was up and says why the held key failed", async () => {
  render(
    <OnboardingProvider
      initial={{
        provider: "compatible",
        baseUrl: "https://openrouter.ai/api/v1",
        model: "anthropic/claude-sonnet-4.5",
        apiKey: "sk-early-key",
        subscriptionModel: "",
        effort: "auto",
      }}
      failure={{ kind: "keyRejected", message: "401" }}
      onConnected={vi.fn()}
      onBack={vi.fn()}
    />,
  );
  await act(async () => {});
  expect((screen.getByLabelText(/^API key/) as HTMLInputElement).value).toBe("sk-early-key");
  expect(screen.getByRole("alert").textContent).toBe(
    "OpenRouter did not accept that key. Check that you pasted all of it, or create a new key there, then test again.",
  );
});
