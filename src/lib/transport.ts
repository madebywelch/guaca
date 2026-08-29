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

/** Whether the runtime is somewhere other than this process. */
export const hosted = !IN_A_WINDOW;

const TOKEN_KEY = "guaca.workspace.token";

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
  try {
    return window.localStorage.getItem(TOKEN_KEY) ?? "";
  } catch {
    // Private browsing, or storage disabled. The app is unusable hosted, and
    // saying so beats throwing out of an unrelated call.
    return "";
  }
}

export function setToken(value: string): void {
  try {
    if (value) window.localStorage.setItem(TOKEN_KEY, value);
    else window.localStorage.removeItem(TOKEN_KEY);
  } catch {
    /* see above */
  }
}

/** Where the daemon is, which is wherever this page came from. */
function origin(): string {
  return window.location.origin;
}

/**
 * Calls one command and resolves with its value.
 *
 * Rejects with a [`Refusal`] on anything else, which is the same contract
 * Tauri's `invoke` has always had: the UI catches, reads `kind`, and draws a
 * duplicate name differently from a disk failure.
 */
export async function invoke<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (IN_A_WINDOW) {
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
    throw (body as { err: Refusal }).err;
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
  if (IN_A_WINDOW) {
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
  // The first open is not a reconnect. Telling the caller to refetch on it
  // would double every read the app already does when it mounts.
  let opened = false;

  const connect = () => {
    if (closed) return;
    const scheme = window.location.protocol === "https:" ? "wss" : "ws";
    const url = `${scheme}://${window.location.host}/v1/events?token=${encodeURIComponent(token())}`;
    socket = new WebSocket(url);

    socket.onopen = () => {
      wait = RETRY_FLOOR;
      if (opened) onReconnect?.();
      opened = true;
    };
    socket.onmessage = (message) => {
      try {
        handler(JSON.parse(message.data as string) as UiEvent);
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
