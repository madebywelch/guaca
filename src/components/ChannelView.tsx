import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { useFollowBottom } from "../lib/follow";
import { api } from "../lib/ipc";
import { thoughtLine } from "../lib/reasoning";
import { ACTIVITY_CHANNEL, type ChannelKey, useAgentLookup, useStore } from "../lib/store";
import { rowStandsFor, toPeer, transcriptRows } from "../lib/transcript";
import {
  type Activity,
  type AgentCard,
  type AgentId,
  type Envelope,
  errorMessage,
  plainText,
} from "../lib/types";
import { ActivityFlow } from "./ActivityFlow";
import { Composer } from "./Composer";
import { MessageItem, StreamingMessage, WhenRow } from "./MessageItem";
import { NewMenu } from "./NewMenu";
import { PairThread } from "./PairThread";
import { PeerBurstRow, RefusedRow, WritingRow } from "./WireRow";

interface Props {
  channel: ChannelKey;
  /** Where the operator asked for an agent's actions, and on whom. */
  onOpenMenu: (agent: AgentCard, at: { x: number; y: number }) => void;
  /** The plus at the end of the header. App-level, not this channel's. */
  onNewAgent: () => void;
  onNewGroup: () => void;
}

/** How long a message arrived at from search stays marked. */
const FLASH_MS = 1800;

export function ChannelView({ channel, onOpenMenu, onNewAgent, onNewGroup }: Props) {
  const lookups = useAgentLookup();
  const messages = useStore((s) => s.messages[channel]);
  const activity = useStore((s) => s.activity);
  const setBanner = useStore((s) => s.setBanner);
  const focused = useStore((s) => s.focused);
  const clearFocus = useStore((s) => s.clearFocus);

  /** The peer whose thread is open over this channel, if any. */
  const [reading, setReading] = useState<AgentId | null>(null);

  // Only ever moves the transcript for somebody who is already watching its
  // newest line. Yanking the view while they are reading back through a
  // cascade is worse than a scrollbar that does not move: `lib/follow.ts`.
  const { ref: scrollRef, node: transcript, follow, pin } = useFollowBottom();

  const isActivity = channel === ACTIVITY_CHANNEL;
  const agent = isActivity ? undefined : lookups.byId(channel);

  useLayoutEffect(follow, [follow, messages]);

  // A channel switch abandons any thread opened off the old one: a conversation
  // between two other agents is not what you asked for by clicking a third.
  useLayoutEffect(() => {
    setReading(null);
  }, [channel]);

  // The newest message, whether the transcript is being opened or coming back
  // from a thread. Coming back to where you were is not on offer: the
  // transcript was unmounted, so there is no scroll position to come back to,
  // and the top of the history is the one place it must not land.
  useLayoutEffect(pin, [pin, channel, reading]);

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
    const row = transcript()?.querySelector<HTMLElement>(`[data-message~="${focused}"]`);
    if (!row) return;
    // Nothing here says to stop following. Coming up the transcript to reach
    // the row is a scroll like any other, and releases the bottom exactly as
    // the operator's own would; a row that was already at the end moves
    // nothing, and following the end is what keeps it in view.
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
        {/* Last, and after the conditional, so it is the same control in the
            same place whichever of the three the header is drawing. */}
        <NewMenu onNewAgent={onNewAgent} onNewGroup={onNewGroup} />
      </header>

      {isActivity ? (
        // A list cannot show who spoke to whom, so the activity view is a
        // board rather than a transcript.
        <ActivityFlow messages={messages ?? []} byId={lookups.byId} />
      ) : (
        // A log rather than a plain box, so a screen reader offers it as the
        // transcript it is. `aria-live` is off deliberately: the role would
        // otherwise announce politely, and opening a channel replaces three
        // hundred rows at once, which is the whole history read aloud for the
        // crime of clicking a name. What is worth hearing is announced by
        // `Arrivals` below, one message at a time, as it lands.
        <div
          className="pane__scroll"
          ref={scrollRef}
          role="log"
          aria-live="off"
          aria-label={`Conversation with ${agent?.name ?? "this agent"}`}
        >
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
          {/* Outside the log on purpose. Inside it the sentence is a second
              copy of the newest message for anybody reading the transcript
              itself, which is the cost of announcing it to everybody else. */}
          <Arrivals channel={channel} messages={messages} lookups={lookups} />
          {agent && <WorkingNote agent={agent} state={activity[agent.id]} />}
          <Composer
            placeholder={`Message ${agent?.name ?? "agent"}`}
            disabled={!agent || agent.lifecycle === "terminated"}
            disabledReason="This agent has been deleted."
            onSend={async (text, files) => {
              if (!agent) return;
              // Typing into the box is a decision to be at the end of the
              // transcript, so this is the one thing besides opening a channel
              // that overrides where the operator was reading. Their own
              // message landing off screen, with nothing following it, is the
              // same bug in the other direction.
              pin();
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
  // One string, so this re-renders when the run changes and not when a token
  // arrives. Subscribing to `streams` here to find it would undo the whole
  // reason this component exists.
  const run = useStore((s) => s.activeRun[agent.id]);
  const setBanner = useStore((s) => s.setBanner);
  const [stopping, setStopping] = useState(false);

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
      {run && (
        // Stops the conversation, not the agent. A message that reached four
        // agents is one run, and stopping only the one on screen would leave
        // the other three working on the operator's bill: hence the wording,
        // which is the same distinction the button actually makes.
        <button
          type="button"
          className="btn btn--ghost btn--small working__stop"
          disabled={stopping}
          title="Stop this conversation and everything it set off"
          onClick={async () => {
            setStopping(true);
            try {
              // False means it had already finished, which needs no telling:
              // the answer is on screen.
              await api.stopRun(run);
            } catch (error) {
              setBanner({ tone: "error", text: errorMessage(error) });
            } finally {
              setStopping(false);
            }
          }}
        >
          Stop
        </button>
      )}
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
    // Never announced. A live region reading a bubble as it is typed says the
    // same sentence a dozen times before it is finished; the finished message
    // is announced once, by `Arrivals`, and "is working" by `WorkingNote`.
    <div aria-live="off">
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
    </div>
  );
}

/**
 * The one thing a transcript says out loud.
 *
 * Waiting for a reply is the whole shape of using this app, and a reader who
 * cannot see the column arrive has no way of knowing one did. So a message
 * addressed to the operator is announced once, when it settles, and nothing
 * else is: what is announced is exactly what is drawn as a full bubble.
 *
 * That symmetry is the rule, and it is the same one the channel is arranged by.
 * Peer traffic and tool trails are collapsed on screen precisely because
 * reading them line by line buries the conversation; read aloud they would bury
 * it far more thoroughly, since there is no glancing past a sentence being
 * spoken.
 *
 * Its own component so a message landing in another agent's channel costs this
 * one nothing, and so the sentence is built where it is announced rather than
 * threaded down from the pane.
 */
function Arrivals({
  channel,
  messages,
  lookups,
}: {
  channel: ChannelKey;
  messages: Envelope[] | undefined;
  lookups: ReturnType<typeof useAgentLookup>;
}) {
  const [said, setSaid] = useState("");
  /** The newest message already accounted for. Null until the channel is read. */
  const seen = useRef<string | null>(null);

  // Declared before the one below so it runs first on the render that changes
  // both: a channel switch has to forget what it had seen before that render
  // decides whether the newest message is news.
  useEffect(() => {
    seen.current = null;
    setSaid("");
  }, [channel]);

  useEffect(() => {
    const newest = messages?.[messages.length - 1];
    if (!newest) return;
    // Opening a channel is not an arrival. Everything in it was already there.
    if (seen.current === null || seen.current === newest.id) {
      seen.current = newest.id;
      return;
    }
    seen.current = newest.id;
    setSaid(announcement(newest, lookups.byId));
  }, [messages, lookups]);

  return (
    // Explicit rather than left to the role: the transcript above declares
    // `aria-live="off"`, live-region settings are inherited, and an implicit
    // value from `role="status"` is a weaker claim than a stated one.
    <p className="offscreen" role="status" aria-live="polite" aria-atomic="true">
      {said}
    </p>
  );
}

/** What an arriving message is worth saying, or nothing. */
function announcement(message: Envelope, byId: ReturnType<typeof useAgentLookup>["byId"]): string {
  if (message.from.kind !== "agent") return "";
  const who = byId(message.from.id)?.name ?? "An agent";

  // Ahead of the text, as everywhere else: this is the one thing in a
  // transcript the operator is expected to act on rather than read.
  const asking = message.parts.find((part) => part.type === "approval");
  if (asking) return `${who} is asking you: ${asking.summary}`;

  // Addressed to a peer, so it is a collapsed row here rather than a bubble.
  if (message.to.kind !== "human") return "";

  const body = plainText(message);
  if (!body) return "";
  // Long enough to be the message, short enough that a reader can interrupt it
  // and go and read the column instead.
  return body.length > 300 ? `${who}: ${body.slice(0, 300)}…` : `${who}: ${body}`;
}
