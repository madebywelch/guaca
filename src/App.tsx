import { useEffect, useState } from "react";

import { AgentAvatar } from "./avatars/AgentAvatar";
import { AgentEditor } from "./components/AgentEditor";
import { AgentMenu, type MenuTarget } from "./components/AgentMenu";
import { ChannelView } from "./components/ChannelView";
import { GroupEditor } from "./components/GroupEditor";
import { Inspector } from "./components/Inspector";
import { Search } from "./components/Search";
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
  const loadChannel = useStore((s) => s.loadChannel);
  const groups = useStore((s) => s.groups);
  const nudgeAgent = useStore((s) => s.nudgeAgent);
  const dropAgent = useStore((s) => s.dropAgent);

  const [editing, setEditing] = useState<AgentCard | "new" | null>(null);
  const [editingGroup, setEditingGroup] = useState<Group | "new" | null>(null);
  const [menu, setMenu] = useState<MenuTarget | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [searching, setSearching] = useState(false);
  const [ready, setReady] = useState(false);
  const [seeding, setSeeding] = useState(false);

  // Both modifiers, on every platform. The app is one window with one find
  // shortcut, and an operator who learned it on a laptop should not have to
  // learn it again on a desktop.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() === "k" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        setSearching(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

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

  /**
   * Runs something on one agent and re-reads the roster.
   *
   * Both menu actions change a card the rail is drawing, and the runtime emits
   * `agentsChanged` for each, but waiting for the round trip means the row does
   * not move until the event lands. Refreshing here as well makes the click
   * feel like it did something.
   */
  const onAgent = async (run: () => Promise<unknown>) => {
    try {
      await run();
      await refreshAgents();
    } catch (error) {
      setBanner({ tone: "error", text: errorMessage(error) });
    }
  };

  const needsKey = ready && settings !== null && !settings.apiKeySet;
  const openAgent =
    selected && selected !== ACTIVITY_CHANNEL ? agents.find((a) => a.id === selected) : undefined;

  return (
    <div className="app">
      <Sidebar
        onNewAgent={() => setEditing("new")}
        onEditAgent={(agent) => setEditing(agent)}
        onNewGroup={() => setEditingGroup("new")}
        onEditGroup={(group) => setEditingGroup(group)}
        onOpenSettings={() => setShowSettings(true)}
        onOpenSearch={() => setSearching(true)}
        onOpenMenu={(agent, at) => setMenu({ agent, ...at })}
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
            onOpenMenu={(agent, at) => setMenu({ agent, ...at })}
          />
        )}
      </main>

      {ready && agents.length > 0 && (
        <Inspector agent={openAgent} onEditProfile={(agent) => setEditing(agent)} />
      )}

      {menu && (
        <AgentMenu
          target={menu}
          groups={groups}
          onClose={() => setMenu(null)}
          onEditProfile={(agent) => setEditing(agent)}
          onTogglePin={(agent) => void onAgent(() => api.setAgentPinned(agent.id, !agent.pinned))}
          // Both go through the store: it holds the roster and the activity the
          // rail's order is computed from, so it is the only place that can say
          // which row is above this one right now.
          onNudge={(agent, delta) => void onAgent(() => nudgeAgent(agent.id, delta))}
          onMoveToGroup={(agent, group) =>
            void onAgent(() => dropAgent(agent.id, { kind: "group", id: group.id }))
          }
          onTogglePause={(agent) =>
            void onAgent(() => api.setAgentPaused(agent.id, agent.lifecycle !== "paused"))
          }
          onDuplicate={(agent) =>
            void onAgent(async () => {
              const copy = await api.duplicateAgent(agent.id);
              await refreshAgents();
              await select(copy.id);
            })
          }
          // The runtime announces the clear and the store re-reads whatever is
          // open, but only once the event has been round-tripped. Reading here
          // as well is what makes the click look like it did something.
          onClearHistory={(agent) =>
            void onAgent(async () => {
              await api.clearChannel(agent.id);
              await loadChannel(agent.id);
            })
          }
        />
      )}

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
      {searching && (
        <Search
          onClose={() => setSearching(false)}
          onEditAgent={(agent) => setEditing(agent)}
          onEditGroup={(group) => setEditingGroup(group)}
          onNewAgent={() => setEditing("new")}
          onNewGroup={() => setEditingGroup("new")}
          onOpenSettings={() => setShowSettings(true)}
        />
      )}
    </div>
  );
}
