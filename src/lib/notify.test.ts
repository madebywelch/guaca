import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Delivery, DeliveryId, DeliveryTarget, NotificationAccess, Reveal } from "./types";

/**
 * What a window does with what the Host offers: claim it, and show it only if
 * the claim won.
 *
 * Every decision about whether something is worth an interruption is on the
 * Host now, and tested there (`domain/outbox.rs` for the rule, and
 * `tests/outbox.rs` for the dispatcher and the claim). What is left to get
 * wrong here is the order of three calls and what a click does, and each way
 * of getting it wrong is concrete: a banner shown by a window that lost the
 * claim is the same banner twice, and a claim made by a page that cannot show
 * anything is a banner nobody sees.
 *
 * The transport is the real one, so the browser half runs through the same
 * `Notification` calls a browser client on a remote Host makes. Only the Host
 * is faked, at `ipc`, and the operating system, at `Notification` and at the
 * Tauri plugin.
 */

const host = vi.hoisted(() => ({
  claimDelivery: vi.fn<(id: string) => Promise<boolean>>(),
  pendingDeliveries: vi.fn<() => Promise<Delivery[]>>(),
  openWindow: vi.fn<(target: Reveal | null) => Promise<void>>(),
}));
vi.mock("./ipc", () => ({ api: host }));

const plugin = vi.hoisted(() => ({
  isPermissionGranted: vi.fn(async () => true),
  requestPermission: vi.fn(async () => "granted"),
  sendNotification: vi.fn<(options: { id?: number; title: string; body?: string }) => void>(),
  onAction: vi.fn<(cb: (action: unknown) => void) => Promise<{ unregister: () => void }>>(),
}));
vi.mock("@tauri-apps/plugin-notification", () => plugin);

/**
 * The app's own reading of macOS: `now` is what it would say without asking,
 * and `answer` is what the operator says when it does ask. `prompts` counts the
 * questions macOS actually put, and `held`, when set, keeps one on screen
 * until the test answers it. `null` is every other desktop, where the plugin
 * is asked instead.
 */
const mac = vi.hoisted(() => ({
  now: null as NotificationAccess | null,
  answer: "allowed" as NotificationAccess,
  prompts: 0,
  held: null as Promise<void> | null,
  invoke: vi.fn(async (name: string): Promise<unknown> => {
    if (name === "notification_access") return mac.now;
    if (name === "ask_notification_access") {
      if (mac.now === "notAsked") {
        mac.prompts++;
        await mac.held;
        mac.now = mac.answer;
      }
      return mac.now;
    }
    throw new Error(`unexpected command ${name}`);
  }),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mac.invoke }));

/** A browser's notification center, as far as a page can see it. */
class Shown {
  static permission: NotificationPermission = "granted";
  static answer: NotificationPermission = "granted";
  static requestPermission = vi.fn(async () => {
    Shown.permission = Shown.answer;
    return Shown.answer;
  });
  static all: Shown[] = [];
  onclick: (() => void) | null = null;
  close = vi.fn();
  constructor(
    readonly title: string,
    readonly options: { body?: string },
  ) {
    Shown.all.push(this);
  }
}

function delivery(id: DeliveryId, over: Partial<Delivery> = {}): Delivery {
  return {
    id,
    kind: "needsYou",
    title: `Title ${id}`,
    body: `Body ${id}`,
    target: { kind: "channel", agentId: `agent-${id}` },
    createdAt: 1_756_000_000_000,
    ...over,
  };
}

/** What a click asked for, through the same `land` the window wires up. */
let revealed: Reveal[] = [];
type Notify = typeof import("./notify");
let notify: Notify;
const open = (target: DeliveryTarget) => notify.land(target, (where) => revealed.push(where));

beforeEach(async () => {
  vi.resetModules();
  Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
  notify = await import("./notify");
  revealed = [];
  Shown.all = [];
  Shown.permission = "granted";
  Shown.answer = "granted";
  vi.stubGlobal("Notification", Shown);
  vi.spyOn(window, "focus").mockImplementation(() => {});
  host.claimDelivery.mockReset().mockResolvedValue(true);
  host.pendingDeliveries.mockReset().mockResolvedValue([]);
  host.openWindow.mockReset().mockResolvedValue(undefined);
  Shown.requestPermission.mockClear();
  mac.now = null;
  mac.answer = "allowed";
  mac.prompts = 0;
  mac.held = null;
  mac.invoke.mockClear();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("a delivery the Host offered", () => {
  it("is claimed once, and shown only when the claim wins", async () => {
    await expect(notify.deliver(delivery("a"), open)).resolves.toBe(true);
    expect(host.claimDelivery.mock.calls).toEqual([["a"]]);
    expect(Shown.all.map((shown) => [shown.title, shown.options.body])).toEqual([
      ["Title a", "Body a"],
    ]);

    // Another window won this one. Its banner is that window's to show.
    host.claimDelivery.mockResolvedValueOnce(false);
    await expect(notify.deliver(delivery("b"), open)).resolves.toBe(false);
    expect(host.claimDelivery).toHaveBeenLastCalledWith("b");
    expect(Shown.all).toHaveLength(1);
  });

  it("says what the Host wrote, word for word", async () => {
    // Titles are the Host's: "has a question", never "needs your permission".
    const asked = delivery("q", { title: "Ada has a question", body: "Which branch?" });
    await notify.deliver(asked, open);
    expect(Shown.all[0]?.title).toBe("Ada has a question");
    expect(Shown.all[0]?.options.body).toBe("Which branch?");
  });

  it("is lost, not shown, when the claim cannot reach the Host", async () => {
    host.claimDelivery.mockRejectedValueOnce({ kind: "unreachable", message: "gone" });
    await expect(notify.deliver(delivery("a"), open)).resolves.toBe(false);
    expect(Shown.all).toHaveLength(0);
  });
});

describe("a browser client on a remote Host", () => {
  it("claims nothing while it may not notify, so a window that can will show it", async () => {
    Shown.permission = "denied";
    Shown.answer = "denied";
    await expect(notify.deliver(delivery("a"), open)).resolves.toBe(false);
    expect(host.claimDelivery).not.toHaveBeenCalled();
    expect(Shown.all).toHaveLength(0);
  });

  it("asks once for a burst of offers, then claims and shows each", async () => {
    Shown.permission = "default";
    await Promise.all([notify.deliver(delivery("a"), open), notify.deliver(delivery("b"), open)]);
    expect(Shown.requestPermission).toHaveBeenCalledTimes(1);
    expect(host.claimDelivery.mock.calls.flat().sort()).toEqual(["a", "b"]);
    expect(Shown.all).toHaveLength(2);
  });

  it("claims nothing when the question is left unanswered", async () => {
    // What a browser that will not prompt without a click answers.
    Shown.permission = "default";
    Shown.answer = "default";
    await expect(notify.deliver(delivery("a"), open)).resolves.toBe(false);
    expect(host.claimDelivery).not.toHaveBeenCalled();
  });

  it("has nowhere to show anything without a Notification at all", async () => {
    vi.stubGlobal("Notification", undefined);
    await expect(notify.deliver(delivery("a"), open)).resolves.toBe(false);
    expect(host.claimDelivery).not.toHaveBeenCalled();
  });

  it("brings the tab forward on a click and selects the channel it is about", async () => {
    await notify.deliver(delivery("a"), open);
    Shown.all[0]?.onclick?.();
    expect(window.focus).toHaveBeenCalled();
    expect(Shown.all[0]?.close).toHaveBeenCalled();
    expect(revealed).toEqual([{ kind: "agent", id: "agent-a" }]);
    expect(host.openWindow).not.toHaveBeenCalled();
  });

  it("opens For you on a click on something about no channel", async () => {
    await notify.deliver(delivery("r", { kind: "reminder", target: { kind: "forYou" } }), open);
    Shown.all[0]?.onclick?.();
    expect(revealed).toEqual([{ kind: "forYou" }]);
  });
});

describe("the desktop window", () => {
  let tapped: ((action: unknown) => void) | undefined;

  beforeEach(async () => {
    vi.resetModules();
    Object.defineProperty(window, "__TAURI_INTERNALS__", { configurable: true, value: {} });
    notify = await import("./notify");
    tapped = undefined;
    plugin.sendNotification.mockReset();
    plugin.onAction.mockReset().mockImplementation(async (cb) => {
      tapped = cb;
      return { unregister: () => {} };
    });
  });

  it("listens for clicks before it raises anything, and raises with an id to find it by", async () => {
    await notify.deliver(delivery("a"), open);
    expect(plugin.onAction).toHaveBeenCalledTimes(1);
    expect(plugin.onAction.mock.invocationCallOrder[0]).toBeLessThan(
      plugin.sendNotification.mock.invocationCallOrder[0] ?? 0,
    );
    expect(plugin.sendNotification).toHaveBeenCalledWith(
      expect.objectContaining({ id: expect.any(Number), title: "Title a", body: "Body a" }),
    );

    await notify.deliver(delivery("b"), open);
    expect(plugin.onAction).toHaveBeenCalledTimes(1);
  });

  it("asks the app to raise the window and land on the channel when one is clicked", async () => {
    await notify.deliver(delivery("a"), open);
    await notify.deliver(delivery("b"), open);
    const [first, second] = plugin.sendNotification.mock.calls.map(([options]) => options.id);
    expect(first).not.toBe(second);

    tapped?.({ actionId: "tap", inputValue: null, notification: { id: second } });
    expect(host.openWindow.mock.calls).toEqual([[{ kind: "agent", id: "agent-b" }]]);
    // The app emits the reveal once the window is up; landing here as well
    // would select before the window is there to show it.
    expect(revealed).toEqual([]);

    // Once only, and nothing for a notification this page did not raise.
    tapped?.({ actionId: "tap", inputValue: null, notification: { id: second } });
    tapped?.({ actionId: "tap", inputValue: null, notification: { id: 12 } });
    tapped?.({ actionId: "tap", inputValue: null, notification: null });
    expect(host.openWindow).toHaveBeenCalledTimes(1);
  });

  it("still lands somewhere when the app cannot raise the window", async () => {
    host.openWindow.mockRejectedValueOnce(new Error("no tray"));
    vi.spyOn(console, "warn").mockImplementation(() => {});
    await notify.deliver(delivery("a", { target: { kind: "forYou" } }), open);
    const [options] = plugin.sendNotification.mock.calls[0] ?? [];
    tapped?.({ actionId: "tap", inputValue: null, notification: { id: options?.id } });
    await vi.waitFor(() => expect(revealed).toEqual([{ kind: "forYou" }]));
  });
});

describe("a desktop window on a Mac", () => {
  beforeEach(async () => {
    vi.resetModules();
    Object.defineProperty(window, "__TAURI_INTERNALS__", { configurable: true, value: {} });
    notify = await import("./notify");
    plugin.isPermissionGranted.mockClear();
    plugin.sendNotification.mockReset();
    plugin.onAction.mockReset().mockResolvedValue({ unregister: () => {} });
  });

  it("claims nothing and raises nothing when macOS has notifications off for Guaca", async () => {
    mac.now = "denied";
    await expect(notify.deliver(delivery("a"), open)).resolves.toBe(false);
    expect(host.claimDelivery).not.toHaveBeenCalled();
    expect(plugin.sendNotification).not.toHaveBeenCalled();
    // The plugin answers yes on every desktop, so it is not the one asked.
    expect(plugin.isPermissionGranted).not.toHaveBeenCalled();
  });

  it("says false, and raises nothing, for a test macOS will not show", async () => {
    mac.now = "denied";
    const transport = await import("./transport");
    await expect(transport.notify("Guaca", "A test.")).resolves.toBe(false);
    expect(plugin.sendNotification).not.toHaveBeenCalled();
  });

  it("raises what it claimed when macOS allows it", async () => {
    mac.now = "allowed";
    await expect(notify.deliver(delivery("a"), open)).resolves.toBe(true);
    expect(host.claimDelivery).toHaveBeenCalledWith("a");
    expect(plugin.sendNotification).toHaveBeenCalledWith(
      expect.objectContaining({ title: "Title a", body: "Body a" }),
    );
  });

  it("asks macOS before it claims the first offer, and not on connect", async () => {
    mac.now = "notAsked";
    await expect(notify.catchUp(open)).resolves.toBe(0);
    expect(mac.prompts).toBe(0);

    host.claimDelivery.mockImplementation(async () => {
      // The answer is in before the claim is made.
      expect(mac.now).toBe("allowed");
      return true;
    });
    await expect(notify.deliver(delivery("a"), open)).resolves.toBe(true);
    expect(mac.prompts).toBe(1);
    expect(host.claimDelivery).toHaveBeenCalledWith("a");
    expect(plugin.sendNotification).toHaveBeenCalledTimes(1);
  });

  it("claims nothing when the first answer is no, leaving the offer to the Host", async () => {
    mac.now = "notAsked";
    mac.answer = "denied";
    await expect(notify.deliver(delivery("a"), open)).resolves.toBe(false);
    await expect(notify.deliver(delivery("b"), open)).resolves.toBe(false);
    expect(mac.prompts).toBe(1);
    expect(host.claimDelivery).not.toHaveBeenCalled();
    expect(plugin.sendNotification).not.toHaveBeenCalled();
  });

  it("puts one prompt for offers that arrive while it is on screen", async () => {
    mac.now = "notAsked";
    let answer = () => {};
    mac.held = new Promise((resolve) => {
      answer = resolve;
    });
    const shown = Promise.all(["a", "b", "c"].map((id) => notify.deliver(delivery(id), open)));
    await vi.waitFor(() => expect(mac.prompts).toBe(1));
    expect(host.claimDelivery).not.toHaveBeenCalled();

    answer();
    // Each one raised: `notify` answers true only once it has. Not counted on
    // the plugin's mock, because vitest hands concurrent dynamic imports of a
    // mocked module the real one, which raises through `Notification`.
    await expect(shown).resolves.toEqual([true, true, true]);
    expect(mac.prompts).toBe(1);
    expect(host.claimDelivery.mock.calls).toEqual([["a"], ["b"], ["c"]]);
  });

  it("raises nothing when another window claimed it while the prompt was up", async () => {
    mac.now = "notAsked";
    host.claimDelivery.mockResolvedValue(false);
    await expect(notify.deliver(delivery("a"), open)).resolves.toBe(false);
    expect(mac.prompts).toBe(1);
    expect(plugin.sendNotification).not.toHaveBeenCalled();
  });

  it("asks before claiming what waited for it on connect, and claims nothing on no", async () => {
    mac.now = "notAsked";
    mac.answer = "denied";
    host.pendingDeliveries.mockResolvedValue([delivery("a"), delivery("b")]);
    await expect(notify.catchUp(open)).resolves.toBe(0);
    expect(mac.prompts).toBe(1);
    expect(host.claimDelivery).not.toHaveBeenCalled();
    expect(plugin.sendNotification).not.toHaveBeenCalled();
  });

  it("asks directly for the test button, and raises it on yes", async () => {
    mac.now = "notAsked";
    const transport = await import("./transport");
    await expect(transport.notify("Guaca", "A test.")).resolves.toBe(true);
    expect(mac.prompts).toBe(1);
    expect(plugin.sendNotification).toHaveBeenCalledTimes(1);
  });

  it("shows nothing from a copy that is not running as an app", async () => {
    mac.now = "unavailable";
    await expect(notify.deliver(delivery("a"), open)).resolves.toBe(false);
    expect(host.claimDelivery).not.toHaveBeenCalled();
    expect(plugin.sendNotification).not.toHaveBeenCalled();
  });
});

describe("what was offered while this page was not connected", () => {
  it("folds four into one banner that opens For you", async () => {
    host.pendingDeliveries.mockResolvedValue(["a", "b", "c", "d"].map((id) => delivery(id)));
    await expect(notify.catchUp(open)).resolves.toBe(4);

    expect(host.claimDelivery.mock.calls).toEqual([["a"], ["b"], ["c"], ["d"]]);
    expect(Shown.all).toHaveLength(1);
    expect(Shown.all[0]?.title).toBe("While you were away");
    expect(Shown.all[0]?.options.body).toBe("Title a\nTitle b\nTitle c\nAnd 1 more in For you.");
    Shown.all[0]?.onclick?.();
    expect(revealed).toEqual([{ kind: "forYou" }]);
  });

  it("shows three or fewer one by one, each opening what it is about", async () => {
    host.pendingDeliveries.mockResolvedValue(["a", "b", "c"].map((id) => delivery(id)));
    await notify.catchUp(open);
    expect(Shown.all.map((shown) => shown.title)).toEqual(["Title a", "Title b", "Title c"]);
    Shown.all[1]?.onclick?.();
    expect(revealed).toEqual([{ kind: "agent", id: "agent-b" }]);
  });

  it("counts only what this window won", async () => {
    host.pendingDeliveries.mockResolvedValue(["a", "b", "c", "d", "e"].map((id) => delivery(id)));
    host.claimDelivery.mockImplementation(async (id) => id !== "b" && id !== "d");
    await expect(notify.catchUp(open)).resolves.toBe(3);
    expect(Shown.all.map((shown) => shown.title)).toEqual(["Title a", "Title c", "Title e"]);
  });

  it("asks the Host for nothing on a page that may not notify", async () => {
    Shown.permission = "denied";
    Shown.answer = "denied";
    await expect(notify.catchUp(open)).resolves.toBe(0);
    expect(host.pendingDeliveries).not.toHaveBeenCalled();
    expect(host.claimDelivery).not.toHaveBeenCalled();
  });

  it("shows nothing when the Host cannot say what is waiting", async () => {
    host.pendingDeliveries.mockRejectedValue({ kind: "unknownCommand", message: "too old" });
    await expect(notify.catchUp(open)).resolves.toBe(0);
    expect(Shown.all).toHaveLength(0);
  });
});

describe("the menu bar panel", () => {
  // A second page on the same Host, offered every delivery the window is. It
  // must never claim one, or a banner would be taken away from the window and
  // shown by a page that cannot show it (docs/gotchas/menubar.md).
  const read = (path: string) => readFileSync(resolve(__dirname, "../..", path), "utf8");

  it("never reaches the code that claims or shows a delivery", () => {
    for (const page of ["src/menubar.tsx", "src/components/MenubarPanel.tsx"]) {
      expect(read(page)).not.toMatch(/lib\/notify|claimDelivery|useReportView/);
    }
  });

  it("is granted no notification command to show one with", () => {
    const capability = JSON.parse(read("src-tauri/capabilities/menubar.json")) as {
      permissions: string[];
    };
    expect(capability.permissions.filter((one) => one.startsWith("notification:"))).toEqual([]);
  });
});
