import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { api } from "../lib/ipc";
import { ACTIVITY_CHANNEL, type ChannelKey, useAgentLookup, useStore } from "../lib/store";
import { type AgentCard, type Envelope, errorMessage } from "../lib/types";
import { ActivityFlow } from "./ActivityFlow";
import { Composer } from "./Composer";
import { ComputerPane } from "./ComputerPane";
import { MessageItem, StreamingMessage } from "./MessageItem";
import { toPeer, WritingRow } from "./WireRow";

interface Props {
  channel: ChannelKey;
  onEditAgent: (agent: AgentCard) => void;
}

/** True when consecutive messages should merge under one header. */
function isContinuation(previous: Envelope | undefined, current: Envelope): boolean {
  if (!previous) return false;
  if (JSON.stringify(previous.from) !== JSON.stringify(current.from)) return false;
  if (JSON.stringify(previous.to) !== JSON.stringify(current.to)) return false;
  return current.createdAt - previous.createdAt < 4 * 60 * 1000;
}

export function ChannelView({ channel, onEditAgent }: Props) {
  const lookups = useAgentLookup();
  const messages = useStore((s) => s.messages[channel]);
  const streams = useStore((s) => s.streams);
  const activity = useStore((s) => s.activity);
  const setBanner = useStore((s) => s.setBanner);

  const loadChannel = useStore((s) => s.loadChannel);
  const [confirmClear, setConfirmClear] = useState(false);

  const scrollRef = useRef<HTMLDivElement>(null);
  const pinnedToBottom = useRef(true);

  const isActivity = channel === ACTIVITY_CHANNEL;
  const agent = isActivity ? undefined : lookups.byId(channel);

  const live = Object.entries(streams).filter(([, buffer]) => buffer?.channelId === channel);

  // Only auto-scroll when the operator is already at the bottom. Yanking the
  // view while they are reading back through a cascade is worse than a
  // scrollbar that does not move.
  useEffect(() => {
    const node = scrollRef.current;
    if (!node) return;
    const onScroll = () => {
      pinnedToBottom.current = node.scrollHeight - node.scrollTop - node.clientHeight < 80;
    };
    node.addEventListener("scroll", onScroll, { passive: true });
    return () => node.removeEventListener("scroll", onScroll);
  }, []);

  useLayoutEffect(() => {
    const node = scrollRef.current;
    if (node && pinnedToBottom.current) node.scrollTop = node.scrollHeight;
  }, [messages, live.length, streams]);

  // A channel switch always starts at the newest message, and abandons any
  // half-confirmed destructive action.
  useLayoutEffect(() => {
    setConfirmClear(false);
    pinnedToBottom.current = true;
    const node = scrollRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [channel]);

  const paused = agent?.lifecycle === "paused";

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
          {/* The agent's computer sits over the transcript rather than beside
              it: the reading column keeps its measure, and a desktop is
              something you glance at while reading, not a second column. */}
          {agent && <ComputerPane agent={agent} />}
          {messages === undefined ? (
            <p className="hint" style={{ padding: "1rem 1.15rem" }}>
              Loading…
            </p>
          ) : messages.length === 0 ? (
            <div className="empty">
              <p className="empty__body">No messages with {agent?.name ?? "this agent"} yet.</p>
            </div>
          ) : (
            messages.map((message, index) => (
              <MessageItem
                key={message.id}
                message={message}
                lookups={lookups}
                continued={isContinuation(messages[index - 1], message)}
                feed={false}
              />
            ))
          )}

          {live.map(([id, buffer]) => {
            if (!buffer) return null;

            // A message bound for a peer will settle into a collapsed row, so
            // it is announced rather than streamed. Only text meant for the
            // operator is worth watching arrive.
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
        </div>
      )}

      {isActivity ? null : (
        <Composer
          placeholder={`Message ${agent?.name ?? "agent"}`}
          disabled={!agent || agent.lifecycle === "terminated"}
          disabledReason="This agent has been deleted."
          onSend={async (text) => {
            if (!agent) return;
            try {
              await api.sendMessage(agent.id, text);
            } catch (error) {
              setBanner({ tone: "error", text: errorMessage(error) });
              throw error;
            }
          }}
        />
      )}
    </section>
  );
}
