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
const { adoptInvitation, invoke, probe, setToken, token, UNAUTHORIZED_EVENT, upload } =
  await import("./transport");

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

describe("a window pointed at a box", () => {
  // The module reads the arrangement once, on import, so this is a second
  // import with the bridge present and a box on record: the third host.
  it("sends its calls to the box, with the box's token, and files from the box", async () => {
    vi.resetModules();
    (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    window.localStorage.setItem(
      "guaca.workspace.remote",
      JSON.stringify({ origin: "http://box.example:8787/", token: "box-token" }),
    );
    try {
      const boxed = await import("./transport");
      expect(boxed.hosted).toBe(true);
      expect(boxed.attached()).toEqual({ origin: "http://box.example:8787", token: "box-token" });
      expect(boxed.token()).toBe("box-token");
      expect(boxed.workspaceOrigin()).toBe("http://box.example:8787");

      fetched.mockResolvedValue(new Response(JSON.stringify({ ok: 1 })));
      await boxed.invoke("capabilities");
      const [url, init] = fetched.mock.calls[0]!;
      expect(String(url)).toBe("http://box.example:8787/v1/call");
      expect((init!.headers as Record<string, string>).authorization).toBe("Bearer box-token");

      // A rotated token stays with the box's record rather than the page's.
      boxed.setToken("rotated");
      expect(JSON.parse(window.localStorage.getItem("guaca.workspace.remote")!)).toEqual({
        origin: "http://box.example:8787",
        token: "rotated",
      });
    } finally {
      delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
      window.localStorage.removeItem("guaca.workspace.remote");
      vi.resetModules();
    }
  });

  it("is not pointed anywhere when the record is malformed", async () => {
    vi.resetModules();
    (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    window.localStorage.setItem("guaca.workspace.remote", "{not json");
    try {
      const boxed = await import("./transport");
      expect(boxed.attached()).toBeNull();
      expect(boxed.hosted).toBe(false);
    } finally {
      delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
      window.localStorage.removeItem("guaca.workspace.remote");
      vi.resetModules();
    }
  });
});

describe("probing a box", () => {
  it("accepts a box that answers as guacad and takes the token", async () => {
    fetched
      .mockResolvedValueOnce(new Response(JSON.stringify({ service: "guacad", build: "abc1234" })))
      .mockResolvedValueOnce(new Response(JSON.stringify({ ok: { localFiles: false } })));
    const found = await probe({ origin: "http://box.example/", token: "t" });
    expect(found.build).toBe("abc1234");
    expect(found.capabilities).toEqual({ localFiles: false });
    expect(String(fetched.mock.calls[0]![0])).toBe("http://box.example/health");
  });

  it("refuses something that answers but is not a workspace", async () => {
    fetched
      .mockResolvedValueOnce(new Response("<html>hello</html>"))
      .mockResolvedValueOnce(new Response("<html>hello</html>"));
    await expect(probe({ origin: "http://box.example", token: "t" })).rejects.toMatchObject({
      message: expect.stringContaining("not a Guaca workspace"),
    });
  });

  it("hands back the box's own refusal of a wrong token", async () => {
    fetched
      .mockResolvedValueOnce(new Response(JSON.stringify({ service: "guacad", build: "" })))
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({ err: { kind: "unauthorized", message: "needs the token" } }),
          {
            status: 401,
          },
        ),
      );
    await expect(probe({ origin: "http://box.example", token: "wrong" })).rejects.toEqual({
      kind: "unauthorized",
      message: "needs the token",
    });
  });
});

describe("an upload", () => {
  it("posts the bytes under the file's name with the token, and unwraps what was stored", async () => {
    setToken("abc123");
    fetched.mockResolvedValue(
      new Response(
        JSON.stringify({ ok: { digest: "d", name: "brief.txt", mime: "text/plain", bytes: 5 } }),
      ),
    );
    const file = new File(["hello"], "brief.txt", { type: "text/plain" });
    await expect(upload(file)).resolves.toMatchObject({ digest: "d", name: "brief.txt" });
    const [url, init] = fetched.mock.calls[0]!;
    expect(String(url)).toBe(`${window.location.origin}/v1/upload?name=brief.txt`);
    expect((init!.headers as Record<string, string>).authorization).toBe("Bearer abc123");
    expect(init!.body).toBe(file);
  });

  it("hands back the store's own sentence for a file it refused", async () => {
    fetched.mockResolvedValue(
      new Response(
        JSON.stringify({
          err: { kind: "file", message: "huge.bin is 30 bytes, and the limit is 25" },
        }),
        {
          status: 422,
        },
      ),
    );
    await expect(upload(new File(["x"], "huge.bin"))).rejects.toMatchObject({
      kind: "file",
      message: expect.stringContaining("the limit is"),
    });
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
