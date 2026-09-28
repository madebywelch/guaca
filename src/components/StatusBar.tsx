import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { prefersDark, resolveSurface, watchSystemSurface } from "../lib/appearance";
import { pageData } from "../lib/artifacts";
import { COMMIT, VERSION } from "../lib/build";
import { artifactUrl } from "../lib/files";
import { api } from "../lib/ipc";
import { quickTitle } from "../lib/quick";
import { shortBuild, skew } from "../lib/releases";
import { useStore } from "../lib/store";
import { desktop } from "../lib/transport";
import {
  type AgentId,
  type ArtifactRead,
  type CondensedView,
  errorMessage,
  type QuickAction,
  type QuickDoes,
  type QuickPlace,
  type Widget,
} from "../lib/types";
import { type BarLook, barLook, dressed } from "../lib/widget";
import { useHostState } from "./HostUpdates";

interface Props {
  /** Opens a place in the app, the way the rail and the palette do. */
  onOpen: (place: QuickPlace) => void;
  /** Sends as the operator, to the agent whose channel it then shows. */
  onMessage: (agentId: AgentId, text: string) => Promise<void>;
}

/**
 * The strip along the bottom of the reading column: which host this window
 * shows, the pages pinned to it, and the operator's quick actions.
 *
 * A pinned page is a kept artifact drawn from its condensed view. An agent asks
 * for one with `artifact` and `pin`, and it appears only once the operator has
 * approved it, reads included; the operator pins their own from Artifacts. A
 * quick action is a button of the operator's own for something done often.
 * Every window draws the same bar, because both are settings on the host.
 */
export function StatusBar({ onOpen, onMessage }: Props) {
  const settings = useStore((s) => s.settings);
  const setSettings = useStore((s) => s.setSettings);
  const agents = useStore((s) => s.agents);
  const host = useHostState();
  const look = useBarLook();
  const [editing, setEditing] = useState(false);
  const [running, setRunning] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const actions = settings?.quickActions ?? [];
  const widgets = settings?.widgets ?? [];
  const nameOf = (id: AgentId) => agents.find((a) => a.id === id)?.name ?? "a deleted agent";

  const version = host?.health?.version;
  const drift = desktop
    ? skew({ version: VERSION, commit: COMMIT }, host?.health ?? null)
    : "unknown";
  // One version on two builds reads as a match unless the label says which.
  const hostLabel = !version
    ? "Host"
    : drift === "otherBuild"
      ? `Host ${version} · ${shortBuild(host?.health?.build ?? "")}`
      : `Host ${version}`;
  const hostTitle =
    drift === "hostBehind"
      ? `This host runs Guaca ${version} and this app is ${VERSION}. Open Workspace settings to update it.`
      : drift === "clientBehind"
        ? `This host runs Guaca ${version} and this app is ${VERSION}. Update this app.`
        : drift === "otherBuild"
          ? `This host and this app are different builds of Guaca ${version}. Open Workspace settings to see which.`
          : "Which host this window shows, and its updates";

  const run = async (action: QuickAction) => {
    setError(null);
    if (action.does.kind === "open") {
      onOpen(action.does.place);
      return;
    }
    setRunning(action.id);
    try {
      await onMessage(action.does.agentId, action.does.text);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setRunning(null);
    }
  };

  const remove = async (action: QuickAction) => {
    setError(null);
    try {
      setSettings(await api.removeQuickAction(action.id));
    } catch (caught) {
      setError(errorMessage(caught));
    }
  };

  const unpin = async (widget: Widget) => {
    setError(null);
    try {
      setSettings(await api.unpinArtifact(widget.artifactId));
    } catch (caught) {
      setError(errorMessage(caught));
    }
  };

  return (
    <footer className="statusbar">
      <button
        type="button"
        className="statusbar__host"
        data-drift={drift === "same" || drift === "unknown" ? undefined : ""}
        title={hostTitle}
        onClick={() => onOpen({ kind: "settings", section: "workspace" })}
      >
        {hostLabel}
      </button>
      {widgets.length > 0 && (
        <div className="statusbar__widgets" role="toolbar" aria-label="Pinned pages">
          {widgets.map((widget) => (
            <PinnedPage
              key={widget.artifactId}
              widget={widget}
              look={look}
              editing={editing}
              onUnpin={() => void unpin(widget)}
            />
          ))}
        </div>
      )}
      <div className="statusbar__actions" role="toolbar" aria-label="Quick actions">
        {actions.map((action) => (
          <span className="statusbar__action" key={action.id}>
            <button
              type="button"
              className="statusbar__button"
              title={quickTitle(action, nameOf)}
              disabled={running !== null}
              onClick={() => void run(action)}
            >
              {action.label}
            </button>
            {editing && (
              <button
                type="button"
                className="statusbar__remove"
                aria-label={`Remove ${action.label}`}
                onClick={() => void remove(action)}
              >
                ×
              </button>
            )}
          </span>
        ))}
        <button
          type="button"
          className="statusbar__edit"
          aria-pressed={editing}
          onClick={() => setEditing(!editing)}
        >
          {editing
            ? "Done"
            : actions.length === 0 && widgets.length === 0
              ? "Add a quick action"
              : "Edit"}
        </button>
      </div>
      {error && (
        <span className="statusbar__error" role="alert">
          {error}
        </span>
      )}
      {editing && <QuickActionForm onAdded={() => setError(null)} />}
    </footer>
  );
}

/**
 * The operator's appearance, as a condensed view is drawn in it. Solved from
 * the preferences rather than read off the document, so it is right on the
 * render that changes it rather than one render later, and followed through
 * the OS for an operator whose surface follows the OS.
 */
function useBarLook(): BarLook {
  const prefs = useStore((s) => s.prefs);
  const [dark, setDark] = useState(prefersDark);
  useEffect(() => watchSystemSurface(setDark), []);
  const { grays, attention, contrast, surface, uiScale } = prefs;
  return useMemo(
    () => barLook({ grays, attention, contrast }, resolveSurface(surface, dark), uiScale),
    [grays, attention, contrast, surface, uiScale, dark],
  );
}

/**
 * One pinned page, drawn from its condensed view.
 *
 * The frame takes no pointer events and a button of the bar's own lies over
 * it, so every click is the bar's and opens the page in Artifacts, where its
 * reads are allowed, its history is kept and its own buttons reach its owner.
 * A frame on another origin swallows its clicks, so this is also the only way
 * the bar could open anything; and a strip of small buttons that each spend a
 * turn of the owner's is a strip of mis-clicks. The frame's own messages are
 * never listened to: a condensed view has nothing to say to anybody.
 *
 * Its reads run on the pin's clock, with no model, and not while the window
 * is hidden: a read nobody can see is a connector call spent on nothing. Coming
 * back to the window reads again if the last one is older than the clock.
 */
function PinnedPage({
  widget,
  look,
  editing,
  onUnpin,
}: {
  widget: Widget;
  look: BarLook;
  editing: boolean;
  onUnpin: () => void;
}) {
  const changed = useStore((s) => s.artifactsVersion);
  const show = useStore((s) => s.showArtifacts);
  const id = widget.artifactId;
  const [view, setView] = useState<CondensedView | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [reads, setReads] = useState<ArtifactRead[] | null>(null);
  const [src, setSrc] = useState<string | null>(null);
  const frame = useRef<HTMLIFrameElement>(null);
  const loaded = useRef(false);

  useEffect(() => {
    let live = true;
    void changed;
    api
      .artifactCondensed(id)
      .then((found) => {
        if (!live) return;
        setView(found);
        setFailed(null);
      })
      .catch((caught) => live && setFailed(errorMessage(caught)));
    return () => {
      live = false;
    };
  }, [id, changed]);

  const version = view?.artifact.version ?? null;
  const declares = (view?.artifact.sources.length ?? 0) > 0;
  const allowed = view?.artifact.sourcesAllowed ?? false;
  useEffect(() => {
    if (version === null) return;
    // A page that reads nothing still gets an answer to `guaca.data()`.
    if (!declares) {
      setReads([]);
      return;
    }
    let live = true;
    let last = 0;
    const every = widget.everyMinutes * 60_000;
    const read = () => {
      if (document.visibilityState === "hidden") return;
      last = Date.now();
      api
        .artifactData(id)
        .then((found) => live && setReads(found))
        .catch(() => {});
    };
    read();
    // Reads the operator has not allowed come back refused, once, so the page
    // can say so. Asking again on a clock would only say it again.
    if (!allowed) {
      return () => {
        live = false;
      };
    }
    const timer = window.setInterval(read, every);
    const onVisible = () => {
      if (document.visibilityState === "visible" && Date.now() - last >= every) read();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      live = false;
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [id, version, declares, allowed, widget.everyMinutes]);

  const html = view?.page ?? null;
  const framed = useMemo(() => (html === null ? null : dressed(html, look)), [html, look]);
  useEffect(() => {
    setSrc(null);
    loaded.current = false;
    if (framed === null) return;
    let live = true;
    api
      .frameArtifact(framed)
      .then((at) => live && setSrc(artifactUrl(at)))
      .catch((caught) => live && setFailed(errorMessage(caught)));
    return () => {
      live = false;
    };
  }, [framed]);

  const post = useCallback(() => {
    if (reads === null || !loaded.current) return;
    frame.current?.contentWindow?.postMessage(
      { guaca: "artifact-data", data: pageData(reads) },
      "*",
    );
  }, [reads]);
  useEffect(() => {
    post();
  }, [post]);

  const title = view?.artifact.title ?? "A pinned page";
  return (
    <span className="statusbar__action">
      <span className="widget" data-width={widget.width}>
        {src ? (
          <iframe
            ref={frame}
            className="widget__frame"
            // Scripts, and deliberately nothing else: `HtmlArtifact` says why.
            sandbox="allow-scripts"
            referrerPolicy="no-referrer"
            title={title}
            tabIndex={-1}
            aria-hidden="true"
            src={src}
            onLoad={() => {
              loaded.current = true;
              post();
            }}
          />
        ) : (
          // A version without a condensed view, which a restore can bring back:
          // the title holds its place. Nothing while one is on its way, so the
          // title does not flash up and vanish.
          <span className="widget__title">{view && html === null ? title : ""}</span>
        )}
        <button
          type="button"
          className="widget__open"
          aria-label={`Open ${title}`}
          title={failed ?? title}
          onClick={() => show({ id })}
        />
      </span>
      {editing && (
        <button
          type="button"
          className="statusbar__remove"
          aria-label={`Unpin ${title}`}
          onClick={onUnpin}
        >
          ×
        </button>
      )}
    </span>
  );
}

type Kind = "message" | "channel" | "calendar" | "forYou" | "settings";

/** The operator's own button. Theirs, so it needs nobody's approval. */
function QuickActionForm({ onAdded }: { onAdded: () => void }) {
  const agents = useStore((s) => s.agents).filter((a) => a.lifecycle !== "terminated");
  const setSettings = useStore((s) => s.setSettings);
  const [label, setLabel] = useState("");
  const [kind, setKind] = useState<Kind>("message");
  const [agentId, setAgentId] = useState<AgentId>(agents[0]?.id ?? "");
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const does = (): QuickDoes => {
    switch (kind) {
      case "message":
        return { kind: "message", agentId, text };
      case "channel":
        return { kind: "open", place: { kind: "channel", agentId } };
      case "calendar":
        return { kind: "open", place: { kind: "calendar" } };
      case "forYou":
        return { kind: "open", place: { kind: "forYou" } };
      case "settings":
        return { kind: "open", place: { kind: "settings", section: null } };
    }
  };
  const needsAgent = kind === "message" || kind === "channel";
  const ready =
    label.trim() !== "" && (!needsAgent || agentId !== "") && (kind !== "message" || text.trim());

  const add = async () => {
    setBusy(true);
    setError(null);
    try {
      setSettings(await api.addQuickAction(label, does()));
      setLabel("");
      setText("");
      onAdded();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="statusbar__form">
      <label className="field">
        <span className="field__label">Label</span>
        <input
          className="input"
          placeholder="Morning brief"
          value={label}
          onChange={(event) => setLabel(event.target.value)}
        />
      </label>
      <label className="field">
        <span className="field__label">When pressed</span>
        <select
          className="select"
          value={kind}
          onChange={(event) => setKind(event.target.value as Kind)}
        >
          <option value="message">Send a message</option>
          <option value="channel">Open a channel</option>
          <option value="calendar">Open the calendar</option>
          <option value="forYou">Open For You</option>
          <option value="settings">Open Settings</option>
        </select>
      </label>
      {needsAgent && (
        <label className="field">
          <span className="field__label">{kind === "message" ? "To" : "Whose channel"}</span>
          <select
            className="select"
            value={agentId}
            onChange={(event) => setAgentId(event.target.value)}
          >
            {agents.map((agent) => (
              <option key={agent.id} value={agent.id}>
                {agent.name}
              </option>
            ))}
          </select>
        </label>
      )}
      {kind === "message" && (
        <label className="field">
          <span className="field__label">Message</span>
          <textarea
            className="textarea"
            rows={3}
            value={text}
            onChange={(event) => setText(event.target.value)}
          />
        </label>
      )}
      <div className="access__row">
        <button
          type="button"
          className="btn btn--small btn--primary"
          disabled={busy || !ready}
          onClick={() => void add()}
        >
          Add to the status bar
        </button>
      </div>
      {error && (
        <p className="field__error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
