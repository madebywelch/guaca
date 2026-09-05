/**
 * How the frontend reaches the runtime, which is not always the same place.
 *
 * Guaca runs in two hosts. In the desktop app the runtime is in the window's
 * own process and Tauri carries a call; on a server it is a daemon on a box
 * and the same call is an HTTP POST, with the event channel a WebSocket. The
 * shapes are identical because Tauri's IPC is already "a name, some named
 * arguments, and a value or a structured error", which is what a POST is.
 *
 * ## One bundle, both hosts
 *
 * Which host this is, is decided at *runtime* rather than at build time, and
 * that is load-bearing rather than clever. The daemon serves the same `dist/`
 * the desktop app embeds, so there is one bundle to build, one to test and one
 * to ship. A build-time flag would mean two, and the second would be the one
 * nobody runs the suite against.
 *
 * The detection is Tauri's own marker on `window`. Absent means a browser,
 * which means a daemon is on the other end of the origin this page was served
 * from.
 *
 * ## The token
 *
 * A hosted workspace holds inference keys, plugin refresh tokens and an
 * operator's transcripts, so every call carries the workspace's bearer token
 * and there is no anonymous mode. It lives in `localStorage` because it has to
 * survive a reload and because a cookie would be sent by any page that could
 * reach the origin. The desktop app has no token and needs none: the runtime is
 * inside the process asking.
 */

import type { UiEvent } from "./types";

/** Tauri v2 puts this on `window` before any of our code runs. */
const IN_A_WINDOW = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/**
 * A box the desktop app is showing instead of its own workspace.
 *
 * The third arrangement, and it is the second one wearing a window: the calls
 * go over HTTP to the box exactly as a browser's would, and only the drop and
 * the menu bar know the difference, because those two are things the window
 * has that a browser does not. Read once at load, like the host itself: the
 * whole page reloads when the operator points it somewhere else, so nothing
 * has to notice a change under it.
 */
export interface Remote {
  origin: string;
  token: string;
}

const REMOTE_KEY = "guaca.workspace.remote";

function readRemote(): Remote | null {
  try {
    const raw = window.localStorage.getItem(REMOTE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<Remote>;
    if (typeof parsed.origin !== "string" || typeof parsed.token !== "string") return null;
    if (!parsed.origin) return null;
    return { origin: parsed.origin.replace(/\/+$/, ""), token: parsed.token };
  } catch {
    return null;
  }
}

const ATTACHED: Remote | null = IN_A_WINDOW ? readRemote() : null;

/** The box this window is showing, or null for this machine's own workspace. */
export function attached(): Remote | null {
  return ATTACHED;
}

/**
 * Points the desktop app at a box, or back at itself. Takes effect on the
 * next load, and the caller is what reloads: everything about which host this
 * is was decided at import time, on purpose, so there is nothing to update in
 * place and nothing that can be half-updated.
 */
export function setRemote(remote: Remote | null): void {
  try {
    if (remote) window.localStorage.setItem(REMOTE_KEY, JSON.stringify(remote));
    else window.localStorage.removeItem(REMOTE_KEY);
  } catch {
    /* storage disabled; the next load reads nothing and shows this machine */
  }
}

/** Reloads the page, which is how a change of workspace takes effect. */
export function restart(): void {
  window.location.reload();
}

/** Whether the runtime is somewhere other than this process. */
export const hosted = !IN_A_WINDOW || ATTACHED !== null;

/** Whether calls go over Tauri's own bridge, to the runtime in this process. */
const OVER_THE_BRIDGE = IN_A_WINDOW && ATTACHED === null;

const TOKEN_KEY = "guaca.workspace.token";

/**
 * The fragment a daemon prints its invitation with.
 *
 * `http://box:8787/#token=…` is one thing to click rather than a URL and a
 * string to paste beside it. A fragment because it is the one part of a URL
 * that never leaves the browser: it is not sent to the daemon, not to a proxy
 * in front of it, and not to whatever logs either of them keeps. The query
 * string the socket uses has none of those properties, which is why the
 * invitation does not use it.
 */
const FRAGMENT_KEY = "token";

/** The name of the event raised when a call is turned away for its token. */
export const UNAUTHORIZED_EVENT = "guaca:unauthorized";

/** A structured refusal, which is the shape both hosts already produced. */
export interface Refusal {
  kind: string;
  message: string;
}

/**
 * The token this browser presents, or empty when it has none yet.
 *
 * Read on every call rather than captured once: a workspace whose token is
 * rotated has to be reachable again by pasting the new one, without a reload.
 */
export function token(): string {
  if (!hosted) return "";
  if (ATTACHED) return ATTACHED.token;
  try {
    return window.localStorage.getItem(TOKEN_KEY) ?? "";
  } catch {
    // Private browsing, or storage disabled. The app is unusable hosted, and
    // saying so beats throwing out of an unrelated call.
    return "";
  }
}

export function setToken(value: string): void {
  if (ATTACHED) {
    // A rotated token on a box the window is showing is kept with the box's
    // address, and in memory for the rest of this load.
    ATTACHED.token = value;
    setRemote(value ? ATTACHED : null);
    return;
  }
  try {
    if (value) window.localStorage.setItem(TOKEN_KEY, value);
    else window.localStorage.removeItem(TOKEN_KEY);
  } catch {
    /* see above */
  }
}

/**
 * Takes a token out of the address bar, if one was clicked in.
 *
 * Stored and then removed from the URL in one step, so a page reloaded, a tab
 * duplicated or a link dragged to a friend carries the address and not the
 * credential. Returns whether one was found, which is what a caller uses to
 * decide whether to skip asking. Only the fragment is read; nothing else on
 * the URL means anything to this app.
 */
export function adoptInvitation(): boolean {
  if (!hosted) return false;
  const hash = window.location.hash.replace(/^#/, "");
  const found = new URLSearchParams(hash).get(FRAGMENT_KEY)?.trim();
  if (!found) return false;
  setToken(found);
  window.history.replaceState(null, "", window.location.pathname + window.location.search);
  return true;
}

/** Where the daemon is: the box this window was pointed at, else wherever
 *  this page came from. */
function origin(): string {
  return ATTACHED?.origin ?? window.location.origin;
}

/**
 * Asks a box whether a token opens it, before anything is stored.
 *
 * Two reads, both the cheapest there are: `/health` for which build the box
 * is, which is what tells a laptop and a box apart when something differs
 * between them, and `capabilities` for whether the token is accepted. Rejects
 * with the box's own refusal, or `unreachable` when nothing answered.
 */
export async function probe(candidate: Remote): Promise<{ build: string; capabilities: unknown }> {
  const base = candidate.origin.replace(/\/+$/, "");
  let health: Response;
  let answer: Response;
  try {
    health = await fetch(`${base}/health`);
    answer = await fetch(`${base}/v1/call`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${candidate.token}`,
      },
      body: JSON.stringify({ name: "capabilities", args: {} }),
    });
  } catch (cause) {
    throw {
      kind: "unreachable",
      message: `nothing answered at ${base} (${cause instanceof Error ? cause.message : cause}). Check the address, and that the box is up and reachable from here`,
    } satisfies Refusal;
  }
  const said = await health.json().catch(() => null);
  if (said?.service !== "guacad") {
    throw {
      kind: "unreachable",
      message: `${base} answered, but it is not a Guaca workspace`,
    } satisfies Refusal;
  }
  const body = await answer.json().catch(() => null);
  if (body && typeof body === "object" && "err" in body) throw (body as { err: Refusal }).err;
  if (!answer.ok) {
    throw {
      kind: "storage",
      message: `${base} answered ${answer.status} and nothing this app could read`,
    } satisfies Refusal;
  }
  return { build: String(said.build ?? ""), capabilities: (body as { ok: unknown }).ok };
}

/**
 * Calls a command on the runtime in this process, whatever the window shows.
 *
 * For the handful of things that are about this machine rather than the
 * workspace: forwarding a dropped file's bytes, feeding the menu bar. On a
 * page that is not a window there is no such runtime, and saying so is
 * better than a fetch to nowhere.
 */
export async function invokeLocal<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (!IN_A_WINDOW) {
    throw {
      kind: "config",
      message: `${name} is a desktop command, and this page is not the desktop app`,
    } satisfies Refusal;
  }
  const core = await import("@tauri-apps/api/core");
  return core.invoke<T>(name, args);
}

/**
 * The origin every file and socket address is spelled against.
 *
 * Exported for `files.ts`, which addresses a stored file by digest on this
 * origin when hosted and never needs to know how the origin was chosen.
 */
export function workspaceOrigin(): string {
  return origin();
}

/**
 * Hands one document to a hosted workspace and gets back what a message
 * carries.
 *
 * Bytes rather than a path, because a browser has no path to give. The
 * desktop's `stage_files` reads the path this side of IPC so a document never
 * enters the renderer; here the renderer is where the document already is,
 * and it crosses once. Refusals come back in the store's own words: the
 * file, its size, and the limit.
 */
export async function upload<T>(file: File): Promise<T> {
  let response: Response;
  try {
    response = await fetch(`${origin()}/v1/upload?name=${encodeURIComponent(file.name)}`, {
      method: "POST",
      headers: { authorization: `Bearer ${token()}` },
      body: file,
    });
  } catch (cause) {
    throw {
      kind: "unreachable",
      message: `could not reach this workspace to send ${file.name} (${cause instanceof Error ? cause.message : cause})`,
    } satisfies Refusal;
  }
  const body = await response.json().catch(() => null);
  if (body && typeof body === "object" && "err" in body) {
    const refused = (body as { err: Refusal }).err;
    if (refused.kind === "unauthorized") window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
    throw refused;
  }
  if (!response.ok) {
    throw {
      kind: "file",
      message: `${file.name} was not taken: this workspace answered ${response.status}`,
    } satisfies Refusal;
  }
  return (body as { ok: T }).ok;
}

/**
 * Calls one command and resolves with its value.
 *
 * Rejects with a [`Refusal`] on anything else, which is the same contract
 * Tauri's `invoke` has always had: the UI catches, reads `kind`, and draws a
 * duplicate name differently from a disk failure.
 */
export async function invoke<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (OVER_THE_BRIDGE) {
    const core = await import("@tauri-apps/api/core");
    return core.invoke<T>(name, args);
  }

  let response: Response;
  try {
    response = await fetch(`${origin()}/v1/call`, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: `Bearer ${token()}` },
      body: JSON.stringify({ name, args: args ?? {} }),
    });
  } catch (cause) {
    // The box is off, asleep, or the tunnel is down. Its own kind, because it
    // is the one failure where the thing to do is wait rather than change
    // anything: a crew keeps working while nobody can see it.
    throw {
      kind: "unreachable",
      message: `could not reach this workspace (${cause instanceof Error ? cause.message : cause}). It may be starting up, or the connection to it is down. Its agents keep working either way`,
    } satisfies Refusal;
  }

  const body = await response.json().catch(() => null);
  if (body && typeof body === "object" && "err" in body) {
    const refused = (body as { err: Refusal }).err;
    // Said once, on the window, rather than handled in every caller. A token
    // that was rotated on the box turns every call away at once, and the
    // right answer is one screen asking for the new one, not forty banners.
    if (refused.kind === "unauthorized") window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
    throw refused;
  }
  if (!response.ok) {
    throw {
      kind: "storage",
      message: `this workspace answered ${response.status} and nothing this app could read`,
    } satisfies Refusal;
  }
  return (body as { ok: T }).ok;
}

export type Unlisten = () => void;

/**
 * Subscribes to the runtime's event channel.
 *
 * On a desktop that is Tauri's own bus. Hosted it is a WebSocket that
 * reconnects, because a box is reached across a network and a network drops:
 * the desktop's channel cannot fail without the process failing, and this one
 * can fail while everything is fine.
 *
 * Reconnecting is not resynchronizing. Events missed while the socket was down
 * are gone, exactly as they are when the desktop app is closed, and the answer
 * is the same in both: what the UI draws it refetches. `onReconnect` is how a
 * caller is told to do that.
 */
export function subscribe(
  channel: string,
  handler: (payload: UiEvent) => void,
  onReconnect?: () => void,
): Promise<Unlisten> {
  if (OVER_THE_BRIDGE) {
    return import("@tauri-apps/api/event").then((events) =>
      events.listen<UiEvent>(channel, (message) => handler(message.payload)),
    );
  }
  return Promise.resolve(openSocket(handler, onReconnect));
}

/** How long to wait before trying the socket again, growing to a ceiling. */
const RETRY_FLOOR = 500;
const RETRY_CEILING = 15_000;

function openSocket(handler: (payload: UiEvent) => void, onReconnect?: () => void): Unlisten {
  let socket: WebSocket | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let wait = RETRY_FLOOR;
  let closed = false;

  const connect = () => {
    if (closed) return;
    const url = `${origin().replace(/^http/, "ws")}/v1/events?token=${encodeURIComponent(token())}`;
    socket = new WebSocket(url);

    socket.onopen = () => {
      wait = RETRY_FLOOR;
      // The first connection may have completed after the initial HTTP reads.
      // Refresh on it too, so that startup gap cannot lose a durable message.
      onReconnect?.();
    };
    socket.onmessage = (message) => {
      try {
        const event = JSON.parse(message.data as string);
        if (event.type === "streamLagged") socket?.close();
        else handler(event as UiEvent);
      } catch {
        // A frame this build cannot parse is a newer daemon talking to an
        // older page. Dropping it beats taking the channel down: everything
        // else on the socket still arrives.
      }
    };
    socket.onclose = () => {
      socket = null;
      if (closed) return;
      timer = setTimeout(connect, wait);
      // Backoff with a ceiling. A box that is rebooting comes back in seconds
      // and a tunnel that is down may be down for an hour, and hammering it
      // helps neither.
      wait = Math.min(wait * 2, RETRY_CEILING);
    };
    // `onerror` is always followed by `onclose`, so reconnecting is handled in
    // one place rather than raced between two.
    socket.onerror = () => {};
  };

  connect();

  return () => {
    closed = true;
    if (timer) clearTimeout(timer);
    socket?.close();
  };
}

/**
 * Opens a URL where the operator can see it.
 *
 * On a desktop that is the system browser rather than the webview, because the
 * session belongs to them. Hosted, this page *is* in their browser, so a new
 * tab is the same act. `noopener` because the opened page must not be handed a
 * reference back to a document holding a workspace token.
 */
export async function openExternal(url: string): Promise<void> {
  if (IN_A_WINDOW) {
    const opener = await import("@tauri-apps/plugin-opener");
    return opener.openUrl(url);
  }
  window.open(url, "_blank", "noopener,noreferrer");
}

/**
 * Raises a notification, if the operator has allowed one.
 *
 * Both hosts can, and the gates in `notify.ts` decide whether one is warranted
 * long before this is reached. Returns whether it was actually raised, because
 * a caller that believes it interrupted somebody and did not is how a parked
 * turn waits ten minutes in silence.
 */
export async function notify(title: string, body: string): Promise<boolean> {
  try {
    if (IN_A_WINDOW) {
      const plugin = await import("@tauri-apps/plugin-notification");
      const granted =
        (await plugin.isPermissionGranted()) || (await plugin.requestPermission()) === "granted";
      if (!granted) return false;
      plugin.sendNotification({ title, body });
      return true;
    }
    if (typeof Notification === "undefined") return false;
    const granted =
      Notification.permission === "granted" ||
      (await Notification.requestPermission()) === "granted";
    if (!granted) return false;
    new Notification(title, { body });
    return true;
  } catch {
    return false;
  }
}
