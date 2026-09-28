import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard } from "../lib/types";
import { Console } from "./Console";

/**
 * The wiring between xterm.js and the socket. xterm draws on a canvas jsdom
 * does not have, and the socket goes to a host that is not here, so both are
 * stand-ins that record what they were handed; what the host does with it is
 * `tests/server.rs`.
 */
const fakes = vi.hoisted(() => {
  const terminals: FakeTerminal[] = [];
  const sockets: FakeSocket[] = [];
  const opened: string[] = [];

  type Listener<T> = (value: T) => void;

  class FakeTerminal {
    cols = 100;
    rows = 30;
    written: (string | Uint8Array)[] = [];
    disposed = false;
    focused = false;
    data?: Listener<string>;
    binary?: Listener<string>;
    resized?: Listener<{ cols: number; rows: number }>;
    constructor(
      readonly options: { linkHandler?: { activate: (e: unknown, uri: string) => void } },
    ) {
      terminals.push(this);
    }
    loadAddon() {}
    open() {}
    focus() {
      this.focused = true;
    }
    write(data: string | Uint8Array) {
      this.written.push(data);
    }
    onData(listener: Listener<string>) {
      this.data = listener;
      return { dispose: () => {} };
    }
    onBinary(listener: Listener<string>) {
      this.binary = listener;
      return { dispose: () => {} };
    }
    onResize(listener: Listener<{ cols: number; rows: number }>) {
      this.resized = listener;
      return { dispose: () => {} };
    }
    dispose() {
      this.disposed = true;
    }
    /** Everything written, as text. */
    text(): string {
      return this.written
        .map((part) => (typeof part === "string" ? part : new TextDecoder().decode(part)))
        .join("");
    }
  }

  class FakeSocket {
    static OPEN = 1;
    readyState = 0;
    binaryType = "blob";
    sent: (string | Uint8Array)[] = [];
    closed = false;
    onopen: (() => void) | null = null;
    onmessage: ((event: { data: unknown }) => void) | null = null;
    onclose: (() => void) | null = null;
    constructor(readonly url: string) {
      sockets.push(this);
    }
    send(frame: string | Uint8Array) {
      this.sent.push(frame);
    }
    close() {
      this.closed = true;
    }
    /** The host accepting it. */
    accept() {
      this.readyState = FakeSocket.OPEN;
      this.onopen?.();
    }
    /** The host saying something. */
    say(data: unknown) {
      this.onmessage?.({ data });
    }
    /** The connection going, with or without a word before it. */
    drop() {
      this.readyState = 3;
      this.onclose?.();
    }
  }

  return { terminals, sockets, opened, FakeTerminal, FakeSocket };
});

vi.mock("@xterm/xterm", () => ({ Terminal: fakes.FakeTerminal }));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit() {}
  },
}));
vi.mock("@xterm/addon-web-links", () => ({ WebLinksAddon: class {} }));
vi.mock("../lib/transport", () => ({
  consoleUrl: (agent: string, cols: number, rows: number) =>
    `ws://box/v1/console/${agent}?token=t&cols=${cols}&rows=${rows}`,
  openExternal: async (url: string) => {
    fakes.opened.push(url);
  },
}));

const PATH = "/var/lib/guaca/data/terminals/a1";

const agent = { id: "a1", name: "Engineer" } as AgentCard;

/** Mounts it and waits for the terminal and its socket to exist. */
async function mount(onClose = vi.fn()) {
  const view = render(<Console agent={agent} path={PATH} onClose={onClose} />);
  await waitFor(() => expect(fakes.sockets).toHaveLength(1));
  return { view, terminal: fakes.terminals[0]!, socket: fakes.sockets[0]!, onClose };
}

beforeEach(() => {
  fakes.terminals.length = 0;
  fakes.sockets.length = 0;
  fakes.opened.length = 0;
  vi.stubGlobal("WebSocket", fakes.FakeSocket);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("Console", () => {
  it("says why the host started no shell, in the terminal", async () => {
    const { terminal, socket } = await mount();
    act(() => {
      socket.accept();
      socket.say(
        '{"type":"refused","message":"Engineer has no terminal. Give it one from the Terminal section of its panel first"}',
      );
      socket.drop();
    });

    expect(terminal.text()).toContain("Engineer has no terminal. Give it one");
    expect(screen.getByText("ended")).toBeTruthy();
    // One ending, not a refusal followed by a dropped connection.
    expect(terminal.text()).not.toContain("dropped");
  });

  it("says a connection that went without a word ended the shell", async () => {
    const { terminal, socket } = await mount();
    act(() => socket.drop());

    expect(terminal.text()).toContain("The connection to the host dropped, which ended this shell");
    expect(screen.getByText("ended")).toBeTruthy();
  });

  it("says how the shell exited, once", async () => {
    const { terminal, socket } = await mount();
    act(() => {
      socket.accept();
      socket.say('{"type":"exit","code":7}');
      socket.drop();
    });

    expect(terminal.text()).toContain("The shell exited with code 7.");
    expect(terminal.text()).not.toContain("dropped");
  });

  it("leaves Escape to the shell", async () => {
    const { onClose } = await mount();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("connects at the size it measured and carries the shell's bytes both ways", async () => {
    const { terminal, socket } = await mount();
    expect(socket.url).toBe("ws://box/v1/console/a1?token=t&cols=100&rows=30");
    expect(socket.binaryType).toBe("arraybuffer");
    expect(terminal.focused).toBe(true);
    expect(screen.getByText("starting")).toBeTruthy();

    // Typed before the host answered: nothing to send it to yet.
    terminal.data?.("x");
    expect(socket.sent).toEqual([]);

    act(() => socket.accept());
    expect(screen.getByText("running")).toBeTruthy();

    terminal.data?.("gh auth login\r");
    expect(new TextDecoder().decode(socket.sent[0] as Uint8Array)).toBe("gh auth login\r");
    // A mouse report past 127 is one byte, not the two UTF-8 would make of it.
    terminal.binary?.("\x1b[M\xff");
    expect(Array.from(socket.sent[1] as Uint8Array)).toEqual([0x1b, 0x5b, 0x4d, 0xff]);
    terminal.resized?.({ cols: 120, rows: 40 });
    expect(JSON.parse(socket.sent[2] as string)).toEqual({ type: "resize", cols: 120, rows: 40 });

    act(() => socket.say(new TextEncoder().encode("! First copy your one-time code").buffer));
    expect(terminal.text()).toContain("First copy your one-time code");
  });

  it("opens a link a program prints where the operator browses", async () => {
    const { terminal } = await mount();
    terminal.options.linkHandler?.activate({}, "https://github.com/login/device");
    expect(fakes.opened).toEqual(["https://github.com/login/device"]);
  });

  it("closes the connection and the terminal with it, and says nothing afterward", async () => {
    const { view, terminal, socket, onClose } = await mount();
    act(() => socket.accept());

    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onClose).toHaveBeenCalled();

    view.unmount();
    expect(socket.closed).toBe(true);
    expect(terminal.disposed).toBe(true);
    const before = terminal.written.length;
    socket.drop();
    expect(terminal.written).toHaveLength(before);
  });

  it("is a dialog named for the agent, and names the directory", async () => {
    await mount();
    expect(screen.getByRole("dialog", { name: "Engineer's terminal" })).toBeTruthy();
    expect(screen.getByText(PATH)).toBeTruthy();
  });
});
