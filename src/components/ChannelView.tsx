import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { api } from "../lib/ipc";
import { thoughtLine } from "../lib/reasoning";
import { ACTIVITY_CHANNEL, type ChannelKey, useAgentLookup, useStore } from "../lib/store";
import { rowStandsFor, toPeer, transcriptRows } from "../lib/transcript";
import { type Activity, type AgentCard, type AgentId, errorMessage } from "../lib/types";
import { ActivityFlow } from "./ActivityFlow";
import { Composer } from "./Composer";
import { MessageItem, StreamingMessage, WhenRow } from "./MessageItem";
import { PairThread } from "./PairThread";
import { PeerBurstRow, RefusedRow, WritingRow } from "./WireRow";

interface Props {
  channel: ChannelKey;
  /** Where the operator asked for an agent's actions, and on whom. */
  onOpenMenu: (agent: AgentCard, at: { x: number; y: number }) => void;
}

/** How long a message arrived at from search stays marked. */
const FLASH_MS = 1800;

export function ChannelView({ channel, onOpenMenu }: Props) {
  const lookups = useAgentLookup();
  const messages = useStore((s) => s.messages[channel]);
  const activity = useStore((s) => s.activity);
  const setBanner = useStore((s) => s.setBanner);
  const focused = useStore((s) => s.focused);
  const clearFocus = useStore((s) => s.clearFocus);

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

  // A channel switch abandons any thread opened off the old one: a conversation
  // between two other agents is not what you asked for by clicking a third.
  useLayoutEffect(() => {
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
            {/* The one thing about an agent that changes what this pane does.
                Everything else it is set up with is edited rarely and read
                behind the menu, rather than sitting over every message. */}
            {paused && <span className="pane__flag">Paused</span>}
            <button
              type="button"
              className="pane__more"
              aria-label={`Actions for ${agent.name}`}
              title={`Actions for ${agent.name}`}
              onClick={(event) => {
                // Under the button it came from. The menu measures itself and
                // slides back inside the window if it does not fit there.
                const box = event.currentTarget.getBoundingClientRect();
                onOpenMenu(agent, { x: box.left, y: box.bottom + 4 });
              }}
            >
              ⋯
            </button>
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
                  ) : row.kind === "when" ? (
                    <WhenRow at={row.at} />
                  ) : (
                    // Two participants, one of them named at the top of the
                    // pane and the other reading this. Nothing here needs
                    // telling whose words it is looking at.
                    <MessageItem
                      message={row.message}
                      lookups={lookups}
                      continued={row.continued}
                      named={false}
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
 * That the agent is still going, above the box you would type into, and what
 * it is thinking while it goes.
 *
 * The sidebar already says "typing" beside the name, but the operator watching
 * a channel is looking at the bottom of it, waiting. A silent gap between
 * sending and the first token is the moment the app looks broken, and it is a
 * long one: a turn can spend several model calls on tool results before a word
 * is written for anybody to read. A pulse says the turn is alive; it does not
 * say what it is doing, and those are different questions when the wait is
 * thirty seconds.
 *
 * So when the model publishes its working, the line shows the line it is on.
 * Which means it is only ever one line: the thinking is gone the moment the
 * turn ends and there is nowhere to scroll back to, so anything but the
 * newest words would be chrome pointing at something the operator cannot
 * reach. Where the model publishes nothing, this is the sentence it always
 * was, revealed on hover, because whose channel this is has been established
 * four times over by the time you reach the bottom of it. It stays in the
 * accessibility tree either way, which is why that is opacity rather than a
 * mount.
 *
 * Subscribed here rather than in the parent for the same reason `LiveStreams`
 * is: the thought changes every sixteen milliseconds and the transcript above
 * it does not.
 */
function WorkingNote({ agent, state }: { agent: AgentCard; state: Activity | undefined }) {
  const thought = thoughtLine(useStore((s) => s.reasoning[agent.id]));

  // Queued counts: the agent has work it has not read yet, and to the operator
  // that is the same thing as working. Awaiting approval does not: it is
  // waiting on a person, and the request itself is in this channel saying so.
  const working = state?.state === "thinking" || state?.state === "queued";
  if (!working) return null;

  return (
    // A sentence that appears once is a status worth announcing; a line
    // replaced several times a second is not. So this is a live region for
    // exactly as long as it holds the sentence, which is from the moment the
    // turn starts until the model publishes its first thought.
    <div
      className="working"
      role={thought ? undefined : "status"}
      data-thinking={thought ? "true" : undefined}
    >
      <AgentAvatar
        avatar={agent.avatar}
        color={agent.color}
        size="sm"
        seed={agent.id}
        activity={{ state: "thinking" }}
        title={`${agent.name} is working`}
      />
      <span className="working__label">{thought || `${agent.name} is working`}</span>
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
        // worth watching arrive. The writer is the peer: this stream is filed
        // in the recipient's channel, which is the one being looked at.
        if (buffer.to.kind === "agent") {
          return (
            <WritingRow key={id} peer={toPeer(lookups.byId(buffer.agentId), buffer.agentId)} />
          );
        }
        return buffer.text.length > 0 ? (
          <StreamingMessage key={id} agent={lookups.byId(buffer.agentId)} text={buffer.text} />
        ) : null;
      })}
    </>
  );
}
