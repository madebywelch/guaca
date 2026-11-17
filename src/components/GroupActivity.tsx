import { useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { useAgentLookup } from "../lib/store";
import { type Envelope, errorMessage, type GroupId } from "../lib/types";
import { ActivityFlow } from "./ActivityFlow";

const WINDOW = 400;

export function GroupActivity({ group }: { group: GroupId }) {
  const lookups = useAgentLookup();
  const [messages, setMessages] = useState<Envelope[] | null>(null);
  const [failed, setFailed] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    setMessages(null);
    setFailed(null);
    void api
      .conversationFlow(group, WINDOW)
      .then((rows) => {
        if (live) setMessages(rows);
      })
      .catch((caught) => {
        if (live) setFailed(errorMessage(caught));
      });
    return () => {
      live = false;
    };
  }, [group]);

  if (failed) {
    return (
      <div className="empty">
        <p className="empty__body">{failed}</p>
      </div>
    );
  }

  if (messages === null) {
    return <p className="hint">Reading…</p>;
  }

  return <ActivityFlow messages={messages} byId={lookups.byId} />;
}
