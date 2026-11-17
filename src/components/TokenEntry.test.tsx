import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useEffect } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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
  borrowedSignins: true,
};

const TOKEN_KEY = "guaca.workspace.token";

function signedIn() {
  fetched.mockResolvedValue(new Response(JSON.stringify({ ok: "signed in" })));
}

function turnedAway(message = "that token was not accepted") {
  fetched.mockResolvedValue(
    new Response(JSON.stringify({ err: { kind: "unauthorized", message } }), { status: 401 }),
  );
}

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

function RefusedOnArrival() {
  useEffect(() => {
    window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
  }, []);
  return <p>the workspace</p>;
}

beforeEach(async () => {
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
  it("does not readmit a session after a later refusal", async () => {
    let answer!: (value: unknown) => void;
    capabilities.mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    open();
    await act(async () => {});
    act(() => window.dispatchEvent(new Event(UNAUTHORIZED_EVENT)));
    await act(async () => answer(EVERYTHING));
    expect(screen.getByLabelText("Host token")).toBeTruthy();
    expect(screen.queryByText("the workspace")).toBeNull();
  });

  it("asks for a token when the box refuses the session", async () => {
    capabilities.mockRejectedValue({ kind: "unauthorized", message: "needs the token" });
    open();
    await waitFor(() => expect(screen.getByLabelText("Host token")).toBeTruthy());
    expect(screen.queryByText("the workspace")).toBeNull();
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
    await waitFor(() => expect(screen.getByText("the workspace")).toBeTruthy());
    expect(screen.queryByLabelText("Host token")).toBeNull();
  });

  it("lets a clicked invitation straight in", async () => {
    window.history.replaceState(null, "", "/#token=abc123");
    open();
    await waitFor(() => expect(screen.getByText("the workspace")).toBeTruthy());
    expect(screen.queryByLabelText("Host token")).toBeNull();
    expect(window.location.hash).toBe("");

    const crossed = exchange();
    expect(crossed).toBeTruthy();
    expect((crossed![1]!.headers as Record<string, string>).authorization).toBe("Bearer abc123");
    expect(crossed![1]!.credentials).toBe("same-origin");
    expect(window.localStorage.getItem(TOKEN_KEY)).toBeNull();
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
    expect((crossed![1]!.headers as Record<string, string>).authorization).toBe("Bearer abc123");
    expect(window.localStorage.getItem(TOKEN_KEY)).toBeNull();
    expect(capabilities).toHaveBeenCalledTimes(2);
  });

  it("refuses a wrong token here, and says only what that added", async () => {
    capabilities.mockRejectedValue({ kind: "unauthorized", message: "needs the token" });
    turnedAway();
    open();
    const field = await screen.findByLabelText("Host token");
    fireEvent.change(field, { target: { value: "wrong" } });
    fireEvent.click(screen.getByRole("button", { name: "Open Guaca" }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe("That token was not accepted.");
    expect(alert.textContent).not.toContain("logs");
    expect(alert.textContent).not.toContain("token file");
    expect(screen.queryByText("the workspace")).toBeNull();
    expect(window.localStorage.getItem(TOKEN_KEY)).toBeNull();
  });

  it("does not send an empty box", async () => {
    capabilities.mockRejectedValue({ kind: "unauthorized", message: "needs the token" });
    open();
    const button = (await screen.findByRole("button", { name: "Open Guaca" })) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    fireEvent.submit(button.closest("form")!);
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

  it("counts the browser as let in from the moment the workspace is drawn", async () => {
    render(
      <TokenEntry>
        <RefusedOnArrival />
      </TokenEntry>,
    );

    await waitFor(() => expect(screen.getByLabelText("Host token")).toBeTruthy());
    expect(screen.getByRole("alert").textContent).toContain("stopped accepting");
  });
});
