import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Prefs } from "../lib/prefs";
import type {
  AccountConnectors,
  AccountStatus,
  DeviceCode,
  GuardLimits,
  HarnessOnMachine,
  Settings,
  SettingsPatch,
  SubscriptionStatus,
} from "../lib/types";
import type { Section } from "./SettingsDialog";

/**
 * The settings dialog, over a mocked runtime.
 *
 * Nine panes, and every field of every one of them held by the shell. That is
 * where the risk is, and none of it is visible in the markup: the patch the
 * Save button sends is assembled by omission, so a box left blank has to be
 * absent from that object rather than present and empty. An empty string sent
 * as an API key clears a stored key, and a zero sent as a limit is a limit that
 * refuses all work. The other half is the same state read the other way round:
 * changing section unmounts a pane, so anything a pane held would be lost by a
 * glance at Limits and back.
 *
 * The runtime is mocked down to the two calls this surface makes, which is
 * enough to assert the one thing that matters about each: what was on screen
 * when it was pressed.
 */

const HELD: GuardLimits = {
  maxHops: 8,
  maxStepsPerRun: 60,
  maxFanoutPerCall: 8,
  maxSendsPerPair: 6,
  maxToolRounds: 24,
};

/** What the runtime is holding when the dialog opens. */
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
    subscriptionModels: ["gpt-5.6-luna", "gpt-5.4-mini"],
    apiKeySet: true,
    apiKeyHint: "…9f2c",
    requestTimeoutSecs: 120,
    limits: { ...HELD },
    ...over,
  };
}

/** Nobody signed in, which is what a fresh install looks like. */
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
const testConnection = vi.fn<(patch?: SettingsPatch) => Promise<string>>(async () => "Reached it.");
const notifyOperator = vi.fn<(title: string, body: string) => Promise<boolean>>(async () => true);
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

/** No Guaca account, which is what an install that never signs in looks like. */
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
/** Installed by default: the provider list draws the same either way, and a
 *  suite that had to opt in to a present program would be asserting the
 *  refusal rather than the row. */
const codingHarnesses = vi.fn<() => Promise<HarnessOnMachine[]>>(async () => [
  {
    harness: "claude",
    installed: true,
    version: "2.1.247 (Claude Code)",
    bridged: true,
    install: "npm install -g @anthropic-ai/claude-code",
  },
  {
    harness: "pi",
    installed: true,
    version: "0.9.0",
    bridged: false,
    install: "npm install -g @mariozechner/pi",
  },
]);

vi.mock("../lib/ipc", () => ({
  api: {
    updateSettings: (patch: SettingsPatch) => updateSettings(patch),
    testConnection: (patch?: SettingsPatch) => testConnection(patch),
    subscriptionStatus: () => subscriptionStatus(),
    beginSubscriptionSignin: () => beginSubscriptionSignin(),
    completeSubscriptionSignin: (code: DeviceCode) => completeSubscriptionSignin(code),
    signOutSubscription: () => signOutSubscription(),
    accountStatus: () => accountStatus(),
    signInAccount: () => signInAccount(),
    accountConnectors: () => accountConnectors(),
    signOutAccount: () => signOutAccount(),
    codingHarnesses: () => codingHarnesses(),
  },
  notifyOperator: (title: string, body: string) => notifyOperator(title, body),
  openExternal: (url: string) => openExternal(url),
}));

const { SettingsDialog } = await import("./SettingsDialog");
const { useStore } = await import("../lib/store");
const { DEFAULT_PREFS, NOTIFY_KINDS } = await import("../lib/prefs");
const { BINDINGS, SURFACES } = await import("../lib/keybinds");
const { PROVIDERS } = await import("../lib/providers");

const onClose = vi.fn();

function open(settings: Settings | null = stored(), prefs: Prefs = DEFAULT_PREFS, on?: Section) {
  useStore.setState({ settings, prefs });
  return render(<SettingsDialog onClose={onClose} section={on} />);
}

function pane(label: string): void {
  fireEvent.click(screen.getByRole("tab", { name: label }));
}

/**
 * One field's input, by the words above it.
 *
 * Anchored patterns rather than exact names: every hint on this surface lives
 * inside its own label, so the accessible name of a box is its label followed
 * by a sentence of explanation.
 */
function field(label: RegExp): HTMLInputElement {
  return screen.getByLabelText(label) as HTMLInputElement;
}

function type(label: RegExp, value: string): void {
  fireEvent.change(field(label), { target: { value } });
}

/** A checkbox on the same surface, found the same way and toggled. */
function check(label: RegExp): void {
  fireEvent.click(field(label));
}

function save(): HTMLButtonElement {
  return screen.getByRole("button", { name: "Save" }) as HTMLButtonElement;
}

/**
 * Test connection.
 *
 * It lives in the Provider pane rather than in the foot, because it reads the
 * endpoint, the key and the model and acts on none of the other seven sections.
 * So every test that presses it opens on that pane first.
 */
function probe(): HTMLButtonElement {
  return screen.getByRole("button", { name: "Test connection" }) as HTMLButtonElement;
}

/** Whatever the dialog is currently saying about itself. */
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

/**
 * The switch on the row carrying this label.
 *
 * By the row rather than by role: the switches on this pane are named "On" and
 * "Off" and nothing ties one to the label beside it, so a role query cannot
 * tell the five of them apart.
 */
function switchOn(label: string): HTMLButtonElement {
  const row = screen.getByText(label).closest(".switch-row");
  const control = row?.querySelector("button");
  if (!control) throw new Error(`no switch on the row for ${label}`);
  return control as HTMLButtonElement;
}

/** Every kind's switch, in the order the pane lists them. The master is first. */
function kindSwitches(): HTMLButtonElement[] {
  return [...document.querySelectorAll(".switch-row")].slice(1).map((row) => {
    const control = row.querySelector("button");
    if (!control) throw new Error("a switch row with no switch in it");
    return control as HTMLButtonElement;
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  updateSettings.mockResolvedValue(stored());
  testConnection.mockResolvedValue("Reached OpenRouter. 312 models.");
  notifyOperator.mockResolvedValue(true);
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
    // Closing on a failed save is the one thing that turns a refusal into lost
    // work: the operator would reopen the dialog onto the stored values with no
    // way to tell what had landed.
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
    // On Provider so both of the buttons a press could double up are on screen,
    // and with the endpoint typed into because the Save is only drawn while
    // something is waiting for it.
    open(stored(), DEFAULT_PREFS, "provider");
    type(/^Inference endpoint/, "http://localhost:1234/v1");
    fireEvent.click(save());

    // Two config writes racing each other is the failure this prevents, and the
    // second one would carry the same patch, so nothing would look wrong after.
    expect(save().disabled).toBe(true);
    expect(probe().disabled).toBe(true);
    fireEvent.click(save());
    expect(updateSettings).toHaveBeenCalledTimes(1);

    settle(stored());
    await waitFor(() => expect(save().disabled).toBe(false));
  });
});

describe("what a save sends", () => {
  it("leaves a blank key, timer and timeout out of the patch entirely", async () => {
    open();
    // The name is touched and nothing else is, because the Save is only drawn
    // while something is waiting for it. Every box below opens blank and is
    // left blank, which is the state this is about.
    type(/^Your name/, "Robert W");
    fireEvent.click(save());

    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    const patch = sentPatch();
    // Absent, not empty and not zero. An empty `apiKey` clears the stored key,
    // and a zero timeout or sleep timer is a setting nobody chose.
    expect("apiKey" in patch).toBe(false);
    expect("e2bApiKey" in patch).toBe(false);
    expect("computerIdleMinutes" in patch).toBe(false);
    expect("kernelApiKey" in patch).toBe(false);
    expect("browserIdleMinutes" in patch).toBe(false);
    expect("requestTimeoutSecs" in patch).toBe(false);
    expect(patch).toEqual({
      operatorName: "Robert W",
      // Both models go every time, and so does the provider. Each model belongs
      // to one provider, so sending only the active one would leave the other
      // to be overwritten by whatever the next save happened to be looking at.
      provider: "compatible",
      baseUrl: "https://openrouter.ai/api/v1",
      defaultModel: "anthropic/claude-sonnet-4.5",
      subscriptionModel: "gpt-5.6-luna",
      // The one field here that cannot be omitted: a checkbox left alone is a
      // decision, and off has to be sendable or stealth can never be turned off.
      browserStealth: false,
      limits: HELD,
    });
  });

  it("does not read a box of spaces as a key", async () => {
    open();
    pane("Provider");
    type(/^API key/, "   ");
    pane("Machines");
    type(/^E2B API key/, " ");
    type(/^Kernel API key/, "   ");
    // Spaces are not an edit either, by the same rule that keeps them out of
    // the patch, so the Save is reached through a field that is one.
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
    // Clearing a number box hands back nothing at all, and a limit of zero
    // refuses every send the runtime is asked to make.
    expect(field(/^Relay depth/).value).toBe("1");

    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch().limits).toEqual({ ...HELD, maxHops: 1 });
  });

  it("sends a limit the runtime will clamp exactly as it was typed", async () => {
    open();
    pane("Limits");
    type(/^Relay depth/, "40");

    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    // The ceiling in the field's metadata is drawn, not enforced: only the floor
    // is applied here, and `GuardLimits::sanitized` is what actually holds this
    // to 16. Recorded rather than endorsed, and the box goes on reading 40.
    expect(sentPatch().limits?.maxHops).toBe(40);
  });

  it("trims a typed key and carries the rest of the panes with it", async () => {
    open();
    pane("Provider");
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
      operatorName: "Robert",
      provider: "compatible",
      baseUrl: "http://localhost:1234/v1",
      defaultModel: "qwen3-coder-30b",
      subscriptionModel: "gpt-5.6-luna",
      apiKey: "sk-or-v1-typed",
      e2bApiKey: "e2b_typed",
      computerIdleMinutes: 45,
      kernelApiKey: "sk_typed",
      browserIdleMinutes: 5,
      browserStealth: true,
      requestTimeoutSecs: 30,
      limits: HELD,
    });
  });

  it("carries the browser half of Machines on its own, with no computer touched", async () => {
    // The half of this pane that pays for browsers went missing from the patch
    // once: three fields typed into, cleared on save, and never sent. What the
    // operator saw was the Kernel key refusing to stick and no browser widget
    // in the channel, with "Saved." over the top of it.
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
    type(/^API key/, "sk-or-v1-typed");
    type(/^Give up on a call after/, "30");
    pane("Machines");
    type(/^E2B API key/, "e2b_typed");
    // Past the ceiling the runtime clamps to, which is the case a box left
    // reading what was typed gets wrong.
    type(/^Sleep computers after/, "2000");

    updateSettings.mockResolvedValue(
      stored({ apiKeySet: true, apiKeyHint: "…yped", computerIdleMinutes: 1440 }),
    );
    fireEvent.click(save());
    await waitFor(() => expect(screen.getByText("Saved.")).toBeTruthy());

    // Blank is the resting state of every box that stages something, and the
    // placeholder beside each one is read from what came back. Left as it was
    // typed, a key is indistinguishable from an edit nobody saved and a
    // duration is a number under "Saved." that the runtime did not store.
    expect(field(/^E2B API key/).value).toBe("");
    expect(field(/^Sleep computers after/).value).toBe("");
    expect(field(/^Sleep computers after/).placeholder).toBe("1440 minutes");
    pane("Provider");
    expect(field(/^API key/).value).toBe("");
    expect(field(/^API key/).placeholder).toBe("Stored …yped");
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

  it("offers nothing on the panes that write as they are clicked", () => {
    // Appearance and Notifications are kept the moment they are pressed, and a
    // Save under them said the opposite about settings that were already saved.
    open(stored(), DEFAULT_PREFS, "appearance");
    fireEvent.click(screen.getByRole("button", { name: "Reading surface: Ink" }));
    expect(saveButton()).toBeNull();

    pane("Notifications");
    fireEvent.click(switchOn("Notify me at all"));
    expect(saveButton()).toBeNull();
  });

  it("offers nothing on the panes there is nothing to save on", () => {
    open(stored(), DEFAULT_PREFS, "shortcuts");
    expect(saveButton()).toBeNull();
    pane("About");
    expect(saveButton()).toBeNull();
  });

  it("is still reachable from a pane that stages nothing", () => {
    // Every pane's state is held by the shell, so an endpoint typed on Provider
    // is still unsaved from Shortcuts. A Save that went missing on the way past
    // is how it would get lost.
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
    // The box is blank while it inherits and the placeholder says what that is,
    // so typing the number back in changes nothing and offers nothing.
    open();
    pane("Machines");
    type(/^Sleep computers after/, "15");
    expect(saveButton()).toBeNull();
    type(/^Sleep computers after/, "45");
    expect(saveButton()).toBeTruthy();
  });
});

describe("moving between sections", () => {
  it("keeps typing that has not been saved", () => {
    open();
    pane("Provider");
    type(/^Inference endpoint/, "http://localhost:1234/v1");
    pane("Machines");
    type(/^Sleep computers after/, "45");
    pane("Limits");
    type(/^Recipients per send/, "3");

    // The whole reason every field is held by the shell. A pane owning its own
    // state loses it the moment the operator glances at another one, and the
    // loss is silent: the box is simply back to what is stored.
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

  it("opens on the section it was pointed at", () => {
    // The palette and the missing-key banner both point at one, and landing on
    // General to hunt for it is the step they exist to remove.
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

    // Bound to the window rather than to the panel for exactly this: focus is
    // on a nav button the moment the operator has finished choosing a section,
    // which is when they reach for Escape.
    fireEvent.keyDown(tab, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);

    // And from outside the panel, which is where focus goes after a click on
    // the scrim. A handler on the panel would never see this one.
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
    // Back to the pane the button lives on. The name typed on the way past is
    // still held by the shell, which is the other half of what this checks.
    pane("Provider");

    fireEvent.click(probe());
    await waitFor(() => expect(testConnection).toHaveBeenCalledTimes(1));

    const patch = testConnection.mock.calls[0]?.[0];
    // Neither a name nor a limit is something an endpoint can be tested
    // against, and the unsaved key is the whole point: probing the stored one
    // reports "no API key" for a key the operator can see in front of them.
    expect(patch).toEqual({
      // The provider goes with it: a test that reached the subscription while
      // the operator is looking at a typed endpoint reports on the wrong place.
      provider: "compatible",
      baseUrl: "https://api.groq.com/openai/v1",
      defaultModel: "anthropic/claude-sonnet-4.5",
      subscriptionModel: "gpt-5.6-luna",
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

    // A hand-typed endpoint is chosen, not unrecognized, so nothing is marked.
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
    // Filling a box is not saving it.
    expect(updateSettings).not.toHaveBeenCalled();
  });

  it("leaves the model alone for a preset that has no opinion about one", () => {
    open();
    pane("Provider");
    type(/^Default model/, "qwen3-coder-30b");
    fireEvent.click(preset("LM Studio"));

    // A local server serves whatever is loaded into it, so blanking the model
    // here would be replacing a working answer with a guess.
    expect(field(/^Inference endpoint/).value).toBe("http://localhost:1234/v1");
    expect(field(/^Default model/).value).toBe("qwen3-coder-30b");
  });

  it("does not ask a machine on this desk for a key", () => {
    open(stored({ apiKeySet: false, apiKeyHint: "" }));
    pane("Provider");
    // An empty key field beside a warning about a missing key is the state an
    // operator running a local model will try to fix forever.
    expect(preset("LM Studio").textContent).toContain("On this machine");
    expect(preset("OpenRouter").textContent).toContain("Needs a key");
  });

  it("withholds the rows a server cannot offer, and says why on each", () => {
    // Withheld rather than hidden. A row that vanishes on a server is a pane
    // that disagrees with the operator's laptop and explains nothing, and a
    // control that fails only after the field is filled in is worse.
    useStore.setState({
      capabilities: {
        localDirectories: false,
        loopbackEndpoints: false,
        claudeProvider: false,
        claudeCodeHarness: false,
        localFiles: false,
      },
    });
    try {
      open();
      pane("Provider");

      expect(preset("LM Studio").textContent).toContain("Not from a server");
      expect((preset("LM Studio") as HTMLButtonElement).disabled).toBe(true);
      expect(preset("Ollama").textContent).toContain("Not from a server");
      // The ones a server can reach are untouched.
      expect((preset("OpenRouter") as HTMLButtonElement).disabled).toBe(false);

      expect(screen.getByText("Not on a server")).toBeTruthy();
      expect(screen.queryByRole("button", { name: "Use the Claude subscription" })).toBeNull();
    } finally {
      useStore.setState({
        capabilities: {
          localDirectories: true,
          loopbackEndpoints: true,
          claudeProvider: true,
          claudeCodeHarness: true,
          localFiles: true,
        },
      });
    }
  });

  it("says a key is stored once one is", () => {
    open(stored({ apiKeySet: true, apiKeyHint: "…9f2c" }));
    pane("Provider");
    expect(preset("OpenRouter").textContent).toContain("Key stored");
    expect(field(/^API key/).getAttribute("placeholder")).toBe("Stored …9f2c");
  });

  it("says nothing about a key on a row that is not the one chosen", () => {
    // There is one key and it belongs to the endpoint in the field below, so a
    // hosted row that is not the chosen one has nothing true to say about it.
    // Reporting on the strength of another provider's key put "Key stored"
    // against five providers the operator had never used.
    open(stored({ apiKeySet: true, apiKeyHint: "…9f2c" }));
    pane("Provider");

    for (const name of ["OpenAI", "Groq", "Together", "Fireworks"]) {
      expect(preset(name).textContent, name).not.toContain("Key stored");
      expect(preset(name).textContent, name).not.toContain("Needs a key");
    }
    // And a local one still says what is true of the server itself.
    expect(preset("Ollama").textContent).toContain("On this machine");
  });
});

/**
 * The ChatGPT subscription.
 *
 * Three states rather than two, and the middle one is the whole reason this is
 * worth testing: the sign-in is one awaited call that parks for as long as the
 * operator takes, so a code drawn on screen and a code that has been redeemed
 * are different renders of the same in-flight promise. All of it lives in the
 * shell, so a glance at Limits mid-sign-in must not discard it.
 */
describe("the ChatGPT subscription", () => {
  /** The subscription row, which is not one of the endpoint presets. */
  function row(): HTMLElement {
    const label = screen.getByText("ChatGPT subscription", { selector: ".preset__name" });
    const found = label.closest(".preset");
    if (!found) throw new Error("no subscription row");
    return found as HTMLElement;
  }

  function button(name: string): HTMLButtonElement {
    return screen.getByRole("button", { name }) as HTMLButtonElement;
  }

  /** A sign-in that has been started and not yet finished. */
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
    // No key field, no URL: there is nothing here to type wrong.
    expect(row().textContent).toContain("no per-token bill");
  });

  it("does not ask the runtime about a sign-in until the pane is opened", async () => {
    open();
    // Seven other panes have no use for it, and a status nobody is looking at
    // is a round trip spent for nothing.
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
    // Both, because a browser that refuses to open leaves the operator with
    // nothing else to go on.
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

    // The pane is unmounted and remounted. A code held by it rather than by the
    // shell would be gone, and the operator would be holding a code the app had
    // forgotten while the call for it was still parked.
    pane("Limits");
    pane("Provider");
    expect(screen.getByText("ABCD-EFGH")).toBeTruthy();
  });

  it("switches the provider to the subscription once it is signed in", async () => {
    open();
    pane("Provider");
    fireEvent.click(button("Sign in"));

    await waitFor(() => expect(screen.getByText(/Signed in as robert@example\.com/)).toBeTruthy());
    // Nobody signs in to a subscription in order to keep paying with a key, so
    // this is done rather than offered as a further step.
    expect(row().textContent).toContain("In use");
    expect(row().textContent).toContain("robert@example.com");
    expect(row().textContent).toContain("Pro");

    // And the save that follows carries it, which is what the message says.
    expect(banner().textContent).toContain("Save to start using it");
    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch().provider).toBe("chatgpt");
  });

  it("does not send an operator to a Save that is not there", async () => {
    // A sign-in that stopped working leaves the provider where it was, so
    // signing back in changes nothing: there is no Save, and a message naming
    // one sends the operator looking for a button that is not on screen.
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

    // A free plan signs in perfectly well and then cannot make one call, so the
    // sign-in that looked like a success has to say what will happen next.
    await waitFor(() => expect(screen.getByText(/does not include Codex/)).toBeTruthy());
    // Drawn as a failure, not as a success. A green "signed in" over a plan that
    // cannot make one call is the message an operator acts on and then wonders
    // why every agent is refused.
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
    // A dead code left on screen is a code somebody will keep typing.
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
    // A model chosen before the list changed must not be silently swapped for
    // another one: that is a turn running on something nobody picked.
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
    // Saved for a reason that has nothing to do with either model, which is the
    // case where one of them going missing would not be noticed.
    pane("General");
    type(/^Your name/, "Robert W");

    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    // The endpoint's model is still sent, untouched. An operator who runs out of
    // quota and switches back has to find it where they left it.
    expect(sentPatch().defaultModel).toBe("anthropic/claude-sonnet-4.5");
    expect(sentPatch().subscriptionModel).toBe("gpt-5.6-luna");
  });

  it("moves off the subscription when an endpoint preset is chosen", async () => {
    subscriptionStatus.mockResolvedValue(signedIn());
    open(stored({ provider: "chatgpt" }));
    pane("Provider");
    await waitFor(() => expect(row().textContent).toContain("In use"));

    const label = screen.getByText("Groq", { selector: ".preset__name" });
    fireEvent.click(label.closest("button") as HTMLButtonElement);

    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    // Otherwise the operator fills in a URL and a key that nothing uses, with
    // no error anywhere to explain why.
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
    // Signing in again would send the operator back to a browser for a
    // credential this machine is already holding.
    expect(beginSubscriptionSignin).not.toHaveBeenCalled();
  });

  it("signing out takes the provider with it", async () => {
    subscriptionStatus.mockResolvedValue(signedIn());
    open(stored({ provider: "chatgpt" }));
    pane("Provider");
    await waitFor(() => expect(button("Sign out")).toBeTruthy());

    fireEvent.click(button("Sign out"));
    await waitFor(() => expect(screen.getByText("Signed out.")).toBeTruthy());

    // Left on the subscription, every agent's next turn is the same refusal.
    expect(button("Sign in")).toBeTruthy();
    expect(row().textContent).not.toContain("In use");
  });
});

/**
 * The Guaca account.
 *
 * The thing worth testing here is that it is optional and behaves like it. An
 * install that never opens this pane must never reach the service, a sign-in
 * that fails must leave the pane signed out rather than half linked, and a
 * service that no longer recognizes a stored sign-in has to redraw as signed
 * out rather than as an account with an empty list under it.
 */
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
    // The one claim that has to survive an edit: nothing here is required.
    expect(screen.getByText(/Optional, and it stays that way/)).toBeTruthy();
    expect(screen.getByText(/client secret would be inside a download/)).toBeTruthy();
  });

  it("offers a sign-in when there is none", async () => {
    open(stored(), DEFAULT_PREFS, "account");
    await waitFor(() => expect(accountStatus).toHaveBeenCalled());
    expect(row().textContent).toContain("Sign in to authorize");
    expect(button("Sign in")).toBeTruthy();
  });

  it("says which account it linked, and offers the way out", async () => {
    accountStatus.mockResolvedValue(linked());
    open(stored(), DEFAULT_PREFS, "account");

    await waitFor(() => expect(row().textContent).toContain("robert@example.com"));
    expect(button("Sign out")).toBeTruthy();
    expect(button("Manage")).toBeTruthy();
  });

  it("says what is happening while the browser has it, and offers nothing to carry", async () => {
    // Unlike the subscription there is no code: the answer comes back to a port
    // this process already holds, so this state is a sentence, not a task.
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
    // The shell holds it. A pane that held its own would discard a sign-in the
    // operator is in the middle of.
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
    expect(row().textContent).toContain("Sign in to authorize");
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("shows what the account has authorized, and what it has not", async () => {
    accountStatus.mockResolvedValue(linked());
    open(stored(), DEFAULT_PREFS, "account");

    await waitFor(() => expect(accountConnectors).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByText("Gmail")).toBeTruthy());
    // Drive is in the list and not granted, so it must not be named as one the
    // agents can reach.
    expect(screen.queryByText(/Drive/)).toBeNull();
  });

  it("says so rather than showing an empty list when nothing is authorized", async () => {
    accountStatus.mockResolvedValue(linked());
    accountConnectors.mockResolvedValue(held(false));
    open(stored(), DEFAULT_PREFS, "account");

    await waitFor(() => expect(screen.getByText("Nothing authorized yet.")).toBeTruthy());
  });

  it("redraws as signed out when the service no longer knows the sign-in", async () => {
    // A stored token the service has revoked. Drawing an account with an empty
    // list under it would tell the operator they are linked when they are not.
    accountStatus.mockResolvedValue(linked());
    accountConnectors.mockRejectedValue({ kind: "signedOut", message: "sign in again" });
    open(stored(), DEFAULT_PREFS, "account");

    await waitFor(() => expect(row().textContent).toContain("Sign in to authorize"));
  });

  it("forgets the sign-in and the list it came with", async () => {
    accountStatus.mockResolvedValue(linked());
    open(stored(), DEFAULT_PREFS, "account");
    await waitFor(() => expect(screen.getByText("Gmail")).toBeTruthy());

    fireEvent.click(button("Sign out"));
    await waitFor(() => expect(row().textContent).toContain("Sign in to authorize"));
    // The list went with it. Leaving it on screen would be a list of what an
    // account this machine no longer holds can reach.
    expect(screen.queryByText("Gmail")).toBeNull();
  });

  it("opens the service's own page to change what is authorized", async () => {
    // The consent screens are Google's and GitHub's, reached through guaca.bot.
    // There is nothing to tick here and pretending otherwise would be a control
    // that cannot work.
    accountStatus.mockResolvedValue(linked());
    open(stored(), DEFAULT_PREFS, "account");
    await waitFor(() => expect(row().textContent).toContain("robert@example.com"));

    fireEvent.click(button("Manage"));
    expect(openExternal).toHaveBeenCalledWith("https://guaca.bot/app");
  });

  it("says out loud when this build signs in somewhere other than guaca.bot", async () => {
    // Development, or a self-hosted service. The two hold different accounts
    // and only one of them is the real one.
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
  /**
   * One of the choice buttons.
   *
   * Matched on its accessible name rather than the word printed on it: a button
   * reading "Ink" or "110%" says nothing about what it is choosing, so each
   * carries an `aria-label` that does.
   */
  function choice(label: string): HTMLButtonElement {
    return screen.getByRole("button", { name: label }) as HTMLButtonElement;
  }

  it("keeps a chosen surface and paints it onto the document", () => {
    open();
    pane("Appearance");
    fireEvent.click(choice("Reading surface: Ink"));

    // Both halves matter: the preference is what survives the window closing,
    // and the attribute is what the stylesheet reads.
    expect(useStore.getState().prefs.surface).toBe("dark");
    expect(document.documentElement.dataset.surface).toBe("dark");
    expect(choice("Reading surface: Ink").getAttribute("aria-pressed")).toBe("true");
    expect(choice("Reading surface: Paper").getAttribute("aria-pressed")).toBe("false");
    expect(screen.getByText(/Currently drawing ink/)).toBeTruthy();
  });

  it("resolves the system's answer rather than handing it on as one", () => {
    open();
    pane("Appearance");
    fireEvent.click(choice("Reading surface: Follow the system"));

    // `system` is a choice the operator makes; it is never what the stylesheet
    // is given. The stub here reports no preference, which is paper.
    expect(useStore.getState().prefs.surface).toBe("system");
    expect(document.documentElement.dataset.surface).toBe("light");
  });

  it("keeps a chosen scale without disturbing the surface", () => {
    open(stored(), { ...DEFAULT_PREFS, surface: "dark" });
    pane("Appearance");
    fireEvent.click(choice("Interface scale: 110%"));

    expect(useStore.getState().prefs.uiScale).toBe(110);
    // Each choice writes both, so a scale that read the wrong surface would
    // flip the column to paper on the way past.
    expect(useStore.getState().prefs.surface).toBe("dark");
    expect(document.documentElement.dataset.surface).toBe("dark");
    expect(document.documentElement.style.getPropertyValue("--ui-scale")).toBe("1.1");
  });
});

describe("notifications", () => {
  const ROUTINE = "A routine fired";

  it("disables every kind while the master switch is off", () => {
    open();
    pane("Notifications");
    fireEvent.click(switchOn("Notify me at all"));

    expect(useStore.getState().prefs.notify.on).toBe(false);
    expect(kindSwitches()).toHaveLength(NOTIFY_KINDS.length);
    // Off means none of the below, whatever they say, and a switch that can
    // still be flipped says the opposite. What each one REPORTS, though, is its
    // own setting rather than its setting and the master together: the row is
    // already unreachable, and what it has to say is what will apply when the
    // master goes back on. Reading them together had the label say On while the
    // control reported itself unchecked, which is a disagreement a sighted
    // operator and a screen reader would resolve differently.
    for (const control of kindSwitches()) {
      expect(control.disabled).toBe(true);
      expect(control.getAttribute("aria-checked")).toBe("true");
    }
  });

  it("toggles one kind and leaves the others alone", () => {
    open();
    pane("Notifications");
    fireEvent.click(switchOn(ROUTINE));

    expect(useStore.getState().prefs.notify.kinds).toEqual({
      approval: true,
      stuck: true,
      routine: false,
      settled: true,
      failed: true,
    });
    expect(useStore.getState().prefs.notify.on).toBe(true);
    expect(switchOn(ROUTINE).textContent).toBe("Off");
  });

  it("says so when the machine refuses a test notification", async () => {
    notifyOperator.mockResolvedValue(false);
    open();
    pane("Notifications");
    fireEvent.click(screen.getByRole("button", { name: "Send a test notification" }));

    // A refused permission and a working one look identical from in here, which
    // is the only reason this button exists.
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
    // Neutral, not the success tone, and that is the point. On desktop there is
    // no per-app grant for the plugin to read, so a machine with notifications
    // switched off accepts every one of these and shows none: a green "it
    // worked" would be the one message in the pane that cannot be checked.
    expect(banner().className).not.toContain("banner--ok");
    expect(banner().className).not.toContain("banner--error");
    expect(screen.getByText(/Handed to the operating system/)).toBeTruthy();
  });
});

describe("shortcuts", () => {
  it("lists every key the app answers to", () => {
    open();
    pane("Shortcuts");

    // Discoverability is the whole feature: a binding this panel omits is a
    // shortcut the operator does not have.
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
  it("says which commit this build was made from", () => {
    open();
    pane("About");

    // The value is baked in at build time, so the assertion is on the shape:
    // a short hash, dirty or not, or the dash a build with no repository behind
    // it draws. Nothing here is read from a host, so nothing can fail.
    expect(document.querySelector(".about__version")?.textContent).toMatch(
      /^Version (—|[0-9a-f]{7,}(-dirty)?)$/,
    );
  });
});
