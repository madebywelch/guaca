import { afterEach, describe, expect, it, vi } from "vitest";

/**
 * What to check when the host does not answer, in the words of the workspace
 * this page shows.
 *
 * Three arrangements and three answers, and they are three files here because
 * the module reads which host it is in once, on import: the setup file gives
 * jsdom Tauri's bridge, so the desktop is what a static import of `./host`
 * finds. A browser is the same import with the bridge taken off, and the
 * desktop pointed at a box is a box on record when it is made.
 */
const REMOTE = "guaca.workspace.remote";
/** The bridge the setup file gives every suite, put back after a test takes it. */
const BRIDGE = (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;

afterEach(() => {
  (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = BRIDGE;
  window.localStorage.removeItem(REMOTE);
  window.localStorage.removeItem("guaca.workspace.hostMode");
  vi.resetModules();
});

describe("what to check when nothing answers", () => {
  it("is Docker, for a desktop showing its own workspace", async () => {
    window.localStorage.removeItem(REMOTE);
    vi.resetModules();
    const { unreachableHint } = await import("./host");
    expect(unreachableHint()).toBe(
      "Check that Docker is running: Guaca runs in a container on this Mac.",
    );
  });

  it("is the box, for a window pointed at one", async () => {
    window.localStorage.setItem(
      REMOTE,
      JSON.stringify({ origin: "https://box.example:8787", token: "t" }),
    );
    vi.resetModules();
    const { unreachableHint } = await import("./host");
    expect(unreachableHint()).toBe(
      "Check that the box is up, and that its address or tunnel still reaches it.",
    );
  });

  it("is the host, for a browser, which cannot tell a box from a container", async () => {
    // A browser only knows the address it was opened with: naming Docker for a
    // server in another country is the wrong afternoon for somebody.
    delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    vi.resetModules();
    const { unreachableHint } = await import("./host");
    expect(unreachableHint()).toBe(
      "Check that the host is up, and that this address still answers from here.",
    );
  });
});

describe("which host the bar names, and whether its build agrees", () => {
  const health = (version: string, build = "a".repeat(40)) => ({
    service: "guacad" as const,
    build,
    version,
  });

  it("is this Mac for a window running its own workspace", async () => {
    window.localStorage.removeItem(REMOTE);
    vi.resetModules();
    const { hostPlace } = await import("./host");
    expect(hostPlace()).toBe("thisMac");
  });

  it("is a box only when the chooser says the host is remote", async () => {
    // The desktop reaches a container on this Mac and a box by the same kind
    // of address, so the stored mode is the only thing that can tell them
    // apart; the address alone would call Docker a server.
    window.localStorage.setItem(
      REMOTE,
      JSON.stringify({ origin: "https://box.example:8787", token: "t" }),
    );
    window.localStorage.setItem("guaca.workspace.hostMode", "remote");
    vi.resetModules();
    const remote = await import("./host");
    expect(remote.hostPlace()).toBe("remote");
  });

  it("is still this Mac for a container here, which attaches by an address too", async () => {
    window.localStorage.setItem(
      REMOTE,
      JSON.stringify({ origin: "http://127.0.0.1:54321", token: "t" }),
    );
    window.localStorage.setItem("guaca.workspace.hostMode", "local");
    vi.resetModules();
    const { hostPlace } = await import("./host");
    expect(hostPlace()).toBe("thisMac");
  });

  it("is the host for a browser, which cannot tell one from the other", async () => {
    delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    vi.resetModules();
    const { hostPlace } = await import("./host");
    expect(hostPlace()).toBe("browser");
  });

  it("says the drift in words, because a color is not a difference everybody reads", async () => {
    const { hostLabel } = await import("./host");
    // The same version on two commits is the case the bar used to draw as a
    // bare hash beside a version that otherwise read "same".
    const other = hostLabel({
      place: "thisMac",
      drift: "otherBuild",
      host: health("0.2.0", "c".repeat(40)),
    });
    expect(other.text).toBe("This Mac · other build");
    expect(other.drift).toBe(true);
    expect(other.title).toBe(
      "This Mac runs Guaca 0.2.0, a different build from this app. Open Workspace settings to see which.",
    );
    expect(
      hostLabel({
        place: "remote",
        drift: "clientBehind",
        host: health("0.3.0"),
        appVersion: "0.2.0",
      }).text,
    ).toBe("Remote host · app older");
    expect(
      hostLabel({
        place: "remote",
        drift: "hostBehind",
        host: health("0.1.0"),
        appVersion: "0.2.0",
      }).text,
    ).toBe("Remote host · host older");
  });

  it("leaves the version to Workspace settings, and keeps it in the hover", async () => {
    const { hostLabel } = await import("./host");
    const same = hostLabel({ place: "thisMac", drift: "same", host: health("0.2.0") });
    expect(same.text).toBe("This Mac");
    expect(same.drift).toBe(false);
    expect(same.title).toBe("This Mac runs Guaca 0.2.0, the same build as this app.");
    // A host old enough to answer without a version says nothing it does not
    // know, and never words a drift it could not compare.
    const old = hostLabel({
      place: "thisMac",
      drift: "unknown",
      host: { service: "guacad", build: "b" },
    });
    expect(old.text).toBe("This Mac");
    expect(old.title).toBe(
      "Which host this window shows. Open Workspace settings for its updates.",
    );
  });
});
