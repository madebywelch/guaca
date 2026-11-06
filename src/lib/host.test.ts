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
      "Check that Docker is running: this workspace is a container on this Mac.",
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
