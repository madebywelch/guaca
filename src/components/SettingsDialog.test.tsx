import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Prefs } from "../lib/prefs";
import type { GuardLimits, Settings, SettingsPatch } from "../lib/types";
import type { Section } from "./SettingsDialog";

/**
 * The settings dialog, over a mocked runtime.
 *
 * Eight panes, and every field of every one of them held by the shell. That is
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
    apiKeySet: true,
    apiKeyHint: "…9f2c",
    requestTimeoutSecs: 120,
    limits: { ...HELD },
    ...over,
  };
}

const updateSettings = vi.fn<(patch: SettingsPatch) => Promise<Settings>>(async () => stored());
const testConnection = vi.fn<(patch?: SettingsPatch) => Promise<string>>(async () => "Reached it.");
const notifyOperator = vi.fn<(title: string, body: string) => Promise<boolean>>(async () => true);
const getVersion = vi.fn<() => Promise<string>>(async () => "0.4.2");

vi.mock("../lib/ipc", () => ({
  api: {
    updateSettings: (patch: SettingsPatch) => updateSettings(patch),
    testConnection: (patch?: SettingsPatch) => testConnection(patch),
  },
  notifyOperator: (title: string, body: string) => notifyOperator(title, body),
}));

// The About pane reads the version through a dynamic import, so the module has
// to answer here or the whole tree comes down on a webview with no Tauri host.
vi.mock("@tauri-apps/api/app", () => ({ getVersion: () => getVersion() }));

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
  // No Tauri host behind this webview, which is the state the About pane is
  // built to shrug off. The one test that wants a version asks for one.
  getVersion.mockRejectedValue(new Error("no host"));
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
    // On Provider so both of the buttons a press could double up are on screen.
    open(stored(), DEFAULT_PREFS, "provider");
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
    fireEvent.click(save());

    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    const patch = sentPatch();
    // Absent, not empty and not zero. An empty `apiKey` clears the stored key,
    // and a zero timeout or sleep timer is a setting nobody chose.
    expect("apiKey" in patch).toBe(false);
    expect("e2bApiKey" in patch).toBe(false);
    expect("computerIdleMinutes" in patch).toBe(false);
    expect("requestTimeoutSecs" in patch).toBe(false);
    expect(patch).toEqual({
      operatorName: "Robert",
      baseUrl: "https://openrouter.ai/api/v1",
      defaultModel: "anthropic/claude-sonnet-4.5",
      limits: HELD,
    });
  });

  it("does not read a box of spaces as a key", async () => {
    open();
    pane("Provider");
    type(/^API key/, "   ");
    pane("Machines");
    type(/^E2B API key/, " ");
    fireEvent.click(save());

    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect("apiKey" in sentPatch()).toBe(false);
    expect("e2bApiKey" in sentPatch()).toBe(false);
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
    pane("General");
    type(/^Your name/, "Robert");

    fireEvent.click(save());
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(sentPatch()).toEqual({
      operatorName: "Robert",
      baseUrl: "http://localhost:1234/v1",
      defaultModel: "qwen3-coder-30b",
      apiKey: "sk-or-v1-typed",
      e2bApiKey: "e2b_typed",
      computerIdleMinutes: 45,
      requestTimeoutSecs: 30,
      limits: HELD,
    });
  });

  it("clears the API key box afterwards and leaves every other box alone", async () => {
    open();
    pane("Provider");
    type(/^API key/, "sk-or-v1-typed");
    type(/^Give up on a call after/, "30");
    pane("Machines");
    type(/^E2B API key/, "e2b_typed");
    type(/^Sleep computers after/, "45");

    updateSettings.mockResolvedValue(stored({ apiKeySet: true, apiKeyHint: "…yped" }));
    fireEvent.click(save());
    await waitFor(() => expect(screen.getByText("Saved.")).toBeTruthy());

    // The asymmetry is asserted as it stands: one secret is dropped from the
    // webview and the other is left on screen.
    expect(field(/^E2B API key/).value).toBe("e2b_typed");
    expect(field(/^Sleep computers after/).value).toBe("45");
    pane("Provider");
    expect(field(/^API key/).value).toBe("");
    expect(field(/^Give up on a call after/).value).toBe("30");
  });

  it("stays open on a save, because saving is not finishing", async () => {
    open();
    fireEvent.click(save());
    await waitFor(() => expect(screen.getByText("Saved.")).toBeTruthy());
    expect(banner().className).toContain("banner--ok");
    expect(onClose).not.toHaveBeenCalled();
    expect(useStore.getState().settings?.operatorName).toBe("Robert");
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
      baseUrl: "https://api.groq.com/openai/v1",
      defaultModel: "anthropic/claude-sonnet-4.5",
      apiKey: "gsk_typed",
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

    // A hand-typed endpoint is chosen, not unrecognised, so nothing is marked.
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

  it("sends something recognisable when the machine allows it", async () => {
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
  it("shows a dash rather than a banner when the version cannot be read", async () => {
    open();
    pane("About");

    await waitFor(() => expect(getVersion).toHaveBeenCalledTimes(1));
    expect(screen.getByText("Version —")).toBeTruthy();
    expect(document.querySelector(".banner")).toBeNull();
  });

  it("shows the version once the app reports one", async () => {
    getVersion.mockResolvedValueOnce("0.4.2");
    open();
    pane("About");

    expect(await screen.findByText("Version 0.4.2")).toBeTruthy();
  });
});
