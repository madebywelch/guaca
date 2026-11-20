import { type ReactNode, useCallback, useEffect, useId, useRef, useState } from "react";
import {
  type AppSource,
  asStall,
  type DockerStatus,
  type Elsewhere,
  type ExistingHost,
  hostMode,
  localHost,
  type Stall,
  thisApp,
} from "../lib/host";
import { INSTRUCTIONS, RELEASES } from "../lib/releases";
import {
  activateRemote,
  adoptRemote,
  attached,
  desktop,
  type HostMode,
  openExternal,
  preflightReload,
  probe,
  type Remote,
  restart,
  saveConnection,
} from "../lib/transport";
import { errorMessage } from "../lib/types";
import { LegacyGroups } from "./GroupTransfer";

const PROBE_DEADLINE_MS = 15_000;
const ENGINE_POLL_MS = 3_000;
const ENGINE_POLLS = 100;

function within<T>(work: Promise<T>, message: string): Promise<T> {
  let deadline: ReturnType<typeof setTimeout> | undefined;
  return Promise.race([
    work,
    new Promise<never>((_, reject) => {
      deadline = setTimeout(() => reject(new Error(message)), PROBE_DEADLINE_MS);
    }),
  ]).finally(() => clearTimeout(deadline));
}

function commit(connection: Remote, mode: HostMode, onConnected?: () => void) {
  preflightReload();
  try {
    saveConnection(connection, mode);
  } catch {
    throw new Error("Guaca could not save this choice. Nothing changed.");
  }
  if (onConnected) {
    adoptRemote(connection);
    onConnected();
  } else {
    restart(false);
  }
}

export function StartOver({ onConnected }: { onConnected?: () => void }) {
  const [plan, setPlan] = useState<Elsewhere | null>(null);
  const [open, setOpen] = useState(false);
  const [sure, setSure] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [imageFailed, setImageFailed] = useState(false);
  const [looked, setLooked] = useState(true);
  const [unanswered, setUnanswered] = useState("");
  useEffect(() => {
    if (!looked) return;
    void localHost
      .elsewhere()
      .then(setPlan)
      .catch(() => setLooked(false));
  }, [looked]);
  if (unanswered) return <p role="alert">{unanswered}</p>;
  if (!looked)
    return (
      <p className="field__hint">Guaca could not check the other Docker engines on this Mac.</p>
    );
  const from = plan?.from;
  if (!from) return null;
  const to = plan.to;
  if (!to)
    return (
      <p className="field__hint">
        No other Docker engine is running on this Mac, so a new host cannot be set up elsewhere yet.
      </p>
    );
  const cancel = () => {
    setOpen(false);
    setSure(false);
    setError("");
  };
  const confirm = async () => {
    setBusy(true);
    setError("");
    setImageFailed(false);
    let connection: Remote;
    try {
      connection = await localHost.startElsewhere(to.context, plan.toEngine ?? "");
    } catch (cause) {
      setError(errorMessage(cause));
      setImageFailed(asStall(cause)?.kind === "image");
      setBusy(false);
      return;
    }
    try {
      await probe(connection);
      commit(connection, "local", onConnected);
    } catch (cause) {
      setUnanswered(
        `The new host in ${to.label} was set up, but it did not answer: ${errorMessage(cause)} Go back to your host to try again.`,
      );
    } finally {
      setBusy(false);
    }
  };
  if (!open)
    return (
      <div className="access__row">
        <button className="btn btn--quiet" type="button" onClick={() => setOpen(true)}>
          Set up a new host in {to.label}
        </button>
      </div>
    );
  return (
    <section className="field" aria-label={`Set up a new host in ${to.label}`}>
      <span className="field__label">Set up a new host in {to.label}</span>
      <p>
        Your current host is recorded in {from.label}. Its agents and conversations are not moved,
        and they will not appear in the new host.
      </p>
      <p>
        The host in {from.label} and its data are left untouched. Once {from.label} runs again, Host
        settings offer to go back to it.
      </p>
      {plan.data && (
        <p>
          {to.label} already holds Guaca data from an earlier host, so the new host starts with that
          data rather than empty.
        </p>
      )}
      <label className="field field--row">
        <input
          type="checkbox"
          checked={sure}
          disabled={busy}
          onChange={(event) => setSure(event.target.checked)}
        />
        <span>I understand that my agents stay with the host in {from.label}.</span>
      </label>
      {error && (
        <div className="banner banner--error" role="alert">
          {error}
        </div>
      )}
      {imageFailed && <ImageRecovery />}
      <div className="access__row">
        <button
          className="btn btn--compact"
          type="button"
          disabled={!sure || busy}
          onClick={() => void confirm()}
        >
          {busy ? "Setting up the new host…" : "Set up the new host"}
        </button>
        <button className="btn btn--quiet" type="button" disabled={busy} onClick={cancel}>
          Cancel
        </button>
      </div>
    </section>
  );
}

export function HostSetup({ children }: { children: ReactNode }) {
  const [ready, setReady] = useState(!desktop);
  const done = useCallback(() => setReady(true), []);
  if (ready) return children;
  if (attached() && hostMode() === "local") return <LocalReconnect onReady={done} />;
  return <HostPicker onReady={done} />;
}

type Reconnect =
  | { kind: "connecting" }
  | { kind: "stalled"; stall: Stall }
  | { kind: "gaveUp"; stall: Stall };

const TITLES: Record<Stall["kind"] | "connecting" | "gaveUp", string> = {
  connecting: "Connecting to your host",
  engine: "Waiting for your host",
  gaveUp: "Your host is not running",
  choose: "Which host is yours?",
  missing: "Guaca can't find your host",
  image: "Your host could not start",
  failed: "Your host could not start",
};

function LocalReconnect({ onReady }: { onReady: () => void }) {
  const saved = attached();
  const [state, setState] = useState<Reconnect>({ kind: "connecting" });
  const [attempt, setAttempt] = useState(0);
  const [chosen, setChosen] = useState<string | null>(null);
  const [choosing, setChoosing] = useState<"another" | "new" | null>(null);
  const [opening, setOpening] = useState("");
  useEffect(() => {
    if (choosing) return;
    let disposed = false;
    let pause: ReturnType<typeof setTimeout> | undefined;
    const waited = () =>
      new Promise<void>((resolve) => {
        pause = setTimeout(resolve, ENGINE_POLL_MS);
      });
    const reach = async () => {
      for (let polls = 0; !disposed; polls++) {
        try {
          const connection = await (chosen ? localHost.choose(chosen) : localHost.resume());
          await within(
            probe(connection),
            `Couldn't connect to your host at ${connection.origin}. Check Docker's Guaca container, then try again.`,
          );
          if (disposed) return;
          activateRemote(connection);
          onReady();
          return;
        } catch (cause) {
          if (disposed) return;
          const stall: Stall = asStall(cause) ?? {
            kind: "failed",
            message: errorMessage(cause),
          };
          if (stall.kind !== "engine") return setState({ kind: "stalled", stall });
          if (polls >= ENGINE_POLLS) return setState({ kind: "gaveUp", stall });
          setState({ kind: "stalled", stall });
          await waited();
        }
      }
    };
    void reach();
    return () => {
      disposed = true;
      clearTimeout(pause);
    };
  }, [attempt, chosen, choosing, onReady]);
  const retry = () => {
    setChoosing(null);
    setOpening("");
    setState({ kind: "connecting" });
    setAttempt((n) => n + 1);
  };
  if (choosing)
    return (
      <main className="threshold">
        <section className="host-setup">
          <h1 className="empty__title">Where should your agents work?</h1>
          {choosing === "new" && (
            <p className="field__hint">
              A new host starts without the agents of the host Guaca can't find.
            </p>
          )}
          <div className="access__row">
            <button className="btn btn--quiet" type="button" onClick={retry}>
              Back to your host
            </button>
          </div>
          <StartOver onConnected={onReady} />
          <HostChoice onConnected={onReady} />
          <LegacyGroups />
        </section>
      </main>
    );
  const stall = state.kind === "connecting" ? null : state.stall;
  const title =
    TITLES[
      state.kind === "connecting"
        ? "connecting"
        : state.kind === "gaveUp"
          ? "gaveUp"
          : state.stall.kind
    ];
  const open = (context: string) => {
    setOpening("");
    void localHost.openDocker(context).catch((cause) => setOpening(errorMessage(cause)));
  };
  return (
    <main className="threshold">
      <section className="host-setup">
        <h1 className="empty__title">{title}</h1>
        <div className="host-choice">
          {!stall ? (
            <p role="status">Connecting to your host on this Mac…</p>
          ) : stall.kind === "engine" ? (
            <>
              <p role="status">{stall.message}</p>
              <p className="field__hint">
                {state.kind === "gaveUp"
                  ? "Guaca stopped waiting after five minutes. Start it, then try again."
                  : "Guaca reconnects by itself as soon as it starts."}
              </p>
            </>
          ) : (
            <p role={stall.kind === "choose" ? "status" : "alert"}>{stall.message}</p>
          )}
          {stall?.kind === "missing" && saved && (
            <p className="field__hint">Guaca last reached it at {saved.origin}.</p>
          )}
          {opening && <p role="alert">{opening}</p>}
          <div className="access__row">
            {stall?.kind === "engine" &&
              stall.engines
                .filter((engine) => engine.action)
                .map((engine) => (
                  <button
                    key={engine.context}
                    className="btn btn--compact"
                    type="button"
                    onClick={() => open(engine.context)}
                  >
                    {engine.action}
                  </button>
                ))}
            {stall?.kind === "choose" &&
              stall.engines.map((engine) => (
                <button
                  key={engine.context}
                  className="btn btn--compact"
                  type="button"
                  onClick={() => {
                    setState({ kind: "connecting" });
                    setChosen(engine.context);
                  }}
                >
                  Use the host in {engine.label}
                </button>
              ))}
            {stall &&
              (state.kind === "gaveUp" || stall.kind !== "engine") &&
              stall.kind !== "choose" && (
                <button className="btn btn--compact" type="button" onClick={retry}>
                  Try again
                </button>
              )}
            {stall?.kind === "missing" ? (
              <button className="btn btn--quiet" type="button" onClick={() => setChoosing("new")}>
                Set up a new host
              </button>
            ) : (
              <button
                className="btn btn--quiet"
                type="button"
                onClick={() => setChoosing("another")}
              >
                Choose another host
              </button>
            )}
          </div>
          {state.kind === "gaveUp" && <StartOver onConnected={onReady} />}
        </div>
        <LegacyGroups />
      </section>
    </main>
  );
}

export function ImageRecovery() {
  const [source, setSource] = useState<AppSource | null>(null);
  const [said, setSaid] = useState("");
  useEffect(() => {
    void thisApp
      .source()
      .then(setSource)
      .catch(() => setSource(null));
  }, []);
  if (source?.checkout && !source.unavailable)
    return (
      <div className="field">
        <p className="field__hint">
          Rebuilding Guaca from {source.checkout} fetches or builds the host image it needs, then
          reopens Guaca.
        </p>
        <div className="access__row">
          <button
            className="btn btn--compact"
            type="button"
            disabled={source.running}
            onClick={() =>
              void thisApp
                .rebuild()
                .then(() =>
                  setSaid("Guaca is rebuilding. It closes and reopens when the build finishes."),
                )
                .catch((cause) => setSaid(errorMessage(cause)))
            }
          >
            Rebuild Guaca
          </button>
        </div>
        {said && <p role="status">{said}</p>}
      </div>
    );
  return (
    <p className="field__hint">
      The current Guaca brings a host image the registry serves.{" "}
      <a href={RELEASES} target="_blank" rel="noopener noreferrer">
        Download Guaca
      </a>
    </p>
  );
}

function HostPicker({ onReady }: { onReady: () => void }) {
  const [checking, setChecking] = useState(desktop && !!attached());
  const [error, setError] = useState("");
  const [chooser, setChooser] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const choosing = useRef(false);
  const saved = attached();
  const origin = saved?.origin;
  const name = saved ? hostName(saved.origin) : "your host";
  useEffect(() => {
    if (!desktop || !attached()) return;
    let disposed = false;
    const connect = async () => {
      try {
        const connection = attached()!;
        await within(
          probe(connection),
          `Couldn't connect to ${name} at ${connection.origin}. It may be off or offline.`,
        );
        if (!disposed && !choosing.current) {
          activateRemote(connection);
          onReady();
        }
      } catch (cause) {
        if (!disposed) {
          const message = errorMessage(cause);
          setError(
            message.startsWith("nothing answered at ") && origin
              ? `${name} isn't answering at ${origin}. It may be off or offline.`
              : message,
          );
          choosing.current = true;
          setChooser(true);
        }
      } finally {
        if (!disposed) setChecking(false);
      }
    };
    void connect();
    return () => {
      disposed = true;
    };
  }, [attempt, name, origin, onReady]);
  return (
    <main className="threshold">
      <section className="host-setup">
        <h1 className="empty__title">Where should your agents work?</h1>
        {saved ? (
          <div className="host-choice">
            {checking ? (
              <div role="status">
                <p>Connecting to {name}…</p>
                <p className="field__hint">{saved.origin}</p>
              </div>
            ) : error ? (
              <p role="alert">{error}</p>
            ) : null}
            <div className="access__row">
              {!checking && (
                <button
                  className="btn btn--compact"
                  type="button"
                  onClick={() => {
                    choosing.current = false;
                    setChooser(false);
                    setError("");
                    setChecking(true);
                    setAttempt((n) => n + 1);
                  }}
                >
                  Try again
                </button>
              )}
              {!chooser && (
                <button
                  className="btn btn--compact"
                  type="button"
                  onClick={() => {
                    choosing.current = true;
                    setChooser(true);
                  }}
                >
                  Choose another host
                </button>
              )}
            </div>
            {!chooser && (
              <p className="field__hint">Choosing another host does not move this host's groups.</p>
            )}
            {chooser && <HostChoice onConnected={onReady} />}
          </div>
        ) : (
          <HostChoice onConnected={onReady} />
        )}
        <LegacyGroups />
      </section>
    </main>
  );
}

function hostName(origin: string): string {
  try {
    return new URL(origin).hostname.split(".")[0] || origin;
  } catch {
    return origin;
  }
}

export function HostChoice({
  initialError = "",
  onConnected,
  showUpdate = true,
  inUse = false,
}: {
  initialError?: string;
  showUpdate?: boolean;
  onConnected?: () => void;
  inUse?: boolean;
}) {
  const remote = hostMode() === "remote" ? attached() : null;
  const [mode, setMode] = useState<"local" | "remote">(remote ? "remote" : "local");
  const [origin, setOrigin] = useState(remote?.origin ?? "");
  const [token, setToken] = useState("");
  const [changing, setChanging] = useState(false);
  const address = useRef<HTMLInputElement>(null);
  const change = useRef<HTMLButtonElement>(null);
  const [docker, setDocker] = useState<DockerStatus | null>(null);
  const [existing, setExisting] = useState<ExistingHost[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(initialError);
  const [imageFailed, setImageFailed] = useState(false);
  const refresh = useCallback(async () => {
    try {
      setDocker(await localHost.status());
      setExisting(await localHost.existing().catch(() => []));
    } catch (cause) {
      setDocker({
        state: "unavailable",
        message: errorMessage(cause),
        updateAvailable: false,
      });
    }
  }, []);
  useEffect(() => {
    if (desktop && mode === "local") void refresh();
  }, [mode, refresh]);
  useEffect(() => {
    if (changing) address.current?.focus();
  }, [changing]);

  const connectExisting = (name: string) => adopt(() => localHost.connect(name), "existing");
  const adopt = async (reach: () => Promise<Remote>, chosen: HostMode) => {
    setBusy(true);
    setError("");
    try {
      const connection = await reach();
      await probe(connection);
      commit(connection, chosen, onConnected);
    } catch (cause) {
      setError(errorMessage(cause));
      void refresh();
    } finally {
      setBusy(false);
    }
  };

  if (!desktop)
    return (
      <p className="settings__lede">
        This browser is connected to this host. Use the Guaca desktop app to choose between On this
        Mac and Remote host.
      </p>
    );

  const connect = async (update = false) => {
    setBusy(true);
    setError("");
    setImageFailed(false);
    try {
      let connection: Remote;
      if (mode === "local")
        connection = update ? await localHost.update() : await localHost.start();
      else {
        const url = new URL(origin.trim());
        if (url.username || url.password || url.search || url.hash || url.pathname !== "/")
          throw new Error("Enter the host address, without a path or sign-in details.");
        if (
          url.protocol !== "https:" &&
          !(url.protocol === "http:" && ["127.0.0.1", "localhost", "[::1]"].includes(url.hostname))
        )
          throw new Error("Use a secure https:// address for a remote host.");
        connection = { origin: url.origin, token: token.trim() };
      }
      await probe(connection);
      preflightReload();
      try {
        saveConnection(connection, mode);
      } catch {
        throw new Error("Guaca could not save this choice. Nothing changed.");
      }
      if (onConnected) {
        adoptRemote(connection);
        onConnected();
      } else {
        restart(false);
      }
    } catch (cause) {
      setError(errorMessage(cause));
      setImageFailed(asStall(cause)?.kind === "image");
      if (mode === "local") void refresh();
    } finally {
      setBusy(false);
    }
  };
  const connected = inUse && !changing && mode === "remote" ? remote : null;
  const keep = () => {
    setChanging(false);
    setOrigin(remote?.origin ?? "");
    setToken("");
    setError("");
    requestAnimationFrame(() => change.current?.focus());
  };
  const usable = !!docker && ["ready", "running", "stopped"].includes(docker.state);
  const failed = showUpdate && docker?.operation?.error ? docker.operation : null;

  return (
    <div className="host-choice">
      <p className="settings__lede">
        Guaca is your desktop interface. Your host runs your agents and keeps their groups,
        conversations and files.
      </p>
      <fieldset className="host-choice__places" aria-label="Host location">
        <Place
          label="On this Mac"
          hint="Runs in Docker here. Agents keep working with Guaca closed, and pause while this Mac sleeps."
          chosen={mode === "local"}
          disabled={busy}
          onChoose={() => setMode("local")}
        />
        <Place
          label="Remote host"
          hint="Runs on a server that stays on. Agents keep working while this Mac sleeps or is offline."
          chosen={mode === "remote"}
          disabled={busy}
          onChoose={() => setMode("remote")}
        />
      </fieldset>
      {mode === "local" ? (
        <>
          {docker?.previous && (
            <section className="field" aria-label="Your earlier host">
              <span className="field__label">Your earlier host is in {docker.previous.label}</span>
              <p className="field__hint">
                Going back leaves the host you use now, and its data, where it is.
              </p>
              <div className="access__row">
                <button
                  className="btn btn--compact"
                  type="button"
                  disabled={busy}
                  onClick={() => {
                    const context = docker.previous?.context;
                    if (context) void adopt(() => localHost.choose(context), "local");
                  }}
                >
                  Go back to the host in {docker.previous.label}
                </button>
              </div>
            </section>
          )}
          {existing.length > 0 && (
            <section className="field" aria-label="Existing local hosts">
              <span className="field__label">Already running on this Mac</span>
              <p className="field__hint">
                Use an existing host to keep working with its groups. These hosts are managed
                outside this app.
              </p>
              {existing.map((host) => (
                <button
                  key={host.name}
                  className="btn btn--compact"
                  type="button"
                  disabled={busy}
                  onClick={() => void connectExisting(host.name)}
                >
                  Use {host.label} ({host.origin})
                </button>
              ))}
            </section>
          )}
          <section className="host-status" aria-label="Docker status">
            <p role="status">{docker?.message ?? "Checking Docker…"}</p>
            {failed && (
              <>
                <p className="host-status__failure">
                  {failed.stage}: {failed.error}
                </p>
                <a href={INSTRUCTIONS} target="_blank" rel="noopener noreferrer">
                  Update and recovery instructions
                </a>
              </>
            )}
            {docker && !usable && (
              <div className="access__row">
                {docker.state === "missing" ? (
                  <button
                    className="btn btn--compact"
                    type="button"
                    onClick={() =>
                      void openExternal("https://www.docker.com/products/docker-desktop/")
                    }
                  >
                    Get Docker Desktop
                  </button>
                ) : (
                  <button
                    className="btn btn--compact"
                    type="button"
                    disabled={busy}
                    onClick={() =>
                      void localHost
                        .openDocker(docker.engine?.context)
                        .then(refresh)
                        .catch((cause) => setError(errorMessage(cause)))
                    }
                  >
                    {docker.engine?.action ?? "Open Docker"}
                  </button>
                )}
                <button
                  className="btn btn--compact"
                  type="button"
                  disabled={busy}
                  onClick={() => void refresh()}
                >
                  Check again
                </button>
              </div>
            )}
            {showUpdate && docker?.updateAvailable && (
              <div className="host-status__update">
                <p>
                  A host update is ready. Work in progress pauses and carries on once the host
                  restarts. Guaca backs up its data first.
                </p>
                <button
                  type="button"
                  className="btn btn--compact"
                  disabled={busy}
                  onClick={() => void connect(true)}
                >
                  Back up and update host
                </button>
              </div>
            )}
          </section>
        </>
      ) : connected ? (
        <section className="host-status" aria-label="Remote host status">
          <p role="status">
            Connected to {connected.origin}. Your access key is saved on this Mac.
          </p>
          <div className="access__row">
            <button
              ref={change}
              className="btn btn--compact"
              type="button"
              onClick={() => setChanging(true)}
            >
              Change host
            </button>
          </div>
        </section>
      ) : (
        <>
          <p className="field__hint">Use the address and access key supplied by your host.</p>
          <label className="field">
            <span className="field__label">Host address</span>
            <input
              ref={address}
              className="input"
              placeholder="https://guaca.example.com"
              value={origin}
              disabled={busy}
              onChange={(event) => setOrigin(event.target.value)}
            />
          </label>
          <label className="field">
            <span className="field__label">Access key</span>
            <input
              className="input"
              type="password"
              autoComplete="off"
              value={token}
              disabled={busy}
              onChange={(event) => setToken(event.target.value)}
            />
          </label>
        </>
      )}
      {error && (
        <div className="banner banner--error" role="alert">
          {error}
        </div>
      )}
      {imageFailed && <ImageRecovery />}
      {error && error === initialError && (
        <p className="field__hint">
          <a href={INSTRUCTIONS} target="_blank" rel="noopener noreferrer">
            Host update instructions
          </a>
          {" · "}
          <a href={RELEASES} target="_blank" rel="noopener noreferrer">
            Guaca downloads
          </a>
        </p>
      )}
      {!connected && (
        <div className="access__row">
          <button
            className="btn btn--primary"
            type="button"
            disabled={
              busy ||
              (mode === "remote"
                ? !origin.trim() || !token.trim()
                : !docker || ["missing", "unavailable"].includes(docker.state))
            }
            onClick={() => void connect()}
          >
            {busy
              ? mode === "local"
                ? "Preparing your host…"
                : "Connecting…"
              : mode === "local"
                ? "Use this Mac"
                : "Connect to host"}
          </button>
          {changing && mode === "remote" && (
            <button className="btn btn--quiet" type="button" disabled={busy} onClick={keep}>
              Cancel
            </button>
          )}
        </div>
      )}
      {busy && mode === "local" && (
        <p className="field__hint" role="status">
          The first setup may take a few minutes while Guaca downloads its host. Your existing
          groups are left in place.
        </p>
      )}
      <p className="field__hint">
        You can switch hosts later. Each group stays on the host it was made on; export it and
        import it on the other host to move it.
      </p>
    </div>
  );
}

function Place({
  label,
  hint,
  chosen,
  disabled,
  onChoose,
}: {
  label: string;
  hint: string;
  chosen: boolean;
  disabled: boolean;
  onChoose: () => void;
}) {
  const hintId = useId();
  return (
    <button
      type="button"
      className="choice choice--place"
      aria-label={label}
      aria-describedby={hintId}
      aria-pressed={chosen}
      disabled={disabled}
      onClick={onChoose}
    >
      <span className="choice__name">{label}</span>
      <span className="choice__hint" id={hintId}>
        {hint}
      </span>
    </button>
  );
}
