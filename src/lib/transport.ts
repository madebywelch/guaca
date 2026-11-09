/** One backend transport for browsers and the native desktop client. */

import { COMMIT, VERSION } from "./build";
import { compatibility, parseHealth } from "./releases";
import type { UiEvent } from "./types";

/** Tauri v2 puts this on `window` before any of our code runs. */
export const desktop = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** This page load, so a sign-in the host opens for one window opens there only. */
export const PAGE_ID: string =
  typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : Math.random().toString(36).slice(2);

/** Said on every call, so a host of another version or build can name the
 *  difference and a sign-in finds its way back to this page. In the body,
 *  because a header an older host's CORS does not admit would fail every
 *  preflight. */
const CLIENT = { version: VERSION, build: COMMIT, desktop, id: PAGE_ID };

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

let ATTACHED: Remote | null = desktop ? readRemote() : null;

/** The host this window is showing, or null before desktop setup. */
export function attached(): Remote | null {
  return ATTACHED;
}

/**
 * Points the desktop app at a box, or back at itself. Takes effect on the
 * next load, and the caller is what reloads: everything about which host this
 * is was decided at import time, on purpose, so there is nothing to update in
 * place and nothing that can be half-updated.
 */
export function activateRemote(remote: Remote): void {
  ATTACHED = remote;
  setRemote(remote);
}

/**
 * Shows a host this page was handed rather than one it chose.
 *
 * The menu bar panel's, which is told by the tray which host the window is
 * attached to. Held for this load and never stored: what is stored is the
 * window's decision, and a second page writing it back is a second place for
 * it to be decided.
 */
export function adoptRemote(remote: Remote): void {
  ATTACHED = remote;
}

export function setRemote(remote: Remote | null): void {
  try {
    if (remote) window.localStorage.setItem(REMOTE_KEY, JSON.stringify(remote));
    else window.localStorage.removeItem(REMOTE_KEY);
  } catch {
    /* storage disabled; the next load reads nothing and shows this machine */
  }
}

/** Reloads the page, which is how a change of workspace takes effect. */
export const BEFORE_RELOAD = "guaca:before-reload";
export const RECONNECTED = "guaca:reconnected";
export function restart(): void {
  if (!window.dispatchEvent(new Event(BEFORE_RELOAD, { cancelable: true }))) {
    throw new Error("Your unsent draft could not be saved. Copy it before reloading Guaca.");
  }
  window.location.reload();
}

/** Whether the runtime is somewhere other than this process. */
export const hosted = true;

/** Whether calls go over Tauri's own bridge, to the runtime in this process. */

const TOKEN_KEY = "guaca.workspace.token";
let credentialGeneration = 0;
let exchangeQueue: Promise<void> = Promise.resolve();

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
  /**
   * The layer underneath's own words, where the message above them is Guaca's.
   *
   * Only some refusals have one, and it is never the first thing to read: it is
   * what a bug report needs, and what an operator who has read the sentence and
   * still wants to know exactly what was said opens. A refusal that is already
   * a sentence for a person carries none.
   */
  detail?: string;
}

/**
 * Whether a failure is the host being gone rather than anything on it.
 *
 * Its own kind on the refusal rather than a message match, and exported because
 * the banner is not the only place that has a different next step for it: a
 * host that is restarting is waited out, and everything else in this app is
 * changed.
 */
export function isUnreachable(value: unknown): boolean {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as { kind?: unknown }).kind === "unreachable"
  );
}

/**
 * The token this client presents in a header or a query, or empty when it has
 * none.
 *
 * A browser has none by design: its credential is the HttpOnly session cookie
 * the daemon issued, which no script can read. A window showing a box keeps
 * the token beside the box's address, and that is the only script-readable
 * copy left. Read on every call rather than captured once, so a token rotated
 * and pasted in this window takes effect without a reload.
 */
export function token(): string {
  if (!hosted) return "";
  if (ATTACHED) return ATTACHED.token;
  // A browser client authenticates by the HttpOnly session cookie the daemon
  // issues at sign-in. The token is not kept in script-readable storage after
  // the exchange: a script on this origin cannot read the cookie, and there is
  // nothing in localStorage for it to find. A desktop window keeps the token
  // for the box it was pointed at, which is a host the operator chose rather
  // than the origin that served the page.
  return "";
}

/**
 * The Authorization header this host's calls carry, or none.
 *
 * A browser client authenticates by the HttpOnly session cookie the daemon
 * issued at sign-in, so it holds no script-readable token and sends no header:
 * the cookie rides along because every call asks for credentials. A desktop
 * window pointed at a box, and every non-browser client, still presents the
 * token it holds. An empty `Bearer ` would be a header the daemon has to parse
 * and turn away, so the header is left off rather than sent empty.
 */
export function bearer(): Record<string, string> {
  const held = token();
  return held ? { authorization: `Bearer ${held}` } : {};
}

/**
 * Exchanges a bearer token for the HttpOnly session cookie.
 *
 * The daemon sets the cookie on the response and the browser sends it from
 * then on. The token itself is not stored: a browser client never holds it in
 * script-readable storage. A token rotated on the box, or a browser whose
 * storage was cleared, goes through this same exchange after pasting.
 */
export async function setToken(value: string): Promise<void> {
  if (ATTACHED) {
    credentialGeneration++;
    // A rotated token on a box the window is showing is kept with the box's
    // address, and in memory for the rest of this load.
    ATTACHED.token = value;
    setRemote(value ? ATTACHED : null);
    return;
  }
  if (!hosted) return;
  if (!value) {
    await signOut();
    return;
  }
  await exchangeToken(value);
}

/**
 * Exchanges a token for the session cookie without keeping the token.
 *
 * Called by `setToken` and by the migration path that reads a token an older
 * build left in localStorage. Either way the token crosses once, the cookie is
 * set, and the script-readable copy is deleted.
 */
export async function exchangeToken(value: string): Promise<void> {
  if (!hosted) return;
  const credential = ++credentialGeneration;
  if (ATTACHED) {
    ATTACHED.token = value;
    setRemote(value ? ATTACHED : null);
    return;
  }
  const exchange = exchangeQueue.then(() => exchangeBrowserToken(value, credential));
  exchangeQueue = exchange.catch(() => {});
  return exchange;
}

async function exchangeBrowserToken(value: string, credential: number): Promise<void> {
  const base = origin();
  let response: Response;
  try {
    response = await fetch(`${base}/v1/auth/session`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${value}`,
      },
      body: JSON.stringify({}),
      // The cookie is set for this origin and only this one: a browser client
      // is served by the daemon it talks to, and a window showing a box
      // exchanges nothing, because it keeps its token instead.
      credentials: "same-origin",
      signal: AbortSignal.timeout(15_000),
    });
  } catch {
    throw {
      kind: "unreachable",
      message: `could not reach this host at ${base}. It may be starting up, or the connection to it is down. Its agents keep working either way`,
    } satisfies Refusal;
  }
  const body = await response.json().catch(() => null);
  if (body && typeof body === "object" && "err" in body) {
    const refused = (body as { err: Refusal }).err;
    if (refused.kind === "unauthorized" && credential === credentialGeneration) {
      window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
    }
    throw refused;
  }
  if (!response.ok) {
    throw {
      kind: "storage",
      message: `${base} answered ${response.status} and nothing this app could read`,
    } satisfies Refusal;
  }
  // The cookie is set. Clear a token an older build left behind so a script on
  // this origin cannot read it again.
  try {
    window.localStorage.removeItem(TOKEN_KEY);
  } catch {
    /* storage disabled; nothing to clear */
  }
}

/**
 * Clears the session cookie on the daemon's side and forgets the token.
 *
 * The cookie is HttpOnly, so the daemon is the only one who can clear it. A
 * request to the logout route does that even when the page has already lost
 * its connection; the local copy goes either way.
 */
export async function signOut(): Promise<void> {
  if (!hosted) return;
  credentialGeneration++;
  if (ATTACHED) {
    ATTACHED.token = "";
    setRemote(null);
    return;
  }
  const logout = exchangeQueue.then(async () => {
    try {
      await fetch(`${origin()}/v1/auth/logout`, {
        method: "POST",
        credentials: "same-origin",
        signal: AbortSignal.timeout(15_000),
      });
    } catch {
      /* the cookie is HttpOnly and the daemon clears it when reachable */
    }
  });
  exchangeQueue = logout.catch(() => {});
  await logout;
  try {
    window.localStorage.removeItem(TOKEN_KEY);
  } catch {
    /* storage disabled; nothing to clear */
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
  credentialGeneration++;
  PENDING_INVITATION = found;
  window.history.replaceState(null, "", window.location.pathname + window.location.search);
  return true;
}

/**
 * The invitation token taken out of the address bar, waiting for its exchange.
 *
 * Held here rather than in storage so a page reloaded or a tab duplicated
 * cannot replay it. `settleInvitation` is the one caller, and it clears this
 * whatever the exchange answers.
 */
let PENDING_INVITATION: string | null = null;

/**
 * Turns an invitation or a legacy localStorage token into a session cookie.
 *
 * Returns whether a session now exists. An invitation token is exchanged and
 * forgotten. A token an older build left in localStorage is exchanged and
 * deleted, so nobody already signed in gets locked out by the change.
 */
export async function settleInvitation(): Promise<boolean> {
  if (!hosted) return false;
  const pending = PENDING_INVITATION;
  PENDING_INVITATION = null;
  if (pending) {
    try {
      await exchangeToken(pending);
      return true;
    } catch {
      return false;
    }
  }
  try {
    const stored = window.localStorage.getItem(TOKEN_KEY);
    if (stored) {
      await exchangeToken(stored);
      return true;
    }
  } catch {
    /* storage disabled, or the exchange was refused: either way the token is
       left where it is, so a host that comes back can still be migrated */
  }
  return false;
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
    health = await fetch(`${base}/health`, { signal: AbortSignal.timeout(15000) });
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
      message: `${base} answered, but it is not a Guaca host`,
    } satisfies Refusal;
  }
  const match = compatibility(parseHealth(said));
  if (match === "hostOld" || match === "clientOld") {
    throw {
      kind: "compatibility",
      message:
        match === "clientOld"
          ? "This host needs a newer Guaca frontend. Download the latest desktop app, or reload the browser from this host."
          : "This Guaca frontend needs a newer host. Update the backend before connecting.",
    } satisfies Refusal;
  }
  try {
    answer = await fetch(`${base}/v1/call`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${candidate.token}`,
      },
      body: JSON.stringify({ name: "capabilities", args: {} }),
      signal: AbortSignal.timeout(15000),
    });
  } catch (cause) {
    throw {
      kind: "unreachable",
      message: `Could not authenticate with ${base}: ${cause instanceof Error ? cause.message : cause}. Try again.`,
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
 * Calls a native client command, whatever host the window shows.
 *
 * For the handful of things that are about this machine rather than the
 * workspace: forwarding a dropped file's bytes, feeding the menu bar. On a
 * page that is not a window there is no such runtime, and saying so is
 * better than a fetch to nowhere.
 */
export async function invokeLocal<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (!desktop) {
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
 * desktop reads a dropped path natively and posts it to the same route, so a
 * document never enters its renderer; here the renderer is where the document
 * already is, and it crosses once. Refusals come back in the store's own words: the
 * file, its size, and the limit.
 */
export async function upload<T>(file: File): Promise<T> {
  const credential = credentialGeneration;
  let response: Response;
  try {
    response = await fetch(`${origin()}/v1/upload?name=${encodeURIComponent(file.name)}`, {
      method: "POST",
      headers: bearer(),
      body: file,
      // Same-origin asks for the session cookie; a window showing a box sends
      // its token in `bearer()` instead and asks for no credentials, which is
      // also what keeps a cross-origin answer readable without CORS credentials.
      credentials: "same-origin",
    });
  } catch {
    // Said about the file rather than about the string a browser's `fetch`
    // throws: the operator's question is whether the document went, and the
    // answer is no.
    throw {
      kind: "unreachable",
      message: `could not reach this host to send ${file.name}. Try attaching it again once the host is back`,
    } satisfies Refusal;
  }
  const body = await response.json().catch(() => null);
  if (body && typeof body === "object" && "err" in body) {
    const refused = (body as { err: Refusal }).err;
    if (refused.kind === "unauthorized" && credential === credentialGeneration) {
      window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
    }
    throw refused;
  }
  if (!response.ok) {
    throw {
      kind: "file",
      message: `${file.name} was not taken: this host answered ${response.status}`,
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
  const credential = credentialGeneration;
  let response: Response;
  try {
    response = await fetch(`${origin()}/v1/call`, {
      method: "POST",
      headers: { "content-type": "application/json", ...bearer() },
      body: JSON.stringify({ name, args: args ?? {}, client: CLIENT }),
      credentials: "same-origin",
    });
  } catch {
    // The box is off, asleep, or the tunnel is down. Its own kind, because it
    // is the one failure where the thing to do is wait rather than change
    // anything: a crew keeps working while nobody can see it.
    //
    // The address is in the sentence and the browser's own words are not. The
    // cause of a `fetch` that fails is one of four strings — "Failed to fetch"
    // for all of them — so it tells the operator nothing the address does not,
    // and reads as a stack trace in the one place that has to sound like a
    // colleague. What to do about it is the banner's, which is the only thing
    // that knows whether this host is this Mac's or a box's.
    throw {
      kind: "unreachable",
      message: `could not reach this host at ${origin()}. It may be starting up, or the connection to it is down. Its agents keep working either way`,
    } satisfies Refusal;
  }

  const body = await response.json().catch(() => null);
  if (body && typeof body === "object" && "err" in body) {
    const refused = (body as { err: Refusal }).err;
    // Said once, on the window, rather than handled in every caller. A token
    // that was rotated on the box turns every call away at once, and the
    // right answer is one screen asking for the new one, not forty banners.
    if (refused.kind === "unauthorized" && credential === credentialGeneration) {
      window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
    }
    throw refused;
  }
  if (!response.ok) {
    throw {
      kind: "storage",
      message: `this host answered ${response.status} and nothing this app could read`,
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
 *
 * `onLost` is the other direction and a slower question. A redial usually
 * answers, and the page that announced a lost host a moment before the host
 * came back is a page that took it back. So the loss is only reported once the
 * redial has had its chance, which is also what keeps a socket dropped on
 * purpose — this page drops a lagging one, and a host does the same — from
 * reading as a host that left.
 */
export function subscribe(
  _channel: string,
  handler: (payload: UiEvent) => void,
  onReconnect?: () => void,
  onLost?: (error: Refusal) => void,
  onClose?: () => void,
): Promise<Unlisten> {
  return Promise.resolve(openSocket(handler, onReconnect, onLost, onClose));
}

/**
 * Nothing is arriving, said as the refusal the rest of the app already reads.
 *
 * The socket knows the host left without being asked, and the sentence it
 * deserves is the one a call that could not reach it gets: the address, and
 * what it may be. The half that is different is what this one costs — a page
 * that is not receiving is a page whose transcript is not current — so it says
 * that instead of claiming the agents are unaffected, which is a thing a dead
 * host cannot be known to be.
 */
function lostHost(): Refusal {
  return {
    kind: "unreachable",
    message: `lost the connection to this host at ${origin()}. It may be restarting, or the connection to it is down, and nothing on this page is live until it answers again`,
  };
}

/** How long to wait before trying the socket again, growing to a ceiling. */
const RETRY_FLOOR = 500;
const RETRY_CEILING = 15_000;

/**
 * How long a closed socket is given to come back before its close is news.
 *
 * The first redial waits at most `RETRY_FLOOR`, and a host that is answering
 * takes the connection in milliseconds, so anything that returns inside this
 * window was never gone. What is left is a host that is not answering, which is
 * what the mark and the banner are for.
 */
const LOST_GRACE = 1_500;
const HANDSHAKE_TIMEOUT = 10_000;

/**
 * The wait before one retry: half the backoff, and a random share of the rest.
 *
 * Every window on a host loses the socket at the same moment when the host
 * restarts, and without the random half they all redial in step and keep
 * doing so. The fixed half keeps a floor under it, so a host that is starting
 * is not asked again before it could have answered.
 */
export function retryDelay(wait: number, random: () => number = Math.random): number {
  return Math.round(wait / 2 + (random() * wait) / 2);
}

function openSocket(
  handler: (payload: UiEvent) => void,
  onReconnect?: () => void,
  onLost?: (error: Refusal) => void,
  onClose?: () => void,
): Unlisten {
  let socket: WebSocket | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  /** The pending "the host is gone", cancelled by a redial that answers. */
  let pending: ReturnType<typeof setTimeout> | null = null;
  /** Whether this outage has already been reported, so one silence is one sentence. */
  let said = false;
  let wait = RETRY_FLOOR;
  let closed = false;
  let handshake: ReturnType<typeof setTimeout> | null = null;
  let generation = 0;
  let recovery = 0;
  let authTimer: ReturnType<typeof setTimeout> | null = null;

  const connect = () => {
    if (closed) return;
    const socketToken = token();
    const base = origin().replace(/^http/, "ws");
    const url = socketToken
      ? `${base}/v1/events?token=${encodeURIComponent(socketToken)}`
      : `${base}/v1/events`;
    const current = new WebSocket(url);
    socket = current;
    const attempt = ++generation;
    let ended = false;

    const end = () => {
      if (closed || ended || attempt !== generation) return;
      ended = true;
      if (handshake) clearTimeout(handshake);
      handshake = null;
      socket = null;
      onClose?.();
      if (!said && !pending) {
        pending = setTimeout(() => {
          pending = null;
          if (closed || said) return;
          said = true;
          onLost?.(lostHost());
          const before = recovery;
          const credential = credentialGeneration;
          const probe = async () => {
            let refused = false;
            try {
              const response = await fetch(`${origin()}/v1/call`, {
                method: "POST",
                headers: { "content-type": "application/json", ...bearer() },
                body: JSON.stringify({ name: "capabilities", args: {}, client: CLIENT }),
                credentials: "same-origin",
                signal: AbortSignal.timeout(5_000),
              });
              if (response.status !== 401) return;
              const body = await response.json();
              if (
                !closed &&
                said &&
                recovery === before &&
                credential === credentialGeneration &&
                body?.err?.kind === "unauthorized"
              ) {
                refused = true;
                window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
              }
            } catch {
            } finally {
              if (
                !refused &&
                !closed &&
                said &&
                recovery === before &&
                credential === credentialGeneration
              ) {
                authTimer = setTimeout(() => void probe(), RETRY_CEILING);
              }
            }
          };
          void probe();
        }, LOST_GRACE);
      }
      timer = setTimeout(connect, retryDelay(wait));
      wait = Math.min(wait * 2, RETRY_CEILING);
    };
    handshake = setTimeout(() => {
      end();
      current.close();
    }, HANDSHAKE_TIMEOUT);

    current.onopen = () => {
      if (closed || ended || attempt !== generation) return;
      if (handshake) clearTimeout(handshake);
      handshake = null;
      recovery++;
      if (authTimer) clearTimeout(authTimer);
      authTimer = null;
      window.dispatchEvent(new Event(RECONNECTED));
      wait = RETRY_FLOOR;
      // The redial answered: whatever closed the last socket was not the host
      // leaving, and a page that said it had would be taking it back a moment
      // later.
      said = false;
      if (pending) {
        clearTimeout(pending);
        pending = null;
      }
      // The first connection may have completed after the initial HTTP reads.
      // Refresh on it too, so that startup gap cannot lose a durable message.
      onReconnect?.();
    };
    current.onmessage = (message) => {
      if (closed || ended || attempt !== generation) return;
      try {
        const event = JSON.parse(message.data as string);
        // Sent by hosts older than the one that closes a lagging socket itself.
        // A page newer than its host still has to resynchronize on it.
        if (event.type === "streamLagged") current.close();
        else handler(event as UiEvent);
      } catch {
        // A frame this build cannot parse is a newer daemon talking to an
        // older page. Dropping it beats taking the channel down: everything
        // else on the socket still arrives.
      }
    };
    current.onclose = end;
    // `onerror` is always followed by `onclose`, so reconnecting is handled in
    // one place rather than raced between two.
    current.onerror = () => {};
  };

  // A laptop waking or a network coming back is the moment the host is most
  // likely to answer, and a backoff grown long while it slept would sit out
  // the next fifteen seconds for nothing.
  const online = () => {
    if (closed || socket) return;
    if (timer) clearTimeout(timer);
    wait = RETRY_FLOOR;
    connect();
  };
  window.addEventListener("online", online);

  connect();

  return () => {
    closed = true;
    window.removeEventListener("online", online);
    if (timer) clearTimeout(timer);
    if (pending) clearTimeout(pending);
    if (handshake) clearTimeout(handshake);
    if (authTimer) clearTimeout(authTimer);
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
  if (desktop) {
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
    if (desktop) {
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
