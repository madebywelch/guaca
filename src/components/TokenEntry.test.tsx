import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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
 *
 * The credential never lands in script-readable storage. Every exchange below
 * is a request to the daemon, which answers with the HttpOnly cookie this
 * browser keeps, so `fetch` is stubbed for each one and the assertion is what
 * crossed the wire rather than what was written to the disk.
 */

const capabilities = vi.fn<() => Promise<unknown>>();

vi.mock("../lib/ipc", () => ({
  api: { capabilities: () => capabilities() },
}));

delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
const { TokenEntry } = await import("./TokenEntry");
const { setToken, UNAUTHORIZED_EVENT } = await import("../lib/transport");

const fetched = vi.fn<typeof fetch>();

const EVERYTHING = {
  loopbackEndpoints: true,
  claudeProvider: true,
  localFiles: true,
};

const TOKEN_KEY = "guaca.workspace.token";

/** A daemon that takes the token and sets the session cookie. */
function signedIn() {
  fetched.mockResolvedValue(new Response(JSON.stringify({ ok: "signed in" })));
}

/** A daemon that turns the token away. */
function turnedAway(message = "that token was not accepted") {
  fetched.mockResolvedValue(
    new Response(JSON.stringify({ err: { kind: "unauthorized", message } }), { status: 401 }),
  );
}

/** The one exchange request that crossed, if one did. */
function exchange() {
  return fetched.mock.calls.find(([url]) => String(url).endsWith("/v1/auth/session"));
}

function open() {
  return render(
    <TokenEntry>
      <p>the workspace</p>
    </TokenEntry>,
  );
}

beforeEach(async () => {
  // Stubbed before signing out: signing out is a request to the daemon now,
  // and a real `fetch` in jsdom would only fail against nothing listening.
  vi.stubGlobal("fetch", fetched);
  fetched.mockResolvedValue(new Response(JSON.stringify({ ok: "signed out" })));
  await setToken("");
  fetched.mockReset();
  signedIn();
  window.history.replaceState(null, "", "/");
  window.localStorage.clear();
  capabilities.mockReset();
  capabilities.mockResolvedValue(EVERYTHING);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("TokenEntry", () => {
  it("asks for a token when the box refuses the session", async () => {
    capabilities.mockRejectedValue({ kind: "unauthorized", message: "needs the token" });
    open();
    await waitFor(() => expect(screen.getByLabelText("Host token")).toBeTruthy());
    expect(screen.queryByText("the workspace")).toBeNull();
    // Nothing was stored to exchange, so the one question asked was whether the
    // session the browser already had still opens the box.
    expect(capabilities).toHaveBeenCalledTimes(1);
    expect(fetched).not.toHaveBeenCalled();
  });

  it("waits rather than flashing the form while it asks whether the session is live", async () => {
    let answer: (value: unknown) => void = () => {};
    capabilities.mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    open();
    // A browser holds a cookie it cannot read, and the daemon has not answered
    // yet. Neither answer is known, so neither is drawn: a reload of a
    // signed-in page must not ask for a token on its way to the workspace.
    expect(screen.queryByLabelText("Host token")).toBeNull();
    expect(screen.queryByText("the workspace")).toBeNull();
    expect(screen.getByRole("status").textContent).toContain("Opening Guaca");

    await act(async () => answer(EVERYTHING));
    await waitFor(() => expect(screen.getByText("the workspace")).toBeTruthy());
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("takes an invitation pasted into the tab that is already open", async () => {
    capabilities.mockRejectedValue({ kind: "unauthorized", message: "needs the token" });
    open();
    await waitFor(() => expect(screen.getByLabelText("Host token")).toBeTruthy());

    // Same document, new fragment: no reload, so the mount has already run.
    window.history.replaceState(null, "", "/#token=late123");
    act(() => {
      window.dispatchEvent(new HashChangeEvent("hashchange"));
    });

    await waitFor(() => expect(screen.getByText("the workspace")).toBeTruthy());
    const crossed = exchange();
    expect(crossed).toBeTruthy();
    expect((crossed![1]!.headers as Record<string, string>).authorization).toBe("Bearer late123");
    expect(window.location.hash).toBe("");
  });

  it("does not accuse a first visitor of a session that stopped", async () => {
    capabilities.mockRejectedValue({ kind: "unauthorized", message: "needs the token" });
    open();
    // The transport raises this for any refused call, and the first question a
    // browser with no session asks is one of them. That is not an expiry, and
    // saying so would name a problem this operator never had.
    act(() => {
      window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
    });
    await waitFor(() => expect(screen.getByLabelText("Host token")).toBeTruthy());
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows the workspace when the box is only unreachable", async () => {
    capabilities.mockRejectedValue({
      kind: "unreachable",
      message: "could not reach this host",
    });
    open();
    // A box that is down is the app's own banner to draw. Asking for a token
    // would name the wrong problem and hide the sentence that says which.
    await waitFor(() => expect(screen.getByText("the workspace")).toBeTruthy());
    expect(screen.queryByLabelText("Host token")).toBeNull();
  });

  it("lets a clicked invitation straight in", async () => {
    window.history.replaceState(null, "", "/#token=abc123");
    open();
    await waitFor(() => expect(screen.getByText("the workspace")).toBeTruthy());
    expect(screen.queryByLabelText("Host token")).toBeNull();
    // The credential must not survive in the URL.
    expect(window.location.hash).toBe("");

    const crossed = exchange();
    expect(crossed).toBeTruthy();
    expect((crossed![1]!.headers as Record<string, string>).authorization).toBe("Bearer abc123");
    expect(crossed![1]!.credentials).toBe("same-origin");
    // Nor in script-readable storage: the cookie is the only copy.
    expect(window.localStorage.getItem(TOKEN_KEY)).toBeNull();
    // The exchange admitted it, so nothing had to ask twice.
    expect(capabilities).not.toHaveBeenCalled();
  });

  it("migrates a token an older build left in storage", async () => {
    window.localStorage.setItem(TOKEN_KEY, "old-token");
    open();
    await waitFor(() => expect(screen.getByText("the workspace")).toBeTruthy());

    const crossed = exchange();
    expect(crossed).toBeTruthy();
    expect((crossed![1]!.headers as Record<string, string>).authorization).toBe("Bearer old-token");
    expect(window.localStorage.getItem(TOKEN_KEY)).toBeNull();
  });

  it("lets a browser whose session cookie is still valid straight in", async () => {
    open();
    await waitFor(() => expect(screen.getByText("the workspace")).toBeTruthy());
    // Nothing to exchange: no invitation and no stored token. The cookie was
    // valid, and one cheap question proved it without asking for a token.
    expect(fetched).not.toHaveBeenCalled();
    expect(capabilities).toHaveBeenCalledTimes(1);
  });

  it("checks a pasted token against the box before admitting it", async () => {
    capabilities.mockRejectedValueOnce({ kind: "unauthorized", message: "needs the token" });
    open();
    const field = await screen.findByLabelText("Host token");
    capabilities.mockResolvedValue(EVERYTHING);

    fireEvent.change(field, { target: { value: " abc123 " } });
    fireEvent.click(screen.getByRole("button", { name: "Open Guaca" }));

    await waitFor(() => expect(screen.getByText("the workspace")).toBeTruthy());
    const crossed = exchange();
    expect(crossed).toBeTruthy();
    // The whitespace a copy leaves behind is trimmed before the exchange.
    expect((crossed![1]!.headers as Record<string, string>).authorization).toBe("Bearer abc123");
    expect(window.localStorage.getItem(TOKEN_KEY)).toBeNull();
    // Once to find the form, once to confirm the token it was given.
    expect(capabilities).toHaveBeenCalledTimes(2);
  });

  it("refuses a wrong token here, and says only what that added", async () => {
    capabilities.mockRejectedValue({ kind: "unauthorized", message: "needs the token" });
    turnedAway();
    open();
    const field = await screen.findByLabelText("Host token");
    fireEvent.change(field, { target: { value: "wrong" } });
    fireEvent.click(screen.getByRole("button", { name: "Open Guaca" }));

    // The hint above the field already says where the token comes from. What
    // the operator does not know until this answers is that the one they
    // pasted was turned away, and that is the whole of what it says now: the
    // old sentence repeated the hint under it, in the daemon's voice, and
    // never said the new fact at all.
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe("That token was not accepted.");
    expect(alert.textContent).not.toContain("logs");
    expect(alert.textContent).not.toContain("token file");
    expect(screen.queryByText("the workspace")).toBeNull();
    // A refused token is not kept: the next reload would only refuse it again,
    // with the app's own reads failing forty times before this form appeared.
    expect(window.localStorage.getItem(TOKEN_KEY)).toBeNull();
  });

  it("does not send an empty box", async () => {
    capabilities.mockRejectedValue({ kind: "unauthorized", message: "needs the token" });
    open();
    const button = (await screen.findByRole("button", { name: "Open Guaca" })) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    fireEvent.submit(button.closest("form")!);
    // The mount's own question was asked once; submitting added nothing.
    expect(capabilities).toHaveBeenCalledTimes(1);
    expect(fetched).not.toHaveBeenCalled();
  });

  it("brings the form back when the box stops accepting the token it had", async () => {
    open();
    await waitFor(() => expect(screen.getByText("the workspace")).toBeTruthy());

    act(() => {
      window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
    });

    await waitFor(() => expect(screen.getByLabelText("Host token")).toBeTruthy());
    expect(screen.queryByText("the workspace")).toBeNull();
    expect(screen.getByRole("alert").textContent).toContain("stopped accepting");
  });
});
