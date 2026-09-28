import { useCallback, useEffect, useRef, useState } from "react";

import {
  detailsLine,
  logRows,
  ownerLabel,
  pageData,
  sentMessage,
  sourceLine,
} from "../lib/artifacts";
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

interface Props {
  onClose: () => void;
}

/**
 * The pages the crews keep, and one of them opened.
 *
 * ## The page is the point, and everything else is quiet
 *
 * The operator comes here for the page. Who owns it, which version it is and
 * when it last changed are one faint line under its name, and the history is
 * behind a button: it is there for working out what happened, by the operator
 * or by an agent asked about it, and it is not something anybody reads on the
 * way to the page. It took a column beside the page when it was first drawn,
 * and that spent the width of the dialog on the thing nobody opened it for.
 *
 * ## What it lists is what the rail is showing
 *
 * Inside a crew, that crew's artifacts; with every crew in the rail, every
 * crew's, with the crew leading each row's line. There is no filter of its own,
 * because the rail already is one and a second control deciding the same thing
 * would be a second answer to disagree with the first. That is a difference
 * from the calendar, which always opens on every crew.
 *
 * ## The page runs as a fenced one does, with nobody to answer
 *
 * Same origin, same policy. No `Answering` is provided here, so a page that
 * calls `guaca.answer` hands back nothing. Every history row that made a
 * version opens that version, and putting one back is a new version, never a
 * rewind, so the page it replaced is still there to put back in turn.
 *
 * ## A page can read, and a click on it can reach its owner
 *
 * A page that declares reads asks for them here, once, as a list the operator
 * can see in full: each connector, each tool, and exactly what it will be
 * sent. Allowed, they run every time the page is opened, with no model in the
 * loop, and again on Refresh. A page's `guaca.send` goes to its owner as the
 * operator's message, and only when the operator's click is what caused it:
 * one at a time, so a page cannot queue a stream of turns while the owner is
 * still working on the last.
 *
 * Ownership is handed from the history, and nowhere else by the operator.
 * Anyone in the crew may edit, so there is nothing to grant; what the operator
 * decides is who answers for the page, which is the way out when its owner has
 * left, and a rare enough act to live beside the record of who held it.
 *
 * ## A page with a condensed view can go on the status bar
 *
 * From the foot, as a width or not at all. The operator's own pin asks nobody
 * and allows nothing: reads are allowed here, above the page, where the list
 * is drawn in full. A page pinned while its current version has no condensed
 * view, which a restore can do, can only be taken off.
 */
export function Artifacts({ onClose }: Props) {
  const open = useStore((state) => state.artifactsOpen);
  const show = useStore((state) => state.showArtifacts);
  const railGroup = useStore((state) => state.railGroup);
  const groups = useStore((state) => state.groups);
  // Bumped whenever an agent or the operator changes one, so the list on screen
  // is the one that is true now rather than when the dialog opened.
  const changed = useStore((state) => state.artifactsVersion);
  const now = useNow(60_000);

  const [list, setList] = useState<Artifact[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const id = open?.id ?? null;

  useEffect(() => {
    panelRef.current?.focus();
  }, []);

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
    <div className="scrim">
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
            <Escape onEscape={onClose} />
            <div className="artifacts__head">
              <h2 className="dialog__title">Artifacts</h2>
              <p className="artifacts__details">{crew ? crew.name : "Every crew"}</p>
            </div>

            <div className="artifacts__body">
              {list === null ? (
                <p className="routines__note">Loading…</p>
              ) : list.length === 0 ? (
                <p className="routines__note">
                  {crew
                    ? `${crew.name} keeps no artifacts yet.`
                    : "No crew keeps any artifacts yet."}{" "}
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
              <p className="artifacts__note">
                Pages your agents keep. Ask one to make or change a page; every change is kept as a
                version.
              </p>
              <button type="button" className="btn" onClick={onClose}>
                Close
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

/**
 * Escape, for whichever view is on top. Each view says what backing out of it
 * means, so Escape on the history closes the history rather than the dialog out
 * from under it.
 */
function Escape({ onEscape }: { onEscape: () => void }) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onEscape();
    };
    globalThis.addEventListener("keydown", onKey);
    return () => globalThis.removeEventListener("keydown", onKey);
  }, [onEscape]);
  return null;
}

/** One artifact: its page at a version, and behind a button, its history. */
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

  const [detail, setDetail] = useState<ArtifactDetail | null>(null);
  const [reads, setReads] = useState<ArtifactRead[] | null>(null);
  // Bumped by Refresh, so the reads run again without anything else changing.
  const [fresh, setFresh] = useState(0);
  // What became of the last thing the page sent, said once, above the page.
  const [said, setSaid] = useState<string | null>(null);
  // `null` is the current version, followed as it moves. A number is one the
  // operator picked from the history, and stays put while agents keep editing.
  const [viewing, setViewing] = useState<number | null>(null);
  const [history, setHistory] = useState(false);
  const [page, setPage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);

  const load = useCallback(async () => {
    try {
      setDetail(await api.artifactDetail(id));
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught));
    }
  }, [id]);

  useEffect(() => {
    void load();
  }, [load, changed]);

  const current = detail?.artifact.version ?? null;
  const shown = viewing ?? current;
  const allowed = detail?.artifact.sourcesAllowed ?? false;

  // A version's reads, made when it is shown and again on Refresh or once the
  // operator allows them. Every version is asked, because Rust is where it is
  // decided whether that version's list is the allowed one; a page with no
  // reads gets an empty object, which is still an answer to `guaca.data()`.
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
        <Escape onEscape={onBack} />
        <div className="artifacts__head">
          <button type="button" className="btn btn--ghost" onClick={onBack}>
            ‹ All artifacts
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

  // Where a click on the page goes. Refused here rather than sent and
  // explained later, because every refusal is something the operator can see
  // the reason for on the screen they are already looking at.
  const send = (json: string, clicked: boolean) => {
    const owner = artifact.owner;
    if (!clicked) {
      setSaid("The page tried to send something without a click, so nothing was sent.");
      return;
    }
    if (!owner || owner.gone) {
      setSaid(
        "Nobody in the crew owns this page, so a click has nobody to reach. Hand it to an agent from History.",
      );
      return;
    }
    const state = activity[owner.id]?.state;
    if (state === "thinking" || state === "queued") {
      setSaid(`${owner.name} is still working. Try again once they have finished.`);
      return;
    }
    void act(async () => {
      await api.sendMessage(owner.id, sentMessage(artifact, json));
      setSaid(`Sent to ${owner.name}. The page changes here when ${owner.name} updates it.`);
    });
  };
  const crewAgents = agents.filter((agent) => agent.groupId === artifact.groupId);
  // An earlier version the operator picked, or `null` while the current one is
  // on screen. The one place a restore can be offered from.
  const earlier = viewing !== null && viewing !== artifact.version ? viewing : null;
  // `null` from a host older than the bar's pages, which offers no control.
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
      <Escape onEscape={back} />
      <div className="artifacts__head">
        <div className="artifacts__title-row">
          <button type="button" className="btn btn--ghost" onClick={back}>
            {history ? "‹ Back to the page" : "‹ All artifacts"}
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
              {/* The current owner stays readable while they are gone, rather
                  than the field quietly showing somebody else: nothing has
                  changed hands until the operator picks. */}
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
          {/* A permission, so it is drawn in full rather than kept quiet: the
              operator is allowing exactly these calls, with exactly these
              arguments, every time the page is opened. */}
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
                className="btn btn--primary btn--small"
                disabled={busy}
                onClick={() => void act(() => api.allowArtifactSources(artifact.id).then(load))}
              >
                Allow these reads
              </button>
            </div>
          )}
          {said && <p className="artifacts__said">{said}</p>}
          {earlier !== null && (
            <div className="artifacts__old">
              <span>
                Version {earlier} of {artifact.version}.
              </span>
              <button
                type="button"
                className="btn btn--small"
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
                className="btn btn--ghost btn--small"
                onClick={() => setViewing(null)}
              >
                Show the current one
              </button>
            </div>
          )}
          {page === null ? (
            <p className="routines__note">Opening…</p>
          ) : (
            // Keyed by version so switching versions frames the new page rather
            // than keeping whatever the operator had done in the old one.
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
          <button type="button" className="btn btn--ghost" onClick={() => setHistory(true)}>
            History
          </button>
        )}
        {!history && artifact.sourcesAllowed && (
          <button
            type="button"
            className="btn btn--ghost"
            onClick={() => setFresh((count) => count + 1)}
          >
            Refresh
          </button>
        )}
        {!history && widgets !== null && (artifact.condensed || pinned) && (
          <label className="artifacts__owner artifacts__pin">
            <span>Status bar</span>
            <select
              className="input input--slim"
              aria-label="Status bar"
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
            <button type="button" className="btn btn--ghost" onClick={() => setConfirming(false)}>
              Keep
            </button>
          </>
        ) : (
          <button type="button" className="btn btn--ghost" onClick={() => setConfirming(true)}>
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

/**
 * The card a turn leaves when it made or changed a kept page.
 *
 * Drawn from the tool call rather than from the reply, because the call is
 * what knows which artifact and which version: the id is assigned by the
 * write. The title and version are the ones at that moment, so an old card
 * still says what it was about after the page has moved on; opening it opens
 * the artifact as it is now.
 */
export function ArtifactCard({ made }: { made: ArtifactMade }) {
  const show = useStore((state) => state.showArtifacts);
  return (
    <button type="button" className="artifact-card" onClick={() => show({ id: made.id })}>
      <span className="artifact-card__mark">
        <ArtifactMark />
      </span>
      <span className="artifact-card__text">
        <span className="artifact-card__title">{made.title}</span>
        <span className="artifact-card__kind">
          {made.version === 1 ? "New artifact" : `Artifact, version ${made.version}`}
        </span>
      </span>
      <svg className="artifact-card__go" viewBox="0 0 24 24" aria-hidden="true">
        <path d="M9.5 6l6 6-6 6" />
      </svg>
    </button>
  );
}

/**
 * The mark for Artifacts: a page with a folded corner. One drawing for the
 * rail's button and the card a turn leaves, because the card is a way into the
 * same place and has to look like it.
 */
export function ArtifactMark() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M13.5 3.5H7A1.5 1.5 0 0 0 5.5 5v14A1.5 1.5 0 0 0 7 20.5h10a1.5 1.5 0 0 0 1.5-1.5V8.5z" />
      <path d="M13.5 3.5v5h5M9 13h6M9 16.5h4" />
    </svg>
  );
}
