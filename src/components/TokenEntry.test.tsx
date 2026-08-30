import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

/**
 * The screen a browser sees before the workspace.
 *
 * Run hosted, which takes arranging: the setup file gives jsdom Tauri's bridge
 * so every other suite draws the desktop, and the transport reads `window`
 * once on import to decide which host it is in. The bridge comes off before
 * the import, and this is the only host the component draws anything in. What
 * matters is the order of decisions: a clicked invitation must never flash
 * the form, a wrong token must be refused here rather than by the app
 * underneath, and a token the box stops accepting must bring the form back
 * with the app unmounted.
 */

const capabilities = vi.fn<() => Promise<unknown>>();

vi.mock("../lib/ipc", () => ({
  api: { capabilities: () => capabilities() },
}));

delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
const { TokenEntry } = await import("./TokenEntry");
const { setToken, token, UNAUTHORIZED_EVENT } = await import("../lib/transport");

const EVERYTHING = {
  localDirectories: true,
  loopbackEndpoints: true,
  claudeProvider: true,
  claudeCodeHarness: true,
  localFiles: true,
};

function open() {
  return render(
    <TokenEntry>
      <p>the workspace</p>
    </TokenEntry>,
  );
}

beforeEach(() => {
  setToken("");
  window.history.replaceState(null, "", "/");
  capabilities.mockReset();
  capabilities.mockResolvedValue(EVERYTHING);
});

describe("TokenEntry", () => {
  it("asks for a token before it shows anything else", () => {
    open();
    expect(screen.getByLabelText("Workspace token")).toBeTruthy();
    expect(screen.queryByText("the workspace")).toBeNull();
    // Nothing is asked of the box until there is a token to ask with.
    expect(capabilities).not.toHaveBeenCalled();
  });

  it("lets a clicked invitation straight in", () => {
    window.history.replaceState(null, "", "/#token=abc123");
    open();
    expect(screen.getByText("the workspace")).toBeTruthy();
    expect(screen.queryByLabelText("Workspace token")).toBeNull();
    expect(token()).toBe("abc123");
    expect(window.location.hash).toBe("");
  });

  it("lets a browser that already has a token straight in", () => {
    setToken("kept");
    open();
    expect(screen.getByText("the workspace")).toBeTruthy();
  });

  it("checks a pasted token against the box before admitting it", async () => {
    open();
    fireEvent.change(screen.getByLabelText("Workspace token"), { target: { value: " abc123 " } });
    fireEvent.click(screen.getByRole("button", { name: "Open workspace" }));

    await waitFor(() => expect(screen.getByText("the workspace")).toBeTruthy());
    expect(capabilities).toHaveBeenCalledTimes(1);
    expect(token()).toBe("abc123");
  });

  it("refuses a wrong token here, and forgets it", async () => {
    capabilities.mockRejectedValue({
      kind: "unauthorized",
      message: "this workspace needs the token it printed when it started",
    });
    open();
    fireEvent.change(screen.getByLabelText("Workspace token"), { target: { value: "wrong" } });
    fireEvent.click(screen.getByRole("button", { name: "Open workspace" }));

    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("needs the token"));
    expect(screen.queryByText("the workspace")).toBeNull();
    // A refused token is not kept: the next reload would only refuse it again,
    // with the app's own reads failing forty times before this form appeared.
    expect(token()).toBe("");
  });

  it("does not send an empty box", () => {
    open();
    const button = screen.getByRole("button", { name: "Open workspace" }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    fireEvent.submit(button.closest("form")!);
    expect(capabilities).not.toHaveBeenCalled();
  });

  it("brings the form back when the box stops accepting the token it had", async () => {
    setToken("rotated-away");
    open();
    expect(screen.getByText("the workspace")).toBeTruthy();

    window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));

    await waitFor(() => expect(screen.getByLabelText("Workspace token")).toBeTruthy());
    expect(screen.queryByText("the workspace")).toBeNull();
    expect(screen.getByRole("alert").textContent).toContain("stopped accepting");
  });
});
