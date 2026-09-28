import "@xterm/xterm/css/xterm.css";

import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal } from "@xterm/xterm";
import { useEffect, useRef, useState } from "react";

import { epitaph, heard, resize, type Standing, stateWord } from "../lib/console";
import { prefersReducedMotion } from "../lib/motion";
import { consoleUrl, openExternal } from "../lib/transport";
import type { AgentCard } from "../lib/types";

interface Props {
  agent: AgentCard;
  /** Where it starts, as the panel already read it. */
  path: string;
  onClose: () => void;
}

/**
 * A shell in an agent's terminal, for the operator, over the window.
 *
 * What an agent asks a person to run goes here: `gh auth login`, a coding
 * program's own sign-in, a look around the directory before deciding
 * something. It is the host's shell standing in the agent's directory, not
 * the agent's: nothing typed reaches a transcript, and none of the secrets
 * Guaca holds are in it. `server/console.rs` is the argument.
 *
 * The stage is the computer screen's grown one, for the same reason that
 * screen is ink: it is a machine, drawn on the ground a terminal expects, and
 * the colors programs print are chosen for it.
 *
 * Escape belongs to the shell. A menu in a sign-in, readline and every editor
 * read it, and a dialog that closed on it would take the shell with it, so the
 * way out is Done and nothing else. Closing ends the shell, and the bar says
 * so, because the host keeps nothing for a page that might come back.
 */
export function Console({ agent, path, onClose }: Props) {
  const screen = useRef<HTMLDivElement>(null);
  const [standing, setStanding] = useState<Standing>({ kind: "opening" });

  useEffect(() => {
    const element = screen.current;
    if (!element) return;
    let live = true;
    let close = () => {
      live = false;
    };
    // xterm measures a cell once, when it opens, and not again when a font
    // arrives. Opened on a fallback face, every glyph after the load is drawn
    // at the wrong width and the cursor walks away from the text.
    const drawn = getComputedStyle(element);
    const face = document.fonts?.load(`${drawn.fontSize} ${drawn.fontFamily}`);
    void Promise.resolve(face)
      .catch(() => [])
      .then(() => {
        if (live) close = attach(element, agent.id, setStanding);
      });
    return () => close();
  }, [agent.id]);

  return (
    <div
      className="console"
      role="dialog"
      aria-modal="true"
      aria-label={`${agent.name}'s terminal`}
    >
      <div className="screen__bar">
        <span className="screen__title">{agent.name}'s terminal</span>
        <code className="console__path" title={path}>
          {path}
        </code>
        <span
          className="screen__state"
          data-state={standing.kind === "open" ? "running" : undefined}
          aria-live="polite"
        >
          {stateWord(standing)}
        </span>
        <span style={{ flex: 1 }} />
        <button
          type="button"
          className="btn btn--small"
          onClick={onClose}
          title="Close. The shell ends, and anything still running in it."
        >
          Done
        </button>
      </div>
      <div className="console__body">
        <div className="console__screen" ref={screen} />
      </div>
    </div>
  );
}

/**
 * Opens the terminal in `element` and connects it to the host.
 *
 * Returns what undoes all of it. The size the terminal measured goes with the
 * connection, and every size after that as it changes.
 */
function attach(
  element: HTMLElement,
  agent: string,
  onStanding: (standing: Standing) => void,
): () => void {
  // Read off the element, so the face, the size and the two colors are the
  // stylesheet's decision and this only carries them to the renderer.
  const drawn = getComputedStyle(element);
  const terminal = new Terminal({
    fontFamily: drawn.fontFamily,
    fontSize: Number.parseFloat(drawn.fontSize),
    cursorBlink: !prefersReducedMotion(),
    theme: { background: drawn.backgroundColor, foreground: drawn.color, cursor: drawn.color },
    // A link a program prints opens where the operator browses, never in
    // this window. `gh`'s device sign-in is a link and a code, and this is
    // how the link is followed.
    linkHandler: { activate: (_event, uri) => void openExternal(uri) },
  });
  const fit = new FitAddon();
  terminal.loadAddon(fit);
  terminal.loadAddon(new WebLinksAddon((_event, uri) => void openExternal(uri)));
  terminal.open(element);
  fit.fit();
  terminal.focus();

  const socket = new WebSocket(consoleUrl(agent, terminal.cols, terminal.rows));
  socket.binaryType = "arraybuffer";
  let over = false;
  const end = (standing: Standing) => {
    over = true;
    onStanding(standing);
    const line = epitaph(standing);
    // Dim, on a line of its own, so it reads as the terminal's and not the
    // shell's.
    if (line) terminal.write(`\r\n\x1b[2m${line}\x1b[0m\r\n`);
  };
  socket.onopen = () => onStanding({ kind: "open" });
  socket.onmessage = ({ data }) => {
    if (typeof data !== "string") {
      terminal.write(new Uint8Array(data as ArrayBuffer));
      return;
    }
    const said = heard(data);
    if (said?.type === "exit") end({ kind: "exited", code: said.code });
    if (said?.type === "refused") end({ kind: "refused", message: said.message });
  };
  // Always after `onerror`, so a failed handshake and a lost network are one
  // ending and not two.
  socket.onclose = () => {
    if (!over) end({ kind: "dropped" });
  };

  const send = (frame: Uint8Array | string) => {
    if (socket.readyState === WebSocket.OPEN) socket.send(frame);
  };
  const encoder = new TextEncoder();
  const typed = terminal.onData((data) => send(encoder.encode(data)));
  // Mouse reports past 127 are bytes rather than text, which is why xterm
  // hands them over apart; encoded as text they would arrive as two bytes.
  const reported = terminal.onBinary((data) =>
    send(Uint8Array.from(data, (character) => character.charCodeAt(0) & 0xff)),
  );
  const resized = terminal.onResize(({ cols, rows }) => send(resize(cols, rows)));
  const observer = new ResizeObserver(() => fit.fit());
  observer.observe(element);

  return () => {
    over = true;
    observer.disconnect();
    typed.dispose();
    reported.dispose();
    resized.dispose();
    socket.close();
    terminal.dispose();
  };
}
