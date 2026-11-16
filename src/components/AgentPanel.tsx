import { useEffect, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { runsOn } from "../lib/profile";
import { useStore } from "../lib/store";
import { useNow, whenLabel } from "../lib/time";
import { callInFlight } from "../lib/trail";
import {
  type Activity,
  type AgentCard,
  type Envelope,
  type Participant,
  plainText,
  type RoutineId,
} from "../lib/types";
import { BrowserScreen } from "./BrowserScreen";
import { ComputerScreen } from "./ComputerScreen";
import { Memory } from "./Memory";
import { Notebook } from "./Notebook";
import { RoutineDetail } from "./RoutineDetail";
import { RoutineList } from "./RoutineList";
import { WorkingNotes } from "./WorkingNotes";

/** Where on the page a tab of the agent's sends the reader. */
export type AgentSection = "now" | "routines" | "memory" | "notebook" | "profile";

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

/** The last message in a channel that `test` holds for. */
function latest(messages: Envelope[] | undefined, test: (one: Envelope) => boolean) {
  if (!messages) return undefined;
  for (let i = messages.length - 1; i >= 0; i--) {
    const one = messages[i] as Envelope;
    if (test(one)) return one;
  }
  return undefined;
}

const firstLine = (one: Envelope) => plainText(one).trim().split("\n")[0] ?? "";

/** How a turn that has a request behind it is said ahead of whose it is. */
const doingWords: Record<"thinking" | "queued", string> = {
  thinking: "Working on a request from",
  queued: "Waiting its turn, with a request from",
};

/**
 * What the agent is doing, at the top of Now: the request it is working on
 * and who it came from, then the steps it has taken on it so far. An agent
 * with nothing running says so, and what it last finished.
 *
 * Read from the conversation and the live trail rather than kept anywhere of
 * its own, so it says what the transcript says and cannot disagree with it.
 */
function Doing({ agent }: { agent: AgentCard }) {
  const activity = useStore((state) => state.activity[agent.id]);
  const trail = useStore((state) => state.trail[agent.id]);
  const messages = useStore((state) => state.messages[agent.id]);
  const run = useStore((state) => state.activeRun[agent.id]);
  const agents = useStore((state) => state.agents);
  const now = useNow(30_000);
  const own = (one: Participant) => one.kind === "agent" && one.id === agent.id;
  const state = activity?.state ?? "idle";

  if (state === "idle" || state === "paused") {
    const said = latest(messages, (one) => own(one.from));
    return (
      <section className="doing" aria-labelledby={`doing-${agent.id}`}>
        <h3 id={`doing-${agent.id}`} className="doing__title">
          {state === "paused" ? "Paused, so nothing is running" : "Nothing is running"}
        </h3>
        <p className="doing__line">
          {said ? (
            <>
              <span className="doing__when">Last finished {whenLabel(said.createdAt, now)}</span>{" "}
              <span className="doing__text">{firstLine(said)}</span>
            </>
          ) : (
            `${agent.name} has not done anything yet.`
          )}
        </p>
      </section>
    );
  }

  // The request that started the run it is on: the first message to it in
  // that run. A later message is something said while it works, not the task.
  const asked =
    run === undefined
      ? undefined
      : messages?.find((one) => one.runId === run && own(one.to) && !own(one.from));
  const from = asked?.from;
  const who =
    from === undefined
      ? "somebody"
      : from.kind === "human"
        ? "you"
        : from.kind === "agent"
          ? (agents.find((one) => one.id === from.id)?.name ?? "another agent")
          : "a routine";
  return (
    <section className="doing" aria-labelledby={`doing-${agent.id}`}>
      <h3 id={`doing-${agent.id}`} className="doing__title">
        {state === "awaitingApproval"
          ? "Waiting for your answer before it goes on"
          : asked
            ? `${doingWords[state]} ${who}`
            : state === "queued"
              ? "Waiting its turn"
              : "Working"}
      </h3>
      {asked && (
        <p className="doing__line">
          <span className="doing__when">{whenLabel(asked.createdAt, now)}</span>{" "}
          <span className="doing__text">{firstLine(asked)}</span>
        </p>
      )}
      <h3 className="doing__title doing__title--steps">Steps</h3>
      {trail?.length ? (
        <ol className="doing__steps">
          {trail.map((call, index) => (
            <li
              // biome-ignore lint/suspicious/noArrayIndexKey: calls append in arrival order; providers may reuse call IDs across rounds.
              key={`${call.callId}-${index}`}
              className="doing__step"
              data-state={call.done ? call.done.outcome.status : "running"}
            >
              {callInFlight(call.name, call.arguments)}
            </li>
          ))}
        </ol>
      ) : (
        <p className="doing__line">Thinking it over, with no steps taken yet.</p>
      )}
    </section>
  );
}

/**
 * What an agent has, beside what it says: one tab of it at a time.
 *
 * Now is what the agent is doing, and the two places it does it in, live.
 * Every store has a tab of its own and is drawn there only: the schedule
 * under Routines, the working notes and the memory under Memory, read against
 * each other, the notebook under Notebook, and what it is set up with under
 * Profile. A tab opens at its top.
 *
 * The agent's name, look and instructions are deliberately not editable here.
 * Those are edited rarely and all at once, which is what a dialog is for; the
 * profile is its summary, and its one button opens it.
 */
export function AgentPanel({ agent, section, onEditProfile }: Props) {
  const [routine, setRoutine] = useState<RoutineId | "new" | null>(null);

  // Switching agent comes back to the list. A routine id belongs to one
  // agent, and leaving it open would show another agent's schedule under this
  // agent's name until it failed to load.
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

  // A tab opens at its top, at once: a glide down from wherever the last tab
  // was left is motion nobody asked for.
  const detail = section === "routines" && routine !== null;
  useEffect(() => {
    if (detail) return;
    document
      .getElementById(`agent-${section}`)
      ?.scrollIntoView?.({ block: "start", behavior: "auto" });
  }, [section, detail]);

  return (
    <aside className="agent-panel" aria-label={`${agent.name}: details`}>
      {/* Hidden rather than unmounted while a routine is open, so the list
          keeps its place behind it.

          The key is the whole mechanism for switching agent: every section
          holds one agent's thing, a desktop connection, a browser, a list read
          over IPC, and none of them may be left under the next agent's name.
          One key on what they share says that once. Saying it once per section
          collided: React indexes the outgoing children by key to decide what to
          delete, so a duplicate overwrote the entry the screen would have been
          deleted through, and the previous agent's screen stayed in the
          document with its polling still running. */}
      <div className="agent-panel__level" hidden={detail} key={agent.id}>
        <div id={`agent-${section}`} className="agent-panel__anchor">
          {section === "now" && (
            <>
              <Doing agent={agent} />
              <BrowserScreen agent={agent} />
              <ComputerScreen agent={agent} />
            </>
          )}
          {section === "routines" && <RoutineList agentId={agent.id} onOpen={setRoutine} />}
          {section === "memory" && (
            <>
              <WorkingNotes agentId={agent.id} />
              <Memory agentId={agent.id} />
            </>
          )}
          {section === "notebook" && <Notebook agentId={agent.id} />}
          {section === "profile" && <Profile agent={agent} onEdit={() => onEditProfile(agent)} />}
        </div>
      </div>

      {detail && (
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
