import { useEffect, useState } from "react";

import { AgentAvatar } from "./avatars/AgentAvatar";
import { AgentEditor } from "./components/AgentEditor";
import { ChannelView } from "./components/ChannelView";
import { GroupEditor } from "./components/GroupEditor";
import { SettingsDialog } from "./components/SettingsDialog";
import { Sidebar } from "./components/Sidebar";
import { api, onRuntimeEvent } from "./lib/ipc";
import { ACTIVITY_CHANNEL, useLiveAgents, useStore } from "./lib/store";
import { type AgentCard, type AgentDraft, errorMessage, type Group } from "./lib/types";

/**
 * A crew that demonstrates the point of the app on first run: one agent whose
 * job is to delegate, and three with distinct jobs for it to delegate to.
 */
const STARTER_CREW: AgentDraft[] = [
  {
    name: "Manager",
    avatar: "avocado",
    color: "#c7d96b",
    model: "",
    skills: ["delegation", "planning"],
    systemPrompt:
      "You coordinate the other agents. Prefer delegating to doing the work yourself: find who is best suited with `directory`, then message them. Keep your replies to two sentences.",
  },
  {
    name: "Researcher",
    avatar: "owl",
    color: "#6aa9d9",
    model: "",
    skills: ["research", "fact checking"],
    systemPrompt:
      "You gather and verify information. State what you are confident about and what you are not. Never invent a citation.",
  },
  {
    name: "Critic",
    avatar: "chilli",
    color: "#e2674a",
    model: "",
    skills: ["review", "finding holes"],
    systemPrompt:
      "You find the weakest part of any plan or claim put to you, say what it is plainly, and suggest the smallest change that fixes it. Be direct, never rude.",
  },
  {
    name: "Scribe",
    avatar: "star",
    color: "#9b8ad4",
    model: "",
    skills: ["summarising", "note taking"],
    systemPrompt:
      "You turn scattered discussion into short, ordered notes. Lead with the decision, then the reasoning. Never pad.",
  },
];

export default function App() {
  const agents = useLiveAgents();
  const selected = useStore((s) => s.selected);
  const settings = useStore((s) => s.settings);
  const banner = useStore((s) => s.banner);
  const setBanner = useStore((s) => s.setBanner);
  const bootstrap = useStore((s) => s.bootstrap);
  const applyEvent = useStore((s) => s.applyEvent);
  const refreshAgents = useStore((s) => s.refreshAgents);
  const select = useStore((s) => s.select);

  const [editing, setEditing] = useState<AgentCard | "new" | null>(null);
  const [editingGroup, setEditingGroup] = useState<Group | "new" | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [ready, setReady] = useState(false);
  const [seeding, setSeeding] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    // Subscribing is async, so a teardown can arrive before it resolves. Without
    // this flag the listener leaks: StrictMode mounts twice in development, the
    // first cleanup finds `unlisten` still undefined, and every stream delta is
    // then applied by two listeners. That renders as text interleaved with
    // itself, which looks like a model bug rather than a subscription bug.
    let cancelled = false;

    void (async () => {
      // Subscribe before the first read so nothing that happens during startup
      // is missed.
      const stop = await onRuntimeEvent(applyEvent);
      if (cancelled) {
        stop();
        return;
      }
      unlisten = stop;

      try {
        await bootstrap();
      } catch (error) {
        setBanner({ tone: "error", text: errorMessage(error) });
      } finally {
        if (!cancelled) setReady(true);
      }
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [applyEvent, bootstrap, setBanner]);

  const addStarterCrew = async () => {
    setSeeding(true);
    try {
      const model = settings?.defaultModel ?? "";
      for (const draft of STARTER_CREW) {
        await api.createAgent({ ...draft, model: draft.model || model });
      }
      await refreshAgents();
      const created = useStore.getState().agents.find((a) => a.name === "Manager");
      if (created) await select(created.id);
    } catch (error) {
      setBanner({ tone: "error", text: errorMessage(error) });
    } finally {
      setSeeding(false);
    }
  };

  const needsKey = ready && settings !== null && !settings.apiKeySet;

  return (
    <div className="app">
      <Sidebar
        onNewAgent={() => setEditing("new")}
        onEditAgent={(agent) => setEditing(agent)}
        onNewGroup={() => setEditingGroup("new")}
        onEditGroup={(group) => setEditingGroup(group)}
        onOpenSettings={() => setShowSettings(true)}
      />

      <main>
        {needsKey && (
          <div className="banner">
            <span>Add an API key before your agents can reply.</span>
            <button type="button" className="btn" onClick={() => setShowSettings(true)}>
              Open settings
            </button>
          </div>
        )}

        {banner && (
          <div className={banner.tone === "error" ? "banner banner--error" : "banner"}>
            <span>{banner.text}</span>
            <button type="button" className="btn btn--ghost" onClick={() => setBanner(null)}>
              Dismiss
            </button>
          </div>
        )}

        {!ready ? (
          <div className="empty" style={{ margin: "auto" }}>
            <p className="empty__body">Starting up…</p>
          </div>
        ) : agents.length === 0 ? (
          <div className="empty" style={{ margin: "auto" }}>
            <span style={{ display: "inline-flex" }}>
              <AgentAvatar avatar="avocado" color="#c7d96b" size="lg" seed="empty-state" />
            </span>
            <h2 className="empty__title">No agents yet</h2>
            <p className="empty__body">
              Agents are the people in this workspace. You talk to them, and they can talk to each
              other. Start with a crew, or build one from scratch.
            </p>
            <div style={{ display: "flex", gap: "0.5rem", justifyContent: "center" }}>
              <button
                type="button"
                className="btn btn--primary"
                disabled={seeding}
                onClick={() => void addStarterCrew()}
              >
                {seeding ? "Adding…" : "Add a starter crew"}
              </button>
              <button type="button" className="btn" onClick={() => setEditing("new")}>
                Create one agent
              </button>
            </div>
          </div>
        ) : (
          <ChannelView
            channel={selected ?? ACTIVITY_CHANNEL}
            onEditAgent={(agent) => setEditing(agent)}
          />
        )}
      </main>

      {editing && (
        <AgentEditor
          agent={editing === "new" ? undefined : editing}
          onClose={() => setEditing(null)}
        />
      )}
      {editingGroup && (
        <GroupEditor
          group={editingGroup === "new" ? undefined : editingGroup}
          onClose={() => setEditingGroup(null)}
        />
      )}
      {showSettings && <SettingsDialog onClose={() => setShowSettings(false)} />}
    </div>
  );
}
