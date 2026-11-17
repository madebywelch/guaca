import { COMMIT, VERSION } from "./build";
import { diagnostic } from "./diagnostic";
import constants from "./mirrors/constants.json";
import { compatibility, parseHealth } from "./releases";
import type { NotificationAccess, UiEvent } from "./types";

export const desktop = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const PAGE_ID: string =
  typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : Math.random().toString(36).slice(2);

const CLIENT = { version: VERSION, build: COMMIT, desktop, id: PAGE_ID };

export interface Remote {
  origin: string;
  token: string;
}

const REMOTE_KEY = "guaca.workspace.remote";
const CONNECTION_KEY = "guaca.workspace.connection";
const MODE_KEY = "guaca.workspace.hostMode";
export type HostMode = "local" | "existing" | "remote";

function connection(): (Remote & { version: 1; mode: HostMode }) | null {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(CONNECTION_KEY) || "null");
    if (
      parsed?.version !== 1 ||
      !["local", "existing", "remote"].includes(parsed.mode) ||
      typeof parsed.origin !== "string" ||
      !parsed.origin ||
      typeof parsed.token !== "string"
    )
      return null;
    return parsed;
  } catch {
    return null;
  }
}

export function savedHostMode(): HostMode {
  const saved = connection()?.mode ?? window.localStorage.getItem(MODE_KEY);
  return saved === "local" || saved === "existing" ? saved : "remote";
}

export function saveConnection(remote: Remote, mode: HostMode): void {
  window.localStorage.setItem(CONNECTION_KEY, JSON.stringify({ version: 1, mode, ...remote }));
  try {
    window.localStorage.setItem(REMOTE_KEY, JSON.stringify(remote));
    window.localStorage.setItem(MODE_KEY, mode);
  } catch {
    console.warn("Could not update the legacy host settings.");
  }
}

function readRemote(): Remote | null {
  try {
    const current = connection();
    if (current) return { origin: current.origin.replace(/\/+$/, ""), token: current.token };
    const raw = window.localStorage.getItem(REMOTE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<Remote>;
    if (typeof parsed.origin !== "string" || typeof parsed.token !== "string") return null;
    if (!parsed.origin) return null;
    const remote = { origin: parsed.origin.replace(/\/+$/, ""), token: parsed.token };
    try {
      saveConnection(remote, savedHostMode());
    } catch {}
    return remote;
  } catch {
    return null;
  }
}

let ATTACHED: Remote | null = desktop ? readRemote() : null;
let panelGuard: { host: Remote; valid: () => Promise<boolean> } | null = null;

export function guardPanelHost(host: Remote, valid: () => Promise<boolean>): void {
  panelGuard = { host, valid };
}

async function checkPanelHost(): Promise<void> {
  const guard = panelGuard;
  if (!guard) return;
  let valid = false;
  try {
    valid = await guard.valid();
  } catch {}
  const saved = readRemote();
  if (valid && saved?.origin === guard.host.origin && saved.token === guard.host.token) return;
  try {
    restart();
  } catch {}
  throw { kind: "config", message: "The host changed. Open Guaca to reconnect." } satisfies Refusal;
}

export function attached(): Remote | null {
  return ATTACHED;
}

export function activateRemote(remote: Remote): void {
  setRemote(remote);
  ATTACHED = remote;
}

export function adoptRemote(remote: Remote): void {
  ATTACHED = remote;
}

export function setRemote(remote: Remote | null): void {
  try {
    if (remote) saveConnection(remote, savedHostMode());
    else {
      window.localStorage.removeItem(CONNECTION_KEY);
      window.localStorage.removeItem(REMOTE_KEY);
    }
  } catch {}
}

export const BEFORE_RELOAD = "guaca:before-reload";
export const RECONNECTED = "guaca:reconnected";
export function preflightReload(): void {
  if (!window.dispatchEvent(new Event(BEFORE_RELOAD, { cancelable: true }))) {
    throw new Error("Your unsent draft could not be saved. Copy it before reloading Guaca.");
  }
}
export function restart(preflight = true): void {
  if (preflight) preflightReload();
  window.location.reload();
}

export const hosted = true;

const TOKEN_KEY = "guaca.workspace.token";
let credentialGeneration = 0;
let exchangeQueue: Promise<void> = Promise.resolve();

const FRAGMENT_KEY = "token";

export const UNAUTHORIZED_EVENT = "guaca:unauthorized";

export interface Refusal {
  kind: string;
  message: string;
  detail?: string;
}

export function isUnreachable(value: unknown): boolean {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as { kind?: unknown }).kind === "unreachable"
  );
}

export function token(): string {
  if (!hosted) return "";
  if (ATTACHED) return ATTACHED.token;
  return "";
}

export function bearer(): Record<string, string> {
  const held = token();
  return held ? { authorization: `Bearer ${held}` } : {};
}

export async function setToken(value: string): Promise<void> {
  if (ATTACHED) {
    credentialGeneration++;
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
  try {
    window.localStorage.removeItem(TOKEN_KEY);
  } catch {}
}

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
    } catch {}
  });
  exchangeQueue = logout.catch(() => {});
  await logout;
  try {
    window.localStorage.removeItem(TOKEN_KEY);
  } catch {}
}

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

let PENDING_INVITATION: string | null = null;

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
  } catch {}
  return false;
}

function origin(): string {
  return ATTACHED?.origin ?? window.location.origin;
}

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

let tauriCore: Promise<typeof import("@tauri-apps/api/core")> | null = null;

export async function invokeLocal<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (!desktop) {
    throw {
      kind: "config",
      message: `${name} is a desktop command, and this page is not the desktop app`,
    } satisfies Refusal;
  }
  tauriCore ??= import("@tauri-apps/api/core");
  return (await tauriCore).invoke<T>(name, args);
}

export function workspaceOrigin(): string {
  return origin();
}

export async function upload<T>(file: File): Promise<T> {
  if (panelGuard) await checkPanelHost();
  const credential = credentialGeneration;
  let response: Response;
  try {
    response = await fetch(`${origin()}/v1/upload?name=${encodeURIComponent(file.name)}`, {
      method: "POST",
      headers: bearer(),
      body: file,
      credentials: "same-origin",
    });
  } catch {
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

export async function invoke<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (panelGuard) await checkPanelHost();
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
    throw {
      kind: "unreachable",
      message: `could not reach this host at ${origin()}. It may be starting up, or the connection to it is down. Its agents keep working either way`,
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
      message: `this host answered ${response.status} and nothing this app could read`,
    } satisfies Refusal;
  }
  return (body as { ok: T }).ok;
}

export type Unlisten = () => void;

export function subscribe(
  _channel: string,
  handler: (payload: UiEvent) => void,
  onReconnect?: () => void,
  onLost?: (error: Refusal) => void,
  onClose?: () => void,
): Promise<Unlisten> {
  return Promise.resolve(openSocket(handler, onReconnect, onLost, onClose));
}

function lostHost(): Refusal {
  return {
    kind: "unreachable",
    message: `lost the connection to this host at ${origin()}. It may be restarting, or the connection to it is down, and nothing on this page is live until it answers again`,
  };
}

const RETRY_FLOOR = 500;
const RETRY_CEILING = 15_000;

const LOST_GRACE = 1_500;
const HANDSHAKE_TIMEOUT = 10_000;

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
  let pending: ReturnType<typeof setTimeout> | null = null;
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
              if (panelGuard) await checkPanelHost();
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
      said = false;
      if (pending) {
        clearTimeout(pending);
        pending = null;
      }
      onReconnect?.();
    };
    current.onmessage = (message) => {
      if (closed || ended || attempt !== generation) return;
      try {
        const event = JSON.parse(message.data as string);
        if (event.type === "streamLagged") current.close();
        else handler(event as UiEvent);
      } catch {}
    };
    current.onclose = end;
    current.onerror = () => {};
  };

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

export async function openExternal(url: string): Promise<void> {
  if (desktop) {
    const opener = await import("@tauri-apps/plugin-opener");
    return opener.openUrl(url);
  }
  window.open(url, "_blank", "noopener,noreferrer");
}

let asking: Promise<boolean> | null = null;

export function mayNotify(): Promise<boolean> {
  asking ??= permitted().finally(() => {
    asking = null;
  });
  return asking;
}

async function permitted(): Promise<boolean> {
  try {
    if (desktop) {
      const system = await invokeLocal<NotificationAccess | null>("notification_access");
      if (system !== null) return system === "allowed" || system === "notAsked";
      const plugin = await import("@tauri-apps/plugin-notification");
      return (
        (await plugin.isPermissionGranted()) || (await plugin.requestPermission()) === "granted"
      );
    }
    if (typeof Notification === "undefined") return false;
    return (
      Notification.permission === "granted" ||
      (await Notification.requestPermission()) === "granted"
    );
  } catch {
    return false;
  }
}

const NOTIFICATION_CLICKED = constants.notificationClicked;

let askingMac: Promise<NotificationAccess | null> | null = null;

let onMac = false;

export async function willNotify(): Promise<boolean> {
  if (!(await mayNotify())) return false;
  if (!desktop) return true;
  askingMac ??= invokeLocal<NotificationAccess | null>("ask_notification_access").finally(() => {
    askingMac = null;
  });
  try {
    const system = await askingMac;
    onMac = system !== null;
    return system === null || system === "allowed";
  } catch {
    return false;
  }
}

const clicks = new Map<number, () => void>();
const CLICKS_KEPT = 64;
let nextClick = Math.floor(Math.random() * 2 ** 30);
let listening: Promise<unknown> | null = null;
let listeningOnMac: Promise<unknown> | null = null;

function remember(onClick: () => void): number {
  const id = nextClick++ % 2 ** 31;
  diagnostic("notification.remembered", id);
  clicks.set(id, onClick);
  for (const oldest of clicks.keys()) {
    if (clicks.size <= CLICKS_KEPT) break;
    clicks.delete(oldest);
  }
  return id;
}

function clicked(id: number | undefined) {
  diagnostic("notification.clicked", id ?? 0);
  const go = id === undefined ? undefined : clicks.get(id);
  if (id === undefined || !go) {
    diagnostic("notification.missing", id ?? 0);
    return;
  }
  clicks.delete(id);
  go();
}

export async function notify(title: string, body: string, onClick?: () => void): Promise<boolean> {
  if (!(await willNotify())) return false;
  try {
    if (desktop && onMac) {
      let click: number | null = null;
      if (onClick) {
        listeningOnMac ??= import("@tauri-apps/api/event")
          .then((events) =>
            events.listen<number>(NOTIFICATION_CLICKED, (message) => clicked(message.payload)),
          )
          .catch(() => {});
        await listeningOnMac;
        click = remember(onClick);
      }
      try {
        return await invokeLocal<boolean>("show_notification", { title, body, click });
      } catch {
        if (click !== null) clicks.delete(click);
        return false;
      }
    }
    if (desktop) {
      const plugin = await import("@tauri-apps/plugin-notification");
      if (!onClick) {
        plugin.sendNotification({ title, body });
        return true;
      }
      listening ??= plugin
        .onAction((action) => {
          if (action.actionId === "tap") clicked(action.notification?.id);
        })
        .catch(() => {});
      await listening;
      plugin.sendNotification({ id: remember(onClick), title, body });
      return true;
    }
    const shown = new Notification(title, { body });
    if (onClick) {
      shown.onclick = () => {
        window.focus();
        shown.close();
        onClick();
      };
    }
    return true;
  } catch {
    return false;
  }
}
