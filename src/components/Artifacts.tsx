import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import {
  detailsLine,
  logRows,
  ownerLabel,
  pageData,
  sentMessage,
  sourceLine,
} from "../lib/artifacts";
import { useDismiss } from "../lib/dismiss";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { useNow, whenLabel } from "../lib/time";
import {
  type AgentId,
  type Artifact,
  type ArtifactDetail,
  type ArtifactId,
  type ArtifactMade,
  type ArtifactRead,
  errorMessage,
  type WidgetWidth,
} from "../lib/types";
import { HtmlArtifact } from "./HtmlArtifact";
import { Icon } from "./Icon";
import { Scrim } from "./Scrim";

interface Props {
  onClose: () => void;
}

let sendingPage = false;

export function Artifacts({ onClose }: Props) {
  const open = useStore((state) => state.artifactsOpen);
  const show = useStore((state) => state.showArtifacts);
  const railGroup = useStore((state) => state.railGroup);
  const groups = useStore((state) => state.groups);
  const changed = useStore((state) => state.artifactsVersion);
  const now = useNow(60_000);

  const [list, setList] = useState<Artifact[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const rows = useRef(new Map<ArtifactId, HTMLButtonElement>());
  const id = open?.id ?? null;
  const previousId = useRef(id);
  useDismiss(() => {
    if (id) show({ id: null });
    else onClose();
  });

  useEffect(() => {
    panelRef.current?.focus();
  }, []);

  useLayoutEffect(() => {
    if (previousId.current !== id) {
      const target =
        id === null && previousId.current ? rows.current.get(previousId.current) : null;
      (target ?? panelRef.current)?.focus();
      previousId.current = id;
    }
  }, [id]);

  useEffect(() => {
    let live = true;
    api
      .artifacts(railGroup)
      .then((found) => {
        if (!live) return;
        setList(found);
        setError(null);
      })
      .catch((caught) => {
        if (!live) return;
        setList([]);
        setError(errorMessage(caught));
      });
    return () => {
      live = false;
    };
  }, [railGroup, changed]);

  const crew = groups.find((group) => group.id === railGroup) ?? null;
  const crewName = (groupId: string) => groups.find((group) => group.id === groupId)?.name ?? null;

  return (
    <Scrim>
      <button type="button" className="scrim__close" aria-label="Close dialog" onClick={onClose} />
      <div
        className="dialog dialog--artifacts"
        data-open={id ? "" : undefined}
        role="dialog"
        aria-modal="true"
        aria-label="Artifacts"
        tabIndex={-1}
        ref={panelRef}
      >
        {id ? (
          <ArtifactView id={id} onBack={() => show({ id: null })} onClose={onClose} />
        ) : (
          <>
            <div className="artifacts__head">
              <h2 className="dialog__title">Artifacts</h2>
              <p className="artifacts__details">{crew ? crew.name : "All groups"}</p>
            </div>

            <div className="artifacts__body">
              {list === null ? (
                <p className="routines__note">Loading…</p>
              ) : list.length === 0 ? (
                <p className="routines__note">
                  {crew
                    ? `${crew.name} keeps no artifacts yet.`
                    : "No group keeps any artifacts yet."}{" "}
                  Ask an agent for a page you will want to come back to, a board or a plan, and it
                  keeps one here.
                </p>
              ) : (
                <ul className="artifacts__list">
                  {list.map((artifact) => (
                    <li key={artifact.id}>
                      <button
                        type="button"
                        className="artifacts__row"
                        ref={(node) => {
                          if (node) rows.current.set(artifact.id, node);
                          else rows.current.delete(artifact.id);
                        }}
                        onClick={() => show({ id: artifact.id })}
                      >
                        <span className="artifacts__name">{artifact.title}</span>
                        <span className="artifacts__details">
                          {detailsLine(artifact, crew ? null : crewName(artifact.groupId), now)}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </div>

            {error && (
              <div className="banner banner--error artifacts__error">
                <span>{error}</span>
              </div>
            )}

            <div className="artifacts__foot">
              <span className="artifacts__spacer" />
              <button type="button" className="btn" onClick={onClose}>
                Close
              </button>
            </div>
          </>
        )}
      </div>
    </Scrim>
  );
}

function ArtifactView({
  id,
  onBack,
  onClose,
}: {
  id: ArtifactId;
  onBack: () => void;
  onClose: () => void;
}) {
  const agents = useStore((state) => state.agents);
  const groups = useStore((state) => state.groups);
  const activity = useStore((state) => state.activity);
  const changed = useStore((state) => state.artifactsVersion);
  const widgets = useStore((state) => state.settings?.widgets ?? null);
  const setSettings = useStore((state) => state.setSettings);
  const now = useNow(60_000);
  const why = useId();

  const [detail, setDetail] = useState<ArtifactDetail | null>(null);
  const [reads, setReads] = useState<ArtifactRead[] | null>(null);
  const [fresh, setFresh] = useState(0);
  const [said, setSaid] = useState<string | null>(null);
  const [viewing, setViewing] = useState<number | null>(null);
  const [history, setHistory] = useState(false);
  useDismiss(() => setHistory(false), history);
  const backRef = useRef<HTMLButtonElement>(null);
  const historyRef = useRef<HTMLButtonElement>(null);
  const wasHistory = useRef(false);
  useLayoutEffect(() => {
    if (history !== wasHistory.current) {
      (history ? backRef.current : historyRef.current)?.focus();
      wasHistory.current = history;
    }
  }, [history]);
  const [page, setPage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [proposal, setProposal] = useState<{
    json: string;
    artifact: Artifact;
    shown: number;
  } | null>(null);
  const proposalRef = useRef(false);
  const [sending, setSending] = useState(false);
  const detailRequest = useRef({ generation: 0, mounted: true });

  useEffect(() => {
    detailRequest.current.mounted = true;
    return () => {
      detailRequest.current.mounted = false;
      detailRequest.current.generation += 1;
    };
  }, []);

  const load = useCallback(async () => {
    if (!detailRequest.current.mounted) return;
    const generation = ++detailRequest.current.generation;
    try {
      const next = await api.artifactDetail(id);
      if (!detailRequest.current.mounted || generation !== detailRequest.current.generation) return;
      setDetail(next);
      setError(null);
    } catch (caught) {
      if (!detailRequest.current.mounted || generation !== detailRequest.current.generation) return;
      setError(errorMessage(caught));
    }
  }, [id]);

  useEffect(() => {
    void load();
  }, [load, changed]);

  const current = detail?.artifact.version ?? null;
  const shown = viewing ?? current;
  const allowed = detail?.artifact.sourcesAllowed ?? false;

  useEffect(() => {
    proposalRef.current = false;
    setProposal(null);
  }, [shown, detail?.artifact.owner?.id, detail?.artifact.owner?.gone, history]);

  useEffect(() => {
    if (shown === null) return;
    let live = true;
    void fresh;
    void allowed;
    setReads(null);
    api
      .artifactData(id, shown)
      .then((found) => live && setReads(found))
      .catch((caught) => live && setError(errorMessage(caught)));
    return () => {
      live = false;
    };
  }, [id, shown, fresh, allowed]);

  useEffect(() => {
    if (shown === null) return;
    let live = true;
    setPage(null);
    api
      .artifactPage(id, shown)
      .then((html) => live && setPage(html))
      .catch((caught) => live && setError(errorMessage(caught)));
    return () => {
      live = false;
    };
  }, [id, shown]);

  const act = async (work: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await work();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  const back = history ? () => setHistory(false) : onBack;

  if (!detail) {
    return (
      <>
        <div className="artifacts__head">
          <button type="button" className="btn btn--quiet" onClick={onBack}>
            <Icon name="chevron-left" size={16} />
            All artifacts
          </button>
        </div>
        <div className="artifacts__body">
          <p className="routines__note">{error ?? "Loading…"}</p>
        </div>
      </>
    );
  }

  const { artifact, log } = detail;
  const crew = groups.find((group) => group.id === artifact.groupId);

  const send = (json: string) => {
    const owner = artifact.owner;
    if (proposalRef.current || sendingPage || shown === null) return;
    if (json.length > 4000) {
      setSaid("This value is too long to send. The limit is 4,000 characters.");
      return;
    }
    if (!owner || owner.gone) {
      setSaid(
        "Nobody in the group owns this page, so a click has nobody to reach. Hand it to an agent from History.",
      );
      return;
    }
    const state = activity[owner.id]?.state;
    if (state === "thinking" || state === "queued") {
      setSaid(`${owner.name} is still working. Try again once they have finished.`);
      return;
    }
    proposalRef.current = true;
    setSaid(null);
    setProposal({ json, artifact, shown });
  };
  const crewAgents = agents.filter((agent) => agent.groupId === artifact.groupId);
  const earlier = viewing !== null && viewing !== artifact.version ? viewing : null;
  const pinned = widgets?.find((widget) => widget.artifactId === artifact.id) ?? null;
  const place = (width: WidgetWidth | "") =>
    void act(async () =>
      setSettings(
        width === ""
          ? await api.unpinArtifact(artifact.id)
          : await api.pinArtifact(artifact.id, width),
      ),
    );

  return (
    <>
      <div className="artifacts__head">
        <div className="artifacts__title-row">
          <button type="button" className="btn btn--quiet" onClick={back} ref={backRef}>
            <Icon name="chevron-left" size={16} />
            {history ? "Back to the page" : "All artifacts"}
          </button>
          <h2 className="dialog__title artifacts__title">{artifact.title}</h2>
        </div>
        <p className="artifacts__details">{detailsLine(artifact, crew?.name ?? null, now)}</p>
      </div>

      {history ? (
        <div className="artifacts__body">
          <label className="artifacts__owner">
            <span>Owner</span>
            <select
              className="input input--slim"
              aria-label="Owner"
              disabled={busy || crewAgents.length === 0}
              value={artifact.owner && !artifact.owner.gone ? artifact.owner.id : ""}
              onChange={(event) =>
                void act(() => api.handArtifact(artifact.id, event.target.value as AgentId))
              }
            >
              {(!artifact.owner || artifact.owner.gone) && (
                <option value="" disabled>
                  {ownerLabel(artifact.owner)}
                </option>
              )}
              {crewAgents.map((agent) => (
                <option key={agent.id} value={agent.id}>
                  {agent.name}
                </option>
              ))}
            </select>
          </label>
          <ol className="artifacts__log" aria-label="History">
            {logRows(log).map((row) => (
              <li key={row.key} className="artifacts__entry">
                {row.opens ? (
                  <button
                    type="button"
                    className="artifacts__version"
                    aria-pressed={row.version === shown}
                    onClick={() => {
                      setViewing(row.version);
                      setHistory(false);
                    }}
                  >
                    v{row.version}
                  </button>
                ) : (
                  <span className="artifacts__version">v{row.version}</span>
                )}
                <span>
                  {row.who} {row.what}
                  {row.note && <span className="artifacts__entry-note">. {row.note}</span>}
                </span>
                <time className="artifacts__entry-at" dateTime={new Date(row.at).toISOString()}>
                  {whenLabel(row.at, now)}
                </time>
              </li>
            ))}
          </ol>
        </div>
      ) : (
        <div className="artifacts__body artifacts__page">
          {artifact.sources.length > 0 && !artifact.sourcesAllowed && earlier === null && (
            <div className="artifacts__ask">
              <p className="artifacts__ask-text">
                This page reads live data. Each time it is opened, Guaca will make these calls as{" "}
                {ownerLabel(artifact.owner)}:
              </p>
              <ul className="artifacts__ask-list">
                {artifact.sources.map((source) => (
                  <li key={source.name}>
                    <code>{sourceLine(source)}</code>
                  </li>
                ))}
              </ul>
              <button
                type="button"
                className="btn btn--primary btn--compact"
                disabled={busy}
                onClick={() =>
                  void act(async () => {
                    try {
                      await api.allowArtifactSources(
                        artifact.id,
                        artifact.version,
                        artifact.owner?.id ?? null,
                        artifact.sources,
                      );
                    } catch (caught) {
                      await load();
                      throw caught;
                    }
                    await load();
                  })
                }
              >
                Allow these reads
              </button>
            </div>
          )}
          {said && <p className="artifacts__said">{said}</p>}
          {proposal &&
            proposal.shown === shown &&
            proposal.artifact.version === artifact.version &&
            proposal.artifact.owner?.id === artifact.owner?.id &&
            !artifact.owner?.gone && (
              <div className="artifact__answer">
                <p className="artifact__answer-note">Send this value to {artifact.owner?.name}?</p>
                <pre className="artifact__answer-value">{proposal.json}</pre>
                <div className="artifact__answer-acts">
                  <button
                    type="button"
                    className="btn btn--compact"
                    disabled={sending}
                    onClick={() => {
                      const owner = artifact.owner;
                      if (!owner || owner.gone || sendingPage || !proposalRef.current) return;
                      const state = useStore.getState().activity[owner.id]?.state;
                      if (state === "thinking" || state === "queued") {
                        setSaid(
                          `${owner.name} is still working. Try again once they have finished.`,
                        );
                        return;
                      }
                      sendingPage = true;
                      proposalRef.current = false;
                      setSending(true);
                      void api
                        .sendMessage(owner.id, sentMessage(proposal.artifact, proposal.json))
                        .then(() => {
                          setSaid(
                            `Sent to ${owner.name}. The page changes here when ${owner.name} updates it.`,
                          );
                          setProposal(null);
                        })
                        .catch((caught) => {
                          setError(errorMessage(caught));
                          setProposal(null);
                        })
                        .finally(() => {
                          sendingPage = false;
                          setSending(false);
                        });
                    }}
                  >
                    {sending ? "Sending…" : `Send to ${artifact.owner?.name}`}
                  </button>
                  <button
                    type="button"
                    className="btn btn--quiet btn--compact"
                    disabled={sending}
                    onClick={() => {
                      proposalRef.current = false;
                      setProposal(null);
                    }}
                  >
                    Dismiss
                  </button>
                </div>
              </div>
            )}
          {earlier !== null && (
            <div className="artifacts__old">
              <span>
                Version {earlier} of {artifact.version}.
              </span>
              <button
                type="button"
                className="btn btn--compact"
                disabled={busy}
                onClick={() =>
                  void act(async () => {
                    await api.restoreArtifact(artifact.id, earlier);
                    setViewing(null);
                    await load();
                  })
                }
              >
                Put this version back
              </button>
              <button
                type="button"
                className="btn btn--quiet btn--compact"
                onClick={() => setViewing(null)}
              >
                Show the current one
              </button>
            </div>
          )}
          {page === null ? (
            <p className="routines__note">Opening…</p>
          ) : (
            <HtmlArtifact
              key={`${artifact.id}:${shown}`}
              html={page}
              title={artifact.title}
              data={reads === null ? undefined : pageData(reads)}
              onSend={send}
              fill
            />
          )}
        </div>
      )}

      {error && (
        <div className="banner banner--error artifacts__error">
          <span>{error}</span>
        </div>
      )}

      <div className="artifacts__foot">
        {!history && (
          <button
            type="button"
            className="btn btn--quiet"
            onClick={(event) => {
              event.currentTarget.focus();
              setHistory(true);
            }}
            ref={historyRef}
          >
            History
          </button>
        )}
        {!history && artifact.sourcesAllowed && (
          <button
            type="button"
            className="btn btn--quiet"
            onClick={() => setFresh((count) => count + 1)}
          >
            Refresh
          </button>
        )}
        {!history && widgets !== null && (
          <>
            <label className="artifacts__owner artifacts__pin">
              <span>Status bar</span>
              <select
                className="input input--slim"
                aria-label="Status bar"
                aria-describedby={artifact.condensed ? undefined : why}
                disabled={busy}
                value={pinned?.width ?? ""}
                onChange={(event) => place(event.target.value as WidgetWidth | "")}
              >
                <option value="">Not on it</option>
                <option value="narrow" disabled={!artifact.condensed}>
                  Narrow
                </option>
                <option value="wide" disabled={!artifact.condensed}>
                  Wide
                </option>
              </select>
            </label>
            {!artifact.condensed && (
              <span className="artifacts__pin-why" id={why}>
                {pinned
                  ? "This version has no condensed view; it can be taken off the bar."
                  : artifact.owner && !artifact.owner.gone
                    ? `No condensed view yet; ask ${artifact.owner.name} to add one.`
                    : "No condensed view yet; any agent in its group can add one."}
              </span>
            )}
          </>
        )}
        <span className="artifacts__spacer" />
        {confirming ? (
          <>
            <button
              type="button"
              className="btn btn--danger"
              disabled={busy}
              onClick={() =>
                void act(async () => {
                  await api.deleteArtifact(artifact.id);
                  onBack();
                })
              }
            >
              Delete it and its history
            </button>
            <button type="button" className="btn btn--quiet" onClick={() => setConfirming(false)}>
              Keep
            </button>
          </>
        ) : (
          <button type="button" className="btn btn--quiet" onClick={() => setConfirming(true)}>
            Delete
          </button>
        )}
        <button type="button" className="btn" onClick={onClose}>
          Close
        </button>
      </div>
    </>
  );
}

export function ArtifactCard({ made }: { made: ArtifactMade }) {
  const show = useStore((state) => state.showArtifacts);
  return (
    <button type="button" className="artifact-card" onClick={() => show({ id: made.id })}>
      <span className="artifact-card__mark">
        <Icon name="artifact" />
      </span>
      <span className="artifact-card__text">
        <span className="artifact-card__title">{made.title}</span>
        <span className="artifact-card__kind">
          {made.version === 1 ? "New artifact" : `Artifact, version ${made.version}`}
        </span>
      </span>
      <Icon name="chevron-right" />
    </button>
  );
}
