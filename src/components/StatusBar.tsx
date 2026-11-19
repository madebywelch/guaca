import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { prefersDark, resolveSurface, watchSystemSurface } from "../lib/appearance";
import { pageData } from "../lib/artifacts";
import { COMMIT, VERSION } from "../lib/build";
import { artifactUrl } from "../lib/files";
import { hostLabel, hostPlace } from "../lib/host";
import { api } from "../lib/ipc";
import { questions } from "../lib/questions";
import { quickTitle } from "../lib/quick";
import { skew } from "../lib/releases";
import { forYouCount, useStore } from "../lib/store";
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
import { type BarFace, type BarLook, barFace, barLook, dressed } from "../lib/widget";
import { useHostState } from "./HostUpdates";
import { Icon } from "./Icon";

interface Props {
  onOpen: (place: QuickPlace) => void;
  onMessage: (agentId: AgentId, text: string) => Promise<void>;
}

export function StatusBar({ onOpen, onMessage }: Props) {
  const settings = useStore((s) => s.settings);
  const setSettings = useStore((s) => s.setSettings);
  const agents = useStore((s) => s.agents);
  const link = useStore((s) => s.link);
  const lastHeard = useStore((s) => s.lastHeard);
  const showArtifacts = useStore((s) => s.showArtifacts);
  const waiting = useStore(forYouCount);
  const host = useHostState();
  const look = useBarLook();
  const [editing, setEditing] = useState(false);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const actions = settings?.quickActions ?? [];
  const widgets = settings?.widgets ?? [];
  const empty = actions.length === 0 && widgets.length === 0;
  const nameOf = (id: AgentId) => agents.find((a) => a.id === id)?.name ?? "a deleted agent";

  const drift = desktop
    ? skew({ version: VERSION, commit: COMMIT }, host?.health ?? null)
    : "unknown";
  const label = hostLabel({
    place: hostPlace(),
    drift,
    host: host?.health ?? null,
    lastHeard: link === "down" ? lastHeard : null,
  });

  const run = async (action: QuickAction) => {
    setError(null);
    if (action.does.kind === "open") {
      onOpen(action.does.place);
      return;
    }
    setRunning(true);
    try {
      await onMessage(action.does.agentId, action.does.text);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setRunning(false);
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

  const opened = new Set(
    actions.flatMap((action) => (action.does.kind === "open" ? [action.does.place.kind] : [])),
  );

  return (
    <footer className="statusbar">
      <button
        type="button"
        className="statusbar__host"
        data-drift={label.drift ? "" : undefined}
        data-stale={link === "down" ? "" : undefined}
        title={label.title}
        onClick={() => onOpen({ kind: "settings", section: "workspace" })}
      >
        {label.text}
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
      {empty && (
        <span className="statusbar__empty">A kept page can be pinned here, from Artifacts.</span>
      )}
      <div className="statusbar__actions" role="toolbar" aria-label="Quick actions">
        {actions.map((action) => (
          <span className="statusbar__action" key={action.id}>
            <button
              type="button"
              className="statusbar__button"
              title={quickTitle(action, nameOf)}
              disabled={running}
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
                <Icon name="close" size={16} />
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
          {editing ? "Done" : empty ? "Add a quick action" : "Edit"}
        </button>
      </div>
      <nav className="statusbar__places" aria-label="Places">
        <button
          type="button"
          className="statusbar__place"
          onClick={() => showArtifacts({ id: null })}
        >
          Artifacts
        </button>
        {!opened.has("calendar") && (
          <button
            type="button"
            className="statusbar__place"
            onClick={() => onOpen({ kind: "calendar" })}
          >
            Calendar
          </button>
        )}
        {!opened.has("forYou") && (
          <button
            type="button"
            className="statusbar__place"
            aria-label={waiting > 0 ? `For you, ${questions(waiting)}` : "For you"}
            onClick={() => onOpen({ kind: "forYou" })}
          >
            For you
            {waiting > 0 && <b className="statusbar__count">{questions(waiting)}</b>}
          </button>
        )}
      </nav>
      {error && (
        <span className="statusbar__error" role="alert">
          {error}
        </span>
      )}
      {editing && <QuickActionForm onAdded={() => setError(null)} />}
    </footer>
  );
}

function useBarLook(): BarLook {
  const prefs = useStore((s) => s.prefs);
  const [dark, setDark] = useState(prefersDark);
  useEffect(() => watchSystemSurface(setDark), []);
  const { grays, attention, contrast, temperature, surface, uiScale, typeface } = prefs;
  return useMemo(
    () =>
      barLook(
        { grays, attention, contrast, temperature },
        resolveSurface(surface, dark),
        uiScale,
        typeface,
      ),
    [grays, attention, contrast, temperature, surface, uiScale, typeface, dark],
  );
}

type RefusalRead = { kind: "waiting" } | { kind: "sent" } | { kind: "disposed" };

export function refusalRead(state: RefusalRead, event: "visible" | "dispose"): RefusalRead {
  if (event === "dispose") return { kind: "disposed" };
  return state.kind === "waiting" ? { kind: "sent" } : state;
}

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
    if (!allowed) {
      let state: RefusalRead = { kind: "waiting" };
      const onVisible = () => {
        if (document.visibilityState !== "visible") return;
        const next = refusalRead(state, "visible");
        if (state.kind === "waiting" && next.kind === "sent") {
          read();
          document.removeEventListener("visibilitychange", onVisible);
        }
        state = next;
      };
      if (document.visibilityState === "hidden") {
        document.addEventListener("visibilitychange", onVisible);
      } else {
        read();
        state = refusalRead(state, "visible");
      }
      return () => {
        state = refusalRead(state, "dispose");
        live = false;
        document.removeEventListener("visibilitychange", onVisible);
      };
    }
    read();
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
  const [face, setFace] = useState<BarFace | null>(null);
  useEffect(() => {
    let live = true;
    void barFace(look.typeface).then((found) => live && setFace(found));
    return () => {
      live = false;
    };
  }, [look.typeface]);
  const framed = useMemo(
    () => (html === null || face === null ? null : dressed(html, look, face)),
    [html, look, face],
  );
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
          <Icon name="close" size={16} />
        </button>
      )}
    </span>
  );
}

type Kind = "message" | "channel" | "calendar" | "forYou" | "settings";

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
          <option value="channel">Open a conversation</option>
          <option value="calendar">Open the calendar</option>
          <option value="forYou">Open For you</option>
          <option value="settings">Open Settings</option>
        </select>
      </label>
      {needsAgent && (
        <label className="field">
          <span className="field__label">{kind === "message" ? "To" : "Whose conversation"}</span>
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
          className="btn btn--compact btn--primary"
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
