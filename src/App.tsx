import { useCallback, useEffect, useState } from "react";

import { AgentAvatar } from "./avatars/AgentAvatar";
import { AgentEditor } from "./components/AgentEditor";
import { AgentMenu, type MenuTarget } from "./components/AgentMenu";
import { Artifacts } from "./components/Artifacts";
import { Cafeteria } from "./components/Cafeteria";
import { Calendar } from "./components/Calendar";
import { ChannelView } from "./components/ChannelView";
import { ForYou } from "./components/ForYou";
import { GroupEditor } from "./components/GroupEditor";
import { HostUpdateNotice } from "./components/HostUpdates";
import { Inspector } from "./components/Inspector";
import { MobileNavigation } from "./components/MobileNavigation";
import { Search } from "./components/Search";
import { asSection, type Section, SettingsDialog } from "./components/SettingsDialog";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { announcementFor } from "./lib/announce";
import { applyAppearance, watchSystemSurface } from "./lib/appearance";
import { api, notifyOperator, onRevealRequest } from "./lib/ipc";
import { bindingFor } from "./lib/keybinds";
import { FEED_COALESCE_MS, presenceOf, samePresence } from "./lib/menubar";
import { away, burst, markQuiet, quiet, shouldNotify } from "./lib/notify";
import { useRuntime } from "./lib/runtime";
import { useLiveAgents, useStore } from "./lib/store";
import { attached } from "./lib/transport";
import {
  type AgentCard,
  errorMessage,
  type Group,
  type Overlay,
  type QuickPlace,
  type UiEvent,
} from "./lib/types";
import { useReportView } from "./lib/view";
import { followViewport } from "./lib/viewport";

export default function App() {
  useEffect(followViewport, []);
  const agents = useLiveAgents();
  const selected = useStore((s) => s.selected);
  const settings = useStore((s) => s.settings);
  const banner = useStore((s) => s.banner);
  const setBanner = useStore((s) => s.setBanner);
  const handoff = useStore((s) => s.handoff);
  const setHandoff = useStore((s) => s.setHandoff);
  const refreshAgents = useStore((s) => s.refreshAgents);
  const select = useStore((s) => s.select);
  const loadChannel = useStore((s) => s.loadChannel);
  const groups = useStore((s) => s.groups);
  const railGroup = useStore((s) => s.railGroup);
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
      onScreen:
        event.type === "decisionReminder"
          ? state.forYou
          : said.channel === null || said.channel === state.selected,
      quiet: quiet(),
    });
    if (!warranted || burst(said.key)) return;

    void notifyOperator(said.title, said.body);
  }, []);

  const [editing, setEditing] = useState<AgentCard | "new" | null>(null);
  const [editingGroup, setEditingGroup] = useState<Group | "new" | null>(null);
  const [menu, setMenu] = useState<MenuTarget | null>(null);
  const forYou = useStore((state) => state.forYou);
  const showForYou = useStore((state) => state.showForYou);
  const artifactsOpen = useStore((state) => state.artifactsOpen);
  const showArtifacts = useStore((state) => state.showArtifacts);
  const [showSettings, setShowSettings] = useState<Section | true | null>(null);
  const [searching, setSearching] = useState(false);
  const [showCafeteria, setShowCafeteria] = useState(false);
  const [showCalendar, setShowCalendar] = useState(false);
  // The pane inside whichever settings dialog is open, reported by the dialog.
  const [openSection, setOpenSection] = useState<string | null>(null);
  const [mobilePane, setMobilePane] = useState<"agents" | "conversation" | "details">(
    "conversation",
  );
  const openConversation = useCallback(() => setMobilePane("conversation"), []);
  const openDetails = useCallback(() => setMobilePane("details"), []);

  // Search and notifications can select a channel without going through the rail.
  useEffect(() => {
    if (selected) setMobilePane("conversation");
  }, [selected]);

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

  // The appearance, written to the root element. Done here rather than where
  // it is chosen so a reload draws it before the first paint of anything else,
  // and re-run when the OS changes its mind, which only matters while the
  // surface is set to follow it.
  const { uiScale, surface, grays, attention, contrast, typeface, reading, readingSize } = prefs;
  useEffect(() => {
    const look = { uiScale, surface, grays, attention, contrast, typeface, reading, readingSize };
    applyAppearance(look);
    return watchSystemSurface(() => applyAppearance(look));
  }, [uiScale, surface, grays, attention, contrast, typeface, reading, readingSize]);

  // Nothing interrupts the operator for the first few seconds. A routine whose
  // slot passed while the app was closed is overdue and fires on the first
  // tick, which is correct, but launching after a weekend away should not
  // announce a weekend of schedule at once. All of it is on screen immediately
  // either way; only the interruption waits. Before the subscription below, so
  // nothing it delivers is announced ahead of the quiet.
  useEffect(markQuiet, []);
  const ready = useRuntime(announce);

  // The menu bar follows the window. The tray process holds no workspace of
  // its own, so its icon is drawn from this window's store, coalesced and only
  // when it would draw differently, and its panel is told which host this
  // window is attached to and becomes a second client of it. A refusal is
  // said rather than swallowed: a report the tray turned away once left the
  // icon idle whatever the crew was doing, and nothing on screen said so.
  useEffect(() => {
    const host = attached();
    if (!host) return;
    const refused = (what: string) => (error: unknown) =>
      console.warn(`The menu bar refused ${what}: ${errorMessage(error)}`);
    void api.reportHost(host).catch(refused("the workspace this window is showing"));

    let last = presenceOf(useStore.getState());
    let timer: ReturnType<typeof setTimeout> | null = null;
    const report = () => {
      timer = null;
      const next = presenceOf(useStore.getState());
      if (samePresence(next, last)) return;
      last = next;
      void api.reportPresence(next).catch(refused("what this window is showing"));
    };
    void api.reportPresence(last).catch(refused("what this window is showing"));
    const unsubscribe = useStore.subscribe(() => {
      if (timer === null) timer = setTimeout(report, FEED_COALESCE_MS);
    });
    return () => {
      unsubscribe();
      if (timer !== null) clearTimeout(timer);
    };
  }, []);

  // The menu bar panel, asking for the window. The window is already up by the
  // time this lands; the only thing left is where it lands, and the newest
  // window of a channel is the right one: a request the panel offered is the
  // last thing in the channel that raised it.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let canceled = false;

    void (async () => {
      const stop = await onRevealRequest((target) => {
        if (target.kind === "forYou") useStore.getState().showForYou(true);
        else void select(target.id);
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

  const openAgent = selected ? agents.find((a) => a.id === selected) : undefined;

  // Where a quick action goes: the same places the rail and the palette open.
  const openPlace = (place: QuickPlace) => {
    switch (place.kind) {
      case "channel":
        void select(place.agentId);
        setMobilePane("conversation");
        break;
      case "calendar":
        setShowCalendar(true);
        break;
      case "forYou":
        showForYou(true);
        break;
      case "settings":
        setShowSettings(asSection(place.section) ?? true);
        break;
      case "crewSettings": {
        const group = groups.find((g) => g.id === place.groupId);
        if (group) setEditingGroup(group);
        break;
      }
    }
  };
  // A message from a quick action is the operator's own, sent from the channel
  // it goes to, which is then on screen for the answer.
  const sendFromBar = async (agentId: string, text: string) => {
    await select(agentId);
    setMobilePane("conversation");
    await api.sendMessage(agentId, text);
  };

  // What is on screen, most specific first: a dialog covers the desk, and the
  // desk covers the channel.
  const overlay: Overlay | null = showSettings
    ? "settings"
    : editingGroup
      ? "crewSettings"
      : editing
        ? "agentEditor"
        : searching
          ? "search"
          : showCafeteria
            ? "cafeteria"
            : showCalendar
              ? "calendar"
              : artifactsOpen
                ? "artifacts"
                : forYou
                  ? "forYou"
                  : null;
  useReportView({
    agentId: selected ?? null,
    overlay,
    section: overlay === "settings" || overlay === "crewSettings" ? openSection : null,
    groupId:
      overlay === "crewSettings" && editingGroup !== "new" ? (editingGroup?.id ?? null) : null,
  });
  const currentGroup = groups.find((group) => group.id === (openAgent?.groupId ?? railGroup));
  // Read the same group-over-workspace provider and key choices as the backend.
  const needsKey =
    ready &&
    settings !== null &&
    (currentGroup?.inference.provider ?? settings.provider) === "compatible" &&
    !currentGroup?.apiKeySet &&
    !settings.apiKeySet;

  return (
    <div className="app" data-mobile-pane={openAgent ? mobilePane : "agents"}>
      <MobileNavigation
        onChats={() => setMobilePane("agents")}
        onSearch={() => setSearching(true)}
        onSettings={() => setShowSettings(true)}
      />
      <Sidebar
        onOpenChannel={openConversation}
        onEditAgent={(agent) => setEditing(agent)}
        onOpenCafeteria={() => setShowCafeteria(true)}
        onOpenCalendar={() => setShowCalendar(true)}
        onOpenArtifacts={() => showArtifacts({ id: null })}
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

        <HostUpdateNotice onReview={() => setShowSettings("workspace")} />

        {handoff && (
          <div className="banner" role="status">
            <span>
              A sign-in is waiting in the tab that just opened. If none did,{" "}
              <a href={handoff} target="_blank" rel="noopener noreferrer">
                open it here
              </a>
              .
            </span>
            <button type="button" className="btn" onClick={() => setHandoff(null)}>
              Dismiss
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
              <AgentAvatar avatar="orb" color="#5a7d99" size="lg" seed="empty-state" />
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
        ) : selected === null ? (
          // Nothing open. Reached by going inside a crew the open channel was
          // not in, and by deleting the last agent that had one: both are the
          // operator ending up somewhere with no conversation attached, and
          // picking one for them would put an agent's history on screen as a
          // side effect of a click that was about something else.
          <div className="empty" style={{ margin: "auto" }}>
            <p className="empty__body">Pick someone in the rail to open their channel.</p>
          </div>
        ) : (
          <ChannelView
            channel={selected}
            onOpenMenu={(agent, at) => setMenu({ agent, ...at })}
            onBack={() => setMobilePane("agents")}
            onDetails={openDetails}
          />
        )}

        {ready && <StatusBar onOpen={openPlace} onMessage={sendFromBar} />}
      </main>

      {ready && agents.length > 0 && (
        <Inspector
          agent={openAgent}
          onEditProfile={(agent) => setEditing(agent)}
          onOpenActions={(agent, at) => setMenu({ agent, ...at })}
          reveal={mobilePane === "details"}
          onReveal={openDetails}
          onHide={openConversation}
        />
      )}

      {/* Outside `main` and after the panes, so nothing that scrolls can clip it
          and no view change can unmount it. Before the dialogs, because a dialog
          is modal and the one thing that should cover this. */}
      {ready && forYou && <ForYou onClose={() => showForYou(false)} />}

      {menu && (
        <AgentMenu
          target={menu}
          groups={groups}
          onClose={() => setMenu(null)}
          onEditProfile={(agent) => setEditing(agent)}
          onTogglePin={(agent) => void onAgent(() => api.setAgentPinned(agent.id, !agent.pinned))}
          // Through the store rather than the API: a move lands the agent at
          // the end of a group it is not in yet, which is a placement only the
          // roster the rail is drawn from can work out.
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
          // Into the compost. The rail drops the row on the next roster read,
          // and the pane is left where it was: what the agent said is still in
          // every channel it said it in, and a view that emptied itself would
          // read as the transcript having gone too.
          onDelete={(agent) => void onAgent(() => api.deleteAgent(agent.id))}
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
          onSection={setOpenSection}
        />
      )}
      {showCafeteria && <Cafeteria onClose={() => setShowCafeteria(false)} />}
      {showCalendar && <Calendar onClose={() => setShowCalendar(false)} />}
      {artifactsOpen && <Artifacts onClose={() => showArtifacts(null)} />}
      {showSettings && (
        <SettingsDialog
          onClose={() => setShowSettings(null)}
          section={showSettings === true ? undefined : showSettings}
          onSection={setOpenSection}
        />
      )}
      {searching && (
        <Search
          onOpenChannel={openConversation}
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
