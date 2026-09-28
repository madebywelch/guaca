/**
 * The operator's own shell in an agent's terminal, as this page speaks to it.
 *
 * The socket carries the shell's bytes both ways, untouched. Everything that
 * is not those bytes is below: two things the host says and one this page
 * does, spelled as `server/console.rs` spells them, and the suites beside each
 * hold the other's literal. No DOM.
 */

/** What the host says that is not the shell's own output. */
export type Said =
  /** The shell has gone. `null` is a shell that was killed rather than one
   *  that exited, which is not the same as a zero. */
  | { type: "exit"; code: number | null }
  /** No shell was started, in a sentence that says what to do. */
  | { type: "refused"; message: string };

/**
 * A text frame from the host, or `null` for one this build has no word for.
 * Dropped rather than thrown, as the event socket drops a frame it cannot
 * parse: a newer host saying something new must not end the shell.
 */
export function heard(text: string): Said | null {
  try {
    const said = JSON.parse(text) as Partial<Said> & { type?: unknown };
    if (said.type === "exit") {
      const code = (said as { code?: unknown }).code;
      return { type: "exit", code: typeof code === "number" ? code : null };
    }
    if (said.type === "refused") {
      const message = (said as { message?: unknown }).message;
      return { type: "refused", message: typeof message === "string" ? message : "" };
    }
  } catch {
    // Not JSON at all.
  }
  return null;
}

/** Tells the host the window now holds this many cells. */
export function resize(cols: number, rows: number): string {
  return JSON.stringify({ type: "resize", cols, rows });
}

/** Where the shell stands, which is what the bar says and what ends it. */
export type Standing =
  | { kind: "opening" }
  | { kind: "open" }
  | { kind: "exited"; code: number | null }
  | { kind: "refused"; message: string }
  /** The socket closed without the host saying why. The host ends a shell
   *  whose socket is gone, so this is the shell over too. */
  | { kind: "dropped" };

/** The word the bar shows. */
export function stateWord(standing: Standing): string {
  if (standing.kind === "opening") return "starting";
  if (standing.kind === "open") return "running";
  return "ended";
}

/**
 * The last line written into the terminal when the shell ends, or `null` while
 * it has not.
 *
 * In the terminal rather than beside it, which is where every terminal says
 * its process ended, and where the operator's eyes already are. Each one says
 * how to get a shell back, because the only way is the panel.
 */
export function epitaph(standing: Standing): string | null {
  switch (standing.kind) {
    case "opening":
    case "open":
      return null;
    case "exited":
      if (standing.code === null)
        return "The shell was stopped. Open the terminal again for a new one.";
      if (standing.code === 0) return "The shell exited. Open the terminal again for a new one.";
      return `The shell exited with code ${standing.code}. Open the terminal again for a new one.`;
    case "refused":
      return sentence(standing.message || "the host started no shell");
    case "dropped":
      return "The connection to the host dropped, which ended this shell. Open the terminal again when it is back.";
  }
}

/** A refusal from Rust, which starts lowercase because it is an error there. */
function sentence(text: string): string {
  const said = text.trim();
  const opened = said.charAt(0).toUpperCase() + said.slice(1);
  return /[.!?]$/.test(opened) ? opened : `${opened}.`;
}
