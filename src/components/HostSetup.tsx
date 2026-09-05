import { type ReactNode, useCallback, useEffect, useState } from "react";
import {
  type DockerStatus,
  type ExistingHost,
  hostMode,
  localHost,
  rememberMode,
} from "../lib/host";
import {
  activateRemote,
  attached,
  desktop,
  openExternal,
  probe,
  type Remote,
  restart,
  setRemote,
} from "../lib/transport";
import { errorMessage } from "../lib/types";
import { LegacyGroups } from "./GroupTransfer";

/** Runs before App mounts, so no workspace calls or subscriptions can race setup. */
export function HostSetup({ children }: { children: ReactNode }) {
  const [ready, setReady] = useState(!desktop);
  const [checking, setChecking] = useState(desktop && !!attached());
  const [error, setError] = useState("");
  useEffect(() => {
    if (!desktop || !attached()) return;
    let disposed = false;
    const connect = async () => {
      try {
        const connection = hostMode() === "local" ? await localHost.start() : attached()!;
        await probe(connection);
        if (!disposed) {
          activateRemote(connection);
          setReady(true);
        }
      } catch (cause) {
        if (!disposed) setError(errorMessage(cause));
      } finally {
        if (!disposed) setChecking(false);
      }
    };
    void connect();
    return () => {
      disposed = true;
    };
  }, []);
  if (ready) return children;
  return (
    <main className="threshold">
      <section className="host-setup">
        <h1 className="empty__title">Where should your agents work?</h1>
        {checking ? (
          <p role="status">Connecting to your host…</p>
        ) : (
          <HostChoice initialError={error} onConnected={() => setReady(true)} />
        )}
        <LegacyGroups />
      </section>
    </main>
  );
}

/** Shared by onboarding and Settings. Switching never transfers or stops groups. */
export function HostChoice({
  initialError = "",
  onConnected,
}: {
  initialError?: string;
  onConnected?: () => void;
}) {
  const [mode, setMode] = useState<"local" | "remote">(attached() ? hostMode() : "local");
  const [origin, setOrigin] = useState(attached()?.origin ?? "");
  const [token, setToken] = useState("");
  const [docker, setDocker] = useState<DockerStatus | null>(null);
  const [existing, setExisting] = useState<ExistingHost[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(initialError);
  const refresh = useCallback(async () => {
    try {
      setDocker(await localHost.status());
      setExisting(await localHost.existing().catch(() => []));
    } catch (cause) {
      setDocker({ state: "unavailable", message: errorMessage(cause), updateAvailable: false });
    }
  }, []);
  useEffect(() => {
    if (desktop && mode === "local") void refresh();
  }, [mode, refresh]);

  const connectExisting = async (name: string) => {
    setBusy(true);
    setError("");
    try {
      const connection = await localHost.connect(name);
      await probe(connection);
      rememberMode("remote");
      if (onConnected) {
        activateRemote(connection);
        onConnected();
      } else {
        setRemote(connection);
        restart();
      }
    } catch (cause) {
      setError(errorMessage(cause));
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
      rememberMode(mode);
      if (onConnected) {
        activateRemote(connection);
        onConnected();
      } else {
        setRemote(connection);
        restart();
      }
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="host-choice">
      <p className="settings__lede">
        Guaca is your desktop interface. Your host runs your agents and keeps their groups,
        conversations and files.
      </p>
      <fieldset className="access__row" aria-label="Host location">
        <button
          type="button"
          className={`btn ${mode === "local" ? "btn--primary" : ""}`}
          aria-pressed={mode === "local"}
          disabled={busy}
          onClick={() => setMode("local")}
        >
          On this Mac
        </button>
        <button
          type="button"
          className={`btn ${mode === "remote" ? "btn--primary" : ""}`}
          aria-pressed={mode === "remote"}
          disabled={busy}
          onClick={() => setMode("remote")}
        >
          Remote host
        </button>
      </fieldset>
      {mode === "local" ? (
        <>
          <p className="field__hint">
            Runs privately in Docker on this Mac. Agents keep working when you close Guaca, but
            pause when your Mac sleeps.
          </p>
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
                  className="btn btn--small"
                  type="button"
                  disabled={busy}
                  onClick={() => void connectExisting(host.name)}
                >
                  Use {host.label} ({host.origin})
                </button>
              ))}
            </section>
          )}
          <section className="preset preset--plain" aria-label="Docker status">
            <div className="preset__text">
              <strong>Docker</strong>
              <p role="status">{docker?.message ?? "Checking Docker…"}</p>
              {docker?.updateAvailable && (
                <div className="field">
                  <p className="field__hint">
                    A host update is ready. Updating interrupts current jobs and restarts the host.
                    Guaca keeps a local data backup first.
                  </p>
                  <button
                    type="button"
                    className="btn btn--small"
                    disabled={busy}
                    onClick={() => void connect(true)}
                  >
                    Back up and update host
                  </button>
                </div>
              )}
              <div className="access__row">
                {docker?.state === "missing" ? (
                  <button
                    className="btn btn--small"
                    type="button"
                    onClick={() =>
                      void openExternal("https://www.docker.com/products/docker-desktop/")
                    }
                  >
                    Get Docker Desktop
                  </button>
                ) : (
                  <button
                    className="btn btn--small"
                    type="button"
                    disabled={busy}
                    onClick={() =>
                      void localHost
                        .openDocker()
                        .then(refresh)
                        .catch((cause) => setError(errorMessage(cause)))
                    }
                  >
                    Open Docker
                  </button>
                )}
                <button
                  className="btn btn--small"
                  type="button"
                  disabled={busy}
                  onClick={() => void refresh()}
                >
                  Check again
                </button>
              </div>
            </div>
          </section>
        </>
      ) : (
        <>
          <p className="field__hint">
            An always-on host keeps agents working while this Mac is asleep or offline. Use the
            address and access key supplied by your host.
          </p>
          <label className="field">
            <span className="field__label">Host address</span>
            <input
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
      <p className="field__hint">
        You can change hosts here at any time. Groups stay on their original host. Export a group
        and import it on another host to move your work. Sign-ins are connected separately on each
        host.
      </p>
      {error && (
        <div className="banner banner--error" role="alert">
          {error}
        </div>
      )}
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
      {busy && mode === "local" && (
        <p className="field__hint" role="status">
          The first setup may take a few minutes while Guaca downloads its host. Your existing
          groups are left in place.
        </p>
      )}
    </div>
  );
}
