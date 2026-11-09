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
const {
  adoptInvitation,
  invoke,
  isUnreachable,
  probe,
  setToken,
  settleInvitation,
  token,
  UNAUTHORIZED_EVENT,
  upload,
} = await import("./transport");

const fetched = vi.fn<typeof fetch>();

beforeEach(async () => {
  // Stubbed before signing out: signing out is a request to the daemon now,
  // and a real `fetch` in jsdom would only fail against nothing listening.
  vi.stubGlobal("fetch", fetched);
  fetched.mockResolvedValue(new Response(JSON.stringify({ ok: "signed out" })));
  await setToken("");
  fetched.mockReset();
  fetched.mockRejectedValue(new TypeError("offline"));
  window.history.replaceState(null, "", "/");
  window.localStorage.clear();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("an invitation", () => {
  it("serializes cookie-setting exchanges so an older response cannot replace a newer session", async () => {
    let finish!: (response: Response) => void;
    fetched.mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    fetched.mockResolvedValueOnce(new Response(JSON.stringify({ ok: "signed in" })));
    const older = setToken("older");
    const newer = setToken("newer");
    await Promise.resolve();
    expect(fetched).toHaveBeenCalledTimes(1);
    expect((fetched.mock.calls[0]![1]!.headers as Record<string, string>).authorization).toBe(
      "Bearer older",
    );
    finish(new Response(JSON.stringify({ ok: "signed in" })));
    await older;
    await newer;
    expect(fetched).toHaveBeenCalledTimes(2);
    expect((fetched.mock.calls[1]![1]!.headers as Record<string, string>).authorization).toBe(
      "Bearer newer",
    );
  });
  it("is taken out of the address bar and exchanged, not kept", async () => {
    fetched.mockResolvedValue(new Response(JSON.stringify({ ok: "signed in" })));
    window.history.replaceState(null, "", "/#token=abc123");

    expect(adoptInvitation()).toBe(true);
    // The credential must not survive in the URL: a reload, a duplicated tab
    // or a link dragged to somebody else would carry it.
    expect(window.location.hash).toBe("");
    expect(window.location.pathname).toBe("/");
    // Nor in script-readable storage. It waits for its exchange, which is what
    // turns it into the HttpOnly cookie this browser keeps instead.
    expect(token()).toBe("");

    await expect(settleInvitation()).resolves.toBe(true);
    const [url, init] = fetched.mock.calls[0]!;
    expect(String(url)).toBe(`${window.location.origin}/v1/auth/session`);
    expect((init!.headers as Record<string, string>).authorization).toBe("Bearer abc123");
    expect(init!.credentials).toBe("same-origin");
    expect(window.localStorage.getItem("guaca.workspace.token")).toBeNull();
  });

  it("migrates a token an older build left in storage, and deletes it", async () => {
    fetched.mockResolvedValue(new Response(JSON.stringify({ ok: "signed in" })));
    window.localStorage.setItem("guaca.workspace.token", "old-token");

    expect(adoptInvitation()).toBe(false);
    await expect(settleInvitation()).resolves.toBe(true);
    const [url, init] = fetched.mock.calls[0]!;
    expect(String(url)).toBe(`${window.location.origin}/v1/auth/session`);
    expect((init!.headers as Record<string, string>).authorization).toBe("Bearer old-token");
    // The script-readable copy is gone the moment the cookie replaces it.
    expect(window.localStorage.getItem("guaca.workspace.token")).toBeNull();
  });

  it("settles nothing when there is neither an invitation nor a stored token", async () => {
    expect(adoptInvitation()).toBe(false);
    await expect(settleInvitation()).resolves.toBe(false);
    expect(fetched).not.toHaveBeenCalled();
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
      expect(boxed.hosted).toBe(true);
    } finally {
      delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
      window.localStorage.removeItem("guaca.workspace.remote");
      vi.resetModules();
    }
  });
});

describe("probing a box", () => {
  it("explains an incompatible host before sending workspace commands", async () => {
    fetched.mockResolvedValueOnce(
      new Response(JSON.stringify({ service: "guacad", build: "abcdef1", apiGeneration: 2 })),
    );
    await expect(probe({ origin: "https://host.example", token: "t" })).rejects.toMatchObject({
      kind: "compatibility",
    });
    expect(fetched).toHaveBeenCalledTimes(1);
  });
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
      message: expect.stringContaining("not a Guaca host"),
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
  it("posts the bytes under the file's name, and unwraps what was stored", async () => {
    fetched.mockResolvedValue(
      new Response(
        JSON.stringify({ ok: { digest: "d", name: "brief.txt", mime: "text/plain", bytes: 5 } }),
      ),
    );
    const file = new File(["hello"], "brief.txt", { type: "text/plain" });
    await expect(upload(file)).resolves.toMatchObject({ digest: "d", name: "brief.txt" });
    const [url, init] = fetched.mock.calls[0]!;
    expect(String(url)).toBe(`${window.location.origin}/v1/upload?name=brief.txt`);
    // A browser holds no script-readable token. The session cookie rides along
    // because a same-origin request asks for credentials, and no header goes
    // out empty.
    expect((init!.headers as Record<string, string>).authorization).toBeUndefined();
    expect(init!.credentials).toBe("same-origin");
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
  it("ignores an old refusal after a new invitation arrives", async () => {
    let finish!: (response: Response) => void;
    fetched.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    const unauthorized = vi.fn();
    window.addEventListener(UNAUTHORIZED_EVENT, unauthorized);
    try {
      const old = invoke("capabilities").catch(() => undefined);
      window.history.replaceState(null, "", "/#token=new-token");
      expect(adoptInvitation()).toBe(true);
      finish(new Response(JSON.stringify({ err: { kind: "unauthorized" } }), { status: 401 }));
      await old;
      expect(unauthorized).not.toHaveBeenCalled();
      fetched.mockResolvedValue(new Response(JSON.stringify({ ok: "signed in" })));
      await settleInvitation();
    } finally {
      window.removeEventListener(UNAUTHORIZED_EVENT, unauthorized);
    }
  });
  it("carries the session cookie and unwraps the answer", async () => {
    fetched.mockResolvedValue(new Response(JSON.stringify({ ok: { fine: true } })));

    await expect(invoke("capabilities")).resolves.toEqual({ fine: true });

    const [url, init] = fetched.mock.calls[0]!;
    expect(String(url)).toBe(`${window.location.origin}/v1/call`);
    const headers = init!.headers as Record<string, string>;
    expect(headers.authorization).toBeUndefined();
    expect(init!.credentials).toBe("same-origin");
    const sent = JSON.parse(String(init!.body));
    expect(sent).toMatchObject({ name: "capabilities", args: {} });
    // In the body and never a header: an older host's CORS would refuse the
    // preflight of every call that carried one it does not know.
    expect(sent.client).toEqual({
      version: expect.stringMatching(/^\d+\.\d+\.\d+/),
      build: expect.any(String),
      desktop: false,
      id: expect.stringMatching(/^[a-z0-9-]{8,}$/),
    });
    expect(Object.keys(headers).sort()).toEqual(["content-type"]);
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

  it("says where it could not reach, and not what the browser called the failure", async () => {
    // `fetch` has one sentence — "Failed to fetch" — for a box that is off, a
    // tunnel that dropped and a name that does not resolve. Printing it inside
    // a sentence that already names the address adds nothing an operator can
    // act on and reads as a stack trace in the one place that should sound like
    // a colleague. The address is the part worth having.
    fetched.mockRejectedValue(new TypeError("Failed to fetch"));

    const refusal = await invoke("capabilities").catch((error: unknown) => error);
    expect(refusal).toMatchObject({ kind: "unreachable" });
    expect((refusal as { message: string }).message).toContain(window.location.origin);
    expect((refusal as { message: string }).message).not.toContain("Failed to fetch");
    expect(isUnreachable(refusal)).toBe(true);
    expect(isUnreachable({ kind: "storage", message: "the disk is full" })).toBe(false);
    expect(isUnreachable(new Error("Failed to fetch"))).toBe(false);
  });
});

describe("the event connection", () => {
  it("bounds a handshake that never opens and reports the outage", async () => {
    vi.useFakeTimers();
    class StalledSocket {
      onopen?: () => void;
      onclose?: () => void;
      onerror?: () => void;
      onmessage?: (event: { data: string }) => void;
      close() {
        this.onclose?.();
      }
    }
    vi.stubGlobal("WebSocket", StalledSocket);
    const { subscribe } = await import("./transport");
    const lost = vi.fn();
    const stop = await subscribe("unused", vi.fn(), vi.fn(), lost);
    try {
      await vi.advanceTimersByTimeAsync(20_000);
      expect(lost).toHaveBeenCalledOnce();
    } finally {
      stop();
      vi.useRealTimers();
    }
  });

  it("refreshes on first open and reconnect, and reconnects on legacy event loss", async () => {
    vi.useFakeTimers();
    const sockets: FakeSocket[] = [];
    class FakeSocket {
      onopen?: () => void;
      onclose?: () => void;
      onerror?: () => void;
      onmessage?: (event: { data: string }) => void;
      constructor() {
        sockets.push(this);
      }
      close() {
        this.onclose?.();
      }
    }
    vi.stubGlobal("WebSocket", FakeSocket);
    const { subscribe } = await import("./transport");
    const changed = vi.fn();
    const refresh = vi.fn();
    const stop = await subscribe("unused", changed, refresh);
    try {
      sockets[0]!.onopen?.();
      expect(refresh).toHaveBeenCalledTimes(1);
      sockets[0]!.onmessage?.({ data: JSON.stringify({ type: "streamLagged", missed: 3 }) });
      expect(changed).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(500);
      sockets[1]!.onopen?.();
      expect(refresh).toHaveBeenCalledTimes(2);
      sockets[1]!.onmessage?.({ data: JSON.stringify({ type: "agentsChanged" }) });
      expect(changed).toHaveBeenCalledWith({ type: "agentsChanged" });
    } finally {
      stop();
      vi.useRealTimers();
    }
  });
});

describe("a host that stops answering", () => {
  class FakeSocket {
    onopen?: () => void;
    onclose?: () => void;
    onerror?: () => void;
    onmessage?: (event: { data: string }) => void;
    constructor(readonly at: number) {}
    close() {
      this.onclose?.();
    }
  }

  async function connected() {
    const sockets: FakeSocket[] = [];
    vi.stubGlobal(
      "WebSocket",
      class extends FakeSocket {
        constructor() {
          super(sockets.length);
          sockets.push(this);
        }
      },
    );
    const { subscribe } = await import("./transport");
    const lost = vi.fn();
    const stop = await subscribe("unused", vi.fn(), vi.fn(), lost);
    return { sockets, lost, stop };
  }

  it("says so when the socket closes and the redial does not answer", async () => {
    vi.useFakeTimers();
    const { sockets, lost, stop } = await connected();
    try {
      sockets[0]!.onopen?.();
      sockets[0]!.onclose?.();
      // The redial is still owed an answer: a socket that dropped and came back
      // inside the first backoff is not a host that left, and a page that said
      // so would be taking it back a moment later.
      await vi.advanceTimersByTimeAsync(600);
      expect(lost).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1_500);
      expect(lost).toHaveBeenCalledTimes(1);
      const refusal = lost.mock.calls[0]![0];
      expect(refusal).toMatchObject({ kind: "unreachable" });
      expect(refusal.message).toContain("lost the connection to this host");
      // Where it is, because what to do about it depends on which host this is.
      expect(refusal.message).toContain(window.location.origin);
      // One silence, one sentence: the banner is not re-raised every attempt.
      await vi.advanceTimersByTimeAsync(60_000);
      expect(lost).toHaveBeenCalledTimes(1);
    } finally {
      stop();
      vi.useRealTimers();
    }
  });

  it("says nothing when the redial answers inside the same second", async () => {
    vi.useFakeTimers();
    const { sockets, lost, stop } = await connected();
    try {
      sockets[0]!.onopen?.();
      sockets[0]!.onclose?.();
      // The first redial waits at most the retry floor, so the connection that
      // lands here is the one that says the host was never gone.
      await vi.advanceTimersByTimeAsync(500);
      sockets[1]!.onopen?.();
      await vi.advanceTimersByTimeAsync(5_000);
      expect(lost).not.toHaveBeenCalled();
    } finally {
      stop();
      vi.useRealTimers();
    }
  });

  it("says nothing when the page lets the subscription go", async () => {
    // A reload, a token screen or a second window closing this one is the page
    // leaving, not the host, and a mark left behind would outlive the reason
    // for it.
    vi.useFakeTimers();
    const { sockets, lost, stop } = await connected();
    try {
      sockets[0]!.onopen?.();
      sockets[0]!.onclose?.();
      stop();
      await vi.advanceTimersByTimeAsync(5_000);
      expect(lost).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("asks for a token only when an authenticated probe confirms a refusal", async () => {
    vi.useFakeTimers();
    fetched.mockResolvedValue(
      new Response(JSON.stringify({ err: { kind: "unauthorized" } }), { status: 401 }),
    );
    const unauthorized = vi.fn();
    window.addEventListener(UNAUTHORIZED_EVENT, unauthorized);
    const { sockets, stop } = await connected();
    try {
      sockets[0]!.onopen?.();
      sockets[0]!.onclose?.();
      await vi.advanceTimersByTimeAsync(2_000);
      expect(unauthorized).toHaveBeenCalledOnce();
      expect(fetched).toHaveBeenCalledOnce();
      await vi.advanceTimersByTimeAsync(30_000);
      expect(unauthorized).toHaveBeenCalledOnce();
    } finally {
      stop();
      window.removeEventListener(UNAUTHORIZED_EVENT, unauthorized);
      vi.useRealTimers();
    }
  });

  it("does not mistake an unreachable probe for an expired session", async () => {
    vi.useFakeTimers();
    const unauthorized = vi.fn();
    window.addEventListener(UNAUTHORIZED_EVENT, unauthorized);
    const { sockets, stop } = await connected();
    try {
      sockets[0]!.onclose?.();
      await vi.advanceTimersByTimeAsync(2_000);
      expect(fetched).toHaveBeenCalledOnce();
      expect(unauthorized).not.toHaveBeenCalled();
    } finally {
      stop();
      window.removeEventListener(UNAUTHORIZED_EVENT, unauthorized);
      vi.useRealTimers();
    }
  });

  it("ignores a late refusal after the socket reconnects", async () => {
    vi.useFakeTimers();
    let finish!: (response: Response) => void;
    fetched.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    const unauthorized = vi.fn();
    window.addEventListener(UNAUTHORIZED_EVENT, unauthorized);
    const { sockets, stop } = await connected();
    try {
      sockets[0]!.onopen?.();
      sockets[0]!.onclose?.();
      await vi.advanceTimersByTimeAsync(2_000);
      sockets[1]!.onopen?.();
      finish(new Response(JSON.stringify({ err: { kind: "unauthorized" } }), { status: 401 }));
      await vi.advanceTimersByTimeAsync(0);
      expect(unauthorized).not.toHaveBeenCalled();
    } finally {
      stop();
      window.removeEventListener(UNAUTHORIZED_EVENT, unauthorized);
      vi.useRealTimers();
    }
  });
});

describe("reconnecting", () => {
  it("waits between half and all of the backoff, so windows do not redial in step", async () => {
    const { retryDelay } = await import("./transport");
    expect(retryDelay(1000, () => 0)).toBe(500);
    expect(retryDelay(1000, () => 1)).toBe(1000);
    expect(retryDelay(1000, () => 0.5)).toBe(750);
  });

  it("tries again at once when the network comes back", async () => {
    vi.useFakeTimers();
    const sockets: { onclose?: () => void; onopen?: () => void }[] = [];
    class FakeSocket {
      onopen?: () => void;
      onclose?: () => void;
      onerror?: () => void;
      onmessage?: (event: { data: string }) => void;
      constructor() {
        sockets.push(this);
      }
      close() {
        this.onclose?.();
      }
    }
    vi.stubGlobal("WebSocket", FakeSocket);
    const { subscribe } = await import("./transport");
    const stop = await subscribe("unused", vi.fn(), vi.fn());
    try {
      sockets[0]!.onclose?.();
      expect(sockets).toHaveLength(1);
      window.dispatchEvent(new Event("online"));
      expect(sockets).toHaveLength(2);
    } finally {
      stop();
      vi.useRealTimers();
    }
  });
});
