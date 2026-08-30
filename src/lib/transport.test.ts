import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * The transport, in the host this app is not usually tested in.
 *
 * The setup file gives jsdom Tauri's bridge so every other suite draws the
 * desktop, and the module decides which host it is in by reading `window`
 * once, on import. So the bridge comes off first and the import comes after:
 * everything below is the hosted path, and the desktop half is one `import()`
 * of Tauri's own API with nothing of its own to test. What is worth checking
 * is the two things a browser does with a token that a webview never has to:
 * where it takes one from, and what it says when the box stops accepting it.
 */

delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
const { adoptInvitation, invoke, setToken, token, UNAUTHORIZED_EVENT } = await import(
  "./transport"
);

const fetched = vi.fn<typeof fetch>();

beforeEach(() => {
  setToken("");
  window.history.replaceState(null, "", "/");
  fetched.mockReset();
  vi.stubGlobal("fetch", fetched);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("an invitation", () => {
  it("is taken out of the address bar and kept", () => {
    window.history.replaceState(null, "", "/#token=abc123");

    expect(adoptInvitation()).toBe(true);
    expect(token()).toBe("abc123");
    // The credential must not survive in the URL: a reload, a duplicated tab
    // or a link dragged to somebody else would carry it.
    expect(window.location.hash).toBe("");
    expect(window.location.pathname).toBe("/");
  });

  it("leaves a stored token alone when there is nothing in the address bar", () => {
    setToken("kept");
    expect(adoptInvitation()).toBe(false);
    expect(token()).toBe("kept");
  });

  it("ignores a fragment that is not a token", () => {
    window.history.replaceState(null, "", "/#settings");
    expect(adoptInvitation()).toBe(false);
    expect(token()).toBe("");
    expect(window.location.hash).toBe("#settings");
  });
});

describe("a call", () => {
  it("carries the token as a bearer and unwraps the answer", async () => {
    setToken("abc123");
    fetched.mockResolvedValue(new Response(JSON.stringify({ ok: { fine: true } })));

    await expect(invoke("capabilities")).resolves.toEqual({ fine: true });

    const [url, init] = fetched.mock.calls[0]!;
    expect(String(url)).toBe(`${window.location.origin}/v1/call`);
    const headers = init!.headers as Record<string, string>;
    expect(headers.authorization).toBe("Bearer abc123");
    expect(JSON.parse(String(init!.body))).toEqual({ name: "capabilities", args: {} });
  });

  it("says once, on the window, that the token was turned away", async () => {
    // One event rather than forty banners: a token rotated on the box turns
    // every call away at once, and the answer is one screen asking again.
    const heard = vi.fn();
    window.addEventListener(UNAUTHORIZED_EVENT, heard);
    fetched.mockResolvedValue(
      new Response(JSON.stringify({ err: { kind: "unauthorized", message: "needs the token" } }), {
        status: 401,
      }),
    );

    await expect(invoke("capabilities")).rejects.toEqual({
      kind: "unauthorized",
      message: "needs the token",
    });
    expect(heard).toHaveBeenCalledTimes(1);
    window.removeEventListener(UNAUTHORIZED_EVENT, heard);
  });

  it("does not raise that event for any other refusal", async () => {
    const heard = vi.fn();
    window.addEventListener(UNAUTHORIZED_EVENT, heard);
    fetched.mockResolvedValue(
      new Response(JSON.stringify({ err: { kind: "notFound", message: "no such agent" } }), {
        status: 404,
      }),
    );

    await expect(invoke("agent_memory", { id: "x" })).rejects.toEqual({
      kind: "notFound",
      message: "no such agent",
    });
    expect(heard).not.toHaveBeenCalled();
    window.removeEventListener(UNAUTHORIZED_EVENT, heard);
  });

  it("names a box that cannot be reached as its own kind of failure", async () => {
    // The one failure where the thing to do is wait: the crew keeps working
    // while nobody can see it, and nothing about the settings is wrong.
    fetched.mockRejectedValue(new TypeError("Failed to fetch"));

    await expect(invoke("capabilities")).rejects.toMatchObject({ kind: "unreachable" });
    await expect(invoke("capabilities")).rejects.toMatchObject({
      message: expect.stringContaining("agents keep working"),
    });
  });
});
