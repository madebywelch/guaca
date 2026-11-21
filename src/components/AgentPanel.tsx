import { useEffect, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { useShown } from "../lib/held";
import { api } from "../lib/ipc";
import { runsOn } from "../lib/profile";
import { useStore } from "../lib/store";
import { useNow, whenLabel } from "../lib/time";
import { callInFlight } from "../lib/trail";
import {
  type Activity,
  type AgentCard,
  type AgentId,
  type Envelope,
  type MessageId,
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

export type AgentSection = "now" | "routines" | "memory" | "notebook" | "profile";

interface Props {
  agent: AgentCard;
  section: AgentSection;
  onEditProfile: (agent: AgentCard) => void;
}

const activityWords: Record<Exclude<Activity["state"], "idle">, string> = {
  thinking: "working",
  queued: "waiting its turn",
  awaitingApproval: "waiting on you",
  paused: "paused",
};

function Profile({ agent, onEdit }: { agent: AgentCard; onEdit: () => void }) {
  const group = useStore((state) => state.groups.find((entry) => entry.id === agent.groupId));
  const settings = useStore((state) => state.settings);
  const activity = useShown().activity[agent.id];
  const runs = runsOn(agent, group, settings);
  return (
    <div className="profile">
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
      <dl className="facts">
        <div className="fact">
          <dt>Model</dt>
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
          className="btn btn--compact btn--quiet"
          aria-label={`Edit ${agent.name}'s profile`}
          onClick={onEdit}
        >
          Edit
        </button>
      </div>
    </div>
  );
}

function latest(messages: Envelope[] | undefined, test: (one: Envelope) => boolean) {
  if (!messages) return undefined;
  for (let i = messages.length - 1; i >= 0; i--) {
    const one = messages[i] as Envelope;
    if (test(one)) return one;
  }
  return undefined;
}

const firstLine = (one: Envelope) => plainText(one).trim().split("\n")[0] ?? "";

function ended(one: Envelope) {
  if (one.from.kind !== "system") return null;
  for (const part of one.parts) {
    if (part.type === "notice" && (part.kind === "upstreamError" || part.kind === "interrupted")) {
      return part;
    }
  }
  return null;
}

function RetryTurn({ agent, cause }: { agent: AgentId; cause: MessageId }) {
  const showFailure = useStore((state) => state.showFailure);
  const [sent, setSent] = useState(false);
  return (
    <button
      type="button"
      className="btn doing__retry"
      disabled={sent}
      onClick={() => {
        setSent(true);
        api.retryTurn(agent, cause).catch((error: unknown) => {
          showFailure(error);
          setSent(false);
        });
      }}
    >
      {sent ? "Sent again" : "Try again"}
    </button>
  );
}

const doingWords: Record<"thinking" | "queued", string> = {
  thinking: "Working on a request from",
  queued: "Waiting its turn, with a request from",
};

function Doing({ agent }: { agent: AgentCard }) {
  const activity = useShown().activity[agent.id];
  const trail = useStore((state) => state.trail[agent.id]);
  const messages = useStore((state) => state.messages[agent.id]);
  const run = useStore((state) => state.activeRun[agent.id]);
  const agents = useStore((state) => state.agents);
  const now = useNow(30_000);
  const own = (one: Participant) => one.kind === "agent" && one.id === agent.id;
  const state = activity?.state ?? "idle";

  if (state === "idle" || state === "paused") {
    const said = latest(messages, (one) => own(one.from) || (own(one.to) && ended(one) !== null));
    const end = said && ended(said);
    return (
      <section className="doing" aria-labelledby={`doing-${agent.id}`}>
        <h3 id={`doing-${agent.id}`} className="doing__title">
          {state === "paused" ? "Paused, so nothing is running" : "Nothing is running"}
        </h3>
        <p className="doing__line">
          {end ? (
            <>
              <span className="doing__when">
                Last run {end.kind === "upstreamError" ? "failed" : "was cut off"}{" "}
                {whenLabel(said.createdAt, now)}
              </span>{" "}
              <span className="doing__text">{end.text}</span>
              {said.cause && <RetryTurn agent={said.channelId} cause={said.cause} />}
            </>
          ) : said ? (
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

export function AgentPanel({ agent, section, onEditProfile }: Props) {
  const [routine, setRoutine] = useState<RoutineId | "new" | null>(null);

  useEffect(() => {
    setRoutine(null);
  }, [agent.id]);

  const asked = useStore((state) => state.openingRoutine);
  const taken = useStore((state) => state.routineOpened);
  useEffect(() => {
    if (asked === null) return;
    setRoutine(asked);
    taken();
  }, [asked, taken]);

  const detail = section === "routines" && routine !== null;
  useEffect(() => {
    if (detail) return;
    document
      .getElementById(`agent-${section}`)
      ?.scrollIntoView?.({ block: "start", behavior: "auto" });
  }, [section, detail]);

  return (
    <aside className="agent-panel" aria-label={`${agent.name}: details`}>
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
          onBack={() => setRoutine(null)}
        />
      )}
    </aside>
  );
}
