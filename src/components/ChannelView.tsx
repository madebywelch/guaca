import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { api } from "../lib/ipc";
import { ACTIVITY_CHANNEL, type ChannelKey, useAgentLookup, useStore } from "../lib/store";
import { rowStandsFor, toPeer, transcriptRows } from "../lib/transcript";
import { type Activity, type AgentCard, type AgentId, errorMessage } from "../lib/types";
import { ActivityFlow } from "./ActivityFlow";
import { Composer } from "./Composer";
import { MessageItem, StreamingMessage } from "./MessageItem";
import { PairThread } from "./PairThread";
import { PeerBurstRow, RefusedRow, WritingRow } from "./WireRow";

interface Props {
  channel: ChannelKey;
  onEditAgent: (agent: AgentCard) => void;
}

/** How long a message arrived at from search stays marked. */
const FLASH_MS = 1800;

export function ChannelView({ channel, onEditAgent }: Props) {
  const lookups = useAgentLookup();
  const messages = useStore((s) => s.messages[channel]);
  const activity = useStore((s) => s.activity);
  const setBanner = useStore((s) => s.setBanner);
  const focused = useStore((s) => s.focused);
  const clearFocus = useStore((s) => s.clearFocus);

  const loadChannel = useStore((s) => s.loadChannel);
  const [confirmClear, setConfirmClear] = useState(false);
  /** The peer whose thread is open over this channel, if any. */
  const [reading, setReading] = useState<AgentId | null>(null);

  const scrollRef = useRef<HTMLDivElement>(null);
  const pinnedToBottom = useRef(true);

  const isActivity = channel === ACTIVITY_CHANNEL;
  const agent = isActivity ? undefined : lookups.byId(channel);

  // Only auto-scroll when the operator is already at the bottom. Yanking the
  // view while they are reading back through a cascade is worse than a
  // scrollbar that does not move.
  //
  // Re-bound when a thread closes: the transcript is unmounted while one is
  // open, so the node that comes back is not the node this was listening to.
  useEffect(() => {
    const node = scrollRef.current;
    if (!node) return;
    const onScroll = () => {
      pinnedToBottom.current = node.scrollHeight - node.scrollTop - node.clientHeight < 80;
    };
    node.addEventListener("scroll", onScroll, { passive: true });
    return () => node.removeEventListener("scroll", onScroll);
  }, [reading]);

  // Reading `scrollHeight` forces the browser to lay the transcript out, so
  // this is a real cost rather than a free one. Coalesced into a frame,
  // because while text is arriving it is asked for far more often than the
  // screen refreshes.
  const pending = useRef(0);
  const follow = useCallback(() => {
    if (pending.current) return;
    pending.current = requestAnimationFrame(() => {
      pending.current = 0;
      const node = scrollRef.current;
      if (node && pinnedToBottom.current) node.scrollTop = node.scrollHeight;
    });
  }, []);

  useLayoutEffect(() => {
    const node = scrollRef.current;
    if (node && pinnedToBottom.current) node.scrollTop = node.scrollHeight;
  }, [messages]);

  // A channel switch abandons any half-confirmed destructive action, and any
  // thread opened off the old channel: a conversation between two other agents
  // is not what you asked for by clicking a third.
  useLayoutEffect(() => {
    setConfirmClear(false);
    setReading(null);
  }, [channel]);

  // The newest message, whether the transcript is being opened or coming back
  // from a thread. Coming back to where you were is not on offer: the
  // transcript was unmounted, so there is no scroll position to come back to,
  // and the top of the history is the one place it must not land.
  useLayoutEffect(() => {
    pinnedToBottom.current = true;
    const node = scrollRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [channel, reading]);

  // Built once per set of messages rather than per render: it walks every
  // message, and it is what decides which of them are drawn at all.
  const rows = useMemo(() => transcriptRows(messages ?? [], lookups), [messages, lookups]);

  // A message somebody arrived at from search. Run after the transcript is on
  // screen, because the row cannot be scrolled to before it exists, and the
  // read that widens the window to reach it lands a render later than the
  // channel switch does. The mark is cleared once it has been shown, so
  // reopening the channel later does not jump again.
  //
  // Matched against a list rather than a single id, because a row does not
  // stand for exactly one message: a burst stands for every message it
  // collapsed, and a message found inside one is read by opening the thread
  // behind the row rather than in the channel.
  useEffect(() => {
    if (!focused || messages === undefined) return;
    const row = scrollRef.current?.querySelector<HTMLElement>(`[data-message~="${focused}"]`);
    if (!row) return;
    // Not the newest message any more, so following the bottom would undo this
    // the moment anything else arrives.
    pinnedToBottom.current = false;
    row.scrollIntoView({ block: "center" });
    const timer = window.setTimeout(clearFocus, FLASH_MS);
    return () => window.clearTimeout(timer);
  }, [focused, messages, clearFocus]);

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
        {isActivity ? (
          <>
            <span
              aria-hidden="true"
              style={{ color: "var(--muted)", fontFamily: "var(--font-mono)" }}
            >
              #
            </span>
            <h1 className="pane__title">activity</h1>
            <p className="pane__subtitle">
              Who spoke to whom, in order. Click any arrow to read the message.
            </p>
          </>
        ) : agent ? (
          <>
            <AgentAvatar
              avatar={agent.avatar}
              color={agent.color}
              activity={activity[agent.id]}
              lifecycle={agent.lifecycle}
              seed={agent.id}
              size="sm"
            />
            <h1 className="pane__title">{agent.name}</h1>
            <p className="pane__subtitle">
              {agent.model}
              {agent.skills.length > 0 && ` · ${agent.skills.join(", ")}`}
            </p>
            <div style={{ marginLeft: "auto", display: "flex", gap: "0.25rem" }}>
              {confirmClear ? (
                <>
                  <button
                    type="button"
                    className="btn btn--danger"
                    onClick={() => {
                      setConfirmClear(false);
                      void api
                        .clearChannel(agent.id)
                        .then(() => loadChannel(agent.id))
                        .catch((error) => setBanner({ tone: "error", text: errorMessage(error) }));
                    }}
                  >
                    Delete this history
                  </button>
                  <button
                    type="button"
                    className="btn btn--ghost"
                    onClick={() => setConfirmClear(false)}
                  >
                    Keep
                  </button>
                </>
              ) : (
                <>
                  <button
                    type="button"
                    className="btn btn--ghost"
                    onClick={() => {
                      void api
                        .setAgentPaused(agent.id, !paused)
                        .catch((error) => setBanner({ tone: "error", text: errorMessage(error) }));
                    }}
                  >
                    {paused ? "Resume" : "Pause"}
                  </button>
                  <button
                    type="button"
                    className="btn btn--ghost"
                    onClick={() => setConfirmClear(true)}
                  >
                    Clear
                  </button>
                  <button
                    type="button"
                    className="btn btn--ghost"
                    onClick={() => onEditAgent(agent)}
                  >
                    Edit
                  </button>
                </>
              )}
            </div>
          </>
        ) : (
          <h1 className="pane__title">Channel unavailable</h1>
        )}
      </header>

      {isActivity ? (
        // A list cannot show who spoke to whom, so the activity view is a
        // board rather than a transcript.
        <ActivityFlow messages={messages ?? []} byId={lookups.byId} />
      ) : (
        <div className="pane__scroll" ref={scrollRef}>
          {messages === undefined ? (
            <p className="hint" style={{ padding: "1rem 1.15rem" }}>
              Loading…
            </p>
          ) : messages.length === 0 ? (
            <div className="empty">
              <p className="empty__body">No messages with {agent?.name ?? "this agent"} yet.</p>
            </div>
          ) : (
            rows.map((row) => {
              const stands = rowStandsFor(row);
              return (
                // Wrapped rather than marked on the entry itself: a row renders
                // as several different shapes depending on who sent what to
                // whom, and one of them is several rows.
                <div
                  key={row.key}
                  data-message={stands.join(" ")}
                  data-found={focused && stands.includes(focused) ? "true" : undefined}
                >
                  {row.kind === "peers" ? (
                    <PeerBurstRow peers={row.peers} onOpen={setReading} />
                  ) : row.kind === "refused" ? (
                    <RefusedRow peer={row.peer} at={row.at} body={row.body} reason={row.reason} />
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

          <LiveStreams channel={channel} lookups={lookups} follow={follow} />
        </div>
      )}

      {isActivity ? null : (
        <>
          {agent && <WorkingNote agent={agent} state={activity[agent.id]} />}
          <Composer
            placeholder={`Message ${agent?.name ?? "agent"}`}
            disabled={!agent || agent.lifecycle === "terminated"}
            disabledReason="This agent has been deleted."
            onSend={async (text, files) => {
              if (!agent) return;
              try {
                await api.sendMessage(agent.id, text, files);
              } catch (error) {
                setBanner({ tone: "error", text: errorMessage(error) });
                throw error;
              }
            }}
          />
        </>
      )}
    </section>
  );
}

/**
 * That the agent is still going, above the box you would type into.
 *
 * The sidebar already says "typing" beside the name, but the operator watching
 * a channel is looking at the bottom of it, waiting. A silent gap between
 * sending and the first token is the moment the app looks broken, and it is a
 * long one: a turn can spend several model calls on tool results before a word
 * is written for anybody to read.
 *
 * The name is revealed on hover rather than sat there permanently. Whose
 * channel this is has been established four times over by the time you reach
 * the bottom of it, so the still frame is one moving character and the
 * sentence is there for the moment you want it. It stays in the accessibility
 * tree either way, which is why this is opacity rather than a mount.
 */
function WorkingNote({ agent, state }: { agent: AgentCard; state: Activity | undefined }) {
  // Queued counts: the agent has work it has not read yet, and to the operator
  // that is the same thing as working. Awaiting approval does not: it is
  // waiting on a person, and the request itself is in this channel saying so.
  const working = state?.state === "thinking" || state?.state === "queued";
  if (!working) return null;

  return (
    <div className="working" role="status">
      <AgentAvatar
        avatar={agent.avatar}
        color={agent.color}
        size="sm"
        seed={agent.id}
        activity={{ state: "thinking" }}
        title={`${agent.name} is working`}
      />
      <span className="working__label">{agent.name} is working</span>
    </div>
  );
}

/**
 * The bubbles that are still being written, and nothing else.
 *
 * Its own component because it is the only thing on screen that changes while
 * text arrives. Subscribing here rather than in the parent means a token
 * re-renders two lines instead of the whole transcript above them: with thirty
 * messages loaded that was six thousand renders for one reply, and the window
 * froze with five agents working at once. It also means a token written in
 * another agent's channel costs this one nothing.
 */
function LiveStreams({
  channel,
  lookups,
  follow,
}: {
  channel: ChannelKey;
  lookups: ReturnType<typeof useAgentLookup>;
  follow: () => void;
}) {
  const streams = useStore((s) => s.streams);
  const live = Object.entries(streams).filter(([, buffer]) => buffer?.channelId === channel);

  // Keeps the newest text in view without the parent re-rendering to notice.
  useLayoutEffect(follow);

  return (
    <>
      {live.map(([id, buffer]) => {
        if (!buffer) return null;

        // A message bound for a peer will settle into a collapsed row, so it is
        // announced rather than streamed. Only text meant for the operator is
        // worth watching arrive.
        if (buffer.to.kind === "agent") {
          return (
            <WritingRow
              key={id}
              from={toPeer(lookups.byId(buffer.agentId), buffer.agentId)}
              to={toPeer(lookups.byId(buffer.to.id), buffer.to.id)}
            />
          );
        }
        return buffer.text.length > 0 ? (
          <StreamingMessage key={id} agent={lookups.byId(buffer.agentId)} text={buffer.text} />
        ) : null;
      })}
    </>
  );
}
