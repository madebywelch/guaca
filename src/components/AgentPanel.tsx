import { useEffect, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { prefersReducedMotion } from "../lib/motion";
import { runsOn } from "../lib/profile";
import { useStore } from "../lib/store";
import type { Activity, AgentCard, RoutineId } from "../lib/types";
import { BrowserScreen } from "./BrowserScreen";
import { ComputerScreen } from "./ComputerScreen";
import { Memory } from "./Memory";
import { Notebook } from "./Notebook";
import { RoutineDetail } from "./RoutineDetail";
import { RoutineList } from "./RoutineList";
import { WorkingNotes } from "./WorkingNotes";

/** Where on the page a tab of the agent's sends the reader. */
export type AgentSection = "now" | "routines" | "memory" | "notebook" | "skills";

interface Props {
  agent: AgentCard;
  /** The section a tab asked for, scrolled to as it is asked. */
  section: AgentSection;
  onEditProfile: (agent: AgentCard) => void;
}

/** What an agent's activity is, said in the panel's own words. */
const activityWords: Record<Exclude<Activity["state"], "idle">, string> = {
  thinking: "working",
  queued: "waiting its turn",
  awaitingApproval: "waiting on you",
  paused: "paused",
};

/**
 * Who the panel belongs to, at the top of it.
 *
 * The panel used to open on "Routines", under a gear and two arrows, and
 * nothing in it said whose routines. The sections below are all things one
 * agent carries, so the agent is the heading: its face, its name, the crew it
 * is in and what answers for it, which is the one fact about its setup that
 * is read constantly and was two clicks away in a dialog. Edit is here rather
 * than a gear in the corner because the gear said nothing about what it
 * opened, and sat beside the arrow that closes the panel.
 *
 * The model is drawn lighter when it is inherited: a group's default is a fact
 * about the group, and an operator reading the profile should be able to tell
 * which agents chose for themselves without opening each one.
 */
function Profile({ agent, onEdit }: { agent: AgentCard; onEdit: () => void }) {
  const group = useStore((state) => state.groups.find((entry) => entry.id === agent.groupId));
  const settings = useStore((state) => state.settings);
  const activity = useStore((state) => state.activity[agent.id]);
  const runs = runsOn(agent, group, settings);
  return (
    <div className="profile">
      {/* The panel opens on a page about one agent, so it opens the way the
          transcript does: what this is, then whose, then what it is set on. */}
      <p className="profile__kicker">The agent</p>
      <p className="profile__name" title={agent.name}>
        {agent.name}
      </p>
      <p className="profile__line" title={runs.crew}>
        {runs.crew}
      </p>
      <div className="profile__plate">
        <AgentAvatar
          avatar={agent.avatar}
          color={agent.color}
          activity={activity}
          lifecycle={agent.lifecycle}
          seed={agent.id}
          size="lg"
        />
      </div>
      {activity && activity.state !== "idle" && (
        <p className="profile__state" data-state={activity.state}>
          <span className="profile__dot" aria-hidden="true" />
          {activityWords[activity.state]}
        </p>
      )}
      {/* What it is set on, as a page's front matter is set: a table of the
          facts that are read constantly and were two clicks away.

          It used to hold the browser and the computer as well, and those were
          the same two things the places below say, twice, one above the other,
          because a fact and a control about one place are one row and not two.
          A place is granted, watched and taken back, so it lives where its
          switch is; what is left here is what has no control at all. */}
      <dl className="facts">
        <div className="fact">
          <dt>Model</dt>
          {/* Lighter when inherited: a group's default is a fact about the
              group, and an operator reading the profile should be able to tell
              which agents chose for themselves without opening each one. */}
          <dd title={runs.model} data-inherited={runs.inherited || undefined}>
            {runs.model}
          </dd>
        </div>
      </dl>
      {agent.skills.length > 0 && (
        <p className="profile__skills" title={agent.skills.join(", ")}>
          {agent.skills.join(", ")}
        </p>
      )}
      <div className="profile__actions">
        <button
          type="button"
          className="btn btn--small btn--ghost"
          aria-label={`Edit ${agent.name}'s profile`}
          onClick={onEdit}
        >
          Edit
        </button>
      </div>
    </div>
  );
}

/**
 * What an agent has, beside what it says: the page every tab but Conversation
 * opens, one column, scrolled to the section the tab names.
 *
 * One page rather than a page per tab, because its order is its design. A
 * screen is a picture of something happening right now, so the two of them
 * lead; then the schedule it keeps, which is the one section nothing else on
 * screen says; then the working notes and the memory, the two halves of what
 * an agent carries between conversations, read against each other; and the
 * notebook last, because it is what the agent reads only when it needs it.
 * Splitting them across tabs would make each one a page with one thing on it
 * and lose what they say beside each other.
 *
 * The agent's name, look and instructions are deliberately not editable here.
 * Those are edited rarely and all at once, which is what a dialog is for; the
 * profile at the top is its summary, and its one button opens it.
 */
export function AgentPanel({ agent, section, onEditProfile }: Props) {
  const [routine, setRoutine] = useState<RoutineId | "new" | null>(null);

  // Switching agent comes back to the top of the page. A routine id belongs
  // to one agent, and leaving it open would show another agent's schedule
  // under this agent's name until it failed to load.
  useEffect(() => {
    setRoutine(null);
  }, [agent.id]);

  // A fired routine in the transcript, clicked. Cleared as it is taken, so the
  // same chip can be clicked again after navigating away.
  const asked = useStore((state) => state.openingRoutine);
  const taken = useStore((state) => state.routineOpened);
  useEffect(() => {
    if (asked === null) return;
    setRoutine(asked);
    taken();
  }, [asked, taken]);

  useEffect(() => {
    if (routine !== null) return;
    document
      .getElementById(`agent-${section}`)
      ?.scrollIntoView?.({ block: "start", behavior: prefersReducedMotion() ? "auto" : "smooth" });
  }, [section, routine]);

  return (
    <aside className="agent-panel" aria-label={`${agent.name}: details`}>
      {/* Hidden rather than unmounted while a routine is open. The screen
          holds a live connection to the agent's desktop, and rebuilding it
          every time somebody looked at a schedule would drop and reconnect it.

          The key is here rather than on each section, and it is the whole
          mechanism for switching agent: every section below holds one agent's
          thing, a desktop connection, a browser, a list read over IPC, and
          none of them may be left under the next agent's name. One key on what
          they share says that once. Saying it once per section collided:
          React indexes the outgoing children by key to decide what to delete,
          so a duplicate overwrote the entry the screen would have been deleted
          through, and the previous agent's screen stayed in the document with
          its polling still running. */}
      <div className="agent-panel__level" hidden={routine !== null} key={agent.id}>
        <div id="agent-skills" className="agent-panel__anchor">
          <Profile agent={agent} onEdit={() => onEditProfile(agent)} />
        </div>
        <div id="agent-now" className="agent-panel__anchor">
          <BrowserScreen agent={agent} />
          <ComputerScreen agent={agent} />
        </div>
        <div id="agent-routines" className="agent-panel__anchor">
          <RoutineList agentId={agent.id} onOpen={setRoutine} />
        </div>
        <div id="agent-memory" className="agent-panel__anchor">
          <WorkingNotes agentId={agent.id} />
          <Memory agentId={agent.id} />
        </div>
        <div id="agent-notebook" className="agent-panel__anchor">
          <Notebook agentId={agent.id} />
        </div>
      </div>

      {routine !== null && (
        <RoutineDetail
          key={`${agent.id}:${routine}`}
          agentId={agent.id}
          routineId={routine}
          // A routine can come back renamed, retimed, switched off or gone.
          // The list behind this reads itself again on the event every one of
          // those emits, so there is nothing to bump here.
          onBack={() => setRoutine(null)}
        />
      )}
    </aside>
  );
}
