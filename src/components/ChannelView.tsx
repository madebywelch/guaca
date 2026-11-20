import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { dismissing, rememberDismissOpener } from "../lib/dismiss";
import { waitingQuestion } from "../lib/field";
import { focusWriting } from "../lib/focus";
import { useFollowBottom } from "../lib/follow";
import { waitingLine } from "../lib/handoff";
import { useShown } from "../lib/held";
import { api } from "../lib/ipc";
import { modelName, runsOn } from "../lib/profile";
import { thoughtNow } from "../lib/reasoning";
import { colleaguesOn, runningNote } from "../lib/running";
import { type ChannelKey, useAgentLookup, useStore } from "../lib/store";
import { clockLabel, elapsed, useNow } from "../lib/time";
import {
  callInFlight,
  type LiveCall,
  type Step,
  tallyLabel,
  tallyTrail,
  trailStep,
} from "../lib/trail";
import { type Row, rowStandsFor, toPeer, transcriptRows, withResults } from "../lib/transcript";
import {
  type Activity,
  type AgentCard,
  type AgentId,
  type Envelope,
  type Group,
  plainText,
  type RunId,
  type Settings,
} from "../lib/types";
import { AgentPanel, type AgentSection } from "./AgentPanel";
import { Composer } from "./Composer";
import { Answering } from "./HtmlArtifact";
import { Icon } from "./Icon";
import { MessageItem, ResultItem, StreamingMessage, WhenRow } from "./MessageItem";
import { PairThread } from "./PairThread";
import { TrailRow } from "./Trail";
import { TurnWork } from "./TurnWork";
import { HandoffRow, RefusedRow, WritingRow } from "./WireRow";

export type AgentView = "conversation" | AgentSection;

export const VIEWS: { view: AgentView; label: string }[] = [
  { view: "now", label: "Now" },
  { view: "conversation", label: "Conversation" },
  { view: "memory", label: "Memory" },
  { view: "notebook", label: "Notebook" },
  { view: "routines", label: "Routines" },
  { view: "profile", label: "Profile" },
];

interface Props {
  channel: ChannelKey;
  onOpenMenu: (agent: AgentCard, at: { x: number; y: number }) => void;
  view?: AgentView;
  onView?: (view: AgentView) => void;
  onQuestions?: () => boolean;
  focusRequest?: { id: string } | null;
  onEditProfile?: (agent: AgentCard) => void;
}

const FLASH_MS = 1800;

const WAITED_MS = 1000;

export function ChannelView({
  channel,
  onOpenMenu,
  view = "conversation",
  onView,
  onQuestions,
  focusRequest,
  onEditProfile,
}: Props) {
  const lookups = useAgentLookup();
  const messages = useStore((s) => s.messages[channel]);
  const { activity } = useShown();
  const showFailure = useStore((s) => s.showFailure);
  const focused = useStore((s) => s.focused);
  const clearFocus = useStore((s) => s.clearFocus);
  const select = useStore((s) => s.select);

  const [reading, setReading] = useState<AgentId | null>(null);
  const [unfolded, setUnfolded] = useState<ReadonlySet<string>>(() => new Set());

  const { ref: scrollRef, node: transcript, follow, pin } = useFollowBottom();

  const agent = lookups.byId(channel);
  const crew = useStore((s) => s.groups.find((group) => group.id === agent?.groupId)?.name);
  const group = useStore((s) => s.groups.find((entry) => entry.id === agent?.groupId));
  const settings = useStore((s) => s.settings);
  const shownNow = useShown();
  const asking = shownNow.pending.some((one) => one.agentId === agent?.id);
  const question = waitingQuestion(shownNow.pending.filter((one) => one.agentId === agent?.id));
  const answeringId = agent?.id;
  const answeringName = agent?.name;
  const answering = useMemo(
    () => (answeringId && answeringName ? { id: answeringId, name: answeringName } : null),
    [answeringId, answeringName],
  );

  const requestedFocus = useRef<Props["focusRequest"]>(null);
  useLayoutEffect(() => {
    if (
      !focusRequest ||
      requestedFocus.current === focusRequest ||
      view !== "conversation" ||
      !transcript ||
      !messages ||
      dismissing()
    )
      return;
    const card = [...(transcript()?.querySelectorAll<HTMLElement>("[data-question]") ?? [])].find(
      (one) => one.dataset.question === focusRequest.id,
    );
    const answer = card?.querySelector<HTMLElement>(":is(button, input):not(:disabled)");
    if (answer) {
      requestedFocus.current = focusRequest;
      answer.focus();
    } else {
      focusWriting();
    }
  }, [focusRequest, transcript, messages, view]);

  useLayoutEffect(follow, [follow, messages]);

  useLayoutEffect(() => {
    setReading(null);
  }, [channel]);

  useLayoutEffect(pin, [pin, channel, reading]);

  const rows = useMemo(() => transcriptRows(messages ?? [], lookups), [messages, lookups]);
  const runs = useStore((s) =>
    s.runningKnown ? s.running.map((one) => one.runId).join(" ") : null,
  );
  const shown = useMemo(
    () => (runs === null ? rows : withResults(rows, new Set(runs.split(" ") as RunId[]))),
    [rows, runs],
  );
  const asked = useMemo((): Asked | null => {
    let last: Row | undefined;
    for (const row of rows) if (row.kind === "peers") last = row;
    return last?.kind === "peers"
      ? {
          runId: last.runId,
          ids: last.peers.flatMap((one) => (one.agentId ? [one.agentId] : [])),
          names: last.peers.filter((one) => one.sent > 0).map((one) => one.peer.name),
          waiting: last.peers
            .filter((one) => one.sent > 0 && one.received === 0)
            .map((one) => one.peer.name),
        }
      : null;
  }, [rows]);

  useEffect(() => {
    if (!focused || messages === undefined) return;
    const row = transcript()?.querySelector<HTMLElement>(`[data-message~="${focused}"]`);
    if (!row) return;
    row.scrollIntoView({ block: "center" });
    const timer = window.setTimeout(clearFocus, FLASH_MS);
    return () => window.clearTimeout(timer);
  }, [focused, messages, clearFocus, transcript]);

  const paused = agent?.lifecycle === "paused";

  if (reading && agent) {
    return (
      <PairThread
        self={agent.id}
        peer={reading}
        lookups={lookups}
        onClose={() => setReading(null)}
      />
    );
  }

  return (
    <section className="pane">
      <header className="pane__header">
        {agent ? (
          <>
            <span className="pane__face">
              <AgentAvatar
                avatar={agent.avatar}
                color={agent.color}
                activity={activity[agent.id]}
                lifecycle={agent.lifecycle}
                seed={agent.id}
                size="md"
              />
            </span>
            <div className="pane__identity">
              <h1 className="pane__title">{agent.name}</h1>
              <p className="pane__about">
                {about(crew ?? "Independent agent", {
                  paused,
                  asking,
                  working: activity[agent.id]?.state === "thinking",
                })}
              </p>
            </div>
            {paused && <span className="pane__flag">Paused</span>}
            <button
              type="button"
              className="pane__more"
              aria-label={`Actions for ${agent.name}`}
              title={`Actions for ${agent.name}`}
              onClick={(event) => {
                const box = event.currentTarget.getBoundingClientRect();
                onOpenMenu(agent, { x: box.left, y: box.bottom + 4 });
              }}
            >
              <Icon name="more" />
            </button>
          </>
        ) : (
          <h1 className="pane__title">Channel unavailable</h1>
        )}
      </header>

      {agent && (
        <div className="agent-tabs">
          {onView && <AgentTabs agent={agent} view={view} onView={onView} />}
          <p className="pane__meta">{answeredBy(agent, group, settings)}</p>
        </div>
      )}

      {agent && view !== "conversation" && view !== "now" && (
        <div className="pane__scroll">
          <AgentPanel agent={agent} section={view} onEditProfile={(one) => onEditProfile?.(one)} />
        </div>
      )}

      <div className="pane__body" hidden={view !== "conversation" && view !== "now"}>
        {agent && view === "now" && (
          <div className="pane__scroll">
            <AgentPanel agent={agent} section="now" onEditProfile={(one) => onEditProfile?.(one)} />
          </div>
        )}
        <div
          className="pane__scroll"
          ref={scrollRef}
          hidden={view === "now"}
          role="log"
          aria-live="off"
          aria-label={`Conversation with ${agent?.name ?? "this agent"}`}
        >
          <Answering.Provider value={answering}>
            {messages === undefined ? (
              <p className="hint" style={{ padding: "1rem 1.15rem" }}>
                Loading…
              </p>
            ) : messages.length === 0 ? (
              <div className="empty">
                <p className="empty__body">No messages with {agent?.name ?? "this agent"} yet.</p>
              </div>
            ) : (
              shown.map((row) => {
                const stands = rowStandsFor(row);
                const toggle = (key: string) =>
                  setUnfolded((held) => {
                    const next = new Set(held);
                    if (!next.delete(key)) next.add(key);
                    return next;
                  });
                return (
                  <div
                    key={row.key}
                    data-message={stands.join(" ")}
                    data-found={focused && stands.includes(focused) ? "true" : undefined}
                    onClickCapture={(event) => {
                      if (
                        !agent ||
                        (row.kind !== "peers" && row.kind !== "result") ||
                        !(event.target instanceof Element)
                      )
                        return;
                      const button = event.target.closest<HTMLButtonElement>(
                        "button.handoff__name, button.footnote__send",
                      );
                      if (button) rememberDismissOpener(button);
                    }}
                  >
                    {row.kind === "peers" ? (
                      <HandoffRow
                        peers={row.peers}
                        runId={row.runId}
                        channel={channel}
                        rowKey={row.key}
                        open={unfolded.has(row.key)}
                        onToggle={() => toggle(row.key)}
                        onOpen={setReading}
                      />
                    ) : row.kind === "result" ? (
                      <ResultItem
                        row={row}
                        lookups={lookups}
                        channel={channel}
                        open={unfolded}
                        onToggle={toggle}
                        onOpen={setReading}
                      />
                    ) : row.kind === "refused" ? (
                      <RefusedRow peer={row.peer} at={row.at} body={row.body} reason={row.reason} />
                    ) : row.kind === "when" ? (
                      <WhenRow at={row.at} />
                    ) : (
                      <MessageItem
                        message={row.message}
                        lookups={lookups}
                        continued={row.continued}
                      />
                    )}
                  </div>
                );
              })
            )}

            {agent && <TurnWork key={agent.id} agent={agent.id} follow={follow} />}
            <LiveStreams channel={channel} lookups={lookups} follow={follow} asked={asked} />
          </Answering.Provider>
        </div>

        <Arrivals key={`arrivals:${channel}`} messages={messages} lookups={lookups} />
        {agent && (
          <TurnFooter
            key={agent.id}
            agent={agent}
            state={activity[agent.id]}
            now={view === "now"}
            asked={asked}
          />
        )}
        <Composer
          key={`composer-${channel}`}
          draftKey={channel}
          recipient={agent ? { id: agent.id, name: agent.name } : undefined}
          onRecipientChange={(id) => void select(id)}
          onQuestions={onQuestions}
          focusOnOpen={!focusRequest}
          waiting={
            agent && question
              ? {
                  asker: agent.name,
                  answer: () =>
                    document
                      .querySelector<HTMLElement>(
                        `[data-question="${question.id}"] :is(button, input):not(:disabled)`,
                      )
                      ?.focus(),
                }
              : undefined
          }
          placeholder={`Write to ${agent?.name ?? "agent"}…`}
          group={agent?.groupId ?? null}
          disabled={!agent || agent.lifecycle === "terminated"}
          disabledReason="This agent has been deleted."
          onSend={async (text, files, key) => {
            if (!agent) return;
            pin();
            try {
              return await api.sendMessage(agent.id, text, files, key);
            } catch (error) {
              showFailure(error);
              throw error;
            }
          }}
        />
      </div>
    </section>
  );
}

interface Asked {
  runId: RunId;
  ids: string[];
  names: string[];
  waiting: string[];
}

function TurnFooter({
  agent,
  state,
  now,
  asked,
}: {
  agent: AgentCard;
  state: Activity | undefined;
  now: boolean;
  asked: Asked | null;
}) {
  const inline = useStore((s) => s.prefs.showReasoning);
  const [showing, setShowing] = useState<Showing | null>(null);
  const panel = useId();

  const behind = useStore((s) => {
    const owned = s.running.find((one) => one.agentId === agent.id);
    return (
      state?.state === "queued" &&
      owned !== undefined &&
      Object.entries(s.activeRun).some(([id, run]) => id !== agent.id && run === owned.runId)
    );
  });
  const working = state?.state === "thinking" || (state?.state === "queued" && !behind);

  useEffect(() => {
    if (!working) setShowing(null);
  }, [working]);

  if (!working) return now ? null : <WaitingLine agent={agent} asked={asked} />;

  return (
    <div className="turn">
      {!inline && showing === "working" && <ThoughtPanel id={panel} agent={agent.id} />}
      {!inline && showing === "steps" && <StepsPanel id={panel} agent={agent.id} />}
      <WorkingNote
        agent={agent}
        now={now}
        panel={panel}
        showing={showing}
        onToggle={(which) => setShowing((held) => (held === which ? null : which))}
      />
    </div>
  );
}

type Showing = "working" | "steps";

function ThoughtPanel({ id, agent }: { id: string; agent: AgentId }) {
  const held = useStore((s) => s.reasoning[agent]);
  const { ref, follow } = useFollowBottom();

  useLayoutEffect(follow, [follow, held]);

  return (
    <div className="thought" id={id} ref={ref}>
      {held ? (
        <pre className="thought__text">{held}</pre>
      ) : (
        <p className="thought__empty">Nothing published yet on this turn.</p>
      )}
    </div>
  );
}

function liveSteps(calls: LiveCall[] | undefined): Step[] {
  return (calls ?? []).flatMap((call, index) =>
    call.done ? [trailStep(call.done, `${call.callId}-${index}`)] : [],
  );
}

function StepsPanel({ id, agent }: { id: string; agent: AgentId }) {
  const calls = useStore((s) => s.trail[agent]);
  const steps = liveSteps(calls);
  const { ref, follow } = useFollowBottom();

  useLayoutEffect(follow, [follow, steps.length]);

  return (
    <div className="steps" id={id} ref={ref}>
      <TrailRow steps={steps} />
    </div>
  );
}

function WorkingNote({
  agent,
  now: onNow,
  panel,
  showing,
  onToggle,
}: {
  agent: AgentCard;
  now: boolean;
  panel: string;
  showing: Showing | null;
  onToggle: (which: Showing) => void;
}) {
  const inline = useStore((s) => s.prefs.showReasoning);
  const held = useStore((s) => (s.prefs.showReasoning ? undefined : s.reasoning[agent.id]));
  const calls = useStore((s) => s.trail[agent.id]);
  const run = useStore((s) => s.activeRun[agent.id]);
  const owned = useStore((s) => s.running.find((one) => one.runId === run));
  const since = owned?.startedAt;
  const noted = owned?.agentId === agent.id ? owned.note : null;
  const now = useNow(1000);

  const thought = thoughtNow(held);
  const tally = useMemo(() => tallyTrail(liveSteps(calls)), [calls]);
  const waiting =
    !inline && calls?.find((call) => call.done === null && now - call.startedAt >= WAITED_MS);
  const saying = Boolean(waiting) || Boolean(thought.heading || thought.line);

  const line = waiting ? (
    <>
      <span className="working__now">{callInFlight(waiting.name, waiting.arguments)}</span>
      <span className="working__for">{elapsed(now, waiting.startedAt)}</span>
    </>
  ) : saying ? (
    <>
      {thought.heading && <span className="working__topic">{thought.heading}</span>}
      {thought.line && <span className="working__thought">{thought.line}</span>}
    </>
  ) : noted ? (
    <span className="working__thought">{noted}</span>
  ) : (
    `${agent.name} is working`
  );

  return (
    <div
      className="working"
      role={saying || onNow ? undefined : "status"}
      data-thinking={saying || Boolean(noted) || undefined}
    >
      {!onNow && (
        <>
          <AgentAvatar
            avatar={agent.avatar}
            color={agent.color}
            size="md"
            seed={agent.id}
            activity={{ state: "thinking" }}
            title={`${agent.name} is working`}
          />
          {held ? (
            <button
              type="button"
              className="working__label working__label--open"
              aria-expanded={showing === "working"}
              aria-controls={showing === "working" ? panel : undefined}
              aria-label={`${showing === "working" ? "Hide" : "Show"} what ${agent.name} is working through`}
              onClick={() => onToggle("working")}
            >
              {line}
            </button>
          ) : (
            <span className="working__label">{line}</span>
          )}
        </>
      )}
      <div className="working__end">
        {!inline &&
          tally.spent.map((credential) => (
            <span className="trail__spent" key={credential}>
              {credential}
            </span>
          ))}
        {!inline && tally.done > 0 && (
          <button
            type="button"
            className="working__steps"
            data-failed={tally.failed > 0 || undefined}
            aria-expanded={showing === "steps"}
            aria-controls={showing === "steps" ? panel : undefined}
            title={`What ${agent.name} has done on this turn`}
            onClick={() => onToggle("steps")}
          >
            <span className="working__caret" aria-hidden="true">
              <Icon name={showing === "steps" ? "chevron-down" : "chevron-right"} size={16} />
            </span>
            {tallyLabel(tally)}
          </button>
        )}
        {run && since !== undefined && (
          <span className="working__for" title={`Started at ${clockLabel(since)}`}>
            {elapsed(now, since)}
          </span>
        )}
        {run && <StopConversation run={run} />}
      </div>
    </div>
  );
}

function StopConversation({ run }: { run: RunId }) {
  const showFailure = useStore((s) => s.showFailure);
  const [stopping, setStopping] = useState(false);
  return (
    <button
      type="button"
      className="btn btn--quiet btn--compact"
      disabled={stopping}
      title="Stop this conversation and everything it set off"
      onClick={async () => {
        setStopping(true);
        try {
          await api.stopRun(run);
        } catch (error) {
          showFailure(error);
        } finally {
          setStopping(false);
        }
      }}
    >
      Stop
    </button>
  );
}

function WaitingLine({ agent, asked }: { agent: AgentCard; asked: Asked | null }) {
  const owned = useStore((s) => s.running.find((one) => one.agentId === agent.id));
  const agents = useStore((s) => s.agents);
  const activeRun = useStore((s) => s.activeRun);
  const { activity: everyone, sending } = useShown();
  const activity = everyone[agent.id];
  const now = useNow(1000);
  if (!owned) return null;

  const mine = asked?.runId === owned.runId ? asked : null;
  const waiting =
    mine?.waiting ??
    colleaguesOn(owned, activeRun, (id) => agents.find((one) => one.id === id)?.name);
  const line =
    waitingLine(mine?.names ?? [], waiting) ??
    runningNote(owned, activity, [], sending.includes(agent.id));

  return (
    <div className="turn">
      <div className="working" role="status" data-thinking>
        <AgentAvatar avatar={agent.avatar} color={agent.color} size="md" seed={agent.id} />
        <span className="working__label">
          <span className="working__thought">{line}</span>
        </span>
        <div className="working__end">
          <span className="working__for" title={`Started at ${clockLabel(owned.startedAt)}`}>
            {elapsed(now, owned.startedAt)}
          </span>
          <StopConversation run={owned.runId} />
        </div>
      </div>
    </div>
  );
}

function LiveStreams({
  channel,
  lookups,
  follow,
  asked,
}: {
  channel: ChannelKey;
  lookups: ReturnType<typeof useAgentLookup>;
  follow: () => void;
  asked: Asked | null;
}) {
  const streams = useStore((s) => s.streams);
  const activeRun = useStore((s) => s.activeRun);
  const live = Object.entries(streams).filter(([, buffer]) => buffer?.channelId === channel);

  useLayoutEffect(follow);

  return (
    <div aria-live="off">
      {live.map(([id, buffer]) => {
        if (!buffer) return null;

        if (buffer.to.kind === "agent") {
          const carded =
            asked?.ids.includes(buffer.agentId) && activeRun[buffer.agentId] === asked.runId;
          return carded ? null : (
            <WritingRow key={id} peer={toPeer(lookups.byId(buffer.agentId), buffer.agentId)} />
          );
        }
        return buffer.text.length > 0 ? (
          <StreamingMessage key={id} agent={lookups.byId(buffer.agentId)} text={buffer.text} />
        ) : null;
      })}
    </div>
  );
}

function Arrivals({
  messages,
  lookups,
}: {
  messages: Envelope[] | undefined;
  lookups: ReturnType<typeof useAgentLookup>;
}) {
  const [said, setSaid] = useState("");
  const seen = useRef<string | null>(null);

  useEffect(() => {
    const newest = messages?.[messages.length - 1];
    if (!newest) return;
    if (seen.current === null || seen.current === newest.id) {
      seen.current = newest.id;
      return;
    }
    seen.current = newest.id;
    setSaid(announcement(newest, lookups.byId));
  }, [messages, lookups]);

  return (
    <p className="offscreen" role="status" aria-live="polite" aria-atomic="true">
      {said}
    </p>
  );
}

function announcement(message: Envelope, byId: ReturnType<typeof useAgentLookup>["byId"]): string {
  if (message.from.kind !== "agent") return "";
  const who = byId(message.from.id)?.name ?? "An agent";

  const asking = message.parts.find((part) => part.type === "approval");
  if (asking) return `${who} is asking you: ${asking.summary}`;

  if (message.to.kind !== "human") return "";

  const body = plainText(message);
  if (!body) return "";
  return body.length > 300 ? `${who}: ${body.slice(0, 300)}…` : `${who}: ${body}`;
}

function about(
  where: string,
  { paused, asking, working }: { paused: boolean; asking: boolean; working: boolean },
): string {
  if (paused) return `${where}, paused`;
  if (asking) return `${where}, waiting on you`;
  if (working) return `${where}, working`;
  return where;
}

function AgentTabs({
  agent,
  view,
  onView,
}: {
  agent: AgentCard;
  view: AgentView;
  onView: (view: AgentView) => void;
}) {
  const counts = useTabCounts(agent.id);
  return (
    <nav className="agent-tabs__list" aria-label={`${agent.name}'s page`}>
      {VIEWS.map((tab) => {
        const count = counts[tab.view];
        return (
          <button
            key={tab.view}
            type="button"
            className="agent-tabs__tab"
            aria-current={tab.view === view ? "page" : undefined}
            onClick={() => onView(tab.view)}
          >
            {tab.label}
            {count ? (
              <span className="agent-tabs__count" aria-hidden="true">
                {count}
              </span>
            ) : null}
          </button>
        );
      })}
    </nav>
  );
}

function useTabCounts(agentId: AgentId): Partial<Record<AgentView, number>> {
  const routines = useStore((s) => s.routineVersion[agentId]);
  const notebook = useStore((s) => s.notebookVersion[agentId]);
  const [counts, setCounts] = useState<Partial<Record<AgentView, number>>>({});
  useEffect(() => {
    let current = true;
    setCounts({});
    void Promise.allSettled([
      Promise.resolve().then(() => api.agentRoutines(agentId)),
      Promise.resolve().then(() => api.agentNotebook(agentId)),
    ]).then(([listed, pages]) => {
      if (!current) return;
      setCounts({
        routines: listed.status === "fulfilled" ? listed.value?.length : undefined,
        notebook: pages.status === "fulfilled" ? pages.value?.length : undefined,
      });
    });
    return () => {
      current = false;
    };
  }, [agentId, routines, notebook]);
  return counts;
}

function answeredBy(agent: AgentCard, group: Group | undefined, settings: Settings | null): string {
  const runs = runsOn(agent, group, settings);
  if (runs.model === "No model set" || runs.model === "Claude subscription") return runs.model;
  return modelName(runs.model);
}
