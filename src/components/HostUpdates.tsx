import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import { COMMIT, VERSION } from "../lib/build";
import { type DockerStatus, hostMode, localHost } from "../lib/host";
import {
  available,
  compatibility,
  type Health,
  INSTRUCTIONS,
  parseHealth,
  parseReleaseStatus,
  RELEASES,
  type ReleaseStatus,
  sameBuild,
} from "../lib/releases";
import { useStore } from "../lib/store";
import {
  desktop,
  RECONNECTED,
  restart,
  token,
  UNAUTHORIZED_EVENT,
  workspaceOrigin,
} from "../lib/transport";
import { errorMessage } from "../lib/types";
import { HostChoice } from "./HostSetup";

interface UpdateState {
  health: Health | null;
  release: ReleaseStatus | null;
  docker: DockerStatus | null;
  error: string;
  checking: boolean;
  refresh: (manual?: boolean) => Promise<void>;
}
const Context = createContext<UpdateState | null>(null);
const isManaged = () => desktop && hostMode() === "local";

export function HostMonitor({ children }: { children: ReactNode }) {
  const [health, setHealth] = useState<Health | null>(null);
  const [release, setRelease] = useState<ReleaseStatus | null>(null);
  const [docker, setDocker] = useState<DockerStatus | null>(null);
  const [error, setError] = useState("");
  const [checking, setChecking] = useState(true);
  const [admitted, setAdmitted] = useState(false);
  const [legacy, setLegacy] = useState(false);
  const [showHostChoice, setShowHostChoice] = useState(false);
  const mounted = useRef(false);
  const pending = useRef<Promise<void> | null>(null);
  const refresh = useCallback((manual = false): Promise<void> => {
    if (pending.current)
      return manual ? pending.current.then(() => refresh(true)) : pending.current;
    const run = async () => {
      setChecking(true);
      // Docker remains reachable while the backend is stopped for replacement.
      if (isManaged())
        void localHost
          .status()
          .then((value) => {
            if (mounted.current) setDocker(value);
          })
          .catch(() => {
            if (mounted.current) setDocker(null);
          });
      try {
        const response = await fetch(`${workspaceOrigin()}/health`, {
          cache: "no-store",
          signal: AbortSignal.timeout(15000),
        });
        if (!response.ok) throw new Error("The host did not answer its health check. Try again.");
        const found = parseHealth(await response.json());
        if (!mounted.current) return;
        setHealth(found);
        if (compatibility(found) === "compatible") setAdmitted(true);
        setError("");
        const results = await Promise.allSettled([
          fetch(`${workspaceOrigin()}/v1/updates?refresh=${manual}`, {
            cache: "no-store",
            headers: { authorization: `Bearer ${token()}` },
            signal: AbortSignal.timeout(15000),
          }).then(async (response) => {
            if (response.status === 401) window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
            if (response.status === 404)
              throw new Error(
                "This host does not support release checks yet. Update it using the host instructions.",
              );
            if (!response.ok) throw new Error("Could not read host update information. Try again.");
            return parseReleaseStatus(await response.json());
          }),
        ]);
        if (!mounted.current) return;
        const [updates] = results;
        if (updates.status === "fulfilled") setRelease(updates.value);
        else
          setRelease((previous) => ({
            automatic: previous?.automatic ?? true,
            checkedAt: previous?.checkedAt ?? null,
            latest: previous?.latest ?? null,
            error: errorMessage(updates.reason),
          }));
      } catch (cause) {
        if (mounted.current) setError(errorMessage(cause));
      } finally {
        pending.current = null;
        if (mounted.current) setChecking(false);
      }
    };
    pending.current = run();
    return pending.current;
  }, []);
  useEffect(() => {
    mounted.current = true;
    void refresh();
    const check = () => {
      if (document.visibilityState !== "hidden") void refresh();
    };
    window.addEventListener(RECONNECTED, check);
    window.addEventListener("focus", check);
    const timer = window.setInterval(check, 6 * 60 * 60 * 1000);
    return () => {
      mounted.current = false;
      window.removeEventListener(RECONNECTED, check);
      window.removeEventListener("focus", check);
      clearInterval(timer);
    };
  }, [refresh]);
  const blocked = health && ["hostOld", "clientOld"].includes(compatibility(health));
  return (
    <Context.Provider value={{ health, release, docker, error, checking, refresh }}>
      {admitted || legacy ? (
        <>
          <div className="host-workspace" hidden={!!blocked}>
            {children}
          </div>
          {blocked && (
            <main className="threshold">
              <section className="host-setup">
                <HostUpdatePanel />
              </section>
            </main>
          )}
        </>
      ) : (
        <main className="threshold">
          <section className="host-setup">
            <HostUpdatePanel />
            {health && compatibility(health) === "unknown" && (
              <button className="btn" type="button" onClick={() => setLegacy(true)}>
                Continue with unverified host
              </button>
            )}
            {desktop && (
              <button
                className="btn btn--ghost"
                type="button"
                onClick={() => setShowHostChoice(!showHostChoice)}
              >
                Choose another host
              </button>
            )}
            {showHostChoice && <HostChoice />}
          </section>
        </main>
      )}
    </Context.Provider>
  );
}

export function HostUpdateNotice({ onReview }: { onReview: () => void }) {
  const state = useContext(Context);
  const [dismissed, setDismissed] = useState("");
  if (!state?.health) return null;
  const newer = available(state.health, state.release);
  const pageChanged =
    !desktop &&
    !!state.health.build &&
    !sameBuild(COMMIT, state.health.build) &&
    !COMMIT.endsWith("-dirty");
  const local = isManaged() && state.docker?.updateAvailable;
  const key = `${workspaceOrigin()}:${pageChanged ? state.health.build : newer ? state.release?.latest?.version : state.docker?.targetImage}`;
  let saved = dismissed;
  try {
    saved = sessionStorage.getItem("guaca.update.dismissed") ?? dismissed;
  } catch {
    /* Session-only fallback. */
  }
  if ((!newer && !pageChanged && !local) || saved === key) return null;
  return (
    <div className="banner" role="status">
      <span>
        {pageChanged
          ? "This page has an older Guaca build. Reload when you are ready."
          : "Host update available."}
      </span>
      <button className="btn" type="button" onClick={onReview}>
        Review update
      </button>
      <button
        className="btn btn--ghost"
        type="button"
        onClick={() => {
          setDismissed(key);
          try {
            sessionStorage.setItem("guaca.update.dismissed", key);
          } catch {
            /* See above. */
          }
        }}
      >
        Later
      </button>
    </div>
  );
}

export function HostUpdatePanel() {
  const state = useContext(Context);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState("");
  const [failure, setFailure] = useState("");
  const [instructions, setInstructions] = useState(false);
  const activity = useStore((s) => s.activity);
  const building = useStore((s) => s.building);
  const checkProgress = state?.refresh;
  const updating = busy || !!state?.docker?.updating;
  useEffect(() => {
    if (!updating || !checkProgress) return;
    const timer = setInterval(() => void checkProgress(), 1500);
    return () => clearInterval(timer);
  }, [updating, checkProgress]);
  if (!state) return null;
  const { health, release, docker, checking, refresh, error } = state;
  const match = health ? compatibility(health) : "unknown";
  const newer = available(health, release);
  const managed = isManaged() && docker?.origin === workspaceOrigin();
  const canUpdate = managed && docker?.updateAvailable;
  const pageChanged =
    !desktop && !!health?.build && !sameBuild(COMMIT, health.build) && !COMMIT.endsWith("-dirty");
  const title =
    match === "hostOld"
      ? "This host needs an update"
      : match === "clientOld"
        ? "This Guaca needs an update"
        : "Host updates";
  const run = async () => {
    setBusy(true);
    setFailure("");
    setResult("");
    try {
      await localHost.update(workspaceOrigin());
      await refresh(true);
      setResult("Host updated. Review any interrupted work before trying it again.");
    } catch (cause) {
      setFailure(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };
  const working = Object.values(activity).filter(
    (a) => a.state === "thinking" || a.state === "awaitingApproval",
  ).length;
  return (
    <section className="host-choice" aria-label="Host updates" aria-busy={updating}>
      <h3>{title}</h3>
      <p className="field__hint">{workspaceOrigin()}</p>
      <p role="status">
        {!health
          ? checking
            ? "Checking host…"
            : "Could not reach the host."
          : match === "hostOld"
            ? "Update the host before opening this workspace."
            : match === "clientOld"
              ? "This frontend cannot use the host API. Update Guaca to reconnect."
              : newer
                ? "A newer host release is available."
                : release?.error
                  ? "Could not check for updates."
                  : !health.release
                    ? "This is an unverified or development build."
                    : !release?.latest
                      ? "Release status has not been checked."
                      : "No newer stable host release was found."}
      </p>
      {health && (
        <dl className="host-update-facts">
          <dt>Installed</dt>
          <dd>
            {health.version ?? "Unknown release"}
            {!health.release && " (unverified)"}
          </dd>
          <dt>Available</dt>
          <dd>{release?.latest?.version ?? "Not verified"}</dd>
          <dt>Compatibility</dt>
          <dd>
            {match === "compatible"
              ? "Compatible"
              : match === "unknown"
                ? "Unverified"
                : "Update required"}
          </dd>
          <dt>Last checked</dt>
          <dd>
            {release?.checkedAt ? new Date(release.checkedAt).toLocaleString() : "Not checked"}
            {release?.error && " (latest check failed)"}
          </dd>
        </dl>
      )}
      {health && (
        <details>
          <summary>Build details</summary>
          <p>
            Guaca frontend: {VERSION} ({COMMIT || "unknown commit"})
          </p>
          <p>Host commit: {health.build || "unknown"}</p>
        </details>
      )}
      {!release?.automatic && release && (
        <p className="field__hint">Automatic release checks are disabled on this host.</p>
      )}
      {(error || release?.error || failure) && (
        <p className="field__error" role="alert">
          {failure || error || release?.error}
        </p>
      )}
      {release?.latest && (
        <a href={release.latest.notes} target="_blank" rel="noopener noreferrer">
          Release notes
        </a>
      )}
      {canUpdate && match !== "clientOld" && (
        <div className="field">
          <p>
            Update this host to the build included with this desktop app
            {docker?.targetVersion ? ` (${docker.targetVersion})` : ""}.
          </p>
          <p className="field__hint">
            {working} agents and {Object.keys(building).length} coding jobs are working. Updating
            interrupts current work, including work from other clients. Guaca saves a complete
            backup before replacing the host.
          </p>
          <button
            className="btn btn--primary"
            disabled={updating}
            type="button"
            onClick={() => void run()}
          >
            Back up and update host
          </button>
        </div>
      )}
      {updating && (
        <p role="status">
          {docker?.operation?.stage ?? "Starting update"}. Keep Guaca open until the update
          finishes.
        </p>
      )}
      {docker?.operation && !updating && (
        <details>
          <summary>Last host update</summary>
          <p>{docker.operation.stage}</p>
          {docker.operation.backup && (
            <p>
              Recovery backup: <code>{docker.operation.backup}</code>
            </p>
          )}
          {docker.operation.error && <p>{docker.operation.error}</p>}
        </details>
      )}
      {((newer && managed && !canUpdate) || match === "clientOld") && desktop && (
        <p>
          <a href={RELEASES} target="_blank" rel="noopener noreferrer">
            Download the latest Guaca desktop app
          </a>{" "}
          before updating its local host.
        </p>
      )}
      {pageChanged && (
        <button
          className="btn"
          type="button"
          onClick={() => {
            try {
              restart();
            } catch (e) {
              setFailure(errorMessage(e));
            }
          }}
        >
          Save draft and reload Guaca
        </button>
      )}
      <div className="access__row">
        <button
          className="btn btn--small"
          disabled={checking || updating}
          type="button"
          onClick={() => void refresh(true)}
        >
          {checking ? "Checking…" : "Check for updates"}
        </button>
        <button
          className="btn btn--small"
          type="button"
          onClick={() => setInstructions(!instructions)}
        >
          View update instructions
        </button>
      </div>
      {instructions && (
        <div className="field">
          <p>
            Update on the machine running this host. Stop it and back up the complete workspace,
            then recreate the service with the target release and the same data volume and
            configuration.
          </p>
          <p className="field__hint">
            Compose source builds must be rebuilt from the desired revision. Restarting the existing
            container does not install a new image. After a failed migration, restore a backup
            before running an older release.
          </p>
          <a href={INSTRUCTIONS} target="_blank" rel="noopener noreferrer">
            Deployment and recovery instructions
          </a>
          {!managed && (
            <p className="field__hint">
              This connection does not manage the server. After updating it, check again here.
            </p>
          )}
        </div>
      )}
      {result && <p role="status">{result}</p>}
    </section>
  );
}
