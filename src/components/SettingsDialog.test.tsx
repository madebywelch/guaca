import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Prefs } from "../lib/prefs";
import type { Section } from "../lib/sections";
import type {
  AccountConnectors,
  AccountStatus,
  ClaudeProgram,
  DeliverySettings,
  DeviceCode,
  GuardLimits,
  NotificationAccess,
  PushReport,
  Settings,
  SettingsPatch,
  SubscriptionStatus,
} from "../lib/types";

const HELD: GuardLimits = {
  maxHops: 8,
  maxStepsPerRun: 60,
  maxFanoutPerCall: 8,
  maxSendsPerPair: 6,
  maxToolRounds: 24,
};

function stored(over: Partial<Settings> = {}): Settings {
  return {
    operatorName: "Robert",
    e2bKeySet: false,
    e2bKeyHint: "",
    computerIdleMinutes: 15,
    kernelKeySet: false,
    kernelKeyHint: "",
    browserIdleMinutes: 60,
    browserStealth: false,
    baseUrl: "https://openrouter.ai/api/v1",
    defaultModel: "anthropic/claude-sonnet-4.5",
    provider: "compatible",
    subscriptionModel: "gpt-5.6-luna",
    reasoningEffort: "auto",
    subscriptionModels: ["gpt-5.6-luna", "gpt-5.4-mini"],
    apiKeySet: true,
    apiKeyHint: "…9f2c",
    requestTimeoutSecs: 120,
    limits: { ...HELD },
    ...over,
  };
}

function signedOut(): SubscriptionStatus {
  return { signedIn: false, email: "", plan: "", includesCodex: false };
}

function signedIn(over: Partial<SubscriptionStatus> = {}): SubscriptionStatus {
  return {
    signedIn: true,
    email: "robert@example.com",
    plan: "pro",
    includesCodex: true,
    ...over,
  };
}

const updateSettings = vi.fn<(patch: SettingsPatch) => Promise<Settings>>(async () => stored());
const getSettings = vi.fn<() => Promise<Settings>>(async () => stored());
const testConnection = vi.fn<(patch?: SettingsPatch) => Promise<string>>(async () => "Reached it.");
const notifyOperator = vi.fn<(title: string, body: string) => Promise<boolean>>(async () => true);
const notificationAccess = vi.fn<() => Promise<NotificationAccess | null>>(async () => null);
const openNotificationSettings = vi.fn<() => Promise<void>>(async () => {});
const pushReport = vi.fn<() => Promise<PushReport>>(async () => nothingSent());
const testPush = vi.fn<() => Promise<string>>(async () => "Sent, and the address took it.");

function nothingSent(): PushReport {
  return { lastSentAt: null, lastAttemptAt: null, lastError: null, retryAt: null };
}
const subscriptionStatus = vi.fn<() => Promise<SubscriptionStatus>>(async () => signedOut());
const beginSubscriptionSignin = vi.fn<() => Promise<DeviceCode>>(async () => ({
  verificationUrl: "https://auth.openai.com/codex/device",
  userCode: "ABCD-EFGH",
  deviceAuthId: "dev-1",
  intervalSecs: 2,
}));
const completeSubscriptionSignin = vi.fn<(code: DeviceCode) => Promise<SubscriptionStatus>>(
  async () => signedIn(),
);
const signOutSubscription = vi.fn<() => Promise<Settings>>(async () =>
  stored({ provider: "compatible" }),
);
const openExternal = vi.fn<(url: string) => Promise<void>>(async () => {});

function noAccount(over: Partial<AccountStatus> = {}): AccountStatus {
  return { signedIn: false, email: "", origin: "https://guaca.bot", ...over };
}

function linked(over: Partial<AccountStatus> = {}): AccountStatus {
  return {
    signedIn: true,
    email: "robert@example.com",
    origin: "https://guaca.bot",
    ...over,
  };
}

function held(granted = true): AccountConnectors {
  return {
    email: "robert@example.com",
    connections: [
      { id: "acct_1", provider: "google", label: "robert@example.com", capabilities: ["gmail"] },
    ],
    providers: [
      {
        id: "google",
        label: "Google",
        capabilities: [
          { id: "gmail", label: "Gmail", granted },
          { id: "drive", label: "Drive", granted: false },
        ],
      },
    ],
  };
}

const accountStatus = vi.fn<() => Promise<AccountStatus>>(async () => noAccount());
const signInAccount = vi.fn<() => Promise<AccountStatus>>(async () => linked());
const accountConnectors = vi.fn<() => Promise<AccountConnectors>>(async () => held());
const signOutAccount = vi.fn<() => Promise<AccountStatus>>(async () => noAccount());
const claudeProgram = vi.fn<() => Promise<ClaudeProgram>>(async () => ({
  installed: true,
  version: "2.1.247 (Claude Code)",
  install: "npm install -g @anthropic-ai/claude-code",
}));
const claudeSigninStatus = vi.fn(async () => ({ signedIn: false }));
const beginClaudeSignin = vi.fn(async () => ({ id: "session", url: "https://example.com" }));

const platform = vi.hoisted(() => ({ desktop: true }));
vi.mock("../lib/transport", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/transport")>()),
  get desktop() {
    return platform.desktop;
  },
}));
vi.mock("../lib/ipc", () => ({
  api: {
    updateSettings: (patch: SettingsPatch) => updateSettings(patch),
    getSettings: () => getSettings(),
    testConnection: (patch?: SettingsPatch) => testConnection(patch),
    subscriptionStatus: () => subscriptionStatus(),
    subscriptionModels: async () =>
      ["gpt-5.6-luna", "gpt-5.4-mini"].map((slug) => ({
        slug,
        defaultReasoningEffort: "medium",
        reasoningEfforts: [
          { effort: "low", description: "Faster" },
          { effort: "high", description: "More thorough" },
        ],
      })),
    beginSubscriptionSignin: () => beginSubscriptionSignin(),
    completeSubscriptionSignin: (code: DeviceCode) => completeSubscriptionSignin(code),
    signOutSubscription: () => signOutSubscription(),
    accountStatus: () => accountStatus(),
    signInAccount: () => signInAccount(),
    accountConnectors: () => accountConnectors(),
    signOutAccount: () => signOutAccount(),
    claudeProgram: () => claudeProgram(),
    claudeSigninStatus: () => claudeSigninStatus(),
    beginClaudeSignin: () => beginClaudeSignin(),
    cancelClaudeSignin: async () => {},
    pushReport: () => pushReport(),
    testPush: () => testPush(),
    notificationAccess: () => notificationAccess(),
    openNotificationSettings: () => openNotificationSettings(),
  },
  notifyOperator: (title: string, body: string) => notifyOperator(title, body),
  openExternal: (url: string) => openExternal(url),
}));

const { SettingsDialog, pushLine } = await import("./SettingsDialog");
const { useStore } = await import("../lib/store");
const transport = await import("../lib/transport");
const { DEFAULT_PREFS, loadPrefs } = await import("../lib/prefs");
const { BINDINGS, SURFACES } = await import("../lib/keybinds");
const { PROVIDERS } = await import("../lib/providers");
const { themeCss, themeTokens } = await import("../lib/theme");

const onClose = vi.fn();

function open(settings: Settings | null = stored(), prefs: Prefs = DEFAULT_PREFS, on?: Section) {
  useStore.setState({ settings, prefs });
  return render(<SettingsDialog onClose={onClose} section={on} />);
}

function pane(label: string): void {
  fireEvent.click(screen.getByRole("tab", { name: label }));
}

function field(label: RegExp): HTMLInputElement {
  return screen.getByLabelText(label) as HTMLInputElement;
}

function type(label: RegExp, value: string): void {
  fireEvent.change(field(label), { target: { value } });
}

function check(label: RegExp): void {
  fireEvent.click(field(label));
}

function save(): HTMLButtonElement {
  return screen.getByRole("button", { name: "Save" }) as HTMLButtonElement;
}

function probe(): HTMLButtonElement {
  return screen.getByRole("button", { name: "Test connection" }) as HTMLButtonElement;
}

function banner(): HTMLElement {
  const found = document.querySelector(".banner");
  if (!found) throw new Error("no banner on screen");
  return found as HTMLElement;
}

function sentPatch(): SettingsPatch {
  const call = updateSettings.mock.calls[0];
  if (!call) throw new Error("nothing was sent");
  return call[0];
}

function switchOn(label: string): HTMLButtonElement {
  const row = screen.getByText(label).closest(".switch-row");
  const control = row?.querySelector("button");
  if (!control) throw new Error(`no switch on the row for ${label}`);
  return control as HTMLButtonElement;
}

beforeEach(() => {
  vi.clearAllMocks();
  claudeSigninStatus.mockResolvedValue({ signedIn: false });
  claudeProgram.mockResolvedValue({
    installed: true,
    version: "2.1.247 (Claude Code)",
    install: "npm install -g @anthropic-ai/claude-code",
  });
  platform.desktop = true;
  localStorage.clear();
  updateSettings.mockResolvedValue(stored());
  getSettings.mockResolvedValue(stored());
  pushReport.mockResolvedValue(nothingSent());
  testPush.mockResolvedValue("Sent, and the address took it.");
  testConnection.mockResolvedValue("Reached OpenRouter. 312 models.");
  notifyOperator.mockResolvedValue(true);
  notificationAccess.mockResolvedValue(null);
  subscriptionStatus.mockResolvedValue(signedOut());
  completeSubscriptionSignin.mockResolvedValue(signedIn());
  signOutSubscription.mockResolvedValue(stored({ provider: "compatible" }));
  accountStatus.mockResolvedValue(noAccount());
  signInAccount.mockResolvedValue(linked());
  accountConnectors.mockResolvedValue(held());
  signOutAccount.mockResolvedValue(noAccount());
});

describe("when the runtime refuses", () => {
  it("says what went wrong and stays open, so nothing typed is lost", async () => {
    updateSettings.mockRejectedValue({ kind: "config", message: "config.json is read-only" });
    open();
    type(/^Your name/, "Robert W");
    fireEvent.click(save());

    await waitFor(() => expect(screen.getByText(/config\.json is read-only/i)).toBeTruthy());
    expect(banner().className).toContain("banner--error");
    expect(onClose).not.toHaveBeenCalled();
    expect(field(/^Your name/).value).toBe("Robert W");
    expect(save().disabled).toBe(false);
  });

  it("reports a refused endpoint without touching what is stored", async () => {
    testConnection.mockRejectedValue({ kind: "inference", message: "connection refused" });
    open(stored(), DEFAULT_PREFS, "provider");
    fireEvent.click(probe());

    await waitFor(() => expect(screen.getByText(/connection refused/i)).toBeTruthy());
    expect(banner().className).toContain("banner--error");
    expect(updateSettings).not.toHaveBeenCalled();
  });

  it("takes one press while a save is in flight, whatever the operator does", async () => {
    let settle: (value: Settings) => void = () => {};
    updateSettings.mockImplementation(
      () =>
        new Promise<Settings>((resolve) => {
          settle = resolve;
        }),
    );
    open(stored(), DEFAULT_PREFS, "provider");
    type(/^Inference endpoint/, "http://localhost:1234/v1");
    fireEvent.click(save());

    expect(save().disabled).toBe(true);
    expect(probe().disabled).toBe(true);
    fireEvent.click(save());
    expect(updateSettings).toHaveBeenCalledTimes(1);

    settle(stored());
    await waitFor(() => expect(save().disabled).toBe(false));
  });
});

describe("concurrent settings edits", () => {
  it("retries an older host's rejected partial limits over its latest settings", async () => {
    updateSettings
      .mockRejectedValueOnce({
        kind: "badArguments",
        message:
          "`update_settings` was called with arguments this build does not recognize (missing field `maxStepsPerRun`). Update the host.",
      })
      .mockResolvedValueOnce(stored({ limits: { ...HELD, maxHops: 12, maxStepsPerRun: 80 } }));
    getSettings.mockResolvedValue(stored({ limits: { ...HELD, maxStepsPerRun: 80 } }));
    open();
    pane("Limits");
    type(/^Relay depth/, "12");
    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(2));
    expect(getSettings).toHaveBeenCalledTimes(1);
    expect(updateSettings.mock.calls[1]?.[0]).toEqual({
      limits: { ...HELD, maxHops: 12, maxStepsPerRun: 80 },
    });
  });

  it("does not retry an unrelated host refusal", async () => {
    updateSettings.mockRejectedValue({ kind: "config", message: "config.json is read-only" });
    open();
    pane("Limits");
    type(/^Relay depth/, "12");
    fireEvent.click(save());
    await waitFor(() => expect(screen.getByText(/config\.json is read-only/i)).toBeTruthy());
    expect(updateSettings).toHaveBeenCalledTimes(1);
    expect(getSettings).not.toHaveBeenCalled();
  });

  it("keeps distinct limit edits from two clients with stale snapshots", async () => {
    let runtime = stored();
    updateSettings.mockImplementation(async (patch) => {
      runtime = { ...runtime, limits: { ...runtime.limits, ...patch.limits } };
      return runtime;
    });
    const first = open(stored());
    pane("Limits");
    type(/^Relay depth/, "12");
    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    first.unmount();

    open(stored());
    pane("Limits");
    type(/^Model calls per conversation/, "80");
    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(2));
    expect(runtime.limits).toEqual({ ...HELD, maxHops: 12, maxStepsPerRun: 80 });
  });

  it("sends only the limit edited by this client", async () => {
    open();
    pane("Limits");
    type(/^Relay depth/, "12");
    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch().limits).toEqual({ maxHops: 12 });
  });

  it("preserves a limit edited again while its save is delayed", async () => {
    let finish!: (value: Settings) => void;
    updateSettings.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    open();
    pane("Limits");
    type(/^Relay depth/, "12");
    fireEvent.click(save());
    type(/^Relay depth/, "14");
    await act(async () => finish(stored({ limits: { ...HELD, maxHops: 12 } })));
    expect(field(/^Relay depth/).value).toBe("14");
    expect(save().disabled).toBe(false);
  });

  it("preserves a revert made while the previous value is saving", async () => {
    let finish!: (value: Settings) => void;
    updateSettings.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    open();
    type(/^Your name/, "Bob");
    fireEvent.click(save());
    type(/^Your name/, "Robert");
    act(() =>
      useStore.getState().applyEvent({
        type: "settingsChanged",
        settings: stored({ operatorName: "Bob" }),
      }),
    );
    expect(field(/^Your name/).value).toBe("Robert");
    await act(async () => finish(stored({ operatorName: "Bob" })));
    expect(field(/^Your name/).value).toBe("Robert");
    expect(save().disabled).toBe(false);
  });

  it("restores a closed draft while following untouched remote fields", () => {
    const view = open();
    type(/^Your name/, "Draft name");
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    view.unmount();
    open(stored({ defaultModel: "remote/model" }));
    expect(field(/^Your name/).value).toBe("Draft name");
    pane("Provider");
    expect(field(/^Default model/).value).toBe("remote/model");
  });

  it("discards all staged edits and does not restore them on reopen", () => {
    const view = open();
    expect(screen.queryByRole("button", { name: "Discard changes" })).toBeNull();
    type(/^Your name/, "Draft name");
    pane("Limits");
    type(/^Relay depth/, "12");
    pane("Machines");
    type(/^E2B API key/, "discard-me");
    type(/^Sleep computers after/, "45");
    fireEvent.click(screen.getByRole("button", { name: "Discard changes" }));
    expect(screen.queryByRole("button", { name: "Discard changes" })).toBeNull();
    expect(field(/^E2B API key/).value).toBe("");
    expect(field(/^Sleep computers after/).value).toBe("");
    pane("Limits");
    expect(field(/^Relay depth/).value).toBe(String(HELD.maxHops));
    pane("General");
    expect(field(/^Your name/).value).toBe("Robert");
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    view.unmount();
    open();
    expect(field(/^Your name/).value).toBe("Robert");
  });

  it("does not retain keys or key editors in a closed draft", () => {
    const view = open();
    type(/^Your name/, "Draft name");
    pane("Provider");
    fireEvent.click(screen.getByRole("button", { name: "Change API key" }));
    type(/^API key/, "provider-secret");
    pane("Machines");
    type(/^E2B API key/, "computer-secret");
    type(/^Kernel API key/, "browser-secret");
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    view.unmount();
    open();
    expect(field(/^Your name/).value).toBe("Draft name");
    pane("Provider");
    fireEvent.click(screen.getByRole("button", { name: "Change API key" }));
    expect(field(/^API key/).value).toBe("");
    pane("Machines");
    expect(field(/^E2B API key/).value).toBe("");
    expect(field(/^Kernel API key/).value).toBe("");
  });
});

describe("what a save sends", () => {
  it("leaves a blank key, timer and timeout out of the patch entirely", async () => {
    open();
    type(/^Your name/, "Robert W");
    fireEvent.click(save());

    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    const patch = sentPatch();
    expect("apiKey" in patch).toBe(false);
    expect("e2bApiKey" in patch).toBe(false);
    expect("computerIdleMinutes" in patch).toBe(false);
    expect("kernelApiKey" in patch).toBe(false);
    expect("browserIdleMinutes" in patch).toBe(false);
    expect("requestTimeoutSecs" in patch).toBe(false);
    expect(patch).toEqual({ operatorName: "Robert W" });
  });

  it("does not read a box of spaces as a key", async () => {
    open();
    pane("Provider");
    fireEvent.click(screen.getByRole("button", { name: "Change API key" }));
    type(/^API key/, "   ");
    pane("Machines");
    type(/^E2B API key/, " ");
    type(/^Kernel API key/, "   ");
    pane("General");
    type(/^Your name/, "Robert W");
    fireEvent.click(save());

    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect("apiKey" in sentPatch()).toBe(false);
    expect("e2bApiKey" in sentPatch()).toBe(false);
    expect("kernelApiKey" in sentPatch()).toBe(false);
  });

  it("refuses anything but digits in a duration, so no patch can carry NaN", async () => {
    open();
    pane("Machines");
    type(/^Sleep computers after/, "45 minutes");
    expect(field(/^Sleep computers after/).value).toBe("45");

    pane("Provider");
    type(/^Give up on a call after/, "abc");
    expect(field(/^Give up on a call after/).value).toBe("");

    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch().computerIdleMinutes).toBe(45);
    expect("requestTimeoutSecs" in sentPatch()).toBe(false);
  });

  it("sends a cleared limit as that limit's floor rather than as zero", async () => {
    open();
    pane("Limits");
    type(/^Relay depth/, "");
    expect(field(/^Relay depth/).value).toBe("1");

    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch().limits).toEqual({ maxHops: 1 });
  });

  it("sends a limit the runtime will clamp exactly as it was typed", async () => {
    open();
    pane("Limits");
    type(/^Relay depth/, "40");

    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch().limits?.maxHops).toBe(40);
  });

  it("trims a typed key and carries the rest of the panes with it", async () => {
    open();
    pane("Provider");
    fireEvent.click(screen.getByRole("button", { name: "Change API key" }));
    type(/^API key/, "  sk-or-v1-typed  ");
    type(/^Inference endpoint/, "http://localhost:1234/v1");
    type(/^Default model/, "qwen3-coder-30b");
    type(/^Give up on a call after/, "30");
    pane("Machines");
    type(/^E2B API key/, " e2b_typed ");
    type(/^Sleep computers after/, "45");
    type(/^Kernel API key/, "  sk_typed  ");
    type(/^Close browsers after/, "5");
    check(/^Hide that browsers are automated/);
    pane("General");
    type(/^Your name/, "Robert");

    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch()).toEqual({
      baseUrl: "http://localhost:1234/v1",
      defaultModel: "qwen3-coder-30b",
      apiKey: "sk-or-v1-typed",
      e2bApiKey: "e2b_typed",
      computerIdleMinutes: 45,
      kernelApiKey: "sk_typed",
      browserIdleMinutes: 5,
      browserStealth: true,
      requestTimeoutSecs: 30,
    });
  });

  it("follows a change made elsewhere, and never saves the stale value back", async () => {
    open();
    pane("Provider");
    type(/^Inference endpoint/, "http://localhost:1234/v1");
    act(() =>
      useStore.getState().applyEvent({
        type: "settingsChanged",
        settings: {
          ...stored(),
          baseUrl: "https://elsewhere.example/v1",
          defaultModel: "someone/else",
        },
      }),
    );
    expect((field(/^Default model/) as HTMLInputElement).value).toBe("someone/else");
    expect((field(/^Inference endpoint/) as HTMLInputElement).value).toBe(
      "http://localhost:1234/v1",
    );
    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch()).toEqual({ baseUrl: "http://localhost:1234/v1" });
  });

  it("carries the browser half of Machines on its own, with no computer touched", async () => {
    open();
    pane("Machines");
    type(/^Kernel API key/, "sk_typed");
    type(/^Close browsers after/, "5");

    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch().kernelApiKey).toBe("sk_typed");
    expect(sentPatch().browserIdleMinutes).toBe(5);
    expect("e2bApiKey" in sentPatch()).toBe(false);
    expect("computerIdleMinutes" in sentPatch()).toBe(false);
  });

  it("sends stealth turned back off, which omission would make unreachable", async () => {
    open(stored({ browserStealth: true }));
    pane("Machines");
    expect(field(/^Hide that browsers are automated/).checked).toBe(true);
    check(/^Hide that browsers are automated/);

    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch().browserStealth).toBe(false);
  });

  it("refuses anything but digits in the browser timer too", async () => {
    open();
    pane("Machines");
    type(/^Close browsers after/, "5 minutes");
    expect(field(/^Close browsers after/).value).toBe("5");

    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch().browserIdleMinutes).toBe(5);
  });

  it("puts every staged box back to blank, saying what was actually stored", async () => {
    open();
    pane("Provider");
    fireEvent.click(screen.getByRole("button", { name: "Change API key" }));
    type(/^API key/, "sk-or-v1-typed");
    type(/^Give up on a call after/, "30");
    pane("Machines");
    type(/^E2B API key/, "e2b_typed");
    type(/^Sleep computers after/, "2000");

    updateSettings.mockResolvedValue(
      stored({ apiKeySet: true, apiKeyHint: "…yped", computerIdleMinutes: 1440 }),
    );
    fireEvent.click(save());
    await waitFor(() => expect(screen.getByText("Saved.")).toBeTruthy());

    expect(field(/^E2B API key/).value).toBe("");
    expect(field(/^Sleep computers after/).value).toBe("");
    expect(field(/^Sleep computers after/).placeholder).toBe("1440 minutes");
    pane("Provider");
    expect(screen.queryByLabelText(/^API key/)).toBeNull();
    expect(screen.getByText("…yped")).toBeTruthy();
    expect(field(/^Give up on a call after/).value).toBe("");
  });

  it("stays open on a save, because saving is not finishing", async () => {
    open();
    type(/^Your name/, "Robert W");
    fireEvent.click(save());
    await waitFor(() => expect(screen.getByText("Saved.")).toBeTruthy());
    expect(banner().className).toContain("banner--ok");
    expect(onClose).not.toHaveBeenCalled();
    expect(useStore.getState().settings?.operatorName).toBe("Robert");
  });
});

describe("the Save in the foot", () => {
  const saveButton = () => screen.queryByRole("button", { name: "Save" });

  it("is not there until something is waiting for it", () => {
    open();
    expect(saveButton()).toBeNull();
    type(/^Your name/, "Robert W");
    expect(saveButton()).toBeTruthy();
  });

  it("offers nothing on the panes that write as they are clicked", async () => {
    const attention = { needsYou: true, failures: true, finished: true };
    updateSettings.mockResolvedValueOnce(stored({ attention: { ...attention, failures: false } }));
    open(stored({ attention }), DEFAULT_PREFS, "appearance");
    fireEvent.click(screen.getByRole("button", { name: "Reading surface: Dark" }));
    expect(saveButton()).toBeNull();

    pane("Notifications");
    await act(async () => fireEvent.click(switchOn("Failures")));
    expect(switchOn("Failures").textContent).toBe("Off");
    expect(saveButton()).toBeNull();
  });

  it("offers nothing on the panes there is nothing to save on", () => {
    open(stored(), DEFAULT_PREFS, "shortcuts");
    expect(saveButton()).toBeNull();
    pane("About");
    expect(saveButton()).toBeNull();
  });

  it("is still reachable from a pane that stages nothing", () => {
    open();
    pane("Provider");
    type(/^Inference endpoint/, "http://localhost:1234/v1");
    pane("Shortcuts");
    expect(saveButton()).toBeTruthy();
  });

  it("goes once what was typed has been saved", async () => {
    open();
    type(/^Your name/, "Robert W");
    updateSettings.mockResolvedValue(stored({ operatorName: "Robert W" }));

    fireEvent.click(save());
    await waitFor(() => expect(screen.getByText("Saved.")).toBeTruthy());
    expect(saveButton()).toBeNull();
  });

  it("stays after a refusal, because the edit is still unsaved", async () => {
    updateSettings.mockRejectedValue({ kind: "config", message: "config.json is read-only" });
    open();
    type(/^Your name/, "Robert W");

    fireEvent.click(save());
    await waitFor(() => expect(screen.getByText(/config\.json is read-only/i)).toBeTruthy());
    expect(saveButton()).toBeTruthy();
  });

  it("does not count a duration retyped as what is already stored", () => {
    open();
    pane("Machines");
    type(/^Sleep computers after/, "15");
    expect(saveButton()).toBeNull();
    type(/^Sleep computers after/, "45");
    expect(saveButton()).toBeTruthy();
  });
});

describe("the menu bar", () => {
  it("says the panel exists, on a host with a menu bar to look at", () => {
    open();
    const said = screen.getByText(/Guaca keeps an icon there/);
    const row = said.closest(".field");
    expect(row?.querySelector(".field__label")?.textContent).toBe("Menu bar");
  });

  it("is not mentioned in a browser, which has no menu bar", () => {
    platform.desktop = false;
    open();
    expect(screen.queryByText(/menu bar/)).toBeNull();
  });
});

describe("moving between sections", () => {
  it("opens Compost inside settings and preserves pending edits", () => {
    useStore.setState({ agents: [] });
    open();
    type(/^Your name/, "New name");
    pane("Compost");
    expect(screen.getByText("No deleted agents.")).toBeTruthy();
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(onClose).not.toHaveBeenCalled();
    pane("General");
    expect(field(/^Your name/).value).toBe("New name");
  });

  it("keeps typing that has not been saved", () => {
    open();
    pane("Provider");
    type(/^Inference endpoint/, "http://localhost:1234/v1");
    pane("Machines");
    type(/^Sleep computers after/, "45");
    pane("Limits");
    type(/^Recipients per send/, "3");

    pane("Provider");
    expect(field(/^Inference endpoint/).value).toBe("http://localhost:1234/v1");
    pane("Machines");
    expect(field(/^Sleep computers after/).value).toBe("45");
    pane("Limits");
    expect(field(/^Recipients per send/).value).toBe("3");
  });

  it("marks exactly one section as the one being read", () => {
    open();
    const chosen = () =>
      screen
        .getAllByRole("tab")
        .filter((tab) => tab.getAttribute("aria-selected") === "true")
        .map((tab) => tab.textContent);

    expect(chosen()).toEqual(["General"]);
    pane("Notifications");
    expect(chosen()).toEqual(["Notifications"]);
  });

  it("is one Tab stop, moved through with the arrow keys", () => {
    open();
    const tabs = screen.getAllByRole("tab");
    expect(tabs.filter((tab) => tab.tabIndex === 0).map((tab) => tab.textContent)).toEqual([
      "General",
    ]);

    fireEvent.keyDown(tabs[0]!, { key: "ArrowDown" });
    const second = screen.getAllByRole("tab")[1]!;
    expect(second.getAttribute("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(second);
    expect(screen.getAllByRole("tab").filter((tab) => tab.tabIndex === 0)).toEqual([second]);

    fireEvent.keyDown(second, { key: "End" });
    const last = screen.getAllByRole("tab").at(-1)!;
    expect(document.activeElement).toBe(last);
    expect(last.getAttribute("aria-selected")).toBe("true");

    fireEvent.keyDown(last, { key: "ArrowDown" });
    expect(document.activeElement).toBe(screen.getAllByRole("tab")[0]);

    fireEvent.keyDown(document.activeElement!, { key: "ArrowUp" });
    expect(document.activeElement).toBe(last);

    fireEvent.keyDown(last, { key: "Home" });
    expect(document.activeElement).toBe(screen.getAllByRole("tab")[0]);
    expect(screen.getByRole("tabpanel", { name: "General" })).toBeTruthy();
  });

  it("suggests the model of the endpoint chosen, and none for a custom one", () => {
    open(stored(), DEFAULT_PREFS, "provider");
    const model = field(/^Default model/);
    expect(model.placeholder).toBe("anthropic/claude-sonnet-4.5");
    fireEvent.change(field(/^Inference endpoint/), { target: { value: "http://127.0.0.1:9/v1" } });
    expect(field(/^Default model/).placeholder).not.toContain("anthropic/");
  });

  it("opens on the section it was pointed at", () => {
    open(stored(), DEFAULT_PREFS, "shortcuts");
    expect(screen.getByRole("tab", { name: "Shortcuts" }).getAttribute("aria-selected")).toBe(
      "true",
    );
    expect(screen.getByText("Every key the app answers to.", { exact: false })).toBeTruthy();
  });
});

describe("closing", () => {
  it("answers Escape while the focus is on a section tab", () => {
    open();
    const tab = screen.getByRole("tab", { name: "Limits" });
    tab.focus();
    expect(document.activeElement).toBe(tab);

    fireEvent.keyDown(tab, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("dismisses on the scrim behind it", () => {
    open();
    fireEvent.click(screen.getByRole("button", { name: "Close dialog" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe("testing the endpoint", () => {
  it("sends what is on screen, without the name or the limits", async () => {
    open(stored(), DEFAULT_PREFS, "provider");
    type(/^Inference endpoint/, "https://api.groq.com/openai/v1");
    type(/^API key/, "gsk_typed");
    pane("General");
    type(/^Your name/, "Somebody else");
    pane("Provider");

    fireEvent.click(probe());
    await waitFor(() => expect(testConnection).toHaveBeenCalledTimes(1));

    const patch = testConnection.mock.calls[0]?.[0];
    expect(patch).toEqual({
      provider: "compatible",
      baseUrl: "https://api.groq.com/openai/v1",
      defaultModel: "anthropic/claude-sonnet-4.5",
      subscriptionModel: "gpt-5.6-luna",
      reasoningEffort: "auto",
      apiKey: "gsk_typed",
      browserStealth: false,
    });
    expect(updateSettings).not.toHaveBeenCalled();
  });

  it("says it is working, and then what came back", async () => {
    let settle: (value: string) => void = () => {};
    testConnection.mockImplementation(
      () =>
        new Promise<string>((resolve) => {
          settle = resolve;
        }),
    );
    open(stored(), DEFAULT_PREFS, "provider");
    fireEvent.click(probe());

    expect(screen.getByText("Testing…")).toBeTruthy();
    expect(banner().className).toBe("banner");

    settle("Reached OpenRouter. 312 models.");
    await waitFor(() => expect(screen.getByText(/312 models/)).toBeTruthy());
    expect(banner().className).toContain("banner--ok");
  });
});

describe("the provider presets", () => {
  function preset(name: string): HTMLButtonElement {
    const label = screen.getByText(name, { selector: ".preset__name" });
    const button = label.closest("button");
    if (!button) throw new Error(`no preset tile for ${name}`);
    return button as HTMLButtonElement;
  }

  it("offers every endpoint that is known to speak the protocol", () => {
    open();
    pane("Provider");
    for (const provider of PROVIDERS) {
      expect(preset(provider.name), provider.id).toBeTruthy();
    }
  });

  it("marks the preset the endpoint currently is, and only that one", () => {
    open();
    pane("Provider");
    expect(preset("OpenRouter").getAttribute("aria-current")).toBe("true");
    expect(preset("OpenAI").getAttribute("aria-current")).toBe("false");

    type(/^Inference endpoint/, "https://gateway.example/v1");
    expect(preset("OpenRouter").getAttribute("aria-current")).toBe("false");
  });

  it("fills both fields under it, and moves the mark", () => {
    open();
    pane("Provider");
    fireEvent.click(preset("Groq"));

    expect(field(/^Inference endpoint/).value).toBe("https://api.groq.com/openai/v1");
    expect(field(/^Default model/).value).toBe("llama-3.3-70b-versatile");
    expect(preset("Groq").getAttribute("aria-current")).toBe("true");
    expect(preset("OpenRouter").getAttribute("aria-current")).toBe("false");
    expect(updateSettings).not.toHaveBeenCalled();
  });

  it("leaves the model alone for a preset that has no opinion about one", () => {
    open();
    pane("Provider");
    type(/^Default model/, "qwen3-coder-30b");
    fireEvent.click(preset("LM Studio"));

    expect(field(/^Inference endpoint/).value).toBe("http://localhost:1234/v1");
    expect(field(/^Default model/).value).toBe("qwen3-coder-30b");
  });

  it("does not ask a machine on this desk for a key", () => {
    open(stored({ apiKeySet: false, apiKeyHint: "" }));
    pane("Provider");
    expect(preset("LM Studio").textContent).toContain("On the backend");
    expect(preset("OpenRouter").textContent).toContain("Needs a key");
  });

  it("withholds the rows a server cannot offer, and says why on each", () => {
    useStore.setState({
      capabilities: {
        loopbackEndpoints: false,
        borrowedSignins: false,
        claudeProvider: false,
        localFiles: false,
      },
    });
    try {
      open();
      pane("Provider");

      expect(preset("LM Studio").textContent).toContain("Not from a server");
      expect((preset("LM Studio") as HTMLButtonElement).disabled).toBe(true);
      expect(preset("Ollama").textContent).toContain("Not from a server");
      expect((preset("OpenRouter") as HTMLButtonElement).disabled).toBe(false);

      expect(screen.getByText("Not on a server")).toBeTruthy();
      expect(screen.queryByRole("button", { name: "Use the Claude subscription" })).toBeNull();
    } finally {
      useStore.setState({
        capabilities: {
          loopbackEndpoints: true,
          borrowedSignins: true,
          claudeProvider: true,
          localFiles: true,
        },
      });
    }
  });

  it("says a key is stored once one is", () => {
    open(stored({ apiKeySet: true, apiKeyHint: "…9f2c" }));
    pane("Provider");
    expect(preset("OpenRouter").textContent).toContain("Key stored");
    expect(screen.queryByLabelText(/^API key/)).toBeNull();
    expect(screen.getByText("API key saved")).toBeTruthy();
    expect(screen.getByText("…9f2c")).toBeTruthy();
  });

  it("says nothing about a key on a row that is not the one chosen", () => {
    open(stored({ apiKeySet: true, apiKeyHint: "…9f2c" }));
    pane("Provider");

    for (const name of ["OpenAI", "Groq", "Together", "Fireworks"]) {
      expect(preset(name).textContent, name).not.toContain("Key stored");
      expect(preset(name).textContent, name).not.toContain("Needs a key");
    }
    expect(preset("Ollama").textContent).toContain("On the backend");
  });
});

describe("the Host pane", () => {
  beforeEach(() => {
    window.localStorage.removeItem("guaca.workspace.remote");
    vi.restoreAllMocks();
  });

  it("stores the box only once it has answered, and then reloads", async () => {
    const probed = vi
      .spyOn(transport, "probe")
      .mockResolvedValue({ build: "abc1234", capabilities: {} });
    const restarted = vi.spyOn(transport, "restart").mockImplementation(() => {});
    open();
    pane("Host");

    fireEvent.click(screen.getByRole("button", { name: "Remote host" }));
    const button = screen.getByRole("button", { name: "Connect to host" }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    fireEvent.change(field(/^Host address/), { target: { value: "https://box.example:8787/" } });
    fireEvent.change(field(/^Access key/), { target: { value: " t0k3n " } });
    expect(button.disabled).toBe(false);
    expect(window.localStorage.getItem("guaca.workspace.remote")).toBeNull();

    fireEvent.click(button);
    await waitFor(() => expect(restarted).toHaveBeenCalled());
    expect(probed).toHaveBeenCalledWith({ origin: "https://box.example:8787", token: "t0k3n" });
    expect(JSON.parse(window.localStorage.getItem("guaca.workspace.remote")!)).toEqual({
      origin: "https://box.example:8787",
      token: "t0k3n",
    });
  });

  it("stores nothing when the box refuses, and says why", async () => {
    vi.spyOn(transport, "probe").mockRejectedValue({
      kind: "unauthorized",
      message: "this workspace needs the token it printed",
    });
    const restarted = vi.spyOn(transport, "restart").mockImplementation(() => {});
    open();
    pane("Host");
    fireEvent.click(screen.getByRole("button", { name: "Remote host" }));
    fireEvent.change(field(/^Host address/), { target: { value: "https://box.example" } });
    fireEvent.change(field(/^Access key/), { target: { value: "wrong" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect to host" }));

    await screen.findByText(/needs the token it printed/);
    expect(window.localStorage.getItem("guaca.workspace.remote")).toBeNull();
    expect(restarted).not.toHaveBeenCalled();
  });
});

describe("provider-specific configuration", () => {
  it.each(["chatgpt", "claude"] as const)(
    "hides API configuration while %s is selected and preserves edits across switches",
    async (provider) => {
      subscriptionStatus.mockResolvedValue(signedIn());
      claudeSigninStatus.mockResolvedValue({ signedIn: true });
      open(stored({ provider }), DEFAULT_PREFS, "provider");
      const subscriptionButton =
        provider === "chatgpt" ? "Use the ChatGPT subscription" : "Use the Claude subscription";

      expect(screen.queryByLabelText(/^Inference endpoint/)).toBeNull();
      expect(screen.queryByLabelText(/^API key/)).toBeNull();
      expect(screen.queryByLabelText(/^Default model/)).toBeNull();
      expect(screen.queryByText("OpenRouter", { selector: ".preset__name" })).toBeNull();
      expect(field(/^Give up on a call after/)).toBeTruthy();
      expect(probe()).toBeTruthy();

      fireEvent.click(screen.getByRole("button", { name: "Use an API provider" }));
      expect(field(/^Inference endpoint/).value).toBe("https://openrouter.ai/api/v1");
      expect(field(/^Default model/).value).toBe("anthropic/claude-sonnet-4.5");
      expect(screen.getByText("API key saved")).toBeTruthy();
      type(/^Inference endpoint/, "https://gateway.example/v1");
      type(/^Default model/, "custom-model");
      type(/^API key/, "replacement-key");

      await waitFor(() => {
        expect(
          (screen.getByRole("button", { name: subscriptionButton }) as HTMLButtonElement).disabled,
        ).toBe(false);
      });
      fireEvent.click(screen.getByRole("button", { name: subscriptionButton }));
      expect(screen.queryByLabelText(/^Inference endpoint/)).toBeNull();
      fireEvent.click(screen.getByRole("button", { name: "Use an API provider" }));
      expect(field(/^Inference endpoint/).value).toBe("https://gateway.example/v1");
      expect(field(/^Default model/).value).toBe("custom-model");
      expect(field(/^API key/).value).toBe("replacement-key");
      fireEvent.click(save());
      await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
      expect(sentPatch()).toMatchObject({
        provider: "compatible",
        baseUrl: "https://gateway.example/v1",
        defaultModel: "custom-model",
        apiKey: "replacement-key",
      });
    },
  );
});

describe("the ChatGPT subscription", () => {
  function row(): HTMLElement {
    const label = screen.getByText("ChatGPT subscription", { selector: ".preset__name" });
    const found = label.closest(".preset");
    if (!found) throw new Error("no subscription row");
    return found as HTMLElement;
  }

  function button(name: string): HTMLButtonElement {
    return screen.getByRole("button", { name }) as HTMLButtonElement;
  }

  function parked(): { finish: (status: SubscriptionStatus) => void } {
    let release: (status: SubscriptionStatus) => void = () => {};
    completeSubscriptionSignin.mockReturnValue(
      new Promise<SubscriptionStatus>((resolve) => {
        release = resolve;
      }),
    );
    return { finish: release };
  }

  it("offers a sign-in when nobody is signed in", async () => {
    open();
    pane("Provider");
    await waitFor(() => expect(subscriptionStatus).toHaveBeenCalled());
    expect(button("Sign in")).toBeTruthy();
    expect(row().querySelector(".preset__url")?.textContent).toBe("Your plan pays for turns");
    expect(row().title).toBe("Sign in and your plan pays for turns, with no per-token bill.");
  });

  it("does not ask the runtime about a sign-in until the pane is opened", async () => {
    open();
    expect(subscriptionStatus).not.toHaveBeenCalled();
    pane("Provider");
    await waitFor(() => expect(subscriptionStatus).toHaveBeenCalledTimes(1));
  });

  it("draws the code and opens a browser at it", async () => {
    const signin = parked();
    open();
    pane("Provider");
    fireEvent.click(button("Sign in"));

    await waitFor(() => expect(screen.getByText("ABCD-EFGH")).toBeTruthy());
    expect(screen.getByText("https://auth.openai.com/codex/device")).toBeTruthy();
    expect(openExternal).toHaveBeenCalledWith("https://auth.openai.com/codex/device");

    signin.finish(signedIn());
    await waitFor(() => expect(screen.queryByText("ABCD-EFGH")).toBeNull());
  });

  it("keeps a code on screen across a change of section", async () => {
    parked();
    open();
    pane("Provider");
    fireEvent.click(button("Sign in"));
    await waitFor(() => expect(screen.getByText("ABCD-EFGH")).toBeTruthy());

    pane("Limits");
    pane("Provider");
    expect(screen.getByText("ABCD-EFGH")).toBeTruthy();
  });

  it("switches the provider to the subscription once it is signed in", async () => {
    open();
    pane("Provider");
    fireEvent.click(button("Sign in"));

    await waitFor(() => expect(screen.getByText(/Signed in as robert@example\.com/)).toBeTruthy());
    expect(row().textContent).toContain("In use");
    expect(row().textContent).toContain("robert@example.com");
    expect(row().textContent).toContain("Pro");

    expect(banner().textContent).toContain("Save to start using it");
    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch().provider).toBe("chatgpt");
  });

  it("does not send an operator to a Save that is not there", async () => {
    open(stored({ provider: "chatgpt" }));
    pane("Provider");
    fireEvent.click(button("Sign in"));

    await waitFor(() => expect(screen.getByText(/Signed in as robert@example\.com/)).toBeTruthy());
    expect(banner().textContent).not.toContain("Save to start using it");
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
  });

  it("says so when the plan signed in to cannot run Codex", async () => {
    completeSubscriptionSignin.mockResolvedValue(signedIn({ plan: "free", includesCodex: false }));
    open();
    pane("Provider");
    fireEvent.click(button("Sign in"));

    await waitFor(() => expect(screen.getByText(/does not include Codex/)).toBeTruthy());
    expect(banner().className).toContain("banner--error");
  });

  it("reports a refused sign-in without leaving a code on screen", async () => {
    beginSubscriptionSignin.mockResolvedValue({
      verificationUrl: "https://auth.openai.com/codex/device",
      userCode: "WXYZ-1234",
      deviceAuthId: "dev-2",
      intervalSecs: 2,
    });
    completeSubscriptionSignin.mockRejectedValue({
      kind: "signinExpired",
      message: "nobody entered the code within fifteen minutes. Start the sign-in again.",
    });
    open();
    pane("Provider");
    fireEvent.click(button("Sign in"));

    await waitFor(() => expect(screen.getByText(/fifteen minutes/)).toBeTruthy());
    expect(screen.queryByText("WXYZ-1234")).toBeNull();
    expect(button("Sign in")).toBeTruthy();
  });

  it("offers a model only once the subscription is the one in use", async () => {
    subscriptionStatus.mockResolvedValue(signedIn());
    open(stored({ provider: "chatgpt" }));
    pane("Provider");

    await waitFor(() => expect(field(/^Model/)).toBeTruthy());
    const select = field(/^Model/) as unknown as HTMLSelectElement;
    expect(select.value).toBe("gpt-5.6-luna");
    expect([...select.options].map((o) => o.value)).toEqual(["gpt-5.6-luna", "gpt-5.4-mini"]);
  });

  it("lists a stored model the known list has never heard of", async () => {
    subscriptionStatus.mockResolvedValue(signedIn());
    open(stored({ provider: "chatgpt", subscriptionModel: "gpt-6-unreleased" }));
    pane("Provider");

    await waitFor(() => expect(field(/^Model/)).toBeTruthy());
    const select = field(/^Model/) as unknown as HTMLSelectElement;
    expect(select.value).toBe("gpt-6-unreleased");
  });

  it("keeps the endpoint model when the subscription is in use", async () => {
    subscriptionStatus.mockResolvedValue(signedIn());
    open(stored({ provider: "chatgpt" }));
    pane("Provider");
    await waitFor(() => expect(field(/^Model/)).toBeTruthy());
    pane("General");
    type(/^Your name/, "Robert W");

    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch()).toEqual({ operatorName: "Robert W" });
  });

  it("moves off the subscription when an endpoint preset is chosen", async () => {
    subscriptionStatus.mockResolvedValue(signedIn());
    open(stored({ provider: "chatgpt" }));
    pane("Provider");
    await waitFor(() => expect(row().textContent).toContain("In use"));

    fireEvent.click(button("Use an API provider"));
    const label = screen.getByText("Groq", { selector: ".preset__name" });
    fireEvent.click(label.closest("button") as HTMLButtonElement);

    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch().provider).toBe("compatible");
    expect(sentPatch().baseUrl).toBe("https://api.groq.com/openai/v1");
  });

  it("can go back to the subscription without signing in again", async () => {
    subscriptionStatus.mockResolvedValue(signedIn());
    open(stored({ provider: "compatible" }));
    pane("Provider");

    await waitFor(() => expect(button("Use the ChatGPT subscription")).toBeTruthy());
    fireEvent.click(button("Use the ChatGPT subscription"));
    expect(row().textContent).toContain("In use");

    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch().provider).toBe("chatgpt");
    expect(beginSubscriptionSignin).not.toHaveBeenCalled();
  });

  it("signing out takes the provider with it", async () => {
    subscriptionStatus.mockResolvedValue(signedIn());
    open(stored({ provider: "chatgpt" }));
    pane("Provider");
    await waitFor(() => expect(button("Sign out")).toBeTruthy());

    fireEvent.click(button("Sign out"));
    await waitFor(() => expect(screen.getByText("Signed out.")).toBeTruthy());

    expect(button("Sign in")).toBeTruthy();
    expect(row().textContent).not.toContain("In use");
  });
});

describe("the Claude subscription", () => {
  function row(): HTMLElement {
    const label = screen.getByText("Claude subscription", { selector: ".preset__name" });
    const found = label.closest(".preset");
    if (!found) throw new Error("no Claude row");
    return found as HTMLElement;
  }

  it("says what it runs, and leaves whether it can to the state beside it", async () => {
    open();
    pane("Provider");
    await waitFor(() => expect(claudeProgram).toHaveBeenCalled());
    expect(row().querySelector(".preset__url")?.textContent).toBe("Runs the claude program");
    expect(row().title).toBe(
      "Runs the claude program, and uses whatever it is signed in to and set to.",
    );
  });

  it("says where to install the program when this host has none", async () => {
    claudeProgram.mockResolvedValue({
      installed: false,
      version: "",
      install: "npm install -g @anthropic-ai/claude-code",
    });
    open();
    pane("Provider");
    await waitFor(() => expect(row().textContent).toContain("Not installed"));
    expect(row().querySelector(".preset__url")?.textContent).toBe("Install the claude program");
    expect(row().title).toBe("Runs the claude program, which is not installed on this host.");
  });

  it("says a group reaches a plan's quota, which is the word everywhere else", async () => {
    open(stored({ provider: "claude" }));
    pane("Provider");
    await waitFor(() => expect(claudeProgram).toHaveBeenCalled());
    const hint = screen.getByText(/rolling quota/);
    expect(hint.textContent).toContain("a group reaches it faster than one person does");
    expect(hint.textContent).not.toContain("crew");
  });

  it("offers host sign-in when Claude is selected", async () => {
    open(stored({ provider: "claude" }));
    pane("Provider");
    expect(await screen.findByRole("button", { name: "Sign in with Claude" })).toBeTruthy();
  });

  it("keeps one row action through sign-in and allows a retry after failure", async () => {
    let reject!: (error: unknown) => void;
    beginClaudeSignin.mockReturnValueOnce(
      new Promise((_, fail) => {
        reject = fail;
      }),
    );
    open();
    pane("Provider");
    const signIn = await within(row()).findByRole("button", { name: "Sign in with Claude" });
    expect(row().parentElement?.querySelector(".claude-signin__panel")).toBeNull();
    expect(within(row()).queryAllByRole("button")).toHaveLength(1);
    fireEvent.click(signIn);
    expect(
      (within(row()).getByRole("button", { name: "Signing in…" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    await act(async () => reject({ kind: "claudeSignin", message: "Claude sign-in timed out." }));
    expect(within(row()).getByRole("button", { name: "Try again" })).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toBe("Claude sign-in timed out.");
    expect(
      row()
        .parentElement?.querySelector(".claude-signin__panel")
        ?.contains(screen.getByRole("alert")),
    ).toBe(true);
    expect(within(row()).queryAllByRole("button")).toHaveLength(1);
  });

  it("keeps the row action disabled while the browser code is requested", async () => {
    open();
    pane("Provider");
    fireEvent.click(await within(row()).findByRole("button", { name: "Sign in with Claude" }));
    await screen.findByLabelText("Code from browser");
    expect(
      row()
        .parentElement?.querySelector(".claude-signin__panel")
        ?.contains(screen.getByLabelText("Code from browser")),
    ).toBe(true);
    expect(
      (within(row()).getByRole("button", { name: "Signing in…" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(within(row()).queryAllByRole("button")).toHaveLength(1);
  });

  it("offers use only for a signed-in Claude, and marks the current provider", async () => {
    claudeSigninStatus.mockResolvedValue({ signedIn: true });
    open();
    pane("Provider");
    const use = await within(row()).findByRole("button", { name: "Use the Claude subscription" });
    expect(row().parentElement?.querySelector(".claude-signin__panel")).toBeNull();
    fireEvent.click(use);
    expect(within(row()).getByText("In use")).toBeTruthy();
    expect(within(row()).queryAllByRole("button")).toHaveLength(0);
  });
});

describe("the panes that name a machine", () => {
  const lede = () => document.querySelector(".settings__pane .settings__lede")?.textContent ?? "";
  const about = () => document.querySelector(".about__facts")?.textContent ?? "";

  it("says this Mac where the runtime is in this window", () => {
    open();
    pane("Account");
    expect(lede()).toContain("Guaca runs on this Mac with your own keys");
    expect(lede()).not.toContain("your host");

    pane("About");
    expect(about()).toContain("This app keeps your display preferences");
    expect(about()).toContain("no address and no token");
  });

  it("says your host in a browser, which is not where the runtime is", () => {
    platform.desktop = false;
    open();
    pane("Account");
    expect(lede()).toContain("Guaca runs on your host with your own keys");
    expect(document.body.textContent).not.toContain("runs on this machine");

    pane("About");
    expect(about()).toContain("a cookie the host sets");
    expect(about()).toContain("the host's token is not stored here");
    expect(about()).not.toContain("keeps the host's address, its token");
  });

  it("says what a machine key lets the host do, not what it gives each agent", () => {
    open();
    pane("Machines");
    const legends = [...document.querySelectorAll<HTMLElement>(".settings__pane .api-key legend")];
    const computer = legends.find((el) => el.title.startsWith("Lets this host rent a computer"));
    expect(computer?.title).toContain(
      "Which agents have one is set on each agent's Computer panel.",
    );
    const browser = legends.find((el) => el.title.startsWith("Lets this host rent a browser"));
    expect(browser?.title).toContain("Which agents have one is set on each agent's Browser panel.");
    expect(legends.some((el) => el.title.startsWith("Gives every agent"))).toBe(false);
  });
});

describe("the Guaca account", () => {
  function row(): HTMLElement {
    const label = screen.getByText("Guaca account", { selector: ".preset__name" });
    const found = label.closest(".preset");
    if (!found) throw new Error("no account row");
    return found as HTMLElement;
  }

  function button(name: string): HTMLButtonElement {
    return screen.getByRole("button", { name }) as HTMLButtonElement;
  }

  it("asks the service nothing until the pane is opened", async () => {
    open();
    expect(accountStatus).not.toHaveBeenCalled();
    expect(accountConnectors).not.toHaveBeenCalled();

    pane("Account");
    await waitFor(() => expect(accountStatus).toHaveBeenCalledTimes(1));
  });

  it("says it is optional, and what an account is for", async () => {
    open(stored(), DEFAULT_PREFS, "account");
    await waitFor(() => expect(accountStatus).toHaveBeenCalled());
    expect(screen.getByText(/Optional, and it stays that way/)).toBeTruthy();
    expect(screen.getByText(/client secret would be inside a download/)).toBeTruthy();
  });

  it("offers a sign-in when there is none", async () => {
    open(stored(), DEFAULT_PREFS, "account");
    await waitFor(() => expect(accountStatus).toHaveBeenCalled());
    expect(row().textContent).toContain("Sign in to connect Gmail");
    expect(button("Sign in")).toBeTruthy();
  });

  it("says which account it linked, and offers the way out", async () => {
    accountStatus.mockResolvedValue(linked());
    open(stored(), DEFAULT_PREFS, "account");

    await waitFor(() => expect(row().textContent).toContain("robert@example.com"));
    expect(button("Sign out")).toBeTruthy();
    expect(button("Manage")).toBeTruthy();
  });

  it("explains broker sign-out and manages through a handoff", async () => {
    accountStatus.mockResolvedValue(linked({ broker: true }));
    signInAccount.mockResolvedValue(linked({ broker: true }));
    open(stored(), DEFAULT_PREFS, "account");
    await waitFor(() => expect(screen.getByText("Your connector")).toBeTruthy());
    expect(screen.getByText(/remove GUACA_BROKER_BEARER/i)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Sign out" })).toBeNull();
    fireEvent.click(button("Manage"));
    await waitFor(() => expect(signInAccount).toHaveBeenCalled());
    expect(openExternal).not.toHaveBeenCalled();
  });

  it("says what is happening while the browser has it, and offers nothing to carry", async () => {
    let release: (status: AccountStatus) => void = () => {};
    signInAccount.mockReturnValue(
      new Promise<AccountStatus>((resolve) => {
        release = resolve;
      }),
    );
    open(stored(), DEFAULT_PREFS, "account");
    await waitFor(() => expect(accountStatus).toHaveBeenCalled());
    fireEvent.click(button("Sign in"));

    await waitFor(() => expect(screen.getByRole("status")).toBeTruthy());
    expect(screen.getByRole("status").textContent).toContain("Finish in the browser window");
    expect(screen.getByRole("status").textContent).toContain("Only continue if you started this");

    release(linked());
    await waitFor(() => expect(row().textContent).toContain("robert@example.com"));
  });

  it("keeps a sign-in in flight across a glance at another section", async () => {
    signInAccount.mockReturnValue(new Promise<AccountStatus>(() => {}));
    open(stored(), DEFAULT_PREFS, "account");
    await waitFor(() => expect(accountStatus).toHaveBeenCalled());
    fireEvent.click(button("Sign in"));
    await waitFor(() => expect(screen.getByRole("status")).toBeTruthy());

    pane("Limits");
    pane("Account");
    expect(screen.getByRole("status").textContent).toContain("Finish in the browser window");
  });

  it("stays signed out when the sign-in fails, and says why", async () => {
    signInAccount.mockRejectedValue({ kind: "account", message: "could not reach guaca.bot" });
    open(stored(), DEFAULT_PREFS, "account");
    await waitFor(() => expect(accountStatus).toHaveBeenCalled());
    fireEvent.click(button("Sign in"));

    await waitFor(() => expect(screen.getByText(/could not reach guaca\.bot/)).toBeTruthy());
    expect(row().textContent).toContain("Sign in to connect Gmail");
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("shows what the account has authorized, and what it has not", async () => {
    accountStatus.mockResolvedValue(linked());
    open(stored(), DEFAULT_PREFS, "account");

    await waitFor(() => expect(accountConnectors).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByText("Gmail")).toBeTruthy());
    expect(screen.queryByText(/Drive/)).toBeNull();
  });

  it("says so rather than showing an empty list when nothing is authorized", async () => {
    accountStatus.mockResolvedValue(linked());
    accountConnectors.mockResolvedValue(held(false));
    open(stored(), DEFAULT_PREFS, "account");

    await waitFor(() => expect(screen.getByText("Nothing authorized yet.")).toBeTruthy());
  });

  it("redraws as signed out when the service no longer knows the sign-in", async () => {
    accountStatus.mockResolvedValue(linked());
    accountConnectors.mockRejectedValue({ kind: "signedOut", message: "sign in again" });
    open(stored(), DEFAULT_PREFS, "account");

    await waitFor(() => expect(row().textContent).toContain("Sign in to connect Gmail"));
  });

  it("forgets the sign-in and the list it came with", async () => {
    accountStatus.mockResolvedValue(linked());
    open(stored(), DEFAULT_PREFS, "account");
    await waitFor(() => expect(screen.getByText("Gmail")).toBeTruthy());

    fireEvent.click(button("Sign out"));
    await waitFor(() => expect(row().textContent).toContain("Sign in to connect Gmail"));
    expect(screen.queryByText("Gmail")).toBeNull();
  });

  it("opens the service's own page to change what is authorized", async () => {
    accountStatus.mockResolvedValue(linked());
    open(stored(), DEFAULT_PREFS, "account");
    await waitFor(() => expect(row().textContent).toContain("robert@example.com"));

    fireEvent.click(button("Manage"));
    expect(openExternal).toHaveBeenCalledWith("https://guaca.bot/app");
  });

  it("says out loud when this build signs in somewhere other than guaca.bot", async () => {
    accountStatus.mockResolvedValue(linked({ origin: "http://localhost:8787" }));
    open(stored(), DEFAULT_PREFS, "account");

    await waitFor(() => expect(screen.getByText(/not guaca\.bot/)).toBeTruthy());
    expect(screen.getByText("http://localhost:8787")).toBeTruthy();
  });

  it("does not say that on a build pointed at guaca.bot", async () => {
    accountStatus.mockResolvedValue(linked());
    open(stored(), DEFAULT_PREFS, "account");
    await waitFor(() => expect(row().textContent).toContain("robert@example.com"));
    expect(screen.queryByText(/not guaca\.bot/)).toBeNull();
  });
});

describe("appearance", () => {
  it("applies and persists the inline reasoning preference immediately", () => {
    open();
    pane("Appearance");
    const toggle = screen.getByRole("switch", { name: "Show reasoning and tool calls" });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    fireEvent.click(toggle);
    expect(useStore.getState().prefs.showReasoning).toBe(true);
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(JSON.parse(localStorage.getItem("guac.prefs")!).showReasoning).toBe(true);
    fireEvent.click(toggle);
    expect(useStore.getState().prefs.showReasoning).toBe(false);
  });

  function choice(label: string): HTMLButtonElement {
    return screen.getByRole("button", { name: label }) as HTMLButtonElement;
  }

  it("keeps a chosen surface and paints it onto the document", () => {
    open();
    pane("Appearance");
    fireEvent.click(choice("Reading surface: Dark"));

    expect(useStore.getState().prefs.surface).toBe("dark");
    expect(document.documentElement.dataset.surface).toBe("dark");
    expect(choice("Reading surface: Dark").getAttribute("aria-pressed")).toBe("true");
    expect(choice("Reading surface: Light").getAttribute("aria-pressed")).toBe("false");
    expect(screen.getByText("Dark mode.")).toBeTruthy();
  });

  it("resolves the system's answer rather than handing it on as one", () => {
    open();
    pane("Appearance");
    fireEvent.click(choice("Reading surface: Match system"));

    expect(useStore.getState().prefs.surface).toBe("system");
    expect(document.documentElement.dataset.surface).toBe("light");
    expect(screen.getByText("Light mode, matching your system.")).toBeTruthy();
  });

  it("updates the system hint when the system changes", () => {
    let change: ((event: MediaQueryListEvent) => void) | undefined;
    vi.stubGlobal("matchMedia", () => ({
      matches: false,
      addEventListener: (_: string, listener: (event: MediaQueryListEvent) => void) => {
        change = listener;
      },
      removeEventListener: () => {},
    }));
    try {
      open();
      pane("Appearance");
      fireEvent.click(choice("Reading surface: Match system"));
      act(() => change?.({ matches: true } as MediaQueryListEvent));
      expect(screen.getByText("Dark mode, matching your system.")).toBeTruthy();
      fireEvent.click(choice("Reading surface: Light"));
      expect(screen.getByText("Light mode.")).toBeTruthy();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("keeps a chosen cast and shows each one before it is chosen", () => {
    open();
    pane("Appearance");
    expect(choice("Characters: Cut").querySelectorAll('[data-cast="cut"]')).toHaveLength(3);
    expect(choice("Characters: Drawn").querySelectorAll('[data-cast="drawn"]')).toHaveLength(3);
    expect(choice("Characters: Drawn").getAttribute("aria-pressed")).toBe("true");

    fireEvent.click(choice("Characters: Cut"));

    expect(useStore.getState().prefs.cast).toBe("cut");
    expect(choice("Characters: Cut").getAttribute("aria-pressed")).toBe("true");
    expect(choice("Characters: Drawn").getAttribute("aria-pressed")).toBe("false");
    expect(choice("Characters: Drawn").querySelectorAll('[data-cast="drawn"]')).toHaveLength(3);
    expect(useStore.getState().prefs.surface).toBe(DEFAULT_PREFS.surface);
  });

  it("keeps a chosen theme and writes it where the stylesheet will lose to it", () => {
    open();
    pane("Appearance");
    fireEvent.click(choice("Grays: Avocado"));
    fireEvent.click(choice("Attention: Violet"));
    fireEvent.click(choice("Contrast: More"));

    const { grays, attention, contrast } = useStore.getState().prefs;
    expect({ grays, attention, contrast }).toEqual({
      grays: "avocado",
      attention: "violet",
      contrast: "more",
    });
    expect(document.getElementById("guac-theme")?.textContent).toBe(
      themeCss({
        grays: "avocado",
        attention: "violet",
        contrast: "more",
        temperature: "plain",
      }),
    );
    expect(choice("Grays: Avocado").getAttribute("aria-pressed")).toBe("true");
    expect(choice("Grays: Neutral").getAttribute("aria-pressed")).toBe("false");
    expect(useStore.getState().prefs.surface).toBe(DEFAULT_PREFS.surface);
  });

  it("applies a preset without changing the surface and recognizes customization", () => {
    open(stored(), { ...DEFAULT_PREFS, surface: "dark" });
    pane("Appearance");
    fireEvent.click(choice("Theme: Field"));
    expect(useStore.getState().prefs).toMatchObject({
      grays: "avocado",
      attention: "amber",
      contrast: "standard",
      temperature: "cool",
      reading: "serif",
      surface: "dark",
    });
    expect(loadPrefs()).toMatchObject({
      grays: "avocado",
      temperature: "cool",
      reading: "serif",
      surface: "dark",
    });
    expect(choice("Theme: Field").getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(choice("Contrast: More"));
    expect(choice("Theme: Field").getAttribute("aria-pressed")).toBe("false");
  });

  it("shows each gray and each attention hue before it is chosen", () => {
    open();
    pane("Appearance");
    const warm = choice("Grays: Warm").querySelector<HTMLElement>(".choice__swatch");
    expect(warm?.style.getPropertyValue("--swatch")).toBe(
      themeTokens({ ...DEFAULT_PREFS, grays: "warm" }, "light")["--rail-ground"],
    );
    const blue = choice("Attention: Blue").querySelector<HTMLElement>(".choice__swatch");
    expect(blue?.style.getPropertyValue("--swatch")).toBe(
      themeTokens({ ...DEFAULT_PREFS, attention: "blue" }, "light")["--attention-fill"],
    );
  });

  it("keeps a typeface, a reading face and a reading size, and names them to the stylesheet", () => {
    open();
    pane("Appearance");
    fireEvent.click(choice("Typeface: Atkinson Hyperlegible"));
    fireEvent.click(choice("Reading face: Literata"));
    fireEvent.click(choice("Reading size: 18px"));

    const { typeface, reading, readingSize } = useStore.getState().prefs;
    expect({ typeface, reading, readingSize }).toEqual({
      typeface: "legible",
      reading: "serif",
      readingSize: 18,
    });
    expect(document.documentElement.dataset.typeface).toBe("legible");
    expect(document.documentElement.dataset.reading).toBe("serif");
    expect(document.documentElement.style.getPropertyValue("--type-read")).toBe("1.125rem");
    expect(useStore.getState().prefs.uiScale).toBe(DEFAULT_PREFS.uiScale);
  });

  it("keeps a chosen scale without disturbing the surface", () => {
    open(stored(), { ...DEFAULT_PREFS, surface: "dark" });
    pane("Appearance");
    fireEvent.click(choice("Interface scale: 110%"));

    expect(useStore.getState().prefs.uiScale).toBe(110);
    expect(useStore.getState().prefs.surface).toBe("dark");
    expect(document.documentElement.dataset.surface).toBe("dark");
    expect(document.documentElement.style.getPropertyValue("--ui-scale")).toBe("1.1");
  });
});

describe("notifications", () => {
  const FAILURES = "Failures";
  const everything = { needsYou: true, failures: true, finished: true };

  it("draws the Host's three switches, and nothing an older build kept in this window", () => {
    open(stored({ attention: everything }));
    pane("Notifications");

    const rows = [...document.querySelectorAll(".switch-row")];
    expect(rows.map((row) => row.querySelector(".switch-row__label")?.textContent)).toEqual([
      "Things that need you",
      FAILURES,
      "Finished work while you are away",
    ]);
    for (const row of rows) {
      expect(row.querySelector("button")?.getAttribute("aria-checked")).toBe("true");
    }
    expect(screen.queryByText("Notify me at all")).toBeNull();
    expect(screen.queryByText("A routine fired")).toBeNull();
  });

  it("toggles one kind and leaves the others alone", async () => {
    updateSettings.mockResolvedValueOnce(stored({ attention: { ...everything, failures: false } }));
    open(stored({ attention: everything }));
    pane("Notifications");
    await act(async () => fireEvent.click(switchOn(FAILURES)));

    expect(switchOn(FAILURES).textContent).toBe("Off");
    expect(updateSettings.mock.calls).toEqual([[{ attention: { failures: false } }]]);
    expect(useStore.getState().settings?.attention).toEqual({ ...everything, failures: false });
    expect(switchOn("Things that need you").textContent).toBe("On");
    expect(useStore.getState().prefs).toEqual(DEFAULT_PREFS);
  });

  it("leaves a switch as it was when the Host refuses, and says why", async () => {
    updateSettings.mockRejectedValueOnce({ kind: "io", message: "The host could not save." });
    open(stored({ attention: everything }));
    pane("Notifications");
    fireEvent.click(switchOn(FAILURES));

    await waitFor(() => expect(screen.getByText(/could not save/)).toBeTruthy());
    expect(banner().className).toContain("banner--error");
    expect(switchOn(FAILURES).textContent).toBe("On");
    expect(switchOn(FAILURES).disabled).toBe(false);
  });

  it("says so when the Host is older than the switches, and still offers the test", () => {
    open(stored());
    pane("Notifications");

    expect(screen.getByText(/older than these switches/)).toBeTruthy();
    expect(document.querySelectorAll(".switch-row")).toHaveLength(0);
    expect(screen.getByRole("button", { name: "Send a test notification" })).toBeTruthy();
  });

  it("says so when the machine refuses a test notification", async () => {
    notifyOperator.mockResolvedValue(false);
    open();
    pane("Notifications");
    fireEvent.click(screen.getByRole("button", { name: "Send a test notification" }));

    await waitFor(() => expect(screen.getByText(/System Settings, then try again/)).toBeTruthy());
    expect(banner().className).toContain("banner--error");
  });

  it("sends something recognizable when the machine allows it", async () => {
    open();
    pane("Notifications");
    fireEvent.click(screen.getByRole("button", { name: "Send a test notification" }));

    await waitFor(() => expect(notifyOperator).toHaveBeenCalledTimes(1));
    expect(notifyOperator.mock.calls[0]).toEqual([
      "Guaca",
      "This is what a notification looks like.",
    ]);
    expect(banner().className).not.toContain("banner--ok");
    expect(banner().className).not.toContain("banner--error");
    expect(screen.getByText(/Handed to the operating system/)).toBeTruthy();
  });

  describe("on a Mac", () => {
    const TURNED_OFF = "macOS has notifications turned off for Guaca.";
    const test = () => screen.getByRole("button", { name: "Send a test notification" });

    it("says macOS allows them, above the test button", async () => {
      notificationAccess.mockResolvedValue("allowed");
      open();
      pane("Notifications");

      const line = await screen.findByText("macOS allows Guaca to show notifications.");
      expect(line.compareDocumentPosition(test()) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      expect(screen.queryByRole("button", { name: "Open System Settings" })).toBeNull();
      expect(screen.queryByText(/The only way to find out/)).toBeNull();
    });

    it("says when macOS has not asked yet, and when it will", async () => {
      notificationAccess.mockResolvedValue("notAsked");
      open();
      pane("Notifications");

      expect(await screen.findByText(/macOS has not asked you about Guaca yet/)).toBeTruthy();
      expect(screen.getByText(/first time Guaca has something to show/)).toBeTruthy();
    });

    it("says they are turned off, and opens System Settings to turn them on", async () => {
      notificationAccess.mockResolvedValue("denied");
      open();
      pane("Notifications");

      expect(await screen.findByText(TURNED_OFF)).toBeTruthy();
      fireEvent.click(screen.getByRole("button", { name: "Open System Settings" }));
      await waitFor(() => expect(openNotificationSettings).toHaveBeenCalledTimes(1));
    });

    it("reads it again when the window comes back from System Settings", async () => {
      notificationAccess.mockResolvedValue("denied");
      open();
      pane("Notifications");
      expect(await screen.findByText(TURNED_OFF)).toBeTruthy();

      notificationAccess.mockResolvedValue("allowed");
      fireEvent.focus(window);
      expect(await screen.findByText("macOS allows Guaca to show notifications.")).toBeTruthy();
      expect(screen.queryByText(TURNED_OFF)).toBeNull();
    });

    it("says a test macOS accepted should be on screen", async () => {
      notificationAccess.mockResolvedValue("allowed");
      open();
      pane("Notifications");
      fireEvent.click(test());

      await waitFor(() =>
        expect(screen.getByText("Sent. It should be on your screen now.")).toBeTruthy(),
      );
      expect(banner().className).toContain("banner--ok");
      expect(screen.queryByText(/Handed to the operating system/)).toBeNull();
    });

    it("says they are turned off when macOS would not show the test", async () => {
      notifyOperator.mockResolvedValue(false);
      notificationAccess.mockResolvedValueOnce("notAsked").mockResolvedValue("denied");
      open();
      pane("Notifications");
      await screen.findByText(/macOS has not asked you about Guaca yet/);
      fireEvent.click(test());

      await waitFor(() => expect(banner().textContent).toContain(TURNED_OFF));
      expect(banner().className).toContain("banner--error");
      expect(screen.queryByText(/Handed to the operating system/)).toBeNull();
      expect(screen.getByRole("button", { name: "Open System Settings" })).toBeTruthy();
    });

    it("says macOS did not take the test when it allows them and refused this one", async () => {
      notifyOperator.mockResolvedValue(false);
      notificationAccess.mockResolvedValue("allowed");
      open();
      pane("Notifications");
      await screen.findByText("macOS allows Guaca to show notifications.");
      fireEvent.click(test());

      await waitFor(() =>
        expect(banner().textContent).toContain("Not sent. macOS did not accept the notification."),
      );
      expect(banner().className).toContain("banner--error");
      expect(banner().textContent).not.toContain(TURNED_OFF);
      expect(screen.queryByText("Sent. It should be on your screen now.")).toBeNull();
    });

    it("is not asked about at all in a browser", () => {
      platform.desktop = false;
      notificationAccess.mockResolvedValue("denied");
      open();
      pane("Notifications");

      expect(notificationAccess).not.toHaveBeenCalled();
      expect(screen.getByText(/In a browser, this is also where it asks/)).toBeTruthy();
    });
  });
});

describe("sending to a phone", () => {
  const everything = { needsYou: true, failures: true, finished: true };
  const NONE: DeliverySettings = {
    urlSet: false,
    urlHint: "",
    tokenSet: false,
    tokenHint: "",
    format: "ntfy",
    content: "outcome",
    linkBase: "",
  };
  const SAVED: DeliverySettings = {
    ...NONE,
    urlSet: true,
    urlHint: "https://ntfy.sh/...7f3a",
    tokenSet: true,
    tokenHint: "...1234",
  };
  const withPush = (delivery: DeliverySettings) => stored({ attention: everything, delivery });
  const status = () => document.querySelector(".push-status");

  it("is not drawn for a Host older than it", () => {
    open(stored({ attention: everything }));
    pane("Notifications");
    expect(screen.queryByText("Send to your phone or another service")).toBeNull();
  });

  it("sends only what was typed, and draws neither the address nor the token back", async () => {
    updateSettings.mockResolvedValueOnce(withPush(SAVED));
    open(withPush(NONE));
    pane("Notifications");
    expect(screen.getByRole("button", { name: "Send a test" })).toHaveProperty("disabled", true);
    type(/^Address/, "  https://ntfy.sh/guaca-secret-topic-7f3a ");
    type(/^Token/, "tk_supersecrettoken1234");

    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Save address" })));
    expect(updateSettings.mock.calls).toEqual([
      [
        {
          delivery: {
            url: "https://ntfy.sh/guaca-secret-topic-7f3a",
            token: "tk_supersecrettoken1234",
          },
        },
      ],
    ]);
    expect(field(/^Address/).value).toBe("");
    expect(field(/^Token/).value).toBe("");
    expect(screen.getByText("https://ntfy.sh/...7f3a")).toBeTruthy();
    expect(document.body.textContent).not.toContain("secret");
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
  });

  it("keeps a typed address across a glance at another section", () => {
    open(withPush(NONE));
    pane("Notifications");
    type(/^Address/, "https://ntfy.example/topic");
    pane("Limits");
    pane("Notifications");
    expect(field(/^Address/).value).toBe("https://ntfy.example/topic");
  });

  it("stops sending by removing the address and the token together", async () => {
    updateSettings.mockResolvedValueOnce(withPush(NONE));
    open(withPush(SAVED));
    pane("Notifications");
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Stop sending" })));
    expect(updateSettings.mock.calls).toEqual([[{ delivery: { url: "", token: "" } }]]);
  });

  it("writes the format and how much a push says at once, one key each", async () => {
    updateSettings.mockResolvedValueOnce(withPush({ ...SAVED, format: "json" }));
    updateSettings.mockResolvedValueOnce(
      withPush({ ...SAVED, format: "json", content: "generic" }),
    );
    open(withPush(SAVED));
    pane("Notifications");
    await act(async () =>
      fireEvent.click(screen.getByRole("button", { name: "Format: JSON webhook" })),
    );
    await act(async () =>
      fireEvent.click(screen.getByRole("button", { name: "Only say that something finished" })),
    );
    expect(updateSettings.mock.calls).toEqual([
      [{ delivery: { format: "json" } }],
      [{ delivery: { content: "generic" } }],
    ]);
    expect(
      screen
        .getByRole("button", { name: "Only say that something finished" })
        .getAttribute("aria-pressed"),
    ).toBe("true");
  });

  it("says how the last attempt failed, in the Host's words", async () => {
    pushReport.mockResolvedValue({
      ...nothingSent(),
      lastAttemptAt: Date.now(),
      lastError: "401 Unauthorized. Check the token.",
    });
    open(withPush(SAVED));
    pane("Notifications");
    await waitFor(() =>
      expect(status()?.textContent).toBe("Last attempt failed: 401 Unauthorized. Check the token."),
    );
    expect(status()?.getAttribute("data-tone")).toBe("error");
  });

  it("says when a push last went out", async () => {
    const at = new Date();
    at.setHours(9, 2, 0, 0);
    pushReport.mockResolvedValue({
      ...nothingSent(),
      lastSentAt: at.getTime(),
      lastAttemptAt: at.getTime(),
    });
    open(withPush(SAVED));
    pane("Notifications");
    await waitFor(() => expect(status()?.textContent).toMatch(/^Last sent today 9:02/));
    expect(status()?.getAttribute("data-tone")).toBe("info");
  });

  it("says what a test got back, and what refused it", async () => {
    open(withPush(SAVED));
    pane("Notifications");
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Send a test" })));
    expect(testPush).toHaveBeenCalledTimes(1);
    expect(status()?.textContent).toBe("Sent, and the address took it.");
    expect(status()?.getAttribute("data-tone")).toBe("ok");

    testPush.mockRejectedValueOnce({
      kind: "delivery",
      message: "401 Unauthorized. Check the token.",
    });
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Send a test" })));
    expect(status()?.textContent).toBe("The test failed: 401 Unauthorized. Check the token.");
    expect(status()?.getAttribute("data-tone")).toBe("error");
  });

  it("says nothing about the address that a person reads with an em dash", () => {
    open(withPush(SAVED));
    pane("Notifications");
    expect(document.body.textContent).not.toContain("\u2014");
  });
});

describe("the push status line", () => {
  const now = new Date(2026, 10, 14, 12, 0).getTime();
  const at = new Date(2026, 10, 14, 9, 12).getTime();

  it("says when a failed push is tried again", () => {
    const line = pushLine(
      { lastSentAt: null, lastAttemptAt: at, lastError: "503 Service Unavailable.", retryAt: at },
      null,
      now,
    );
    expect(line?.tone).toBe("error");
    expect(line?.text).toMatch(
      /^Last attempt failed: 503 Service Unavailable\. Trying again today 9:12/,
    );
  });

  it("puts a test just pressed ahead of the record", () => {
    const line = pushLine(
      { lastSentAt: at, lastAttemptAt: at, lastError: null, retryAt: null },
      { ok: false, text: "The test failed: 404 Not Found. Check the address." },
      now,
    );
    expect(line).toEqual({
      tone: "error",
      text: "The test failed: 404 Not Found. Check the address.",
    });
    expect(pushLine(null, null, now)).toBeNull();
    expect(
      pushLine({ lastSentAt: null, lastAttemptAt: null, lastError: null, retryAt: null }, null, now)
        ?.text,
    ).toBe("Nothing sent yet.");
  });
});

describe("shortcuts", () => {
  it("lists every key the app answers to", () => {
    open();
    pane("Shortcuts");

    const rows = [...document.querySelectorAll(".keys__row")];
    expect(rows).toHaveLength(BINDINGS.length);
    for (const binding of BINDINGS) {
      const row = screen.getByText(binding.what).closest(".keys__row");
      expect(row?.querySelector(".keys__combo")?.textContent, binding.id).toBeTruthy();
    }
  });

  it("groups the rows under the surface each one belongs to", () => {
    open();
    pane("Shortcuts");
    const headings = [...document.querySelectorAll(".keys__group")].map((row) => row.textContent);
    expect(headings).toEqual(
      SURFACES.filter((where) => BINDINGS.some((binding) => binding.where === where)),
    );
  });
});

describe("about", () => {
  it("names the version this build ships as, and the commit behind it", () => {
    open();
    pane("About");

    expect(document.querySelector(".about__version")?.textContent).toMatch(
      /^Version \d+\.\d+\.\d+ \([0-9a-f]{7,}(-dirty)?\)$/,
    );
  });

  it("says where the agents run, which is not in this window", () => {
    open();
    pane("About");

    expect(document.querySelector(".about__facts")?.textContent).toContain("guacad");
    expect(document.body.textContent).not.toContain("not on anybody's server");
  });
});

describe.each([
  {
    name: "Provider",
    label: "API key",
    section: "provider",
    key: "apiKey",
    configured: { apiKeySet: true, apiKeyHint: "…1234" },
  },
  {
    name: "E2B",
    label: "E2B API key",
    section: "machines",
    key: "e2bApiKey",
    configured: { e2bKeySet: true, e2bKeyHint: "…1234" },
  },
  {
    name: "Kernel",
    label: "Kernel API key",
    section: "machines",
    key: "kernelApiKey",
    configured: { kernelKeySet: true, kernelKeyHint: "…1234" },
  },
] as const)("$name credential state", ({ label: title, section, key, configured }) => {
  const label = new RegExp(`^${title}`);
  const change = () => screen.getByRole("button", { name: `Change ${title}` });
  const cancel = () => screen.getByRole("button", { name: `Cancel changing ${title}` });
  const group = () => within(screen.getByRole("group", { name: title }));

  it("keeps a failed first save editable without claiming the key was saved", async () => {
    updateSettings.mockRejectedValue(new Error("Could not save settings. Try again."));
    open(stored({ apiKeySet: false, apiKeyHint: "" }), DEFAULT_PREFS, section);
    type(label, "new-key");
    fireEvent.click(save());
    await waitFor(() => expect(banner().textContent).toContain("Could not save settings"));
    expect(field(label).value).toBe("new-key");
    expect(group().queryByText("API key saved")).toBeNull();
    expect(save().disabled).toBe(false);
  });

  it("keeps the saved key and the replacement draft when saving fails", async () => {
    updateSettings.mockRejectedValue(new Error("Could not save settings. Try again."));
    open(stored(configured), DEFAULT_PREFS, section);
    fireEvent.click(change());
    expect(document.activeElement).toBe(field(label));
    type(label, "replacement-key");
    fireEvent.click(save());
    await waitFor(() => expect(banner().textContent).toContain("Could not save settings"));
    expect(field(label).value).toBe("replacement-key");
    expect(group().getByText("…1234")).toBeTruthy();
    fireEvent.click(cancel());
    expect(screen.queryByLabelText(label)).toBeNull();
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
    expect(updateSettings).toHaveBeenCalledTimes(1);
  });

  it("waits for the runtime before replacing the entry with a saved state", async () => {
    let settle: (value: Settings) => void = () => {};
    updateSettings.mockImplementation(
      () =>
        new Promise((resolve) => {
          settle = resolve;
        }),
    );
    open(stored({ apiKeySet: false, apiKeyHint: "" }), DEFAULT_PREFS, section);
    type(label, "  new-key  ");
    fireEvent.click(save());
    expect(field(label).disabled).toBe(true);
    expect(group().queryByText("API key saved")).toBeNull();
    settle(stored(configured));
    await waitFor(() => expect(group().getByText("API key saved")).toBeTruthy());
    expect(screen.queryByLabelText(label)).toBeNull();
    expect(sentPatch()[key]).toBe("new-key");
    expect(group().getByText("…1234")).toBeTruthy();
    expect(change()).toBeTruthy();
    if (section === "machines") {
      expect(field(/^Sleep computers after/)).toBeTruthy();
      expect(field(/^Close browsers after/)).toBeTruthy();
    } else {
      expect(field(/^Default model/)).toBeTruthy();
      expect(probe()).toBeTruthy();
    }
    pane("General");
    pane(section === "provider" ? "Provider" : "Machines");
    expect(screen.queryByLabelText(label)).toBeNull();
    expect(group().getByText("API key saved")).toBeTruthy();
  });

  it("preserves replacement edits across panes and hides the editor after saving", async () => {
    open(stored(configured), DEFAULT_PREFS, section);
    expect(group().getByText("API key saved")).toBeTruthy();
    expect(screen.queryByLabelText(label)).toBeNull();
    fireEvent.click(change());
    expect(field(label).value).toBe("");
    type(label, "replacement-key");
    pane("General");
    pane(section === "provider" ? "Provider" : "Machines");
    expect(field(label).value).toBe("replacement-key");
    updateSettings.mockResolvedValue(stored(configured));
    fireEvent.click(save());
    await waitFor(() => expect(screen.queryByLabelText(label)).toBeNull());
    expect(sentPatch()[key]).toBe("replacement-key");
    fireEvent.click(change());
    expect(field(label).value).toBe("");
  });

  it("can cancel a replacement and save a timer without sending a key", async () => {
    open(stored(configured), DEFAULT_PREFS, section);
    fireEvent.click(change());
    type(label, "discard-this-key");
    type(section === "provider" ? /^Give up on a call after/ : /^Close browsers after/, "30");
    fireEvent.click(cancel());
    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch()).not.toHaveProperty(key);
    expect(sentPatch()[section === "provider" ? "requestTimeoutSecs" : "browserIdleMinutes"]).toBe(
      30,
    );
  });
});

describe("the provider key when an endpoint changes", () => {
  it.each(["preset", "typed"])("opens the key editor for a %s endpoint change", async (kind) => {
    open(stored(), DEFAULT_PREFS, "provider");
    expect(screen.queryByLabelText(/^API key/)).toBeNull();
    expect(screen.queryByText(/Blank keeps the saved key/)).toBeNull();
    if (kind === "preset") {
      fireEvent.click(screen.getByRole("button", { name: /Groq/ }));
    } else {
      type(/^Inference endpoint/, "https://gateway.example/v1");
    }
    expect(field(/^API key/).value).toBe("");
    expect(screen.getByText(/including when changing endpoints/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel changing API key" }));
    fireEvent.click(probe());
    await waitFor(() => expect(testConnection).toHaveBeenCalledTimes(1));
    expect(testConnection.mock.calls[0]?.[0]).not.toHaveProperty("apiKey");
    expect(updateSettings).not.toHaveBeenCalled();
  });
});

it("saves a ChatGPT effort default", async () => {
  subscriptionStatus.mockResolvedValue(signedIn());
  open(stored({ provider: "chatgpt", reasoningEffort: "high" }), DEFAULT_PREFS, "provider");
  const effort = await screen.findByRole("combobox", { name: /^Reasoning effort/ });
  await screen.findByRole("option", { name: "low" });
  fireEvent.change(effort, { target: { value: "low" } });
  fireEvent.click(save());
  await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
  expect(sentPatch().reasoningEffort).toBe("low");
});
