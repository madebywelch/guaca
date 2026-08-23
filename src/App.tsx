import { useCallback, useEffect, useState } from "react";

import { AgentAvatar } from "./avatars/AgentAvatar";
import { AgentEditor } from "./components/AgentEditor";
import { AgentMenu, type MenuTarget } from "./components/AgentMenu";
import { Cafeteria } from "./components/Cafeteria";
import { ChannelView } from "./components/ChannelView";
import { GroupEditor } from "./components/GroupEditor";
import { Inspector } from "./components/Inspector";
import { Search } from "./components/Search";
import { type Section, SettingsDialog } from "./components/SettingsDialog";
import { Sidebar } from "./components/Sidebar";
import { announcementFor } from "./lib/announce";
import { applyAppearance, watchSystemSurface } from "./lib/appearance";
import { api, notifyOperator, onRevealRequest, onRuntimeEvent } from "./lib/ipc";
import { bindingFor } from "./lib/keybinds";
import { away, burst, markQuiet, quiet, shouldNotify } from "./lib/notify";
import { ACTIVITY_CHANNEL, useLiveAgents, useStore } from "./lib/store";
import { type AgentCard, errorMessage, type Group, type UiEvent } from "./lib/types";

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
  const prefs = useStore((s) => s.prefs);

  /**
   * Raises an operating system notification, when one is warranted.
   *
   * Reads the store at the moment of the event rather than closing over it. The
   * subscription below is made once, on purpose, and a preference or a
   * selection that has changed since then is the one that has to apply; a
   * dependency on either would tear the event listener down and rebuild it
   * every time the operator clicked a different agent.
   */
  const announce = useCallback((event: UiEvent) => {
    const state = useStore.getState();
    const said = announcementFor(
      event,
      (id) => state.agents.find((agent) => agent.id === id)?.name ?? "An agent",
    );
    if (!said) return;

    const warranted = shouldNotify(said.kind, state.prefs.notify, {
      away: away(),
      // An announcement about no channel in particular is never held back for
      // being about the wrong one.
      onScreen: said.channel === null || said.channel === state.selected,
      quiet: quiet(),
    });
    if (!warranted || burst(said.key)) return;

    void notifyOperator(said.title, said.body);
  }, []);

  const [editing, setEditing] = useState<AgentCard | "new" | null>(null);
  const [editingGroup, setEditingGroup] = useState<Group | "new" | null>(null);
  const [menu, setMenu] = useState<MenuTarget | null>(null);
  const [showSettings, setShowSettings] = useState<Section | true | null>(null);
  const [searching, setSearching] = useState(false);
  const [ready, setReady] = useState(false);
  const [showCafeteria, setShowCafeteria] = useState(false);

  // The three shortcuts that work wherever the operator is, matched against
  // the same table the Shortcuts pane draws from, so a key listed there is a key
  // that works. Everything else in that table belongs to the surface it acts
  // on: see `lib/keybinds`.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const binding = bindingFor(event);
      if (!binding) return;
      event.preventDefault();

      if (binding.id === "search") setSearching(true);
      if (binding.id === "settings") setShowSettings(true);
      if (binding.id === "shortcuts") setShowSettings("shortcuts");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // The scale and the surface, written to the root element. Done here rather
  // than where they are chosen so a reload draws them before the first paint of
  // anything else, and re-run when the OS changes its mind, which only matters
  // while the surface is set to follow it.
  useEffect(() => {
    applyAppearance(prefs.uiScale, prefs.surface);
    return watchSystemSurface(() => applyAppearance(prefs.uiScale, prefs.surface));
  }, [prefs.uiScale, prefs.surface]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    // Subscribing is async, so a teardown can arrive before it resolves. Without
    // this flag the listener leaks: StrictMode mounts twice in development, the
    // first cleanup finds `unlisten` still undefined, and every stream delta is
    // then applied by two listeners. That renders as text interleaved with
    // itself, which looks like a model bug rather than a subscription bug.
    let canceled = false;

    void (async () => {
      // Subscribe before the first read so nothing that happens during startup
      // is missed.
      const stop = await onRuntimeEvent((event) => {
        applyEvent(event);
        announce(event);
      });
      if (canceled) {
        stop();
        return;
      }
      unlisten = stop;

      // Nothing interrupts the operator for the first few seconds. A routine
      // whose slot passed while the app was closed is overdue and fires on the
      // first tick, which is correct, but launching after a weekend away should
      // not announce a weekend of schedule at once. All of it is on screen
      // immediately either way; only the interruption waits.
      markQuiet();

      try {
        await bootstrap();
      } catch (error) {
        setBanner({ tone: "error", text: errorMessage(error) });
      } finally {
        if (!canceled) setReady(true);
      }
    })();

    return () => {
      canceled = true;
      unlisten?.();
    };
  }, [announce, applyEvent, bootstrap, setBanner]);

  // A row in the menu bar, clicked. The window is already up by the time this
  // lands; the only thing left is which channel it lands in, and the newest
  // window of it is the right one: a request the strip offered is the last
  // thing in the channel that raised it.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let canceled = false;

    void (async () => {
      const stop = await onRevealRequest((agent) => {
        void select(agent);
      });
      if (canceled) {
        stop();
        return;
      }
      unlisten = stop;
    })();

    return () => {
      canceled = true;
      unlisten?.();
    };
  }, [select]);

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

  // A key is only missing if a key is what pays. An operator on a subscription
  // has nothing to paste, so this asked them forever for something the Provider
  // pane does not offer them, and the one banner the app uses to say "you are
  // not set up yet" stopped meaning anything.
  const needsKey =
    ready && settings !== null && settings.provider === "compatible" && !settings.apiKeySet;
  const openAgent =
    selected && selected !== ACTIVITY_CHANNEL ? agents.find((a) => a.id === selected) : undefined;

  return (
    <div className="app">
      <Sidebar
        onEditAgent={(agent) => setEditing(agent)}
        onOpenCafeteria={() => setShowCafeteria(true)}
        onEditGroup={(group) => setEditingGroup(group)}
        onOpenSettings={() => setShowSettings(true)}
        onOpenSearch={() => setSearching(true)}
        onNewAgent={() => setEditing("new")}
        onNewGroup={() => setEditingGroup("new")}
        onOpenMenu={(agent, at) => setMenu({ agent, ...at })}
      />

      <main>
        {needsKey && (
          <div className="banner">
            <span>Add an API key before your agents can reply.</span>
            {/* Onto the pane that holds the key, rather than onto the first one
                with the key two sections away. */}
            <button type="button" className="btn" onClick={() => setShowSettings("provider")}>
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
              other. Hire a few who are already set up, or write one from scratch.
            </p>
            <div style={{ display: "flex", gap: "0.5rem", justifyContent: "center" }}>
              <button
                type="button"
                className="btn btn--primary"
                onClick={() => setShowCafeteria(true)}
              >
                Open the cafeteria
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
      {showCafeteria && <Cafeteria onClose={() => setShowCafeteria(false)} />}
      {showSettings && (
        <SettingsDialog
          onClose={() => setShowSettings(null)}
          section={showSettings === true ? undefined : showSettings}
        />
      )}
      {searching && (
        <Search
          onClose={() => setSearching(false)}
          onEditAgent={(agent) => setEditing(agent)}
          onEditGroup={(group) => setEditingGroup(group)}
          onNewAgent={() => setEditing("new")}
          onNewGroup={() => setEditingGroup("new")}
          onOpenCafeteria={() => setShowCafeteria(true)}
          onOpenSettings={() => setShowSettings(true)}
        />
      )}
    </div>
  );
}
