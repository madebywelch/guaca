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
import {
  type AppSource,
  type DockerStatus,
  hostMode,
  localHost,
  type Manager,
  parseManager,
  thisApp,
} from "../lib/host";
import {
  available,
  clientUpdate,
  compareVersions,
  compatibility,
  type Health,
  INSTRUCTIONS,
  pageStale,
  parseHealth,
  parseReleaseStatus,
  protocol,
  RELEASES,
  type ReleaseStatus,
  sameBuild,
  shortBuild,
  skew,
  updateNotice,
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
  /** The updater on a box, which this host relays. `null` is a host with none. */
  manager: Manager | null;
  /** When the read that produced `manager` began, so an answer from before a
   *  click is never taken for one after it. */
  managerAt: number;
  adoptManager: (manager: Manager) => void;
  error: string;
  checking: boolean;
  refresh: (manual?: boolean) => Promise<void>;
}
const Context = createContext<UpdateState | null>(null);

/** What the host monitor last learned, for a surface that summarizes it. */
export function useHostState(): UpdateState | null {
  return useContext(Context);
}
const isManaged = () => desktop && hostMode() === "local";

export function HostMonitor({ children }: { children: ReactNode }) {
  const [health, setHealth] = useState<Health | null>(null);
  const [release, setRelease] = useState<ReleaseStatus | null>(null);
  const [docker, setDocker] = useState<DockerStatus | null>(null);
  const [manager, setManager] = useState<{ value: Manager | null; at: number }>({
    value: null,
    at: 0,
  });
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
        const asked = Date.now();
        const results = await Promise.allSettled([
          fetch(`${workspaceOrigin()}/v1/updates?refresh=${manual}`, {
            cache: "no-store",
            headers: { authorization: `Bearer ${token()}` },
            signal: AbortSignal.timeout(15000),
          }).then(async (response) => {
            if (response.status === 401) window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
            // A host older than this route that serves the page answers any
            // unknown path with index.html, which is a 200 and not the answer.
            if (
              response.status === 404 ||
              (response.ok && !response.headers.get("content-type")?.includes("application/json"))
            )
              throw new Error(
                "This host does not support release checks yet. Update it using the host instructions.",
              );
            if (!response.ok) throw new Error("Could not read host update information. Try again.");
            return parseReleaseStatus(await response.json());
          }),
          fetch(`${workspaceOrigin()}/v1/host`, {
            cache: "no-store",
            headers: { authorization: `Bearer ${token()}` },
            signal: AbortSignal.timeout(15000),
          }).then(async (response) => {
            // Older than the route: a 404, or the page served in its place.
            if (
              response.status === 404 ||
              (response.ok && !response.headers.get("content-type")?.includes("application/json"))
            )
              return null;
            if (!response.ok) throw new Error("The host's updater did not answer.");
            return parseManager(await response.json());
          }),
        ]);
        if (!mounted.current) return;
        const [updates, updater] = results;
        // An updater that does not answer is kept as it was last seen: it is
        // replacing itself, or its host is restarting, and the click that
        // started that is waiting on the next answer rather than this one.
        if (updater.status === "fulfilled") {
          const value = updater.value;
          setManager((seen) => (asked >= seen.at ? { value, at: asked } : seen));
        }
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
    <Context.Provider
      value={{
        health,
        release,
        docker,
        manager: manager.value,
        managerAt: manager.at,
        adoptManager: (value) => setManager({ value, at: Date.now() }),
        error,
        checking,
        refresh,
      }}
    >
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
  const notice = updateNotice({
    desktop,
    client: { version: VERSION, commit: COMMIT },
    health: state.health,
    release: state.release,
    localUpdate:
      isManaged() && state.docker?.updateAvailable ? (state.docker.targetImage ?? "") : null,
  });
  const key = `${workspaceOrigin()}:${notice?.key}`;
  let saved = dismissed;
  try {
    saved = sessionStorage.getItem("guaca.update.dismissed") ?? dismissed;
  } catch {
    /* Session-only fallback. */
  }
  if (!notice || saved === key) return null;
  return (
    <div className="banner" role="status">
      <span>{notice.text}</span>
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

/** An update a box accepted: what it was asked for, and what to call it. */
interface Accepted {
  at: number;
  version: string;
  /** The image, on main, where every build carries the same version. */
  image?: string;
  name: string;
}

/** What the operator is told when an update on a box has finished. */
function outcome(manager: Manager, asked: Accepted): { result: string } | { failure: string } {
  // The journal holds the last update that ran. One that never began leaves
  // it holding an earlier one, whose outcome is not this one's.
  const ran = manager.operation;
  const op = (asked.image ? ran?.targetImage === asked.image : ran?.targetVersion === asked.version)
    ? ran
    : null;
  if (!op)
    return {
      failure:
        manager.error ?? "The host did not start the update. Check for updates and try again.",
    };
  if (op.stage === "Host updated")
    return {
      result: `Host updated to ${asked.name}. Review any interrupted work before trying it again.`,
    };
  if (op.stage === "Previous version restored")
    return {
      failure: `${op.error ?? "The update did not finish."} Guaca put the previous version back, with the workspace as it was when the update began.`,
    };
  return { failure: op.error ?? manager.error ?? "The update did not finish. Try again." };
}

/**
 * This app, rebuilt from the checkout it came from. A source build has no
 * release to download: its latest is the tip of the branch it follows.
 *
 * On a box that follows main, the host goes first. An app newer than its host
 * calls commands the host does not have, which is the failure the build
 * comparison above exists to name; a host newer than its app is only unused.
 */
function AppRebuild({ hostFirst }: { hostFirst: boolean }) {
  const [source, setSource] = useState<AppSource | null>(null);
  const [failure, setFailure] = useState("");
  const read = useCallback(
    () =>
      thisApp
        .source()
        .then(setSource)
        .catch(() => setSource(null)),
    [],
  );
  useEffect(() => {
    void read();
  }, [read]);
  const running = !!source?.running;
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(() => void read(), 2000);
    return () => clearInterval(timer);
  }, [running, read]);
  if (!source?.checkout || !source.upstream) return null;
  // Edits on top of the branch's own commit are the operator's, not news.
  const current = sameBuild(COMMIT.replace(/-dirty$/, ""), source.upstream);
  const failed = failure || source.failure;
  if (current && !running && !failed) return null;
  const branch = source.branch ?? "main";
  const start = async () => {
    setFailure("");
    try {
      await thisApp.rebuild();
    } catch (cause) {
      setFailure(errorMessage(cause));
    }
    await read();
  };
  return (
    <div className="field">
      <p role="status">
        {running
          ? `Rebuilding this app from ${branch}. Guaca closes and reopens when the build finishes.`
          : `This app is at ${shortBuild(COMMIT) || "an unknown build"}, and ${branch} is at ${shortBuild(source.upstream)}.`}
      </p>
      {!running &&
        !current &&
        (hostFirst ? (
          <p className="field__hint">
            Update the host first. This app rebuilds from {branch} after it.
          </p>
        ) : (
          <button className="btn" type="button" onClick={() => void start()}>
            Rebuild this app
          </button>
        ))}
      {failed && (
        <>
          <p className="field__error" role="alert">
            {failure || "The rebuild did not finish."}
          </p>
          {source.failure && <pre className="md__pre">{source.failure}</pre>}
          <p className="field__hint">
            The whole log is at <code>{source.log}</code>.
          </p>
        </>
      )}
    </div>
  );
}

export function HostUpdatePanel() {
  const state = useContext(Context);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState("");
  const [failure, setFailure] = useState("");
  const [instructions, setInstructions] = useState(false);
  /** An update a box accepted from this panel, until it reports it finished. */
  const [accepted, setAccepted] = useState<Accepted | null>(null);
  const activity = useStore((s) => s.activity);
  const building = useStore((s) => s.building);
  const checkProgress = state?.refresh;
  const boxBusy = !!accepted || !!state?.manager?.updating;
  const updating = busy || !!state?.docker?.updating || boxBusy;
  useEffect(() => {
    if (!updating || !checkProgress) return;
    const timer = setInterval(() => void checkProgress(), 1500);
    return () => clearInterval(timer);
  }, [updating, checkProgress]);
  const manager = state?.manager ?? null;
  const managerAt = state?.managerAt ?? 0;
  const answered = !!state?.health && !state?.error;
  useEffect(() => {
    // Finished only by an answer read after the box accepted, from a host
    // that is answering again: the one before the click also says "not updating".
    if (!accepted || !manager || manager.updating || managerAt <= accepted.at || !answered) return;
    const said = outcome(manager, accepted);
    if ("result" in said) setResult(said.result);
    else setFailure(said.failure);
    setAccepted(null);
  }, [accepted, manager, managerAt, answered]);
  if (!state) return null;
  const { health, release, docker, checking, refresh, error } = state;
  const match = health ? compatibility(health) : "unknown";
  const newer = available(health, release);
  const managed = isManaged() && docker?.origin === workspaceOrigin();
  const canUpdate = managed && docker?.updateAvailable;
  // A box with an updater. A window only offers a release it can still talk
  // to afterward; a browser is served the new release's own page.
  const latest = release?.latest ?? null;
  const reachable =
    !desktop ||
    (!!latest &&
      latest.apiGeneration >= protocol.minimum &&
      latest.apiGeneration <= protocol.maximum);
  const boxed = !managed && !!manager;
  const onMain = latest?.channel === "main";
  const target =
    latest && (onMain ? `main at ${shortBuild(latest.commit)}` : `Guaca ${latest.version}`);
  // A box's updater installs only a signed release, so a source build on one
  // is offered the release that makes it verified, not only a newer number.
  // On main every build is the same version, and the commit is what moves.
  const order = latest && health?.version ? compareVersions(health.version, latest.version) : null;
  const boxNewer = onMain ? newer : order === -1 || (order === 0 && !health?.release);
  const canUpdateBox = boxed && boxNewer && reachable && match !== "clientOld";
  const pageChanged = pageStale(desktop, COMMIT, health);
  const drift = desktop ? skew({ version: VERSION, commit: COMMIT }, health) : "unknown";
  // The API generation says this app can talk to the host at all. Commands
  // added since the older build are missing whatever the generation says.
  const apart = drift === "otherBuild" || drift === "hostBehind" || drift === "clientBehind";
  const appBehind = desktop && (drift === "clientBehind" || clientUpdate(VERSION, release));
  const hostBuild = shortBuild(health?.build ?? "");
  const appBuild = shortBuild(COMMIT);
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
      void refresh(true);
    } finally {
      setBusy(false);
    }
  };
  const runOnBox = async () => {
    if (!latest) return;
    setBusy(true);
    setFailure("");
    setResult("");
    try {
      const response = await fetch(`${workspaceOrigin()}/v1/host/update`, {
        method: "POST",
        cache: "no-store",
        headers: { authorization: `Bearer ${token()}`, "content-type": "application/json" },
        body: JSON.stringify(
          onMain ? { version: latest.version, commit: latest.commit } : { version: latest.version },
        ),
        signal: AbortSignal.timeout(45000),
      });
      const body: unknown = await response.json().catch(() => null);
      if (!response.ok)
        throw new Error(
          (body as { err?: { message?: string } } | null)?.err?.message ??
            "The host did not accept the update. Try again.",
        );
      const report = parseManager(body);
      if (report) state.adoptManager(report);
      setAccepted({
        at: Date.now(),
        version: latest.version,
        image: onMain ? latest.image : undefined,
        name: target ?? `Guaca ${latest.version}`,
      });
    } catch (cause) {
      setFailure(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };
  const working = Object.values(activity).filter(
    (a) => a.state === "thinking" || a.state === "awaitingApproval",
  ).length;
  const operation = docker?.operation ?? manager?.operation ?? null;
  // While a box restarts its host, not answering is the update working.
  const restarting = boxBusy && !answered;
  return (
    <section className="host-choice" aria-label="Host updates" aria-busy={updating}>
      <h3>{title}</h3>
      <p className="field__hint">{workspaceOrigin()}</p>
      <p role="status">
        {restarting
          ? "The host is restarting on its new version."
          : !health
            ? checking
              ? "Checking host…"
              : "Could not reach the host."
            : match === "hostOld"
              ? "Update the host before opening this workspace."
              : match === "clientOld"
                ? "This frontend cannot use the host API. Update Guaca to reconnect."
                : drift === "hostBehind"
                  ? "This host runs an older Guaca than this app."
                  : drift === "clientBehind"
                    ? "This app runs an older Guaca than its host."
                    : drift === "otherBuild"
                      ? "This app and its host are different builds of the same version."
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
          <dt>Host</dt>
          <dd data-drift={apart ? "" : undefined}>
            {health.version ?? "Unknown release"}
            {hostBuild && ` · ${hostBuild}`}
            {release?.channel === "main" ? " (main)" : !health.release && " (unverified)"}
          </dd>
          {desktop && (
            <>
              <dt>This app</dt>
              <dd data-drift={apart ? "" : undefined}>
                {VERSION}
                {appBuild && ` · ${appBuild}`}
              </dd>
            </>
          )}
          <dt>Available</dt>
          <dd>
            {latest?.channel === "main"
              ? `main at ${shortBuild(latest.commit)}`
              : (latest?.version ?? "Not verified")}
          </dd>
          <dt>Compatibility</dt>
          <dd>
            {match === "compatible"
              ? apart
                ? "Connects; features may differ"
                : "Compatible"
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
      {!release?.automatic && release && (
        <p className="field__hint">Automatic release checks are disabled on this host.</p>
      )}
      {(failure || (!restarting && (error || release?.error || manager?.error))) && (
        <p className="field__error" role="alert">
          {failure || error || release?.error || manager?.error}
        </p>
      )}
      {release?.latest && (
        <a href={release.latest.notes} target="_blank" rel="noopener noreferrer">
          {onMain ? "The commit" : "Release notes"}
        </a>
      )}
      {((canUpdate && match !== "clientOld") || canUpdateBox) && (
        <div className="field">
          <p>
            {canUpdateBox && latest
              ? `Update this host to ${target}.`
              : `Update this host to the build included with this desktop app${
                  docker?.targetVersion ? ` (${docker.targetVersion})` : ""
                }.`}
          </p>
          <p className="field__hint">
            {working} agents and {Object.keys(building).length} coding jobs are working. Updating
            interrupts current work, including work from other clients. Guaca saves a complete
            backup before replacing the host, and puts the previous version back if the update does
            not finish.
          </p>
          <button
            className="btn btn--primary"
            disabled={updating}
            type="button"
            onClick={() => void (canUpdateBox ? runOnBox() : run())}
          >
            Back up and update host
          </button>
        </div>
      )}
      {boxed && boxNewer && !reachable && (
        <p>
          {target} needs a newer desktop app.{" "}
          <a href={RELEASES} target="_blank" rel="noopener noreferrer">
            Download it
          </a>
          , then update the host from it.
        </p>
      )}
      {updating && (
        <p role="status">
          {boxBusy
            ? `${restarting ? "Starting updated host" : (manager?.operation?.stage ?? "Starting update")}. The update runs on the host, so closing Guaca does not stop it.`
            : `${docker?.operation?.stage ?? "Starting update"}. Keep Guaca open until the update finishes.`}
        </p>
      )}
      {operation && !updating && (
        <details>
          <summary>Last host update</summary>
          <p>{operation.stage}</p>
          {operation.backup && (
            <p>
              Recovery backup: <code>{operation.backup}</code>
            </p>
          )}
          {operation.preserved && (
            <p>
              Workspace the failed version left: <code>{operation.preserved}</code>
            </p>
          )}
          {operation.error && <p>{operation.error}</p>}
        </details>
      )}
      {desktop && <AppRebuild hostFirst={boxed && onMain && newer} />}
      {desktop && (appBehind || match === "clientOld" || (newer && managed && !canUpdate)) && (
        <p>
          <a href={RELEASES} target="_blank" rel="noopener noreferrer">
            Download the latest Guaca desktop app
          </a>
          {managed ? " before updating its local host." : "."}
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
        {!boxed && (
          <button
            className="btn btn--small"
            type="button"
            onClick={() => setInstructions(!instructions)}
          >
            View update instructions
          </button>
        )}
      </div>
      {instructions && !boxed && (
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
