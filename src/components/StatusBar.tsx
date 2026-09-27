import { useState } from "react";

import { VERSION } from "../lib/build";
import { api } from "../lib/ipc";
import { quickTitle } from "../lib/quick";
import { skew } from "../lib/releases";
import { useStore } from "../lib/store";
import { desktop } from "../lib/transport";
import {
  type AgentId,
  errorMessage,
  type QuickAction,
  type QuickDoes,
  type QuickPlace,
} from "../lib/types";
import { useHostState } from "./HostUpdates";

interface Props {
  /** Opens a place in the app, the way the rail and the palette do. */
  onOpen: (place: QuickPlace) => void;
  /** Sends as the operator, to the agent whose channel it then shows. */
  onMessage: (agentId: AgentId, text: string) => Promise<void>;
}

/**
 * The strip along the bottom of the reading column: which host this window
 * shows, and the operator's quick actions.
 *
 * A quick action is a button for something done often. The operator adds them
 * here; an agent can ask for one through `settings`, and it appears only once
 * the operator has approved it with the whole message in front of them. Every
 * window draws the same bar, because the buttons are a setting on the host.
 */
export function StatusBar({ onOpen, onMessage }: Props) {
  const settings = useStore((s) => s.settings);
  const setSettings = useStore((s) => s.setSettings);
  const agents = useStore((s) => s.agents);
  const host = useHostState();
  const [editing, setEditing] = useState(false);
  const [running, setRunning] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const actions = settings?.quickActions ?? [];
  const nameOf = (id: AgentId) => agents.find((a) => a.id === id)?.name ?? "a deleted agent";

  const version = host?.health?.version;
  const drift = desktop ? skew(VERSION, host?.health ?? null) : "unknown";
  const hostLabel = version ? `Host ${version}` : "Host";
  const hostTitle =
    drift === "hostBehind"
      ? `This host runs Guaca ${version} and this app is ${VERSION}. Open Workspace settings to update it.`
      : drift === "clientBehind"
        ? `This host runs Guaca ${version} and this app is ${VERSION}. Update this app.`
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

  return (
    <footer className="statusbar">
      <button
        type="button"
        className="statusbar__host"
        data-drift={drift === "hostBehind" || drift === "clientBehind" ? "" : undefined}
        title={hostTitle}
        onClick={() => onOpen({ kind: "settings", section: "workspace" })}
      >
        {hostLabel}
      </button>
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
          {editing ? "Done" : actions.length === 0 ? "Add a quick action" : "Edit"}
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
