import { useEffect, useLayoutEffect, useState } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { useDismiss } from "../lib/dismiss";
import { useFollowBottom } from "../lib/follow";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { continues, type Lookups, toPeer } from "../lib/transcript";
import type { AgentId, Envelope } from "../lib/types";
import { Icon } from "./Icon";
import { MessageItem } from "./MessageItem";

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
  const showFailure = useStore((s) => s.showFailure);
  const [messages, setMessages] = useState<Envelope[] | undefined>(undefined);
  const { ref: scrollRef, follow } = useFollowBottom();

  const beat = useStore((s) => Math.max(s.lastActive[self] ?? 0, s.lastActive[peer] ?? 0));

  useEffect(() => {
    let live = true;
    void api
      .pairMessages(self, peer)
      .then((rows) => {
        if (live) setMessages(rows);
      })
      .catch((error) => {
        if (live) showFailure(error);
      });
    return () => {
      live = false;
    };
  }, [self, peer, beat, showFailure]);

  useDismiss(onClose);

  useLayoutEffect(follow, [follow, messages]);

  const here = toPeer(lookups.byId(self), self);
  const there = toPeer(lookups.byId(peer), peer);

  return (
    <section className="pane">
      <header className="pane__header">
        <h1 className="pane__title pane__pair">
          <AgentAvatar avatar={here.avatar} color={here.color} size="sm" seed={here.id} />
          {here.name}
          <span className="pane__between" aria-hidden="true">
            <Icon name="swap" size={16} />
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
          <span className="viewonly__lock">
            <Icon name="lock" size={16} />
          </span>
          This chat is view-only
        </span>
        <button type="button" className="btn" onClick={onClose}>
          Close chat
        </button>
      </div>
    </section>
  );
}
