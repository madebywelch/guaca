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
import { dismissing } from "./lib/dismiss";
import { unreachableHint } from "./lib/host";
import { api, onRevealRequest } from "./lib/ipc";
import { bindingFor } from "./lib/keybinds";
import { FEED_COALESCE_MS, presenceOf, presenceReporter, samePresence } from "./lib/menubar";
import { catchUp, deliver, land } from "./lib/notify";
import { readFirstTask } from "./lib/onboarding";
import { useRuntime } from "./lib/runtime";
import { asSection, type Section } from "./lib/sections";
import { type Banner, useLiveAgents, useStore } from "./lib/store";
import { attached, desktop } from "./lib/transport";
import {
  type AgentCard,
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
  /** Set while a click from outside the window is opening an agent. */
  const revealing = useRef(false);

  /**
   * Where the window lands when the operator asks to be taken somewhere from
   * outside it: the menu bar panel, or a click on a notification. Read from
   * the store at the moment it is asked, so it can be made once. An agent
   * opens on the conversation, working or not: what was clicked was a message.
   */
  const reveal = useCallback((target: Reveal) => {
    const state = useStore.getState();
    if (target.kind === "forYou") {
      state.showForYou(true);
      return;
    }
    revealing.current = state.selected !== target.id;
    setAgentView("conversation");
    void state.select(target.id);
  }, []);

  /**
   * A delivery the Host offered every window. Whether it is worth a
   * notification was decided there; this window only claims it, and shows it
   * if the claim won. Made once, for the same reason as `reveal`: the
   * subscription that calls it is.
   */
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
  // The pane inside whichever settings dialog is open, reported by the dialog.
  const [openSection, setOpenSection] = useState<string | null>(null);
  const leaveChannel = useStore((state) => state.leaveChannel);
  const focused = useStore((state) => state.focused);

  // Opening someone lands on what they are doing when they are doing
  // something, and on the conversation otherwise. Decided once, as the agent
  // is opened: the found message's mark is cleared a moment after it is shown,
  // and a landing that reran on that would take the reader off the message
  // to the agent's screen.
  useEffect(() => {
    if (!selected) return;
    if (revealing.current) {
      revealing.current = false;
      setAgentView("conversation");
      return;
    }
    const state = useStore.getState();
    const busy = state.activity[selected]?.state === "thinking";
    setAgentView(busy && !state.focused ? "now" : "conversation");
  }, [selected]);

  // A message asked for by search is in the conversation, including one in the
  // agent already open on another tab.
  useEffect(() => {
    if (focused) setAgentView("conversation");
  }, [focused]);

  // A fired routine clicked in the transcript. The panel that opens it is
  // only drawn on a tab other than the conversation, so the request goes there
  // and the panel takes it as it mounts.
  const openingRoutine = useStore((state) => state.openingRoutine);
  useEffect(() => {
    if (openingRoutine !== null) setAgentView("routines");
  }, [openingRoutine]);

  // The three shortcuts that work wherever the operator is, matched against
  // the same table the Shortcuts pane draws from, so a key listed there is a key
  // that works. Everything else in that table belongs to the surface it acts
  // on: see `lib/keybinds`.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const binding = bindingFor(event);
      if (!binding) return;
      event.preventDefault();

      if (binding.id === "field") {
        // Not from under a dialog or the Groups sheet: the box is behind it,
        // and focus moved there would be typing into something out of sight.
        if (dismissing()) return;
        // The field on the opening screen, or the open conversation's box:
        // whichever box writes to someone from where the operator is.
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

  // The appearance, written to the root element. Done here rather than where
  // it is chosen so a reload draws it before the first paint of anything else,
  // and re-run when the OS changes its mind, which only matters while the
  // surface is set to follow it.
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

  // What was offered while this page was not connected: once the first read
  // lands, and again each time the link comes back up, since an offer made
  // while the socket was down reached nobody.
  useEffect(() => {
    if (!ready || link !== "up") return;
    void catchUp((target) => land(target, reveal));
  }, [ready, link, reveal]);

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

  // The menu bar panel, asking for the window. The window is already up by the
  // time this lands; the only thing left is where it lands, and the newest
  // window of a channel is the right one: a request the panel offered is the
  // last thing in the channel that raised it.
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
      showFailure(error);
    }
  };

  const openAgent = selected ? agents.find((a) => a.id === selected) : undefined;

  // Where a quick action goes: the same places the rail and the palette open.
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
  // A message from a quick action is the operator's own, sent from the channel
  // it goes to, which is then on screen for the answer.
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
  // Read the same group-over-workspace provider and key choices as the backend.
  // On Today no group is in front of the operator, so the banner speaks for
  // every group with someone in it, and only when none of them can reply.
  const unpaid = (group: Group | undefined) =>
    settings !== null &&
    (group?.inference.provider ?? settings.provider) === "compatible" &&
    !group?.apiKeySet &&
    !settings.apiKeySet;
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

  // `data-link` is the one attribute the whole window reads: whether what it
  // draws is current. `styles.css` is where the transcript loses its weight.
  return (
    <div className="app" data-link={link === "down" ? "down" : undefined}>
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
        <Timeline onOpenAgent={(id) => void select(id)} onOpenPile={() => setShowGroups(true)} />
      )}

      <main>
        {needsKey && !firstRun && (
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
          // Nothing open: the opening screen. Also where going inside a group
          // the open channel was not in, and deleting the agent that was open,
          // both leave the operator, rather than on somebody picked for them.
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

/**
 * The one line the window keeps for something that went wrong.
 *
 * Dismiss is the only control a message can always have, and for most of them
 * it is the only one that would do anything: a refused write, a duplicate name
 * and a missing key are all fixed by changing something elsewhere, in a place
 * the banner cannot open. The host being gone is the exception, and it is the
 * one failure where the app knows more than it says: nothing answered, and the
 * two things that help are asking again and looking at which host this is.
 * Both are actions, and leaving the operator with only the button that removes
 * the diagnosis is what this exists to stop.
 */
function FailureBanner({
  banner,
  onSettings,
}: {
  banner: Banner;
  onSettings: (section: Section | true) => void;
}) {
  const setBanner = useStore((state) => state.setBanner);
  const [retrying, setRetrying] = useState(false);

  /**
   * Asks the host everything again, which is what came back missing.
   *
   * The socket redials on its own, and that is why this is not a reload: a host
   * that has come back is a host whose next read succeeds, and the whole page
   * reloading would lose the draft in the composer for news the operator can
   * get without it.
   */
  const retry = async () => {
    setRetrying(true);
    try {
      await useStore.getState().resynchronize();
      setBanner(null);
    } catch {
      // The banner is already saying nothing answered. Saying it again in a
      // sentence the operator has just read is noise on the one line that
      // matters.
    } finally {
      setRetrying(false);
    }
  };

  return (
    <div className={banner.tone === "error" ? "banner banner--error" : "banner"}>
      <span className="banner__text">
        <span>{banner.text}</span>
        {/* Under the sentence rather than instead of it: the sentence is what
            happened, and this is the part the app could not know from the
            failure alone. */}
        {banner.fix === "host" && <span className="banner__hint">{unreachableHint()}</span>}
      </span>
      <span className="banner__actions">
        {banner.fix === "host" && (
          <button type="button" className="btn" disabled={retrying} onClick={() => void retry()}>
            {retrying ? "Trying…" : "Retry"}
          </button>
        )}
        {/* Where a person changes hosts, and only a desktop can: a browser is
            pointed at its host by the address it was opened with. */}
        {banner.fix === "host" && desktop && (
          <button type="button" className="btn btn--ghost" onClick={() => onSettings("workspace")}>
            Check the host
          </button>
        )}
        <button type="button" className="btn btn--ghost" onClick={() => setBanner(null)}>
          Dismiss
        </button>
      </span>
    </div>
  );
}
