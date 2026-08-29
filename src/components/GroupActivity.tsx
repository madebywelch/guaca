/**
 * A crew's traffic, as the flow board, inside that crew's settings.
 *
 * It was a channel at the top of the rail, under the wordmark and beside the
 * search box: the first thing in the app after Guaca's own name, and one of the
 * least-pressed things in it. What it answers — who spoke to whom, in what
 * order, what set a run off and what the run cost — is analysis. Somebody
 * arrives at it having decided to look into something, which is a completely
 * different act from opening a channel, and a permanent row in the primary
 * navigation is what said otherwise.
 *
 * Read here rather than held in the store, and that is the same decision again.
 * The store maintained a second copy of every arriving message against a board
 * nobody had necessarily ever opened; a board somebody opens deliberately can
 * afford one read at the moment they open it. It does not follow the
 * conversation afterward either, which is not a gap: a board that reshuffles
 * under a reader is one they lose their place in, and the thing they came here
 * to read has already happened.
 */

import { useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { useAgentLookup } from "../lib/store";
import { type Envelope, errorMessage, type GroupId } from "../lib/types";
import { ActivityFlow } from "./ActivityFlow";

/** How far back the board reaches. The store caps this at 2,000. */
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
