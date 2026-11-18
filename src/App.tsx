import { useCallback, useEffect, useRef, useState } from "react";

import { AgentAvatar } from "./avatars/AgentAvatar";
import { AgentEditor } from "./components/AgentEditor";
import { AgentMenu, type MenuTarget } from "./components/AgentMenu";
import { Artifacts } from "./components/Artifacts";
import { Cafeteria } from "./components/Cafeteria";
import { Calendar } from "./components/Calendar";
import { type AgentView, ChannelView } from "./components/ChannelView";
import { ForYou } from "./components/ForYou";
import { GroupEditor } from "./components/GroupEditor";
import { GroupsSheet } from "./components/GroupsSheet";
import { Home } from "./components/Home";
import { HostUpdateNotice } from "./components/HostUpdates";
import { Onboarding } from "./components/Onboarding";
import { Search } from "./components/Search";
import { SettingsDialog } from "./components/SettingsDialog";
import { StatusBar } from "./components/StatusBar";
import { Timeline } from "./components/Timeline";
import { TopBar } from "./components/TopBar";
import { applyAppearance, watchSystemSurface } from "./lib/appearance";
import { diagnostic } from "./lib/diagnostic";
import { dismissing } from "./lib/dismiss";
import { unreachableHint } from "./lib/host";
import { api, onRevealRequest } from "./lib/ipc";
import { bindingFor } from "./lib/keybinds";
import { FEED_COALESCE_MS, presenceOf, presenceReporter, samePresence } from "./lib/menubar";
import { catchUp, deliver, land } from "./lib/notify";
import { readFirstTask } from "./lib/onboarding";
import { inferenceOf, providerFor } from "./lib/providers";
import { useRuntime } from "./lib/runtime";
import { asSection, type Section } from "./lib/sections";
import { type Banner, useLiveAgents, useStore } from "./lib/store";
import { attached, desktop } from "./lib/transport";
import {
  type AgentCard,
  type AgentId,
  errorMessage,
  type Group,
  type Overlay,
  type QuickPlace,
  type Reveal,
  type UiEvent,
} from "./lib/types";
import { useReportView } from "./lib/view";
import { followViewport } from "./lib/viewport";

const reportPresence = presenceReporter(async (presence: ReturnType<typeof presenceOf>) => {
  try {
    await api.reportPresence(presence);
  } catch (error) {
    console.warn(`The menu bar refused what this window is showing: ${errorMessage(error)}`);
    throw error;
  }
}, samePresence);

export default function App() {
  useEffect(followViewport, []);
  const agents = useLiveAgents();
  const allAgents = useStore((s) => s.agents);
  const [onboardingActive, setOnboardingActive] = useState(() => {
    const task = readFirstTask();
    return !!(task.agentId || task.key);
  });
  const [checkingFirstTask, setCheckingFirstTask] = useState(false);
  const selected = useStore((s) => s.selected);
  const settings = useStore((s) => s.settings);
  const banner = useStore((s) => s.banner);
  const link = useStore((s) => s.link);
  const showFailure = useStore((s) => s.showFailure);
  const handoff = useStore((s) => s.handoff);
  const setHandoff = useStore((s) => s.setHandoff);
  const refreshAgents = useStore((s) => s.refreshAgents);
  const select = useStore((s) => s.select);
  const loadChannel = useStore((s) => s.loadChannel);
  const groups = useStore((s) => s.groups);
  const railGroup = useStore((s) => s.railGroup);
  const dropAgent = useStore((s) => s.dropAgent);
  const prefs = useStore((s) => s.prefs);

  const [agentView, setAgentView] = useState<AgentView>("conversation");
  const revealing = useRef<AgentId | null>(null);

  const reveal = useCallback((target: Reveal) => {
    if (target.kind === "agent") diagnostic("reveal.agent", target.id);
    const state = useStore.getState();
    if (target.kind === "forYou") {
      revealing.current = null;
      state.showForYou(true);
      return;
    }
    revealing.current = target.id;
    setAgentView("conversation");
    void state.select(target.id);
  }, []);

  const onEvent = useCallback(
    (event: UiEvent) => {
      if (event.type !== "delivery") return;
      void deliver(event, (target) => land(target, reveal));
    },
    [reveal],
  );

  const [editing, setEditing] = useState<AgentCard | "new" | null>(null);
  const [editingGroup, setEditingGroup] = useState<Group | "new" | null>(null);
  const [menu, setMenu] = useState<MenuTarget | null>(null);
  const forYou = useStore((state) => state.forYou);
  const showForYou = useStore((state) => state.showForYou);
  const artifactsOpen = useStore((state) => state.artifactsOpen);
  const showArtifacts = useStore((state) => state.showArtifacts);
  const [showSettings, setShowSettings] = useState<Section | true | null>(null);
  const [searching, setSearching] = useState<string | false>(false);
  const [showGroups, setShowGroups] = useState(false);
  const [showCafeteria, setShowCafeteria] = useState(false);
  const [showCalendar, setShowCalendar] = useState(false);
  const [openSection, setOpenSection] = useState<string | null>(null);
  const leaveChannel = useStore((state) => state.leaveChannel);
  const focused = useStore((state) => state.focused);

  useEffect(
    () =>
      useStore.subscribe((state) => {
        if (revealing.current !== null && state.selected !== revealing.current)
          revealing.current = null;
      }),
    [],
  );

  useEffect(() => {
    const revealed = revealing.current;
    revealing.current = null;
    if (!selected) return;
    if (revealed === selected) {
      setAgentView("conversation");
      return;
    }
    const state = useStore.getState();
    const busy = state.activity[selected]?.state === "thinking";
    setAgentView(busy && !state.focused ? "now" : "conversation");
  }, [selected]);

  useEffect(() => {
    if (focused) setAgentView("conversation");
  }, [focused]);

  const openingRoutine = useStore((state) => state.openingRoutine);
  useEffect(() => {
    if (openingRoutine !== null) setAgentView("routines");
  }, [openingRoutine]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const binding = bindingFor(event);
      if (!binding) return;
      event.preventDefault();

      if (binding.id === "field") {
        if (dismissing()) return;
        if (useStore.getState().selected) setAgentView("conversation");
        requestAnimationFrame(() =>
          document.querySelector<HTMLElement>("[data-field]:not([disabled])")?.focus(),
        );
      }
      if (binding.id === "search") setSearching("");
      if (binding.id === "settings") setShowSettings(true);
      if (binding.id === "shortcuts") setShowSettings("shortcuts");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const {
    uiScale,
    surface,
    grays,
    attention,
    contrast,
    temperature,
    typeface,
    reading,
    readingSize,
  } = prefs;
  useEffect(() => {
    const look = {
      uiScale,
      surface,
      grays,
      attention,
      contrast,
      temperature,
      typeface,
      reading,
      readingSize,
    };
    applyAppearance(look);
    return watchSystemSurface(() => applyAppearance(look));
  }, [uiScale, surface, grays, attention, contrast, temperature, typeface, reading, readingSize]);

  const ready = useRuntime(onEvent);

  useEffect(() => {
    if (!ready || link !== "up") return;
    void catchUp((target) => land(target, reveal));
  }, [ready, link, reveal]);

  useEffect(() => {
    const host = attached();
    if (!host) return;
    const refused = (what: string) => (error: unknown) =>
      console.warn(`The menu bar refused ${what}: ${errorMessage(error)}`);
    void api.reportHost(host).catch(refused("the workspace this window is showing"));

    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const report = async () => {
      await reportPresence(presenceOf(useStore.getState()));
      if (!stopped) timer = setTimeout(() => void report(), FEED_COALESCE_MS);
    };
    void report();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let canceled = false;

    void (async () => {
      const stop = await onRevealRequest(reveal);
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
  }, [reveal]);

  const onAgent = async (run: () => Promise<unknown>) => {
    try {
      await run();
      await refreshAgents();
    } catch (error) {
      showFailure(error);
    }
  };

  const openAgent = selected ? agents.find((a) => a.id === selected) : undefined;

  const openPlace = (place: QuickPlace) => {
    switch (place.kind) {
      case "channel":
        void select(place.agentId);
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
  const sendFromBar = async (agentId: string, text: string) => {
    await select(agentId);
    setAgentView("conversation");
    await api.sendMessage(agentId, text);
  };

  const openDialogs: Overlay[] = [
    ...(ready && forYou ? ["forYou" as const] : []),
    ...(editing ? ["agentEditor" as const] : []),
    ...(editingGroup ? ["crewSettings" as const] : []),
    ...(showCafeteria ? ["cafeteria" as const] : []),
    ...(showCalendar ? ["calendar" as const] : []),
    ...(artifactsOpen ? ["artifacts" as const] : []),
    ...(showSettings ? ["settings" as const] : []),
    ...(searching !== false ? ["search" as const] : []),
  ];
  const [dialogOrder, setDialogOrder] = useState<Overlay[]>([]);
  const orderedDialogs = [
    ...dialogOrder.filter((dialog) => openDialogs.includes(dialog)),
    ...openDialogs.filter((dialog) => !dialogOrder.includes(dialog)),
  ];
  if (orderedDialogs.join() !== dialogOrder.join()) setDialogOrder(orderedDialogs);

  const overlay: Overlay | null = orderedDialogs.at(-1) ?? null;
  useReportView({
    agentId: selected ?? null,
    overlay,
    section: overlay === "settings" || overlay === "crewSettings" ? openSection : null,
    groupId:
      overlay === "crewSettings" && editingGroup !== "new" ? (editingGroup?.id ?? null) : null,
  });
  const currentGroup = groups.find((group) => group.id === (openAgent?.groupId ?? railGroup));
  const unpaid = (group: Group | undefined) => {
    if (settings === null) return false;
    const { provider, keyed } = inferenceOf(group, settings);
    const preset = providerFor(group?.inference.baseUrl || settings.baseUrl);
    return provider === "compatible" && !keyed && preset !== undefined && !preset.local;
  };
  const peopled = groups.filter((group) => agents.some((agent) => agent.groupId === group.id));
  const needsKey =
    ready &&
    settings !== null &&
    (currentGroup || peopled.length === 0 ? unpaid(currentGroup) : peopled.every(unpaid));
  const firstRun =
    ready &&
    settings !== null &&
    groups.length === 1 &&
    (onboardingActive ||
      (allAgents.length === 0 &&
        !groups[0]!.apiKeySet &&
        Object.values(groups[0]!.inference).every((value) => value === null)));

  return (
    <div
      className="app"
      data-link={link === "down" ? "down" : undefined}
      data-page={selected === null ? "today" : "agent"}
    >
      <TopBar
        open={openAgent?.name ?? null}
        onHome={leaveChannel}
        onSearch={() => setSearching("")}
        onGroups={() => setShowGroups(true)}
        onSettings={() => setShowSettings(true)}
        onNewAgent={() => setEditing("new")}
        onNewGroup={() => setEditingGroup("new")}
        onOpenCafeteria={() => setShowCafeteria(true)}
      />

      {ready && (
        <Timeline
          onOpenAgent={(id) => void select(id)}
          onOpenPile={() => setShowGroups(true)}
          openAgentId={selected}
        />
      )}

      <main>
        {needsKey && !firstRun && (
          <div className="banner">
            <span>Add an API key before your agents can reply.</span>
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

        {banner && <FailureBanner banner={banner} onSettings={setShowSettings} />}

        {firstRun && (
          <Onboarding
            group={groups[0]!}
            onStarting={() => setOnboardingActive(true)}
            onDone={() => {
              setOnboardingActive(false);
              setCheckingFirstTask(false);
            }}
            onInspect={setCheckingFirstTask}
            onBrowse={() => setShowCafeteria(true)}
            onCreate={() => setEditing("new")}
          />
        )}

        {!ready ? (
          <div className="empty" style={{ margin: "auto" }}>
            <p className="empty__body">Starting up…</p>
          </div>
        ) : firstRun && !checkingFirstTask ? null : agents.length === 0 ? (
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
          <Home
            onSearch={(query) => setSearching(query)}
            onManageGroups={() => setShowGroups(true)}
          />
        ) : (
          <ChannelView
            channel={selected}
            onOpenMenu={(agent, at) => setMenu({ agent, ...at })}
            view={agentView}
            onView={setAgentView}
            onEditProfile={(agent) => setEditing(agent)}
          />
        )}

        {ready && <StatusBar onOpen={openPlace} onMessage={sendFromBar} />}
      </main>

      {menu && (
        <AgentMenu
          target={menu}
          groups={groups}
          provider={
            groups.find((group) => group.id === menu.agent.groupId)?.inference.provider ??
            settings?.provider
          }
          onClose={() => setMenu(null)}
          onEditProfile={(agent) => setEditing(agent)}
          onTogglePin={(agent) => void onAgent(() => api.setAgentPinned(agent.id, !agent.pinned))}
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
          onClearHistory={(agent) =>
            void onAgent(async () => {
              await api.clearChannel(agent.id);
              await loadChannel(agent.id);
            })
          }
          onDelete={(agent) => void onAgent(() => api.deleteAgent(agent.id))}
        />
      )}

      {showGroups && (
        <GroupsSheet
          onClose={() => setShowGroups(false)}
          onEditAgent={(agent) => setEditing(agent)}
          onOpenCafeteria={() => setShowCafeteria(true)}
          onOpenCalendar={() => setShowCalendar(true)}
          onOpenArtifacts={() => showArtifacts({ id: null })}
          onEditGroup={(group) => setEditingGroup(group)}
          onOpenSettings={() => setShowSettings(true)}
          onOpenSearch={() => setSearching("")}
          onNewAgent={() => setEditing("new")}
          onNewGroup={() => setEditingGroup("new")}
          onOpenMenu={(agent, at) => setMenu({ agent, ...at })}
        />
      )}

      {orderedDialogs.map((dialog) => {
        switch (dialog) {
          case "forYou":
            return <ForYou key={dialog} onClose={() => showForYou(false)} />;
          case "agentEditor":
            return (
              editing && (
                <AgentEditor
                  key={dialog}
                  agent={editing === "new" ? undefined : editing}
                  onClose={() => setEditing(null)}
                />
              )
            );
          case "crewSettings":
            return (
              editingGroup && (
                <GroupEditor
                  key={dialog}
                  group={editingGroup === "new" ? undefined : editingGroup}
                  onClose={() => setEditingGroup(null)}
                  onSection={setOpenSection}
                />
              )
            );
          case "cafeteria":
            return <Cafeteria key={dialog} onClose={() => setShowCafeteria(false)} />;
          case "calendar":
            return <Calendar key={dialog} onClose={() => setShowCalendar(false)} />;
          case "artifacts":
            return <Artifacts key={dialog} onClose={() => showArtifacts(null)} />;
          case "settings":
            return (
              showSettings && (
                <SettingsDialog
                  key={dialog}
                  onClose={() => setShowSettings(null)}
                  section={showSettings === true ? undefined : showSettings}
                  onSection={setOpenSection}
                />
              )
            );
          case "search":
            return (
              <Search
                key={dialog}
                initial={searching === false ? "" : searching}
                onClose={() => setSearching(false)}
                onEditAgent={(agent) => setEditing(agent)}
                onEditGroup={(group) => setEditingGroup(group)}
                onNewAgent={() => setEditing("new")}
                onNewGroup={() => setEditingGroup("new")}
                onOpenCafeteria={() => setShowCafeteria(true)}
                onOpenSettings={() => setShowSettings(true)}
              />
            );
          default:
            return null;
        }
      })}
    </div>
  );
}

function FailureBanner({
  banner,
  onSettings,
}: {
  banner: Banner;
  onSettings: (section: Section | true) => void;
}) {
  const setBanner = useStore((state) => state.setBanner);
  const [retrying, setRetrying] = useState(false);

  const retry = async () => {
    setRetrying(true);
    try {
      await useStore.getState().resynchronize();
      setBanner(null);
    } catch {
    } finally {
      setRetrying(false);
    }
  };

  return (
    <div className={banner.tone === "error" ? "banner banner--error" : "banner"}>
      <span className="banner__text">
        <span>{banner.text}</span>
        {banner.fix === "host" && <span className="banner__hint">{unreachableHint()}</span>}
      </span>
      <span className="banner__actions">
        {banner.fix === "host" && (
          <button type="button" className="btn" disabled={retrying} onClick={() => void retry()}>
            {retrying ? "Trying…" : "Retry"}
          </button>
        )}
        {banner.fix === "host" && desktop && (
          <button type="button" className="btn btn--quiet" onClick={() => onSettings("workspace")}>
            Check the host
          </button>
        )}
        <button type="button" className="btn btn--quiet" onClick={() => setBanner(null)}>
          Dismiss
        </button>
      </span>
    </div>
  );
}
