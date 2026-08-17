import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { continues, type Lookups, toPeer } from "../lib/transcript";
import { type AgentId, type Envelope, errorMessage } from "../lib/types";
import { MessageItem } from "./MessageItem";

/**
 * What two agents said to each other, on its own.
 *
 * The channel shows that an exchange happened and with whom; this is where it
 * is read. It is a separate view rather than a modal because it is a
 * conversation, sometimes a long one, and a dialog over the top of the window
 * is a bad place to read forty messages.
 *
 * Read-only, and said so on the page. Neither of these agents is addressable
 * from here: a message the operator typed into this thread would have to be
 * sent as one of them, which is not a thing this app lets anybody do. Saying
 * "view-only" is cheaper than a composer that refuses.
 *
 * Loaded from the runtime rather than from what the channel already holds,
 * because no channel holds it. A send is filed under the recipient and the
 * answer under the sender, and an automatic reply leaves no trace at all in
 * the channel of the agent that wrote it.
 */
export function PairThread({
  self,
  peer,
  lookups,
  onClose,
}: {
  self: AgentId;
  peer: AgentId;
  lookups: Lookups;
  onClose: () => void;
}) {
  const setBanner = useStore((s) => s.setBanner);
  const [messages, setMessages] = useState<Envelope[] | undefined>(undefined);
  const scrollRef = useRef<HTMLDivElement>(null);
  const pinnedToBottom = useRef(true);

  // Deliberately over-eager. `lastActive` moves for both ends of every message,
  // so this fires for traffic that has nothing to do with this pair, and it
  // fires for every message that does, which is the half that matters. A thread
  // left open while its two agents are still talking has to keep up, and one
  // small indexed read is cheaper than the state it would take to know better.
  const beat = useStore((s) => Math.max(s.lastActive[self] ?? 0, s.lastActive[peer] ?? 0));

  useEffect(() => {
    let live = true;
    void api
      .pairMessages(self, peer)
      .then((rows) => {
        if (live) setMessages(rows);
      })
      .catch((error) => {
        if (live) setBanner({ tone: "error", text: errorMessage(error) });
      });
    return () => {
      live = false;
    };
  }, [self, peer, beat, setBanner]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Only while they are at the bottom. This reloads on any activity by either
  // agent, so without it a reader half way up a long exchange is thrown back
  // to the end by a message that had nothing to do with what they were reading.
  useEffect(() => {
    const node = scrollRef.current;
    if (!node) return;
    const onScroll = () => {
      pinnedToBottom.current = node.scrollHeight - node.scrollTop - node.clientHeight < 80;
    };
    node.addEventListener("scroll", onScroll, { passive: true });
    return () => node.removeEventListener("scroll", onScroll);
  }, []);

  // The newest is what the operator came for: this opens off a row saying
  // something just happened between these two.
  useLayoutEffect(() => {
    const node = scrollRef.current;
    if (node && pinnedToBottom.current) node.scrollTop = node.scrollHeight;
  }, [messages]);

  const here = toPeer(lookups.byId(self), self);
  const there = toPeer(lookups.byId(peer), peer);

  return (
    <section className="pane">
      <header className="pane__header">
        {/* One heading, not two: the title of this view is the pair. */}
        <h1 className="pane__title pane__pair">
          <AgentAvatar avatar={here.avatar} color={here.color} size="sm" seed={here.id} />
          {here.name}
          <span className="pane__between" aria-hidden="true">
            ⇄
          </span>
          <AgentAvatar avatar={there.avatar} color={there.color} size="sm" seed={there.id} />
          {there.name}
        </h1>
      </header>

      <div className="pane__scroll" ref={scrollRef}>
        {messages === undefined ? (
          <p className="hint" style={{ padding: "1rem 1.15rem" }}>
            Loading…
          </p>
        ) : messages.length === 0 ? (
          <div className="empty">
            <p className="empty__body">
              {here.name} and {there.name} have not written to each other.
            </p>
          </div>
        ) : (
          messages.map((message, index) => (
            <MessageItem
              key={message.id}
              message={message}
              lookups={lookups}
              continued={continues(messages[index - 1], message)}
            />
          ))
        )}
      </div>

      <div className="viewonly">
        <span className="viewonly__note">
          <svg viewBox="0 0 24 24" className="viewonly__lock" aria-hidden="true">
            <path
              d="M7 10V7a5 5 0 0 1 10 0v3M5.5 10h13v10h-13z"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.6"
              strokeLinejoin="round"
            />
          </svg>
          This chat is view-only
        </span>
        <button type="button" className="btn" onClick={onClose}>
          Close chat
        </button>
      </div>
    </section>
  );
}
